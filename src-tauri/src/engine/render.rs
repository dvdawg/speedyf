//! Raw-bindings render path. All calls MUST happen on the engine thread —
//! PDFium is not thread-safe. Whole pages and tiles both go through
//! `FPDF_RenderPageBitmap`, which applies the page's own /Rotate itself;
//! tiles are produced by offsetting the page origin negatively inside a
//! tile-sized bitmap, so no oversized full-page bitmap is ever allocated.

use crate::engine::types::TileRect;
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_DOCUMENT, FPDF_PAGE, FS_SIZEF, IFSDK_PAUSE,
};
use std::ffi::c_void;

const FPDF_ANNOT_FLAG: i32 = 0x01;
const FORMAT_BGRA: i32 = 4; // FPDFBitmap_BGRA
/// Hard guard: refuse absurd single allocations (frontend tiles above 4 Mpx).
const MAX_PIXELS: u64 = 64_000_000;
const RENDER_TO_BE_CONTINUED: i32 = 1;
const RENDER_DONE: i32 = 2;

struct PauseContext<'a> {
    cancelled: &'a dyn Fn() -> bool,
}

unsafe extern "C" fn need_to_pause_now(interface: *mut IFSDK_PAUSE) -> i32 {
    if interface.is_null() {
        return 0;
    }
    // SAFETY: `render_page()` keeps both the interface and context alive for
    // every synchronous Start/Continue call. The callback catches panics so
    // none can cross PDFium's C ABI.
    let context = unsafe { (*interface).user.cast::<PauseContext<'_>>().as_ref() };
    context
        .and_then(|context| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (context.cancelled)())).ok()
        })
        .unwrap_or(true) as i32
}

pub fn open_document(
    b: &dyn PdfiumLibraryBindings,
    path: &str,
    password: Option<&str>,
) -> AppResult<FPDF_DOCUMENT> {
    let doc = b.FPDF_LoadDocument(path, password);
    if doc.is_null() {
        // FPDF_GetLastError returns c_ulong: 32-bit on Windows, 64-bit on
        // Unix. Clippy only ever sees the target it runs on, so on Windows
        // this reads as a no-op cast — but dropping it fails to compile on
        // Unix, where it is a real narrowing. The mirror of the cast in
        // engine::formal, which is redundant on Unix and required on Windows.
        #[allow(clippy::unnecessary_cast)]
        let code = b.FPDF_GetLastError() as u32;
        return Err(match code {
            2 => AppError::Io(format!("cannot open file: {path}")),
            3 => AppError::Malformed("file is not a valid PDF".into()),
            4 => AppError::PasswordRequired,
            5 => AppError::Malformed("unsupported security scheme".into()),
            6 => AppError::Malformed("page error".into()),
            e => AppError::Internal(format!("pdfium error {e}")),
        });
    }
    Ok(doc)
}

/// Display size (points) of a page WITHOUT loading the page object.
/// PDFium folds the page's /Rotate into this size.
pub fn page_display_size(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    index: i32,
) -> Option<[f32; 2]> {
    let mut size = FS_SIZEF {
        width: 0.0,
        height: 0.0,
    };
    if b.FPDF_GetPageSizeByIndexF(doc, index, &mut size) != 0 && size.width > 0.0 {
        Some([size.width, size.height])
    } else {
        None
    }
}

pub struct RenderOutput {
    pub png: Vec<u8>,
}

/// Render a page (or a tile of it) at `scale` device pixels per point with an
/// extra rotation on top of the page's own /Rotate, returning encoded PNG.
pub fn render_page<F>(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    display_size_pt: [f32; 2],
    scale: f32,
    extra_rot_deg: u16,
    tile: Option<TileRect>,
    cancelled: F,
) -> AppResult<RenderOutput>
where
    F: Fn() -> bool,
{
    let [disp_w_pt, disp_h_pt] = display_size_pt;
    let full_w = (disp_w_pt * scale).round().max(1.0) as i64;
    let full_h = (disp_h_pt * scale).round().max(1.0) as i64;
    // FPDF_RenderPageBitmap expects size_x/size_y in FINAL (rotated) space.
    let (size_x, size_y) = if extra_rot_deg == 90 || extra_rot_deg == 270 {
        (full_h, full_w)
    } else {
        (full_w, full_h)
    };
    let (bmp_w, bmp_h, start_x, start_y) = match tile {
        Some(t) => (
            t.w.max(1) as i64,
            t.h.max(1) as i64,
            -(t.x as i64),
            -(t.y as i64),
        ),
        None => (size_x, size_y, 0, 0),
    };
    if bmp_w * bmp_h > MAX_PIXELS as i64 {
        return Err(AppError::Internal(format!(
            "render too large: {bmp_w}x{bmp_h} px (tile this page)"
        )));
    }

    let bitmap = b.FPDFBitmap_CreateEx(
        bmp_w as i32,
        bmp_h as i32,
        FORMAT_BGRA,
        std::ptr::null_mut(),
        0,
    );
    if bitmap.is_null() {
        return Err(AppError::Internal("bitmap allocation failed".into()));
    }
    b.FPDFBitmap_FillRect(bitmap, 0, 0, bmp_w as i32, bmp_h as i32, 0xFFFF_FFFF);
    let context = PauseContext {
        cancelled: &cancelled,
    };
    let mut pause = IFSDK_PAUSE {
        version: 1,
        NeedToPauseNow: Some(need_to_pause_now),
        user: (&context as *const PauseContext<'_>)
            .cast_mut()
            .cast::<c_void>(),
    };
    let mut status = b.FPDF_RenderPageBitmap_Start(
        bitmap,
        page,
        start_x as i32,
        start_y as i32,
        size_x as i32,
        size_y as i32,
        (extra_rot_deg / 90) as i32,
        FPDF_ANNOT_FLAG,
        &mut pause,
    );
    while status == RENDER_TO_BE_CONTINUED && !cancelled() {
        status = b.FPDF_RenderPage_Continue(page, &mut pause);
    }
    b.FPDF_RenderPage_Close(page);
    if cancelled() {
        b.FPDFBitmap_Destroy(bitmap);
        return Err(AppError::Stale);
    }
    if status != RENDER_DONE {
        b.FPDFBitmap_Destroy(bitmap);
        return Err(AppError::Internal(format!(
            "progressive render failed with status {status}"
        )));
    }

    let stride = b.FPDFBitmap_GetStride(bitmap) as usize;
    let buffer = b.FPDFBitmap_GetBuffer(bitmap) as *const u8;
    if buffer.is_null() {
        b.FPDFBitmap_Destroy(bitmap);
        return Err(AppError::Internal("bitmap buffer unavailable".into()));
    }
    let (w, h) = (bmp_w as usize, bmp_h as usize);
    // BGRA → RGB (pages are composited over an opaque white fill).
    let mut rgb = vec![0u8; w * h * 3];
    // SAFETY: buffer is valid for stride*h bytes until FPDFBitmap_Destroy.
    let src = unsafe { std::slice::from_raw_parts(buffer, stride * h) };
    for row in 0..h {
        let s = &src[row * stride..row * stride + w * 4];
        let d = &mut rgb[row * w * 3..(row + 1) * w * 3];
        for col in 0..w {
            d[col * 3] = s[col * 4 + 2];
            d[col * 3 + 1] = s[col * 4 + 1];
            d[col * 3 + 2] = s[col * 4];
        }
    }
    b.FPDFBitmap_Destroy(bitmap);

    let png = encode_png(&rgb, w as u32, h as u32)?;
    Ok(RenderOutput { png })
}

fn encode_png(rgb: &[u8], w: u32, h: u32) -> AppResult<Vec<u8>> {
    let mut out = Vec::with_capacity(rgb.len() / 6);
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        enc.set_filter(png::FilterType::Sub);
        let mut writer = enc
            .write_header()
            .map_err(|e| AppError::Internal(format!("png: {e}")))?;
        writer
            .write_image_data(rgb)
            .map_err(|e| AppError::Internal(format!("png: {e}")))?;
    }
    Ok(out)
}

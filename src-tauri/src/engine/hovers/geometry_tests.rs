//! Link previews on pages whose display is not their user space: turned by
//! /Rotate, cut by an offset crop box, or scanned - an image with an OCR
//! text layer under it, or an image alone. Real PDFs written here, read and
//! rendered by PDFium: the crop must land on the target and the raster of
//! it must hold ink, never a blank or shifted slice.

use super::page::Page;
use super::{extract, prepare_page, targets};
use crate::engine::{pdfium_init, preview, render, text};
use pdfium_render::prelude::*;
use std::io::Write as _;

const W: f32 = 612.0;
const H: f32 = 792.0;

/// How a fixture page is set.
#[derive(Clone, Copy, Debug)]
struct Setup {
    rotate: u16,
    /// The crop box's offset into a larger media box.
    offset: f32,
    /// Text drawn as an image of itself, with invisible text over it.
    scan: bool,
    /// The image only: no text layer at all.
    image_only: bool,
}

const PLAIN: Setup = Setup {
    rotate: 0,
    offset: 0.0,
    scan: false,
    image_only: false,
};

/// The page's lines in display space (y-up, upright 612 x 792 portrait):
/// (x, baseline, font, text).
fn layout() -> Vec<(f32, f32, &'static str, &'static str)> {
    let mut lines = Vec::new();
    for (i, y) in [740.0, 726.0, 712.0, 698.0].into_iter().enumerate() {
        let text = [
            "Running text of the body set across the page, line one.",
            "Running text of the body set across the page, line two.",
            "Running text of the body set across the page, line three.",
            "Running text of the body set across the page, line four.",
        ][i];
        lines.push((72.0, y, "F1", text));
    }
    lines.push((72.0, 672.0, "F2", "Theorem 1."));
    lines.push((
        128.0,
        672.0,
        "F3",
        "Every bounded sequence of reals has a convergent",
    ));
    lines.push((
        72.0,
        658.0,
        "F3",
        "subsequence, and its limit lies in the closed hull.",
    ));
    lines.push((72.0, 634.0, "F2", "Proof."));
    lines.push((
        110.0,
        634.0,
        "F1",
        "Take the bounded sequence and pass to a subsequence.",
    ));
    let mut y = 620.0;
    while y > 420.0 {
        lines.push((
            72.0,
            y,
            "F1",
            "Running text of the body set across the page after it.",
        ));
        y -= 14.0;
    }
    lines
}

/// Display (y-up) to user space, and the text matrix's rotation, for a
/// page turned `rotate` degrees clockwise whose crop box starts at
/// (`offset`, `offset`).
fn to_user(setup: Setup, x: f32, y: f32) -> (f32, f32) {
    let o = setup.offset;
    let (ux, uy) = match setup.rotate {
        90 => (H - y, x),
        180 => (W - x, H - y),
        270 => (y, W - x),
        _ => (x, y),
    };
    (ux + o, uy + o)
}

fn text_matrix(rotate: u16) -> [i32; 4] {
    match rotate {
        90 => [0, 1, -1, 0],
        180 => [-1, 0, 0, -1],
        270 => [0, -1, 1, 0],
        _ => [1, 0, 0, 1],
    }
}

/// An upright page of the layout, whatever the user space under it.
fn fixture(setup: Setup) -> Vec<u8> {
    // A turned page's user space is the portrait page on its side.
    let (mw, mh) = if setup.rotate == 90 || setup.rotate == 270 {
        (H, W)
    } else {
        (W, H)
    };
    let mut content = String::new();
    let mut image = None;
    if setup.scan || setup.image_only {
        // A 4pt-per-pixel scan of the page: ink where the words are.
        let (iw, ih) = ((W / 4.0) as usize, (H / 4.0) as usize);
        let mut pixels = vec![255u8; iw * ih];
        for (x, y, _, t) in layout() {
            let x1 = x + 5.0 * t.len() as f32;
            let row0 = ((H - y - 7.0) / 4.0) as usize;
            for row in row0..(row0 + 2).min(ih) {
                for col in (x / 4.0) as usize..((x1 / 4.0) as usize).min(iw) {
                    pixels[row * iw + col] = 20;
                }
            }
        }
        content.push_str(&format!(
            "q {W} 0 0 {H} {} {} cm /Im1 Do Q\n",
            setup.offset, setup.offset
        ));
        image = Some((iw, ih, pixels));
    }
    if !setup.image_only {
        let [a, b, c, d] = text_matrix(setup.rotate);
        content.push_str("BT\n");
        if setup.scan {
            content.push_str("3 Tr\n");
        }
        for (x, y, font, t) in layout() {
            let (ux, uy) = to_user(setup, x, y);
            content.push_str(&format!(
                "/{font} 10 Tf {a} {b} {c} {d} {ux} {uy} Tm ({t}) Tj\n"
            ));
        }
        content.push_str("ET\n");
    }
    let o = setup.offset;
    let media = format!("[0 0 {} {}]", mw + 2.0 * o, mh + 2.0 * o);
    let crop = format!("[{o} {o} {} {}]", mw + o, mh + o);
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox {media} /CropBox {crop} /Rotate {} \
             /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R >> /XObject << /Im1 8 0 R >> >> \
             /Contents 4 0 R >>",
            setup.rotate
        ),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique >>".to_string(),
    ];
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.drain(..).enumerate() {
        offsets.push(bytes.len());
        writeln!(&mut bytes, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
    }
    offsets.push(bytes.len());
    let (iw, ih, pixels) = image.unwrap_or((1, 1, vec![255]));
    write!(
        &mut bytes,
        "8 0 obj\n<< /Type /XObject /Subtype /Image /Width {iw} /Height {ih} \
         /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n",
        pixels.len()
    )
    .unwrap();
    bytes.extend_from_slice(&pixels);
    writeln!(&mut bytes, "\nendstream\nendobj").unwrap();
    let xref = bytes.len();
    writeln!(&mut bytes, "xref\n0 {}", offsets.len() + 1).unwrap();
    writeln!(&mut bytes, "0000000000 65535 f ").unwrap();
    for offset in offsets {
        writeln!(&mut bytes, "{offset:010} 00000 n ").unwrap();
    }
    writeln!(
        &mut bytes,
        "trailer\n<< /Size 9 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    )
    .unwrap();
    bytes
}

/// What a link to the theorem previews on a fixture page: the crop, the
/// lines inside it, and the share of the rendered crop that is ink.
struct Seen {
    rect: [f32; 4],
    size: [f32; 2],
    lines: Vec<String>,
    ink: f32,
}

fn preview_of(setup: Setup) -> Seen {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"));
    let b = pdfium.bindings();
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("page.pdf");
    std::fs::write(&path, fixture(setup)).expect("write fixture");
    let document = render::open_document(b, path.to_str().unwrap(), None).expect("open fixture");
    let size = render::page_display_size(b, document, 0).expect("page size");
    let [w, h] = size;
    let handle = b.FPDF_LoadPage(document, 0);
    assert!(!handle.is_null());
    let page: Page = prepare_page(
        Page::new(extract::extract(b, handle, w, h)),
        Default::default(),
    );
    let runs = text::extract_page(b, handle, w, h).runs;
    // The link's destination, in user space as hyperref writes it: just
    // over the theorem's head, mapped to the display as links.rs maps it.
    let (ux, uy) = to_user(setup, 72.0, 684.0);
    let mapper = text::DispMapper::new(b, handle, w, h);
    let (dx, dy) = mapper.map(f64::from(ux), f64::from(uy));
    let target = targets::link_preview(Some(&page), &runs, w, h, Some(dx), Some(dy), None);
    let rect = target.rect;
    let lines = (0..page.lines.len())
        .filter(|&i| {
            let l = &page.lines[i];
            let centre = (l.top + l.bottom) / 2.0;
            centre >= rect[1] && centre <= rect[1] + rect[3] && l.x0 < rect[0] + rect[2]
        })
        .map(|i| page.line_text(i))
        .collect();
    let tile = preview::rect_to_tile(rect, h, 1_000);
    let png = render::render_page(b, handle, size, 1.0, 0, Some(tile), false, || false)
        .expect("render")
        .png;
    let raster = image::load_from_memory(&png).expect("png").to_luma8();
    let dark = raster.pixels().filter(|p| p.0[0] < 128).count();
    let ink = dark as f32 / (raster.width() * raster.height()).max(1) as f32;
    b.FPDF_ClosePage(handle);
    b.FPDF_CloseDocument(document);
    Seen {
        rect,
        size,
        lines,
        ink,
    }
}

fn assert_on_theorem(setup: Setup) {
    let seen = preview_of(setup);
    let [w, h] = seen.size;
    let r = seen.rect;
    assert!(
        r[0] >= 0.0 && r[1] >= 0.0 && r[0] + r[2] <= w + 0.5 && r[1] + r[3] <= h + 0.5,
        "{setup:?}: crop {r:?} off the {w}x{h} page"
    );
    assert!(
        seen.lines
            .first()
            .is_some_and(|l| l.starts_with("Theorem 1")),
        "{setup:?}: {:?}",
        seen.lines
    );
    assert!(
        seen.lines.iter().any(|l| l.starts_with("subsequence")),
        "{setup:?}: {:?}",
        seen.lines
    );
    assert!(
        !seen.lines.iter().any(|l| l.starts_with("Proof")),
        "{setup:?}: {:?}",
        seen.lines
    );
    assert!(seen.ink > 0.01, "{setup:?}: a blank crop ({})", seen.ink);
}

#[test]
fn an_upright_page_previews_its_theorem() {
    assert_on_theorem(PLAIN);
}

#[test]
fn turned_pages_preview_the_same_theorem() {
    for rotate in [90, 180, 270] {
        assert_on_theorem(Setup { rotate, ..PLAIN });
    }
}

#[test]
fn an_offset_crop_box_shifts_nothing() {
    assert_on_theorem(Setup {
        offset: 90.0,
        ..PLAIN
    });
    assert_on_theorem(Setup {
        offset: 90.0,
        rotate: 90,
        ..PLAIN
    });
}

#[test]
fn a_scan_with_an_ocr_layer_previews_its_theorem() {
    assert_on_theorem(Setup {
        scan: true,
        ..PLAIN
    });
    assert_on_theorem(Setup {
        scan: true,
        offset: 40.0,
        ..PLAIN
    });
}

#[test]
fn an_image_only_scan_previews_ink_under_the_anchor() {
    let seen = preview_of(Setup {
        image_only: true,
        ..PLAIN
    });
    let r = seen.rect;
    // No text to read: the ordinary crop around the anchor, on the page,
    // over the scan's ink there.
    assert!(r[1] + r[3] >= 684.0 && r[1] <= 672.0, "{r:?}");
    assert!(seen.ink > 0.01, "a blank crop ({})", seen.ink);
}

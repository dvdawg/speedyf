//! Markup export: one annotated PDF to send back to an author.
//!
//! Everything happens on the SOURCE document itself, never on a rebuilt copy,
//! so its links, outline, metadata, structure tree and form all survive:
//!
//!   1. every page is widened on the right by `margin_pt`, in display space;
//!   2. each margin sheet (a one-page PDF captured from HTML, see
//!      `markup_capture`) is placed into its page's new strip as a form XObject;
//!   3. comment marks become real annotations (highlight, strikeout, square);
//!   4. feedback sheets are appended as pages;
//!   5. PDFium saves all of that as an incremental update, so the original
//!      bytes are kept verbatim at the start of the output;
//!   6. a second incremental update, written with lopdf, adds what PDFium's API
//!      cannot express: internal GoTo links carrying a /Dest, and Caret
//!      annotations. If that pass fails the export still succeeds, without
//!      links and with carets downgraded to text notes.
//!
//! The output then goes through the same reopen-verify and atomic rename every
//! save does (`save::verified_atomic_replace`).
//!
//! Coordinates arrive in the display-normalized page space the annotation
//! tools use (y-up, origin at the displayed crop-box bottom-left, pt);
//! `save::PageSpace` maps them to user space. Widening keeps that display
//! origin fixed, so the original content does not move.

use crate::engine::render;
use crate::engine::save::{verified_atomic_replace, PageSpace};
use crate::engine::types::RectDto;
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_ANNOTATION, FPDF_DOCUMENT, FPDF_FILEWRITE, FPDF_PAGE,
    FS_QUADPOINTSF, FS_RECTF,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::raw::{c_int, c_ulong, c_void};
use std::path::Path;

/// Widest margin accepted, in pt (20 inches).
pub const MAX_MARGIN_PT: f32 = 1440.0;
/// PDF's own page size limit.
const MAX_SHEET_PT: f32 = 14_400.0;
const MAX_SHEETS: usize = 5_000;
const MAX_MARKS: usize = 50_000;
const MAX_LINKS: usize = 50_000;
const MAX_HTML_BYTES: usize = 64 << 20;
const MAX_CONTENTS_CHARS: usize = 20_000;

const FPDF_INCREMENTAL: u32 = 1;
const ANNOT_TEXT: c_int = 1;
const ANNOT_SQUARE: c_int = 5;
const ANNOT_HIGHLIGHT: c_int = 9;
const ANNOT_STRIKEOUT: c_int = 12;
const ANNOT_FLAG_PRINT: c_int = 4;
const COLORTYPE_COLOR: u32 = 0;
const FILL_ALTERNATE: c_int = 1;
/// The rule between the page and its margin: light grey, half a point.
const RULE_GREY: u32 = 208;
const RULE_WIDTH: f32 = 0.5;

// ---------------------------------------------------------------------------
// Request / result (camelCase on the wire; see the markup contract)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarkupExportRequest {
    pub source: String,
    pub dest: String,
    pub margin_pt: f32,
    #[serde(default)]
    pub marks: Vec<MarkupMark>,
    #[serde(default)]
    pub sheets: Vec<MarkupSheet>,
    /// Extra CSS for the print page; only the capture reads it.
    #[serde(default)]
    pub css: Option<String>,
}

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct MarkupQuad {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub x3: f32,
    pub y3: f32,
    pub x4: f32,
    pub y4: f32,
}

impl MarkupQuad {
    fn points(&self) -> [(f32, f32); 4] {
        [
            (self.x1, self.y1),
            (self.x2, self.y2),
            (self.x3, self.y3),
            (self.x4, self.y4),
        ]
    }

    /// The quad's axis-aligned display bounds as `[l, b, r, t]`.
    fn bounds(&self) -> [f32; 4] {
        let pts = self.points();
        let mut out = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for (x, y) in pts {
            out[0] = out[0].min(x);
            out[1] = out[1].min(y);
            out[2] = out[2].max(x);
            out[3] = out[3].max(y);
        }
        out
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MarkKind {
    Highlight,
    Strikeout,
    Caret,
    Square,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarkupMark {
    pub page: u32,
    pub kind: MarkKind,
    #[serde(default)]
    pub quads: Option<Vec<MarkupQuad>>,
    #[serde(default)]
    pub rect: Option<RectDto>,
    pub color: String,
    #[serde(default)]
    pub contents: String,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SheetKind {
    Margin,
    Feedback,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarkupSheet {
    pub kind: SheetKind,
    /// The source page a margin sheet belongs to. Ignored for feedback.
    #[serde(default)]
    pub page: Option<u32>,
    pub width_pt: f32,
    pub height_pt: f32,
    pub html: String,
    #[serde(default)]
    pub links: Vec<SheetLink>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SheetLink {
    /// Sheet-relative, CSS px == pt, top-left origin.
    pub rect: RectDto,
    pub target: LinkTarget,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LinkTarget {
    /// `y` is pt from the sheet's top.
    Sheet {
        sheet: usize,
        #[serde(default)]
        y: f32,
    },
    /// `y` is display pt from the page's top.
    Page {
        page: u32,
        #[serde(default)]
        y: f32,
    },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarkupExportResult {
    pub path: String,
    /// Pages in the written file.
    pub pages: u32,
    /// Internal links written. Fewer than requested only when the link pass
    /// could not run on this file (it is then 0).
    pub links: u32,
}

// ---------------------------------------------------------------------------
// Validation (pure apart from the same-file check)
// ---------------------------------------------------------------------------

fn finite(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite())
}

fn rect_ok(r: &RectDto) -> bool {
    finite(&[r.x, r.y, r.w, r.h]) && r.w >= 0.0 && r.h >= 0.0
}

/// `#rrggbb` -> 0..255 channels.
pub(crate) fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Whether two paths name one file: by identity when both exist (catches
/// symlinks and hard links), by canonical path otherwise.
pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    let canon = |p: &Path| -> std::path::PathBuf {
        if let Ok(c) = p.canonicalize() {
            return c;
        }
        match (p.parent(), p.file_name()) {
            (Some(dir), Some(name)) => dir
                .canonicalize()
                .map(|d| d.join(name))
                .unwrap_or_else(|_| p.to_path_buf()),
            _ => p.to_path_buf(),
        }
    };
    canon(a) == canon(b)
}

/// Everything about a request that can be checked without opening the PDF.
/// Page indices are checked against the document on the worker.
pub fn validate(req: &MarkupExportRequest) -> AppResult<()> {
    let bad = |m: &str| Err(AppError::Unsupported(format!("markup export: {m}")));
    let source = Path::new(&req.source);
    let dest = Path::new(&req.dest);
    if !source.is_absolute() || !dest.is_absolute() {
        return bad("source and destination must be absolute paths");
    }
    if !source.is_file() {
        return Err(AppError::Io(format!("not a readable file: {}", req.source)));
    }
    if same_file(source, dest) {
        return bad("the destination must not be the source file");
    }
    if dest.is_dir() {
        return bad("the destination is a folder");
    }
    if !(req.margin_pt.is_finite() && req.margin_pt > 0.0 && req.margin_pt <= MAX_MARGIN_PT) {
        return bad("margin width out of range");
    }
    if req.sheets.len() > MAX_SHEETS || req.marks.len() > MAX_MARKS {
        return bad("too many sheets or marks");
    }
    let html_bytes: usize = req.sheets.iter().map(|s| s.html.len()).sum::<usize>()
        + req.css.as_ref().map_or(0, String::len);
    if html_bytes > MAX_HTML_BYTES {
        return bad("sheets too large");
    }
    let mut margin_pages = HashSet::new();
    let mut links = 0usize;
    for sheet in &req.sheets {
        let size_ok = |v: f32| v.is_finite() && v >= 1.0 && v <= MAX_SHEET_PT;
        if !size_ok(sheet.width_pt) || !size_ok(sheet.height_pt) {
            return bad("sheet size out of range");
        }
        if sheet.kind == SheetKind::Margin {
            let Some(page) = sheet.page else {
                return bad("a margin sheet needs a page");
            };
            if !margin_pages.insert(page) {
                return bad("two margin sheets for one page");
            }
        }
        links += sheet.links.len();
        for link in &sheet.links {
            if !rect_ok(&link.rect) {
                return bad("bad link rectangle");
            }
            match link.target {
                LinkTarget::Sheet { sheet, y } => {
                    if sheet >= req.sheets.len() || !y.is_finite() {
                        return bad("link to a sheet that does not exist");
                    }
                }
                LinkTarget::Page { y, .. } => {
                    if !y.is_finite() {
                        return bad("bad link target");
                    }
                }
            }
        }
    }
    if links > MAX_LINKS {
        return bad("too many links");
    }
    for mark in &req.marks {
        if parse_hex_color(&mark.color).is_none() {
            return bad("mark color must be #rrggbb");
        }
        if mark.contents.chars().count() > MAX_CONTENTS_CHARS {
            return bad("mark text too long");
        }
        let quads_ok = mark
            .quads
            .as_ref()
            .is_some_and(|q| !q.is_empty() && q.iter().all(|q| finite(&q.bounds())));
        let rect_given = mark.rect.as_ref().is_some_and(rect_ok);
        if mark.quads.as_ref().is_some_and(|q| !q.is_empty()) && !quads_ok {
            return bad("bad mark quads");
        }
        if !quads_ok && !rect_given {
            return bad("a mark needs quads or a rect");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Geometry (pure)
// ---------------------------------------------------------------------------

/// A box as `[left, bottom, right, top]` in user space.
pub(crate) type Bounds = [f32; 4];

/// Normalize `/Rotate` to 0, 90, 180 or 270.
pub(crate) fn normalize_rotation(deg: i64) -> u16 {
    (deg.rem_euclid(360) / 90 * 90) as u16
}

/// The box widened by `m` on the edge that DISPLAYS as its right edge.
///
/// Under `/Rotate` the displayed right edge is a different user-space edge:
/// 0 -> right, 90 -> top, 180 -> left, 270 -> bottom. The displayed origin
/// (bottom-left) is never on that edge, so nothing already on the page moves.
pub(crate) fn widen_display_right(b: Bounds, rot: u16, m: f32) -> Bounds {
    let [l, bo, r, t] = b;
    match rot {
        90 => [l, bo, r, t + m],
        180 => [l - m, bo, r, t],
        270 => [l, bo - m, r, t],
        _ => [l, bo, r + m, t],
    }
}

pub(crate) fn union(a: Bounds, b: Bounds) -> Bounds {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Displayed (width, height) of a crop box under a rotation.
pub(crate) fn display_size(crop: Bounds, rot: u16) -> (f32, f32) {
    let (w, h) = (crop[2] - crop[0], crop[3] - crop[1]);
    if rot == 90 || rot == 270 {
        (h, w)
    } else {
        (w, h)
    }
}

/// A PDF matrix `[a b c d e f]`: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    /// The matrix of an affine map, sampled at three points.
    pub(crate) fn sample(map: impl Fn(f32, f32) -> (f32, f32)) -> Affine {
        let (e, f) = map(0.0, 0.0);
        let (x1, y1) = map(1.0, 0.0);
        let (x2, y2) = map(0.0, 1.0);
        Affine {
            a: x1 - e,
            b: y1 - f,
            c: x2 - e,
            d: y2 - f,
            e,
            f,
        }
    }

    pub(crate) fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

/// One output page's display geometry, after widening.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageGeo {
    /// The (widened) crop box.
    pub crop: Bounds,
    pub rot: u16,
    /// Display width of the original content (the margin strip starts here).
    pub content_w: f32,
    /// Display height.
    pub h: f32,
}

impl PageGeo {
    fn space(&self) -> PageSpace {
        PageSpace::from_box(self.crop, self.rot)
    }

    /// A display-space rect as user-space bounds.
    fn rect_to_user(&self, r: &RectDto) -> Bounds {
        let ur = self.space().rect_to_user(r);
        [
            ur.left().value,
            ur.bottom().value,
            ur.right().value,
            ur.top().value,
        ]
    }

    /// A rect given top-left-origin relative to a box whose top-left sits at
    /// display `(x0, top of page)`, as user-space bounds.
    fn top_left_rect_to_user(&self, x0: f32, r: &RectDto) -> Bounds {
        self.rect_to_user(&RectDto {
            x: x0 + r.x,
            y: self.h - r.y - r.h,
            w: r.w,
            h: r.h,
        })
    }

    /// `/XYZ` left and top for a display point `y` pt below the page top.
    fn xyz(&self, y_from_top: f32) -> (f32, f32) {
        let y = (self.h - y_from_top).clamp(0.0, self.h);
        self.space().disp_to_user(0.0, y)
    }
}

/// The matrix placing a sheet page (user-space box `sheet_box`, unrotated)
/// into the margin strip of a widened page: sheet top-left at the strip's
/// top-left, 1 sheet pt == 1 display pt, upright on screen whatever the
/// page's `/Rotate`.
pub(crate) fn sheet_matrix(geo: &PageGeo, sheet_box: Bounds) -> Affine {
    let space = geo.space();
    let (sl, st) = (sheet_box[0], sheet_box[3]);
    Affine::sample(|sx, sy| space.disp_to_user(geo.content_w + (sx - sl), geo.h - (st - sy)))
}

/// Display corners UL, UR, LL, LR of a display box, mapped to user space in
/// that order, so a strikeout's line runs along the text even on a rotated
/// page.
fn quad_to_user(space: &PageSpace, b: [f32; 4]) -> FS_QUADPOINTSF {
    let [l, bo, r, t] = b;
    let (x1, y1) = space.disp_to_user(l, t);
    let (x2, y2) = space.disp_to_user(r, t);
    let (x3, y3) = space.disp_to_user(l, bo);
    let (x4, y4) = space.disp_to_user(r, bo);
    FS_QUADPOINTSF {
        x1,
        y1,
        x2,
        y2,
        x3,
        y3,
        x4,
        y4,
    }
}

fn quad_bounds(q: &FS_QUADPOINTSF) -> Bounds {
    let xs = [q.x1, q.x2, q.x3, q.x4];
    let ys = [q.y1, q.y2, q.y3, q.y4];
    [
        xs.iter().cloned().fold(f32::MAX, f32::min),
        ys.iter().cloned().fold(f32::MAX, f32::min),
        xs.iter().cloned().fold(f32::MIN, f32::max),
        ys.iter().cloned().fold(f32::MIN, f32::max),
    ]
}

// ---------------------------------------------------------------------------
// PDFium plumbing
// ---------------------------------------------------------------------------

struct DocGuard<'b> {
    b: &'b dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
}

impl Drop for DocGuard<'_> {
    fn drop(&mut self) {
        self.b.FPDF_CloseDocument(self.doc);
    }
}

struct PageGuard<'b> {
    b: &'b dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
}

impl Drop for PageGuard<'_> {
    fn drop(&mut self) {
        self.b.FPDF_ClosePage(self.page);
    }
}

struct AnnotGuard<'b> {
    b: &'b dyn PdfiumLibraryBindings,
    annot: FPDF_ANNOTATION,
}

impl Drop for AnnotGuard<'_> {
    fn drop(&mut self) {
        self.b.FPDFPage_CloseAnnot(self.annot);
    }
}

fn load_page<'b>(
    b: &'b dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    index: usize,
) -> AppResult<PageGuard<'b>> {
    let page = b.FPDF_LoadPage(doc, index as c_int);
    if page.is_null() {
        return Err(AppError::Malformed(format!("cannot load page {}", index + 1)));
    }
    Ok(PageGuard { b, page })
}

type BoxGetter = fn(
    &dyn PdfiumLibraryBindings,
    FPDF_PAGE,
    *mut f32,
    *mut f32,
    *mut f32,
    *mut f32,
) -> pdfium_render::prelude::FPDF_BOOL;

fn read_box(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE, get: BoxGetter) -> Option<Bounds> {
    let (mut l, mut bo, mut r, mut t) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let ok = get(b, page, &mut l, &mut bo, &mut r, &mut t) != 0;
    // Normalized: some writers store boxes with swapped corners.
    (ok && finite(&[l, bo, r, t]) && (r - l).abs() > 0.0 && (t - bo).abs() > 0.0)
        .then(|| [l.min(r), bo.min(t), l.max(r), bo.max(t)])
}

fn media_box(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> Option<Bounds> {
    read_box(b, page, |b, p, l, bo, r, t| b.FPDFPage_GetMediaBox(p, l, bo, r, t))
}

fn crop_box(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> Option<Bounds> {
    read_box(b, page, |b, p, l, bo, r, t| b.FPDFPage_GetCropBox(p, l, bo, r, t))
}

/// PDFium's effective page box: inherited boxes resolved.
fn bbox(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> Option<Bounds> {
    let mut rect = FS_RECTF {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    if b.FPDF_GetPageBoundingBox(page, &mut rect) == 0 {
        return None;
    }
    let v = [rect.left, rect.bottom, rect.right, rect.top];
    (finite(&v) && v[2] > v[0] && v[3] > v[1]).then_some(v)
}

/// The box display coordinates are measured from: the page's own crop box,
/// else its media box (as `save::PageSpace::of` reads them), else PDFium's
/// effective box for pages that inherit theirs.
fn display_box(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> AppResult<Bounds> {
    crop_box(b, page)
        .or_else(|| media_box(b, page))
        .or_else(|| bbox(b, page))
        .ok_or_else(|| AppError::Malformed("page has no usable box".into()))
}

fn page_rotation(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> u16 {
    normalize_rotation(b.FPDFPage_GetRotation(page).clamp(0, 3) as i64 * 90)
}

fn page_geo(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> AppResult<PageGeo> {
    let crop = display_box(b, page)?;
    let rot = page_rotation(b, page);
    let (w, h) = display_size(crop, rot);
    Ok(PageGeo {
        crop,
        rot,
        content_w: w,
        h,
    })
}

/// Widen one page and return its new geometry. Media and crop box are both
/// written explicitly, so an inherited box can no longer disagree; trim,
/// bleed and art boxes the page carries are widened alike.
fn widen_page(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE, m: f32) -> AppResult<PageGeo> {
    let crop = display_box(b, page)?;
    let rot = page_rotation(b, page);
    let media = media_box(b, page).unwrap_or(crop);
    let new_crop = widen_display_right(crop, rot, m);
    let new_media = union(media, new_crop);
    b.FPDFPage_SetMediaBox(page, new_media[0], new_media[1], new_media[2], new_media[3]);
    b.FPDFPage_SetCropBox(page, new_crop[0], new_crop[1], new_crop[2], new_crop[3]);
    let boxes: [(BoxGetter, fn(&dyn PdfiumLibraryBindings, FPDF_PAGE, f32, f32, f32, f32)); 3] = [
        (
            |b, p, l, bo, r, t| b.FPDFPage_GetTrimBox(p, l, bo, r, t),
            |b, p, l, bo, r, t| b.FPDFPage_SetTrimBox(p, l, bo, r, t),
        ),
        (
            |b, p, l, bo, r, t| b.FPDFPage_GetBleedBox(p, l, bo, r, t),
            |b, p, l, bo, r, t| b.FPDFPage_SetBleedBox(p, l, bo, r, t),
        ),
        (
            |b, p, l, bo, r, t| b.FPDFPage_GetArtBox(p, l, bo, r, t),
            |b, p, l, bo, r, t| b.FPDFPage_SetArtBox(p, l, bo, r, t),
        ),
    ];
    for (get, set) in boxes {
        if let Some(bx) = read_box(b, page, get) {
            let w = widen_display_right(bx, rot, m);
            set(b, page, w[0], w[1], w[2], w[3]);
        }
    }
    let (w, h) = display_size(crop, rot);
    Ok(PageGeo {
        crop: new_crop,
        rot,
        content_w: w,
        h,
    })
}

/// A filled, unstroked rectangle, in user space, inserted on top.
fn fill_rect(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    r: Bounds,
    rgb: [u32; 3],
) -> AppResult<()> {
    let obj = b.FPDFPageObj_CreateNewRect(r[0], r[1], r[2] - r[0], r[3] - r[1]);
    if obj.is_null() {
        return Err(AppError::Internal("cannot create a rectangle".into()));
    }
    b.FPDFPageObj_SetFillColor(obj, rgb[0], rgb[1], rgb[2], 255);
    b.FPDFPath_SetDrawMode(obj, FILL_ALTERNATE, 0);
    b.FPDFPage_InsertObject(page, obj);
    Ok(())
}

/// Paint the strip white (a crop box widened into a larger media box would
/// otherwise reveal whatever the original crop hid), then the sheet, then
/// the rule.
fn decorate_page(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    page: FPDF_PAGE,
    geo: &PageGeo,
    m: f32,
    sheet: Option<FPDF_DOCUMENT>,
) -> AppResult<()> {
    let strip = geo.rect_to_user(&RectDto {
        x: geo.content_w,
        y: 0.0,
        w: m,
        h: geo.h,
    });
    fill_rect(b, page, strip, [255, 255, 255])?;
    if let Some(sheet_doc) = sheet {
        let sheet_box = {
            let sp = load_page(b, sheet_doc, 0)?;
            media_box(b, sp.page)
                .or_else(|| bbox(b, sp.page))
                .ok_or_else(|| AppError::Malformed("sheet page has no box".into()))?
        };
        let xobject = b.FPDF_NewXObjectFromPage(doc, sheet_doc, 0);
        if xobject.is_null() {
            return Err(AppError::Internal("cannot import a margin sheet".into()));
        }
        let form = b.FPDF_NewFormObjectFromXObject(xobject);
        b.FPDF_CloseXObject(xobject);
        if form.is_null() {
            return Err(AppError::Internal("cannot place a margin sheet".into()));
        }
        let mx = sheet_matrix(geo, sheet_box);
        b.FPDFPageObj_Transform(
            form,
            mx.a as f64,
            mx.b as f64,
            mx.c as f64,
            mx.d as f64,
            mx.e as f64,
            mx.f as f64,
        );
        b.FPDFPage_InsertObject(page, form);
    }
    let rule = geo.rect_to_user(&RectDto {
        x: geo.content_w,
        y: 0.0,
        w: RULE_WIDTH,
        h: geo.h,
    });
    fill_rect(b, page, rule, [RULE_GREY; 3])
}

fn create_annot<'b>(
    b: &'b dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    subtype: c_int,
) -> AppResult<AnnotGuard<'b>> {
    let annot = b.FPDFPage_CreateAnnot(page, subtype as _);
    if annot.is_null() {
        return Err(AppError::Internal("cannot create an annotation".into()));
    }
    Ok(AnnotGuard { b, annot })
}

fn set_rect(b: &dyn PdfiumLibraryBindings, annot: FPDF_ANNOTATION, r: Bounds) {
    let rect = FS_RECTF {
        left: r[0],
        bottom: r[1],
        right: r[2],
        top: r[3],
    };
    b.FPDFAnnot_SetRect(annot, &rect);
}

/// A caret the lopdf pass writes, in user space.
#[derive(Clone, Debug)]
struct PendingCaret {
    page: usize,
    rect: Bounds,
    rgb: [u8; 3],
    contents: String,
}

/// A GoTo link the lopdf pass writes.
#[derive(Clone, Debug)]
struct PendingLink {
    page: usize,
    rect: Bounds,
    dest_page: usize,
    left: f32,
    top: f32,
}

/// The mark's display box: its quads' union, else its rect.
fn mark_display_box(mark: &MarkupMark) -> Option<Bounds> {
    let from_quads = mark
        .quads
        .as_ref()
        .filter(|q| !q.is_empty())
        .map(|qs| qs.iter().map(MarkupQuad::bounds).reduce(union).unwrap());
    from_quads.or_else(|| {
        mark.rect
            .as_ref()
            .map(|r| [r.x, r.y, r.x + r.w, r.y + r.h])
    })
}

fn add_mark(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    page_index: usize,
    geo: &PageGeo,
    mark: &MarkupMark,
    carets_as_notes: bool,
    carets: &mut Vec<PendingCaret>,
) -> AppResult<()> {
    let space = geo.space();
    let rgb = parse_hex_color(&mark.color).unwrap_or([0, 0, 0]);
    let Some(display) = mark_display_box(mark) else {
        return Ok(());
    };
    let to_user = |d: Bounds| {
        geo.rect_to_user(&RectDto {
            x: d[0],
            y: d[1],
            w: d[2] - d[0],
            h: d[3] - d[1],
        })
    };
    let annot = match mark.kind {
        MarkKind::Caret if !carets_as_notes => {
            // A caret marks a point, not a span: a small box at the end of
            // the marked text, as Acrobat draws one.
            let user = to_user(caret_box(display));
            carets.push(PendingCaret {
                page: page_index,
                rect: user,
                rgb,
                contents: mark.contents.clone(),
            });
            return Ok(());
        }
        MarkKind::Caret => {
            let a = create_annot(b, page, ANNOT_TEXT)?;
            let tip = caret_box(display);
            set_rect(b, a.annot, to_user([tip[0], tip[3] - 18.0, tip[0] + 18.0, tip[3]]));
            a
        }
        MarkKind::Square => {
            let a = create_annot(b, page, ANNOT_SQUARE)?;
            set_rect(b, a.annot, to_user(display));
            b.FPDFAnnot_SetBorder(a.annot, 0.0, 0.0, 1.5);
            a
        }
        MarkKind::Highlight | MarkKind::Strikeout => {
            let subtype = if mark.kind == MarkKind::Highlight {
                ANNOT_HIGHLIGHT
            } else {
                ANNOT_STRIKEOUT
            };
            let a = create_annot(b, page, subtype)?;
            let boxes: Vec<Bounds> = match mark.quads.as_ref().filter(|q| !q.is_empty()) {
                Some(qs) => qs.iter().map(MarkupQuad::bounds).collect(),
                None => vec![display],
            };
            let mut user: Option<Bounds> = None;
            for d in boxes {
                let q = quad_to_user(&space, d);
                b.FPDFAnnot_AppendAttachmentPoints(a.annot, &q);
                let qb = quad_bounds(&q);
                user = Some(user.map_or(qb, |u| union(u, qb)));
            }
            set_rect(b, a.annot, user.unwrap_or_else(|| to_user(display)));
            a
        }
    };
    b.FPDFAnnot_SetColor(
        annot.annot,
        COLORTYPE_COLOR as _,
        rgb[0] as u32,
        rgb[1] as u32,
        rgb[2] as u32,
        255,
    );
    if !mark.contents.is_empty() {
        b.FPDFAnnot_SetStringValue_str(annot.annot, "Contents", &mark.contents);
    }
    b.FPDFAnnot_SetFlags(annot.annot, ANNOT_FLAG_PRINT);
    Ok(())
}

/// A caret's box in display space: at the right end of the marked span,
/// along its baseline.
fn caret_box(display: Bounds) -> Bounds {
    let h = (display[3] - display[1]).clamp(4.0, 14.0);
    let w = h * 0.7;
    let x = display[2];
    [x - w / 2.0, display[1] - h * 0.35, x + w / 2.0, display[1] + h * 0.65]
}

#[repr(C)]
struct VecWrite {
    base: FPDF_FILEWRITE,
    out: Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    if this.is_null() {
        return 0;
    }
    // SAFETY: `this` is the `base` field of a live, #[repr(C)] VecWrite (its
    // first field), handed to PDFium only for the duration of the save.
    let writer = unsafe { &mut *(this as *mut VecWrite) };
    if size == 0 {
        return 1;
    }
    if data.is_null() {
        return 0;
    }
    // SAFETY: PDFium guarantees `data` points at `size` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
    writer.out.extend_from_slice(bytes);
    1
}

/// Save `doc` as an incremental update: the original file's bytes, verbatim,
/// followed by the changed and new objects and a new xref section.
pub(crate) fn save_incremental(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
) -> AppResult<Vec<u8>> {
    let mut writer = VecWrite {
        base: FPDF_FILEWRITE {
            version: 1,
            WriteBlock: Some(write_block),
        },
        out: Vec::new(),
    };
    let ok = b.FPDF_SaveAsCopy(doc, &mut writer.base, FPDF_INCREMENTAL as _);
    if ok == 0 || writer.out.is_empty() {
        return Err(AppError::Internal("PDFium could not save the document".into()));
    }
    Ok(writer.out)
}

/// What the PDFium pass produced: the file so far, plus the work left for the
/// lopdf pass.
struct PdfiumPass {
    bytes: Vec<u8>,
    pages: u32,
    links: Vec<PendingLink>,
    carets: Vec<PendingCaret>,
}

fn pdfium_pass(
    b: &dyn PdfiumLibraryBindings,
    req: &MarkupExportRequest,
    sheet_pdfs: &[Vec<u8>],
    carets_as_notes: bool,
) -> AppResult<PdfiumPass> {
    if sheet_pdfs.len() != req.sheets.len() {
        return Err(AppError::Internal(format!(
            "{} sheets captured for {} requested",
            sheet_pdfs.len(),
            req.sheets.len()
        )));
    }
    let doc = DocGuard {
        b,
        doc: render::open_document(b, &req.source, None)?,
    };
    let page_count = b.FPDF_GetPageCount(doc.doc).max(0) as usize;
    if page_count == 0 {
        return Err(AppError::Malformed("the document has no pages".into()));
    }
    let in_range = |p: u32| (p as usize) < page_count;
    if req.marks.iter().any(|m| !in_range(m.page))
        || req.sheets.iter().any(|s| {
            (s.kind == SheetKind::Margin && !s.page.is_some_and(in_range))
                || s.links.iter().any(|l| match l.target {
                    LinkTarget::Page { page, .. } => !in_range(page),
                    LinkTarget::Sheet { .. } => false,
                })
        })
    {
        return Err(AppError::Unsupported(
            "markup export: a page index is past the end of the document".into(),
        ));
    }

    let mut sheet_docs: Vec<DocGuard> = Vec::with_capacity(sheet_pdfs.len());
    for (i, bytes) in sheet_pdfs.iter().enumerate() {
        let sd = b.FPDF_LoadMemDocument(bytes, None);
        if sd.is_null() {
            return Err(AppError::Malformed(format!("sheet {} is not a PDF", i + 1)));
        }
        let guard = DocGuard { b, doc: sd };
        if b.FPDF_GetPageCount(sd) < 1 {
            return Err(AppError::Malformed(format!("sheet {} is empty", i + 1)));
        }
        sheet_docs.push(guard);
    }

    let margin_for: HashMap<usize, usize> = req
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.kind == SheetKind::Margin)
        .filter_map(|(i, s)| s.page.map(|p| (p as usize, i)))
        .collect();
    let mut marks_for: BTreeMap<usize, Vec<&MarkupMark>> = BTreeMap::new();
    for mark in &req.marks {
        marks_for.entry(mark.page as usize).or_default().push(mark);
    }

    let mut geos: Vec<PageGeo> = Vec::with_capacity(page_count);
    let mut carets = Vec::new();
    for index in 0..page_count {
        let page = load_page(b, doc.doc, index)?;
        let geo = widen_page(b, page.page, req.margin_pt)?;
        let sheet = margin_for.get(&index).map(|&s| sheet_docs[s].doc);
        decorate_page(b, doc.doc, page.page, &geo, req.margin_pt, sheet)?;
        for mark in marks_for.get(&index).into_iter().flatten() {
            add_mark(
                b,
                page.page,
                index,
                &geo,
                mark,
                carets_as_notes,
                &mut carets,
            )?;
        }
        if b.FPDFPage_GenerateContent(page.page) == 0 {
            return Err(AppError::Internal(format!(
                "cannot write page {}",
                index + 1
            )));
        }
        geos.push(geo);
    }

    // Feedback sheets, appended in request order.
    let mut out_page_of_sheet: HashMap<usize, usize> = margin_for
        .iter()
        .map(|(&page, &sheet)| (sheet, page))
        .collect();
    for (i, sheet) in req.sheets.iter().enumerate() {
        if sheet.kind != SheetKind::Feedback {
            continue;
        }
        let at = b.FPDF_GetPageCount(doc.doc).max(0);
        if b.FPDF_ImportPages(doc.doc, sheet_docs[i].doc, "1", at) == 0 {
            return Err(AppError::Internal("cannot append a feedback sheet".into()));
        }
        let page = load_page(b, doc.doc, at as usize)?;
        geos.push(page_geo(b, page.page)?);
        out_page_of_sheet.insert(i, at as usize);
    }
    drop(sheet_docs);

    // Links: rect on the sheet's output page, destination wherever it points.
    let mut links = Vec::new();
    for (i, sheet) in req.sheets.iter().enumerate() {
        let Some(&from) = out_page_of_sheet.get(&i) else {
            continue;
        };
        let from_geo = geos[from];
        let x0 = if sheet.kind == SheetKind::Margin {
            from_geo.content_w
        } else {
            0.0
        };
        for link in &sheet.links {
            let (dest_page, y) = match link.target {
                LinkTarget::Page { page, y } => (page as usize, y),
                LinkTarget::Sheet { sheet, y } => match out_page_of_sheet.get(&sheet) {
                    Some(&p) => (p, y),
                    None => continue,
                },
            };
            let (left, top) = geos[dest_page].xyz(y);
            links.push(PendingLink {
                page: from,
                rect: from_geo.top_left_rect_to_user(x0, &link.rect),
                dest_page,
                left,
                top,
            });
        }
    }

    let pages = b.FPDF_GetPageCount(doc.doc).max(0) as u32;
    let bytes = save_incremental(b, doc.doc)?;
    Ok(PdfiumPass {
        bytes,
        pages,
        links,
        carets,
    })
}

// ---------------------------------------------------------------------------
// The lopdf pass: GoTo links and carets, as a second incremental update
// ---------------------------------------------------------------------------

fn real_array(v: &[f32]) -> lopdf::Object {
    lopdf::Object::Array(v.iter().map(|&x| lopdf::Object::Real(x)).collect())
}

/// A PDF text string: PDFDocEncoding when ASCII, else UTF-16BE with a BOM.
fn text_string(s: &str) -> lopdf::Object {
    if s.is_ascii() {
        return lopdf::Object::string_literal(s.as_bytes().to_vec());
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in s.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    lopdf::Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

fn lopdf_err(e: lopdf::Error) -> String {
    e.to_string()
}

fn lopdf_pass(
    bytes: &[u8],
    links: &[PendingLink],
    carets: &[PendingCaret],
) -> Result<Vec<u8>, String> {
    use lopdf::{dictionary, Object};
    let mut inc: lopdf::IncrementalDocument = bytes.try_into().map_err(lopdf_err)?;
    let page_ids: Vec<lopdf::ObjectId> = inc
        .get_prev_documents()
        .get_pages()
        .values()
        .copied()
        .collect();
    let page_id = |i: usize| {
        page_ids
            .get(i)
            .copied()
            .ok_or_else(|| format!("page {i} not found by the link pass"))
    };

    let mut added: BTreeMap<usize, Vec<lopdf::ObjectId>> = BTreeMap::new();
    for link in links {
        let dict = dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => real_array(&link.rect),
            "Border" => vec![0.into(), 0.into(), 0.into()],
            "Dest" => vec![
                Object::Reference(page_id(link.dest_page)?),
                "XYZ".into(),
                Object::Real(link.left),
                Object::Real(link.top),
                Object::Null,
            ],
            "P" => Object::Reference(page_id(link.page)?),
        };
        let id = inc.new_document.add_object(dict);
        added.entry(link.page).or_default().push(id);
    }
    for caret in carets {
        let rgb: Vec<f32> = caret.rgb.iter().map(|&c| c as f32 / 255.0).collect();
        let mut dict = dictionary! {
            "Type" => "Annot",
            "Subtype" => "Caret",
            "Rect" => real_array(&caret.rect),
            "C" => real_array(&rgb),
            "F" => 4,
            "Sy" => "None",
            "P" => Object::Reference(page_id(caret.page)?),
        };
        if !caret.contents.is_empty() {
            dict.set("Contents", text_string(&caret.contents));
        }
        let id = inc.new_document.add_object(dict);
        added.entry(caret.page).or_default().push(id);
    }

    for (page, ids) in added {
        let pid = page_id(page)?;
        inc.opt_clone_object_to_new_document(pid).map_err(lopdf_err)?;
        let existing = inc
            .new_document
            .get_object(pid)
            .and_then(Object::as_dict)
            .map_err(lopdf_err)?
            .get(b"Annots")
            .ok()
            .cloned();
        let refs = ids.into_iter().map(Object::Reference);
        match existing {
            Some(Object::Reference(array_id)) => {
                inc.opt_clone_object_to_new_document(array_id)
                    .map_err(lopdf_err)?;
                inc.new_document
                    .get_object_mut(array_id)
                    .and_then(Object::as_array_mut)
                    .map_err(lopdf_err)?
                    .extend(refs);
            }
            Some(Object::Array(mut array)) => {
                array.extend(refs);
                set_annots(&mut inc, pid, Object::Array(array))?;
            }
            _ => set_annots(&mut inc, pid, Object::Array(refs.collect()))?,
        }
    }

    let mut out = Vec::with_capacity(bytes.len() + 4096);
    inc.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn set_annots(
    inc: &mut lopdf::IncrementalDocument,
    page: lopdf::ObjectId,
    annots: lopdf::Object,
) -> Result<(), String> {
    inc.new_document
        .get_object_mut(page)
        .and_then(lopdf::Object::as_dict_mut)
        .map_err(lopdf_err)?
        .set("Annots", annots);
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// The finished file's bytes and how many links it carries.
fn build(
    b: &dyn PdfiumLibraryBindings,
    req: &MarkupExportRequest,
    sheet_pdfs: &[Vec<u8>],
) -> AppResult<(Vec<u8>, u32, u32)> {
    let pass = pdfium_pass(b, req, sheet_pdfs, false)?;
    if pass.links.is_empty() && pass.carets.is_empty() {
        return Ok((pass.bytes, pass.pages, 0));
    }
    match lopdf_pass(&pass.bytes, &pass.links, &pass.carets) {
        Ok(bytes) => Ok((bytes, pass.pages, pass.links.len() as u32)),
        Err(e) => {
            log::warn!("markup export: link pass skipped: {e}");
            let fallback = pdfium_pass(b, req, sheet_pdfs, true)?;
            Ok((fallback.bytes, fallback.pages, 0))
        }
    }
}

/// Export `req` (with its sheets already captured, one PDF per sheet in
/// request order) to `req.dest`.
pub fn export(
    b: &dyn PdfiumLibraryBindings,
    req: &MarkupExportRequest,
    sheet_pdfs: &[Vec<u8>],
) -> AppResult<MarkupExportResult> {
    validate(req)?;
    let dest = Path::new(&req.dest);
    let summary = std::cell::Cell::new((0u32, 0u32));
    verified_atomic_replace(
        dest,
        |temp| {
            let (bytes, pages, links) = build(b, req, sheet_pdfs)?;
            std::fs::write(temp, &bytes)
                .map_err(|e| AppError::Io(format!("cannot write the export: {e}")))?;
            summary.set((pages, links));
            Ok(())
        },
        |temp| {
            let path = temp
                .to_str()
                .ok_or_else(|| AppError::Io("bad temp path".into()))?;
            let check = DocGuard {
                b,
                doc: render::open_document(b, path, None)?,
            };
            let count = b.FPDF_GetPageCount(check.doc).max(0) as u32;
            let expected = summary.get().0;
            if count != expected {
                return Err(AppError::Internal(format!(
                    "verification failed: wrote {count} pages, expected {expected}"
                )));
            }
            Ok(())
        },
        || Ok(()),
    )?;
    let (pages, links) = summary.get();
    Ok(MarkupExportResult {
        path: req.dest.clone(),
        pages,
        links,
    })
}

/// The document's permanent identifier (trailer `/ID[0]`) as lowercase hex,
/// when it has a usable one.
pub fn permanent_id(b: &dyn PdfiumLibraryBindings, path: &str) -> Option<(String, usize)> {
    let doc = DocGuard {
        b,
        doc: render::open_document(b, path, None).ok()?,
    };
    let len = b.FPDF_GetFileIdentifier(doc.doc, 0, std::ptr::null_mut(), 0) as usize;
    if len <= 1 {
        return None;
    }
    let mut buf = vec![0u8; len];
    b.FPDF_GetFileIdentifier(doc.doc, 0, buf.as_mut_ptr() as *mut c_void, len as c_ulong);
    buf.truncate(len - 1);
    let pages = b.FPDF_GetPageCount(doc.doc).max(0) as usize;
    // An identifier of one repeated byte is a writer's placeholder, shared by
    // every file it ever made; it identifies nothing.
    let degenerate = buf.len() < 4 || buf.iter().all(|&x| x == buf[0]);
    if degenerate {
        return None;
    }
    Some((buf.iter().map(|x| format!("{x:02x}")).collect(), pages))
}

#[cfg(test)]
mod tests;

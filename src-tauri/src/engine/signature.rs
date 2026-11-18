//! Visual signatures: a drawn, typed or imported mark placed on a page.
//!
//! A visual signature is a picture of a signature, not a certificate
//! signature - nothing here signs the file cryptographically. It is written as
//! a Stamp annotation whose appearance is plain PDF content:
//!
//! - drawn: the strokes as smoothed vector paths;
//! - typed: the name's glyph outlines from one of the bundled handwriting
//!   fonts, converted to filled paths, so no font is embedded and the mark
//!   looks the same in every viewer;
//! - image: an image XObject whose white background has been made
//!   transparent (an /SMask).
//!
//! The stamp also carries its artwork as JSON under a private key, which is
//! what lets a saved signature reopen as an editable signature instead of an
//! opaque stamp. "Flatten" writes the same objects into the page content
//! instead, for those signatures only: PDFium's own `FPDFPage_Flatten` would
//! flatten every annotation on the page and drop the rest of `/Annots` with
//! them.

use super::save::{parse_color, PageSpace};
use super::types::{PlanAnnot, PointDto, RectDto, SignatureArt};
use crate::errors::{AppError, AppResult};
use base64::Engine as _;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

/// `/T` of every signature stamp SpeedyF writes.
pub(crate) const SIGNATURE_CREATOR: &str = "SpeedyF Signature";
/// The annotation dictionary key holding the artwork JSON.
pub(crate) const SIGNATURE_KEY: &str = "SpeedyFSignature";
/// `/Contents`, for anyone reading the annotation list: says what it is.
const SIGNATURE_CONTENTS: &str = "Visual signature";

// The three typed styles. Shared with the frontend, which previews them with
// the same files through @font-face (src/assets/signature-fonts, OFL 1.1).
static FONT_FLOWING: &[u8] =
    include_bytes!("../../../src/assets/signature-fonts/GreatVibes-Regular.ttf");
static FONT_SCRIPT: &[u8] = include_bytes!("../../../src/assets/signature-fonts/DancingScript.ttf");
static FONT_HAND: &[u8] =
    include_bytes!("../../../src/assets/signature-fonts/Cedarville-Cursive.ttf");

/// Local units per em for typed signatures. Arbitrary - only the aspect ratio
/// of the box matters once it is placed - but fixed so outlines are stable.
const EM: f32 = 100.0;
/// Blank space kept around a typed name's ink, in local units.
const TYPED_PAD: f32 = 4.0;

fn font_for(style: &str) -> &'static [u8] {
    match style {
        "script" => FONT_SCRIPT,
        "hand" => FONT_HAND,
        _ => FONT_FLOWING,
    }
}

/// One path segment in a signature's local space (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A typed name as outlines, ready to draw.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TypedOutline {
    /// SVG path data in the box, y down.
    pub d: String,
    pub w: f32,
    pub h: f32,
    /// Characters the style has no glyph for, which were left out.
    pub missing: String,
    #[serde(skip)]
    pub(crate) segs: Vec<Seg>,
}

/// Collects glyph outlines in font units (y up), with the pen offset applied.
struct Collector {
    segs: Vec<Seg>,
    dx: f32,
    current: (f32, f32),
}

impl Collector {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (x + self.dx, y)
    }
}

impl ttf_parser::OutlineBuilder for Collector {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.segs.push(Seg::Move(x, y));
        self.current = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        self.segs.push(Seg::Line(x, y));
        self.current = (x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // A quadratic is an exact cubic with control points two thirds of the
        // way from each end to the quadratic's single control point.
        let (qx, qy) = self.at(x1, y1);
        let (x, y) = self.at(x, y);
        let (sx, sy) = self.current;
        self.segs.push(Seg::Cubic(
            sx + (qx - sx) * 2.0 / 3.0,
            sy + (qy - sy) * 2.0 / 3.0,
            x + (qx - x) * 2.0 / 3.0,
            y + (qy - y) * 2.0 / 3.0,
            x,
            y,
        ));
        self.current = (x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (ax, ay) = self.at(x1, y1);
        let (bx, by) = self.at(x2, y2);
        let (x, y) = self.at(x, y);
        self.segs.push(Seg::Cubic(ax, ay, bx, by, x, y));
        self.current = (x, y);
    }

    fn close(&mut self) {
        self.segs.push(Seg::Close);
    }
}

fn map_segs(segs: &[Seg], f: impl Fn(f32, f32) -> (f32, f32)) -> Vec<Seg> {
    segs.iter()
        .map(|seg| match *seg {
            Seg::Move(x, y) => {
                let (x, y) = f(x, y);
                Seg::Move(x, y)
            }
            Seg::Line(x, y) => {
                let (x, y) = f(x, y);
                Seg::Line(x, y)
            }
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                let (ax, ay) = f(ax, ay);
                let (bx, by) = f(bx, by);
                let (x, y) = f(x, y);
                Seg::Cubic(ax, ay, bx, by, x, y)
            }
            Seg::Close => Seg::Close,
        })
        .collect()
}

fn path_data(segs: &[Seg]) -> String {
    let mut d = String::new();
    for seg in segs {
        let part = match *seg {
            Seg::Move(x, y) => format!("M{x:.2} {y:.2}"),
            Seg::Line(x, y) => format!("L{x:.2} {y:.2}"),
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                format!("C{ax:.2} {ay:.2} {bx:.2} {by:.2} {x:.2} {y:.2}")
            }
            Seg::Close => "Z".to_string(),
        };
        d.push_str(&part);
    }
    d
}

/// `text` in `style`, as filled outlines in a tight box.
///
/// No shaping and no kerning: glyphs are set one after another at their
/// advance widths. The dialog previews with ligatures, contextual alternates
/// and kerning switched off, so what is chosen there is what gets placed.
pub fn typed_outline(text: &str, style: &str) -> AppResult<TypedOutline> {
    let face = ttf_parser::Face::parse(font_for(style), 0)
        .map_err(|e| AppError::Internal(format!("signature font unreadable: {e}")))?;
    let scale = EM / f32::from(face.units_per_em().max(1));
    let mut collector = Collector {
        segs: Vec::new(),
        dx: 0.0,
        current: (0.0, 0.0),
    };
    let mut missing = String::new();
    let mut pen = 0.0f32;
    for c in text.trim().chars() {
        let Some(glyph) = face.glyph_index(c) else {
            if !c.is_whitespace() && !missing.contains(c) {
                missing.push(c);
            }
            continue;
        };
        collector.dx = pen;
        face.outline_glyph(glyph, &mut collector);
        pen += f32::from(face.glyph_hor_advance(glyph).unwrap_or(0));
    }
    let segs = collector.segs;
    if segs.is_empty() {
        return Ok(TypedOutline {
            d: String::new(),
            w: 0.0,
            h: 0.0,
            missing,
            segs,
        });
    }

    // Bounds over every point, control points included: a curve never leaves
    // the hull of its control points, so this can only be a little generous.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut grow = |x: f32, y: f32| {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    };
    for seg in &segs {
        match *seg {
            Seg::Move(x, y) | Seg::Line(x, y) => grow(x, y),
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                grow(ax, ay);
                grow(bx, by);
                grow(x, y);
            }
            Seg::Close => {}
        }
    }
    // Font units, y up -> local units, y down, origin at the padded corner.
    let local = map_segs(&segs, |x, y| {
        ((x - x0) * scale + TYPED_PAD, (y1 - y) * scale + TYPED_PAD)
    });
    Ok(TypedOutline {
        d: path_data(&local),
        w: (x1 - x0) * scale + 2.0 * TYPED_PAD,
        h: (y1 - y0) * scale + 2.0 * TYPED_PAD,
        missing,
        segs: local,
    })
}

/// A drawn stroke as a smooth curve through the midpoints of its polyline.
///
/// Each captured point becomes the control point of a quadratic that runs from
/// the midpoint before it to the midpoint after it - the usual way to draw a
/// pointer trail without corners. The frontend draws the same curve
/// (`sign/art.ts`), so the page and the saved file agree.
pub(crate) fn stroke_segs(points: &[PointDto]) -> Vec<Seg> {
    let Some(first) = points.first() else {
        return Vec::new();
    };
    let mut segs = vec![Seg::Move(first.x, first.y)];
    if points.len() == 1 {
        // A dot: a hair of a line, which a round cap turns into a disc.
        segs.push(Seg::Line(first.x + 0.01, first.y));
        return segs;
    }
    if points.len() == 2 {
        segs.push(Seg::Line(points[1].x, points[1].y));
        return segs;
    }
    let mut current = (first.x, first.y);
    for i in 1..points.len() - 1 {
        let q = (points[i].x, points[i].y);
        let next = &points[i + 1];
        let end = if i + 1 == points.len() - 1 {
            (next.x, next.y)
        } else {
            ((q.0 + next.x) / 2.0, (q.1 + next.y) / 2.0)
        };
        segs.push(Seg::Cubic(
            current.0 + (q.0 - current.0) * 2.0 / 3.0,
            current.1 + (q.1 - current.1) * 2.0 / 3.0,
            end.0 + (q.0 - end.0) * 2.0 / 3.0,
            end.1 + (q.1 - end.1) * 2.0 / 3.0,
            end.0,
            end.1,
        ));
        current = end;
    }
    segs
}

/// Near-white pixels become transparent, and the picture is cropped to what
/// is left. A scan or phone photo of a signature on paper then sits on the
/// page as ink, not as a white rectangle.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PreparedImage {
    /// base64 PNG, RGBA.
    pub png: String,
    pub w: u32,
    pub h: u32,
}

/// Lightness at and above which a pixel is background.
const PAPER: f32 = 232.0;
/// Lightness at and below which a pixel is fully ink.
const INK: f32 = 150.0;
/// The longest side kept: plenty for a signature at any print size.
const MAX_SIDE: u32 = 1600;

pub fn prepare_image_bytes(bytes: &[u8]) -> AppResult<PreparedImage> {
    let image = image::load_from_memory(bytes)
        .map_err(|e| AppError::Unsupported(format!("cannot read that image: {e}")))?;
    let mut rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, pixel) in rgba.enumerate_pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let light = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
        let keep = ((PAPER - light) / (PAPER - INK)).clamp(0.0, 1.0);
        let alpha = (f32::from(a) * keep).round() as u8;
        pixel.0[3] = alpha;
        if alpha > 24 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 || y0 > y1 {
        return Err(AppError::Unsupported(
            "no signature found in that image: it looks blank".into(),
        ));
    }
    let pad = 4;
    let (cx, cy) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
    let cw = (x1 + pad + 1).min(width) - cx;
    let ch = (y1 + pad + 1).min(height) - cy;
    let mut cropped = image::imageops::crop_imm(&rgba, cx, cy, cw, ch).to_image();
    if cw.max(ch) > MAX_SIDE {
        let k = MAX_SIDE as f32 / cw.max(ch) as f32;
        let nw = ((cw as f32 * k).round() as u32).max(1);
        let nh = ((ch as f32 * k).round() as u32).max(1);
        cropped = image::imageops::resize(&cropped, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (w, h) = cropped.dimensions();
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(cropped)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| AppError::Internal(format!("cannot encode signature image: {e}")))?;
    Ok(PreparedImage {
        png: base64::engine::general_purpose::STANDARD.encode(out),
        w,
        h,
    })
}

pub fn prepare_image_file(path: &str) -> AppResult<PreparedImage> {
    let bytes =
        std::fs::read(path).map_err(|e| AppError::Io(format!("cannot read {path}: {e}")))?;
    prepare_image_bytes(&bytes)
}

/// What the stamp keeps under `SIGNATURE_KEY`.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Stored {
    v: u32,
    color: String,
    art: SignatureArt,
}

/// The artwork and ink colour of the signature stamp at `index`, if it is one
/// of SpeedyF's.
pub(crate) fn read_stored(
    bindings: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    index: u32,
) -> Option<(SignatureArt, String)> {
    let annot = bindings.FPDFPage_GetAnnot(page, index as i32);
    if annot.is_null() {
        return None;
    }
    let mut json = None;
    if bindings.is_true(bindings.FPDFAnnot_HasKey(annot, SIGNATURE_KEY)) {
        let len = bindings.FPDFAnnot_GetStringValue(annot, SIGNATURE_KEY, std::ptr::null_mut(), 0);
        if len > 2 {
            let mut buffer = vec![0u16; (len as usize).div_ceil(2)];
            bindings.FPDFAnnot_GetStringValue(annot, SIGNATURE_KEY, buffer.as_mut_ptr(), len);
            while buffer.last() == Some(&0) {
                buffer.pop();
            }
            json = Some(String::from_utf16_lossy(&buffer));
        }
    }
    bindings.FPDFPage_CloseAnnot(annot);
    let stored: Stored = serde_json::from_str(&json?).ok()?;
    Some((stored.art, stored.color))
}

/// Whether a signature form field already holds a signature (a `/V`), on the
/// widget itself or on the field it belongs to.
///
/// A visual signature is only offered for an empty field: placing a picture
/// over a real certificate signature would only hide what it says.
pub(crate) fn field_is_signed(
    bindings: &dyn PdfiumLibraryBindings,
    widget: FPDF_ANNOTATION,
) -> bool {
    if bindings.is_true(bindings.FPDFAnnot_HasKey(widget, "V")) {
        return true;
    }
    let parent = bindings.FPDFAnnot_GetLinkedAnnot(widget, "Parent");
    if parent.is_null() {
        return false;
    }
    let signed = bindings.is_true(bindings.FPDFAnnot_HasKey(parent, "V"));
    bindings.FPDFPage_CloseAnnot(parent);
    signed
}

/// Local signature space -> raw PDF user space for one placement.
struct Placement<'s> {
    space: &'s PageSpace,
    rect: RectDto,
    sx: f32,
    sy: f32,
}

impl Placement<'_> {
    fn new<'s>(space: &'s PageSpace, rect: RectDto, w: f32, h: f32) -> Placement<'s> {
        Placement {
            space,
            rect,
            sx: rect.w / w.max(1e-3),
            sy: rect.h / h.max(1e-3),
        }
    }

    fn user(&self, lx: f32, ly: f32) -> (f32, f32) {
        self.space.disp_to_user(
            self.rect.x + lx * self.sx,
            self.rect.y + self.rect.h - ly * self.sy,
        )
    }
}

/// The objects one signature is drawn with.
enum Built<'a> {
    Path(PdfPagePathObject<'a>),
    Image(PdfPageImageObject<'a>),
}

fn path_object<'a>(
    doc: &PdfDocument<'a>,
    segs: &[Seg],
    at: &Placement<'_>,
    stroke: Option<(PdfColor, f32)>,
    fill: Option<PdfColor>,
) -> AppResult<Option<PdfPagePathObject<'a>>> {
    let Some(Seg::Move(x, y)) = segs.first().copied() else {
        return Ok(None);
    };
    let (ux, uy) = at.user(x, y);
    let mut path = PdfPagePathObject::new(
        doc,
        PdfPoints::new(ux),
        PdfPoints::new(uy),
        stroke.map(|(color, _)| color),
        stroke.map(|(_, width)| PdfPoints::new(width)),
        fill,
    )?;
    for seg in &segs[1..] {
        match *seg {
            Seg::Move(x, y) => {
                let (x, y) = at.user(x, y);
                path.move_to(PdfPoints::new(x), PdfPoints::new(y))?;
            }
            Seg::Line(x, y) => {
                let (x, y) = at.user(x, y);
                path.line_to(PdfPoints::new(x), PdfPoints::new(y))?;
            }
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                let (ax, ay) = at.user(ax, ay);
                let (bx, by) = at.user(bx, by);
                let (x, y) = at.user(x, y);
                path.bezier_to(
                    PdfPoints::new(x),
                    PdfPoints::new(y),
                    PdfPoints::new(ax),
                    PdfPoints::new(ay),
                    PdfPoints::new(bx),
                    PdfPoints::new(by),
                )?;
            }
            Seg::Close => path.close_path()?,
        }
    }
    if stroke.is_some() {
        path.set_line_cap(PdfPageObjectLineCap::Round)?;
        path.set_line_join(PdfPageObjectLineJoin::Round)?;
    }
    if fill.is_some() {
        // TrueType outlines are drawn for the nonzero rule; even-odd would
        // punch holes where a script's strokes overlap themselves.
        path.set_fill_and_stroke_mode(PdfPathFillMode::Winding, stroke.is_some())?;
    }
    Ok(Some(path))
}

fn build<'a>(
    doc: &PdfDocument<'a>,
    space: &PageSpace,
    rect: RectDto,
    art: &SignatureArt,
    color: PdfColor,
) -> AppResult<Vec<Built<'a>>> {
    let mut out = Vec::new();
    match art {
        SignatureArt::Drawn {
            w,
            h,
            strokes,
            width,
        } => {
            let at = Placement::new(space, rect, *w, *h);
            let pen = (width * (at.sx * at.sy).sqrt()).max(0.25);
            for stroke in strokes {
                if let Some(path) =
                    path_object(doc, &stroke_segs(stroke), &at, Some((color, pen)), None)?
                {
                    out.push(Built::Path(path));
                }
            }
        }
        SignatureArt::Typed { text, style, .. } => {
            // Regenerated from the font rather than trusted from `d`: the
            // file then holds exactly what the font draws, whatever the
            // frontend happened to keep.
            let outline = typed_outline(text, style)?;
            let at = Placement::new(space, rect, outline.w, outline.h);
            if let Some(path) = path_object(doc, &outline.segs, &at, None, Some(color))? {
                out.push(Built::Path(path));
            }
        }
        SignatureArt::Image { png, .. } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(png)
                .map_err(|e| AppError::Malformed(format!("signature image: {e}")))?;
            let image = image::load_from_memory(&bytes)
                .map_err(|e| AppError::Malformed(format!("signature image: {e}")))?;
            let mut object = PdfPageImageObject::new(doc, &image)?;
            // The image fills its unit square; map that square onto the
            // displayed rectangle through the page's rotation, as text does.
            let at = Placement::new(space, rect, 1.0, 1.0);
            let (x0, y0) = at.user(0.0, 1.0);
            let (xx, xy) = at.user(1.0, 1.0);
            let (yx, yy) = at.user(0.0, 0.0);
            object.transform(xx - x0, xy - y0, yx - x0, yy - y0, x0, y0)?;
            out.push(Built::Image(object));
        }
    }
    Ok(out)
}

/// Write one signature from the edit plan: as a Stamp annotation, or into the
/// page content when the plan says to flatten it.
pub(crate) fn write<'a>(
    doc: &PdfDocument<'a>,
    page: &mut PdfPage<'a>,
    space: &PageSpace,
    annot: &PlanAnnot,
) -> AppResult<()> {
    let Some(art) = &annot.signature else {
        log::warn!("signature without artwork ignored at save");
        return Ok(());
    };
    let color = parse_color(&annot.color, 1.0);
    let built = build(doc, space, annot.rect, art, color)?;
    if built.is_empty() {
        return Ok(());
    }

    if annot.flatten {
        for object in built {
            match object {
                Built::Path(path) => {
                    page.objects_mut().add_path_object(path)?;
                }
                Built::Image(image) => {
                    page.objects_mut().add_image_object(image)?;
                }
            }
        }
        return Ok(());
    }

    {
        let mut stamp = page.annotations_mut().create_stamp_annotation()?;
        // Bounds before objects: PDFium sizes the appearance /BBox from the
        // rect at the moment an object is appended (see save.rs, "bubble").
        stamp.set_bounds(space.rect_to_user(&annot.rect))?;
        stamp.set_contents(SIGNATURE_CONTENTS)?;
        stamp.set_creator(SIGNATURE_CREATOR)?;
        stamp.set_is_printed(true)?;
        for object in built {
            match object {
                Built::Path(path) => {
                    stamp.objects_mut().add_path_object(path)?;
                }
                Built::Image(image) => {
                    stamp.objects_mut().add_image_object(image)?;
                }
            }
        }
    }

    // The artwork goes under a private key, through the raw API: pdfium-render
    // has no setter for arbitrary keys. The stamp just made is the last
    // annotation on the page.
    let stored = serde_json::to_string(&Stored {
        v: 1,
        color: annot.color.clone(),
        art: art.clone(),
    })
    .map_err(|e| AppError::Internal(format!("signature data: {e}")))?;
    let bindings = page.bindings();
    let handle = bindings.get_handle_from_page(page);
    let count = bindings.FPDFPage_GetAnnotCount(handle);
    let raw = bindings.FPDFPage_GetAnnot(handle, count - 1);
    if raw.is_null() {
        return Err(AppError::Internal("signature stamp vanished".into()));
    }
    let ok = bindings.FPDFAnnot_SetStringValue_str(raw, SIGNATURE_KEY, &stored);
    bindings.FPDFPage_CloseAnnot(raw);
    if !bindings.is_true(ok) {
        return Err(AppError::Internal("cannot store the signature data".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_outlines_a_latin_name() {
        for style in ["flowing", "script", "hand"] {
            let outline = typed_outline("Ada Lovelace", style).expect("outline");
            assert!(outline.missing.is_empty(), "{style}: {}", outline.missing);
            assert!(outline.w > outline.h, "{style}: a name is wider than tall");
            assert!(outline.d.starts_with('M'), "{style}");
            assert!(outline.d.contains('C') || outline.d.contains('L'));
            // Everything inside the box.
            for seg in &outline.segs {
                if let Seg::Move(x, y) | Seg::Line(x, y) = *seg {
                    assert!((0.0..=outline.w + 0.01).contains(&x), "{style} x {x}");
                    assert!((0.0..=outline.h + 0.01).contains(&y), "{style} y {y}");
                }
            }
        }
    }

    #[test]
    fn the_three_styles_are_different_fonts() {
        let a = typed_outline("Signature", "flowing").unwrap().d;
        let b = typed_outline("Signature", "script").unwrap().d;
        let c = typed_outline("Signature", "hand").unwrap().d;
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn characters_a_font_lacks_are_reported() {
        let outline = typed_outline("Ada \u{4e2d}", "flowing").unwrap();
        assert_eq!(outline.missing, "\u{4e2d}");
        assert!(!outline.d.is_empty());
        let empty = typed_outline("   ", "hand").unwrap();
        assert!(empty.d.is_empty());
    }

    #[test]
    fn strokes_are_smoothed_through_midpoints() {
        let p = |x: f32, y: f32| PointDto { x, y };
        let segs = stroke_segs(&[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(20.0, 10.0)]);
        assert_eq!(segs[0], Seg::Move(0.0, 0.0));
        // One curve per interior point, the first ending at a midpoint, the
        // last at the stroke's real end.
        assert_eq!(segs.len(), 3);
        assert!(matches!(segs[1], Seg::Cubic(_, _, _, _, x, y) if x == 10.0 && y == 5.0));
        assert!(matches!(segs[2], Seg::Cubic(_, _, _, _, x, y) if x == 20.0 && y == 10.0));
        assert_eq!(stroke_segs(&[p(1.0, 1.0)]).len(), 2);
        assert!(stroke_segs(&[]).is_empty());
    }

    fn png_of(image: image::RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn an_imported_image_loses_its_paper_and_is_cropped_to_ink() {
        // White paper, slightly off-white noise, one dark bar of "ink".
        let mut source = image::RgbaImage::from_pixel(200, 100, image::Rgba([250, 250, 248, 255]));
        source.put_pixel(5, 5, image::Rgba([240, 240, 240, 255]));
        for x in 60..140 {
            for y in 40..50 {
                source.put_pixel(x, y, image::Rgba([20, 30, 60, 255]));
            }
        }
        let prepared = prepare_image_bytes(&png_of(source)).expect("prepared");
        assert_eq!((prepared.w, prepared.h), (80 + 8, 10 + 8));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&prepared.png)
            .unwrap();
        let out = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "paper is transparent");
        let ink = out.get_pixel(prepared.w / 2, prepared.h / 2).0;
        assert_eq!(ink, [20, 30, 60, 255], "ink keeps its colour and opacity");
    }

    #[test]
    fn a_blank_image_is_refused() {
        let blank = image::RgbaImage::from_pixel(50, 50, image::Rgba([255, 255, 255, 255]));
        assert!(prepare_image_bytes(&png_of(blank)).is_err());
    }

    // ---------- save round-trips ----------

    use crate::engine::annots::read_annotations;
    use crate::engine::pdfium_init;
    use crate::engine::save::build_output;
    use crate::engine::types::{EditPlan, PlanPage};
    use std::path::Path;

    const PAGE_W: f32 = 300.0;
    const PAGE_H: f32 = 420.0;

    fn test_pdfium() -> Pdfium {
        Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
    }

    /// One page of text, optionally rotated.
    fn source_fixture(pdfium: &Pdfium, path: &Path, rotation: PdfPageRenderRotation) {
        let mut document = pdfium.create_new_pdf().expect("create fixture");
        let font = document.fonts_mut().helvetica();
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(PAGE_W),
                PdfPoints::new(PAGE_H),
            ))
            .expect("page");
        let mut text =
            PdfPageTextObject::new(&document, "Sign here", font, PdfPoints::new(12.0)).unwrap();
        text.translate(PdfPoints::new(30.0), PdfPoints::new(380.0))
            .unwrap();
        page.objects_mut().add_text_object(text).unwrap();
        page.set_rotation(rotation);
        document.save_to_file(path).expect("save fixture");
    }

    fn drawn() -> SignatureArt {
        let p = |x: f32, y: f32| PointDto { x, y };
        SignatureArt::Drawn {
            w: 200.0,
            h: 60.0,
            strokes: vec![
                vec![p(10.0, 40.0), p(40.0, 10.0), p(70.0, 50.0), p(110.0, 15.0)],
                vec![p(120.0, 30.0), p(190.0, 30.0)],
            ],
            width: 4.0,
        }
    }

    fn typed() -> SignatureArt {
        let outline = typed_outline("Ada Lovelace", "flowing").unwrap();
        SignatureArt::Typed {
            w: outline.w,
            h: outline.h,
            text: "Ada Lovelace".into(),
            style: "flowing".into(),
            d: outline.d,
        }
    }

    fn picture() -> SignatureArt {
        let mut ink = image::RgbaImage::from_pixel(80, 30, image::Rgba([0, 0, 0, 0]));
        for x in 5..75 {
            for y in 10..20 {
                ink.put_pixel(x, y, image::Rgba([10, 20, 40, 255]));
            }
        }
        SignatureArt::Image {
            w: 80.0,
            h: 30.0,
            png: base64::engine::general_purpose::STANDARD.encode(png_of(ink)),
        }
    }

    fn placed(art: SignatureArt, rect: RectDto, flatten: bool) -> PlanAnnot {
        PlanAnnot {
            kind: "signature".into(),
            rect,
            color: "#1a1a1a".into(),
            opacity: 1.0,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
            signature: Some(art),
            flatten,
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> RectDto {
        RectDto { x, y, w, h }
    }

    fn page_plan(annots: Vec<PlanAnnot>, drop: Vec<u32>) -> EditPlan {
        EditPlan {
            pages: vec![PlanPage {
                src_index: Some(0),
                width_pt: PAGE_W,
                height_pt: PAGE_H,
                rotation: 0,
                annots,
                texts: Vec::new(),
                images: Vec::new(),
                drop_src_annots: drop,
            }],
            form: Vec::new(),
            form_buttons: Vec::new(),
            form_choices: Vec::new(),
            form_display: Vec::new(),
        }
    }

    fn save(pdfium: &Pdfium, source: &Path, output: &Path, plan: &EditPlan) {
        build_output(pdfium, source.to_str().unwrap(), None, plan, output).expect("save");
    }

    /// Dark pixels inside a display-space rect of an upright page rendered at
    /// one pixel per point. Statistics only; nothing is looked at.
    fn ink_in(page: &PdfPage<'_>, r: RectDto) -> usize {
        let (w, h) = (page.width().value as i32, page.height().value as i32);
        let bitmap = page.render(w, h, None).expect("render");
        let rgba = bitmap.as_rgba_bytes();
        let mut dark = 0;
        let y_top = (h as f32 - r.y - r.h).max(0.0) as i32;
        let y_bottom = (h as f32 - r.y).min(h as f32) as i32;
        for y in y_top..y_bottom {
            for x in (r.x as i32).max(0)..((r.x + r.w) as i32).min(w) {
                let i = ((y * w + x) * 4) as usize;
                if rgba[i] < 128 && rgba[i + 1] < 128 && rgba[i + 2] < 128 {
                    dark += 1;
                }
            }
        }
        dark
    }

    /// The normal appearance stream of every annotation on page 0.
    fn appearances(pdfium: &Pdfium, document: &PdfDocument<'_>) -> Vec<(i32, String)> {
        let bindings = pdfium.bindings();
        let page = document.pages().get(0).unwrap();
        let handle = bindings.get_handle_from_page(&page);
        let mut out = Vec::new();
        for index in 0..bindings.FPDFPage_GetAnnotCount(handle) {
            let annot = bindings.FPDFPage_GetAnnot(handle, index);
            let subtype = bindings.FPDFAnnot_GetSubtype(annot);
            let len = bindings.FPDFAnnot_GetAP(annot, 0, std::ptr::null_mut(), 0);
            let mut buffer = vec![0u16; (len as usize).div_ceil(2).max(1)];
            bindings.FPDFAnnot_GetAP(annot, 0, buffer.as_mut_ptr(), len);
            while buffer.last() == Some(&0) {
                buffer.pop();
            }
            out.push((subtype, String::from_utf16_lossy(&buffer)));
            bindings.FPDFPage_CloseAnnot(annot);
        }
        out
    }

    const FPDF_ANNOT_STAMP: i32 = 13;

    #[test]
    fn placed_signatures_save_as_stamps_with_appearances_and_reopen_as_signatures() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let (source, output) = (dir.path().join("in.pdf"), dir.path().join("out.pdf"));
        source_fixture(&pdfium, &source, PdfPageRenderRotation::None);

        let (r_drawn, r_typed, r_image) = (
            rect(30.0, 60.0, 150.0, 45.0),
            rect(30.0, 140.0, 160.0, 40.0),
            rect(30.0, 220.0, 120.0, 45.0),
        );
        save(
            &pdfium,
            &source,
            &output,
            &page_plan(
                vec![
                    placed(drawn(), r_drawn, false),
                    placed(typed(), r_typed, false),
                    placed(picture(), r_image, false),
                ],
                Vec::new(),
            ),
        );

        // The file: three Stamps, each with a real appearance stream of the
        // right kind - curves for strokes, a filled outline for the name, an
        // image XObject for the picture - and a soft mask for its alpha.
        let bytes = std::fs::read(&output).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Subtype /Stamp") || text.contains("/Subtype/Stamp"));
        assert!(
            text.contains("/SMask"),
            "transparent picture keeps its alpha"
        );
        assert!(text.contains("SpeedyFSignature"));
        let document = pdfium.load_pdf_from_file(&output, None).unwrap();
        let aps = appearances(&pdfium, &document);
        assert_eq!(aps.len(), 3);
        assert!(aps.iter().all(|(subtype, _)| *subtype == FPDF_ANNOT_STAMP));
        let (_, drawn_ap) = &aps[0];
        assert!(
            drawn_ap.contains(" c\n") || drawn_ap.contains(" c "),
            "{drawn_ap}"
        );
        assert!(
            drawn_ap.contains("1 J") && drawn_ap.contains("1 j"),
            "round caps"
        );
        assert!(drawn_ap.contains('S'), "stroked");
        let (_, typed_ap) = &aps[1];
        assert!(typed_ap.contains(" c"), "outlines are curves");
        assert!(typed_ap.contains('f'), "outlines are filled");
        assert!(
            !typed_ap.contains("Tj") && !typed_ap.contains("BT"),
            "no text, no font"
        );
        let (_, image_ap) = &aps[2];
        assert!(image_ap.contains("Do"), "{image_ap}");

        // On the page: every signature puts ink inside its own box.
        let page = document.pages().get(0).unwrap();
        for r in [r_drawn, r_typed, r_image] {
            assert!(ink_in(&page, r) > 50, "no ink in {r:?}");
        }
        drop(page);
        drop(document);

        // Reading back: signatures again, artwork intact.
        let pages = read_annotations(&pdfium, output.to_str().unwrap(), None).unwrap();
        let annots = &pages[0].annots;
        assert_eq!(annots.len(), 3);
        assert!(annots.iter().all(|a| a.kind == "signature"));
        assert!(matches!(
            annots[0].signature,
            Some(SignatureArt::Drawn { .. })
        ));
        assert!(
            matches!(&annots[1].signature, Some(SignatureArt::Typed { text, style, .. })
                if text == "Ada Lovelace" && style == "flowing")
        );
        assert!(matches!(
            annots[2].signature,
            Some(SignatureArt::Image { .. })
        ));
        for (a, r) in annots.iter().zip([r_drawn, r_typed, r_image]) {
            assert_eq!(a.color, "#1a1a1a");
            assert!((a.rect.x - r.x).abs() < 0.5 && (a.rect.y - r.y).abs() < 0.5);
            assert!((a.rect.w - r.w).abs() < 0.5 && (a.rect.h - r.h).abs() < 0.5);
        }
    }

    #[test]
    fn a_reopened_signature_moves_without_leaving_a_copy_behind() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let (source, once, twice) = (
            dir.path().join("in.pdf"),
            dir.path().join("once.pdf"),
            dir.path().join("twice.pdf"),
        );
        source_fixture(&pdfium, &source, PdfPageRenderRotation::None);
        let first = rect(30.0, 60.0, 150.0, 45.0);
        save(
            &pdfium,
            &source,
            &once,
            &page_plan(vec![placed(typed(), first, false)], Vec::new()),
        );
        let read = read_annotations(&pdfium, once.to_str().unwrap(), None).unwrap();
        let index = read[0].annots[0].index;
        let moved = rect(100.0, 250.0, 150.0, 45.0);
        let art = read[0].annots[0].signature.clone().unwrap();
        save(
            &pdfium,
            &once,
            &twice,
            &page_plan(vec![placed(art, moved, false)], vec![index]),
        );
        let read = read_annotations(&pdfium, twice.to_str().unwrap(), None).unwrap();
        assert_eq!(read[0].annots.len(), 1);
        assert!((read[0].annots[0].rect.x - moved.x).abs() < 0.5);
        let document = pdfium.load_pdf_from_file(&twice, None).unwrap();
        let page = document.pages().get(0).unwrap();
        assert_eq!(ink_in(&page, first), 0, "nothing left where it was");
        assert!(ink_in(&page, moved) > 50);
    }

    #[test]
    fn flattening_writes_signatures_into_the_page_and_no_annotation() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let (source, output) = (dir.path().join("in.pdf"), dir.path().join("out.pdf"));
        source_fixture(&pdfium, &source, PdfPageRenderRotation::None);
        let before = {
            let document = pdfium.load_pdf_from_file(&source, None).unwrap();
            let page = document.pages().get(0).unwrap();
            page.objects().len()
        };
        let (r_drawn, r_typed, r_image) = (
            rect(30.0, 60.0, 150.0, 45.0),
            rect(30.0, 140.0, 160.0, 40.0),
            rect(30.0, 220.0, 120.0, 45.0),
        );
        save(
            &pdfium,
            &source,
            &output,
            &page_plan(
                vec![
                    placed(drawn(), r_drawn, true),
                    placed(typed(), r_typed, true),
                    placed(picture(), r_image, true),
                ],
                Vec::new(),
            ),
        );

        let document = pdfium.load_pdf_from_file(&output, None).unwrap();
        let page = document.pages().get(0).unwrap();
        assert_eq!(page.annotations().len(), 0, "flattened: no annotation");
        let objects: Vec<_> = page.objects().iter().collect();
        let paths = objects
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Path)
            .count();
        let images = objects
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Image)
            .count();
        // Two drawn strokes and one outline, plus the picture.
        assert_eq!(objects.len(), before + 4);
        assert_eq!((paths, images), (3, 1));
        for r in [r_drawn, r_typed, r_image] {
            assert!(ink_in(&page, r) > 50, "no ink in {r:?}");
        }
        drop(objects);
        drop(page);
        drop(document);
        let read = read_annotations(&pdfium, output.to_str().unwrap(), None).unwrap();
        assert!(read.is_empty());
    }

    #[test]
    fn an_imported_picture_is_placed_upright() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let (source, output) = (dir.path().join("in.pdf"), dir.path().join("out.pdf"));
        source_fixture(&pdfium, &source, PdfPageRenderRotation::None);
        // Ink in the top half of the picture only.
        let mut ink = image::RgbaImage::from_pixel(40, 40, image::Rgba([0, 0, 0, 0]));
        for x in 0..40 {
            for y in 0..20 {
                ink.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let art = SignatureArt::Image {
            w: 40.0,
            h: 40.0,
            png: base64::engine::general_purpose::STANDARD.encode(png_of(ink)),
        };
        let r = rect(50.0, 100.0, 80.0, 80.0);
        save(
            &pdfium,
            &source,
            &output,
            &page_plan(vec![placed(art, r, false)], Vec::new()),
        );
        let document = pdfium.load_pdf_from_file(&output, None).unwrap();
        let page = document.pages().get(0).unwrap();
        let top = ink_in(&page, rect(50.0, 145.0, 80.0, 30.0));
        let bottom = ink_in(&page, rect(50.0, 105.0, 80.0, 30.0));
        assert!(top > 1000 && bottom == 0, "top {top}, bottom {bottom}");
    }

    #[test]
    fn a_signature_on_a_rotated_page_reads_back_where_it_was_placed() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let (source, output) = (dir.path().join("in.pdf"), dir.path().join("out.pdf"));
        source_fixture(&pdfium, &source, PdfPageRenderRotation::Degrees90);
        // Display space of a 90-degree page is the page turned on its side.
        let r = rect(40.0, 50.0, 160.0, 48.0);
        let mut plan = page_plan(vec![placed(typed(), r, false)], Vec::new());
        plan.pages[0].width_pt = PAGE_H;
        plan.pages[0].height_pt = PAGE_W;
        save(&pdfium, &source, &output, &plan);
        let read = read_annotations(&pdfium, output.to_str().unwrap(), None).unwrap();
        let back = read[0].annots[0].rect;
        assert!(
            (back.x - r.x).abs() < 0.5 && (back.y - r.y).abs() < 0.5,
            "{back:?}"
        );
        assert!(
            (back.w - r.w).abs() < 0.5 && (back.h - r.h).abs() < 0.5,
            "{back:?}"
        );
    }
}

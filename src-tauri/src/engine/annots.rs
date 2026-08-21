//! Reading annotations back out of a PDF.
//!
//! The counterpart to `save.rs`, and the reason annotating a paper stopped
//! being one-way. Renders have always included annotations — `render.rs` passes
//! `FPDF_ANNOT` — so a file's existing markup was *visible*; it was simply
//! unreachable, because nothing turned it back into something the editor could
//! select, move or delete.
//!
//! Only the subtypes `save.rs` knows how to write are read. Everything else —
//! links, form widgets, popups, XFA, redactions — is skipped here and left
//! untouched by the save path too. Adopting an annotation this model cannot
//! represent would mean silently degrading it the next time the file was saved.
//!
//! Coordinates come back through `PageSpace::user_to_disp`, the exact inverse of
//! what writing applies. Both directions live in `save.rs` so they cannot drift.

use super::save::PageSpace;
use super::types::{AnnotationDto, PageAnnotationsDto, PointDto, QuadDto, RectDto};
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::*;

/// Default stroke width for annotations that do not report one.
const DEFAULT_STROKE_PT: f32 = 2.0;
/// Default size for text SpeedyF did not write itself.
const DEFAULT_FONT_PT: f32 = 14.0;

fn hex_of(color: &PdfColor) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color.red(),
        color.green(),
        color.blue()
    )
}

/// Alpha as an opacity, treating a fully transparent colour as opaque.
///
/// Plenty of writers leave alpha at zero and express transparency through the
/// graphics state instead. Taking that literally would make a highlight vanish
/// the moment it was re-saved, so a degenerate value falls back to opaque.
fn opacity_of(color: &PdfColor) -> f32 {
    match color.alpha() {
        0 => 1.0,
        alpha => f32::from(alpha) / 255.0,
    }
}

/// A colour and opacity, or the fallback when the annotation carries none.
///
/// A missing colour is common, not an error: writers routinely leave it to the
/// appearance stream. Failing the read over one would lose every other
/// annotation on the page.
fn color_or(found: Result<PdfColor, PdfiumError>, fallback: &str) -> (String, f32) {
    match found {
        Ok(color) => (hex_of(&color), opacity_of(&color)),
        Err(_) => (fallback.to_string(), 1.0),
    }
}

/// A highlight's quads, in display space.
///
/// Highlights carry attachment points — one quad per line of text covered —
/// rather than one rectangle, which is what lets a highlight follow a sentence
/// across a line break instead of boxing the whole paragraph.
fn quads_of(points: &PdfPageAnnotationAttachmentPoints<'_>, space: &PageSpace) -> Vec<QuadDto> {
    let point = |x: f32, y: f32| {
        let (dx, dy) = space.user_to_disp(x, y);
        PointDto { x: dx, y: dy }
    };
    points
        .iter()
        .map(|quad| QuadDto {
            p1: point(quad.x1.value, quad.y1.value),
            p2: point(quad.x2.value, quad.y2.value),
            p3: point(quad.x3.value, quad.y3.value),
            p4: point(quad.x4.value, quad.y4.value),
        })
        .collect()
}

/// A rectangle as a single quad, for a highlight that carries no attachment
/// points. Degenerate, but still selectable — better than dropping it.
fn quad_of_rect(rect: &RectDto) -> QuadDto {
    let (l, r) = (rect.x, rect.x + rect.w);
    let (b, t) = (rect.y, rect.y + rect.h);
    QuadDto {
        p1: PointDto { x: l, y: t },
        p2: PointDto { x: r, y: t },
        p3: PointDto { x: l, y: b },
        p4: PointDto { x: r, y: b },
    }
}

/// The colour and width of an ink annotation, taken from its path objects.
///
/// Not from the annotation itself: `save.rs` deliberately never sets a stroke
/// colour on an annotation holding appended objects, because doing so crashes
/// PDFium. The style lives on the paths, so that is where it is read from.
fn ink_style(objects: &PdfPageAnnotationObjects<'_>) -> (Option<String>, Option<f32>) {
    for object in objects.iter() {
        let Some(path) = object.as_path_object() else {
            continue;
        };
        let color = path.stroke_color().ok().map(|c| hex_of(&c));
        let width = path.stroke_width().ok().map(|w| w.value);
        if color.is_some() || width.is_some() {
            return (color, width);
        }
    }
    (None, None)
}

/// The polylines inside an ink annotation.
///
/// Ink is written as path objects living *within* the annotation, so reading it
/// back means walking those objects' segments. A `MoveTo` starts a new stroke;
/// every other segment extends the current one. Only segment endpoints are
/// recovered, which is exact for the polylines SpeedyF writes and an
/// approximation of curves drawn elsewhere.
fn strokes_of(objects: &PdfPageAnnotationObjects<'_>, space: &PageSpace) -> Vec<Vec<PointDto>> {
    let mut strokes: Vec<Vec<PointDto>> = Vec::new();
    let mut current: Vec<PointDto> = Vec::new();

    for object in objects.iter() {
        let Some(path) = object.as_path_object() else {
            continue;
        };
        let segments = path.segments();
        for segment in segments.iter() {
            if segment.segment_type() == PdfPathSegmentType::MoveTo && !current.is_empty() {
                strokes.push(std::mem::take(&mut current));
            }
            let (x, y) = space.user_to_disp(segment.x().value, segment.y().value);
            current.push(PointDto { x, y });
        }
        if !current.is_empty() {
            strokes.push(std::mem::take(&mut current));
        }
    }

    // A single point is not a stroke; it would render as nothing and grab as
    // nothing.
    strokes.retain(|stroke| stroke.len() >= 2);
    strokes
}

/// One annotation, or nothing if it is a kind SpeedyF does not own.
fn read_annotation(
    annot: &PdfPageAnnotation<'_>,
    index: u32,
    space: &PageSpace,
) -> Option<AnnotationDto> {
    let rect = space.rect_from_user(&annot.bounds().ok()?);
    let mut dto = AnnotationDto {
        index,
        kind: String::new(),
        rect,
        color: "#000000".to_string(),
        opacity: 1.0,
        stroke_width: None,
        quads: None,
        strokes: None,
        text: None,
        font_size_pt: None,
    };

    match annot.annotation_type() {
        PdfPageAnnotationType::Highlight => {
            let (color, opacity) = color_or(annot.fill_color(), "#ffd54a");
            dto.kind = "highlight".into();
            dto.color = color;
            dto.opacity = opacity;
            let quads = quads_of(annot.attachment_points(), space);
            dto.quads = Some(if quads.is_empty() {
                vec![quad_of_rect(&rect)]
            } else {
                quads
            });
        }
        PdfPageAnnotationType::Square => {
            let (color, opacity) = color_or(annot.stroke_color(), "#1e88e5");
            dto.kind = "rect".into();
            dto.color = color;
            dto.opacity = opacity;
            dto.stroke_width = Some(DEFAULT_STROKE_PT);
        }
        PdfPageAnnotationType::Text => {
            let (color, opacity) = color_or(annot.fill_color(), "#fb8c00");
            dto.kind = "note".into();
            dto.color = color;
            dto.opacity = opacity;
            dto.text = Some(annot.contents().unwrap_or_default());
        }
        PdfPageAnnotationType::Ink => {
            let strokes = strokes_of(annot.objects(), space);
            // Nothing to draw and nothing to grab.
            if strokes.is_empty() {
                return None;
            }
            let (path_color, path_width) = ink_style(annot.objects());
            // The annotation's own colour is the fallback, for ink drawn by a
            // tool that does put one there.
            let (color, opacity) = color_or(annot.stroke_color(), "#e53935");
            dto.kind = "ink".into();
            dto.color = path_color.unwrap_or(color);
            dto.opacity = opacity;
            dto.stroke_width = Some(path_width.unwrap_or(DEFAULT_STROKE_PT));
            dto.strokes = Some(strokes);
        }
        PdfPageAnnotationType::FreeText => {
            let (color, opacity) = color_or(annot.fill_color(), "#d81b60");
            dto.kind = "textbox".into();
            dto.color = color;
            dto.opacity = opacity;
            dto.text = Some(annot.contents().unwrap_or_default());
            dto.font_size_pt = Some(DEFAULT_FONT_PT);
        }
        PdfPageAnnotationType::Stamp => {
            // The pixels stay in the file; the editor gets a movable box. It
            // cannot re-encode the image, so a stamp is only ever rewritten
            // when the user moves it.
            dto.kind = "image".into();
        }
        // Links, widgets, popups, XFA, redactions, anything unrecognized:
        // still rendered by PDFium, and strictly left alone.
        _ => return None,
    }

    Some(dto)
}

/// Whether an annotation is one SpeedyF represents, and therefore one the save
/// path may replace.
///
/// Deliberately beside the reader: the two must agree exactly. If save owned a
/// subtype the reader skipped, saving would delete an annotation the user was
/// never given an editable copy of.
pub fn is_owned(annot: &PdfPageAnnotation<'_>) -> bool {
    matches!(
        annot.annotation_type(),
        PdfPageAnnotationType::Highlight
            | PdfPageAnnotationType::Square
            | PdfPageAnnotationType::Text
            | PdfPageAnnotationType::Ink
            | PdfPageAnnotationType::FreeText
            | PdfPageAnnotationType::Stamp
    )
}

/// Every annotation SpeedyF owns, page by page.
///
/// Reads the file rather than the open render document, matching `save.rs`:
/// both sides of the round-trip then see the same bytes, and neither is
/// affected by anything the render document has been asked to do.
pub fn read_annotations(
    pdfium: &Pdfium,
    src_path: &str,
    password: Option<&str>,
) -> AppResult<Vec<PageAnnotationsDto>> {
    let document = pdfium
        .load_pdf_from_file(src_path, password)
        .map_err(|e| AppError::Malformed(format!("cannot open for annotations: {e:?}")))?;

    let mut pages = Vec::new();
    for (src, page) in document.pages().iter().enumerate() {
        let space = PageSpace::of(&page);
        let annots: Vec<AnnotationDto> = page
            .annotations()
            .iter()
            .enumerate()
            .filter_map(|(index, annot)| read_annotation(&annot, index as u32, &space))
            .collect();
        if annots.is_empty() {
            continue;
        }
        pages.push(PageAnnotationsDto {
            src: src as u32,
            annots,
        });
    }
    Ok(pages)
}

//! Finding the bubbles and checkboxes a document *draws* rather than declares.
//!
//! An AcroForm checkbox is a widget: it has a name, a value, and a viewer that
//! knows how to toggle it. A LaTeX `\bigcirc` is none of those things — it is a
//! glyph, no different to the engine from a comma. Yet it is what almost every
//! problem set and paper quiz actually uses, and "shade the correct bubble" is
//! a real instruction a reader has to be able to follow.
//!
//! So this module treats the glyph as the control. Detection runs over the
//! character stream that `text.rs` already extracts for selection and search,
//! which means it costs no extra PDFium calls: the boxes, sizes and font
//! classification are all sitting there once a page has been read at all.
//!
//! Nothing here writes. A detected bubble is only a target; shading one creates
//! an ordinary annotation over it, so the underlying page is never modified and
//! a bubble can be un-shaded by deleting that annotation like any other.

use super::types::{BubbleDto, RectDto};

/// Characters that are drawn as an empty, fillable control.
///
/// `\bigcirc` in CMSY10 — the LaTeX bubble — reaches us as U+20DD, a
/// *combining* enclosing circle, because that is the nearest thing the font's
/// `ToUnicode` map can name. It is the single most common entry here.
const BUBBLE_CHARS: &[char] = &[
    '\u{20DD}', // COMBINING ENCLOSING CIRCLE — LaTeX \bigcirc
    '\u{25CB}', // WHITE CIRCLE
    '\u{25EF}', // LARGE CIRCLE
    '\u{26AA}', // MEDIUM WHITE CIRCLE
    '\u{25CC}', // DOTTED CIRCLE
    '\u{20DE}', // COMBINING ENCLOSING SQUARE
    '\u{25A1}', // WHITE SQUARE
    '\u{25FB}', // WHITE MEDIUM SQUARE
    '\u{2610}', // BALLOT BOX
];

/// Smallest and largest side, in points, a glyph may have and still be a
/// control someone is expected to aim a cursor at. The lower bound rejects
/// degree signs and bullet-sized rings; the upper rejects decorative rings and
/// the circles that make up a diagram.
const MIN_SIDE_PT: f32 = 4.0;
const MAX_SIDE_PT: f32 = 40.0;

/// How far from square a bubble's box may be. Circles and squares are both
/// close to 1:1; a ring stretched well past that is part of a figure, not a
/// control. Kept loose because a glyph box is the *font's* bounding box, which
/// carries side bearings and is rarely exactly square.
const MIN_ASPECT: f32 = 0.55;

/// Every fillable-looking glyph on a page, in character order.
///
/// `boxes` and `sizes` are indexed by PDFium character index, exactly as
/// `text::extract_page` returns them; `raw` holds one Rust char per index. A
/// zero box means PDFium reported no geometry, which disqualifies a character
/// outright — there is nothing to aim at.
pub fn detect(raw: &str, boxes: &[[f32; 4]], sizes: &[f32]) -> Vec<BubbleDto> {
    let mut out = Vec::new();
    for (index, ch) in raw.chars().enumerate() {
        if !BUBBLE_CHARS.contains(&ch) {
            continue;
        }
        let Some([x, y, w, h]) = boxes.get(index).copied() else {
            continue;
        };
        if !is_control_sized(w, h) {
            continue;
        }
        out.push(BubbleDto {
            char_index: index as u32,
            rect: RectDto { x, y, w, h },
            font_size_pt: sizes.get(index).copied().unwrap_or(0.0),
        });
    }
    out
}

/// Whether a glyph box is plausibly a control rather than punctuation or art.
fn is_control_sized(w: f32, h: f32) -> bool {
    let (small, large) = if w < h { (w, h) } else { (h, w) };
    small >= MIN_SIDE_PT && large <= MAX_SIDE_PT && small / large >= MIN_ASPECT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boxes sized like the CMSY10 `\bigcirc` PDFium actually reports.
    fn bubble_box() -> [f32; 4] {
        [72.61, 531.10, 10.63, 11.13]
    }

    #[test]
    fn finds_latex_bigcirc() {
        let raw = "A \u{20DD} RP";
        let mut boxes = vec![[0.0; 4]; raw.chars().count()];
        boxes[2] = bubble_box();
        let found = detect(raw, &boxes, &vec![11.96; raw.chars().count()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].char_index, 2);
        assert_eq!(found[0].rect.w, 10.63);
        assert_eq!(found[0].font_size_pt, 11.96);
    }

    #[test]
    fn ignores_ordinary_text() {
        let raw = "no bubbles here";
        let boxes = vec![[0.0, 0.0, 6.0, 8.0]; raw.chars().count()];
        assert!(detect(raw, &boxes, &vec![10.0; raw.chars().count()]).is_empty());
    }

    /// A glyph PDFium gave no geometry cannot be clicked, so it is not offered.
    #[test]
    fn skips_glyphs_without_geometry() {
        let raw = "\u{20DD}";
        assert!(detect(raw, &[[0.0; 4]], &[11.96]).is_empty());
    }

    /// A ring the size of a bullet is punctuation; one the size of a figure is
    /// artwork. Neither is a control.
    #[test]
    fn rejects_sizes_outside_the_control_range() {
        let raw = "\u{25CB}";
        assert!(detect(raw, &[[0.0, 0.0, 2.0, 2.0]], &[3.0]).is_empty());
        assert!(detect(raw, &[[0.0, 0.0, 90.0, 90.0]], &[80.0]).is_empty());
    }

    /// A circle squashed into an oval belongs to a diagram.
    #[test]
    fn rejects_boxes_far_from_square() {
        let raw = "\u{25EF}";
        assert!(detect(raw, &[[0.0, 0.0, 30.0, 6.0]], &[12.0]).is_empty());
    }

    /// Square controls are as common as round ones on printed forms.
    #[test]
    fn finds_square_checkboxes() {
        let raw = "\u{2610}\u{25A1}";
        let boxes = vec![[0.0, 0.0, 10.0, 10.0], [20.0, 0.0, 10.0, 10.0]];
        assert_eq!(detect(raw, &boxes, &[12.0, 12.0]).len(), 2);
    }
}

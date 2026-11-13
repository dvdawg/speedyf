//! A page's text as typeset lines: what each line says, how large and how
//! heavy its type is, and where it stands. The library reads a paper's
//! title and authors off its first page from these (`library::firstpage`),
//! because the printed page says which line is the title only through
//! layout — it is the largest type near the top — and which lines are
//! names only through position: the ones right under it.
//!
//! One pass over the page's characters, a handful of FFI calls each, for
//! one page per document; the result is assembled by a pure function
//! (`assemble_lines`) so the rules can be tested without a PDF.

use crate::engine::text::DispMapper;
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_PAGE, FPDF_TEXTPAGE, FS_MATRIX, FS_RECTF,
};
use std::collections::HashMap;
use std::os::raw::{c_int, c_ulong};

/// One typeset line, or one segment of a line when a wide gap splits it
/// (side-by-side author names, a two-column page). Coordinates are in
/// points on the displayed page, measured from its top-left corner, so
/// `top < bottom`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    /// The line's text, word spaces recovered from the geometry, raised
    /// small type (footnote and affiliation markers) mapped to Unicode
    /// superscripts so a reader can tell "Smith¹" from "Smith1".
    pub text: String,
    /// Median font size of the line's characters, in points.
    pub size: f32,
    /// Median font weight PDFium reports (400 regular, 700 bold); 0 when
    /// unknown.
    pub weight: i32,
    /// Whether most of the line is set in a bold face, by weight, by the
    /// font's ForceBold flag, or by its name ("Times-Bold", "CMBX10",
    /// "NimbusRomNo9L-Medi").
    pub bold: bool,
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// One character as the line assembler sees it. Public within the crate
/// so tests can build pages by hand.
#[derive(Clone, Debug)]
pub(crate) struct LayoutGlyph {
    pub ch: char,
    /// Effective font size in points: the nominal size times the text
    /// matrix's vertical scale, so "/F1 1 Tf" scaled by the matrix reads
    /// as the size it prints at.
    pub size: f32,
    pub weight: i32,
    pub bold: bool,
    /// Whether the glyph runs left to right on the displayed page. The
    /// arXiv stamp up the margin does not.
    pub upright: bool,
    /// Baseline, top-down.
    pub baseline: f32,
    /// `[left, top, right, bottom]`, top-down: left and right from the
    /// glyph's loose box (its advance), top and bottom from its ink. `None`
    /// for a glyph with no ink (a space, a line break).
    pub bbox: Option<[f32; 4]>,
}

/// A gap wider than this share of an em is a word space.
///
/// Measured between *loose* glyph boxes, which span each glyph's advance
/// rather than its ink: letters within a word then sit flush (or kern a
/// little), and an interword space — never under about 0.2 em, even in a
/// tightly justified line — stands well clear of the threshold. Ink boxes,
/// which `library::lines_from_chars` measures, open false gaps beside
/// narrow-inked glyphs: "Maj ority", "I ntroduction", "20 16".
const WORD_GAP_EMS: f32 = 0.12;
/// Baseline movement, as a share of the larger font size, past which a
/// glyph starts a new line. A superscript rises about 0.4 em; the next
/// line is at least 1.1 em away.
const LINE_BREAK_EMS: f32 = 0.7;
/// Horizontal gap, in ems, that splits a line into segments: the space
/// between side-by-side author blocks or page columns, never a word space.
const SEGMENT_GAP_EMS: f32 = 2.5;
/// A glyph this much smaller than the line's type, raised this far above
/// its baseline, is a superscript.
const SUPERSCRIPT_SIZE_RATIO: f32 = 0.85;
const SUPERSCRIPT_RISE_EMS: f32 = 0.2;
/// PDF font descriptor flag ForceBold (PDF 1.7, table 123, bit 19).
const FORCE_BOLD_FLAG: c_int = 1 << 18;

/// The lines of one page, in the order PDFium reads them. `page_w` and
/// `page_h` are the displayed page's size in points
/// (`render::page_display_size`).
pub fn extract_lines(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    page_w: f32,
    page_h: f32,
) -> Vec<TextLine> {
    let tp = b.FPDFText_LoadPage(page);
    if tp.is_null() {
        return Vec::new();
    }
    let mapper = DispMapper::new(b, page, page_w, page_h);
    let count = b.FPDFText_CountChars(tp).max(0);
    let mut glyphs = Vec::with_capacity(count as usize);
    // Font names repeat for every glyph of a run; judge each name once.
    let mut bold_by_font: HashMap<String, bool> = HashMap::new();
    for index in 0..count {
        let ch = char::from_u32(b.FPDFText_GetUnicode(tp, index)).unwrap_or(' ');
        if ch == '\n' || ch == '\r' {
            glyphs.push(LayoutGlyph {
                ch,
                size: 0.0,
                weight: 0,
                bold: false,
                upright: true,
                baseline: 0.0,
                bbox: None,
            });
            continue;
        }
        let mut matrix = FS_MATRIX {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        let has_matrix = b.FPDFText_GetMatrix(tp, index, &mut matrix) != 0;
        let nominal = b.FPDFText_GetFontSize(tp, index) as f32;
        let (mut ox, mut oy) = (0f64, 0f64);
        let has_origin = b.FPDFText_GetCharOrigin(tp, index, &mut ox, &mut oy) != 0;

        let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
        let has_box = b.FPDFText_GetCharBox(tp, index, &mut l, &mut r, &mut bo, &mut t) != 0
            && (r - l).abs() > f64::EPSILON
            && (t - bo).abs() > f64::EPSILON;
        let mut loose = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        let has_loose = b.FPDFText_GetLooseCharBox(tp, index, &mut loose) != 0
            && loose.right > loose.left
            && loose.top > loose.bottom;
        let bbox = has_box.then(|| {
            let (x, y, w, h) = mapper.map_box(l, r, bo, t);
            let (left, right) = if has_loose {
                let (lx, _, lw, _) = mapper.map_box(
                    loose.left as f64,
                    loose.right as f64,
                    loose.bottom as f64,
                    loose.top as f64,
                );
                (lx, lx + lw)
            } else {
                (x, x + w)
            };
            [left, page_h - (y + h), right, page_h - y]
        });

        // Direction on the displayed page: the glyph's x axis, mapped.
        let upright = if has_matrix && has_origin {
            let (x0, y0) = mapper.map(ox, oy);
            let (x1, y1) = mapper.map(ox + matrix.a as f64, oy + matrix.b as f64);
            let (dx, dy) = (x1 - x0, y1 - y0);
            dx > 0.0 && dy.abs() <= 0.2 * dx
        } else {
            true
        };
        let scale = if has_matrix {
            (matrix.c * matrix.c + matrix.d * matrix.d).sqrt()
        } else {
            1.0
        };
        let mut size = nominal * scale;
        if size.is_nan() || size <= 0.5 {
            // No usable size: the glyph's ink is about 0.7 em tall.
            size = bbox.map_or(0.0, |[_, top, _, bottom]| (bottom - top) / 0.7);
        }
        let baseline = if has_origin {
            page_h - mapper.map(ox, oy).1
        } else {
            bbox.map_or(0.0, |[_, _, _, bottom]| bottom)
        };

        let weight = b.FPDFText_GetFontWeight(tp, index);
        let (name, flags) = font_info(b, tp, index);
        let named_bold = *bold_by_font
            .entry(name)
            .or_insert_with_key(|name| is_bold_font_name(name));
        let bold = weight >= 600 || named_bold || flags & FORCE_BOLD_FLAG != 0;

        glyphs.push(LayoutGlyph {
            ch,
            size,
            weight: weight.max(0),
            bold,
            upright,
            baseline,
            bbox,
        });
    }
    b.FPDFText_ClosePage(tp);
    assemble_lines(&glyphs)
}

/// The font's name and descriptor flags, or an empty name.
fn font_info(b: &dyn PdfiumLibraryBindings, tp: FPDF_TEXTPAGE, index: i32) -> (String, c_int) {
    let mut flags: c_int = 0;
    let mut buffer = [0u8; 128];
    let written = b.FPDFText_GetFontInfo(
        tp,
        index,
        buffer.as_mut_ptr().cast(),
        buffer.len() as c_ulong,
        &mut flags,
    ) as usize;
    if written == 0 || written > buffer.len() {
        return (String::new(), 0);
    }
    (
        String::from_utf8_lossy(&buffer[..written.saturating_sub(1)]).into_owned(),
        flags,
    )
}

/// Whether a font's name says it is a bold face. TeX's bold Computer
/// Modern is CMBX/CMB; the Times that LaTeX substitutes is
/// "NimbusRomNo9L-Medi" for bold and "-Regu" for regular.
pub(crate) fn is_bold_font_name(name: &str) -> bool {
    let family = name.split_once('+').map_or(name, |(_, rest)| rest);
    let lower = family.to_ascii_lowercase();
    [
        "bold", "black", "heavy", "semibold", "demi", "-medi", "-bd", ",bold",
    ]
    .iter()
    .any(|mark| lower.contains(mark))
        || lower.starts_with("cmbx")
        || lower.starts_with("cmb10")
        || lower.starts_with("sfbx")
        || lower.starts_with("lmroman10-bold")
}

/// Group glyphs, in reading order, into lines and segments.
///
/// A glyph starts a new line at a stated line break, when its baseline
/// moves by more than `LINE_BREAK_EMS`, or when it jumps back left; it
/// starts a new segment of the same line when a gap of `SEGMENT_GAP_EMS`
/// separates it from the previous glyph. Glyphs that do not run left to
/// right (margin stamps, rotated figure labels) are left out.
pub(crate) fn assemble_lines(glyphs: &[LayoutGlyph]) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut current: Vec<&LayoutGlyph> = Vec::new();
    // Reference baseline and size: the largest glyph so far, so a
    // superscript that opens a line does not set the line's baseline.
    let mut reference: Option<(f32, f32)> = None;
    let mut last: Option<[f32; 4]> = None;

    for glyph in glyphs {
        // A line break PDFium states is only its guess, and it guesses one
        // before every superscript; it separates words, and the geometry
        // decides lines.
        if !glyph.upright && glyph.bbox.is_some() {
            continue;
        }
        let Some(bbox) = glyph.bbox else {
            if !current.is_empty() {
                current.push(glyph);
            }
            continue;
        };
        if glyph.ch.is_whitespace() {
            if !current.is_empty() {
                current.push(glyph);
            }
            continue;
        }
        if let (Some((baseline, size)), Some(prev)) = (reference, last) {
            let em = size.max(glyph.size).max(1.0);
            let new_line = (glyph.baseline - baseline).abs() > LINE_BREAK_EMS * em;
            let wide_gap = bbox[0] - prev[2] > SEGMENT_GAP_EMS * em;
            let jumped_back = bbox[0] < prev[0] - em;
            if new_line || wide_gap || jumped_back {
                flush(&mut current, &mut lines);
                reference = None;
            }
        }
        current.push(glyph);
        match reference {
            Some((_, size)) if glyph.size <= size + 0.5 => {}
            _ => reference = Some((glyph.baseline, glyph.size)),
        }
        last = Some(bbox);
    }
    flush(&mut current, &mut lines);
    lines
}

fn flush(glyphs: &mut Vec<&LayoutGlyph>, lines: &mut Vec<TextLine>) {
    if let Some(line) = build_line(glyphs) {
        lines.push(line);
    }
    glyphs.clear();
}

/// One line from its glyphs: dominant size and weight, the union of the
/// inked boxes, and the text with word spaces put back and superscripts
/// marked.
fn build_line(glyphs: &[&LayoutGlyph]) -> Option<TextLine> {
    let inked: Vec<(&LayoutGlyph, [f32; 4])> = glyphs
        .iter()
        .filter(|g| !g.ch.is_whitespace())
        .filter_map(|g| g.bbox.map(|bbox| (*g, bbox)))
        .collect();
    if inked.is_empty() {
        return None;
    }
    let size = median(inked.iter().map(|(g, _)| g.size).collect())?;
    let full_size = |g: &LayoutGlyph| g.size >= SUPERSCRIPT_SIZE_RATIO * size;
    let baseline = median(
        inked
            .iter()
            .filter(|(g, _)| full_size(g))
            .map(|(g, _)| g.baseline)
            .collect(),
    )
    .unwrap_or(inked[0].0.baseline);
    let is_superscript =
        |g: &LayoutGlyph| !full_size(g) && baseline - g.baseline > SUPERSCRIPT_RISE_EMS * size;
    let weight = median(
        inked
            .iter()
            .filter(|(g, _)| full_size(g))
            .map(|(g, _)| g.weight as f32)
            .collect(),
    )
    .unwrap_or(0.0) as i32;
    let body: Vec<&LayoutGlyph> = inked
        .iter()
        .map(|(g, _)| *g)
        .filter(|g| !is_superscript(g))
        .collect();
    let bold = !body.is_empty() && body.iter().filter(|g| g.bold).count() * 2 > body.len();

    // Letter-spaced type ("S T U D Y  O F") sets every letter apart; a word
    // gap there is the letter spacing plus a space. Even the narrower gaps
    // between neighbours — the lower quartile — are that spacing, where
    // in ordinary type, and in a formula's spaced operators ("L = RL"),
    // they are about zero.
    let mut gaps: Vec<f32> = inked
        .windows(2)
        .map(|pair| pair[1].1[0] - pair[0].1[2])
        .filter(|gap| gap.abs() < 2.0 * size)
        .collect();
    gaps.sort_by(f32::total_cmp);
    let spacing = if gaps.len() >= 6 {
        gaps[gaps.len() / 4].max(0.0)
    } else {
        0.0
    };
    let letterspaced = spacing > WORD_GAP_EMS * size;
    let threshold = WORD_GAP_EMS * size + spacing;

    let mut text = String::new();
    let mut previous: Option<[f32; 4]> = None;
    for glyph in glyphs {
        if glyph.ch.is_whitespace() || glyph.ch.is_control() {
            // Spaces between letter-spaced letters say nothing; the
            // geometry decides there.
            if !letterspaced && !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            continue;
        }
        if let (Some(prev), Some(bbox)) = (previous, glyph.bbox) {
            if bbox[0] - prev[2] > threshold && !text.ends_with(' ') {
                text.push(' ');
            }
        }
        if is_superscript(glyph) {
            text.push(superscript(glyph.ch));
        } else {
            text.push(glyph.ch);
        }
        if glyph.bbox.is_some() {
            previous = glyph.bbox;
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    let fold = |init: f32, pick: fn(&[f32; 4]) -> f32, keep: fn(f32, f32) -> f32| {
        inked.iter().map(|(_, b)| pick(b)).fold(init, keep)
    };
    Some(TextLine {
        text,
        size,
        weight,
        bold,
        left: fold(f32::INFINITY, |b| b[0], f32::min),
        top: fold(f32::INFINITY, |b| b[1], f32::min),
        right: fold(f32::NEG_INFINITY, |b| b[2], f32::max),
        bottom: fold(f32::NEG_INFINITY, |b| b[3], f32::max),
    })
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    Some(values[values.len() / 2])
}

/// The superscript form of a raised character: digits and letters get
/// their Unicode superscript or modifier letter; marks that are already
/// footnote symbols (*, †, ‡) and punctuation stay as they are.
pub fn superscript(ch: char) -> char {
    const DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    const LETTERS: [char; 26] = [
        'ᵃ', 'ᵇ', 'ᶜ', 'ᵈ', 'ᵉ', 'ᶠ', 'ᵍ', 'ʰ', 'ⁱ', 'ʲ', 'ᵏ', 'ˡ', 'ᵐ', 'ⁿ', 'ᵒ', 'ᵖ', '𐞥', 'ʳ',
        'ˢ', 'ᵗ', 'ᵘ', 'ᵛ', 'ʷ', 'ˣ', 'ʸ', 'ᶻ',
    ];
    if let Some(digit) = ch.to_digit(10) {
        return DIGITS[digit as usize];
    }
    if ch.is_ascii_alphabetic() {
        return LETTERS[(ch.to_ascii_lowercase() as u8 - b'a') as usize];
    }
    ch
}

/// Whether a character is a superscript as `superscript` writes one.
pub fn is_superscript_char(ch: char) -> bool {
    matches!(ch, '⁰' | '¹' | '²' | '³' | '⁴'..='⁹' | 'ⁱ' | 'ⁿ' | 'ʰ'..='ʸ' | 'ˡ'..='ˣ')
        || ('\u{1D2C}'..='\u{1D6A}').contains(&ch)
        || ('\u{1D9B}'..='\u{1DBF}').contains(&ch)
        || ch == '𐞥'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs for `text` set at `size` from (`left`, `baseline`), each
    /// 0.5 em wide, a word gap of 0.3 em at each space (encoded as
    /// position only, the way LaTeX does).
    pub(crate) fn set(text: &str, size: f32, left: f32, baseline: f32) -> Vec<LayoutGlyph> {
        let mut x = left;
        let mut out = Vec::new();
        for ch in text.chars() {
            if ch == ' ' {
                x += 0.3 * size;
                continue;
            }
            out.push(LayoutGlyph {
                ch,
                size,
                weight: 400,
                bold: false,
                upright: true,
                baseline,
                bbox: Some([x, baseline - 0.7 * size, x + 0.5 * size, baseline]),
            });
            x += 0.5 * size;
        }
        out
    }

    fn end_of(glyphs: &[LayoutGlyph]) -> f32 {
        glyphs.last().and_then(|g| g.bbox).map_or(0.0, |b| b[2])
    }

    #[test]
    fn groups_by_baseline_and_recovers_spaces() {
        let mut glyphs = set("A Theory of Saddle", 17.0, 100.0, 120.0);
        glyphs.extend(set("Andrew Saxe", 11.0, 150.0, 160.0));
        let lines = assemble_lines(&glyphs);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "A Theory of Saddle");
        assert!((lines[0].size - 17.0).abs() < 0.01);
        assert!(lines[0].top < lines[0].bottom && lines[0].bottom <= 120.0);
        assert_eq!(lines[1].text, "Andrew Saxe");
    }

    #[test]
    fn raised_small_type_becomes_superscript_on_the_same_line() {
        let mut glyphs = set("Alice Smith", 11.0, 100.0, 200.0);
        let x = end_of(&glyphs);
        glyphs.extend(set("1", 7.0, x, 196.0));
        let x = end_of(&glyphs);
        glyphs.extend(set(", Bob Jones", 11.0, x, 200.0));
        let lines = assemble_lines(&glyphs);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Alice Smith¹, Bob Jones");
        assert!((lines[0].size - 11.0).abs() < 0.01);
    }

    #[test]
    fn letter_spaced_type_keeps_its_words() {
        // Every letter 0.3 em apart, words a further 0.3 em.
        let mut glyphs = Vec::new();
        let mut x = 100.0;
        for word in ["STUDY", "OF", "FISSION"] {
            for ch in word.chars() {
                glyphs.extend(set(&ch.to_string(), 10.0, x, 100.0));
                x += 8.0;
            }
            x += 3.0;
        }
        let lines = assemble_lines(&glyphs);
        assert_eq!(lines[0].text, "STUDY OF FISSION");
    }

    #[test]
    fn a_wide_gap_splits_segments() {
        let mut glyphs = set("Alice Smith", 10.0, 100.0, 200.0);
        glyphs.extend(set("Bob Jones", 10.0, 300.0, 200.0));
        let lines = assemble_lines(&glyphs);
        let texts: Vec<_> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Alice Smith", "Bob Jones"]);
        assert_eq!(lines[0].top, lines[1].top);
    }

    #[test]
    fn rotated_margin_text_is_left_out() {
        let mut glyphs = set("arXiv:1201.4166v2", 20.0, 10.0, 400.0);
        for glyph in &mut glyphs {
            glyph.upright = false;
        }
        glyphs.extend(set("Title of the Paper", 17.0, 100.0, 120.0));
        let lines = assemble_lines(&glyphs);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Title of the Paper");
    }

    #[test]
    fn bold_is_the_majority_of_full_size_glyphs() {
        let mut glyphs = set("Bold Title", 14.0, 100.0, 100.0);
        for glyph in &mut glyphs {
            glyph.bold = true;
            glyph.weight = 700;
        }
        let lines = assemble_lines(&glyphs);
        assert!(lines[0].bold);
        assert_eq!(lines[0].weight, 700);
    }

    #[test]
    fn bold_font_names() {
        for name in [
            "ABCDEF+NimbusRomNo9L-Medi",
            "Times-Bold",
            "XYZABC+CMBX12",
            "Arial,Bold",
            "MinionPro-Semibold",
        ] {
            assert!(is_bold_font_name(name), "{name}");
        }
        for name in ["ABCDEF+NimbusRomNo9L-Regu", "CMR10", "Times-Roman", ""] {
            assert!(!is_bold_font_name(name), "{name}");
        }
    }

    #[test]
    fn superscripts_round_trip() {
        for ch in "0123456789abcxyzq".chars() {
            assert!(is_superscript_char(superscript(ch)), "{ch}");
        }
        assert!(!is_superscript_char('a'));
        assert!(!is_superscript_char('1'));
        assert_eq!(superscript('*'), '*');
    }
}

/// Recovering formal environments — theorems, lemmas, definitions, figures —
/// from a LaTeX-compiled PDF.
///
/// hyperref anchors every stepped counter with a named destination, so a paper
/// carries `theorem.3.1`, `figure.2`, `definition.1.4` and friends whether or
/// not the author ever wrote a `\label`. That gives a complete, precisely
/// positioned candidate list.
///
/// What it does NOT give is the name to show. A destination carries the LaTeX
/// *counter*, and the near-universal `\newtheorem{lemma}[theorem]{Lemma}`
/// idiom shares one counter across lemmas, corollaries and propositions — so
/// all three anchor as `theorem.N`. Trusting the destination name would label
/// every corollary in the document "Theorem".
///
/// The printed text is what actually knows. So the destination supplies the
/// anchor and the number, the text at that anchor supplies the real name, and
/// the number appearing in *both* is what lets us reject a bad pairing rather
/// than emit a mislabelled entry.
use pdfium_render::prelude::{FPDF_DOCUMENT, PdfiumLibraryBindings};

/// Destinations hyperref emits that are never formal environments. Most junk
/// would fail the text check anyway; skipping these up front avoids doing the
/// text work for them at all.
const STRUCTURAL_PREFIXES: &[&str] = &[
    "page.",
    "subsection.",
    "subsubsection.",
    "part.",
    "chapter.",
    "paragraph.",
    "subparagraph.",
    "cite.",
    "Item.",
    "Hfootnote.",
    "footnote.",
    // Anchor text for an equation is the equation itself, so it can never pass
    // the name check — and a list of every numbered equation is noise anyway.
    "equation.",
];

/// Top-level structure, used to group the list rather than populate it.
pub fn is_section_destination(name: &str) -> bool {
    name.starts_with("section.") || name.starts_with("chapter.")
}

pub fn is_structural_destination(name: &str) -> bool {
    name == "Doc-Start"
        || name == "Navigation"
        || STRUCTURAL_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The counter value encoded in a destination name: `theorem.3.1` -> `3.1`.
/// Returns None when the name has no dotted numeric tail.
pub fn destination_number(name: &str) -> Option<&str> {
    let (_, tail) = name.split_once('.')?;
    let ok = !tail.is_empty()
        && tail
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_alphabetic());
    // Require at least one digit so `table.of.contents` and similar are out.
    if ok && tail.chars().any(|c| c.is_ascii_digit()) {
        Some(tail)
    } else {
        None
    }
}

/// Long enough for a real section title, short enough to stay one line.
const TITLE_MAX_CHARS: usize = 120;

/// Titles are read out of a page's character stream, so they arrive carrying
/// the line breaks of the typeset column. Flatten them for a one-line label.
fn clean_title(s: &str, max: usize) -> String {
    let mut flat = String::with_capacity(s.len());
    let mut pending = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            pending = !flat.is_empty();
        } else {
            if pending {
                flat.push(' ');
                pending = false;
            }
            flat.push(ch);
        }
    }
    truncate_chars(&flat, max)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct Heading {
    /// the word as printed: "Theorem", "Lemma", "Figure"
    pub kind: String,
    pub number: String,
}

/// Parse the head of the text sitting at an anchor.
///
/// Handles the shapes LaTeX actually produces, including the missing space in
/// `Theorem 1.1(Name).` that PDF text extraction hands back:
///   `Theorem 1.1.`            -> kind/number, no title
///   `Theorem 1.1 (Main).`     -> title "Main"
///   `Lemma 2.`                -> shared-counter environments keep their name
///   `Figure 3: A caption.`    -> title "A caption"
pub fn parse_heading(text: &str) -> Option<Heading> {
    let text = text.trim_start();
    let mut chars = text.char_indices().peekable();

    // A leading capitalized word.
    let start = chars.peek()?.0;
    if !text[start..].chars().next()?.is_uppercase() {
        return None;
    }
    let mut end = start;
    while let Some(&(i, c)) = chars.peek() {
        if c.is_alphabetic() {
            end = i + c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    let kind = &text[start..end];
    if kind.len() < 3 {
        return None;
    }

    // Whitespace, then a dotted number. A '.' is only part of the number when
    // a digit follows it, so the sentence-ending period in "Theorem 1.1." is
    // left alone.
    let rest = text[end..].trim_start();
    let bytes = rest.as_bytes();
    let mut n = 0usize;
    while n < bytes.len() {
        let c = bytes[n];
        if c.is_ascii_digit() {
            n += 1;
        } else if c == b'.' && n + 1 < bytes.len() && bytes[n + 1].is_ascii_digit() {
            n += 1;
        } else {
            break;
        }
    }
    if n == 0 {
        return None;
    }
    let number = &rest[..n];

    Some(Heading {
        kind: kind.to_string(),
        number: number.to_string(),
    })
}

/// A section heading prints its number *first* — `1 Setup` — which is exactly
/// the shape `parse_heading` rejects. Returns (number, title).
pub fn parse_section_heading(text: &str) -> Option<(String, String)> {
    let text = text.trim_start();
    let bytes = text.as_bytes();
    let mut n = 0usize;
    while n < bytes.len() {
        let c = bytes[n];
        if c.is_ascii_digit() {
            n += 1;
        } else if c == b'.' && n + 1 < bytes.len() && bytes[n + 1].is_ascii_digit() {
            n += 1;
        } else {
            break;
        }
    }
    if n == 0 {
        return None;
    }
    let title = clean_title(text[n..].trim(), TITLE_MAX_CHARS);
    if title.is_empty() {
        return None;
    }
    Some((text[..n].to_string(), title))
}

/// Label for a section heading, or None unless the printed number matches the
/// destination's — the same check `reconcile` makes, and for the same reason.
///
/// Without it anything starting with digits passes: on an arXiv paper the
/// stamp running down the margin, `2211.11320v3 [stat.ML] 22 Jun 2022 …`,
/// parsed as section "2211.11320" and dragged the paper title and author list
/// in as its name. `section.1` says the number is "1", which settles it.
pub fn reconcile_section(dest_name: &str, line: &str) -> Option<String> {
    let dest_number = destination_number(dest_name)?;
    let (number, title) = parse_section_heading(line)?;
    (number == dest_number).then(|| format!("{number} {title}"))
}

/// Pair a destination with the text found at it. Returns None unless the two
/// agree on the number — that agreement is the whole verification story.
pub fn reconcile(dest_name: &str, anchor_text: &str) -> Option<Heading> {
    if is_structural_destination(dest_name) {
        return None;
    }
    let dest_number = destination_number(dest_name)?;
    let heading = parse_heading(anchor_text)?;
    (heading.number == dest_number).then_some(heading)
}

/// A destination's position in the document.
pub struct Anchor {
    pub name: String,
    pub page: u32,
    /// x in page space, when the destination specifies one
    pub x: Option<f32>,
    pub y: f32,
}

/// Backstop against a pathological document; real papers have hundreds.
const MAX_NAMED_DESTS: u64 = 200_000;
const MAX_DEST_NAME_BYTES: i64 = 64 * 1024;

/// Every named destination that could plausibly be a formal environment or a
/// section heading, as `(name, page index, y in page space)`.
///
/// PDFium's two-call idiom: pass a null buffer to learn the length, then call
/// again to fill it. The name comes back as UTF-16, NUL-terminated.
pub fn enumerate_anchors(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    page_count: u32,
) -> Vec<Anchor> {
    let total = (b.FPDF_CountNamedDests(raw) as u64).min(MAX_NAMED_DESTS);
    let mut anchors = Vec::new();
    for index in 0..total {
        let mut buflen: std::os::raw::c_long = 0;
        let dest = b.FPDF_GetNamedDest(raw, index as i32, std::ptr::null_mut(), &mut buflen);
        if dest.is_null() || buflen <= 0 || buflen as i64 > MAX_DEST_NAME_BYTES {
            continue;
        }
        let mut units = vec![0u16; (buflen as usize).div_ceil(2)];
        let dest = b.FPDF_GetNamedDest(
            raw,
            index as i32,
            units.as_mut_ptr() as *mut std::os::raw::c_void,
            &mut buflen,
        );
        if dest.is_null() || buflen <= 0 {
            continue;
        }
        units.truncate((buflen as usize).div_ceil(2).min(units.len()));
        let name = String::from_utf16_lossy(&units);
        let name = name.trim_end_matches('\0');
        if name.is_empty() || is_structural_destination(name) || destination_number(name).is_none()
        {
            continue;
        }
        let page_index = b.FPDFDest_GetDestPageIndex(raw, dest);
        if page_index < 0 || page_index as u32 >= page_count {
            continue;
        }
        let (mut has_x, mut has_y, mut has_zoom) = (0, 0, 0);
        let (mut x, mut y, mut zoom) = (0.0f32, 0.0f32, 0.0f32);
        b.FPDFDest_GetLocationInPage(
            dest, &mut has_x, &mut has_y, &mut has_zoom, &mut x, &mut y, &mut zoom,
        );
        if has_y == 0 {
            continue;
        }
        anchors.push(Anchor {
            name: name.to_string(),
            page: page_index as u32,
            x: (has_x != 0).then_some(x),
            y,
        });
    }
    anchors
}

/// How far below a hyperref anchor a line's top may sit and still be its line.
const ANCHOR_TOLERANCE_PT: f32 = 6.0;
/// Runs whose distance from the anchor differs by less than this are treated
/// as the same line of text.
const LINE_GROUPING_PT: f32 = 5.0;
/// How far left of a destination's own x a character may start and still be
/// counted as part of the text it points at.
const ANCHOR_X_TOLERANCE_PT: f32 = 4.0;
const ANCHOR_TEXT_CHARS: usize = 220;

/// Index of the first character an anchor points at, in the page's character
/// stream. Search matches carry indices into that same stream, which is what
/// lets a hit be attributed to the environment containing it without going
/// anywhere near geometry.
pub fn anchor_start(boxes: &[[f32; 4]], anchor_x: Option<f32>, anchor_y: f32) -> Option<usize> {
    anchored_line(boxes, anchor_x, anchor_y).map(|(start, _)| start)
}

/// Text starting at a destination anchor, in reading order, capped at enough
/// characters to cover a name or caption.
///
/// The anchor sits at the line's top while a run's `y` is the bottom of its
/// box (display space is y-up), so we compare against each run's top edge.
///
/// The subtlety is that PDFium hands back one run per *word*, and a word's box
/// top varies by a point or so with its ascenders — so the run closest to the
/// anchor is whichever word happens to sit lowest, not the first one. Taking it
/// would start the text mid-line: for `Theorem 1.1 (Main).` the nearest run is
/// `(Main).`, which loses the very name we came for. So: find the nearest line,
/// then take the first run *on* that line.
/// Index of the first character on the line an anchor points at, plus that
/// line's distance below the anchor.
///
/// The anchor sits at the line's top while a box's `y` is its bottom (display
/// space is y-up), so we compare against each box's top edge. Characters are
/// matched rather than runs because a run's box top varies by a point or so
/// with its ascenders: the *nearest* thing to the anchor is whichever glyph
/// happens to sit lowest, not the first one. For `Theorem 1.1 (Main).` that is
/// the paren — starting there would lose the very name we came for. So: find
/// the nearest line, then take the first character on it.
fn anchored_line(boxes: &[[f32; 4]], anchor_x: Option<f32>, anchor_y: f32) -> Option<(usize, f32)> {
    // Characters left of the destination's own x are not its text. This is
    // what keeps the arXiv stamp out: it runs rotated down the margin at
    // x≈13 while the text block starts at x≈134, its glyphs span the whole
    // page height so one of them sits at almost any anchor's y, and being
    // background content it is drawn *first* — so it holds the lowest
    // character indices and won every "first match on this line" search.
    // It cost the panel a theorem per collision and, when the stamp happened
    // to start with the right digit, produced a section named after the paper.
    let in_column = |b: &[f32; 4]| {
        anchor_x.is_none_or(|x| b[0] >= x - ANCHOR_X_TOLERANCE_PT)
    };
    let top_of = |b: &[f32; 4]| (b[2] > 0.0 && b[3] > 0.0 && in_column(b)).then(|| b[1] + b[3]);
    let nearest = boxes
        .iter()
        .filter_map(top_of)
        .map(|top| anchor_y - top)
        .filter(|gap| *gap >= -ANCHOR_TOLERANCE_PT)
        .fold(f32::INFINITY, f32::min);
    if !nearest.is_finite() {
        return None;
    }
    let start = boxes.iter().position(|b| {
        top_of(b).is_some_and(|top| {
            let gap = anchor_y - top;
            gap >= -ANCHOR_TOLERANCE_PT && gap <= nearest + LINE_GROUPING_PT
        })
    })?;
    Some((start, nearest))
}

fn take_chars(raw: &str, start: usize, limit: usize) -> String {
    raw.chars().skip(start).take(limit).collect()
}

/// Text starting at a destination anchor, in reading order, capped at enough
/// characters to cover a name or caption.
///
/// Reads the raw character stream rather than the text runs. The run builder
/// strips inter-word spaces — a caption arrives as `Aplaceholderfigure.` —
/// while the stream PDFium hands back still has them, and its indices line up
/// with the character boxes one-for-one.
pub fn anchor_text(
    raw: &str,
    boxes: &[[f32; 4]],
    anchor_x: Option<f32>,
    anchor_y: f32,
) -> Option<String> {
    let (start, _) = anchored_line(boxes, anchor_x, anchor_y)?;
    let text = take_chars(raw, start, ANCHOR_TEXT_CHARS);
    (!text.trim().is_empty()).then_some(text)
}

/// As `anchor_text`, but stopping at the end of the anchored line. A section
/// heading occupies its own line, and letting it run on would swallow the
/// first paragraph of the section as its title.
pub fn anchor_line_text(
    raw: &str,
    boxes: &[[f32; 4]],
    anchor_x: Option<f32>,
    anchor_y: f32,
) -> Option<String> {
    let (start, nearest) = anchored_line(boxes, anchor_x, anchor_y)?;
    let mut text = String::new();
    for (offset, ch) in raw.chars().skip(start).enumerate().take(ANCHOR_TEXT_CHARS) {
        // A character with no geometry (a space) belongs to the line it sits in.
        if let Some(b) = boxes.get(start + offset).filter(|b| b[2] > 0.0 && b[3] > 0.0) {
            if anchor_y - (b[1] + b[3]) > nearest + LINE_GROUPING_PT {
                break;
            }
        }
        text.push(ch);
    }
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_the_structural_destinations_hyperref_also_emits() {
        for name in [
            "Doc-Start",
            "page.7",
            "subsection.2.1",
            "cite.Knuth1984",
            "Item.12",
            "equation.4",
        ] {
            assert!(is_structural_destination(name), "{name} should be skipped");
        }
        assert!(!is_structural_destination("theorem.1.1"));
        assert!(!is_structural_destination("figure.3"));
        // sections are kept, but to group the list rather than populate it
        assert!(!is_structural_destination("section.2"));
        assert!(is_section_destination("section.2"));
    }

    #[test]
    fn reads_the_counter_value_out_of_a_destination_name() {
        assert_eq!(destination_number("theorem.3.1"), Some("3.1"));
        assert_eq!(destination_number("figure.2"), Some("2"));
        assert_eq!(destination_number("definition.1.4"), Some("1.4"));
        assert_eq!(destination_number("Doc-Start"), None);
        assert_eq!(destination_number("nodots"), None);
    }

    #[test]
    fn parses_a_bare_numbered_environment() {
        let h = parse_heading("Theorem 1.1. The mistake bound is finite.").unwrap();
        assert_eq!(h.kind, "Theorem");
        assert_eq!(h.number, "1.1");
    }

    #[test]
    fn parses_a_named_environment_even_without_the_space_pdf_drops() {
        // this is the exact shape PDFium hands back for `\begin{theorem}[Main]`
        let h = parse_heading("Theorem 1.1(Main).No label here.").unwrap();
        // the bracketed name is not shown, but must not derail kind/number
        assert_eq!(h.kind, "Theorem");
        assert_eq!(h.number, "1.1");
    }

    #[test]
    fn keeps_the_printed_name_for_shared_counter_environments() {
        // the whole point: these anchor as theorem.1.2 / theorem.1.3
        assert_eq!(parse_heading("Lemma 1.2.An unlabeled lemma.").unwrap().kind, "Lemma");
        assert_eq!(
            parse_heading("Corollary 1.3.An unlabeled corollary.").unwrap().kind,
            "Corollary"
        );
    }

    #[test]
    fn flattens_a_section_title_too() {
        assert_eq!(
            parse_section_heading("2 Results\r\n"),
            Some(("2".into(), "Results".into()))
        );
    }

    /// Lay out `lines` as (text, top edge) the way the engine hands them over:
    /// a raw character stream plus one box per character, with spaces carrying
    /// no geometry.
    fn page(lines: &[(&str, f32)]) -> (String, Vec<[f32; 4]>) {
        let mut raw = String::new();
        let mut boxes = Vec::new();
        for (text, top) in lines {
            let mut x = 0.0f32;
            for ch in text.chars() {
                raw.push(ch);
                boxes.push(if ch == ' ' {
                    [0.0; 4]
                } else {
                    [x, top - 8.0, 5.0, 8.0]
                });
                x += 6.0;
            }
        }
        (raw, boxes)
    }

    #[test]
    fn keeps_the_inter_word_spaces_the_runs_drop() {
        // the run builder gives "Aplaceholderfigure."; the stream has spaces
        let (raw, boxes) = page(&[("Figure 1: A placeholder figure.", 500.0)]);
        assert_eq!(
            anchor_text(&raw, &boxes, None, 505.0).unwrap(),
            "Figure 1: A placeholder figure."
        );
    }

    #[test]
    fn ignores_marginal_text_outside_the_anchors_column() {
        // An arXiv stamp runs rotated down the margin: it is drawn first, so
        // it holds the lowest character indices, and its glyphs span the page
        // height so one sits at almost any anchor's y. Only its x gives it
        // away — the text block starts far to its right.
        let mut raw = String::new();
        let mut boxes: Vec<[f32; 4]> = Vec::new();
        for ch in "2211.11320v3 [stat.ML]".chars() {
            raw.push(ch);
            boxes.push([13.5, 434.0, 6.0, 8.0]); // margin column, at our anchor's y
        }
        let mut x = 134.0f32;
        for ch in "Theorem 1.1. First result.".chars() {
            raw.push(ch);
            boxes.push([x, 434.0, 5.0, 8.0]);
            x += 6.0;
        }

        // without the column filter the stamp wins, because it comes first
        assert!(anchor_text(&raw, &boxes, None, 443.0)
            .unwrap()
            .starts_with("2211"));
        // with it, the anchor finds its own text
        assert!(anchor_text(&raw, &boxes, Some(133.8), 443.0)
            .unwrap()
            .starts_with("Theorem 1.1."));
    }

    #[test]
    fn starts_at_the_first_character_on_the_anchored_line_not_the_nearest() {
        let (raw, boxes) = page(&[
            ("1 Setup", 667.0),
            ("Theorem 1.1 (Unlabeled).", 642.3),
            ("No label here at all.", 630.0),
        ]);
        let text = anchor_text(&raw, &boxes, None, 647.4).unwrap();
        assert!(text.starts_with("Theorem 1.1 (Unlabeled)."), "got {text:?}");
    }

    #[test]
    fn ignores_text_above_the_anchor() {
        let (raw, boxes) = page(&[("Header", 700.0)]);
        assert!(anchor_text(&raw, &boxes, None, 400.0).is_none());
        assert!(anchor_text(&raw, &boxes, None, 705.0).is_some());
        assert!(anchor_text("", &[], None, 700.0).is_none());
    }

    #[test]
    fn a_section_title_stops_at_its_own_line() {
        let (raw, boxes) = page(&[("1 Setup", 667.0), ("Definition 1.1 (Feedback).", 642.0)]);
        assert_eq!(anchor_line_text(&raw, &boxes, None, 670.0).unwrap(), "1 Setup");
        // the unbounded form runs on into the body, as it should
        assert!(anchor_text(&raw, &boxes, None, 670.0).unwrap().contains("Definition"));
    }

    #[test]
    fn a_section_heading_is_number_first() {
        assert_eq!(
            parse_section_heading("1 Setup"),
            Some(("1".into(), "Setup".into()))
        );
        // an environment heading is the other way round and must not match
        assert_eq!(parse_section_heading("Theorem 1.1."), None);
    }

    #[test]
    fn rejects_a_section_whose_number_is_not_the_destinations() {
        // the arXiv stamp running down the margin of page one, which this
        // panel used to show as a section spanning the whole paper
        let stamp = "2211.11320v3 [stat.ML] 22 Jun 2022 Private and polynomial time \
                     algorithms for learning Gaussians and beyond Hassan Ashtiani";
        assert!(reconcile_section("section.1", stamp).is_none());

        // a real heading still resolves
        assert_eq!(
            reconcile_section("section.1", "1 Introduction\r\n").as_deref(),
            Some("1 Introduction")
        );
        // and a heading that is not the one this destination names does not
        assert_eq!(reconcile_section("section.2", "2.1 A subsection"), None);
    }

    #[test]
    fn only_top_level_structure_groups_the_list() {
        assert!(is_section_destination("section.2"));
        assert!(is_section_destination("chapter.4"));
        assert!(!is_section_destination("subsection.2.1"));
        assert!(!is_section_destination("theorem.1.1"));
    }

    #[test]
    fn rejects_text_that_is_not_a_numbered_environment() {
        assert!(parse_heading("1 Setup").is_none()); // a section heading
        assert!(parse_heading("The theorem states that").is_none()); // prose
        assert!(parse_heading("").is_none());
        assert!(parse_heading("Proof. Immediate.").is_none()); // unnumbered
    }

    #[test]
    fn reconciles_only_when_the_numbers_agree() {
        // a lemma sharing the theorem counter: names differ, numbers agree
        let ok = reconcile("theorem.1.2", "Lemma 1.2.An unlabeled lemma.").unwrap();
        assert_eq!(ok.kind, "Lemma");
        assert_eq!(ok.number, "1.2");

        // the text at the anchor belongs to something else entirely
        assert!(reconcile("theorem.1.2", "Theorem 9.9. Elsewhere.").is_none());
        assert!(reconcile("section.1", "Theorem 1.1.").is_none());
    }
}

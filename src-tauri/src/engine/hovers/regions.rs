//! Where on a page nothing should be hoverable, and which part of the
//! document each line belongs to.
//!
//! - Running heads and feet ("furniture") repeat on every page and are never
//!   anything's definition: a section title printed as a running head would
//!   otherwise be a second, wrong "Section 4".
//! - A bibliography is full of things shaped like references - "Phys. Rev.
//!   Lett. 12", "Table 2 in" a title - none of them about this document.
//! - A table of contents or list of figures repeats every caption and
//!   heading with a page number after it.
//!
//! Parts are what numbering restarts within: the main text, an appendix, a
//! supplement, a chapter. A reference prefers a definition in its own part.
//!
//! State carries across pages, in page order: a bibliography started on one
//! page runs on through the next.

use super::page::Page;

#[derive(Clone, Debug, Default)]
pub struct RegionState {
    pub in_bibliography: bool,
    /// Pages of contents seen in a row: a contents list runs over several.
    pub in_contents: bool,
    pub part: u32,
}

/// Per-line part numbers for one page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Regions {
    pub part_of_line: Vec<u32>,
}

const REFERENCE_HEADINGS: &[&str] = &[
    "references",
    "reference",
    "bibliography",
    "references and notes",
    "notes and references",
    "literature cited",
    "works cited",
    "cited literature",
    "literature",
    "references cited",
];

const CONTENTS_HEADINGS: &[&str] = &[
    "contents",
    "table of contents",
    "list of figures",
    "list of tables",
    "list of algorithms",
    "brief contents",
];

const PART_OPENERS: &[&str] = &[
    "appendix",
    "appendices",
    "supplementary",
    "supplemental",
    "supporting information",
    "online methods",
    "electronic supplementary",
];

/// Strip a leading section number ("7", "7.", "VII.", "A") from a heading.
fn without_number(text: &str) -> &str {
    let trimmed = text.trim();
    let mut parts = trimmed.splitn(2, ' ');
    let first = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    let first_core = first.trim_end_matches(['.', ':']);
    let numeric = !first_core.is_empty()
        && (first_core.chars().all(|c| c.is_ascii_digit() || c == '.')
            || first_core.chars().all(|c| "IVXLC".contains(c)));
    if numeric && !rest.is_empty() {
        rest.trim()
    } else {
        trimmed
    }
}

/// The line, as a heading: lowercase, number and trailing colon removed.
fn heading_words(text: &str) -> String {
    without_number(text)
        .trim_end_matches(':')
        .trim()
        .to_lowercase()
}

/// Whether a line is set apart from the body text the way headings are: in
/// another font, larger, or in capitals.
pub fn styled_apart(page: &Page, line: usize) -> bool {
    let l = &page.lines[line];
    let text = page.line_text(line);
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    let capitals = letters.len() >= 3 && letters.iter().all(|c| c.is_uppercase());
    l.font != page.body_font || (page.body_size > 0.0 && l.size > 1.05 * page.body_size) || capitals
}

fn is_reference_heading(page: &Page, line: usize) -> bool {
    let text = page.line_text(line);
    if text.chars().count() > 40 || page.continues_previous(line) {
        return false;
    }
    let first = text.trim().chars().find(|c| c.is_alphabetic());
    if !first.is_some_and(|c| c.is_uppercase()) || text.trim_end().ends_with('.') {
        return false;
    }
    REFERENCE_HEADINGS.contains(&heading_words(&text).as_str())
}

fn is_contents_heading(page: &Page, line: usize) -> bool {
    let text = page.line_text(line);
    if text.chars().count() > 40 || page.continues_previous(line) {
        return false;
    }
    CONTENTS_HEADINGS.contains(&heading_words(&text).as_str())
}

/// A heading that starts a new part: "Appendix A: Proofs", "Supplementary
/// Material", "Chapter 3".
fn is_part_heading(page: &Page, line: usize) -> bool {
    let text = page.line_text(line);
    if text.chars().count() > 90 || page.continues_previous(line) {
        return false;
    }
    let lower = without_number(&text).to_lowercase();
    let opens = PART_OPENERS.iter().any(|w| lower.starts_with(w));
    let chapter = lower.starts_with("chapter ")
        && page.body_size > 0.0
        && page.lines[line].size >= 1.15 * page.body_size;
    (opens && styled_apart(page, line)) || chapter
}

/// A numbered section heading in a heading font: what ends a bibliography
/// that an appendix follows without saying "Appendix".
fn is_strong_heading(page: &Page, line: usize) -> bool {
    let text = page.line_text(line);
    if text.chars().count() > 80 || page.continues_previous(line) {
        return false;
    }
    let l = &page.lines[line];
    let larger = page.body_size > 0.0 && l.size >= 1.15 * page.body_size;
    let mut words = text.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    let label = first.trim_end_matches('.');
    let labelled = !label.is_empty()
        && label.len() <= 6
        && label
            .split('.')
            .all(|p| !p.is_empty() && (p.chars().all(|c| c.is_ascii_digit()) || p.len() == 1));
    let titled = words
        .next()
        .and_then(|w| w.chars().next())
        .is_some_and(|c| c.is_uppercase());
    let bibliographic = text.contains("et al")
        || text.matches(',').count() >= 2
        || text
            .split(|c: char| !c.is_ascii_digit())
            .any(|d| d.len() == 4);
    (larger || (l.font != page.body_font && labelled && titled)) && !bibliographic
}

/// A table-of-contents entry: dot leaders, or a title ending in a page
/// number set well apart.
fn is_contents_entry(page: &Page, line: usize) -> bool {
    let text = page.line_text(line);
    text.contains("....") || text.contains(". . . .")
}

/// Running heads and feet: short lines in the top or bottom band of the
/// page, standing apart from everything else.
fn mark_furniture(page: &mut Page) {
    let height = page.height.max(1.0);
    let count = page.lines.len();
    let mut furniture = vec![false; count];
    for (index, line) in page.lines.iter().enumerate() {
        let top_band = line.bottom >= 0.91 * height;
        let bottom_band = line.top <= 0.07 * height;
        if !top_band && !bottom_band {
            continue;
        }
        let chars = line.end - line.start;
        if chars > 100 || (bottom_band && chars > 60) {
            continue;
        }
        // Nearest other line's distance, vertically.
        let nearest = page
            .lines
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, other)| {
                if other.top <= line.bottom {
                    line.bottom - other.top
                } else if other.bottom >= line.top {
                    other.bottom - line.top
                } else {
                    0.0
                }
            })
            .filter(|gap| *gap > 0.0)
            .fold(f32::MAX, f32::min);
        if nearest > 1.2 * line.em() {
            furniture[index] = true;
        }
    }
    for (line, flag) in page.lines.iter_mut().zip(furniture) {
        line.flags.furniture = flag;
    }
}

/// Classify every line of a page, carrying state from the pages before it.
pub fn classify(page: &mut Page, state: &mut RegionState) -> Regions {
    mark_furniture(page);
    let count = page.lines.len();

    // A contents run ends on the first page that does not look like one.
    if state.in_contents {
        let entries = (0..count).filter(|&l| is_contents_entry(page, l)).count();
        if count == 0 || entries * 3 < count {
            state.in_contents = false;
        }
    }

    let mut parts = Vec::with_capacity(count);
    for line in 0..count {
        if page.lines[line].flags.furniture {
            parts.push(state.part);
            continue;
        }
        if is_contents_heading(page, line) {
            state.in_contents = true;
        }
        if is_reference_heading(page, line) {
            state.in_bibliography = true;
            state.part += 1;
        } else if is_part_heading(page, line) {
            state.in_bibliography = false;
            state.in_contents = false;
            state.part += 1;
        } else if state.in_bibliography && is_strong_heading(page, line) {
            state.in_bibliography = false;
            state.part += 1;
        }
        let contents = state.in_contents || is_contents_entry(page, line);
        let flags = &mut page.lines[line].flags;
        flags.bibliography = state.in_bibliography;
        flags.contents = contents;
        parts.push(state.part);
    }
    Regions {
        part_of_line: parts,
    }
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::*;

    #[test]
    fn a_references_heading_starts_the_bibliography() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "The end of the paper.");
        b.line(670.0, BOLD, "References");
        b.line(650.0, BODY, "[1] A. Author, Phys. Rev. Lett. 12, 3 (2019).");
        let mut page = b.page();
        let mut state = RegionState::default();
        let regions = classify(&mut page, &mut state);
        assert!(!page.lines[0].flags.bibliography);
        assert!(page.lines[2].flags.bibliography);
        assert!(state.in_bibliography);
        assert_eq!(regions.part_of_line, vec![0, 1, 1]);
    }

    #[test]
    fn a_sentence_ending_in_references_is_not_a_heading() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "as discussed in the");
        b.line(688.0, BODY, "References");
        let mut page = b.page();
        let mut state = RegionState::default();
        classify(&mut page, &mut state);
        assert!(!state.in_bibliography);
    }

    #[test]
    fn an_appendix_heading_ends_the_bibliography_and_starts_a_part() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "[9] B. Writer, J. Chem. 4, 1 (2001).");
        b.line(660.0, BOLD, "Appendix A: Proofs");
        b.line(640.0, BODY, "Some proof text.");
        let mut page = b.page();
        let mut state = RegionState {
            in_bibliography: true,
            in_contents: false,
            part: 1,
        };
        let regions = classify(&mut page, &mut state);
        assert!(page.lines[0].flags.bibliography);
        assert!(!page.lines[2].flags.bibliography);
        assert_eq!(regions.part_of_line, vec![1, 2, 2]);
    }

    #[test]
    fn a_contents_page_is_flagged() {
        let mut b = Builder::new();
        b.line(700.0, BOLD, "Contents");
        b.line(680.0, BODY, "1 Introduction . . . . . . 3");
        b.line(668.0, BODY, "2 Results . . . . . . . . 5");
        let mut page = b.page();
        let mut state = RegionState::default();
        classify(&mut page, &mut state);
        assert!(page.lines.iter().all(|l| l.flags.contents));
    }

    #[test]
    fn a_running_head_is_furniture() {
        let mut b = Builder::new();
        b.line(760.0, SMALL, "J. Phys. 12 (2020) 1-20");
        for i in 0..10 {
            b.line(700.0 - 12.0 * i as f32, BODY, "body text of the page");
        }
        b.line(40.0, BODY, "7");
        let mut page = b.page();
        classify(&mut page, &mut RegionState::default());
        assert!(page.lines[0].flags.furniture);
        assert!(!page.lines[1].flags.furniture);
        assert!(page.lines.last().unwrap().flags.furniture);
    }
}

//! Terms an extension knows the document defines - acronyms, in practice,
//! from Notation - matched against a page's text.
//!
//! The extension says what a term is and where it is defined; the core does
//! the matching, because only the core knows where the text sits, which of
//! it is mathematics, and where the bibliography is. Matching is by whole
//! token, case-sensitive unless the term says otherwise:
//!
//! - "FDT" matches "FDT", "FDTs", "FDT's", "FDT-based", "non-FDT".
//! - A defined "IT" never matches "it", "ITS" or "ITEM"; "ITs" is its plural.
//! - Nothing in mathematics, scripts, the bibliography, running heads or a
//!   table of contents.
//! - Not the definition itself.
//! - A term defined twice takes the nearest preceding definition, or shows
//!   both when neither precedes.
//!
//! PDFium-free.

use super::page::{Page, Script};
use super::Rect;
use serde::Deserialize;

/// Most terms one document may contribute.
pub const MAX_TERMS: usize = 2_000;
const MAX_TERM_CHARS: usize = 64;
const MAX_TEXT_CHARS: usize = 600;

/// One term, as an extension contributes it.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TermInput {
    /// The term as it appears in text: "FDT", "mRNA", "COVID-19".
    pub text: String,
    #[serde(default = "default_true")]
    pub case_sensitive: bool,
    /// What it is: "acronym", "symbol", ...; shown nowhere, kept for later.
    #[serde(default)]
    pub kind: Option<String>,
    /// The popover's heading.
    pub title: String,
    /// The popover's text: plain, or with `$...$` TeX.
    pub body: String,
    /// The page in the file where it is defined.
    pub target_page: u32,
    /// Where on that page, y-up page space; the definition there is not
    /// itself hoverable.
    #[serde(default)]
    pub target_rect: Option<Rect>,
}

fn default_true() -> bool {
    true
}

/// Drop what cannot be a term: too short, too long, multi-line. A single
/// character is never a term.
pub fn sanitize(terms: Vec<TermInput>) -> Vec<TermInput> {
    terms
        .into_iter()
        .filter(|t| {
            let chars = t.text.chars().count();
            (2..=MAX_TERM_CHARS).contains(&chars)
                && !t.text.contains(['\n', '\r'])
                && t.text.trim() == t.text
                && t.text.chars().any(|c| c.is_alphanumeric())
                && t.text.chars().next().is_some_and(|c| c.is_alphanumeric())
        })
        .map(|mut t| {
            t.title = t.title.chars().take(MAX_TERM_CHARS * 2).collect();
            t.body = t.body.chars().take(MAX_TEXT_CHARS).collect();
            t.target_rect = t
                .target_rect
                .filter(|r| r.iter().all(|v| v.is_finite()) && r[2] >= 0.0 && r[3] >= 0.0);
            t
        })
        .take(MAX_TERMS)
        .collect()
}

/// One occurrence of a term on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct TermHit {
    pub range: (usize, usize),
    pub rects: Vec<Rect>,
    /// Indices into the contributed terms: the definition this occurrence
    /// takes, or every one when none precedes it.
    pub definitions: Vec<usize>,
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

fn matches_at(text: &[char], at: usize, term: &[char], case_sensitive: bool) -> bool {
    if at + term.len() > text.len() {
        return false;
    }
    term.iter().zip(&text[at..]).all(|(t, c)| {
        if case_sensitive {
            t == c
        } else {
            t.to_lowercase().eq(c.to_lowercase())
        }
    })
}

/// Where a whole-token occurrence ends, plural included, or None.
fn token_end(text: &[char], end: usize, term: &[char]) -> Option<usize> {
    let word = |i: usize| text.get(i).is_some_and(|c| c.is_alphanumeric());
    if !word(end) {
        return Some(end);
    }
    // "FDTs": a lowercase s after a term ending in a capital or digit.
    let last = term.last()?;
    if text[end] == 's' && (last.is_uppercase() || last.is_ascii_digit()) && !word(end + 1) {
        return Some(end);
    }
    None
}

/// Whether the occurrence is set flush against mathematics: the glyph just
/// before or after it, across at most a space, is a math glyph within a
/// third of an em - inline math the term is part of.
fn touches_math(page: &Page, start: usize, end: usize) -> bool {
    let Some(first) = page.glyph_at(start) else {
        return false;
    };
    let Some(last) = end.checked_sub(1).and_then(|i| page.glyph_at(i)) else {
        return false;
    };
    let em = page.line_at(start).map_or(10.0, |l| l.em());
    let near = |index: Option<usize>, before: bool| {
        let Some(index) = index else { return false };
        let index = if page.text.get(index) == Some(&' ') {
            if before {
                index.checked_sub(1)
            } else {
                Some(index + 1)
            }
        } else {
            Some(index)
        };
        let Some(glyph) = index.and_then(|i| page.glyph_at(i)) else {
            return false;
        };
        if !glyph.math || !glyph.has_box() {
            return false;
        }
        let gap = if before {
            first.rect[0] - (glyph.rect[0] + glyph.rect[2])
        } else {
            glyph.rect[0] - (last.rect[0] + last.rect[2])
        };
        gap.abs() < 0.35 * em
    };
    near(start.checked_sub(1), true) || near(Some(end), false)
}

/// Letters and digits of `text`, lowercase: what two spellings of a long
/// form share.
fn letters(text: impl Iterator<Item = char>) -> String {
    text.filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether the occurrence is a definition of its own, wherever it is: in
/// parentheses straight after its long form, "fluctuation-dissipation
/// theorem (FDT)", or straight before it in parentheses, "FDT
/// (fluctuation-dissipation theorem)".
fn redefined_here(
    page: &Page,
    terms: &[TermInput],
    same: &[usize],
    start: usize,
    end: usize,
) -> bool {
    let text = &page.text;
    same.iter().any(|&i| {
        let long = letters(terms[i].body.chars());
        if long.len() < 3 {
            return false;
        }
        let window = terms[i].body.chars().count() + 12;
        if start > 0 && text[start - 1] == '(' {
            let from = start.saturating_sub(1 + window);
            let before = letters(text[from..start - 1].iter().copied());
            if before.ends_with(&long) {
                return true;
            }
        }
        let open = (end..(end + 2).min(text.len())).find(|&k| text[k] == '(');
        if let Some(open) = open {
            let to = (open + 1 + window).min(text.len());
            let after = letters(text[open + 1..to].iter().copied());
            if after.starts_with(&long) {
                return true;
            }
        }
        false
    })
}

/// Every occurrence of the contributed terms on page `src`.
pub fn find_terms(page: &Page, src: u32, terms: &[TermInput]) -> Vec<TermHit> {
    if terms.is_empty() {
        return Vec::new();
    }
    let text = &page.text;
    let patterns: Vec<Vec<char>> = terms.iter().map(|t| t.text.chars().collect()).collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let boundary = index == 0 || !text[index - 1].is_alphanumeric();
        if !boundary {
            index += 1;
            continue;
        }
        // The longest term matching here; all its definitions.
        let mut best: Option<(usize, usize)> = None;
        for (i, (term, pattern)) in terms.iter().zip(&patterns).enumerate() {
            if !matches_at(text, index, pattern, term.case_sensitive) {
                continue;
            }
            let end = index + pattern.len();
            if token_end(text, end, pattern).is_none() {
                continue;
            }
            if best.is_none_or(|(_, len)| pattern.len() > len) {
                best = Some((i, pattern.len()));
            }
        }
        let Some((first, len)) = best else {
            index += 1;
            continue;
        };
        let end = index + len;
        let same: Vec<usize> = (0..terms.len())
            .filter(|&i| {
                patterns[i].len() == len
                    && matches_at(text, index, &patterns[i], terms[i].case_sensitive)
                    && terms[i].case_sensitive == terms[first].case_sensitive
            })
            .collect();
        if let Some(hit) = hit_at(page, src, terms, &same, index, end) {
            out.push(hit);
        }
        index = end;
    }
    out
}

fn hit_at(
    page: &Page,
    src: u32,
    terms: &[TermInput],
    same: &[usize],
    start: usize,
    end: usize,
) -> Option<TermHit> {
    for at in start..end {
        if page.script[at] != Script::Normal {
            return None;
        }
        let line = page.line_at(at)?;
        if line.excluded() || line.is_display() {
            return None;
        }
        // Set smaller than its line: a subscript ("f_ERM") whether or not
        // it sits far enough off the baseline to read as one.
        let glyph = page.glyph_at(at)?;
        if glyph.size > 0.0 && line.size > 0.0 && glyph.size < 0.85 * line.size {
            return None;
        }
    }
    if page.any_math(start, end) || touches_math(page, start, end) {
        return None;
    }
    if redefined_here(page, terms, same, start, end) {
        return None;
    }
    let rects = page.rects(start, end);
    let top = rects.iter().map(|r| r[1] + r[3]).fold(f32::MIN, f32::max);
    // The definition itself.
    for &i in same {
        let term = &terms[i];
        if term.target_page != src {
            continue;
        }
        match term.target_rect {
            Some(target) => {
                if rects.iter().any(|r| overlaps(*r, target)) {
                    return None;
                }
            }
            None => {
                // No rectangle given: the occurrence in parentheses right
                // after its long form, or right before "(long form)".
                let before = start.checked_sub(1).and_then(|i| page.text.get(i));
                let after = page.text.get(end);
                let next = page.text.get(end + 1);
                if before == Some(&'(') || (after == Some(&' ') && next == Some(&'(')) {
                    return None;
                }
            }
        }
    }
    // Nearest preceding definition; all of them when none precedes.
    let position = |page_index: u32, y: f32| (page_index, -y);
    let here = position(src, top);
    let preceding = same
        .iter()
        .copied()
        .filter(|&i| {
            let t = &terms[i];
            let y = t.target_rect.map(|r| r[1] + r[3]).unwrap_or(f32::MAX);
            position(t.target_page, y) <= here
        })
        .max_by(|&a, &b| {
            let ta = &terms[a];
            let tb = &terms[b];
            let ya = ta.target_rect.map(|r| r[1] + r[3]).unwrap_or(f32::MAX);
            let yb = tb.target_rect.map(|r| r[1] + r[3]).unwrap_or(f32::MAX);
            position(ta.target_page, ya)
                .partial_cmp(&position(tb.target_page, yb))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let definitions = match preceding {
        Some(i) => vec![i],
        None => same.to_vec(),
    };
    Some(TermHit {
        range: (start, end),
        rects,
        definitions,
    })
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::super::page::text_of;
    use super::super::regions::{classify, RegionState};
    use super::*;

    fn term(text: &str, page: u32, rect: Option<Rect>) -> TermInput {
        TermInput {
            text: text.into(),
            case_sensitive: true,
            kind: Some("acronym".into()),
            title: text.into(),
            body: format!("long form of {text}"),
            target_page: page,
            target_rect: rect,
        }
    }

    fn hits(line: &str, terms: &[TermInput]) -> Vec<String> {
        let mut b = Builder::new();
        b.line(700.0, BODY, line);
        let page = b.page();
        let all = text_of(&page);
        find_terms(&page, 5, terms)
            .into_iter()
            .map(|h| {
                all.chars()
                    .skip(h.range.0)
                    .take(h.range.1 - h.range.0)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn pathology_plurals_and_suffixes() {
        let terms = [term("FDT", 0, None)];
        assert_eq!(
            hits("the FDTs and FDT's and FDT-based and non-FDT FDT.", &terms),
            vec!["FDT", "FDT", "FDT", "FDT", "FDT"]
        );
    }

    #[test]
    fn pathology_whole_tokens_only_and_case_sensitive() {
        assert!(hits("AND IT US and it", &[term("FDT", 0, None)]).is_empty());
        let terms = [term("IT", 0, None)];
        assert_eq!(
            hits("IT and it and ITS and ITEM and ITs", &terms),
            vec!["IT", "IT"]
        );
        assert!(hits("SFDT and FDTX and FDTss", &[term("FDT", 0, None)]).is_empty());
    }

    #[test]
    fn pathology_mixed_case_and_digits() {
        let terms = [
            term("mRNA", 0, None),
            term("QoL", 0, None),
            term("COVID-19", 0, None),
            term("H2O", 0, None),
            term("2D", 0, None),
        ];
        assert_eq!(
            hits(
                "mRNA QoL COVID-19 H2O 2D but not MRNA qol COVID-198 H2Ox 12D",
                &terms
            ),
            vec!["mRNA", "QoL", "COVID-19", "H2O", "2D"]
        );
    }

    #[test]
    fn pathology_single_letters_are_never_terms() {
        let kept = sanitize(vec![
            term("A", 0, None),
            term("AB", 0, None),
            term(" X", 0, None),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "AB");
    }

    #[test]
    fn pathology_defined_twice_takes_the_nearest_preceding_or_both() {
        let terms = [
            term("PS", 1, Some([72.0, 600.0, 50.0, 10.0])),
            term("PS", 7, Some([72.0, 600.0, 50.0, 10.0])),
        ];
        let mut b = Builder::new();
        b.line(700.0, BODY, "the PS here");
        let page = b.page();
        // on page 5: only the page-1 definition precedes
        assert_eq!(find_terms(&page, 5, &terms)[0].definitions, vec![0]);
        // on page 9: the page-7 one is nearer
        assert_eq!(find_terms(&page, 9, &terms)[0].definitions, vec![1]);
        // on page 0: neither precedes - both
        assert_eq!(find_terms(&page, 0, &terms)[0].definitions, vec![0, 1]);
    }

    #[test]
    fn pathology_use_before_definition_still_shows() {
        let terms = [term("NESS", 4, Some([72.0, 600.0, 50.0, 10.0]))];
        let mut b = Builder::new();
        b.line(700.0, BODY, "the NESS here");
        assert_eq!(find_terms(&b.page(), 1, &terms)[0].definitions, vec![0]);
    }

    #[test]
    fn pathology_the_definition_site_is_not_hoverable() {
        // With a rectangle.
        let mut b = Builder::new();
        b.line(700.0, BODY, "steady state (NESS) and NESS");
        let page = b.page();
        let def = page.rects(0, 19)[0];
        let terms = [term("NESS", 5, Some(def))];
        assert_eq!(find_terms(&page, 5, &terms).len(), 1);
        // Without one: the parenthesised occurrence on the defining page.
        let terms = [term("NESS", 5, None)];
        assert_eq!(find_terms(&page, 5, &terms).len(), 1);
        assert_eq!(
            hits("NESS (nonequilibrium steady state) then NESS", &terms).len(),
            1
        );
    }

    #[test]
    fn pathology_a_second_definition_elsewhere_is_not_hoverable_either() {
        let terms = [term("NESS", 0, Some([72.0, 100.0, 50.0, 10.0]))];
        let mut b = Builder::new();
        b.line(
            700.0,
            BODY,
            "again the nonequilibrium steady state (NESS) and the NESS",
        );
        let page = b.page();
        let found = find_terms(
            &page,
            4,
            &[TermInput {
                body: "nonequilibrium steady state".into(),
                ..terms[0].clone()
            }],
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].range.0 > 50);
    }

    #[test]
    fn pathology_a_subscript_or_inline_math_is_not_a_term() {
        let terms = [term("ERM", 0, None)];
        let mut b = Builder::new();
        // f_ERM: a math f, then ERM in a smaller roman size
        let x = b.text(72.0, 700.0, BODY, "the output ");
        let x = b.text(x, 700.0, MATH, "f");
        b.text(
            x,
            698.5,
            Style {
                size: 7.0,
                font: 1,
                bold: false,
                math: false,
            },
            "ERM",
        );
        b.newline();
        b.line(680.0, BODY, "and every ERM algorithm");
        let page = b.page();
        let found = find_terms(&page, 3, &terms);
        assert_eq!(found.len(), 1);
        assert!(found[0].rects[0][1] < 690.0);
    }

    #[test]
    fn pathology_not_in_the_bibliography_or_math() {
        let terms = [term("JAM", 0, None), term("AM", 0, None)];
        let mut b = Builder::new();
        b.line(700.0, BODY, "Body text with AM in it.");
        b.line(660.0, BOLD, "References");
        b.line(640.0, BODY, "[1] Smith AM, J. A. M. Jones, JAM 4 (2001).");
        b.text(72.0, 600.0, MATH, "AM=1");
        b.newline();
        let mut page = b.page();
        classify(&mut page, &mut RegionState::default());
        let found = find_terms(&page, 3, &terms);
        assert_eq!(found.len(), 1);
        assert!(found[0].rects[0][1] > 690.0);
    }
}

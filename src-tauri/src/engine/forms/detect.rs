//! Auto-detect for Prepare form mode: fields proposed from a printed form's
//! blanks.
//!
//! The blanks are `blanks::detect`'s, the same the Fill tool snaps to:
//! underlines, boxes and table cells become text fields at the font size
//! the blank suggests, comb rows text fields with one cell per character,
//! and small squares checkboxes. Each is named from the label nearest it -
//! to its left on the same line, above it, or below a signature rule; for a
//! checkbox, to its right - with numbering and parenthetical notes dropped,
//! and made unique in the document.

use crate::engine::blanks::{Glyph, PageInput};
use crate::engine::types::{BlankDto, ProposedFieldDto, RectDto};
use std::collections::HashSet;

/// A run of words on one line, as a label candidate.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Phrase {
    pub text: String,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub size: f32,
}

/// Group a page's characters into phrases: lines split where the gap
/// between characters is wider than about two spaces.
pub(crate) fn phrases(glyphs: &[Glyph]) -> Vec<Phrase> {
    let mut sorted: Vec<&Glyph> = glyphs
        .iter()
        .filter(|g| g.b.x1 > g.b.x0 || g.ch == ' ')
        .collect();
    sorted.sort_by(|a, b| {
        let (ca, cb) = ((a.b.y0 + a.b.y1) / 2.0, (b.b.y0 + b.b.y1) / 2.0);
        cb.total_cmp(&ca).then(a.b.x0.total_cmp(&b.b.x0))
    });
    // Lines: characters whose vertical centres are close.
    let mut lines: Vec<Vec<&Glyph>> = Vec::new();
    for g in sorted {
        let cy = (g.b.y0 + g.b.y1) / 2.0;
        let size = g.size.max(4.0);
        let joined = lines.iter_mut().rev().take(3).find(|line| {
            let first = line[0];
            let fy = (first.b.y0 + first.b.y1) / 2.0;
            (fy - cy).abs() < 0.4 * size.min(first.size.max(4.0))
        });
        match joined {
            Some(line) => line.push(g),
            None => lines.push(vec![g]),
        }
    }
    let mut out = Vec::new();
    for mut line in lines {
        line.sort_by(|a, b| a.b.x0.total_cmp(&b.b.x0));
        let mut current: Option<Phrase> = None;
        for g in line {
            if g.ch == ' ' {
                if let Some(p) = current.as_mut() {
                    if !p.text.ends_with(' ') {
                        p.text.push(' ');
                    }
                }
                continue;
            }
            let size = g.size.max(4.0);
            match current.as_mut() {
                Some(p) if g.b.x0 - p.x1 < 1.6 * size => {
                    if g.b.x0 - p.x1 > 0.2 * size && !p.text.ends_with(' ') {
                        p.text.push(' ');
                    }
                    p.text.push(g.ch);
                    p.x1 = p.x1.max(g.b.x1);
                    p.y0 = p.y0.min(g.b.y0);
                    p.y1 = p.y1.max(g.b.y1);
                }
                _ => {
                    if let Some(done) = current.take() {
                        out.push(done);
                    }
                    current = Some(Phrase {
                        text: g.ch.to_string(),
                        x0: g.b.x0,
                        y0: g.b.y0,
                        x1: g.b.x1,
                        y1: g.b.y1,
                        size,
                    });
                }
            }
        }
        if let Some(done) = current {
            out.push(done);
        }
    }
    for p in &mut out {
        p.text = p.text.trim().to_string();
    }
    out.retain(|p| p.text.chars().any(char::is_alphanumeric));
    out
}

/// The label text nearest a blank, if any is close enough.
fn nearest_label<'p>(blank: &BlankDto, phrases: &'p [Phrase]) -> Option<&'p Phrase> {
    let r = &blank.rect;
    let (bx0, by0, bx1, by1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let check = blank.kind == "check";
    let mut best: Option<(f32, &Phrase)> = None;
    for p in phrases {
        let size = p.size;
        let cy = (p.y0 + p.y1) / 2.0;
        // Same line: the phrase's middle within the blank's band, widened
        // for an underline (its rect is the room above the rule).
        let band = (by0 - 0.6 * size, by1 + 0.4 * size);
        let same_line = cy > band.0 && cy < band.1;
        let h_overlap = p.x0 < bx1 && p.x1 > bx0;
        let mut score = f32::INFINITY;
        if same_line && p.x1 <= bx0 + 2.0 {
            let gap = bx0 - p.x1;
            if gap < 220.0 {
                score = gap * if check { 1.6 } else { 1.0 };
            }
        }
        if same_line && p.x0 >= bx1 - 2.0 {
            let gap = p.x0 - bx1;
            if gap < 220.0 {
                score = score.min(gap * if check { 0.5 } else { 3.0 } + 4.0);
            }
        }
        if !check && h_overlap && p.y0 >= by1 - 2.0 {
            let gap = p.y0 - by1;
            if gap < 2.5 * size {
                score = score.min(gap * 2.0 + (p.x0 - bx0).max(0.0) * 0.15 + 2.0);
            }
        }
        if !check && h_overlap && p.y1 <= by0 + 2.0 {
            let gap = by0 - p.y1;
            if gap < 1.6 * size {
                score = score.min(gap * 3.0 + 8.0);
            }
        }
        if score.is_finite() && best.is_none_or(|(s, _)| score < s) {
            best = Some((score, p));
        }
    }
    best.map(|(_, p)| p)
}

/// A label as a field name: numbering, notes in parentheses, trailing
/// punctuation and filler dropped; a few words at most.
pub(crate) fn clean_label(text: &str) -> String {
    // Notes in parentheses go: "Name (as shown on your return)".
    let mut s = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = s
        .split(|c: char| c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    // Leading enumerators: "1", "1a", "2b.", "a.", "Line", "Part I".
    let mut start = 0;
    while start < words.len() {
        let w = words[start].trim_end_matches(['.', ':', ')']);
        let short = w.chars().count() <= 3;
        let enumerator = short
            && (w.chars().any(|c| c.is_ascii_digit())
                || (w.chars().count() == 1 && w.chars().all(|c| c.is_ascii_lowercase())));
        if enumerator || (start == 0 && words.len() > 1 && w.eq_ignore_ascii_case("line")) {
            start += 1;
        } else {
            break;
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut length = 0;
    for w in &words[start..] {
        let cleaned: String = w
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '\'' | '-' | '/' | '&' | '#'))
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        if out.len() >= 6 || length + cleaned.len() > 40 {
            break;
        }
        length += cleaned.len() + 1;
        out.push(cleaned);
    }
    let mut name = out.join(" ");
    // A label running into a sentence: keep it to its first clause.
    if let Some(i) = name.find(" if ").or_else(|| name.find(" see ")) {
        name.truncate(i);
    }
    // Sentence case, as a person would type the name.
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `name` made unique against `taken`, which it is then added to.
pub(crate) fn unique_name(name: &str, fallback: &str, taken: &mut HashSet<String>) -> String {
    let base = if name.is_empty() { fallback } else { name };
    let mut candidate = base.to_string();
    let mut n = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base} {n}");
        n += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

/// Fields proposed for one page's blanks, named against `taken` (the
/// document's field names so far).
pub fn propose(
    page: u32,
    input: &PageInput,
    blanks: &[BlankDto],
    taken: &mut HashSet<String>,
) -> Vec<ProposedFieldDto> {
    let labels = phrases(&input.glyphs);
    blanks
        .iter()
        .map(|blank| {
            let label = nearest_label(blank, &labels)
                .map(|p| p.text.clone())
                .unwrap_or_default();
            let check = blank.kind == "check";
            let name = unique_name(
                &clean_label(&label),
                if check { "Check Box" } else { "Text" },
                taken,
            );
            let comb = blank.kind == "comb" && blank.cells > 1;
            let font = if check { 0.0 } else { blank.font_size_pt };
            let multiline = matches!(blank.kind.as_str(), "box" | "cell")
                && blank.rect.h >= 3.0 * blank.font_size_pt.max(6.0)
                && blank.rect.w >= 60.0;
            ProposedFieldDto {
                page,
                rect: RectDto { ..blank.rect },
                kind: if check { "checkbox" } else { "text" }.into(),
                name,
                label: label.chars().take(120).collect(),
                font_size: font,
                max_len: if comb { blank.cells } else { 0 },
                comb,
                multiline,
                blank: blank.kind.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::blanks::Bounds;

    fn word(text: &str, x: f32, y: f32, size: f32) -> Vec<Glyph> {
        let w = size * 0.5;
        text.chars()
            .enumerate()
            .map(|(i, ch)| Glyph {
                ch,
                b: Bounds {
                    x0: x + i as f32 * w,
                    y0: y,
                    x1: x + i as f32 * w + if ch == ' ' { 0.0 } else { w * 0.9 },
                    y1: y + size * 0.7,
                },
                size,
            })
            .collect()
    }

    fn blank(kind: &str, x: f32, y: f32, w: f32, h: f32) -> BlankDto {
        BlankDto {
            rect: RectDto { x, y, w, h },
            kind: kind.into(),
            font_size_pt: 10.0,
            cells: if kind == "comb" { 9 } else { 0 },
        }
    }

    #[test]
    fn cleans_labels() {
        assert_eq!(
            clean_label("1a Name (as shown on your income tax return)"),
            "Name"
        );
        assert_eq!(clean_label("Date of birth:"), "Date of birth");
        assert_eq!(clean_label("b. first name"), "First name");
        assert_eq!(
            clean_label("Line 5 City, state, and ZIP code"),
            "City state and ZIP code"
        );
        assert_eq!(clean_label("..."), "");
    }

    #[test]
    fn groups_words_into_phrases() {
        let mut glyphs = word("Name", 50.0, 700.0, 10.0);
        glyphs.extend(word("Date of birth", 300.0, 700.0, 10.0));
        let p = phrases(&glyphs);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].text, "Name");
        assert_eq!(p[1].text, "Date of birth");
    }

    #[test]
    fn names_from_nearest_label() {
        let mut glyphs = word("Name", 50.0, 700.0, 10.0);
        glyphs.extend(word("Date of birth", 300.0, 700.0, 10.0));
        glyphs.extend(word("Married", 80.0, 650.0, 10.0));
        glyphs.extend(word("Social security number", 50.0, 612.0, 9.0));
        let input = PageInput {
            glyphs,
            ..PageInput::default()
        };
        let blanks = vec![
            blank("line", 75.0, 699.0, 200.0, 12.0),
            blank("line", 370.0, 699.0, 150.0, 12.0),
            blank("check", 66.0, 649.0, 9.0, 9.0),
            blank("comb", 50.0, 590.0, 120.0, 16.0),
        ];
        let mut taken: HashSet<String> = ["Name".to_string()].into();
        let got = propose(0, &input, &blanks, &mut taken);
        assert_eq!(got[0].name, "Name 2");
        assert_eq!(got[1].name, "Date of birth");
        assert_eq!(got[2].name, "Married");
        assert_eq!(got[2].kind, "checkbox");
        assert_eq!(got[3].name, "Social security number");
        assert!(got[3].comb);
        assert_eq!(got[3].max_len, 9);
        let mut again = taken.clone();
        assert_eq!(unique_name("", "Text", &mut again), "Text");
        assert_eq!(unique_name("", "Text", &mut again), "Text 2");
    }
}

//! Every preview kind checked against the page it shows, read-only, over
//! real PDFs. Ignored by default:
//!
//! ```text
//! SPEEDYF_HOVER_EVAL=list.txt SPEEDYF_HOVER_OUT=report.txt \
//!   cargo test --lib hovers::eval_links -- --ignored --nocapture
//! ```
//!
//! Link previews are classified by the words of the link and just before it
//! ("Theorem 2", "Sec. 3", "Eq. (4)", "[12]", "Fig. 5"); text-detected hover
//! spans by their kind. The target is then found on the destination page
//! independently of the preview code - a caption, statement or section
//! head, an equation's number, a bibliography entry's first line - and its
//! extent read off the page's lines. A preview is right when it holds all
//! of the target and none of what surrounds it: the statement's proof, the
//! next reference, the previous section's tail, a line cut in half.
//!
//! Every preview - classified or not, floats too - is also checked for the
//! invariants `settle` keeps, against the displays the equation layer finds
//! (glyphs and raster): (a) a display it touches is in it whole, (b) a line
//! it touches is in it whole, (c) it is as wide as the target's lines and
//! the displays it touches ("inv a/b/c"). SPEEDYF_SWEEP=1 lists every
//! preview with whether they hold.

use super::page::{Line, Page};
use super::{extract, IndexBuilder};
use crate::engine::types::{LinkTarget, TextRun};
use crate::engine::{links, pdfium_init, render, text};
use pdfium_render::prelude::*;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const THEOREM_WORDS: &[&str] = &[
    "theorem",
    "lemma",
    "proposition",
    "corollary",
    "definition",
    "remark",
    "example",
    "assumption",
    "claim",
    "conjecture",
    "fact",
    "observation",
    "algorithm",
    "hypothesis",
    "condition",
    "property",
    "problem",
    "question",
];

/// What a link or span points at.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Citation(Option<String>),
    Theorem(String, String),
    Equation(String),
    Section(String),
    Float(String, String),
    Footnote(String),
}

impl Target {
    fn kind(&self) -> &'static str {
        match self {
            Target::Citation(_) => "citation",
            Target::Theorem(..) => "theorem",
            Target::Equation(_) => "equation",
            Target::Section(_) => "section",
            Target::Float(kind, _) if kind == "table" => "table",
            Target::Float(..) => "figure",
            Target::Footnote(_) => "footnote",
        }
    }
}

fn is_number(token: &str) -> bool {
    let t = token.trim_matches(|c: char| matches!(c, '.' | ':' | ',' | ')' | '('));
    !t.is_empty()
        && t.split('.').all(|p| {
            !p.is_empty()
                && (p.chars().all(|c| c.is_ascii_digit())
                    || (p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase())))
        })
}

fn clean_number(token: &str) -> String {
    token
        .trim_matches(|c: char| matches!(c, '.' | ':' | ',' | ')' | '(' | '[' | ']' | '~'))
        .to_string()
}

/// Read a link: its own text and the words before it.
fn classify_link(before: &str, inside: &str, rect: [f32; 4]) -> Option<Target> {
    let inside = super::targets::joined_number(inside.trim());
    let inside = inside.as_str();
    let words = format!("{} {}", before.trim_end(), inside);
    let lower = words.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let last_number = inside
        .split(|c: char| c.is_whitespace() || c == '~')
        .rev()
        .find(|t| is_number(t))
        .map(clean_number);
    // "Fig. 3", "Table 2"
    if let Some(number) = last_number.clone() {
        let n = tokens.len();
        let prior = |k: usize| tokens.get(n.wrapping_sub(k)).copied().unwrap_or("");
        for k in 2..=3 {
            // No bracket or sentence's end between the kind word and the
            // number: "for example [8]" is a citation, the mark after
            // "Section 3." a footnote.
            let sentence_end = prior(2).ends_with('.') && is_number(prior(2));
            if k == 3 && (prior(2).contains(['[', ']']) || sentence_end) {
                break;
            }
            let word = prior(k).trim_end_matches(['.', '~']);
            if matches!(word, "fig" | "figure" | "figs" | "figures") {
                return Some(Target::Float("figure".into(), number));
            }
            if matches!(word, "table" | "tables" | "tab") {
                return Some(Target::Float("table".into(), number));
            }
            if matches!(
                word,
                "section" | "sections" | "sec" | "secs" | "appendix" | "appendices" | "chapter"
            ) || word == "\u{a7}"
            {
                return Some(Target::Section(number));
            }
            if let Some(kind) = THEOREM_WORDS
                .iter()
                .find(|w| word == **w || word == format!("{w}s"))
            {
                // An algorithm is a float, judged by the float evaluation.
                if *kind == "algorithm" {
                    return Some(Target::Float("algorithm".into(), number));
                }
                return Some(Target::Theorem(kind.to_string(), number));
            }
            if matches!(word, "eq" | "eqs" | "equation" | "equations") {
                return Some(Target::Equation(number));
            }
        }
        if let Some(stripped) = inside.strip_prefix('\u{a7}') {
            return Some(Target::Section(clean_number(stripped)));
        }
    }
    // "(3)" alone: an equation; "[12]" or a bare number in brackets: a
    // citation; an author-year link: a citation.
    if inside.starts_with('(') && inside.ends_with(')') && is_number(inside) {
        return Some(Target::Equation(clean_number(inside)));
    }
    let bracketed = before.trim_end().ends_with('[') || inside.starts_with('[');
    if bracketed && inside.chars().any(|c| c.is_ascii_digit()) {
        return Some(Target::Citation(last_number));
    }
    let has_year = inside
        .split(|c: char| !c.is_ascii_digit())
        .any(|t| t.len() == 4 && (t.starts_with("19") || t.starts_with("20")));
    if has_year {
        return Some(Target::Citation(None));
    }
    // An author-year link on the names alone: "(Littlestone and Warmuth".
    let names = inside
        .split_whitespace()
        .all(|w| w.starts_with(char::is_uppercase) || matches!(w, "and" | "et" | "al." | "&"));
    let opens = before.trim_end().ends_with(['(', ';']);
    if names && opens && !inside.is_empty() {
        return Some(Target::Citation(None));
    }
    // A footnote mark: a lone small number.
    if is_number(inside) && inside.len() <= 2 && rect[3] < 7.0 {
        return Some(Target::Footnote(clean_number(inside)));
    }
    None
}

/// A hover span's target, by its kind and label.
fn classify_span(kind: &str, label: &str) -> Option<Target> {
    let number = clean_number(label.rsplit(' ').next()?);
    let word = label.split(' ').next().unwrap_or("").to_lowercase();
    Some(match kind {
        "equation" => Target::Equation(number),
        "section" => Target::Section(number),
        "figure" => Target::Float("figure".into(), number),
        "table" => Target::Float("table".into(), number),
        "footnote" => Target::Footnote(number),
        "theorem" => Target::Theorem(word, number),
        _ => return None,
    })
}

fn text_of(page: &Page, i: usize) -> String {
    page.line_text(i).trim().to_string()
}

/// The gap a page leaves between its lines of prose.
fn leading_gap(page: &Page) -> f32 {
    let mut gaps: Vec<f32> = page
        .lines
        .windows(2)
        .filter(|w| super::page::same_column(&w[0], &w[1]) && w[0].bottom > w[1].top)
        .map(|w| w[0].bottom - w[1].top)
        .filter(|g| *g < 20.0)
        .collect();
    gaps.sort_by(f32::total_cmp);
    gaps.get(gaps.len() / 2).copied().unwrap_or(3.0)
}

fn starts_statement(text: &str) -> bool {
    let lower = text.to_lowercase();
    let mut tokens = lower.split_whitespace();
    let (Some(word), Some(number)) = (tokens.next(), tokens.next()) else {
        return false;
    };
    // A head, not a mention running on at a line's start ("Theorem 16,
    // and let", "Definition 6). Then", "Lemma 15 gives").
    let original = text.split_whitespace().nth(2).unwrap_or("");
    THEOREM_WORDS.contains(&word.trim_end_matches('.'))
        && is_number(number)
        && !number.ends_with([',', ')', ';'])
        && !original.starts_with(char::is_lowercase)
}

fn starts_section(text: &str) -> bool {
    let mut tokens = text.split_whitespace();
    let (Some(number), Some(word)) = (tokens.next(), tokens.next()) else {
        return false;
    };
    // A heading is short and ends without a full stop; an enumerated
    // sentence ("1. A is private.") is neither.
    let short = text.split_whitespace().count() <= 12 && !text.trim_end().ends_with('.');
    // "3 A framework for ...": a one-letter first word, more words after.
    let worded = word.chars().filter(|c| c.is_alphabetic()).count() >= 2
        || text.split_whitespace().count() >= 4;
    is_number(number)
        && !number.starts_with('(')
        && word.starts_with(|c: char| c.is_uppercase())
        && worded
        && short
}

/// An enumerated or itemised line: "(i)", "(a)", "(3)", "1.", "b)", a
/// bullet.
fn starts_item(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with(['\u{2022}', '\u{2013}', '\u{00b7}', '*']) {
        return true;
    }
    let label: String = t.chars().take_while(|c| !c.is_whitespace()).collect();
    let inner = match (label.strip_prefix('('), label.strip_suffix(')')) {
        (Some(rest), _) => match rest.strip_suffix(')') {
            Some(inner) => inner.to_string(),
            None => return false,
        },
        (None, Some(inner)) => inner.to_string(),
        (None, None) => match label.strip_suffix('.') {
            Some(inner) => inner.to_string(),
            None => return false,
        },
    };
    let roman = !inner.is_empty() && inner.chars().all(|c| matches!(c, 'i' | 'v' | 'x'));
    let letter = inner.len() == 1 && inner.chars().all(|c| c.is_ascii_lowercase());
    let digits = !inner.is_empty() && inner.len() <= 2 && inner.chars().all(|c| c.is_ascii_digit());
    roman || letter || digits
}

/// A page's foot: its number alone on a line.
fn is_page_number(text: &str) -> bool {
    !text.is_empty() && text.len() <= 4 && text.chars().all(|c| c.is_ascii_digit())
}

/// The line `i` is followed by, in reading order.
fn next_line(page: &Page, i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < page.lines.len() && page.lines[j].flags.furniture {
        j += 1;
    }
    (j < page.lines.len()).then_some(j)
}

/// The next line of body text after `i` in reading order: past running
/// heads and feet, page numbers, and the smaller type of footnotes.
fn next_body_line(page: &Page, i: usize) -> Option<usize> {
    next_line_of_size(page, i, page.body_size)
}

/// The next line after `i` in reading order set at least about `size`:
/// past running heads and feet, page numbers and smaller type.
fn next_line_of_size(page: &Page, i: usize, size: f32) -> Option<usize> {
    let mut j = i;
    loop {
        j = next_line(page, j)?;
        let l = &page.lines[j];
        let small = size > 0.0 && l.size > 0.0 && l.size < 0.92 * size;
        if !small && !is_page_number(&text_of(page, j)) {
            return Some(j);
        }
    }
}

/// The first line of a page set at least about `size`, in reading order.
fn first_line_of_size(page: &Page, size: f32) -> Option<usize> {
    let first = (0..page.lines.len()).find(|&i| !page.lines[i].flags.furniture)?;
    let l = &page.lines[first];
    let small = size > 0.0 && l.size > 0.0 && l.size < 0.92 * size;
    if small || is_page_number(&text_of(page, first)) {
        next_line_of_size(page, first, size)
    } else {
        Some(first)
    }
}

/// A line of the target: on the destination page (0) or the next (1).
type At = (usize, usize);

/// Where the target is and how far it runs: its lines - over a column or
/// page break too - and the lines that must stay out (the proof after it,
/// the next entry, the line before a heading).
struct Truth {
    lines: Vec<At>,
    outside: Vec<At>,
}

fn head_near(page: &Page, heads: Vec<usize>, anchor: Option<f32>) -> Option<usize> {
    match anchor {
        Some(y) => heads.into_iter().min_by(|&a, &b| {
            (page.lines[a].top - y)
                .abs()
                .total_cmp(&(page.lines[b].top - y).abs())
        }),
        None => heads.into_iter().next(),
    }
}

/// Where reading goes on after line `at`: the next body line, and whether
/// it opens a new column or page.
fn step(pages: &[&Page], at: At) -> Option<(At, bool)> {
    step_in(pages, at, None)
}

/// `step` through lines set at least about `size` (each page's body size
/// when None): a bibliography may be set smaller than the body.
fn step_in(pages: &[&Page], at: At, size: Option<f32>) -> Option<(At, bool)> {
    let (p, i) = at;
    let page = pages[p];
    match next_line_of_size(page, i, size.unwrap_or(page.body_size)) {
        // A new column starts above the line before it - wholly, not as an
        // accent or a script over that line does.
        Some(j) => Some((
            (p, j),
            page.lines[j].top > page.lines[i].top + 1.0
                && page.lines[j].bottom > page.lines[i].top + 2.0,
        )),
        None => {
            let next = pages.get(p + 1)?;
            let size = size.unwrap_or(next.body_size);
            Some(((p + 1, first_line_of_size(next, size)?), true))
        }
    }
}

/// Whether line `i` is a piece of a display: inside one of the page's
/// display boxes, or a lone operator or equation number set as a line of
/// its own ("sup", "max", ". (8.13)").
fn in_display(page: &Page, i: usize) -> bool {
    let l = &page.lines[i];
    let (cx, cy) = ((l.x0 + l.x1) / 2.0, (l.top + l.bottom) / 2.0);
    let boxed = page.displays.iter().any(|d| {
        cx >= d[0] - 1.0 && cx <= d[0] + d[2] + 1.0 && cy >= d[1] - 1.0 && cy <= d[1] + d[3] + 1.0
    });
    let text = text_of(page, i);
    let bare = text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ','));
    let operator = matches!(
        bare,
        "sup" | "inf" | "max" | "min" | "lim" | "limsup" | "liminf" | "argmax" | "argmin" | "log"
    );
    let tag = bare.starts_with('(')
        && bare.ends_with(')')
        && bare.len() <= 8
        && bare[1..bare.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.');
    boxed || operator || tag
}

fn ends_sentence(text: &str) -> bool {
    matches!(
        text.trim_end().chars().last(),
        Some('.' | '!' | '?' | ':' | ';')
    )
}

fn prose(l: &Line) -> bool {
    !l.is_display() && l.math_share < 0.2
}

/// A theorem-like statement: its label to the proof, the next statement or
/// heading, or the paragraph after it - back in the body face after an
/// italic statement, or set apart. Its lists ("(i)", "(a)", "1." - often
/// upright, set apart by the list's own skip, their further lines hanging
/// indented) and displays (with any lone script or symbol line of them)
/// are its own, and so is what it runs on into at the top of the next
/// column or page.
fn statement_truth(pages: &[&Page], head: usize) -> Truth {
    let page = pages[0];
    let lines = &page.lines;
    let left = |page: &Page, l: &Line| {
        page.lines
            .iter()
            .filter(|m| {
                !m.flags.furniture
                    && super::page::same_column(m, l)
                    && (m.top - l.top).abs() < 200.0
                    && m.x1 - m.x0 > 0.5 * (l.x1 - l.x0).max(1.0)
            })
            .map(|m| m.x0)
            .fold(l.x0, f32::min)
    };
    // The statement's face: that of the head line's own words - past its
    // label, not mathematics, not a name in parentheses - when it has
    // enough of them; else its first line of prose that is not an item.
    let mut face: Option<u16> = None;
    let mut italic = {
        let l = &lines[head];
        let mut depth = 0i32;
        let mut fonts = Vec::new();
        let mut tokens = 0;
        let mut previous = ' ';
        for i in l.start..l.end {
            let c = page.text[i];
            if previous == ' ' && c != ' ' {
                tokens += 1;
            }
            previous = c;
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                c if c.is_alphabetic() && depth <= 0 && tokens > 2 => {
                    if let Some(g) = page.glyph_at(i).filter(|g| !g.math) {
                        fonts.push(g.font);
                    }
                }
                _ => {}
            }
        }
        // The face itself: the commonest of the head's words not the body's.
        let mut count: BTreeMap<u16, usize> = BTreeMap::new();
        for &f in fonts.iter().filter(|&&f| f != page.body_font) {
            *count.entry(f).or_default() += 1;
        }
        face = count.into_iter().max_by_key(|&(_, n)| n).map(|(f, _)| f);
        (fonts.len() >= 12).then(|| {
            let own = fonts.iter().filter(|&&f| f != page.body_font).count();
            2 * own > fonts.len()
        })
    };
    let mut k = head;
    for _ in 0..4 {
        if italic.is_some() {
            break;
        }
        let Some(j) = next_body_line(page, k) else {
            break;
        };
        let t = text_of(page, j);
        if prose(&lines[j]) && !starts_item(&t) && lines[j].x1 - lines[j].x0 > 60.0 {
            italic = Some(lines[j].font != page.body_font);
            face = Some(lines[j].font);
            break;
        }
        k = j;
    }
    let mut out = vec![(0, head)];
    let mut outside = Vec::new();
    let mut in_list = false;
    let mut at = (0, head);
    let mut margin = left(page, &lines[head]);
    while out.len() < 60 {
        let Some((next, broke)) = step(pages, at) else {
            break;
        };
        let (prev_page, prev_line) = (pages[at.0], &pages[at.0].lines[at.1]);
        let here = pages[next.0];
        let l = &here.lines[next.1];
        let t = text_of(here, next.1);
        let item = starts_item(&t);
        // "1. Dense family ..." in a statement is an item of its list, not
        // a heading: it follows a colon or other items, or runs long, in
        // the body's size.
        let listed = item
            && (here.body_size <= 0.0 || l.size <= 1.05 * here.body_size)
            && (in_list
                || text_of(prev_page, at.1).ends_with(':')
                || t.split_whitespace().count() > 6);
        let head_like = t.starts_with("Proof")
            || t.starts_with("PROOF")
            || starts_statement(&t)
            || (starts_section(&t) && !listed);
        let display = !prose(l) || t.chars().count() < 3 || in_display(here, next.1);
        let own_face = l.font != here.body_font;
        if broke {
            // A new column or page: more of the statement only if it reads
            // as such.
            // In a face of its own: that face again, a list's item, or a
            // display mid-sentence - not a figure's labels in some other
            // face at the top of the page.
            let open = !ends_sentence(&text_of(prev_page, at.1));
            let goes_on = !head_like
                && match italic {
                    Some(true) => item || face.is_some_and(|f| l.font == f) || (display && open),
                    _ => open,
                };
            if !goes_on {
                break;
            }
            margin = left(here, l);
            out.push(next);
            at = next;
            continue;
        }
        if head_like {
            outside.push(next);
            break;
        }
        let indented = l.x0 > margin + 3.0;
        let gap = prev_line.bottom - l.top;
        let apart = gap > leading_gap(here) + 3.5;
        let prev_display = !prose(prev_line)
            || text_of(prev_page, at.1).chars().count() < 4
            || in_display(prev_page, at.1);
        let take = if item {
            in_list = true;
            true
        } else if prose(l)
            && !display
            && italic == Some(true)
            && face.is_some_and(|f| l.font == f)
            && t.chars().count() >= 4
        {
            // The statement's own face goes on: it is the statement's
            // text, however far it is set from the line before.
            true
        } else if prose(l)
            && italic == Some(true)
            && !own_face
            && t.chars().count() >= 4
            && l.x0 <= margin + 25.0
        {
            // The body face again, at the margin (not a display's piece in
            // that face): an item's further lines, or the paragraph after.
            in_list && indented
        } else if apart && prev_display && !display && italic != Some(true) {
            // After a display an upright statement goes on unindented
            // ("where ..."), or mid-sentence; a new paragraph is not its.
            !indented || t.starts_with(char::is_lowercase)
        } else if !apart || display || prev_display || (in_list && indented) {
            // Close by, a display, or an item's further lines.
            true
        } else if in_list {
            in_list = false;
            italic == Some(true) || !indented
        } else {
            false
        };
        if std::env::var("SPEEDYF_TRUTH_DEBUG").is_ok() {
            eprintln!(
                "truth italic {italic:?} take {take} apart {apart} prev_display {prev_display} display {display} indented {indented} own {own_face} list {in_list} {t:?}"
            );
        }
        if !take {
            if t.chars().count() >= 3 {
                outside.push(next);
            }
            break;
        }
        if !indented && prose(l) && !item {
            in_list = false;
        }
        out.push(next);
        at = next;
    }
    Truth {
        lines: out,
        outside,
    }
}

fn truth(pages: &[&Page], target: &Target, anchor: Option<(Option<f32>, f32)>) -> Option<Truth> {
    let page = pages[0];
    let lines = &page.lines;
    let anchor_y = anchor.map(|a| a.1);
    let gap0 = leading_gap(page);
    let gap = |a: usize, b: usize| lines[a].bottom - lines[b].top;
    match target {
        Target::Theorem(word, number) => {
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let lower = text_of(page, i).to_lowercase();
                    let mut t = lower.split_whitespace();
                    // A head, not a sentence opening a line ("Lemma 10
                    // below shows"): no lowercase word after the number.
                    let original = text_of(page, i);
                    let after = original.split_whitespace().nth(2).unwrap_or("");
                    t.next().map(|w| w.trim_end_matches('.')) == Some(word.as_str())
                        && t.next().map(clean_number).as_deref() == Some(number.as_str())
                        && !after.starts_with(char::is_lowercase)
                })
                .collect();
            let head = head_near(page, heads, anchor_y)?;
            Some(statement_truth(pages, head))
        }
        Target::Section(number) => {
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = text_of(page, i);
                    let mut tokens = t.split_whitespace();
                    let first = tokens.next().map(clean_number);
                    let first = if first.as_deref() == Some("Appendix") {
                        tokens.next().map(clean_number)
                    } else {
                        first
                    };
                    first.as_deref() == Some(number.as_str()) && starts_section(&t)
                        || t.starts_with(&format!("Appendix {number}"))
                })
                .collect();
            let head = head_near(page, heads, anchor_y)?;
            let mut out = vec![(0, head)];
            let mut i = head;
            for _ in 0..2 {
                let Some(j) = next_body_line(page, i) else {
                    break;
                };
                if lines[j].top > lines[i].top + 1.0 {
                    break;
                }
                out.push((0, j));
                i = j;
            }
            // The line just above the heading, in its column: the previous
            // section's tail. Never a page number or running head.
            let above = (0..lines.len())
                .filter(|&k| {
                    k != head
                        && !lines[k].flags.furniture
                        && !is_page_number(&text_of(page, k))
                        && lines[k].bottom >= lines[head].top
                        && super::page::same_column(&lines[k], &lines[head])
                })
                .min_by(|&a, &b| lines[a].bottom.total_cmp(&lines[b].bottom));
            Some(Truth {
                lines: out,
                outside: above.into_iter().map(|k| (0, k)).collect(),
            })
        }
        Target::Equation(number) => {
            let tag = format!("({number})");
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = text_of(page, i);
                    t.ends_with(&tag) || t.starts_with(&tag)
                })
                .collect();
            let head = head_near(page, heads, anchor_y)?;
            // The display: the math lines set close above and below - the
            // other rows of an align or gather too, numbered or not.
            let mut out = vec![head];
            let (mut x0, mut x1) = (lines[head].x0, lines[head].x1);
            loop {
                let mut grew = false;
                let top = out.iter().map(|&i| lines[i].top).fold(f32::MIN, f32::max);
                let bottom = out
                    .iter()
                    .map(|&i| lines[i].bottom)
                    .fold(f32::MAX, f32::min);
                for (k, l) in lines.iter().enumerate() {
                    if out.contains(&k) || l.flags.furniture {
                        continue;
                    }
                    let overlaps = l.x1.min(x1) - l.x0.max(x0) > 0.0;
                    let near = (l.bottom >= bottom - 0.1 && l.bottom - top <= 0.6 * l.em())
                        || (l.top <= top + 0.1 && bottom - l.top <= 0.6 * l.em())
                        || (l.top <= top + 0.5 && l.bottom >= bottom - 0.5);
                    if overlaps && near && l.is_display() {
                        out.push(k);
                        x0 = x0.min(l.x0);
                        x1 = x1.max(l.x1);
                        grew = true;
                    }
                }
                if !grew || out.len() > 16 {
                    break;
                }
            }
            Some(Truth {
                lines: out.into_iter().map(|i| (0, i)).collect(),
                outside: Vec::new(),
            })
        }
        Target::Citation(number) => {
            let head = match number {
                Some(n) => {
                    let label = format!("[{n}]");
                    let heads: Vec<usize> = (0..lines.len())
                        .filter(|&i| text_of(page, i).starts_with(&label))
                        .collect();
                    head_near(page, heads, anchor_y)
                }
                None => None,
            };
            let head = match head {
                Some(h) => h,
                None => {
                    // The entry's first line: the first line at or below the
                    // anchor, by its x when the destination names one.
                    let (x, y) = anchor?;
                    let near: Vec<usize> = (0..lines.len())
                        .filter(|&i| {
                            let l = &lines[i];
                            !l.flags.furniture
                                && l.top <= y + 3.0
                                && l.top >= y - 30.0
                                && x.is_none_or(|x| l.x0 <= x + 30.0 && l.x1 >= x)
                        })
                        .collect();
                    // An anchor sits at or before an entry's first letter,
                    // never inside a line of another column; a line opening
                    // with a label starts an entry, a run-on line does not.
                    let starting: Vec<usize> = near
                        .iter()
                        .copied()
                        .filter(|&i| x.is_none_or(|x| lines[i].x0 >= x - 5.0))
                        .collect();
                    let near = if starting.is_empty() { near } else { starting };
                    let labelled: Vec<usize> = near
                        .iter()
                        .copied()
                        .filter(|&i| text_of(page, i).starts_with('['))
                        .collect();
                    let near = if labelled.is_empty() { near } else { labelled };
                    near.into_iter()
                        .max_by(|&a, &b| lines[a].top.total_cmp(&lines[b].top))?
                }
            };
            let mut out = vec![(0, head)];
            let mut outside = Vec::new();
            // Hanging: lines on the page that start at this entry's margin
            // go on indented.
            let indents = |a: usize, b: usize| {
                (3.0..=30.0).contains(&(lines[b].x0 - lines[a].x0))
                    && gap(a, b) <= gap0 + 4.0
                    && lines[b].top < lines[a].top
            };
            let hanging = (0..lines.len())
                .filter(|&k| {
                    (lines[k].x0 - lines[head].x0).abs() <= 1.5
                        && next_line(page, k).is_some_and(|j| indents(k, j))
                })
                .count()
                >= 2;
            let mut margin = lines[head].x0;
            let mut at = (0, head);
            // The last whole line: a piece of its row (a ligature splits
            // lines; an accent sits above) is read with it.
            let mut whole = (0, head);
            // The entries' own type, which may be smaller than the body's.
            let size = Some(lines[head].size.min(page.body_size));
            while out.len() < 24 {
                let Some((next, broke)) = step_in(pages, at, size) else {
                    break;
                };
                let here = pages[next.0];
                let (prev, l) = (&pages[whole.0].lines[whole.1], &here.lines[next.1]);
                let t = text_of(here, next.1);
                let centre = (l.top + l.bottom) / 2.0;
                if !broke
                    && next.0 == whole.0
                    && centre > prev.bottom
                    && centre < prev.top + 0.5 * prev.em()
                {
                    out.push(next);
                    at = next;
                    continue;
                }
                if broke {
                    // Over a break: more of the entry unless a new one
                    // starts there.
                    let left = here
                        .lines
                        .iter()
                        .filter(|m| !m.flags.furniture && super::page::same_column(m, l))
                        .map(|m| m.x0)
                        .fold(l.x0, f32::min);
                    let new_entry = t.starts_with('[') || (hanging && l.x0 <= left + 1.5);
                    if new_entry || !hanging && number.is_none() {
                        break;
                    }
                    margin = left;
                    out.push(next);
                    at = next;
                    whole = next;
                    continue;
                }
                let g = prev.bottom - l.top;
                let starts = t.starts_with('[')
                    || (hanging && l.x0 <= margin + 1.5)
                    || (!hanging && g > leading_gap(here) + 1.5);
                if starts {
                    outside.push(next);
                    break;
                }
                // A line far below is the page's foot.
                if g > leading_gap(here) + 6.0 {
                    break;
                }
                out.push(next);
                at = next;
                whole = next;
            }
            Some(Truth {
                lines: out,
                outside,
            })
        }
        Target::Footnote(mark) => {
            let heads: Vec<usize> = (0..lines.len())
                .filter(|&i| {
                    let t = text_of(page, i);
                    // A note's words after its mark: not a lone script.
                    t.starts_with(mark.as_str())
                        && !t[mark.len()..].starts_with(|c: char| c.is_ascii_digit())
                        && t[mark.len()..].trim().chars().count() >= 3
                        && prose(&lines[i])
                        && lines[i].bottom < 0.35 * page.height
                        && (page.body_size <= 0.0 || lines[i].size < page.body_size - 0.3)
                })
                .collect();
            let head = head_near(page, heads, None)?;
            let size = lines[head].size;
            let next_mark = |t: &str| {
                t.starts_with(|c: char| c.is_ascii_digit())
                    && t.chars().take_while(char::is_ascii_digit).count() <= 2
            };
            let mut out = vec![(0, head)];
            let mut at = (0, head);
            while out.len() < 16 {
                let (p, i) = at;
                let here = pages[p];
                let notes = |page: &Page, k: usize| {
                    let l = &page.lines[k];
                    !l.flags.furniture
                        && (l.size - size).abs() <= 0.5
                        && l.bottom < 0.35 * page.height
                };
                // The next line of a note: in its column, or - its column's
                // foot reached - the next column's foot, or the next page's.
                let following = next_line(here, i).filter(|&j| notes(here, j));
                let (next, broke) = match following {
                    Some(j) => ((p, j), here.lines[j].top > here.lines[i].top + 1.0),
                    None => {
                        let Some(next_page) = pages.get(p + 1) else {
                            break;
                        };
                        // Over the page only mid-sentence, into the next
                        // page's notes: lines of prose under all its body
                        // and displays, not a display's small scripts.
                        if p > 0 || ends_sentence(&text_of(here, i)) {
                            break;
                        }
                        let Some(j) = (0..next_page.lines.len())
                            .find(|&k| notes(next_page, k) && under_the_body(next_page, k))
                        else {
                            break;
                        };
                        ((p + 1, j), true)
                    }
                };
                let page_here = pages[next.0];
                let t = text_of(page_here, next.1);
                if next_mark(&t) {
                    break;
                }
                if !broke && gap_in(page_here, at, next) > 4.0 {
                    break;
                }
                out.push(next);
                at = next;
            }
            Some(Truth {
                lines: out,
                outside: Vec::new(),
            })
        }
        Target::Float(..) => None,
    }
}

/// Whether line `k` is prose set under every line of the body and every
/// display it shares an x with: a page's foot.
fn under_the_body(page: &Page, k: usize) -> bool {
    let l = &page.lines[k];
    let overlaps = |x0: f32, x1: f32| x1.min(l.x1) - x0.max(l.x0) > 0.0;
    prose(l)
        && text_of(page, k).chars().count() >= 3
        && page.lines.iter().all(|m| {
            let body = page.body_size > 0.0 && m.size >= 0.92 * page.body_size;
            m.flags.furniture
                || !(body || m.is_display())
                || !overlaps(m.x0, m.x1)
                || m.bottom > l.top
        })
        && page
            .displays
            .iter()
            .all(|d| !overlaps(d[0], d[0] + d[2]) || d[1] > l.top)
}

fn gap_in(page: &Page, a: At, b: At) -> f32 {
    page.lines[a.1].bottom - page.lines[b.1].top
}

#[derive(Default)]
struct Tally {
    checked: usize,
    no_truth: usize,
    missing: usize,
    start_cut: usize,
    leaks: usize,
    cut: usize,
    /// Targets over a column or page break.
    split: usize,
    /// Targets whose text after a display - "where ..." - is cut.
    after_display: usize,
    /// Previews breaking an invariant: a display touched but not whole (a),
    /// a line touched but not whole (b), narrower than the target's lines
    /// and displays (c).
    inv: [usize; 3],
}

impl Tally {
    fn add(&mut self, t: &Tally) {
        self.checked += t.checked;
        self.no_truth += t.no_truth;
        self.missing += t.missing;
        self.start_cut += t.start_cut;
        self.leaks += t.leaks;
        self.cut += t.cut;
        self.split += t.split;
        self.after_display += t.after_display;
        for k in 0..3 {
            self.inv[k] += t.inv[k];
        }
    }

    fn line(&self, name: &str) -> String {
        format!(
            "{name:<22} checked {:>5}  target-cut {:>4}  head-missing {:>4}  leaks {:>4}  cut-line {:>4}  inv a/b/c {:>3}/{:>3}/{:>3}  (split {}, after-display cut {}, no truth {})",
            self.checked,
            self.missing,
            self.start_cut,
            self.leaks,
            self.cut,
            self.inv[0],
            self.inv[1],
            self.inv[2],
            self.split,
            self.after_display,
            self.no_truth
        )
    }
}

/// A preview: its pieces, each on the destination page (0) or the next.
type Crops = Vec<(usize, [f32; 4])>;

/// Judge a preview: all of the target inside one of its pieces, its head
/// above all, nothing that must stay out, no line cut through.
fn judge(pages: &[&Page], crops: &Crops, truth: &Truth, tally: &mut Tally) -> Vec<String> {
    tally.checked += 1;
    if truth
        .lines
        .iter()
        .any(|&(p, i)| p > 0 || pages[0].lines[i].top > pages[0].lines[truth.lines[0].1].top + 1.0)
    {
        tally.split += 1;
    }
    let mut problems = Vec::new();
    let inside = |(p, i): At| {
        let l = &pages[p].lines[i];
        crops.iter().filter(|(cp, _)| *cp == p).any(|(_, crop)| {
            let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]);
            l.x0 >= cx0 - 2.0 && l.x1 <= cx1 + 2.0 && l.bottom >= cy0 - 1.5 && l.top <= cy1 + 1.5
        })
    };
    let share = |l: &Line, crop: &[f32; 4]| {
        let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]);
        if l.x1.min(cx1) - l.x0.max(cx0) <= 3.0 {
            return 0.0;
        }
        ((l.top.min(cy1) - l.bottom.max(cy0)) / l.height()).max(0.0)
    };
    let text = |(p, i): At| text_of(pages[p], i);
    // A lone script or symbol line is a display's fragment, not a line.
    let real = |at: At| text(at).chars().count() >= 3;
    let head = truth.lines[0];
    if !inside(head) {
        tally.start_cut += 1;
        problems.push(format!("HEAD missing: {:?}", text(head)));
    }
    // A line of text straight after a display, left out.
    let display = |(p, i): At| {
        let l = &pages[p].lines[i];
        l.is_display() || l.math_share >= 0.2 || !real((p, i))
    };
    if let Some(w) = truth.lines.windows(2).find(|w| {
        display(w[0]) && !display(w[1]) && text(w[1]).chars().count() >= 12 && !inside(w[1])
    }) {
        tally.after_display += 1;
        problems.push(format!("AFTER DISPLAY cut: {:?}", text(w[1])));
    }
    if let Some(&at) = truth.lines.iter().find(|&&at| !inside(at) && real(at)) {
        tally.missing += 1;
        problems.push(format!(
            "TARGET cut: {:?}{} ({} lines)",
            text(at),
            if at.0 > 0 { " on the next page" } else { "" },
            truth.lines.len()
        ));
    }
    if let Some(&at) = truth.outside.iter().find(|&&(p, i)| {
        real((p, i))
            && crops
                .iter()
                .any(|(cp, crop)| *cp == p && share(&pages[p].lines[i], crop) > 0.5)
    }) {
        tally.leaks += 1;
        problems.push(format!("LEAKS: {:?}", text(at)));
    }
    let cut = crops.iter().find_map(|(p, crop)| {
        let page = pages[*p];
        (0..page.lines.len())
            .find(|&i| {
                let l = &page.lines[i];
                let s = share(l, crop);
                !l.flags.furniture && s > 0.15 && s < 0.85 && text_of(page, i).chars().count() >= 3
            })
            .map(|i| text_of(page, i))
    });
    if let Some(line) = cut {
        tally.cut += 1;
        problems.push(format!("CUTS: {line:?}"));
    }
    problems
}

/// How far a line or display reaches into a crop: its horizontal overlap
/// in points and the share of its height inside.
fn reach(x0: f32, x1: f32, bottom: f32, top: f32, crop: &[f32; 4]) -> (f32, f32) {
    let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]);
    let ox = x1.min(cx1) - x0.max(cx0);
    let oy = (top.min(cy1) - bottom.max(cy0)).max(0.0);
    (ox, oy / (top - bottom).max(0.5))
}

/// Whether `[x0, x1] x [bottom, top]` lies inside a crop, give or take.
fn holds(x0: f32, x1: f32, bottom: f32, top: f32, crop: &[f32; 4]) -> bool {
    x0 >= crop[0] - 2.0
        && x1 <= crop[0] + crop[2] + 2.0
        && bottom >= crop[1] - 1.5
        && top <= crop[1] + crop[3] + 1.5
}

/// The invariants every preview crop keeps, checked independently of the
/// engine: (a) a display it touches is in it whole - limits, fractions,
/// number; (b) a line it touches is in it whole; (c) it is as wide as the
/// target's lines (those level with it) and the displays it touches. A
/// display is touched through a line of it (a line whose middle is inside
/// it) or by a good part of its height. One message per broken invariant.
fn invariants(
    pages: &[&Page],
    displays: &[&[[f32; 4]]],
    crops: &Crops,
    truth: Option<&Truth>,
    tally: &mut Tally,
) -> Vec<String> {
    let mut broken: [Option<String>; 3] = [None, None, None];
    let touched_line = |l: &Line, crop: &[f32; 4]| {
        let (ox, share) = reach(l.x0, l.x1, l.bottom, l.top, crop);
        !l.flags.furniture && l.end > l.start && ox > 3.0 && share > 0.15
    };
    for (p, crop) in crops {
        let page = pages[*p];
        let mut wide: Option<(f32, f32)> = None;
        let mut widen = |x0: f32, x1: f32| {
            wide = Some(wide.map_or((x0, x1), |(a, b)| (a.min(x0), b.max(x1))));
        };
        for d in displays.get(*p).copied().unwrap_or(&[]) {
            let (dx0, dx1, db, dt) = (d[0], d[0] + d[2], d[1], d[1] + d[3]);
            let member = page.lines.iter().any(|l| {
                let (cx, cy) = ((l.x0 + l.x1) / 2.0, (l.top + l.bottom) / 2.0);
                cx >= dx0 - 1.0
                    && cx <= dx1 + 1.0
                    && cy >= db - 1.0
                    && cy <= dt + 1.0
                    && touched_line(l, crop)
            });
            let (ox, share) = reach(dx0, dx1, db, dt, crop);
            let touched = member || (ox > 3.0 && share > 0.25 && share * (dt - db) > 3.0);
            if !touched {
                continue;
            }
            widen(dx0, dx1);
            if broken[0].is_none() && !holds(dx0, dx1, db, dt, crop) {
                broken[0] = Some(format!(
                    "INV-A display [{dx0:.0},{db:.0},{dx1:.0},{dt:.0}] cut"
                ));
            }
        }
        for (i, l) in page.lines.iter().enumerate() {
            if broken[1].is_none()
                && touched_line(l, crop)
                && !holds(l.x0, l.x1, l.bottom, l.top, crop)
            {
                broken[1] = Some(format!("INV-B line cut: {:?}", text_of(page, i)));
            }
        }
        if let Some(truth) = truth {
            // The crop's columns: those it covers a good part of. A target
            // line in another column is a target cut, not a narrow crop.
            let columns =
                crate::engine::preview::page_columns(&super::xref::runs_of(page), page.width);
            let (cx0, cx1) = (crop[0], crop[0] + crop[2]);
            let own: Vec<(f32, f32)> = columns
                .iter()
                .filter(|c| {
                    let overlap = c.end.min(cx1) - c.start.max(cx0);
                    overlap >= 0.5 * (c.end - c.start).min(cx1 - cx0)
                })
                .map(|c| (c.start, c.end))
                .collect();
            for &(tp, i) in &truth.lines {
                if tp != *p {
                    continue;
                }
                let l = &page.lines[i];
                let centre = (l.top + l.bottom) / 2.0;
                let mid = (l.x0 + l.x1) / 2.0;
                let in_own = own.is_empty()
                    || own.iter().any(|&(a, b)| mid >= a - 2.0 && mid <= b + 2.0)
                    || (l.x1.min(cx1) - l.x0.max(cx0)) > 0.0;
                if in_own && centre >= crop[1] && centre <= crop[1] + crop[3] {
                    widen(l.x0, l.x1);
                }
            }
        }
        if let Some((x0, x1)) = wide {
            let (x0, x1) = (x0.max(0.0), x1.min(page.width));
            if broken[2].is_none() && (crop[0] > x0 + 2.0 || crop[0] + crop[2] < x1 - 2.0) {
                broken[2] = Some(format!(
                    "INV-C narrow: [{:.0},{:.0}] of [{x0:.0},{x1:.0}]",
                    crop[0],
                    crop[0] + crop[2]
                ));
            }
        }
    }
    let mut out = Vec::new();
    for (k, b) in broken.into_iter().enumerate() {
        if let Some(b) = b {
            tally.inv[k] += 1;
            out.push(b);
        }
    }
    out
}

/// A link's preview as the engine shows it: its crop, and the pieces the
/// target runs on into.
#[allow(clippy::too_many_arguments)]
fn link_pieces(
    page: &Page,
    next: Option<&Page>,
    runs: &[TextRun],
    w: f32,
    h: f32,
    x: Option<f32>,
    y: Option<f32>,
    words: &super::targets::LinkText,
) -> Crops {
    let target = super::targets::link_preview(Some(page), runs, w, h, x, y, Some(words));
    let mut crops = vec![(0, target.rect)];
    if let Some(flow) = target.flow {
        for piece in super::continuation::continuation(flow, page, next, target.rect) {
            crops.push((usize::from(piece.next_page), piece.rect));
        }
    }
    crops
}

/// Where the engine sends a link whose destination does not carry the
/// number its words name.
fn corrected(
    labels: &LabelMap,
    page: &Page,
    src: u32,
    rect: [f32; 4],
    words: &super::targets::LinkText,
) -> Option<(u32, f32, f32)> {
    super::targets::corrected_target(labels, page, src, rect, words)
}

/// A hover span's preview: its region, and the pieces it runs on into.
fn span_pieces(kind: &str, page: &Page, next: Option<&Page>, region: [f32; 4]) -> Crops {
    // The worker settles the index's region against the page's displays.
    let region = if matches!(kind, "figure" | "table" | "algorithm") {
        region
    } else {
        super::settle::settle(page, region)
    };
    let flow = match kind {
        "theorem" => Some(super::targets::Flow::Statement),
        "footnote" => Some(super::targets::Flow::Footnote),
        _ => None,
    };
    let mut crops = vec![(0, region)];
    if let Some(flow) = flow {
        for piece in super::continuation::continuation(flow, page, next, region) {
            crops.push((usize::from(piece.next_page), piece.rect));
        }
    }
    crops
}

type Link = ([f32; 4], Option<f32>, Option<f32>, u32);

struct Doc {
    name: String,
    pages: Vec<Option<Page>>,
    runs: Vec<Vec<TextRun>>,
    sizes: Vec<[f32; 2]>,
    links: Vec<Vec<Link>>,
    spans: std::collections::HashMap<u32, Vec<super::HoverSpanDto>>,
    /// Every numbered definition, by kind and label: page and click point.
    labels: LabelMap,
    /// Each page's display equations, y-up, numbers joined.
    displays: Vec<Vec<[f32; 4]>>,
}

type LabelMap = std::collections::HashMap<(super::xref::Kind, String), Vec<(u32, f32, f32)>>;

/// The number a target names, as printed: "A.2", "8", "(12)".
fn target_number(target: &Target) -> Option<String> {
    match target {
        Target::Theorem(_, n) | Target::Section(n) => Some(n.clone()),
        Target::Equation(n) => Some(format!("({n})")),
        _ => None,
    }
}

/// Whether a crop's lines carry `number`, as a word of their text.
fn carries_number(page: &Page, crop: [f32; 4], number: &str) -> bool {
    (0..page.lines.len())
        .filter(|&i| {
            let l = &page.lines[i];
            let centre = (l.top + l.bottom) / 2.0;
            centre >= crop[1]
                && centre <= crop[1] + crop[3]
                && l.x1 > crop[0]
                && l.x0 < crop[0] + crop[2]
        })
        .any(|i| {
            let text = text_of(page, i);
            if number.starts_with('(') {
                return text.contains(number);
            }
            text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | ')'))
                .any(|t| t.trim_end_matches('.') == number)
        })
}

/// Time spent finding pages' displays, in microseconds.
static DISPLAY_TIME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn load(b: &dyn PdfiumLibraryBindings, path: &str) -> Option<Doc> {
    let raw = render::open_document(b, path, None).ok()?;
    let count = (b.FPDF_GetPageCount(raw).max(0) as u32).min(80);
    let name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut doc = Doc {
        name,
        pages: Vec::new(),
        runs: Vec::new(),
        sizes: Vec::new(),
        links: Vec::new(),
        spans: Default::default(),
        labels: Default::default(),
        displays: Vec::new(),
    };
    let mut builder = IndexBuilder::new(count);
    while let Some(src) = builder.next_page() {
        let size = render::page_display_size(b, raw, src as i32);
        let handle = b.FPDF_LoadPage(raw, src as i32);
        let model = match (size, handle.is_null()) {
            (Some([w, h]), false) => {
                let input = extract::extract(b, handle, w, h);
                let extracted = text::extract_page(b, handle, w, h);
                let tp = b.FPDFText_LoadPage(handle);
                let found =
                    links::extract_links(b, raw, handle, tp, [w, h], &extracted.boxes, count);
                if !tp.is_null() {
                    b.FPDFText_ClosePage(tp);
                }
                doc.links.push(
                    found
                        .into_iter()
                        .filter_map(|l| match l.target {
                            LinkTarget::Internal { page, x, y } => Some((l.rect, x, y, page)),
                            _ => None,
                        })
                        .collect(),
                );
                // As the engine reads it: the index's page with the
                // displays its glyphs show; a preview's with the raster's.
                let started = std::time::Instant::now();
                let mut page = Page::new(input);
                let quick = extract::index_displays(b, handle, &page);
                let full = extract::displays(b, handle, w, h, &extracted.runs, true);
                DISPLAY_TIME.fetch_add(
                    started.elapsed().as_micros() as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
                let mut shown = page.clone();
                shown.set_displays(full);
                doc.displays.push(shown.displays.clone());
                page.set_displays(quick);
                if std::env::var("SPEEDYF_NO_DISPLAYS").is_ok() {
                    page.displays.clear();
                    shown.displays.clear();
                }
                let page = (page, shown);
                doc.runs.push(extracted.runs);
                doc.sizes.push([w, h]);
                Some(page)
            }
            _ => {
                doc.links.push(Vec::new());
                doc.runs.push(Vec::new());
                doc.sizes.push([0.0, 0.0]);
                doc.displays.push(Vec::new());
                None
            }
        };
        if !handle.is_null() {
            b.FPDF_ClosePage(handle);
        }
        let (model, shown) = model.unzip();
        builder.add_page(model);
        doc.pages
            .push(shown.map(|p| super::prepare_page(p, Default::default())));
    }
    doc.spans = builder.finish().spans;
    for (src, page) in doc.pages.iter().enumerate() {
        let Some(page) = page else { continue };
        let mut page = page.clone();
        let regions = super::regions::classify(&mut page, &mut Default::default());
        for d in super::xref::find_definitions(&page, &regions, src as u32) {
            doc.labels
                .entry((d.kind, d.label.clone()))
                .or_default()
                .push((d.page, d.jump.0, d.jump.1));
        }
    }
    b.FPDF_CloseDocument(raw);
    Some(doc)
}

fn show_crops(crops: &Crops) -> String {
    crops
        .iter()
        .map(|(p, c)| {
            format!(
                "{}[{:.0},{:.0},{:.0},{:.0}]",
                if *p > 0 { "next " } else { "" },
                c[0],
                c[1],
                c[0] + c[2],
                c[1] + c[3]
            )
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Where a run of the page's characters sits: the characters whose glyphs
/// fall inside `rect`.
fn locate(page: &Page, rect: [f32; 4]) -> Option<(usize, usize)> {
    let inside: Vec<usize> = (0..page.text.len())
        .filter(|&i| {
            page.glyph_at(i).is_some_and(|g| {
                let cx = g.rect[0] + g.rect[2] / 2.0;
                let cy = g.rect[1] + g.rect[3] / 2.0;
                g.has_box()
                    && cx >= rect[0]
                    && cx <= rect[0] + rect[2]
                    && cy >= rect[1]
                    && cy <= rect[1] + rect[3]
            })
        })
        .collect();
    Some((*inside.first()?, *inside.last()? + 1))
}

#[test]
#[ignore]
fn evaluate_all_previews() {
    let Ok(list) = std::env::var("SPEEDYF_HOVER_EVAL") else {
        return;
    };
    let out_path =
        std::env::var("SPEEDYF_HOVER_OUT").unwrap_or_else(|_| "preview-kinds.txt".into());
    let paths: Vec<String> = std::fs::read_to_string(list)
        .expect("list")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let mut totals: BTreeMap<String, Tally> = BTreeMap::new();
    let mut detail = String::new();
    // Every preview listed with whether its invariants hold.
    let sweep = std::env::var("SPEEDYF_SWEEP").is_ok();
    for path in paths {
        let Some(doc) = load(b, &path) else { continue };
        // link previews
        let mut seen = std::collections::HashSet::new();
        for (src, links_here) in doc.links.iter().enumerate() {
            let Some(source) = doc.pages[src].as_ref() else {
                continue;
            };
            for &(rect, x, y, to) in links_here {
                let Some((start, end)) = locate(source, rect) else {
                    continue;
                };
                let before: String = source.text[start.saturating_sub(16)..start]
                    .iter()
                    .collect();
                let inside: String = source.text[start..end].iter().collect();
                let classified = classify_link(&before, &inside, rect);
                let Some(Some(dest)) = doc.pages.get(to as usize) else {
                    continue;
                };
                // Destinations can be shared (a page-level one, the top of
                // the reference list): the link's text tells them apart.
                if !seen.insert((
                    to,
                    x.map(f32::to_bits),
                    y.map(f32::to_bits),
                    inside.trim().to_string(),
                    classified.as_ref().map(Target::kind),
                )) {
                    continue;
                }
                let words = super::targets::LinkText::from_runs(&doc.runs[src], rect);
                let old = std::env::var("SPEEDYF_OLD_CROP").is_ok();
                let [w, h] = doc.sizes[to as usize];
                let dest_next = doc.pages.get(to as usize + 1).and_then(Option::as_ref);
                let mut crops: Crops = if old {
                    vec![(
                        0,
                        crate::engine::preview::crop_rect(&crate::engine::preview::CropInput {
                            page_w_pt: w,
                            page_h_pt: h,
                            dest_x: x,
                            dest_y: y,
                            runs: &doc.runs[to as usize],
                        }),
                    )]
                } else {
                    link_pieces(dest, dest_next, &doc.runs[to as usize], w, h, x, y, &words)
                };
                // A link the words do not classify, or a float's: only the
                // invariants are checked, against no truth.
                let target = match classified {
                    Some(Target::Float(..)) | None => {
                        let key = if classified.is_some() {
                            "link float"
                        } else {
                            "link other"
                        };
                        let tally = totals.entry(key.into()).or_default();
                        tally.checked += 1;
                        let pages: Vec<&Page> = std::iter::once(dest).chain(dest_next).collect();
                        let displays: Vec<&[[f32; 4]]> = [to as usize, to as usize + 1]
                            .iter()
                            .map(|&p| doc.displays.get(p).map_or(&[][..], Vec::as_slice))
                            .collect();
                        let problems = invariants(&pages, &displays, &crops, None, tally);
                        if sweep {
                            let _ = writeln!(
                                detail,
                                "SWEEP {key} {inside:?} p{}->p{} crop {} invariants {}",
                                src + 1,
                                to + 1,
                                show_crops(&crops),
                                if problems.is_empty() {
                                    "hold"
                                } else {
                                    "BROKEN"
                                }
                            );
                        }
                        if !problems.is_empty() {
                            let _ = writeln!(
                                detail,
                                "{} {key} {inside:?} p{}->p{} x={x:?} y={y:?} crop {}\n    {}",
                                doc.name,
                                src + 1,
                                to + 1,
                                show_crops(&crops),
                                problems.join("\n    ")
                            );
                        }
                        continue;
                    }
                    Some(target) => target,
                };
                // A destination that does not carry the number the words
                // name gives way, in the engine, to the one that does.
                let mut shown = (to, x, y);
                if let Some((p, cx, cy)) = (!old)
                    .then(|| corrected(&doc.labels, dest, to, crops[0].1, &words))
                    .flatten()
                    .filter(|c| doc.pages.get(c.0 as usize).is_some_and(Option::is_some))
                {
                    let [pw, ph] = doc.sizes[p as usize];
                    let (page_p, next_p) = (
                        doc.pages[p as usize].as_ref().expect("read"),
                        doc.pages.get(p as usize + 1).and_then(Option::as_ref),
                    );
                    let none = super::targets::LinkText::default();
                    crops = link_pieces(
                        page_p,
                        next_p,
                        &doc.runs[p as usize],
                        pw,
                        ph,
                        Some(cx),
                        Some(cy),
                        &none,
                    );
                    shown = (p, Some(cx), Some(cy));
                }
                let (to, x, y) = shown;
                let page = doc.pages[to as usize].as_ref().expect("read");
                let next = doc.pages.get(to as usize + 1).and_then(Option::as_ref);
                let pages: Vec<&Page> = std::iter::once(page).chain(next).collect();
                if let Some(number) = target_number(&target) {
                    let numbered = totals.entry("numbered links".into()).or_default();
                    numbered.checked += 1;
                    if !carries_number(page, crops[0].1, &number) {
                        numbered.missing += 1;
                        let _ = writeln!(
                            detail,
                            "{} WRONG-NUMBER {target:?} {inside:?} p{}->p{} crop {} words {:?} names {:?}",
                            doc.name,
                            src + 1,
                            to + 1,
                            show_crops(&crops),
                            (&words.before, &words.inside),
                            super::targets::named_number(&words)
                        );
                    }
                }
                let key = format!("link {}", target.kind());
                let tally = totals.entry(key.clone()).or_default();
                let anchor = y.map(|y| (x, y));
                let found = truth(&pages, &target, anchor);
                let displays: Vec<&[[f32; 4]]> = [to as usize, to as usize + 1]
                    .iter()
                    .map(|&p| doc.displays.get(p).map_or(&[][..], Vec::as_slice))
                    .collect();
                let mut problems = Vec::new();
                match &found {
                    Some(truth) => problems.extend(judge(&pages, &crops, truth, tally)),
                    None => {
                        tally.no_truth += 1;
                        let _ = writeln!(
                            detail,
                            "{} NO-TRUTH {key} {target:?} {inside:?} p{}->p{} x={x:?} y={y:?}",
                            doc.name,
                            src + 1,
                            to + 1
                        );
                    }
                }
                problems.extend(invariants(&pages, &displays, &crops, found.as_ref(), tally));
                if sweep {
                    let _ = writeln!(
                        detail,
                        "SWEEP {key} {target:?} {inside:?} p{}->p{} crop {} invariants {}",
                        src + 1,
                        to + 1,
                        show_crops(&crops),
                        if problems.iter().any(|p| p.starts_with("INV")) {
                            "BROKEN"
                        } else {
                            "hold"
                        }
                    );
                }
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{} {key} {target:?} p{}->p{} x={x:?} y={y:?} crop {}\n    {}",
                        doc.name,
                        src + 1,
                        to + 1,
                        show_crops(&crops),
                        problems.join("\n    ")
                    );
                }
            }
        }
        // text-detected hover spans
        let mut seen = std::collections::HashSet::new();
        for spans in doc.spans.values() {
            for span in spans {
                let Some(region) = span.target.region else {
                    continue;
                };
                let Some(target) = classify_span(span.kind, &span.label) else {
                    continue;
                };
                if matches!(target, Target::Float(..)) {
                    continue;
                }
                let to = span.target.page as usize;
                if !seen.insert((to, region.map(f32::to_bits))) {
                    continue;
                }
                let Some(Some(page)) = doc.pages.get(to) else {
                    continue;
                };
                let next = doc.pages.get(to + 1).and_then(Option::as_ref);
                let pages: Vec<&Page> = std::iter::once(page).chain(next).collect();
                let key = format!("span {}", target.kind());
                let tally = totals.entry(key.clone()).or_default();
                let anchor = Some((span.target.x, span.target.y));
                let found = truth(&pages, &target, anchor);
                let crops = span_pieces(span.kind, page, next, region);
                let displays: Vec<&[[f32; 4]]> = [to, to + 1]
                    .iter()
                    .map(|&p| doc.displays.get(p).map_or(&[][..], Vec::as_slice))
                    .collect();
                let mut problems = Vec::new();
                match &found {
                    Some(truth) => problems.extend(judge(&pages, &crops, truth, tally)),
                    None => tally.no_truth += 1,
                }
                problems.extend(invariants(&pages, &displays, &crops, found.as_ref(), tally));
                if sweep {
                    let _ = writeln!(
                        detail,
                        "SWEEP {key} {target:?} {:?} ->p{} crop {} invariants {}",
                        span.label,
                        to + 1,
                        show_crops(&crops),
                        if problems.iter().any(|p| p.starts_with("INV")) {
                            "BROKEN"
                        } else {
                            "hold"
                        }
                    );
                }
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{} {key} {target:?} ->p{} region {}\n    {}",
                        doc.name,
                        to + 1,
                        show_crops(&crops),
                        problems.join("\n    ")
                    );
                }
            }
        }
    }
    let mut report = String::new();
    let mut all = Tally::default();
    for (kind, tally) in &totals {
        let _ = writeln!(report, "{}", tally.line(kind));
        all.add(tally);
    }
    let _ = writeln!(report, "{}", all.line("TOTAL"));
    let _ = writeln!(
        report,
        "displays found in {} ms",
        DISPLAY_TIME.load(std::sync::atomic::Ordering::Relaxed) / 1000
    );
    let _ = writeln!(report, "\nPROBLEMS\n{detail}");
    std::fs::write(&out_path, &report).expect("write report");
    eprintln!("wrote {out_path}");
}

fn near_y(d: &[f32; 4], y: f32) -> bool {
    d[1] - 60.0 < y && d[1] + d[3] + 60.0 > y
}

/// One destination's lines and target, for debugging:
/// `SPEEDYF_TARGET=path.pdf:page:x:y` (1-based page; x may be `-`).
#[test]
#[ignore]
fn dump_link_target() {
    let Ok(spec) = std::env::var("SPEEDYF_TARGET") else {
        return;
    };
    let parts: Vec<&str> = spec.rsplitn(4, ':').collect();
    let (y, x, page, path) = (parts[0], parts[1], parts[2], parts[3]);
    let y: f32 = y.parse().expect("y");
    let x: Option<f32> = x.parse().ok();
    let src: i32 = page.parse::<i32>().expect("page") - 1;
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let raw = render::open_document(b, path, None).expect("open");
    let [w, h] = render::page_display_size(b, raw, src).expect("size");
    let read = |src: i32| {
        let [w, h] = render::page_display_size(b, raw, src)?;
        let handle = b.FPDF_LoadPage(raw, src);
        if handle.is_null() {
            return None;
        }
        let mut page = Page::new(extract::extract(b, handle, w, h));
        let runs = text::extract_page(b, handle, w, h).runs;
        page.set_displays(extract::displays(b, handle, w, h, &runs, true));
        let page = super::prepare_page(page, Default::default());
        b.FPDF_ClosePage(handle);
        Some(page)
    };
    let page = read(src).expect("page");
    let next = read(src + 1);
    let all = std::env::var("SPEEDYF_ALL").is_ok();
    let show = |page: &Page, near: Option<f32>| {
        for (i, l) in page.lines.iter().enumerate() {
            if near.is_none_or(|y| (l.top - y).abs() < 60.0) {
                eprintln!(
                    "line {i} x {:.1}..{:.1} y {:.1}..{:.1} size {:.1} font {} disp {} furn {} {:?}",
                    l.x0,
                    l.x1,
                    l.bottom,
                    l.top,
                    l.size,
                    l.font,
                    l.is_display(),
                    l.flags.furniture,
                    text_of(page, i).chars().take(70).collect::<String>()
                );
            }
        }
    };
    show(&page, (!all).then_some(y));
    for d in &page.displays {
        if near_y(d, y) || all {
            eprintln!("display {d:?}");
        }
    }
    let target = super::targets::link_preview(Some(&page), &[], w, h, x, Some(y), None);
    eprintln!("target {target:?}");
    if let Some(flow) = target.flow {
        let pieces = super::continuation::continuation(flow, &page, next.as_ref(), target.rect);
        eprintln!("pieces {pieces:?}");
    }
    // The eval's truth for a target: `SPEEDYF_TRUTH=theorem:1`.
    if let Ok(spec) = std::env::var("SPEEDYF_TRUTH") {
        let (word, number) = spec.split_once(':').unwrap_or(("", ""));
        let target = match word {
            "equation" => Target::Equation(number.into()),
            "section" => Target::Section(number.into()),
            "footnote" => Target::Footnote(number.into()),
            "citation" => Target::Citation((!number.is_empty()).then(|| number.into())),
            _ => Target::Theorem(word.into(), number.into()),
        };
        let pages: Vec<&Page> = std::iter::once(&page).chain(next.as_ref()).collect();
        if let Some(t) = truth(&pages, &target, Some((x, y))) {
            for (p, i) in t.lines {
                eprintln!("truth line p+{p} {:?}", text_of(pages[p], i));
            }
            for (p, i) in t.outside {
                eprintln!("truth outside p+{p} {:?}", text_of(pages[p], i));
            }
        }
    }
    let mut classified = page.clone();
    let regions = super::regions::classify(&mut classified, &mut Default::default());
    for d in super::xref::find_definitions(&classified, &regions, 0) {
        if (d.jump.1 - y).abs() < 60.0 {
            eprintln!(
                "definition {:?} {} jump {:?} region {:?} row {:?}",
                d.kind, d.label, d.jump, d.region, d.row
            );
        }
    }
    if let (true, Some(next)) = (all, next.as_ref()) {
        eprintln!("NEXT PAGE");
        show(next, None);
    }
    b.FPDF_CloseDocument(raw);
}

/// A document's internal links with their text and destination:
/// `SPEEDYF_LINKS=path.pdf`.
#[test]
#[ignore]
fn dump_links() {
    let Ok(path) = std::env::var("SPEEDYF_LINKS") else {
        return;
    };
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let doc = load(b, &path).expect("load");
    let mut labels: Vec<_> = doc.labels.iter().collect();
    labels.sort_by(|a, b| a.0 .1.cmp(&b.0 .1));
    for ((kind, label), at) in labels {
        eprintln!("label {kind:?} {label} at {at:?}");
    }
    for (src, links_here) in doc.links.iter().enumerate() {
        let Some(source) = doc.pages[src].as_ref() else {
            continue;
        };
        for &(rect, x, y, to) in links_here {
            let (before, inside) = locate(source, rect)
                .map(|(s, e)| {
                    (
                        source.text[s.saturating_sub(16)..s]
                            .iter()
                            .collect::<String>(),
                        source.text[s..e].iter().collect::<String>(),
                    )
                })
                .unwrap_or_default();
            eprintln!(
                "p{} {:?} | {:?} -> p{} x={x:?} y={y:?} as {:?}",
                src + 1,
                before,
                inside,
                to + 1,
                classify_link(&before, &inside, rect)
            );
        }
    }
}

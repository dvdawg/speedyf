//! Footnote marks and the notes they point at.
//!
//! A mark is a raised, smaller glyph run - a digit or two, `*`, a dagger, a
//! section sign, a lowercase letter - attached to the word before it. Its
//! note is a small-type line at the foot of the same page starting with the
//! same mark. Everything else that looks like a mark is turned away:
//!
//! - Superscript citations (Nature, Science, ACS) are numbers with no note
//!   at the foot of the page, used again and again, often as lists "1,2" or
//!   ranges "3-5". A page with several unmatched numeric marks uses them for
//!   citations, and none of its numbers is trusted.
//! - Exponents (x^2, 10^-3, cm^2) sit on a variable, a number or a unit, or
//!   are set in a math font.
//! - Affiliation marks on a title page point at affiliation lines; they are
//!   only accepted above the abstract, and an affiliation line is never the
//!   note for a mark in running text.
//! - A table note ("a Measured at 300 K") sits under its table: a letter
//!   mark goes to the nearest small-type line below it, in its column,
//!   starting with that letter.
//! - The note is always on the mark's own page; a note continuing onto the
//!   next page is flagged, not followed.
//!
//! PDFium-free.

use super::page::{Line, Page, Script};
use super::Rect;

/// A footnote mark and its note.
#[derive(Clone, Debug, PartialEq)]
pub struct Footnote {
    pub mark: String,
    /// The mark in the page's normalised text.
    pub range: (usize, usize),
    pub rects: Vec<Rect>,
    /// The note's lines, y-up page space.
    pub region: Rect,
    /// The note's first line: where a click lands.
    pub jump: (f32, f32),
    /// The note runs on onto the next page.
    pub continues: bool,
}

const SYMBOLS: &[&str] = &[
    "*",
    "**",
    "***",
    "\u{2217}",
    "\u{2020}",
    "\u{2020}\u{2020}",
    "\u{2021}",
    "\u{2021}\u{2021}",
    "\u{00A7}",
    "\u{00B6}",
    "\u{2016}",
    "#",
];

/// Units an exponent is commonly written on.
const UNITS: &[&str] = &[
    "m", "cm", "mm", "km", "nm", "um", "s", "ms", "kg", "g", "mg", "Hz", "kHz", "MHz", "GHz", "eV",
    "keV", "MeV", "GeV", "K", "J", "W", "V", "A", "L", "mL", "mol", "sec", "ft", "in",
];

/// Words in an affiliation line.
const AFFILIATION_WORDS: &[&str] = &[
    "university",
    "universit",
    "institute",
    "institut",
    "department",
    "dept.",
    "laboratory",
    "laboratoire",
    "school of",
    "college",
    "faculty",
    "hospital",
    "gmbh",
    "academy",
    "@",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkKind {
    Number,
    Symbol,
    Letter,
}

fn mark_kind(mark: &str) -> Option<MarkKind> {
    if !mark.is_empty() && mark.len() <= 3 && mark.chars().all(|c| c.is_ascii_digit()) {
        let value: u32 = mark.parse().ok()?;
        return (1..=300).contains(&value).then_some(MarkKind::Number);
    }
    if SYMBOLS.contains(&mark) {
        return Some(MarkKind::Symbol);
    }
    let mut chars = mark.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() {
            return Some(MarkKind::Letter);
        }
    }
    None
}

fn canonical(mark: &str) -> String {
    mark.replace('\u{2217}', "*")
}

/// A note line opening with a mark: the mark, and where its text starts.
fn note_start(page: &Page, line: usize) -> Option<String> {
    let l = &page.lines[line];
    let text = &page.text;
    let raised = |i: usize| page.script.get(i) == Some(&Script::Super);
    // A raised mark: every character up to the first lowered one.
    if raised(l.start) {
        let end = (l.start..l.end).find(|&i| !raised(i)).unwrap_or(l.end);
        let mark: String = text[l.start..end].iter().collect();
        let mark = mark.trim().to_string();
        mark_kind(&mark)?;
        return (end < l.end).then_some(canonical(&mark));
    }
    // A mark in the note's own size, then a space: "1 This...", "* Email".
    let space = (l.start..l.end).find(|&i| text[i] == ' ')?;
    let mark: String = text[l.start..space].iter().collect();
    let kind = mark_kind(&mark)?;
    let next = text.get(space + 1).copied();
    let ok = match kind {
        MarkKind::Symbol => true,
        // A digit or letter in body size at a line start is as likely to be
        // the start of a sentence: "2 times larger". Only a capital after
        // it reads as a note.
        MarkKind::Number | MarkKind::Letter => next.is_some_and(|c| c.is_uppercase()),
    };
    ok.then(|| canonical(&mark))
}

fn is_body_line(page: &Page, l: &Line) -> bool {
    page.body_size > 0.0
        && (l.size - page.body_size).abs() <= 0.06 * page.body_size
        && l.end - l.start >= 10
        && !l.is_display()
}

fn x_overlap(a: &Line, x0: f32, x1: f32) -> bool {
    a.x1.min(x1) - a.x0.max(x0) > 0.0
}

/// Lines in the page's footnote zone: small type below every body line in
/// their column, in the lower half of the page.
fn zone(page: &Page) -> Vec<usize> {
    if page.body_size <= 0.0 {
        return Vec::new();
    }
    (0..page.lines.len())
        .filter(|&i| {
            let l = &page.lines[i];
            l.size > 0.0
                && l.size <= 0.92 * page.body_size
                && !l.flags.furniture
                && !l.flags.bibliography
                && l.top < 0.5 * page.height
                && page
                    .lines
                    .iter()
                    .filter(|b| is_body_line(page, b) && x_overlap(b, l.x0, l.x1))
                    .all(|b| b.bottom > l.top)
        })
        .collect()
}

/// A note: its mark, and its lines in stream order.
struct Note {
    mark: String,
    lines: Vec<usize>,
}

fn notes_in(page: &Page, lines: &[usize]) -> Vec<Note> {
    let mut out: Vec<Note> = Vec::new();
    for &line in lines {
        if let Some(mark) = note_start(page, line) {
            out.push(Note {
                mark,
                lines: vec![line],
            });
        } else if let Some(last) = out.last_mut() {
            let previous = &page.lines[*last.lines.last().unwrap_or(&line)];
            let current = &page.lines[line];
            let close = previous.bottom - current.top <= 1.2 * current.em();
            if close && previous.bottom > current.bottom {
                last.lines.push(line);
            }
        }
    }
    out
}

fn note_region(page: &Page, lines: &[usize]) -> Rect {
    let mut x0 = f32::MAX;
    let mut y0 = f32::MAX;
    let mut x1 = f32::MIN;
    let mut y1 = f32::MIN;
    for &i in lines {
        let l = &page.lines[i];
        x0 = x0.min(l.x0);
        y0 = y0.min(l.bottom);
        x1 = x1.max(l.x1);
        y1 = y1.max(l.top);
    }
    let pad = 3.0;
    let x0 = (x0 - pad).max(0.0);
    let y0 = (y0 - pad).max(0.0);
    let x1 = (x1 + pad).min(page.width.max(1.0));
    let y1 = (y1 + pad).min(page.height.max(1.0));
    [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
}

/// The top of the abstract, if this page has one: marks above it are in the
/// title block.
fn abstract_top(page: &Page) -> Option<f32> {
    page.lines.iter().enumerate().find_map(|(i, l)| {
        let text = page.line_text(i);
        let word = [
            "Abstract",
            "ABSTRACT",
            "A B S T R A C T",
            "Summary",
            "SUMMARY",
        ]
        .into_iter()
        .find(|w| text.starts_with(w))?;
        let rest = text[word.len()..].chars().next();
        matches!(rest, None | Some('.' | ':' | ' ' | '-')).then_some(l.top)
    })
}

/// One raised run in running text.
struct Raised {
    /// The run split at commas: "1,2" is two elements.
    elements: Vec<(String, (usize, usize))>,
    list: bool,
    title: bool,
}

fn previous_token(text: &[char], before: usize) -> String {
    let mut start = before;
    while start > 0 && text[start - 1].is_alphanumeric() {
        start -= 1;
    }
    text[start..before].iter().collect()
}

fn raised_runs(page: &Page, zone_lines: &[usize], title_bottom: Option<f32>) -> Vec<Raised> {
    let text = &page.text;
    let mut out = Vec::new();
    let mut index = 0;
    while index < text.len() {
        if page.script[index] != Script::Super {
            index += 1;
            continue;
        }
        let start = index;
        while index < text.len() && page.script[index] == Script::Super {
            index += 1;
        }
        let end = index;
        let Some(line_index) = page.line_of.get(start).map(|&l| l as usize) else {
            continue;
        };
        let line = &page.lines[line_index];
        if zone_lines.contains(&line_index) || line.excluded() || line.is_display() {
            continue;
        }
        // A run at the very start of a line is a note's own mark.
        if start == line.start {
            continue;
        }
        if page.any_math(start, end) {
            continue;
        }
        // Attached to what it marks: a word, or the punctuation after one.
        let Some(&before) = text.get(start.wrapping_sub(1)) else {
            continue;
        };
        let run: String = text[start..end].iter().collect();
        let letters_only = run
            .split([',', '-'])
            .all(|p| mark_kind(p.trim()) == Some(MarkKind::Letter));
        // A number before a mark makes it an exponent - except a table
        // note's letter, which marks a value: "300^a".
        if before == ' ' || (before.is_ascii_digit() && !letters_only) {
            continue;
        }
        if page.glyph_at(start - 1).is_some_and(|g| g.math) {
            continue;
        }
        if before.is_alphabetic() {
            let token = previous_token(text, start);
            if token.chars().count() == 1 || UNITS.contains(&token.as_str()) {
                continue;
            }
        } else if !before.is_ascii_digit()
            && !matches!(before, '.' | ',' | ';' | ':' | ')' | '\'' | '"')
        {
            continue;
        }
        let list = run.contains(',') || run.contains('-');
        let mut elements = Vec::new();
        let mut cursor = start;
        let mut ok = true;
        for piece in run.split([',', '-']) {
            let len = piece.chars().count();
            let trimmed = piece.trim();
            if mark_kind(trimmed).is_none() {
                ok = false;
                break;
            }
            let lead = piece.chars().take_while(|c| *c == ' ').count();
            let s = cursor + lead;
            elements.push((canonical(trimmed), (s, s + trimmed.chars().count())));
            cursor += len + 1;
        }
        if !ok || elements.is_empty() {
            continue;
        }
        let title = title_bottom.is_some_and(|bottom| line.bottom > bottom);
        out.push(Raised {
            elements,
            list,
            title,
        });
    }
    out
}

fn is_affiliation(page: &Page, lines: &[usize]) -> bool {
    let text: String = lines
        .iter()
        .map(|&i| page.line_text(i))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    AFFILIATION_WORDS.iter().any(|w| text.contains(w))
}

fn padded_rects(page: &Page, start: usize, end: usize) -> Vec<Rect> {
    page.rects(start, end)
        .into_iter()
        .map(|[x, y, w, h]| [x - 1.5, y - 1.5, w + 3.0, h + 3.0])
        .collect()
}

/// Every footnote mark on a page with its note. `next` is the following
/// page, to tell whether a note runs on.
pub fn find_footnotes(page: &Page, next: Option<&Page>) -> Vec<Footnote> {
    let zone_lines = zone(page);
    let notes = notes_in(page, &zone_lines);
    let title_bottom = abstract_top(page);
    let runs = raised_runs(page, &zone_lines, title_bottom);

    // Superscript citations: numeric marks with no note.
    let unmatched_numbers = runs
        .iter()
        .filter(|r| !r.title)
        .flat_map(|r| r.elements.iter())
        .filter(|(m, _)| mark_kind(m) == Some(MarkKind::Number))
        .filter(|(m, _)| !notes.iter().any(|n| &n.mark == m))
        .count();
    let citation_page = unmatched_numbers >= 3;

    let count_in_text = |mark: &str| {
        runs.iter()
            .filter(|r| !r.title)
            .flat_map(|r| r.elements.iter())
            .filter(|(m, _)| m == mark)
            .count()
    };
    let last_zone_line = zone_lines.last().copied();

    let mut out = Vec::new();
    for run in &runs {
        for (mark, range) in &run.elements {
            let Some(kind) = mark_kind(mark) else {
                continue;
            };
            let mark_line = page.line_at(range.0).cloned();
            let Some(mark_line) = mark_line else {
                continue;
            };
            let target: Option<Vec<usize>> = if kind == MarkKind::Letter && !run.title {
                table_note(page, &mark_line, mark, range.0)
            } else {
                let matching: Vec<&Note> = notes.iter().filter(|n| &n.mark == mark).collect();
                let title_lines: Vec<usize> = if run.title {
                    title_notes(page, mark, title_bottom)
                } else {
                    Vec::new()
                };
                if run.title {
                    // Title block: a note at the foot, or an affiliation
                    // line, whichever exists - and only one of them.
                    let mut found: Vec<Vec<usize>> =
                        matching.iter().map(|n| n.lines.clone()).collect();
                    if !title_lines.is_empty() {
                        found.push(title_lines);
                    }
                    (found.len() == 1).then(|| found.remove(0))
                } else {
                    let single = !run.list && count_in_text(mark) == 1;
                    let trusted = kind != MarkKind::Number || !citation_page;
                    match (single && trusted, matching.as_slice()) {
                        (true, [note]) if !is_affiliation(page, &note.lines) => {
                            let top = page.lines[note.lines[0]].top;
                            (top < mark_line.bottom).then(|| note.lines.clone())
                        }
                        _ => None,
                    }
                }
            };
            let Some(lines) = target else {
                continue;
            };
            if run.title && !matches!(kind, MarkKind::Symbol) {
                // A numbered or lettered title mark must clearly be an
                // affiliation or a note of its own.
                let note_like = notes.iter().any(|n| n.lines == lines);
                if !note_like && !is_affiliation(page, &lines) {
                    continue;
                }
            }
            let first = &page.lines[lines[0]];
            let continues = last_zone_line.is_some_and(|last| lines.contains(&last))
                && notes.last().is_some_and(|n| n.lines == lines)
                && next.is_some_and(continues_on);
            out.push(Footnote {
                mark: mark.clone(),
                range: *range,
                rects: padded_rects(page, range.0, range.1),
                region: note_region(page, &lines),
                jump: (first.x0, first.top),
                continues,
            });
        }
    }
    out
}

/// Whether the next page's footnote zone opens mid-note.
fn continues_on(next: &Page) -> bool {
    let lines = zone(next);
    let Some(&first) = lines.first() else {
        return false;
    };
    note_start(next, first).is_none()
}

/// Affiliation lines in the title block opening with `mark`.
fn title_notes(page: &Page, mark: &str, title_bottom: Option<f32>) -> Vec<usize> {
    let Some(bottom) = title_bottom else {
        return Vec::new();
    };
    let starts: Vec<usize> = (0..page.lines.len())
        .filter(|&i| page.lines[i].bottom > bottom || page.lines[i].top < 0.5 * page.height)
        .filter(|&i| {
            let l = &page.lines[i];
            page.script.get(l.start) == Some(&Script::Super)
                && note_start(page, i).as_deref() == Some(mark)
        })
        .collect();
    match starts.as_slice() {
        [one] => vec![*one],
        _ => Vec::new(),
    }
}

/// The table note for a letter mark: the nearest small-type line below it,
/// in its column, starting with the same letter.
fn table_note(page: &Page, mark_line: &Line, mark: &str, at: usize) -> Option<Vec<usize>> {
    if page.body_size <= 0.0 {
        return None;
    }
    let mark_glyph = page.glyph_at(at)?;
    let mx0 = mark_glyph.rect[0];
    let mut best: Option<(f32, usize)> = None;
    for (i, l) in page.lines.iter().enumerate() {
        if l.top >= mark_line.bottom || l.size <= 0.0 || l.size > 0.92 * page.body_size {
            continue;
        }
        if mark_line.bottom - l.top > 0.5 * page.height {
            continue;
        }
        // In the mark's column: the note spans the mark's x, or starts left
        // of it within the same block.
        if !(l.x0 <= mx0 + 2.0 && l.x1 >= l.x0) || l.x1 < mark_line.x0 {
            continue;
        }
        if note_start(page, i).as_deref() != Some(mark) {
            continue;
        }
        let distance = mark_line.bottom - l.top;
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, i));
        }
    }
    best.map(|(_, i)| vec![i])
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::*;

    /// A page of body text with `marks` attached to words on some lines,
    /// and notes in small type at the foot.
    fn page_with(body: &[(&str, Option<&str>)], notes: &[&str]) -> Page {
        let mut b = Builder::new();
        let mut y = 700.0;
        for (text, mark) in body {
            let end = b.text(72.0, y, BODY, text);
            if let Some(mark) = mark {
                b.newline();
                let after = b.sup(end, y, mark);
                b.newline();
                b.text(after + 3.0, y, BODY, " and more words here");
            }
            b.newline();
            y -= 12.0;
        }
        let mut y = 90.0;
        for note in notes {
            if let Some((mark, rest)) = note.split_once('|') {
                let after = b.sup(72.0, y, mark);
                b.text(after, y, SMALL, rest);
            } else {
                b.text(72.0, y, SMALL, note);
            }
            b.newline();
            y -= 10.0;
        }
        b.page()
    }

    #[test]
    fn a_numbered_mark_finds_its_note_at_the_foot() {
        let page = page_with(
            &[
                ("This is a sentence with a footnote", Some("1")),
                ("and some more running text in the body", None),
            ],
            &["1|This is the note text in small type."],
        );
        let found = find_footnotes(&page, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mark, "1");
        assert!(found[0].region[1] < 100.0);
        assert!(!found[0].continues);
    }

    #[test]
    fn symbol_marks_work() {
        let page = page_with(
            &[("Corresponding author is marked", Some("\u{2020}"))],
            &["\u{2020}|Present address: somewhere."],
        );
        assert_eq!(find_footnotes(&page, None).len(), 1);
    }

    #[test]
    fn pathology_superscript_citations_have_no_note_and_are_not_footnotes() {
        let page = page_with(
            &[
                ("Previous work established this", Some("1")),
                ("as was shown by many authors", Some("2,3")),
                ("and confirmed again recently", Some("4")),
                ("and once more in another study", Some("5")),
            ],
            &["1|Department of Physics, University of Somewhere."],
        );
        assert!(find_footnotes(&page, None).is_empty());
    }

    #[test]
    fn pathology_a_mark_with_no_note_is_not_a_footnote() {
        let page = page_with(&[("A sentence with a raised mark", Some("7"))], &[]);
        assert!(find_footnotes(&page, None).is_empty());
    }

    #[test]
    fn pathology_math_exponents_are_not_marks() {
        let mut b = Builder::new();
        // x^2: the base is a single letter
        let end = b.text(72.0, 700.0, BODY, "the value of x");
        b.sup(end, 700.0, "2");
        b.newline();
        // 10^3: the base is a number
        let end = b.text(72.0, 688.0, BODY, "roughly 10");
        b.sup(end, 688.0, "3");
        b.newline();
        // cm^2: a unit
        let end = b.text(72.0, 676.0, BODY, "an area of 4 cm");
        b.sup(end, 676.0, "2");
        b.newline();
        // a math-font exponent
        let end = b.text(72.0, 664.0, MATH, "energy E");
        b.text(
            end,
            667.5,
            Style {
                size: 7.0,
                font: 4,
                bold: false,
                math: true,
            },
            "2",
        );
        b.newline();
        let mut y = 90.0;
        for mark in ["2", "3"] {
            let after = b.sup(72.0, y, mark);
            b.text(after, y, SMALL, "A note in small type.");
            b.newline();
            y -= 10.0;
        }
        assert!(find_footnotes(&b.page(), None).is_empty());
    }

    #[test]
    fn pathology_the_same_mark_twice_in_text_is_refused() {
        let page = page_with(
            &[
                ("A first use of the mark", Some("1")),
                ("and a second use of the same", Some("1")),
            ],
            &["1|A note."],
        );
        assert!(find_footnotes(&page, None).is_empty());
    }

    #[test]
    fn pathology_title_page_affiliation_marks() {
        let mut b = Builder::new();
        b.line(740.0, BOLD, "A Title of the Paper");
        let end = b.text(150.0, 720.0, BODY, "Alice Author");
        b.sup(end, 720.0, "1");
        b.newline();
        let end = b.text(150.0, 708.0, BODY, "Bob Writer");
        b.sup(end, 708.0, "2");
        b.newline();
        let after = b.sup(150.0, 694.0, "1");
        b.text(
            after,
            694.0,
            SMALL,
            "Department of Physics, University of A.",
        );
        b.newline();
        let after = b.sup(150.0, 684.0, "2");
        b.text(after, 684.0, SMALL, "A line that names no place at all.");
        b.newline();
        b.line(660.0, BOLD, "Abstract");
        let end = b.text(72.0, 640.0, BODY, "The abstract cites prior work");
        b.sup(end, 640.0, "1");
        b.newline();
        let found = find_footnotes(&b.page(), None);
        let marks: Vec<(&str, f32)> = found.iter().map(|f| (f.mark.as_str(), f.jump.1)).collect();
        // Alice's affiliation clearly matches; Bob's line is not clearly one;
        // the abstract's superscript is a citation.
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, "1");
        assert!(marks[0].1 > 690.0);
    }

    #[test]
    fn pathology_table_notes_go_to_the_nearest_note_below_in_the_column() {
        let mut b = Builder::new();
        for i in 0..5 {
            b.line(
                700.0 - 12.0 * i as f32,
                BODY,
                "Body text of the paper runs here.",
            );
        }
        let end = b.text(100.0, 600.0, SMALL, "Sample A 300");
        b.sup(end, 600.0, "a");
        b.newline();
        let end = b.text(100.0, 590.0, SMALL, "Sample B 310");
        b.sup(end, 590.0, "a");
        b.newline();
        let after = b.sup(100.0, 578.0, "a");
        b.text(after, 578.0, SMALL, "Measured at room temperature.");
        b.newline();
        b.line(560.0, BODY, "More body text after the table here.");
        let after = b.sup(72.0, 90.0, "a");
        b.text(after, 90.0, SMALL, "A footnote a far away.");
        b.newline();
        let found = find_footnotes(&b.page(), None);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|f| (f.jump.1 - 578.0).abs() < 10.0));
    }

    #[test]
    fn pathology_notes_are_on_the_marks_own_page_and_continuations_are_flagged() {
        let page = page_with(
            &[("A sentence with a long footnote", Some("3"))],
            &["3|The note starts here and runs on to the"],
        );
        let mut next = Builder::new();
        next.line(700.0, BODY, "Next page body text runs here now.");
        next.line(90.0, SMALL, "next page, where it ends.");
        let next = next.page();
        let found = find_footnotes(&page, Some(&next));
        assert_eq!(found.len(), 1);
        assert!(found[0].continues);
        // The next page's own copy of mark 3 (none here) is never consulted.
        let found = find_footnotes(&next, None);
        assert!(found.is_empty());
    }

    #[test]
    fn pathology_full_width_footnotes_under_two_columns() {
        let mut b = Builder::new();
        for i in 0..20 {
            let y = 700.0 - 12.0 * i as f32;
            b.text(50.0, y, BODY, "Left column body text.");
            b.newline();
            if i == 10 {
                let end = b.text(320.0, y, BODY, "Right marked text");
                b.sup(end, y, "\u{2021}");
            } else {
                b.text(320.0, y, BODY, "Right column body text.");
            }
            b.newline();
        }
        let after = b.sup(50.0, 200.0, "\u{2021}");
        b.text(
            after,
            200.0,
            SMALL,
            "A note set across the whole width of the page, under both columns of it.",
        );
        b.newline();
        let found = find_footnotes(&b.page(), None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mark, "\u{2021}");
    }

    #[test]
    fn pathology_per_column_footnotes_in_two_columns() {
        let mut b = Builder::new();
        for i in 0..20 {
            let y = 700.0 - 12.0 * i as f32;
            b.text(50.0, y, BODY, "Left column body text.");
            b.newline();
            if i < 30 {
                b.text(320.0, y, BODY, "Right column body text.");
                b.newline();
            }
        }
        let end = b.text(50.0, 400.0, BODY, "A left marked sentence");
        b.sup(end, 400.0, "4");
        b.newline();
        // the left column's note, under the left column only
        let after = b.sup(50.0, 380.0, "4");
        b.text(after, 380.0, SMALL, "Left note under its column.");
        b.newline();
        // the right column keeps going below it
        for i in 0..5 {
            b.text(
                320.0,
                400.0 - 12.0 * i as f32,
                BODY,
                "Right column body text.",
            );
            b.newline();
        }
        let found = find_footnotes(&b.page(), None);
        assert_eq!(found.len(), 1);
        assert!((found[0].jump.1 - 380.0).abs() < 10.0);
    }
}

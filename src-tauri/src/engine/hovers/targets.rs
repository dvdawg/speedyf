//! What a link destination points at, read off the destination page: a
//! theorem's whole statement, a display with its number, a section's
//! heading and first lines, a bibliography entry, a footnote. A link
//! carries only a point on a page; the preview of what lies under that
//! point should be the thing itself, not a fixed-height slice that starts
//! in the previous section's tail and runs on into the next reference.
//!
//! The regions are the hover index's own (`xref::find_definitions`,
//! `footnotes`), so a link and a text-detected mention of the same target
//! preview the same crop. Bibliography entries have no definition there
//! and are read here.
//!
//! PDFium-free.

use super::page::{same_column, Line, Page};
use super::regions::{classify, RegionState};
use super::statement::{column_left, leading_gap, Reader};
use super::xref::{find_definitions, Kind};
use super::Rect;

/// How a target runs on past a column or page break, when it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// A theorem-like statement.
    Statement,
    Footnote,
    /// A bibliography entry.
    Reference,
}

/// What a link destination previews: the region, how it may run on, and
/// the row to mark when it is a display block holding several numbered
/// rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub rect: Rect,
    pub flow: Option<Flow>,
    pub mark: Option<Rect>,
}

impl Target {
    fn plain(rect: Rect) -> Target {
        Target {
            rect,
            flow: None,
            mark: None,
        }
    }
}

/// How far above a definition's top its anchor may sit: hyperref raises a
/// heading's anchor by the skip above it.
const ANCHOR_ABOVE_PT: f32 = 28.0;
/// ...and how far below: an anchor at a heading's baseline.
const ANCHOR_BELOW_PT: f32 = 8.0;

pub(crate) fn pad(rect: Rect, by: f32, page: &Page) -> Rect {
    let x0 = (rect[0] - by).max(0.0);
    let y0 = (rect[1] - by).max(0.0);
    let x1 = (rect[0] + rect[2] + by).min(page.width.max(1.0));
    let y1 = (rect[1] + rect[3] + by).min(page.height.max(1.0));
    [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
}

pub(crate) fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

pub(crate) fn line_rect(l: &Line) -> Rect {
    [l.x0, l.bottom, (l.x1 - l.x0).max(1.0), l.height()]
}

/// `rect`, its padding pulled back off any line just outside it: set
/// tight, the next line's top sits inside the usual 3pt and would show
/// as a sliver.
pub(crate) fn clear_of_neighbours(rect: Rect, page: &Page) -> Rect {
    let (x0, x1) = (rect[0], rect[0] + rect[2]);
    let (mut y0, mut y1) = (rect[1], rect[1] + rect[3]);
    for l in &page.lines {
        if l.flags.furniture || l.x1.min(x1) - l.x0.max(x0) <= 3.0 {
            continue;
        }
        let centre = (l.top + l.bottom) / 2.0;
        // A line mostly outside, poking into the padding.
        if centre < y0 + 1.0 && l.top > y0 {
            y0 = y0.max(l.top + 0.3).min(y1 - 1.0);
        } else if centre > y1 - 1.0 && l.bottom < y1 {
            y1 = y1.min(l.bottom - 0.3).max(y0 + 1.0);
        }
    }
    [x0, y0, rect[2], (y1 - y0).max(1.0)]
}

/// The region a link destination at (`x`, `y`) previews, when the page
/// has a target there; None for anything else (running text, a float -
/// which `floats` reads - or a page with no text).
pub fn link_target(page: &Page, x: Option<f32>, y: f32) -> Option<Rect> {
    found_at(page, x, y).map(|target| target.rect)
}

fn found_at(page: &Page, x: Option<f32>, y: f32) -> Option<Target> {
    target_at(page, x, y).map(|target| Target {
        rect: clear_of_neighbours(target.rect, page),
        ..target
    })
}

fn statement_flow(kind: Kind) -> Option<Flow> {
    kind.is_statement().then_some(Flow::Statement)
}

fn target_at(page: &Page, x: Option<f32>, y: f32) -> Option<Target> {
    let mut page = page.clone();
    let regions = classify(&mut page, &mut RegionState::default());
    let near_x = |rect: Rect| x.is_none_or(|x| x >= rect[0] - 40.0 && x <= rect[0] + rect[2] + 2.0);
    // A definition whose top the anchor sits on or just over.
    let definitions = find_definitions(&page, &regions, 0);
    let best = definitions
        .iter()
        .filter(|d| !matches!(d.kind, Kind::Figure | Kind::Table | Kind::Algorithm))
        .filter(|d| {
            let top = d.jump.1;
            y >= top - ANCHOR_BELOW_PT && y <= top + ANCHOR_ABOVE_PT && near_x(d.region)
        })
        .min_by(|a, b| (a.jump.1 - y).abs().total_cmp(&(b.jump.1 - y).abs()));
    if let Some(d) = best {
        return Some(Target {
            rect: d.region,
            flow: statement_flow(d.kind),
            mark: d.row,
        });
    }
    for note in super::footnotes::find_footnotes(&page, None) {
        let top = note.jump.1;
        if y >= top - ANCHOR_BELOW_PT && y <= top + 14.0 && near_x(note.region) {
            return Some(Target {
                rect: note.region,
                flow: Some(Flow::Footnote),
                mark: None,
            });
        }
    }
    statement_at(&page, x, y)
        .map(|rect| Target {
            rect,
            flow: Some(Flow::Statement),
            mark: None,
        })
        .or_else(|| {
            bibliography_entry(&page, x, y).map(|rect| Target {
                rect,
                flow: Some(Flow::Reference),
                mark: None,
            })
        })
}

/// Words that open a numbered statement.
const STATEMENT_WORDS: &[&str] = &[
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
    "hypothesis",
    "condition",
    "property",
    "problem",
    "question",
    "notation",
    "exercise",
];

fn opens_statement(text: &str) -> bool {
    let lower = text.trim_start().to_lowercase();
    let mut tokens = lower.split_whitespace();
    let (Some(word), Some(number)) = (tokens.next(), tokens.next()) else {
        return false;
    };
    STATEMENT_WORDS.contains(&word.trim_end_matches('.'))
        && number
            .trim_end_matches(['.', ':'])
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
        && number.chars().any(|c| c.is_ascii_digit())
}

/// A numbered heading: "3 Method", "4.2 Results" - short, no full stop.
fn opens_section(text: &str) -> bool {
    let mut tokens = text.split_whitespace();
    let (Some(number), Some(word)) = (tokens.next(), tokens.next()) else {
        return false;
    };
    let numbered = number.trim_end_matches('.').split('.').all(|p| {
        !p.is_empty()
            && (p.chars().all(|c| c.is_ascii_digit())
                || (p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase())))
    });
    numbered
        && word.starts_with(char::is_uppercase)
        && text.split_whitespace().count() <= 12
        && !text.trim_end().ends_with('.')
}

/// A statement the hover index did not read as one (a head split by a
/// ligature, a kind it does not know): from its numbered head on, as far
/// as `statement::Reader` reads it - to the proof, the next statement or
/// heading, or the paragraph after it - or the column's end.
fn statement_at(page: &Page, x: Option<f32>, y: f32) -> Option<Rect> {
    let lines = &page.lines;
    let head = (0..lines.len())
        .filter(|&i| {
            let l = &lines[i];
            !l.flags.furniture
                && l.top <= y + ANCHOR_BELOW_PT
                && l.top >= y - ANCHOR_ABOVE_PT
                && x.is_none_or(|x| l.x0 <= x + 40.0 && l.x1 >= x)
                && opens_statement(&page.line_text(i))
        })
        .min_by(|&a, &b| {
            (lines[a].top - y)
                .abs()
                .total_cmp(&(lines[b].top - y).abs())
        })?;
    let first = &lines[head];
    let region = statement_lines(page, head, None, &mut None);
    Some(pad(union(line_rect(first), region), 3.0, page))
}

/// A line opening something of its own, read by its words: a numbered
/// statement or heading.
pub(crate) fn opens_another(page: &Page, i: usize) -> bool {
    let text = page.line_text(i);
    opens_statement(&text) || opens_section(&text)
}

/// The lines of a statement from `first` down its column, `first` included
/// unless it already ended (`italic` the statement's face, when known; it
/// is updated as the lines say). The region they cover, unpadded.
pub(crate) fn statement_lines(
    page: &Page,
    first: usize,
    italic: Option<bool>,
    face: &mut Option<bool>,
) -> Rect {
    let lines = &page.lines;
    let column = &lines[first];
    let mut reader = Reader::new(page, column_left(page, column), italic);
    let another = |i: usize| opens_another(page, i);
    let mut region = line_rect(column);
    // The last whole line taken, and where reading has got to: a piece of
    // a row is taken without being the line the next is read against.
    let mut previous = first;
    let mut cursor = first;
    let mut count = 0;
    while count < 40 {
        let Some(j) = next_line(page, cursor) else {
            break;
        };
        cursor = j;
        let l = &lines[j];
        let p = &lines[previous];
        if !same_row(p, l) {
            let below = l.top < p.top;
            if !below || !same_column(l, column) || !reader.takes(page, p, j, &another) {
                break;
            }
            previous = j;
        }
        region = union(region, line_rect(l));
        count += 1;
    }
    *face = reader.italic;
    region
}

/// A link's own words and the words just before it on its line: what
/// names its target when the destination alone does not ("Theorem 2",
/// "Sec. 3", "Eq. (4)", "[12]", "Littlestone and Warmuth, 1986").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkText {
    pub before: String,
    pub inside: String,
}

impl LinkText {
    /// A link's words, read from its page's runs: the characters inside
    /// its rectangle, and those before it on its line.
    pub fn from_runs(runs: &[crate::engine::types::TextRun], rect: Rect) -> LinkText {
        let (x0, x1) = (rect[0], rect[0] + rect[2]);
        let mut row: Vec<&crate::engine::types::TextRun> = runs
            .iter()
            .filter(|r| {
                let overlap = (r.y + r.h).min(rect[1] + rect[3]) - r.y.max(rect[1]);
                r.w > 0.0 && overlap >= 0.5 * r.h.min(rect[3]).max(0.1)
            })
            .collect();
        row.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut before = String::new();
        let mut inside = String::new();
        for run in row {
            let units: Vec<u16> = run.text.encode_utf16().collect();
            let placed = run.edges.len() == units.len() + 1;
            let mut unit = 0usize;
            for ch in run.text.chars() {
                let width = ch.len_utf16();
                let centre = if placed {
                    run.x + (run.edges[unit] + run.edges[unit + width]) / 2.0
                } else {
                    run.x + run.w * (unit as f32 + 0.5 * width as f32) / units.len().max(1) as f32
                };
                unit += width;
                if centre >= x0 && centre <= x1 {
                    inside.push(ch);
                } else if centre < x0 {
                    before.push(ch);
                }
            }
            if !before.is_empty() && !before.ends_with(' ') && run.x + run.w < x0 {
                before.push(' ');
            }
        }
        let keep = before.chars().count().saturating_sub(60);
        LinkText {
            before: before.chars().skip(keep).collect(),
            inside,
        }
    }
}

/// What a link's words name.
#[derive(Clone, Debug, PartialEq)]
enum Named {
    Statement(Kind, String),
    Float(String),
    NumberedCitation(String),
    AuthorYear(String, Option<String>),
}

fn number_token(token: &str) -> Option<String> {
    let t = token
        .trim_matches(|c: char| matches!(c, '.' | ':' | ',' | ';' | ')' | '(' | '[' | ']' | '~'));
    let ok = !t.is_empty()
        && t.split('.').all(|p| {
            !p.is_empty()
                && (p.chars().all(|c| c.is_ascii_digit())
                    || (p.len() == 1 && p.chars().all(|c| c.is_ascii_uppercase())))
        });
    ok.then(|| t.to_string())
}

fn year_of(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_digit())
        .rfind(|t| t.len() == 4 && (t.starts_with("19") || t.starts_with("20")))
        .map(str::to_string)
}

fn kind_of(word: &str) -> Option<Kind> {
    let word = word.trim_end_matches(['.', '~']).to_lowercase();
    let word = word
        .strip_suffix('s')
        .filter(|w| w.len() > 3)
        .unwrap_or(&word);
    Some(match word {
        "eq" | "equation" => Kind::Equation,
        "section" | "sec" | "appendix" | "chapter" | "\u{a7}" => Kind::Section,
        "theorem" | "thm" => Kind::Theorem,
        "lemma" => Kind::Lemma,
        "proposition" | "prop" => Kind::Proposition,
        "corollary" | "cor" => Kind::Corollary,
        "definition" | "def" => Kind::Definition,
        "remark" => Kind::Remark,
        "example" => Kind::Example,
        "conjecture" => Kind::Conjecture,
        _ => return None,
    })
}

/// Read what a link names.
fn named(text: &LinkText) -> Option<Named> {
    let inside = text.inside.trim();
    let before = text.before.trim_end();
    let words: Vec<&str> = before
        .split_whitespace()
        .chain(inside.split_whitespace())
        .collect();
    let number = inside.split_whitespace().rev().find_map(number_token);
    if let Some(number) = &number {
        // The kind word: in the link or just before it.
        for word in words.iter().rev().skip(1).take(2) {
            let word = word.trim_end_matches(['(', '~']);
            let lower = word.to_lowercase();
            let lower = lower.trim_end_matches(['.', '~']);
            if matches!(lower, "fig" | "figs" | "figure" | "figures") {
                return Some(Named::Float(format!("fig {number}")));
            }
            if matches!(lower, "table" | "tables" | "tab") {
                return Some(Named::Float(format!("tab {number}")));
            }
            if let Some(kind) = kind_of(word) {
                return Some(Named::Statement(kind, number.clone()));
            }
        }
        // "(29)", or "29" linked inside the parentheses of "(29)".
        let parenthesised = inside.starts_with('(') && inside.ends_with(')')
            || before.ends_with('(') && !inside.contains(char::is_alphabetic);
        if parenthesised {
            return Some(Named::Statement(Kind::Equation, number.clone()));
        }
    }
    // Inside brackets - "[12]", "[AL22]", "[3, 7]": the entry's label.
    let open = before.rfind('[');
    let bracketed =
        open.is_some_and(|o| before.rfind(']').is_none_or(|c| c < o)) || inside.starts_with('[');
    if bracketed {
        let key: String = inside
            .trim_matches(|c: char| matches!(c, '[' | ']' | ',' | ';' | ' '))
            .to_string();
        let plain = !key.is_empty()
            && key.len() <= 12
            && key
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if plain {
            return Some(Named::NumberedCitation(key));
        }
    }
    // Author-year: the first author's surname, the first capitalised word
    // after the citation opens - "(Littlestone and Warmuth, 1986; ..." -
    // and the year, which the link or the words before it must have: a
    // name alone could be anything.
    let year = year_of(inside).or_else(|| year_of(before.rsplit(['(', ';']).next().unwrap_or("")));
    year.as_ref()?;
    let tail = format!("{before} {inside}");
    let opened = tail
        .rfind(['(', ';', '['])
        .map_or(tail.as_str(), |i| &tail[i + 1..]);
    let surname = opened
        .split(|c: char| !c.is_alphabetic() && c != '-' && c != '\'')
        .find(|w| w.len() >= 2 && w.starts_with(char::is_uppercase))?;
    Some(Named::AuthorYear(surname.to_string(), year))
}

/// Where on `page` what the link names is, found by its words.
fn locate_named(
    page: &Page,
    runs: &[crate::engine::types::TextRun],
    named: &Named,
) -> Option<Target> {
    let reference = |rect: Rect| Target {
        rect,
        flow: Some(Flow::Reference),
        mark: None,
    };
    match named {
        Named::Statement(kind, number) => {
            let mut page = page.clone();
            let regions = classify(&mut page, &mut RegionState::default());
            find_definitions(&page, &regions, 0)
                .into_iter()
                .find(|d| {
                    &d.label == number
                        && (d.kind == *kind
                            || (kind.is_statement()
                                && d.kind.is_statement()
                                && *kind == Kind::Theorem))
                })
                .map(|d| Target {
                    rect: d.region,
                    flow: statement_flow(d.kind),
                    mark: d.row,
                })
        }
        Named::Float(key) => {
            let (word, number) = key.split_once(' ')?;
            let block = crate::engine::floats::caption_blocks(runs)
                .into_iter()
                .find(|b| {
                    let lower = b.label.to_lowercase();
                    lower.starts_with(word) && lower.rsplit(' ').next() == Some(number)
                })?;
            crate::engine::floats::float_for_caption(
                runs,
                page.width,
                page.height,
                block.x,
                block.top - 1.0,
            )
            .map(Target::plain)
        }
        Named::NumberedCitation(number) => {
            let label = format!("[{number}]");
            let line = (0..page.lines.len())
                .find(|&i| page.line_text(i).trim_start().starts_with(&label))?;
            let l = &page.lines[line];
            bibliography_entry(page, Some(l.x0), l.top).map(reference)
        }
        Named::AuthorYear(surname, year) => {
            // An entry whose first line names the author, with the year in
            // its first lines.
            let starts: Vec<usize> = (0..page.lines.len())
                .filter(|&i| {
                    let text = page.line_text(i);
                    let head: String = text.chars().take(80).collect();
                    head.contains(surname.as_str())
                })
                .collect();
            let entry = |i: usize| {
                let l = &page.lines[i];
                bibliography_entry(page, Some(l.x0), l.top).map(|rect| (i, rect))
            };
            let with_year = |(_, rect): &(usize, Rect)| {
                let Some(year) = year else { return true };
                (0..page.lines.len()).any(|k| {
                    let l = &page.lines[k];
                    l.bottom >= rect[1] - 0.5
                        && l.top <= rect[1] + rect[3] + 0.5
                        && l.x0 >= rect[0] - 0.5
                        && page.line_text(k).contains(year.as_str())
                })
            };
            starts
                .into_iter()
                .filter_map(entry)
                .find(with_year)
                .map(|(_, rect)| reference(rect))
        }
    }
}

/// Whether the lines in `rect` are what `named` names: an entry with that
/// label, or by that author in that year.
fn answers_to(page: &Page, rect: Rect, named: &Named) -> bool {
    let text: String = (0..page.lines.len())
        .filter(|&i| {
            let l = &page.lines[i];
            l.bottom >= rect[1] - 0.5
                && l.top <= rect[1] + rect[3] + 0.5
                && l.x1.min(rect[0] + rect[2]) - l.x0.max(rect[0]) > 0.0
        })
        .map(|i| page.line_text(i))
        .collect::<Vec<_>>()
        .join(" ");
    match named {
        // The year can be past a column break, in the entry's next piece:
        // an entry opening with the author's name answers to it too.
        Named::AuthorYear(surname, year) => {
            text.contains(surname.as_str())
                && (year.as_ref().is_none_or(|y| text.contains(y.as_str()))
                    || text.trim_start().starts_with(surname.as_str()))
        }
        Named::NumberedCitation(key) => text.contains(&format!("[{key}]")),
        _ => false,
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// The crop a link destination previews: the float under it, the target
/// the page has there, or - running text, an unknown page - the preview's
/// ordinary crop around the anchor. A destination that is only a page, or
/// one many links share (the top of a reference list), is read by the
/// link's own words instead: what they name, where the page has it.
pub fn link_crop(
    page: Option<&Page>,
    runs: &[crate::engine::types::TextRun],
    page_w_pt: f32,
    page_h_pt: f32,
    x: Option<f32>,
    y: Option<f32>,
    text: Option<&LinkText>,
) -> Rect {
    link_preview(page, runs, page_w_pt, page_h_pt, x, y, text).rect
}

/// `link_crop`, with how the target runs on and the row it marks.
pub fn link_preview(
    page: Option<&Page>,
    runs: &[crate::engine::types::TextRun],
    page_w_pt: f32,
    page_h_pt: f32,
    x: Option<f32>,
    y: Option<f32>,
    text: Option<&LinkText>,
) -> Target {
    let input = crate::engine::preview::CropInput {
        page_w_pt,
        page_h_pt,
        dest_x: x,
        dest_y: y,
        runs,
    };
    let by_anchor = y.and_then(|y| {
        crate::engine::floats::float_for_anchor(runs, page_w_pt, page_h_pt, x, y)
            .map(Target::plain)
            .or_else(|| page.and_then(|page| found_at(page, x, y)))
    });
    let named = text.and_then(named);
    let by_words = page.zip(named.as_ref()).and_then(|(page, named)| {
        locate_named(page, runs, named).map(|t| Target {
            rect: clear_of_neighbours(t.rect, page),
            ..t
        })
    });
    // What the anchor found already answers to the words (two entries by
    // the same author in the same year): the anchor is the finer.
    let answers = |rect: Rect| {
        page.zip(named.as_ref())
            .is_some_and(|(page, named)| answers_to(page, rect, named))
    };
    match (by_anchor, by_words) {
        // They disagree: the destination is coarse, the words are not.
        (Some(anchor), Some(words))
            if !overlaps(anchor.rect, words.rect) && !answers(anchor.rect) =>
        {
            words
        }
        (Some(anchor), _) => anchor,
        (None, Some(words)) => words,
        (None, None) => Target::plain(crate::engine::preview::crop_rect(&input)),
    }
}

/// The text after the first line's label: "[12] Author" -> "Author".
pub(crate) fn labelled(text: &str) -> bool {
    let t = text.trim_start();
    // A review copy's line number before it: "351 [12] ...".
    let t = match t.split_once(' ') {
        Some((number, rest)) if number.chars().all(|c| c.is_ascii_digit()) => rest.trim_start(),
        _ => t,
    };
    let Some(rest) = t.strip_prefix('[') else {
        return false;
    };
    let Some(close) = rest.find(']') else {
        return false;
    };
    close > 0 && close <= 12 && rest[..close].chars().any(|c| c.is_alphanumeric())
}

/// Whether `l` is a piece of the row `p` is on: a ligature splits a line in
/// the page's text, and an accent or a script over a letter sits a little
/// above it.
pub(crate) fn same_row(p: &Line, l: &Line) -> bool {
    let centre = (l.top + l.bottom) / 2.0;
    centre > p.bottom && centre < p.top + 0.5 * p.em()
}

/// The next line in reading order, past running heads and feet.
pub(crate) fn next_line(page: &Page, i: usize) -> Option<usize> {
    (i + 1..page.lines.len()).find(|&j| !page.lines[j].flags.furniture)
}

/// The bibliography entry starting under the anchor: its first line opens
/// with a label ("[12]"), or the entry hangs - its next line indented past
/// its first. It runs on to the line before the next entry: the next
/// label, or the next line back at the first line's margin; over a column
/// or page break as far as the column runs.
fn bibliography_entry(page: &Page, x: Option<f32>, y: f32) -> Option<Rect> {
    let lines = &page.lines;
    let candidates: Vec<usize> = (0..lines.len())
        .filter(|&i| {
            let l = &lines[i];
            !l.flags.furniture
                && l.top <= y + ANCHOR_BELOW_PT
                && l.top >= y - 30.0
                && x.is_none_or(|x| l.x0 <= x + 30.0 && l.x1 >= x)
        })
        .collect();
    // A labelled line near the anchor is the entry; else the first line
    // whose top the anchor is on or over - not the previous entry's last
    // line, whose top can be just above it.
    let nearest = |of: &mut dyn Iterator<Item = usize>| {
        of.min_by(|&a, &b| {
            (lines[a].top - y)
                .abs()
                .total_cmp(&(lines[b].top - y).abs())
        })
    };
    let first = nearest(
        &mut candidates
            .iter()
            .copied()
            .filter(|&i| labelled(&page.line_text(i))),
    )
    .or_else(|| {
        candidates
            .iter()
            .copied()
            .filter(|&i| lines[i].top <= y + 2.0)
            .max_by(|&a, &b| lines[a].top.total_cmp(&lines[b].top))
    })?;
    let head = &lines[first];
    let label = labelled(&page.line_text(first));
    // The page's last line has no next line: a one-line entry, if any.
    let second = next_line(page, first);
    // Close: a line break's gap, single or double spaced.
    let reach = 1.6 * leading_gap(page);
    let close = |a: &Line, b: &Line| a.bottom - b.top <= (0.9 * a.em()).max(reach) && b.top < a.top;
    // A hanging list: this entry's next line indented, or - a one-line
    // entry - other entries starting at this line's margin and going on
    // indented.
    let indents = |a: &Line, b: &Line| {
        (3.0..=30.0).contains(&(b.x0 - a.x0))
            && a.bottom - b.top <= 1.2 * a.em()
            && b.top < a.top
            && same_column(a, b)
    };
    let hangs_here = (0..lines.len()).filter(|&i| {
        let l = &lines[i];
        (l.x0 - head.x0).abs() <= 1.5
            && same_column(l, head)
            && next_line(page, i).is_some_and(|j| indents(l, &lines[j]))
    });
    let hanging = second.is_some_and(|j| indents(head, &lines[j]) && close(head, &lines[j]))
        || hangs_here.count() >= 2;
    if !label && !hanging {
        return None;
    }
    let mut region = line_rect(head);
    let mut previous = first;
    let mut cursor = first;
    let mut count = 1;
    while count < 20 {
        let Some(j) = next_line(page, cursor) else {
            break;
        };
        cursor = j;
        let l = &lines[j];
        // A piece of the row before (a ligature splits a line) is its.
        if same_row(&lines[previous], l) {
            region = union(region, line_rect(l));
            continue;
        }
        let text = page.line_text(j);
        let new_entry = labelled(&text) || (hanging && l.x0 <= head.x0 + 1.5);
        let in_column = same_column(l, head) && close(&lines[previous], l);
        if new_entry || !in_column {
            break;
        }
        region = union(region, line_rect(l));
        previous = j;
        count += 1;
    }
    Some(pad(region, 3.0, page))
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::*;

    fn references(b: &mut Builder, hanging: bool) {
        b.line(740.0, BOLD, "References");
        let mut y = 720.0;
        for n in 1..=6 {
            let label = format!("[{n}] A. Author{n}, B. Writer, A title of the paper number {n},");
            b.text(72.0, y, BODY, &label);
            b.newline();
            y -= 12.0;
            for _ in 0..(n % 3 + 1) {
                b.text(
                    if hanging { 92.0 } else { 72.0 },
                    y,
                    BODY,
                    "Journal of Things 12 (2019) 345-678, more words here.",
                );
                b.newline();
                y -= 12.0;
            }
        }
    }

    #[test]
    fn a_labelled_reference_previews_its_whole_entry_only() {
        let mut b = Builder::new();
        references(&mut b, true);
        let page = b.page();
        // [1] at 720 with two more lines (708, 696); [2] at 684 with three
        // (672, 660, 648); [3] at 636. Lines are 7.5pt tall.
        let rect = link_target(&page, Some(72.0), 694.0).expect("an entry");
        let top = rect[1] + rect[3];
        assert!((691.5..696.0).contains(&top), "[1]'s last line: {rect:?}");
        assert!(rect[1] <= 648.0 && rect[1] > 643.5, "[3]: {rect:?}");
    }

    #[test]
    fn running_text_is_no_target() {
        let mut b = Builder::new();
        for i in 0..10 {
            b.line(
                700.0 - 12.0 * i as f32,
                BODY,
                "Running text from margin to margin of the column.",
            );
        }
        assert_eq!(link_target(&b.page(), Some(72.0), 712.0), None);
    }

    #[test]
    fn a_theorem_link_previews_its_statement_not_its_proof() {
        let mut b = Builder::new();
        b.line(
            740.0,
            BODY,
            "Running text before the statement, wide enough.",
        );
        let x = b.text(72.0, 720.0, BOLD, "Theorem 2.");
        b.text(
            x + 5.0,
            720.0,
            ITALIC,
            "Every bounded thing is bounded, and",
        );
        b.newline();
        b.line(708.0, ITALIC, "so is every other thing that is bounded.");
        b.line(690.0, BOLD, "Proof.");
        b.line(
            678.0,
            BODY,
            "Obvious from the statement, and we are done here.",
        );
        let rect = link_target(&b.page(), Some(72.0), 732.0).expect("a statement");
        assert!(rect[1] + rect[3] >= 727.5, "{rect:?}");
        assert!(rect[1] > 697.0, "the proof: {rect:?}");
        assert!(rect[1] <= 708.0, "{rect:?}");
    }

    /// A reference list as in an alpha/plain-style paper: no labels, each
    /// entry's further lines indented, entries set a little apart, some a
    /// single line.
    fn author_year_references() -> Page {
        let mut b = Builder::new();
        let entries: &[(&str, &[&str])] = &[
            (
                "A. Beimel, K. Nissim, and U. Stemmer. Characterizing the sample complexity.",
                &["In ITCS, pages 1-10, 2013."],
            ),
            (
                "A. Blumer, A. Ehrenfeucht, D. Haussler, and M. Warmuth. Occam's razor. 1987.",
                &[],
            ),
            (
                "M. Braverman and A. Garg. Public vs private coin in bounded-round information.",
                &["In ICALP, pages 502-513, 2014."],
            ),
            (
                "C. Dwork, V. Feldman, M. Hardt, and A. Roth. Generalization in adaptive analysis.",
                &["In NIPS, 2015."],
            ),
            (
                "C. Dwork, V. Feldman, M. Hardt, and A. Roth. Preserving statistical validity.",
                &["In STOC, 2015."],
            ),
            (
                "N. Littlestone and M. Warmuth. Relating data compression and learnability. 1986.",
                &[],
            ),
        ];
        let mut y = 720.0;
        for (first, rest) in entries {
            b.text(69.0, y, BODY, first);
            b.newline();
            y -= 12.0;
            for line in *rest {
                b.text(80.0, y, BODY, line);
                b.newline();
                y -= 12.0;
            }
            y -= 4.0;
        }
        b.page()
    }

    fn words(before: &str, inside: &str) -> LinkText {
        LinkText {
            before: before.into(),
            inside: inside.into(),
        }
    }

    fn lines_in(page: &Page, rect: Rect) -> Vec<String> {
        (0..page.lines.len())
            .filter(|&i| {
                let l = &page.lines[i];
                l.bottom >= rect[1] - 0.5 && l.top <= rect[1] + rect[3] + 0.5
            })
            .map(|i| page.line_text(i))
            .collect()
    }

    #[test]
    fn a_hanging_one_line_entry_previews_alone() {
        // Blumer et al. starts at 720 - 2*12 - 4 = 692; one line.
        let page = author_year_references();
        let rect = link_target(&page, Some(41.0), 701.0).expect("an entry");
        let lines = lines_in(&page, rect);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("A. Blumer"), "{lines:?}");
    }

    #[test]
    fn a_shared_destination_is_read_by_the_links_words() {
        // Every citation goes to the top of the list; the words name
        // Littlestone and Warmuth, 1986.
        let page = author_year_references();
        let text = words("compression schemes (", "Littlestone and Warmuth, 1986");
        let rect = link_crop(
            Some(&page),
            &[],
            612.0,
            792.0,
            Some(41.0),
            Some(730.0),
            Some(&text),
        );
        let lines = lines_in(&page, rect);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("N. Littlestone"), "{lines:?}");
    }

    #[test]
    fn same_author_and_year_keeps_the_anchors_entry() {
        // Two Dwork et al. 2015 entries: the words fit both, the anchor
        // says which.
        let page = author_year_references();
        let second_dwork = 720.0 - 12.0 * 7.0 - 4.0 * 4.0;
        let text = words("(", "Dwork et al., 2015");
        let rect = link_crop(
            Some(&page),
            &[],
            612.0,
            792.0,
            Some(41.0),
            Some(second_dwork + 9.0),
            Some(&text),
        );
        let lines = lines_in(&page, rect);
        assert!(lines[0].contains("Preserving"), "{lines:?}");
        assert_eq!(lines.len(), 2, "{lines:?}");
    }

    #[test]
    fn an_alpha_label_is_found_by_its_key() {
        let mut b = Builder::new();
        b.line(740.0, BOLD, "References");
        for (n, key) in ["AL22", "BNS13", "KS23"].iter().enumerate() {
            let y = 720.0 - 30.0 * n as f32;
            b.text(
                72.0,
                y,
                BODY,
                &format!("[{key}] Some Author. A title of a paper, in a journal,"),
            );
            b.newline();
            b.text(110.0, y - 12.0, BODY, "volume 12, pages 1-10, 2022.");
            b.newline();
        }
        let page = b.page();
        let text = words("as shown in [", "BNS13");
        let rect = link_crop(Some(&page), &[], 612.0, 792.0, None, None, Some(&text));
        let lines = lines_in(&page, rect);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("[BNS13]"), "{lines:?}");
    }

    #[test]
    fn link_words_are_read_from_runs() {
        let run = |text: &str, x: f32, w: f32| crate::engine::types::TextRun {
            text: text.into(),
            start: 0,
            x,
            y: 700.0,
            w,
            h: 8.0,
            edges: Vec::new(),
            math: Vec::new(),
        };
        let runs = vec![run("as in Theorem", 72.0, 65.0), run("2.1,", 140.0, 20.0)];
        let text = LinkText::from_runs(&runs, [139.0, 699.0, 14.0, 10.0]);
        assert_eq!(text.inside, "2.1");
        assert!(text.before.ends_with("Theorem "), "{text:?}");
    }
}

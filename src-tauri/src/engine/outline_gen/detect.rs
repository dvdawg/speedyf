//! Headings recovered from a document's typography, for PDFs that ship no
//! outline.
//!
//! Precision over recall throughout: a short correct outline beats a long
//! noisy one, and when the evidence is thin the answer is no outline at
//! all, the state the reader was already in. Pure: every rule here runs on
//! hand-built pages in the tests below.
//!
//! The layers, most authoritative first:
//!
//! 1. A tagged PDF's own `H1`..`H6` structure.
//! 2. A printed table of contents, mapped onto the pages it names
//!    (`toc`) - unless the headings found on the pages reproduce it, in
//!    which case those are used, since a printed contents often lists
//!    sections only.
//! 3. Typography: lines set apart from the body text - larger, bolder, in
//!    capitals or small caps, with space above - that are short, start a
//!    block, and are not captions, theorem labels, equations, list items,
//!    references or title-page matter. Then validation: section numbers
//!    must run in order, a heading style must recur and must not also be
//!    the style of ordinary lines, and at least two top-level headings
//!    must survive.
//!
//! The title is left out of the outline. It names the document rather
//! than a place in it, the tab already shows it, and a title-page line is
//! exactly the kind of large type the detector must otherwise refuse.

use super::normalize;
use super::numbering::{self, Kind, Numbering};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// One typeset line with the facts a heading is recognised by. Positions
/// in points from the displayed page's top-left corner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub text: String,
    pub size: f32,
    pub bold: bool,
    /// Bold by the font's name or flags alone, not by the weight PDFium
    /// reports - which some files state as bold for every font.
    pub bold_named: bool,
    pub italic: bool,
    pub smallcaps: bool,
    /// Share of the line's glyphs set in a mathematics font.
    pub math: f32,
    /// The font most of the line is set in (a `FontTable` id).
    pub font: u32,
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
    /// The bold words the line opens with, when the rest is not bold.
    pub lead: Option<Lead>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lead {
    pub text: String,
    pub size: f32,
}

/// A heading a tagged PDF's structure tree names.
#[derive(Clone, Debug, PartialEq)]
pub struct TaggedHeading {
    /// 1 for `H1`.
    pub level: u8,
    pub text: String,
    pub top: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    /// Index in the file.
    pub index: u32,
    pub width: f32,
    pub height: f32,
    pub lines: Vec<Line>,
    pub label: Option<String>,
    /// Inked characters on the page: zero for a scan with no text layer.
    pub chars: u32,
    pub tagged: Vec<TaggedHeading>,
}

/// Font facts the detector needs, by `Line::font`.
#[derive(Clone, Copy, Debug, Default)]
pub struct FontInfo {
    pub glyphless: bool,
}

/// A whole document as read so far.
#[derive(Default)]
pub struct Doc {
    pub pages: Vec<Page>,
    pub fonts: Vec<FontInfo>,
    pub tagged: bool,
    pub page_count: u32,
}

/// One generated outline entry, flat; `level` 0 is the top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub page: u32,
    /// Where to land: the heading's top edge, in page space (points, y-up),
    /// a little above the ink so the line is not flush with the edge.
    pub y: Option<f32>,
}

/// Levels shown; deeper headings are left out rather than flattened.
pub const MAX_LEVELS: u8 = 3;
/// A heading is at most this many words; a wrapped one, joined, at most
/// `MAX_JOINED_WORDS`.
const MAX_WORDS: usize = 12;
const MAX_JOINED_WORDS: usize = 20;
/// Larger than the body by both this ratio and this many points.
const LARGER_RATIO: f32 = 1.1;
const LARGER_POINTS: f32 = 0.7;
/// A heading style must be at least this share of the lines set in it.
const MIN_PURITY: f32 = 0.5;
/// Running heads and footers sit in these bands of the page.
const MARGIN_BAND: f32 = 0.055;
const RUNNING_BAND: f32 = 0.13;
/// Space above a heading beyond the body's own line gap, in body ems.
const EXTRA_SPACE_EMS: f32 = 0.35;

/// The outline for a document: tagged structure first, then a printed
/// table of contents, then typography. Empty when none is confident.
pub fn generate(doc: &Doc) -> Vec<Heading> {
    if doc.pages.len() < 2 {
        return Vec::new();
    }
    if doc.tagged {
        if let Some(headings) = tagged_outline(doc) {
            return headings;
        }
    }
    if is_scanned(doc) || is_slides(doc) {
        return Vec::new();
    }
    if let Some(repaired) = without_false_weight(doc) {
        return generate_typeset(&repaired);
    }
    generate_typeset(doc)
}

fn generate_typeset(doc: &Doc) -> Vec<Heading> {
    let stats = Stats::compute(doc);
    let typeset = typographic(doc, &stats);
    if let Some(toc) = super::toc::outline(doc, &stats) {
        // The printed contents is the author's own list, but often a
        // shallow one (sections only). Headings found on the pages that
        // reproduce it are as trustworthy and may go deeper.
        if super::toc::covers(&typeset, &toc) {
            return typeset;
        }
        // A contents page that comes after the document's first numbered
        // section lists only part of it - an appendix's own contents. (A
        // book's preface may precede its contents; its chapters do not.)
        let contents_page = stats.toc_pages.iter().min().map(|&p| doc.pages[p].index);
        let first_numbered = typeset
            .iter()
            .find(|h| numbering::parse(&h.title).is_some())
            .map(|h| h.page);
        if first_numbered.is_some_and(|first| contents_page.is_some_and(|c| c > first)) {
            return typeset;
        }
        return in_page_order(toc);
    }
    typeset
}

/// The longest run of `headings` whose pages never go backwards: an outline
/// is read top to bottom, and an entry that would jump back - a contents
/// line mapped through a misread page number - is the one left out.
pub fn in_page_order(headings: Vec<Heading>) -> Vec<Heading> {
    let n = headings.len();
    if n < 2 {
        return headings;
    }
    let mut best = vec![1usize; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        for j in 0..i {
            if headings[j].page <= headings[i].page && best[j] + 1 > best[i] {
                best[i] = best[j] + 1;
                back[i] = Some(j);
            }
        }
    }
    let mut end = 0;
    for i in 0..n {
        if best[i] > best[end] {
            end = i;
        }
    }
    let mut keep = vec![false; n];
    let mut at = Some(end);
    while let Some(i) = at {
        keep[i] = true;
        at = back[i];
    }
    headings
        .into_iter()
        .zip(keep)
        .filter_map(|(h, k)| k.then_some(h))
        .collect()
}

/// A copy of the document with boldness read from font names alone, when
/// the reported weights call most of the body text bold but its fonts'
/// names do not: then the weight says nothing, and a bold heading font is
/// told apart only by name ("CharisSIL-Bold" against "CharisSIL").
fn without_false_weight(doc: &Doc) -> Option<Doc> {
    let (mut chars, mut weighted, mut named) = (0usize, 0usize, 0usize);
    for line in doc.pages.iter().flat_map(|p| &p.lines) {
        let n = chars_of(line);
        chars += n;
        if line.bold {
            weighted += n;
        }
        if line.bold_named {
            named += n;
        }
    }
    if weighted * 2 <= chars || named * 2 > chars {
        return None;
    }
    let pages = doc
        .pages
        .iter()
        .map(|page| Page {
            lines: page
                .lines
                .iter()
                .map(|line| Line {
                    bold: line.bold_named,
                    lead: None,
                    ..line.clone()
                })
                .collect(),
            tagged: Vec::new(),
            label: page.label.clone(),
            ..*page
        })
        .collect();
    Some(Doc {
        pages,
        fonts: doc.fonts.clone(),
        tagged: doc.tagged,
        page_count: doc.page_count,
    })
}

// ---------------------------------------------------------------------------
// Layer 1: tagged structure

fn tagged_outline(doc: &Doc) -> Option<Vec<Heading>> {
    let mut flat: Vec<(u8, String, u32, f32, f32)> = Vec::new();
    for page in &doc.pages {
        for heading in &page.tagged {
            let title = normalize::display(&heading.text);
            if title.chars().filter(|c| c.is_alphanumeric()).count() < 2
                || title.split_whitespace().count() > 30
            {
                continue;
            }
            flat.push((heading.level, title, page.index, heading.top, page.height));
        }
    }
    // Levels as used: H1/H3 with no H2 is two levels, not three.
    let mut used: Vec<u8> = flat.iter().map(|h| h.0).collect();
    used.sort_unstable();
    used.dedup();
    let headings: Vec<Heading> = flat
        .into_iter()
        .filter_map(|(level, title, page, top, height)| {
            let rank = used.iter().position(|l| *l == level)? as u8;
            (rank < MAX_LEVELS).then(|| Heading {
                level: rank,
                title,
                page,
                y: Some(anchor_y(height, top)),
            })
        })
        .collect();
    let top = headings.iter().filter(|h| h.level == 0).count();
    (top >= 2).then_some(headings)
}

/// Page-space y (up from the bottom) for a line whose top is `top` points
/// down from the page's top edge, with a little air above it.
pub fn anchor_y(page_height: f32, top: f32) -> f32 {
    (page_height - top + 4.0).min(page_height)
}

// ---------------------------------------------------------------------------
// Documents the detector does not attempt

/// Most pages without a text layer: a scan. Its OCR layer, if any, has no
/// font facts worth reading; the outline is left to the reader.
fn is_scanned(doc: &Doc) -> bool {
    let empty = doc.pages.iter().filter(|p| p.chars < 50).count();
    empty * 2 > doc.pages.len()
}

/// Slides: landscape pages carrying little text each, where large type is
/// everywhere and "the body font" means nothing.
fn is_slides(doc: &Doc) -> bool {
    let landscape = doc
        .pages
        .iter()
        .filter(|p| p.width > p.height * 1.15)
        .count();
    if landscape * 3 < doc.pages.len() * 2 {
        return false;
    }
    let chars: u32 = doc.pages.iter().map(|p| p.chars).sum();
    (chars as usize) < doc.pages.len() * 1500
}

// ---------------------------------------------------------------------------
// Document statistics

/// Size buckets of half a point.
fn bucket(size: f32) -> i32 {
    (size * 2.0).round() as i32
}

/// What the body text looks like, and which lines are page furniture.
pub struct Stats {
    pub body: f32,
    pub body_bold: bool,
    pub body_font: u32,
    /// Median gap between consecutive body lines (ink bottom to ink top).
    pub gap: f32,
    /// (page position in `doc.pages`, line index) of running heads, page
    /// numbers and other furniture.
    pub furniture: HashSet<(usize, usize)>,
    /// Pages that are a table of contents (or a list of figures), whose
    /// lines must never be read as headings.
    pub toc_pages: HashSet<usize>,
    /// The text layer is an OCR tool's: sizes and weights mean nothing.
    pub ocr_like: bool,
}

/// Letters of a line with every digit folded to '#', so "Page 3" and
/// "Page 14" are the same running footer.
fn furniture_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| if c.is_numeric() { '#' } else { c })
        .flat_map(char::to_lowercase)
        .collect()
}

fn chars_of(line: &Line) -> usize {
    line.text.chars().filter(|c| !c.is_whitespace()).count()
}

impl Stats {
    pub fn compute(doc: &Doc) -> Stats {
        // Body size, weight and font: the mode by character count.
        let mut by_size: HashMap<i32, usize> = HashMap::new();
        for line in doc.pages.iter().flat_map(|p| &p.lines) {
            *by_size.entry(bucket(line.size)).or_default() += chars_of(line);
        }
        let body_bucket = by_size
            .iter()
            .max_by_key(|(b, n)| (**n, -**b))
            .map_or(20, |(b, _)| *b);
        let body = body_bucket as f32 / 2.0;
        let (mut bold_chars, mut all_chars) = (0usize, 0usize);
        let mut by_font: HashMap<u32, usize> = HashMap::new();
        for line in doc.pages.iter().flat_map(|p| &p.lines) {
            if bucket(line.size) == body_bucket {
                let n = chars_of(line);
                all_chars += n;
                if line.bold {
                    bold_chars += n;
                }
                *by_font.entry(line.font).or_default() += n;
            }
        }
        let body_font = by_font
            .iter()
            .max_by_key(|(f, n)| (**n, std::cmp::Reverse(**f)))
            .map_or(0, |(f, _)| *f);
        let ocr_like = doc
            .fonts
            .get(body_font as usize)
            .is_some_and(|f| f.glyphless);

        // The gap between consecutive body lines.
        let mut gaps: Vec<f32> = Vec::new();
        for page in &doc.pages {
            for (i, line) in page.lines.iter().enumerate() {
                if bucket(line.size) != body_bucket || chars_of(line) < 20 {
                    continue;
                }
                if let Some(j) = below(page, i) {
                    let next = &page.lines[j];
                    let gap = next.top - line.bottom;
                    if bucket(next.size) == body_bucket && gap > -1.0 && gap < 2.0 * body {
                        gaps.push(gap);
                    }
                }
            }
        }
        gaps.sort_by(f32::total_cmp);
        let gap = gaps.get(gaps.len() / 2).copied().unwrap_or(0.25 * body);

        let furniture = furniture(doc, body);
        let toc_pages = super::toc::toc_pages(doc);
        Stats {
            body,
            body_bold: bold_chars * 2 > all_chars,
            body_font,
            gap,
            furniture,
            toc_pages,
            ocr_like,
        }
    }
}

/// Running heads and feet, page numbers, stamps: text in the page's margin
/// bands, or repeated at the same height on several pages.
fn furniture(doc: &Doc, body: f32) -> HashSet<(usize, usize)> {
    let mut seen: HashMap<(String, i32), HashSet<usize>> = HashMap::new();
    for (p, page) in doc.pages.iter().enumerate() {
        for line in &page.lines {
            let key = furniture_key(&line.text);
            if key.is_empty() {
                continue;
            }
            let height = (line.top / 8.0).round() as i32;
            seen.entry((key, height)).or_default().insert(p);
        }
    }
    let pages = doc.pages.len();
    let threshold = if pages <= 4 { 2 } else { 3 };
    let mut out = HashSet::new();
    for (p, page) in doc.pages.iter().enumerate() {
        let h = page.height.max(1.0);
        for (i, line) in page.lines.iter().enumerate() {
            let in_margin = line.top < MARGIN_BAND * h || line.bottom > (1.0 - MARGIN_BAND) * h;
            if in_margin && line.size < 1.3 * body {
                out.insert((p, i));
                continue;
            }
            let key = furniture_key(&line.text);
            if key.is_empty() {
                out.insert((p, i));
                continue;
            }
            if line.text.contains("arXiv:") {
                out.insert((p, i));
                continue;
            }
            let height = (line.top / 8.0).round() as i32;
            let mut on: HashSet<usize> = HashSet::new();
            for dh in -1..=1 {
                if let Some(set) = seen.get(&(key.clone(), height + dh)) {
                    on.extend(set);
                }
            }
            let near_edge = line.top < RUNNING_BAND * h || line.bottom > (1.0 - RUNNING_BAND) * h;
            if (on.len() >= threshold && near_edge) || (pages >= 5 && on.len() * 5 >= pages * 2) {
                out.insert((p, i));
            }
        }
    }
    out
}

fn overlaps(a: &Line, b: &Line) -> bool {
    a.left.max(b.left) < a.right.min(b.right)
}

/// The nearest line above `i` in its column (overlapping it horizontally).
pub fn above(page: &Page, i: usize) -> Option<usize> {
    let line = &page.lines[i];
    let mut best: Option<usize> = None;
    for (j, other) in page.lines.iter().enumerate() {
        if j == i || !overlaps(line, other) || other.bottom > line.top + 0.3 * line.size {
            continue;
        }
        if other.top >= line.top {
            continue;
        }
        if best.is_none_or(|b| other.bottom > page.lines[b].bottom) {
            best = Some(j);
        }
    }
    best
}

/// The nearest line below `i` in its column.
pub fn below(page: &Page, i: usize) -> Option<usize> {
    let line = &page.lines[i];
    let mut best: Option<usize> = None;
    for (j, other) in page.lines.iter().enumerate() {
        if j == i || !overlaps(line, other) || other.top < line.bottom - 0.3 * line.size {
            continue;
        }
        if other.top <= line.top {
            continue;
        }
        if best.is_none_or(|b| other.top < page.lines[b].top) {
            best = Some(j);
        }
    }
    best
}

/// Reading-order key within a page: on a two-column page, full-width lines
/// cut it into bands, and each band reads left column then right.
fn order_keys(page: &Page) -> Vec<(u32, u8, f32)> {
    let mid = page.width / 2.0;
    let slack = 0.04 * page.width;
    let side = |line: &Line| -> u8 {
        if line.right <= mid + slack {
            0
        } else if line.left >= mid - slack {
            1
        } else {
            2
        }
    };
    let (mut left, mut right, mut total) = (0usize, 0usize, 0usize);
    for line in &page.lines {
        let n = chars_of(line);
        if n < 20 {
            continue;
        }
        total += n;
        match side(line) {
            0 => left += n,
            1 => right += n,
            _ => {}
        }
    }
    let two_column = total > 0 && left * 4 >= total && right * 4 >= total;
    let full: Vec<f32> = if two_column {
        page.lines
            .iter()
            .filter(|l| side(l) == 2 && chars_of(l) >= 3)
            .map(|l| l.top)
            .collect()
    } else {
        Vec::new()
    };
    page.lines
        .iter()
        .map(|line| {
            if !two_column {
                return (0, 0, line.top);
            }
            let band = full.iter().filter(|t| **t <= line.top + 0.5).count() as u32;
            match side(line) {
                2 => (band, 0, line.top),
                s => (band, s, line.top),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Layer 3: typography

/// What makes a style a style: two lines in the same one are set alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StyleKey {
    size: i32,
    bold: bool,
    caps: bool,
    smallcaps: bool,
    font: u32,
}

#[derive(Clone, Debug)]
struct Cand {
    /// position in `doc.pages`
    p: usize,
    page: u32,
    top: f32,
    order: (u32, u8, f32),
    style: StyleKey,
    /// how far the style stands out from the body
    prominence: f32,
    /// set in larger type than the body
    larger: bool,
    numbering: Option<Numbering>,
    known: bool,
    /// the title as shown
    title: String,
    words: usize,
    /// set by validation
    depth: Option<usize>,
    run_in: bool,
}

/// Theorem-like labels, captions and the like: not sections when followed
/// by a counter, punctuation or nothing ("Theorem 2.1", "Proof.", "Case
/// 2:"), though they may open a real title ("Problem formulation", "Case
/// study").
const LABEL_WORDS: &[&str] = &[
    "theorem",
    "lemma",
    "proposition",
    "corollary",
    "definition",
    "remark",
    "remarks",
    "example",
    "proof",
    "claim",
    "conjecture",
    "assumption",
    "hypothesis",
    "fact",
    "observation",
    "note",
    "exercise",
    "problem",
    "question",
    "condition",
    "property",
    "axiom",
    "case",
    "step",
    "input",
    "output",
    "require",
    "ensure",
    "return",
    "solution",
    "answer",
    "hint",
    "algorithm",
    "procedure",
    "figure",
    "fig",
    "table",
    "tab",
    "listing",
    "scheme",
    "chart",
    "plate",
    "equation",
    "eq",
];

/// Front-matter fields: never a section, whatever follows.
const FIELD_WORDS: &[&str] = &[
    "keywords",
    "keyword",
    "pacs",
    "msc",
    "jel",
    "received",
    "accepted",
    "published",
    "revised",
    "submitted",
    "communicated",
    "correspondence",
    "corresponding",
    "email",
    "doi",
    "citation",
    "copyright",
    "license",
    "edited",
    "reviewed",
];

/// Words that mark an affiliation, not a heading, when the line also has a
/// comma.
const AFFILIATION_WORDS: &[&str] = &[
    "university",
    "department",
    "institute",
    "laboratory",
    "college",
    "school",
    "centre",
    "center",
    "faculty",
    "universit",
];

fn first_word_lower(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_lowercase()
}

/// Mathematics by its characters: relations and operators a heading has no
/// use for.
fn mathy(text: &str) -> bool {
    // ascii-guard: data begin - operators that mark a displayed equation
    let ops = text
        .chars()
        .filter(|c| {
            matches!(
                c,
                '=' | '<'
                    | '>'
                    | '+'
                    | '\u{2264}'
                    | '\u{2265}'
                    | '\u{2211}'
                    | '\u{222b}'
                    | '\u{2202}'
                    | '\u{2208}'
                    | '\u{2192}'
                    | '\u{00b1}'
                    | '\u{00d7}'
                    | '|'
                    | '^'
                    | '_'
                    | '\u{221e}'
                    | '\u{2248}'
                    | '\u{2260}'
                    | '\u{2212}'
            )
        })
        .count();
    // ascii-guard: data end
    ops >= 2 || text.contains('=')
}

/// Whether a title is made of words: at least one of four letters or more,
/// most of its tokens words, and not opening on a number ("5 2x lhil") or
/// on a sentence's adverb ("Finally, we consider").
fn reads_as_words(title: &str) -> bool {
    let tokens: Vec<&str> = title.split_whitespace().collect();
    let Some(first) = tokens.first() else {
        return false;
    };
    if first.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    if let Some(adverb) = first.strip_suffix(',') {
        let lower = adverb.to_lowercase();
        if lower.ends_with("ly")
            || matches!(
                lower.as_str(),
                "however"
                    | "moreover"
                    | "thus"
                    | "hence"
                    | "therefore"
                    | "furthermore"
                    | "first"
                    | "second"
                    | "next"
                    | "then"
                    | "here"
                    | "now"
                    | "also"
                    | "indeed"
                    | "nevertheless"
                    | "instead"
                    | "again"
                    | "still"
            )
        {
            return false;
        }
    }
    // A word: letters, with hyphens, slashes and apostrophes inside, and a
    // digit or two after its first letter ("W2", "QUBO/Ising", "Lp").
    let wordy = |t: &str| {
        let core = t.trim_matches(|c: char| !c.is_alphanumeric());
        let digits = core.chars().filter(char::is_ascii_digit).count();
        core.starts_with(char::is_alphabetic)
            && digits <= 2
            && core
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | '\'' | '\u{2019}' | '&'))
    };
    let number = |t: &str| {
        let core = t.trim_matches(|c: char| !c.is_alphanumeric());
        !core.is_empty() && core.chars().all(|c| c.is_ascii_digit())
    };
    let words = tokens.iter().filter(|t| wordy(t)).count();
    // A plain number ("Phase 2") is neither word nor noise.
    let numbers = tokens.iter().filter(|t| number(t)).count();
    let long = tokens
        .iter()
        .any(|t| wordy(t) && t.chars().filter(|c| c.is_alphabetic()).count() >= 4);
    long && words * 10 >= (tokens.len() - numbers) * 6
}

/// A review copy's line number run into its line ("15 1 Introduction"):
/// dropped when what follows it is itself a numbered heading.
fn strip_line_number(text: &str) -> String {
    let Some((first, rest)) = text.split_once(' ') else {
        return text.to_owned();
    };
    let is_number =
        !first.is_empty() && first.len() <= 4 && first.chars().all(|c| c.is_ascii_digit());
    if is_number && numbering::parse(text).is_none() {
        if let Some(n) = numbering::parse(rest) {
            if !n.title.is_empty() {
                return rest.to_owned();
            }
        }
    }
    // No section is numbered in the hundreds: "312 References" is a line
    // number and a heading.
    if is_number && first.len() >= 3 && rest.starts_with(char::is_uppercase) {
        return rest.to_owned();
    }
    text.to_owned()
}

/// Where a heading's own sentence ends inside its line: the first ". "
/// past the numbering that is followed by a capital and at least two more
/// words. Returns the byte offset of the period.
fn sentence_break(text: &str) -> Option<usize> {
    let skip = numbering::parse(text).map_or(0, |n| n.prefix.len());
    let mut search = skip.min(text.len());
    while let Some(found) = text[search..].find(". ") {
        let at = search + found;
        let rest = &text[at + 2..];
        // The word the period ends must be a word, not an abbreviation
        // ("Fig.", "Thm.", "et al.").
        let last_word = text[skip..at]
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_lowercase();
        let abbreviation = last_word.chars().count() < 3
            || matches!(
                last_word.as_str(),
                "fig"
                    | "eqs"
                    | "thm"
                    | "sec"
                    | "ref"
                    | "refs"
                    | "prop"
                    | "lem"
                    | "def"
                    | "cor"
                    | "vol"
                    | "etc"
                    | "resp"
                    | "approx"
                    | "eq"
            );
        if !abbreviation
            && rest.starts_with(|c: char| c.is_uppercase())
            && rest.split_whitespace().count() >= 2
        {
            return Some(at);
        }
        search = at + 2;
    }
    None
}

/// Back-matter names that end a document's main run of sections.
fn is_back_matter(title: &str) -> bool {
    let key = normalize::letters(title);
    normalize::is_top_level_name(title)
        && !matches!(key.as_str(), "abstract" | "preface" | "foreword")
        || key.contains("checklist")
}

/// The right edge of the column a line sits in: the furthest any long line
/// overlapping it reaches.
fn column_right(page: &Page, line: &Line) -> f32 {
    page.lines
        .iter()
        .filter(|other| chars_of(other) >= 30 && overlaps(line, other))
        .map(|other| other.right)
        .fold(line.right, f32::max)
}

/// Reasons a line's text rules it out as a heading, whatever its style.
fn text_rejects(text: &str, numbered: bool) -> bool {
    let trimmed = text.trim();
    let letters = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    let visible = trimmed.chars().filter(|c| !c.is_whitespace()).count();
    if letters < 2 || letters * 2 < visible {
        return true;
    }
    if trimmed.ends_with([',', ';', '-', '(', '/', '&']) {
        return true;
    }
    if trimmed.ends_with(':') {
        return true;
    }
    if trimmed.ends_with('.') && !numbered {
        return true;
    }
    if trimmed.starts_with([
        '[', '(', '\u{2022}', '-', '*', '\u{2013}', '\u{2014}', '\u{00b7}',
    ]) {
        return true;
    }
    let lower = trimmed.to_lowercase();
    if lower.contains('@')
        || lower.contains("http")
        || lower.contains("www.")
        || lower.contains("doi")
        || lower.contains("et al")
        || lower.contains("arxiv")
        || lower.contains("....")
        || lower.contains(". . .")
        || lower.contains('\u{2026}')
    {
        return true;
    }
    if mathy(trimmed) {
        return true;
    }
    if crate::engine::figures::parse_caption(trimmed).is_some() {
        return true;
    }
    let commas = trimmed.matches(',').count();
    if commas >= 3 {
        return true;
    }
    if commas >= 1 && AFFILIATION_WORDS.iter().any(|w| lower.contains(w)) {
        return true;
    }
    // A year in a heading is a reference or a date line ("Received 3 May
    // 2021"), except as part of a longer title.
    let year = trimmed
        .split(|c: char| !c.is_ascii_digit())
        .any(|run| run.len() == 4 && (run.starts_with("19") || run.starts_with("20")));
    year && (commas >= 1 || trimmed.contains('('))
}

/// Whether a title (numbering removed) opens with a word that makes it a
/// theorem label, caption or front-matter field rather than a section.
fn not_a_section(title: &str) -> bool {
    let word = first_word_lower(title);
    if FIELD_WORDS.contains(&word.as_str()) {
        return true;
    }
    if !LABEL_WORDS.contains(&word.as_str()) {
        return false;
    }
    // What follows the word decides: a counter or punctuation makes it a
    // label, an ordinary word a title.
    let trimmed = title.trim_start();
    let after = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_alphabetic())
        .map_or("", |(i, _)| &trimmed[i..])
        .trim_start();
    let Some(next) = after.split_whitespace().next() else {
        return true;
    };
    let first = next.chars().next().unwrap_or(' ');
    if !first.is_alphabetic() {
        return true;
    }
    // "Lemma A.2", "Case B", "Theorem IV"
    let counter = next.trim_end_matches(['.', ':', ',', ')']);
    counter.chars().count() == 1
        || counter.contains('.')
        || numbering::roman_value(counter).is_some()
}

struct Detector<'a> {
    doc: &'a Doc,
    stats: &'a Stats,
}

impl Detector<'_> {
    fn style(&self, line: &Line, text: &str) -> StyleKey {
        let title = numbering::parse(text).map_or_else(|| text.to_owned(), |n| n.title);
        StyleKey {
            size: bucket(line.size),
            bold: line.bold,
            caps: normalize::is_all_caps(&title),
            smallcaps: line.smallcaps,
            font: line.font,
        }
    }

    /// How far a line's style stands out from the body; 0 when it does not.
    fn prominence(&self, line: &Line, style: &StyleKey) -> f32 {
        let body = self.stats.body;
        let larger = line.size >= body * LARGER_RATIO && line.size >= body + LARGER_POINTS;
        let smaller = line.size < body - 1.0;
        let bolder = line.bold && !self.stats.body_bold;
        let capsy = style.caps && !smaller;
        let small_caps = line.smallcaps && !smaller && !self.is_body_smallcaps();
        if smaller && !larger {
            return 0.0;
        }
        if !(larger || bolder || capsy || small_caps) {
            return 0.0;
        }
        line.size / body
            + if bolder { 0.15 } else { 0.0 }
            + if capsy { 0.1 } else { 0.0 }
            + if small_caps { 0.05 } else { 0.0 }
    }

    fn is_body_smallcaps(&self) -> bool {
        false
    }

    fn excluded(&self, p: usize, i: usize) -> bool {
        self.stats.furniture.contains(&(p, i)) || self.stats.toc_pages.contains(&p)
    }

    /// Whether text continues on the line's own row, just to its right, in
    /// its own column: then it is a label hanging off an entry (a
    /// reference key, a glossary term), not a heading standing alone.
    fn has_row_neighbour(&self, p: usize, i: usize) -> bool {
        let page = &self.doc.pages[p];
        let line = &page.lines[i];
        let mid = page.width / 2.0;
        let reach = 3.0 * self.stats.body;
        page.lines.iter().enumerate().any(|(j, other)| {
            if j == i || self.stats.furniture.contains(&(p, j)) {
                return false;
            }
            let overlap = line.bottom.min(other.bottom) - line.top.max(other.top);
            let height = (line.bottom - line.top).min(other.bottom - other.top);
            let same_row = overlap >= 0.5 * height;
            let gap = other.left - line.right;
            let same_side = line.right > mid || other.left < mid;
            same_row && gap >= -1.0 && gap < reach && same_side
        })
    }

    /// Whether the line has room above it: more than the body's line gap,
    /// or nothing above it in its column.
    fn space_above(&self, p: usize, i: usize) -> bool {
        let page = &self.doc.pages[p];
        let line = &page.lines[i];
        let mut j = above(page, i);
        // Page furniture above does not count as text.
        while let Some(k) = j {
            if !self.stats.furniture.contains(&(p, k)) {
                break;
            }
            j = above_of(page, k, line);
        }
        match j {
            None => true,
            Some(k) => {
                let gap = line.top - page.lines[k].bottom;
                // Directly under a section name ("Results", then its first
                // subsection) a heading needs no space of its own.
                let under_name = normalize::is_known_name(&page.lines[k].text)
                    && page.lines[k].bold == line.bold
                    && page.lines[k].size >= line.size - 0.5;
                under_name || gap >= self.stats.gap + EXTRA_SPACE_EMS * self.stats.body
            }
        }
    }

    /// Candidates on every page, wrapped lines joined.
    #[allow(clippy::type_complexity)]
    fn candidates(
        &self,
    ) -> (
        Vec<Cand>,
        HashMap<StyleKey, (usize, usize)>,
        Option<(u32, u8, f32)>,
    ) {
        let mut out = Vec::new();
        let mut first_body = None;
        // style -> (lines in it, lines that were candidates), for purity
        let mut purity: HashMap<StyleKey, (usize, usize)> = HashMap::new();
        for (p, page) in self.doc.pages.iter().enumerate() {
            let keys = order_keys(page);
            let mut order: Vec<usize> = (0..page.lines.len()).collect();
            order.sort_by(|a, b| {
                let (ka, kb) = (keys[*a], keys[*b]);
                ka.0.cmp(&kb.0)
                    .then(ka.1.cmp(&kb.1))
                    .then(ka.2.total_cmp(&kb.2))
            });
            if p == 0 {
                first_body = order
                    .iter()
                    .copied()
                    .find(|&i| self.paragraph_start(p, i))
                    .map(|i| keys[i]);
            }
            let mut consumed: HashSet<usize> = HashSet::new();
            for &i in &order {
                if consumed.contains(&i) || self.excluded(p, i) {
                    continue;
                }
                let line = &page.lines[i];
                let style = self.style(line, &line.text);
                let first_page = p == 0 && page.index == 0;
                let tally = !first_page && chars_of(line) >= 2;
                if tally {
                    purity.entry(style).or_default().0 += 1;
                }
                let Some((cand, joined)) = self.candidate(p, i, style, &keys) else {
                    continue;
                };
                if tally {
                    purity.entry(style).or_default().1 += 1 + joined.len();
                    // a joined line also counted itself in the total
                    purity.entry(style).or_default().0 += joined.len();
                }
                consumed.extend(joined);
                out.push(cand);
            }
        }
        out.sort_by(|a, b| {
            a.p.cmp(&b.p)
                .then(a.order.0.cmp(&b.order.0))
                .then(a.order.1.cmp(&b.order.1))
                .then(a.order.2.total_cmp(&b.order.2))
        });
        (out, purity, first_body)
    }

    /// Two body lines, one under the other: running text has begun.
    fn paragraph_start(&self, p: usize, i: usize) -> bool {
        let page = &self.doc.pages[p];
        let body = bucket(self.stats.body);
        let is_body = |k: usize| {
            let line = &page.lines[k];
            bucket(line.size) == body && chars_of(line) >= 40 && !self.excluded(p, k)
        };
        is_body(i) && below(page, i).is_some_and(is_body)
    }

    /// One line as a heading candidate, with the lines it wraps onto.
    fn candidate(
        &self,
        p: usize,
        i: usize,
        style: StyleKey,
        keys: &[(u32, u8, f32)],
    ) -> Option<(Cand, Vec<usize>)> {
        let page = &self.doc.pages[p];
        let line = &page.lines[i];
        let ocr = self.stats.ocr_like;
        let prominence = if ocr {
            1.0
        } else {
            self.prominence(line, &style)
        };
        // A line that opens bold and runs on in roman is a run-in head
        // and its paragraph, read by `run_ins`, not a heading line.
        if prominence <= 0.0 || line.math > 0.25 || line.lead.is_some() {
            return None;
        }
        if !self.space_above(p, i) || self.has_row_neighbour(p, i) {
            return None;
        }
        let first = strip_line_number(&normalize::clean(&line.text));
        if first.split_whitespace().count() > MAX_WORDS {
            return None;
        }
        // The second line of a run-in head that wraps: its continuation
        // opens the line below in bold.
        if let Some(j) = below(page, i) {
            let next = &page.lines[j];
            if next.lead.is_some() && next.top - line.bottom < 0.6 * line.size {
                return None;
            }
        }
        let mut text = first;
        let mut joined: Vec<usize> = Vec::new();
        let mut last = i;
        let lone_number = numbering::parse(&text).is_some_and(|n| n.title.is_empty());
        // A number or "Chapter 3" set on its own line takes the title that
        // follows it.
        if lone_number {
            let j = below(page, last)?;
            let next = &page.lines[j];
            let gap = next.top - page.lines[last].bottom;
            let next_style = self.style(next, &next.text);
            if self.excluded(p, j)
                || gap > 4.0 * line.size
                || (!ocr && self.prominence(next, &next_style) <= 0.0)
                || numbering::parse(&next.text).is_some()
            {
                return None;
            }
            let title = normalize::clean(&next.text);
            let keyword = numbering::parse(&text).is_some_and(|n| {
                matches!(
                    n.kind,
                    Kind::Chapter | Kind::Part | Kind::Appendix | Kind::Supp
                )
            });
            text = if keyword {
                format!("{text}: {title}")
            } else {
                format!("{text} {title}")
            };
            joined.push(j);
            last = j;
        }
        // Wrapped lines: the same style, close beneath, not numbered.
        while !ocr && joined.len() < 3 {
            let Some(j) = below(page, last) else { break };
            let next = &page.lines[j];
            let prev = &page.lines[last];
            let gap = next.top - prev.bottom;
            // A line wraps only once it has filled its measure; a short one
            // ended where its author ended it.
            let measure = column_right(page, prev) - prev.left;
            let filled = prev.right - prev.left >= 0.6 * measure;
            if self.excluded(p, j)
                || !filled
                || normalize::is_known_name(&text)
                || next.lead.is_some()
                || keys[j].1 != keys[last].1 && keys[j].1 != 0 && keys[last].1 != 0
                || gap > 0.6 * prev.size.max(next.size)
                || (next.size - prev.size).abs() > 0.6
                || next.bold != prev.bold
                || next.font != prev.font
                || next.smallcaps != prev.smallcaps
                || normalize::is_all_caps(&next.text) != normalize::is_all_caps(&prev.text)
                || numbering::parse(&next.text).is_some()
                || text.ends_with('.')
            {
                break;
            }
            let combined = normalize::join_lines(&text, &normalize::clean(&next.text));
            if combined.split_whitespace().count() > MAX_JOINED_WORDS {
                return None;
            }
            text = combined;
            joined.push(j);
            last = j;
        }
        // A run-in head the extraction could not split by weight: a full
        // line, its title ended by a period, its paragraph running on
        // ("8.1. Conservative equations. In this subsection we").
        if joined.is_empty() {
            let measure = column_right(page, line) - line.left;
            let full = line.right - line.left >= 0.9 * measure;
            if let Some(cut) = sentence_break(&text).filter(|_| full) {
                // Unnumbered, it is a run-in head for `run_ins` to judge.
                numbering::parse(&text)?;
                text.truncate(cut);
            }
        }
        let numbering = numbering::parse(&text);
        let title_part = numbering
            .as_ref()
            .map_or(text.as_str(), |n| n.title.as_str());
        if title_part.trim().is_empty() {
            return None;
        }
        // The title of a contents page or a list of figures names a list,
        // not a section of the text.
        if text_rejects(&text, numbering.is_some())
            || super::toc::is_contents_title(&text)
            || super::toc::is_list_title(&text)
        {
            return None;
        }
        if title_part.starts_with(|c: char| c.is_lowercase()) || !reads_as_words(title_part) {
            return None;
        }
        if not_a_section(title_part) {
            return None;
        }
        // A numbered line ending in a period is a list item unless it is
        // short and plainly set as a heading.
        if text.ends_with('.') && text.split_whitespace().count() > 6 {
            return None;
        }
        let known = numbering
            .as_ref()
            .map_or(normalize::is_known_name(&text), |n| {
                n.kind == Kind::Appendix && n.title.is_empty() || normalize::is_known_name(&n.title)
            });
        let words = text.split_whitespace().count();
        Some((
            Cand {
                p,
                page: page.index,
                top: line.top,
                order: keys[i],
                style,
                prominence,
                larger: line.size >= self.stats.body * LARGER_RATIO
                    && line.size >= self.stats.body + LARGER_POINTS,
                title: normalize::display(&text),
                numbering,
                known,
                words,
                depth: None,
                run_in: false,
            },
            joined,
        ))
    }

    /// Bold run-in heads ("Model. We consider..."), kept only when the
    /// document uses them consistently.
    fn run_ins(&self) -> Vec<Cand> {
        let mut found: Vec<Cand> = Vec::new();
        for (p, page) in self.doc.pages.iter().enumerate() {
            if self.stats.toc_pages.contains(&p) {
                continue;
            }
            let keys = order_keys(page);
            for (i, line) in page.lines.iter().enumerate() {
                let Some(lead) = &line.lead else { continue };
                if self.excluded(p, i) || bucket(line.size) != bucket(self.stats.body) {
                    continue;
                }
                let raw = lead.text.trim();
                // The head ends the way a run-in head does: a period, or a
                // period and a dash ("Model.---").
                let head = raw
                    .trim_end_matches(['\u{2014}', '\u{2013}', '-', ' '])
                    .trim_end();
                let Some(stem) = head.strip_suffix('.') else {
                    continue;
                };
                let stem = normalize::clean(stem);
                let words = stem.split_whitespace().count();
                if words == 0 || words > 8 || numbering::parse(&stem).is_some() {
                    continue;
                }
                if not_a_section(&stem)
                    || text_rejects(&stem, false)
                    || !stem.starts_with(|c: char| c.is_uppercase())
                {
                    continue;
                }
                // A paragraph's first line: room above, or indented.
                if !self.space_above(p, i) && !self.indented(p, i) {
                    continue;
                }
                found.push(Cand {
                    p,
                    page: page.index,
                    top: line.top,
                    order: keys[i],
                    style: StyleKey {
                        size: bucket(lead.size),
                        bold: true,
                        caps: false,
                        smallcaps: false,
                        font: line.font,
                    },
                    prominence: 0.5,
                    larger: false,
                    numbering: None,
                    known: normalize::is_known_name(&stem),
                    title: normalize::display(&stem),
                    words,
                    depth: None,
                    run_in: true,
                });
            }
        }
        // Consistent use: three or more, on two or more pages, and not a
        // feature of every paragraph.
        let pages: HashSet<usize> = found.iter().map(|c| c.p).collect();
        let consistent =
            found.len() >= 3 && pages.len() >= 2 && found.len() <= 2 * self.doc.pages.len() + 2;
        if consistent {
            found
        } else {
            Vec::new()
        }
    }

    /// Whether a line starts further right than the lines below it - the
    /// first line of an indented paragraph.
    fn indented(&self, p: usize, i: usize) -> bool {
        let page = &self.doc.pages[p];
        let line = &page.lines[i];
        below(page, i).is_some_and(|j| line.left - page.lines[j].left > 0.5 * self.stats.body)
    }
}

/// The next line above `from` that also overlaps `line`.
fn above_of(page: &Page, from: usize, line: &Line) -> Option<usize> {
    let start = &page.lines[from];
    let mut best: Option<usize> = None;
    for (j, other) in page.lines.iter().enumerate() {
        if j == from || !overlaps(line, other) || other.top >= start.top {
            continue;
        }
        if best.is_none_or(|b| other.bottom > page.lines[b].bottom) {
            best = Some(j);
        }
    }
    best
}

/// The typographic layer: candidates, then validation.
fn typographic(doc: &Doc, stats: &Stats) -> Vec<Heading> {
    let detector = Detector { doc, stats };
    let (mut cands, purity, first_body) = detector.candidates();
    let purity_of = |style: &StyleKey| -> f32 {
        purity.get(style).map_or(1.0, |(all, hit)| {
            if *all == 0 {
                1.0
            } else {
                *hit as f32 / *all as f32
            }
        })
    };

    let numbered_ok = validate_numbered(&mut cands, stats.ocr_like);
    #[cfg(test)]
    if std::env::var("OUTLINE_DEBUG").is_ok() {
        eprintln!(
            "body {} bold {} gap {}",
            stats.body, stats.body_bold, stats.gap
        );
        for (style, (all, hit)) in &purity {
            if *hit > 0 {
                eprintln!("purity {style:?} {hit}/{all}");
            }
        }
        for c in &cands {
            eprintln!(
                "cand p{} d{:?} {:?} prom{:.2} {:?} | {}",
                c.page,
                c.depth,
                c.numbering.as_ref().map(|n| (n.kind, n.values.clone())),
                c.prominence,
                c.style,
                c.title
            );
        }
    }
    // After validation: an initial on the title page ("T. Anderson") reads
    // as a numbered heading until the numbering is checked.
    drop_title_page(&mut cands, first_body);
    if stats.ocr_like {
        // Scanned text: numbering and spacing only, and more of it.
        cands.retain(|c| c.depth.is_some());
        if cands.iter().filter(|c| c.depth == Some(0)).count() < 3 {
            return Vec::new();
        }
    } else if numbered_ok {
        accept_unnumbered_in_numbered(&mut cands, &purity_of);
    } else {
        accept_unnumbered_styles(&mut cands, &purity_of, doc.pages.len());
    }
    cands.retain(|c| c.depth.is_some());

    // Run-in heads hang under the nearest heading before them - in a
    // document without numbered sections, where they are its structure
    // (PRL, Nature). Under numbered sections they are paragraph labels,
    // detail an outline is better without.
    if !stats.ocr_like && !numbered_ok {
        let run_ins = detector.run_ins();
        if !run_ins.is_empty() {
            merge_run_ins(&mut cands, run_ins);
        }
    }

    finish(doc, cands)
}

/// Nothing before the document's first numbered heading or known section
/// name is a heading: that is the title, the authors, their affiliations,
/// the journal's masthead - and, where the title page is followed by an
/// abstract page, that too. With no such anchor anywhere, the first page
/// loses what stands before its running text begins.
fn drop_title_page(cands: &mut Vec<Cand>, first_body: Option<(u32, u8, f32)>) {
    let anchor = cands.iter().position(|c| c.known || c.depth.is_some());
    if let Some(anchor) = anchor {
        cands.drain(..anchor);
        return;
    }
    let Some(p0) = cands.iter().find(|c| c.page == 0).map(|c| c.p) else {
        return;
    };
    let before = |a: (u32, u8, f32), b: (u32, u8, f32)| {
        a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)) == std::cmp::Ordering::Less
    };
    cands.retain(|c| c.p != p0 || first_body.is_some_and(|cut| !before(c.order, cut)));
}

/// Run each numbering system's longest plausible chain, set the depth of
/// every numbered heading kept, and say whether the document is numbered:
/// at least two top-level numbered headings survive.
fn validate_numbered(cands: &mut [Cand], ocr: bool) -> bool {
    let idx = |pred: &dyn Fn(&Numbering) -> bool| -> Vec<usize> {
        cands
            .iter()
            .enumerate()
            .filter(|(_, c)| c.numbering.as_ref().is_some_and(pred))
            .map(|(i, _)| i)
            .collect()
    };
    let romans = idx(&|n| n.kind == Kind::Roman);
    let aps = romans.len() >= 2;
    let parts = idx(&|n| n.kind == Kind::Part);
    let chapters = idx(&|n| n.kind == Kind::Chapter);
    let mut kept: HashMap<usize, usize> = HashMap::new(); // cand -> depth

    let chain = |members: &[usize], keys: &[Vec<u32>]| -> Vec<usize> {
        numbering::longest_chain(keys)
            .into_iter()
            .map(|k| members[k])
            .collect()
    };

    let values_of = |members: &[usize]| -> Vec<Vec<u32>> {
        members
            .iter()
            .map(|&i| {
                cands[i]
                    .numbering
                    .as_ref()
                    .map_or_else(Vec::new, |n| n.values.clone())
            })
            .collect()
    };
    let part_chain = chain(&parts, &values_of(&parts));
    let chapter_chain = chain(&chapters, &values_of(&chapters));
    let has_parts = part_chain.len() >= 2;
    let base_after_parts = usize::from(has_parts);
    let has_chapters = chapter_chain.len() >= 2;

    // Main arabic system, restarting after each chapter or part heading.
    let arabic: Vec<usize> = idx(&|n| n.kind == Kind::Arabic && !(aps && n.values.len() == 1));
    let breaks: Vec<usize> = if has_chapters || has_parts {
        let mut b: Vec<usize> = chapter_chain
            .iter()
            .chain(part_chain.iter())
            .copied()
            .collect();
        b.sort_unstable();
        b
    } else {
        // Back matter: a numbered list after the references (a venue's
        // checklist, an appendix's own numbering) is a separate run, kept
        // only if it is set like the sections themselves.
        cands
            .iter()
            .enumerate()
            .filter(|(_, c)| c.numbering.is_none() && c.known && is_back_matter(&c.title))
            .map(|(i, _)| i)
            .collect()
    };
    let mut main_style: Option<StyleKey> = None;
    let mut main_top = 0usize;
    let mut segment_start = 0usize;
    let mut opener: Option<usize> = None;
    let mut segments: Vec<(Vec<usize>, Option<usize>)> = Vec::new();
    for &b in breaks.iter().chain(std::iter::once(&usize::MAX)) {
        let seg: Vec<usize> = arabic
            .iter()
            .copied()
            .filter(|&i| i >= segment_start && i < b)
            .collect();
        segments.push((seg, opener));
        segment_start = b;
        opener = Some(b);
    }
    let min_chain = if ocr { 3 } else { 2 };
    for (seg, opener) in segments {
        let keys = values_of(&seg);
        let kept_chain = chain(&seg, &keys);
        // The chapter this run of sections sits in, if any: "3.1" under
        // "Chapter 3" carries the chapter's number; "1 Introduction" under
        // "Chapter 4" (a thesis's included paper) restarts below it.
        let chapter_number = opener
            .filter(|o| chapter_chain.contains(o))
            .and_then(|o| cands[o].numbering.as_ref())
            .map(|n| n.values[0]);
        let carries = chapter_number.is_some_and(|c| {
            kept_chain.iter().all(|&i| {
                cands[i]
                    .numbering
                    .as_ref()
                    .is_some_and(|n| n.values[0] == c)
            })
        });
        let nested = has_chapters && opener.is_some() && !carries;
        let single_ok = carries && keys.iter().any(|k| k.len() >= 2);
        if kept_chain.len() < min_chain && !single_ok {
            continue;
        }
        // A document's sections start at 1 (or 2, the first one missed). A
        // run that starts at "4" means the first three went unseen, and
        // what was seen is not to be trusted either.
        let first_top = kept_chain
            .iter()
            .filter_map(|&i| cands[i].numbering.as_ref())
            .filter(|n| n.values.len() == 1)
            .map(|n| n.values[0])
            .min();
        if opener.is_none() && first_top.is_some_and(|v| v > 2) {
            continue;
        }
        let top_style = majority_style(
            cands,
            kept_chain.iter().copied().filter(|&i| {
                cands[i]
                    .numbering
                    .as_ref()
                    .is_some_and(|n| n.values.len() == 1)
            }),
        );
        if !(has_chapters || has_parts) {
            match main_style {
                None => main_style = top_style,
                Some(style) if top_style != Some(style) => continue,
                Some(_) => {}
            }
        }
        for i in kept_chain {
            let values = &cands[i].numbering.as_ref().unwrap().values;
            if values.len() == 1 && !nested {
                main_top += 1;
            }
            // In a book "3 Lattices" sits level with "Chapter 3", and
            // "3.1" one below either.
            let depth = base_after_parts + values.len() - 1 + usize::from(nested);
            kept.insert(i, depth);
        }
    }
    if has_chapters {
        for &i in &chapter_chain {
            kept.insert(i, base_after_parts);
        }
    }
    if has_parts {
        for &i in &part_chain {
            kept.insert(i, 0);
        }
    }

    // APS and IEEE: "I." sections, "A." subsections, "1." or "1)" below.
    let mut aps_top = 0usize;
    if aps {
        let members: Vec<usize> = idx(&|n| {
            matches!(n.kind, Kind::Roman | Kind::ArabicParen)
                || (n.kind == Kind::Letter && n.values.len() == 1 && !n.bare)
                || (n.kind == Kind::Arabic && n.values.len() == 1)
        });
        let mut keys = Vec::new();
        let (mut r, mut l) = (0u32, 0u32);
        for &i in &members {
            let n = cands[i].numbering.as_ref().unwrap();
            let v = n.values[0];
            let key = match n.kind {
                Kind::Roman => {
                    r = v;
                    l = 0;
                    vec![v]
                }
                Kind::Letter => {
                    l = v;
                    vec![r, v]
                }
                _ => vec![r, l, v],
            };
            keys.push(key);
        }
        let kept_chain = chain(&members, &keys);
        if kept_chain.len() >= min_chain {
            for i in kept_chain {
                let n = cands[i].numbering.as_ref().unwrap();
                let depth = match n.kind {
                    Kind::Roman => 0,
                    Kind::Letter => 1,
                    _ => 2,
                };
                if depth == 0 {
                    aps_top += 1;
                }
                kept.insert(i, base_after_parts + depth);
            }
        }
    }

    let numbered_doc = main_top >= 2 || aps_top >= 2 || has_chapters || has_parts;

    // Appendices: "Appendix A", "A.1", and - where the sections vouch for
    // it - a bare "A Proofs" set in the sections' own style.
    let top_style = majority_style(
        cands,
        kept.iter()
            .filter(|(_, d)| **d == base_after_parts)
            .map(|(i, _)| *i),
    );
    let first_main = kept.keys().copied().min();
    let appendix: Vec<usize> = cands
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            let Some(n) = &c.numbering else { return false };
            match n.kind {
                Kind::Appendix => !n.values.is_empty(),
                Kind::Letter if n.bare => {
                    numbered_doc && Some(c.style) == top_style && first_main.is_some_and(|m| *i > m)
                }
                Kind::Letter => !(aps && n.values.len() == 1),
                _ => false,
            }
        })
        .map(|(i, _)| i)
        .collect();
    let keys: Vec<Vec<u32>> = appendix
        .iter()
        .map(|&i| cands[i].numbering.as_ref().unwrap().values.clone())
        .collect();
    let mut appendix_chain = chain(&appendix, &keys);
    // Appendices start at A (or B, the first missed): a lone "T." is
    // somebody's initial.
    if appendix_chain
        .first()
        .and_then(|&i| cands[i].numbering.as_ref())
        .is_some_and(|n| n.values.first().is_none_or(|v| *v > 2))
    {
        appendix_chain.clear();
    }
    if appendix_chain.len() >= 2 || (numbered_doc && !appendix_chain.is_empty()) {
        for i in appendix_chain {
            let n = cands[i].numbering.as_ref().unwrap();
            let depth = base_after_parts
                + if n.kind == Kind::Appendix {
                    0
                } else {
                    n.values.len() - 1
                };
            kept.insert(i, depth);
        }
    }

    // Supplementary sections.
    let supp = idx(&|n| n.kind == Kind::Supp);
    let keys: Vec<Vec<u32>> = supp
        .iter()
        .map(|&i| cands[i].numbering.as_ref().unwrap().values.clone())
        .collect();
    let supp_chain = chain(&supp, &keys);
    let mut supp_top = 0;
    if supp_chain.len() >= 2 || (numbered_doc && !supp_chain.is_empty()) {
        for i in supp_chain {
            let len = cands[i].numbering.as_ref().unwrap().values.len();
            if len == 1 {
                supp_top += 1;
            }
            kept.insert(i, base_after_parts + len - 1);
        }
    }

    for (i, depth) in &kept {
        cands[*i].depth = Some(*depth);
    }
    drop_off_style(cands);
    numbered_doc || supp_top >= 2
}

/// The style most of `members` share.
fn majority_style(cands: &[Cand], members: impl Iterator<Item = usize>) -> Option<StyleKey> {
    let mut count: HashMap<StyleKey, usize> = HashMap::new();
    for i in members {
        *count.entry(cands[i].style).or_default() += 1;
    }
    count
        .into_iter()
        .max_by_key(|(s, n)| (*n, s.size, s.bold))
        .map(|(s, _)| s)
}

/// Within each depth, a numbered heading set unlike the rest of its depth
/// is something else that happened to be numbered in sequence.
fn drop_off_style(cands: &mut [Cand]) {
    let mut by_depth: HashMap<(usize, Kind), Vec<usize>> = HashMap::new();
    for (i, c) in cands.iter().enumerate() {
        if let (Some(d), Some(n)) = (c.depth, &c.numbering) {
            by_depth.entry((d, n.kind)).or_default().push(i);
        }
    }
    for members in by_depth.values() {
        if members.len() < 3 {
            continue;
        }
        let Some(major) = majority_style(cands, members.iter().copied()) else {
            continue;
        };
        let alike = |s: &StyleKey| (s.size - major.size).abs() <= 1 && s.bold == major.bold;
        let off: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&i| !alike(&cands[i].style))
            .collect();
        if off.len() * 3 <= members.len() {
            for i in off {
                cands[i].depth = None;
            }
            continue;
        }
        // Two styles in earnest: the run changed lists part way (sections,
        // then a venue checklist numbered on from them). The document's
        // sections are the ones it starts with.
        let first = cands[members[0]].style;
        let like_first = |s: &StyleKey| (s.size - first.size).abs() <= 1 && s.bold == first.bold;
        let others = members
            .iter()
            .filter(|&&i| !like_first(&cands[i].style))
            .count();
        // Only a list that follows them all; styles that alternate are one
        // document's inconsistency, not two lists.
        let last_first = members
            .iter()
            .copied()
            .filter(|&i| like_first(&cands[i].style))
            .max()
            .unwrap_or(0);
        let tail = members
            .iter()
            .all(|&i| like_first(&cands[i].style) || i > last_first);
        if tail && others >= 2 && members.len() - others >= 2 {
            for &i in members {
                if !like_first(&cands[i].style) {
                    cands[i].depth = None;
                }
            }
        }
    }
}

/// In a numbered document, an unnumbered heading is accepted when it is a
/// known section name, or set exactly like the numbered headings of some
/// depth in a style used for little else.
fn accept_unnumbered_in_numbered(cands: &mut [Cand], purity_of: &dyn Fn(&StyleKey) -> f32) {
    let mut depth_styles: HashMap<StyleKey, HashMap<usize, usize>> = HashMap::new();
    for c in cands.iter() {
        if let (Some(d), Some(_)) = (c.depth, &c.numbering) {
            *depth_styles
                .entry(c.style)
                .or_default()
                .entry(d)
                .or_default() += 1;
        }
    }
    let depth_of_style = |style: &StyleKey| -> Option<usize> {
        depth_styles
            .get(style)
            .and_then(|m| m.iter().max_by_key(|(d, n)| (**n, std::cmp::Reverse(**d))))
            .map(|(d, _)| *d)
    };
    for c in cands.iter_mut() {
        if c.depth.is_some() {
            continue;
        }
        let unnumbered = c.numbering.is_none()
            || c.numbering
                .as_ref()
                .is_some_and(|n| n.kind == Kind::Appendix && n.values.is_empty());
        if !unnumbered {
            continue;
        }
        if c.known {
            let top_name = normalize::is_top_level_name(&c.title);
            c.depth = Some(if top_name {
                0
            } else {
                depth_of_style(&c.style).unwrap_or(0)
            });
            continue;
        }
        if let Some(d) = depth_of_style(&c.style) {
            if c.words <= 10 && purity_of(&c.style) >= MIN_PURITY && d <= 1 {
                c.depth = Some(d);
            }
        }
    }
}

/// With no numbering to go on, a style is a heading style when it recurs,
/// is used for little else, and either is larger than the body or is used
/// for at least one known section name.
fn accept_unnumbered_styles(
    cands: &mut [Cand],
    purity_of: &dyn Fn(&StyleKey) -> f32,
    pages: usize,
) {
    let mut members: HashMap<StyleKey, Vec<usize>> = HashMap::new();
    for (i, c) in cands.iter().enumerate() {
        if c.depth.is_none() && (c.numbering.is_none() || c.known) {
            members.entry(c.style).or_default().push(i);
        }
    }
    let mut accepted: Vec<(StyleKey, f32)> = Vec::new();
    for (style, list) in &members {
        let known = list.iter().filter(|&&i| cands[i].known).count();
        let larger = list.iter().all(|&i| cands[i].larger);
        let count = list.len();
        let words: f32 =
            list.iter().map(|&i| cands[i].words as f32).sum::<f32>() / count.max(1) as f32;
        let pages_hit: HashSet<usize> = list.iter().map(|&i| cands[i].p).collect();
        // Section names the style is used for vouch for it: with two of
        // them, it may share its look with more of the page.
        let purity_needed = if known >= 2 { 0.35 } else { MIN_PURITY };
        let ok = count >= 2
            && purity_of(style) >= purity_needed
            && count <= 3 * pages + 5
            && words <= 8.0
            && (known >= 1 || (larger && count >= 3 && pages_hit.len() >= 2));
        if ok {
            let prominence = list
                .iter()
                .map(|&i| cands[i].prominence)
                .fold(0.0, f32::max);
            accepted.push((*style, prominence));
        }
    }
    accepted.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (rank, (style, _)) in accepted.iter().enumerate().take(MAX_LEVELS as usize) {
        for &i in &members[style] {
            cands[i].depth = Some(rank);
        }
    }
    // A known name in a style of its own still counts when the document
    // has headings of its own to sit beside.
    if !accepted.is_empty() {
        for c in cands.iter_mut() {
            if c.depth.is_none() && c.known && c.numbering.is_none() {
                c.depth = Some(0);
            }
        }
    }
}

/// Put run-in heads into the outline, one level under the heading before
/// them - or at the top where none is.
fn merge_run_ins(cands: &mut Vec<Cand>, run_ins: Vec<Cand>) {
    let key = |c: &Cand| (c.p, c.order.0, c.order.1, c.order.2);
    let mut all: Vec<Cand> = cands.drain(..).chain(run_ins).collect();
    all.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0)
            .then(ka.1.cmp(&kb.1))
            .then(ka.2.cmp(&kb.2))
            .then(ka.3.total_cmp(&kb.3))
    });
    let mut parent: Option<usize> = None;
    for c in all.iter_mut() {
        if c.run_in {
            let depth = parent.map_or(0, |d| d + 1);
            c.depth = (depth < MAX_LEVELS as usize).then_some(depth);
        } else if let Some(d) = c.depth {
            parent = Some(d);
        }
    }
    *cands = all;
}

/// Order, compress the levels used, apply the final gates.
fn finish(doc: &Doc, mut cands: Vec<Cand>) -> Vec<Heading> {
    cands.retain(|c| c.depth.is_some());
    cands.sort_by(|a, b| {
        a.p.cmp(&b.p)
            .then(a.order.0.cmp(&b.order.0))
            .then(a.order.1.cmp(&b.order.1))
            .then(a.order.2.total_cmp(&b.order.2))
    });
    // The same heading twice at one spot (a line read twice) is one.
    cands.dedup_by(|b, a| a.p == b.p && (a.top - b.top).abs() < 1.0 && a.title == b.title);
    // A back-matter name over and over ("References" closing every chapter
    // of an edited volume) is clutter at the top of an outline whose
    // chapters were not found to hold it.
    let mut repeats: HashMap<String, usize> = HashMap::new();
    for c in cands.iter().filter(|c| c.known && c.numbering.is_none()) {
        *repeats.entry(normalize::letters(&c.title)).or_default() += 1;
    }
    cands.retain(|c| {
        !(c.known
            && c.numbering.is_none()
            && is_back_matter(&c.title)
            && repeats
                .get(&normalize::letters(&c.title))
                .is_some_and(|n| *n >= 3))
    });
    let mut used: Vec<usize> = cands.iter().filter_map(|c| c.depth).collect();
    used.sort_unstable();
    used.dedup();
    let headings: Vec<Heading> = cands
        .iter()
        .filter_map(|c| {
            let rank = used.iter().position(|d| Some(*d) == c.depth)? as u8;
            if rank >= MAX_LEVELS {
                return None;
            }
            let page = &doc.pages[c.p];
            Some(Heading {
                level: rank,
                title: c.title.clone(),
                page: c.page,
                y: Some(anchor_y(page.height, c.top)),
            })
        })
        .collect();
    let top = headings.iter().filter(|h| h.level == 0).count();
    if top < 2 || headings.len() > 4 * doc.pages.len() + 10 {
        return Vec::new();
    }
    let pages: HashSet<u32> = headings.iter().map(|h| h.page).collect();
    if doc.pages.len() >= 4 && pages.len() < 2 {
        return Vec::new();
    }
    headings
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A builder for hand-made pages: US Letter, one column unless the
    /// lines say otherwise.
    pub struct PageBuilder {
        pub page: Page,
        y: f32,
    }

    pub const BODY: &str =
        "We study the problem in a setting where the observations arrive one at a time and";

    impl PageBuilder {
        pub fn new(index: u32) -> Self {
            PageBuilder {
                page: Page {
                    index,
                    width: 612.0,
                    height: 792.0,
                    chars: 3000,
                    ..Page::default()
                },
                y: 72.0,
            }
        }

        pub fn at(mut self, y: f32) -> Self {
            self.y = y;
            self
        }

        pub fn line(
            mut self,
            text: &str,
            size: f32,
            bold: bool,
            left: f32,
            right: f32,
            gap: f32,
        ) -> Self {
            self.y += gap;
            self.page.lines.push(Line {
                text: text.to_owned(),
                size,
                bold,
                font: u32::from(bold),
                top: self.y,
                bottom: self.y + size * 0.7,
                left,
                right,
                ..Line::default()
            });
            self.y += size * 0.7;
            self
        }

        /// `n` lines of body text, 10pt with a 2.5pt gap.
        pub fn body(mut self, n: usize) -> Self {
            for _ in 0..n {
                self = self.line(BODY, 10.0, false, 72.0, 540.0, 5.0);
            }
            self
        }

        pub fn heading(self, text: &str, size: f32, bold: bool) -> Self {
            self.line(text, size, bold, 72.0, 72.0 + 7.0 * text.len() as f32, 16.0)
        }

        pub fn with_font(mut self, font: u32) -> Self {
            if let Some(l) = self.page.lines.last_mut() {
                l.font = font;
            }
            self
        }

        pub fn lead(mut self, lead: &str) -> Self {
            if let Some(l) = self.page.lines.last_mut() {
                l.lead = Some(Lead {
                    text: lead.to_owned(),
                    size: l.size,
                });
            }
            self
        }

        pub fn math(mut self) -> Self {
            if let Some(l) = self.page.lines.last_mut() {
                l.math = 0.8;
            }
            self
        }

        pub fn running(self, text: &str) -> Self {
            let y = self.y;
            let mut s = self.at(36.0).line(text, 9.0, false, 200.0, 400.0, 0.0);
            s.y = y;
            s
        }

        pub fn build(self) -> Page {
            self.page
        }
    }

    pub fn doc(pages: Vec<Page>) -> Doc {
        Doc {
            page_count: pages.len() as u32,
            pages,
            fonts: vec![FontInfo::default(); 4],
            tagged: false,
        }
    }

    fn titles(headings: &[Heading]) -> Vec<(u8, &str)> {
        headings
            .iter()
            .map(|h| (h.level, h.title.as_str()))
            .collect()
    }

    /// A numbered article: title page, sections, subsections.
    fn article() -> Vec<Page> {
        vec![
            PageBuilder::new(0)
                .heading("A Study of Things", 17.0, true)
                .heading("Alice Author and Bob Writer", 12.0, false)
                .heading("Abstract", 10.0, true)
                .body(8)
                .heading("1 Introduction", 12.0, true)
                .body(20)
                .build(),
            PageBuilder::new(1)
                .body(10)
                .heading("2 Methods", 12.0, true)
                .body(5)
                .heading("2.1 Data", 10.0, true)
                .body(10)
                .heading("2.2 Model", 10.0, true)
                .body(8)
                .build(),
            PageBuilder::new(2)
                .body(10)
                .heading("3 Results", 12.0, true)
                .body(12)
                .heading("References", 12.0, true)
                .body(6)
                .build(),
        ]
    }

    #[test]
    fn numbered_article_gives_sections_and_subsections() {
        let out = generate(&doc(article()));
        assert_eq!(
            titles(&out),
            vec![
                (0, "Abstract"),
                (0, "1 Introduction"),
                (0, "2 Methods"),
                (1, "2.1 Data"),
                (1, "2.2 Model"),
                (0, "3 Results"),
                (0, "References"),
            ]
        );
        // Entries land at the heading, not the page top.
        let intro = &out[1];
        assert_eq!(intro.page, 0);
        assert!(intro.y.unwrap() < 700.0);
    }

    #[test]
    fn title_page_items_are_not_headings() {
        let out = generate(&doc(article()));
        assert!(out.iter().all(|h| h.title != "A Study of Things"));
        assert!(out.iter().all(|h| !h.title.contains("Alice")));
    }

    #[test]
    fn too_few_headings_gives_nothing() {
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("1 Introduction", 12.0, true)
                .body(10)
                .build(),
            PageBuilder::new(1).body(40).build(),
        ];
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn running_headers_and_page_numbers_are_excluded() {
        let mut pages = article();
        for (i, page) in pages.iter_mut().enumerate() {
            let mut b = PageBuilder {
                page: std::mem::take(page),
                y: 0.0,
            };
            b = b.running("JOURNAL OF THINGS");
            b = b
                .at(760.0)
                .line(&format!("{}", i + 1), 10.0, true, 300.0, 310.0, 0.0);
            *page = b.build();
        }
        // a bold running head at the same spot on every page
        for page in pages.iter_mut() {
            page.lines.push(Line {
                text: "Smith et al".into(),
                size: 10.0,
                bold: true,
                font: 1,
                top: 50.0,
                bottom: 57.0,
                left: 72.0,
                right: 130.0,
                ..Line::default()
            });
        }
        let out = generate(&doc(pages));
        assert!(out.iter().all(|h| !h.title.contains("JOURNAL")));
        assert!(out.iter().all(|h| !h.title.contains("Smith")));
        assert!(out.iter().all(|h| h.title.parse::<u32>().is_err()));
        assert_eq!(out.len(), 7);
    }

    #[test]
    fn captions_theorems_and_equations_are_excluded() {
        let mut pages = article();
        let extra = PageBuilder::new(3)
            .body(5)
            .heading("Figure 3: Accuracy against noise", 10.0, true)
            .body(5)
            .heading("Table 2. Results", 10.0, true)
            .body(5)
            .heading("Theorem 2.1 (Main result)", 10.0, true)
            .body(5)
            .heading("Proof.", 10.0, true)
            .body(5)
            .heading("x + y = z", 12.0, false)
            .math()
            .body(5)
            .heading("Lemma 3", 10.0, true)
            .body(5)
            .build();
        pages.push(extra);
        let out = generate(&doc(pages));
        for bad in ["Figure", "Table", "Theorem", "Proof", "x + y", "Lemma"] {
            assert!(out.iter().all(|h| !h.title.starts_with(bad)), "{bad}");
        }
    }

    #[test]
    fn numbered_list_items_in_body_text_are_not_headings() {
        let mut pages = article();
        pages[2] = PageBuilder::new(2)
            .body(4)
            .line(
                "1. We first collect the samples from the field.",
                10.0,
                false,
                72.0,
                400.0,
                5.0,
            )
            .line(
                "2. Then we dry them in the oven for a day.",
                10.0,
                false,
                72.0,
                400.0,
                5.0,
            )
            .body(4)
            .heading("3 Results", 12.0, true)
            .body(4)
            .heading("2. Second step only.", 10.0, true)
            .body(4)
            .build();
        let out = generate(&doc(pages));
        assert!(out.iter().all(|h| !h.title.contains("collect")));
        assert!(out.iter().all(|h| !h.title.contains("Second step")));
        assert!(out.iter().any(|h| h.title == "3 Results"));
    }

    #[test]
    fn bibliography_entries_are_not_headings() {
        let mut pages = article();
        pages.push(
            PageBuilder::new(3)
                .heading("[12] A. Author, Things, J. Things 4 (2019)", 10.0, true)
                .body(3)
                .heading("1. Smith J. On things. Nature (2020)", 10.0, true)
                .body(3)
                .build(),
        );
        let out = generate(&doc(pages));
        assert!(out.iter().all(|h| !h.title.contains("Author")));
        assert!(out.iter().all(|h| !h.title.contains("Smith")));
    }

    #[test]
    fn aps_style_roman_sections_with_letter_subsections() {
        let pages = vec![
            PageBuilder::new(0)
                .heading("Quantum Things in Wires", 14.0, true)
                .body(10)
                .heading("I. INTRODUCTION", 10.0, true)
                .body(20)
                .build(),
            PageBuilder::new(1)
                .body(5)
                .heading("II. MODEL", 10.0, true)
                .body(5)
                .heading("A. Hamiltonian", 10.0, true)
                .body(5)
                .heading("B. Methods of solution", 10.0, true)
                .body(5)
                .heading("III. RESULTS", 10.0, true)
                .body(10)
                .build(),
        ];
        let out = generate(&doc(pages));
        assert_eq!(
            titles(&out),
            vec![
                (0, "I. Introduction"),
                (0, "II. Model"),
                (1, "A. Hamiltonian"),
                (1, "B. Methods of solution"),
                (0, "III. Results"),
            ]
        );
    }

    #[test]
    fn unnumbered_nature_style_needs_a_known_name_or_larger_type() {
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("Results", 10.0, true)
                .body(10)
                .build(),
            PageBuilder::new(1)
                .body(10)
                .heading("Spin dynamics in the bulk", 10.0, true)
                .body(10)
                .heading("Discussion", 10.0, true)
                .body(10)
                .heading("Methods", 10.0, true)
                .body(10)
                .build(),
        ];
        let out = generate(&doc(pages));
        assert_eq!(
            titles(&out),
            vec![
                (0, "Results"),
                (0, "Spin dynamics in the bulk"),
                (0, "Discussion"),
                (0, "Methods"),
            ]
        );
        // The same bold style without a single known name is not enough.
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("Spin waves", 10.0, true)
                .body(10)
                .build(),
            PageBuilder::new(1)
                .body(10)
                .heading("Spin dynamics in the bulk", 10.0, true)
                .body(10)
                .heading("Edge states", 10.0, true)
                .body(10)
                .build(),
        ];
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn non_english_headings_by_typography_alone() {
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("Einleitung", 14.0, true)
                .body(10)
                .build(),
            PageBuilder::new(1)
                .body(10)
                .heading("Methoden und Daten", 14.0, true)
                .body(10)
                .heading("Ergebnisse", 14.0, true)
                .body(10)
                .build(),
        ];
        let out = generate(&doc(pages));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn a_bold_style_used_for_ordinary_lines_is_not_a_heading_style() {
        // Bold body-size lines everywhere: table headers, labels.
        let mut pages = Vec::new();
        for i in 0..3 {
            let mut b = PageBuilder::new(i).body(5);
            for k in 0..6 {
                b = b
                    .line(&format!("Label {k} value"), 10.0, true, 72.0, 200.0, 5.0)
                    .body(1);
            }
            b = b.heading("Discussion", 10.0, true).body(5);
            pages.push(b.build());
        }
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn wrapped_headings_are_joined() {
        let mut pages = article();
        pages[2] = PageBuilder::new(2)
            .body(10)
            .line(
                "3 Results for the non-equilibrium dyna-",
                12.0,
                true,
                72.0,
                500.0,
                16.0,
            )
            .line("mics of the model", 12.0, true, 72.0, 200.0, 3.0)
            .body(10)
            .build();
        let out = generate(&doc(pages));
        assert!(out
            .iter()
            .any(|h| h.title == "3 Results for the non-equilibrium dynamics of the model"));
        assert!(out.iter().all(|h| h.title != "mics of the model"));
    }

    #[test]
    fn two_column_reading_order_left_then_right() {
        // Right column heading higher on the page than the left one.
        let page1 = PageBuilder::new(1)
            .at(72.0)
            .line(BODY, 10.0, false, 300.0, 540.0, 0.0)
            .heading_at("3 Results", 300.0, 150.0)
            .heading_at("2 Methods", 72.0, 400.0)
            .two_column_body()
            .build();
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("1 Introduction", 12.0, true)
                .body(5)
                .build(),
            page1,
        ];
        let out = generate(&doc(pages));
        let order: Vec<&str> = out.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(order, vec!["1 Introduction", "2 Methods", "3 Results"]);
    }

    impl PageBuilder {
        fn heading_at(mut self, text: &str, left: f32, top: f32) -> Self {
            self.page.lines.push(Line {
                text: text.to_owned(),
                size: 12.0,
                bold: true,
                font: 1,
                top,
                bottom: top + 8.4,
                left,
                right: left + 80.0,
                ..Line::default()
            });
            self
        }

        fn two_column_body(mut self) -> Self {
            for col in [72.0f32, 320.0] {
                let mut y: f32 = 200.0;
                for _ in 0..20 {
                    if (y - 150.0).abs() < 30.0 || (y - 400.0).abs() < 30.0 {
                        y += 12.0;
                        continue;
                    }
                    self.page.lines.push(Line {
                        text: BODY[..45].to_owned(),
                        size: 10.0,
                        top: y,
                        bottom: y + 7.0,
                        left: col,
                        right: col + 220.0,
                        ..Line::default()
                    });
                    y += 12.0;
                }
            }
            self
        }
    }

    #[test]
    fn books_with_chapters_and_sections() {
        let mut pages = vec![PageBuilder::new(0)
            .heading("A Book", 24.0, true)
            .body(5)
            .build()];
        for ch in 1..=3u32 {
            pages.push(
                PageBuilder::new(ch)
                    .at(150.0)
                    .heading(&format!("Chapter {ch}"), 20.0, true)
                    .line(&format!("Topic number {ch}"), 24.0, true, 72.0, 300.0, 20.0)
                    .body(10)
                    .heading(&format!("{ch}.1 First part"), 12.0, true)
                    .body(10)
                    .heading(&format!("{ch}.2 Second part"), 12.0, true)
                    .body(10)
                    .build(),
            );
        }
        let out = generate(&doc(pages));
        assert_eq!(out.iter().filter(|h| h.level == 0).count(), 3);
        assert_eq!(out[0].title, "Chapter 1: Topic number 1");
        assert_eq!(out[1].title, "1.1 First part");
        assert_eq!(out[1].level, 1);
    }

    #[test]
    fn concatenated_papers_restart_numbering_after_chapters() {
        let mut pages = Vec::new();
        for ch in 1..=2u32 {
            pages.push(
                PageBuilder::new(ch * 2 - 2)
                    .at(150.0)
                    .heading(&format!("Chapter {ch}: Paper {ch}"), 20.0, true)
                    .body(10)
                    .heading("1 Introduction", 12.0, true)
                    .body(10)
                    .build(),
            );
            pages.push(
                PageBuilder::new(ch * 2 - 1)
                    .body(10)
                    .heading("2 Theory", 12.0, true)
                    .body(10)
                    .heading("3 Outlook", 12.0, true)
                    .body(10)
                    .build(),
            );
        }
        let out = generate(&doc(pages));
        assert_eq!(out.iter().filter(|h| h.level == 0).count(), 2, "{out:?}");
        assert_eq!(
            out.iter().filter(|h| h.title == "1 Introduction").count(),
            2
        );
    }

    #[test]
    fn run_in_headings_when_consistent() {
        let mut pages = Vec::new();
        for (i, heads) in [["Model.", "Setup."], ["Results.", "Outlook."]]
            .iter()
            .enumerate()
        {
            let mut b = PageBuilder::new(i as u32).body(10);
            for head in heads {
                b = b
                    .line(&format!("{head} {BODY}"), 10.0, false, 72.0, 540.0, 12.0)
                    .lead(head)
                    .body(8);
            }
            pages.push(b.build());
        }
        let out = generate(&doc(pages));
        assert_eq!(
            out.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
            vec!["Model", "Setup", "Results", "Outlook"]
        );
        // Two of them are not consistent use.
        let pages = vec![
            PageBuilder::new(0)
                .body(10)
                .line(&format!("Model. {BODY}"), 10.0, false, 72.0, 540.0, 12.0)
                .lead("Model.")
                .body(8)
                .line(&format!("Setup. {BODY}"), 10.0, false, 72.0, 540.0, 12.0)
                .lead("Setup.")
                .build(),
            PageBuilder::new(1).body(30).build(),
        ];
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn theorem_run_ins_are_not_headings() {
        let mut pages = Vec::new();
        for i in 0..3u32 {
            pages.push(
                PageBuilder::new(i)
                    .body(5)
                    .line(
                        &format!("Theorem {i}. {BODY}"),
                        10.0,
                        false,
                        72.0,
                        540.0,
                        12.0,
                    )
                    .lead(&format!("Theorem {i}."))
                    .body(5)
                    .line(&format!("Proof. {BODY}"), 10.0, false, 72.0, 540.0, 12.0)
                    .lead("Proof.")
                    .body(5)
                    .build(),
            );
        }
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn slides_are_skipped() {
        let pages: Vec<Page> = (0..10)
            .map(|i| {
                let mut p = PageBuilder::new(i)
                    .heading("Results", 28.0, true)
                    .line("A point", 20.0, false, 72.0, 300.0, 20.0)
                    .build();
                p.width = 960.0;
                p.height = 540.0;
                p.chars = 200;
                p
            })
            .collect();
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn scans_without_text_are_skipped() {
        let pages: Vec<Page> = (0..4)
            .map(|i| {
                let mut p = PageBuilder::new(i).build();
                p.chars = 0;
                p
            })
            .collect();
        assert!(generate(&doc(pages)).is_empty());
    }

    #[test]
    fn ocr_text_layers_need_numbering_and_more_of_it() {
        let make = |n: u32| -> Vec<Page> {
            (0..3)
                .map(|i| {
                    let mut b = PageBuilder::new(i).body(10);
                    if i < n {
                        b = b.heading(&format!("{} Section", i + 1), 10.0, false);
                    }
                    b.body(10).build()
                })
                .collect()
        };
        let mut d = doc(make(3));
        d.fonts = vec![FontInfo { glyphless: true }; 4];
        assert_eq!(generate(&d).len(), 3);
        let mut d = doc(make(2));
        d.fonts = vec![FontInfo { glyphless: true }; 4];
        assert!(generate(&d).is_empty());
    }

    #[test]
    fn tagged_structure_wins() {
        let mut pages = article();
        pages[0].tagged = vec![TaggedHeading {
            level: 1,
            text: "Intro".into(),
            top: 100.0,
        }];
        pages[1].tagged = vec![
            TaggedHeading {
                level: 1,
                text: "Body".into(),
                top: 100.0,
            },
            TaggedHeading {
                level: 2,
                text: "Detail".into(),
                top: 300.0,
            },
        ];
        let mut d = doc(pages);
        d.tagged = true;
        let out = generate(&d);
        assert_eq!(titles(&out), vec![(0, "Intro"), (0, "Body"), (1, "Detail")]);
    }

    #[test]
    fn appendix_letters_follow_numbered_sections() {
        let mut pages = article();
        pages.push(
            PageBuilder::new(3)
                .body(5)
                .heading("A Proofs of the main results", 12.0, true)
                .body(5)
                .heading("A.1 Proof of the bound", 10.0, true)
                .body(5)
                .build(),
        );
        let out = generate(&doc(pages));
        assert!(out
            .iter()
            .any(|h| h.title == "A Proofs of the main results" && h.level == 0));
        assert!(out
            .iter()
            .any(|h| h.title == "A.1 Proof of the bound" && h.level == 1));
    }

    #[test]
    fn affiliations_emails_and_dates_are_not_headings() {
        let mut pages = article();
        pages[0] = PageBuilder::new(0)
            .heading("A Study of Things", 17.0, true)
            .heading("Department of Physics, University of Somewhere", 10.0, true)
            .heading("alice@example.org", 10.0, true)
            .heading("Received 3 May 2021, accepted 9 June 2021", 10.0, true)
            .heading("Keywords: things", 10.0, true)
            .body(8)
            .heading("1 Introduction", 12.0, true)
            .body(20)
            .build();
        let out = generate(&doc(pages));
        assert_eq!(out[0].title, "1 Introduction");
    }

    #[test]
    fn review_line_numbers_are_dropped_from_headings() {
        let mut pages = article();
        pages[1] = PageBuilder::new(1)
            .body(10)
            .heading("123 2 Methods", 12.0, true)
            .body(5)
            .heading("140 2.1 Data", 10.0, true)
            .body(10)
            .heading("2.2 Model", 10.0, true)
            .body(8)
            .build();
        let out = generate(&doc(pages));
        assert!(out.iter().any(|h| h.title == "2 Methods"), "{out:?}");
        assert!(out.iter().any(|h| h.title == "2.1 Data"));
    }

    #[test]
    fn a_numbered_run_in_head_keeps_only_its_title() {
        let mut pages = article();
        pages[2] = PageBuilder::new(2)
            .body(10)
            .line(
                "3 Results. In this section we show the results we obtained",
                12.0,
                true,
                72.0,
                540.0,
                16.0,
            )
            .body(12)
            .build();
        let out = generate(&doc(pages));
        assert!(out.iter().any(|h| h.title == "3 Results"), "{out:?}");
    }

    #[test]
    fn garbled_and_sentence_like_lines_are_not_headings() {
        assert!(!reads_as_words("2x lhil"));
        assert!(!reads_as_words("PL f3lH.I P"));
        assert!(!reads_as_words("Cwi"));
        assert!(!reads_as_words(
            "Finally, we consider the offset complexity"
        ));
        assert!(reads_as_words("Streams, records, and uniformity"));
        assert!(reads_as_words("QUBO/Ising Hamiltonian"));
        assert!(reads_as_words("Phase 2 results"));
        assert!(reads_as_words("Methods"));
    }

    #[test]
    fn a_checklist_after_the_references_is_not_the_sections() {
        let mut pages = article();
        let mut b = PageBuilder::new(3).body(5);
        for (i, item) in ["Claims", "Limitations", "Theory", "Experiments", "Code"]
            .iter()
            .enumerate()
        {
            b = b.heading(&format!("{}. {item}", i + 1), 10.0, true).body(3);
        }
        pages.push(b.build());
        let out = generate(&doc(pages));
        assert!(out.iter().all(|h| !h.title.contains("Claims")), "{out:?}");
        assert!(out.iter().any(|h| h.title == "3 Results"));
    }

    #[test]
    fn a_subsection_directly_under_a_section_name_is_kept() {
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("Introduction", 10.0, true)
                .body(10)
                .build(),
            PageBuilder::new(1)
                .body(10)
                .heading("Results", 10.0, true)
                .line(
                    "Spins align at low temperature",
                    10.0,
                    true,
                    72.0,
                    300.0,
                    3.0,
                )
                .body(10)
                .heading("Discussion", 10.0, true)
                .body(10)
                .build(),
        ];
        let out = generate(&doc(pages));
        assert!(
            out.iter()
                .any(|h| h.title == "Spins align at low temperature"),
            "{out:?}"
        );
    }

    #[test]
    fn false_bold_weights_fall_back_to_font_names() {
        // Every line reports a bold weight; only the heading font's name
        // says bold.
        let mut pages = article();
        for page in pages.iter_mut() {
            for line in page.lines.iter_mut() {
                line.bold_named = line.bold;
                line.bold = true;
            }
        }
        let out = generate(&doc(pages));
        assert!(out.iter().any(|h| h.title == "2.1 Data"), "{out:?}");
        assert!(out.iter().all(|h| !h.title.starts_with("We study")));
    }

    #[test]
    fn a_contents_title_is_not_an_entry() {
        let mut pages = article();
        pages.insert(
            1,
            PageBuilder::new(1)
                .heading("Table of Contents", 12.0, true)
                .body(20)
                .build(),
        );
        for (i, page) in pages.iter_mut().enumerate() {
            page.index = i as u32;
        }
        let out = generate(&doc(pages));
        assert!(
            out.iter().all(|h| h.title != "Table of Contents"),
            "{out:?}"
        );
    }

    #[test]
    fn small_caps_headings_count_as_set_apart() {
        let mut pages = Vec::new();
        for (i, name) in ["1. Introduction", "2. Background", "3. Results"]
            .iter()
            .enumerate()
        {
            let mut b = PageBuilder::new(i as u32)
                .body(20)
                .heading(name, 10.0, false)
                .with_font(3);
            if let Some(line) = b.page.lines.last_mut() {
                line.smallcaps = true;
            }
            pages.push(b.body(10).build());
        }
        let out = generate(&doc(pages));
        assert_eq!(out.len(), 3, "{out:?}");
    }

    #[test]
    fn a_full_width_heading_on_a_two_column_page_comes_before_both_columns() {
        let mut page1 = PageBuilder::new(1)
            .two_column_body()
            .heading_at("2 Methods", 200.0, 150.0)
            .heading_at("3 Results", 320.0, 400.0)
            .build();
        // the full-width heading spans the gutter
        if let Some(l) = page1.lines.iter_mut().find(|l| l.text == "2 Methods") {
            l.left = 72.0;
            l.right = 540.0;
        }
        let pages = vec![
            PageBuilder::new(0)
                .body(30)
                .heading("1 Introduction", 12.0, true)
                .body(5)
                .build(),
            page1,
        ];
        let out = generate(&doc(pages));
        let order: Vec<&str> = out.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(order, vec!["1 Introduction", "2 Methods", "3 Results"]);
    }

    #[test]
    fn text_inside_figures_is_not_a_heading() {
        // A plot's title in the figure's own font, as large and bold as a
        // section heading, and its axis labels.
        let mut pages = article();
        pages[1] = PageBuilder::new(1)
            .body(6)
            .heading("Accuracy against noise", 12.0, true)
            .with_font(2)
            .line("Noise level", 8.0, false, 200.0, 260.0, 4.0)
            .line("Accuracy", 8.0, false, 80.0, 120.0, 4.0)
            .body(4)
            .heading("2 Methods", 12.0, true)
            .body(5)
            .heading("2.1 Data", 10.0, true)
            .body(10)
            .heading("2.2 Model", 10.0, true)
            .body(8)
            .build();
        let out = generate(&doc(pages));
        assert!(out.iter().all(|h| !h.title.contains("Accuracy")), "{out:?}");
        assert!(out.iter().all(|h| !h.title.contains("Noise")));
        assert!(out.iter().any(|h| h.title == "2.1 Data"));
    }

    #[test]
    fn a_single_page_poster_gets_nothing() {
        let page = PageBuilder::new(0)
            .heading("Introduction", 40.0, true)
            .line("A point", 28.0, false, 72.0, 300.0, 20.0)
            .heading("Methods", 40.0, true)
            .line("Another point", 28.0, false, 72.0, 300.0, 20.0)
            .heading("Results", 40.0, true)
            .build();
        assert!(generate(&doc(vec![page])).is_empty());
    }

    #[test]
    fn pages_never_go_backwards() {
        let h = |page: u32| Heading {
            level: 0,
            title: format!("p{page}"),
            page,
            y: None,
        };
        let kept = in_page_order(vec![h(31), h(18), h(24), h(31), h(36)]);
        let pages: Vec<u32> = kept.iter().map(|h| h.page).collect();
        assert_eq!(pages, vec![18, 24, 31, 36]);
    }

    #[test]
    fn line_numbers_in_the_hundreds_are_not_section_numbers() {
        assert_eq!(strip_line_number("312 References"), "References");
        assert_eq!(strip_line_number("15 1 Introduction"), "1 Introduction");
        assert_eq!(strip_line_number("2 Methods"), "2 Methods");
    }

    #[test]
    fn a_checklist_numbered_on_from_the_sections_is_dropped() {
        // Sections 1-3 in 12pt bold, then a 10pt bold checklist whose
        // items run on as 4, 5, 6...
        let mut pages = article();
        let mut b = PageBuilder::new(3).body(5);
        for (i, item) in ["Claims", "Limitations", "Theory", "Experiments"]
            .iter()
            .enumerate()
        {
            b = b.heading(&format!("{}. {item}", i + 4), 10.0, true).body(3);
        }
        pages.push(b.build());
        let out = generate(&doc(pages));
        assert!(out.iter().all(|h| !h.title.contains("Claims")), "{out:?}");
        assert!(out.iter().any(|h| h.title == "3 Results"));
    }
}

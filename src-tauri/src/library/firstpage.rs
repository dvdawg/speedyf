//! A paper's title and authors as its first page prints them.
//!
//! The PDF's Info dictionary and XMP packet name the authors of fewer
//! than half the files in a typical library, and the first line of text
//! is as often a journal banner or the arXiv stamp as a title. What the
//! printed page says is plain to a reader, and only through layout: the
//! title is the largest type near the top, the authors are the lines
//! directly under it, before the affiliations. This module reads that
//! layout from `engine::layout_text::TextLine`s; it is pure, so every rule
//! here is tested against hand-built pages.
//!
//! Every answer carries a confidence. `High` means the page looked like
//! a paper's title page and parsed cleanly; the caller ranks `Low` values
//! below whatever the Info dictionary or XMP says.

use crate::engine::layout_text::{is_superscript_char, TextLine};
use crate::library::types::Author;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Low,
    High,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FirstPage {
    pub title: Option<String>,
    pub authors: Vec<Author>,
    pub title_confidence: Confidence,
    pub authors_confidence: Confidence,
}

/// Lines in the top this share of the page are running heads unless set
/// large; likewise the bottom `FOOTER_BAND`.
const HEADER_BAND: f32 = 0.06;
const FOOTER_BAND: f32 = 0.08;
/// The title is looked for in the top this share of the page.
const TITLE_REGION: f32 = 0.55;
/// Type this much larger than the body survives the header and footer
/// bands: a title page that starts high.
const LARGE_TYPE: f32 = 1.3;
/// The title's type must be this much larger than the body's for the
/// title to be "clearly the largest type".
const CLEAR_TITLE: f32 = 1.15;
/// A first page with fewer characters than this has no body to compare
/// the title against: a cover, a slide.
const MIN_PAGE_CHARS: usize = 400;
const MAX_TITLE_CHARS: usize = 300;
const MAX_TITLE_LINES: usize = 4;
const MAX_AUTHORS: usize = 50;

/// The title and authors printed on a first page. `lines` in reading
/// order as `layout_text::extract_lines` gives them; `page_width` and
/// `page_height` the displayed page's size in points.
pub fn extract(lines: &[TextLine], page_width: f32, page_height: f32) -> FirstPage {
    let none = FirstPage {
        title: None,
        authors: Vec::new(),
        title_confidence: Confidence::Low,
        authors_confidence: Confidence::Low,
    };
    if lines.is_empty() || page_height <= 0.0 {
        return none;
    }
    let body = body_size(lines);
    let total_chars: usize = lines.iter().map(|l| l.text.chars().count()).sum();
    let masthead = elsevier_masthead(lines);
    let kept: Vec<&TextLine> = lines
        .iter()
        .filter(|line| !is_noise(line, page_height, body))
        .filter(|line| !masthead.is_some_and(|(from, to)| line.top >= from && line.top <= to))
        .collect();

    let Some(title) = find_title(&kept, page_height, body) else {
        return none;
    };
    let portrait = page_width <= page_height * 1.05;
    // Clearly the largest type - or, in the AMS style, bold capitals at
    // the body's own size, which nothing else in the top tier is.
    let distinct = title.size >= CLEAR_TITLE * body || (title.bold && title.size >= body - 0.25);
    let title_high = title.top_tier
        && distinct
        && portrait
        && total_chars >= MIN_PAGE_CHARS
        && title.text.chars().count() <= 250;

    let sparse = total_chars < MIN_PAGE_CHARS;
    let block = find_authors(&kept, &title, page_height, body, sparse);
    let authors_high = title_high && !block.authors.is_empty() && block.rejected == 0;
    FirstPage {
        title: Some(title.text),
        authors: block.authors,
        title_confidence: if title_high {
            Confidence::High
        } else {
            Confidence::Low
        },
        authors_confidence: if authors_high {
            Confidence::High
        } else {
            Confidence::Low
        },
    }
}

/// The size most of the page's characters are set in, to the half point.
fn body_size(lines: &[TextLine]) -> f32 {
    let mut counts: Vec<(i32, usize)> = Vec::new();
    for line in lines {
        let key = (line.size * 2.0).round() as i32;
        let chars = line.text.chars().count();
        match counts.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += chars,
            None => counts.push((key, chars)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(key, n)| (n, -key))
        .map_or(10.0, |(key, _)| key as f32 / 2.0)
        .max(1.0)
}

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

/// The span, top to top, of an Elsevier masthead: from "Contents lists
/// available at ScienceDirect" to "journal homepage: ...", which frame the
/// journal's name in type as large as a title.
fn elsevier_masthead(lines: &[TextLine]) -> Option<(f32, f32)> {
    let find = |phrase: &str| {
        lines
            .iter()
            .find(|line| line.text.to_lowercase().contains(phrase))
            .map(|line| line.top)
    };
    let from = find("contents lists available")?;
    let to = find("journal homepage")?;
    (to > from).then_some((from, to))
}

/// Whether a line is page furniture rather than the paper's own heading:
/// running text (`is_running_text`), small type in the header or footer
/// band, text up a narrow left margin, or a line with no letters.
fn is_noise(line: &TextLine, page_height: f32, body: f32) -> bool {
    let alone_ampersand = line.text.trim() == "&";
    if (!line.text.chars().any(char::is_alphabetic) && !alone_ampersand)
        || is_running_text(&line.text)
    {
        return true;
    }
    let large = line.size >= LARGE_TYPE * body;
    let in_band =
        line.top < HEADER_BAND * page_height || line.bottom > (1.0 - FOOTER_BAND) * page_height;
    if in_band && !large {
        return true;
    }
    // Margin text set sideways that PDFium still reports glyph by glyph.
    let width = line.right - line.left;
    let height = line.bottom - line.top;
    line.left < 40.0 && width < 20.0 && height > 3.0 * width.max(1.0)
}

/// Words that name a journal in a running head.
const JOURNAL_WORDS: &[&str] = &[
    "journal",
    "review",
    "letters",
    "transactions",
    "proceedings",
    "annals",
    "bulletin",
    "communications",
    "acta",
    "reports",
    "physica",
    "magazine",
    "quarterly",
    "archives",
    "instruments",
    "j.",
];

/// Phrases that only running heads, footers, and publisher boilerplate
/// carry.
const BOILERPLATE: &[&str] = &[
    "contents lists available",
    "journal homepage",
    "sciencedirect",
    "www.",
    "http://",
    "https://",
    "doi:",
    "doi.org",
    "issn",
    "isbn",
    "\u{a9}",
    "(c) ",
    "copyright",
    "all rights reserved",
    "creative commons",
    "licensed under",
    "open access",
    "preprint",
    "under review",
    "submitted to",
    "to appear in",
    "accepted for publication",
    "available online",
    "printed in",
    "manuscript received",
    "communicated by",
    "corresponding author",
    "e-mail",
    "email:",
    "proceedings of",
    "published as",
    "published in",
    "conference paper at",
    "north-holland",
    "elsevier",
    "pergamon",
    "academic press",
    "springer-verlag",
    "full length article",
    "research article",
    "original article",
    "regular article",
    "review article",
    "short communication",
    "article info",
    "a r t i c l e",
    "anonymous author",
    "double-blind",
];

/// Prefixes of the dates a publisher prints under the title.
const DATE_PREFIXES: &[&str] = &[
    "received",
    "accepted",
    "revised",
    "published",
    "submitted",
    "dated:",
    "date:",
    "draft",
];

/// Whether a line is a running head, footer, or boilerplate rather than
/// content: the arXiv stamp, a journal banner ("PHYSICAL REVIEW LETTERS
/// 133, 057102 (2024)", "152 Nuclear Instruments and Methods..."), "Vol.
/// 12, No. 3", a DOI, ISSN or copyright line, received/accepted dates,
/// "Preprint", "Under review", a URL or email, a page number.
pub fn is_running_text(text: &str) -> bool {
    let text = text.trim();
    let lower = text.to_lowercase();
    if lower.starts_with("arxiv:") || lower.contains('@') {
        return true;
    }
    if BOILERPLATE.iter().any(|phrase| lower.contains(phrase)) {
        return true;
    }
    if DATE_PREFIXES.iter().any(|prefix| lower.starts_with(prefix)) {
        return true;
    }
    let words: Vec<&str> = lower.split_whitespace().collect();
    // "Vol. 12, No. 3", "Volume 36", "pp. 137-157", "Page 3 of 12".
    let numbered_label = words.windows(2).any(|pair| {
        matches!(pair[0], "vol." | "vol" | "volume" | "no." | "pp." | "page")
            && pair[1].starts_with(|c: char| c.is_ascii_digit())
    });
    if numbered_label {
        return true;
    }
    let significant: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let digits = significant.iter().filter(|c| c.is_ascii_digit()).count();
    if significant.is_empty() || digits * 10 > significant.len() * 4 {
        return true;
    }
    // A page number alone, arabic or roman: "137", "xii", "- 3 -".
    let core: String = lower
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '\u{2013}' | '\u{2014}'))
        .collect();
    if core.chars().all(|c| c.is_ascii_digit())
        || (core.len() <= 6
            && core
                .chars()
                .all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c')))
    {
        return true;
    }
    // A journal banner: a year in parentheses beside other numbers, or a
    // journal word beside a bare number ("SCIENCES 36", "152 Nuclear
    // Instruments", "Letters 8 (1989").
    let has_number = words.iter().any(|word| {
        let core = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        core.chars().any(|c| c.is_ascii_digit())
            && core.chars().all(|c| c.is_ascii_digit() || c == '-')
    });
    let has_year_in_parens = lower.match_indices('(').any(|(i, _)| {
        let year: String = lower[i + 1..].chars().take(4).collect();
        year.len() == 4
            && year.chars().all(|c| c.is_ascii_digit())
            && matches!(&year[..2], "18" | "19" | "20")
    });
    if has_year_in_parens && digits >= 6 {
        return true;
    }
    let journal_word = words
        .iter()
        .any(|w| JOURNAL_WORDS.contains(&w.trim_matches(|c: char| c == ',' || c == ':')));
    journal_word && has_number
}

// ---------------------------------------------------------------------------
// Title
// ---------------------------------------------------------------------------

struct Title {
    text: String,
    size: f32,
    bottom: f32,
    /// Index into `kept` of the title's last line.
    lines: Vec<usize>,
    /// Whether it was set in the largest type among the candidates.
    top_tier: bool,
    bold: bool,
}

/// The largest type in the top of the page, consecutive lines of it read
/// as one title. Tiers of size are tried from the largest down, so a
/// large line that cannot be a title (a single word, a logo) gives way
/// to the next; within a tier a bold group wins over a regular one and
/// the higher over the lower.
fn find_title(kept: &[&TextLine], page_height: f32, body: f32) -> Option<Title> {
    let candidates: Vec<usize> = (0..kept.len())
        .filter(|&i| {
            kept[i].top < TITLE_REGION * page_height
                && kept[i].text.chars().filter(|c| c.is_alphabetic()).count() >= 3
                && !is_heading(&kept[i].text)
        })
        .collect();
    let mut tiers: Vec<f32> = Vec::new();
    let mut sizes: Vec<f32> = candidates.iter().map(|&i| kept[i].size).collect();
    sizes.sort_by(|a, b| b.total_cmp(a));
    for size in sizes {
        if tiers.last().is_none_or(|last| *last - size > 0.5) {
            tiers.push(size);
        }
    }
    // In reading position: top to bottom, left to right.
    let mut ordered = candidates.clone();
    ordered.sort_by(|&a, &b| {
        kept[a]
            .top
            .total_cmp(&kept[b].top)
            .then(kept[a].left.total_cmp(&kept[b].left))
    });

    for (tier_index, &tier) in tiers.iter().take(4).enumerate() {
        if tier < body - 0.25 {
            break;
        }
        let in_tier = |i: usize| (kept[i].size - tier).abs() <= 0.5;
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        for &i in &ordered {
            if !in_tier(i) {
                // A smaller line between two title lines ends the group,
                // unless it stands beside them on the same row.
                let beside = current
                    .last()
                    .is_some_and(|&last| same_row(kept[last], kept[i]));
                if !beside && !current.is_empty() {
                    groups.push(std::mem::take(&mut current));
                }
                continue;
            }
            if let Some(&last) = current.last() {
                let gap = kept[i].top - kept[last].bottom;
                if !same_row(kept[last], kept[i]) && gap > 1.2 * tier {
                    groups.push(std::mem::take(&mut current));
                }
            }
            current.push(i);
        }
        if !current.is_empty() {
            groups.push(current);
        }

        let mut best: Option<(bool, Title)> = None;
        for group in groups {
            let Some(text) = title_text(&group.iter().map(|&i| kept[i]).collect::<Vec<_>>()) else {
                continue;
            };
            let bold = group.iter().filter(|&&i| kept[i].bold).count() * 2 > group.len();
            let better = match &best {
                None => true,
                Some((best_bold, _)) => bold && !best_bold,
            };
            if better {
                let bottom = group.iter().map(|&i| kept[i].bottom).fold(0.0, f32::max);
                best = Some((
                    bold,
                    Title {
                        text,
                        size: tier,
                        bottom,
                        lines: group,
                        top_tier: tier_index == 0,
                        bold,
                    },
                ));
            }
        }
        if let Some((_, title)) = best {
            return Some(title);
        }
    }
    None
}

fn same_row(a: &TextLine, b: &TextLine) -> bool {
    (a.top - b.top).abs() < 0.5 * a.size.min(b.size)
}

/// A group of lines as a title, or `None` when it cannot be one: fewer
/// than two words or twelve characters, too long, a banner, mostly
/// digits. Footnote markers at the end go; an all-capitals title is
/// recased.
fn title_text(lines: &[&TextLine]) -> Option<String> {
    if lines.len() > MAX_TITLE_LINES {
        return None;
    }
    let mut text = String::new();
    for line in lines {
        let part = line.text.trim();
        if text.ends_with('-') && part.starts_with(|c: char| c.is_lowercase()) {
            // "Non-" + "linear": the hyphen stays, the break goes.
        } else if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(part);
    }
    let text = clean_title(&text);
    let chars = text.chars().count();
    if text.split_whitespace().count() < 2
        || !(12..=MAX_TITLE_CHARS).contains(&chars)
        || is_running_text(&text)
    {
        return None;
    }
    Some(text)
}

fn is_marker(c: char) -> bool {
    // ascii-guard: data begin - characters matched in page text
    matches!(c, '*' | '∗' | '†' | '‡' | '§' | '¶' | '⋆' | '♯' | '♭' | '‖') || is_superscript_char(c)
    // ascii-guard: data end
}

/// Whitespace collapsed, trailing footnote markers removed, an
/// all-capitals title put in title case.
pub fn clean_title(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim_end_matches(|c: char| is_marker(c) || c == ',' || c.is_whitespace())
        .to_string();
    let letters: Vec<char> = trimmed.chars().filter(|c| c.is_alphabetic()).collect();
    let upper = letters.iter().filter(|c| c.is_uppercase()).count();
    if letters.len() >= 6 && upper * 10 >= letters.len() * 9 {
        title_case(&trimmed)
    } else {
        trimmed
    }
}

const MINOR_WORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "for", "and", "or", "to", "with", "by", "at", "from",
    "via", "as", "is", "into", "over", "under", "its",
];

fn title_case(text: &str) -> String {
    text.split(' ')
        .enumerate()
        .map(|(i, word)| {
            let lower = word.to_lowercase();
            if i > 0 && MINOR_WORDS.contains(&lower.as_str()) {
                lower
            } else {
                capitalize_word(word)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "JEAN-PIERRE" -> "Jean-Pierre", "O'NEIL" -> "O'neil"; a word with a
/// digit ("3D", "SU(2)") is left alone.
fn capitalize_word(word: &str) -> String {
    if word.chars().any(|c| c.is_ascii_digit()) {
        return word.to_string();
    }
    let mut out = String::with_capacity(word.len());
    let mut start = true;
    for c in word.chars() {
        if start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            start = false;
        } else {
            out.extend(c.to_lowercase());
        }
        if matches!(c, '-' | '\u{2013}') {
            start = true;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Authors
// ---------------------------------------------------------------------------

// ascii-guard: data begin - words matched in page text
/// Words that mark an affiliation or address; a name never contains one.
const AFFILIATION_WORDS: &[&str] = &[
    "university",
    "universität",
    "universitat",
    "université",
    "universite",
    "universidad",
    "università",
    "universiteit",
    "univ.",
    "college",
    "department",
    "dept.",
    "dept",
    "institute",
    "institut",
    "instituto",
    "istituto",
    "laboratory",
    "laboratories",
    "laboratoire",
    "lab",
    "labs",
    "school",
    "center",
    "centre",
    "faculty",
    "inc.",
    "inc",
    "corp.",
    "corp",
    "corporation",
    "ltd",
    "ltd.",
    "llc",
    "gmbh",
    "company",
    "hospital",
    "academy",
    "division",
    "research",
    "foundation",
    "street",
    "avenue",
    "ave.",
    "road",
    "campus",
    "building",
    "usa",
    "u.s.a.",
    "uk",
    "u.k.",
    "china",
    "germany",
    "france",
    "japan",
    "canada",
    "italy",
    "switzerland",
    "israel",
    "korea",
    "india",
    "australia",
    "netherlands",
    "spain",
    "sweden",
    "singapore",
    "denmark",
    "austria",
    "belgium",
    "brazil",
    "russia",
    "poland",
    "finland",
    "norway",
    "taiwan",
    "united",
    "google",
    "microsoft",
    "deepmind",
    "openai",
    "anthropic",
    "meta",
    "amazon",
    "apple",
    "facebook",
    "nvidia",
    "ibm",
    "cnrs",
    "inria",
    "mit",
    "cmu",
    "eth",
    "epfl",
    "uc",
    "ucla",
    "ucsd",
    "nyu",
    "caltech",
    "ttic",
    "kaist",
    "cern",
    "riken",
    "anonymous",
];
// ascii-guard: data end

/// Capitalized words a title, heading or role has and a name does not.
const NON_NAME_WORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "on",
    "of",
    "for",
    "learning",
    "network",
    "networks",
    "theory",
    "analysis",
    "model",
    "models",
    "using",
    "toward",
    "towards",
    "approach",
    "system",
    "systems",
    "method",
    "methods",
    "data",
    "optimization",
    "algorithm",
    "algorithms",
    "neural",
    "deep",
    "quantum",
    "dynamics",
    "study",
    "introduction",
    "abstract",
    "keywords",
    "chapter",
    "lecture",
    "notes",
    "manual",
    "guide",
    "version",
    "edition",
    "volume",
    "part",
    "section",
    "appendix",
    "table",
    "figure",
    "contents",
    "preface",
    "report",
    "technical",
    "thesis",
    "dissertation",
    "conference",
    "journal",
    "review",
    "press",
    "publishing",
    "editor",
    "editors",
    "author",
    "authors",
    "correspondence",
    "equal",
    "contribution",
    "acm",
    "siam",
    "user",
    "reference",
    "handbook",
    "overview",
    "tutorial",
    "slides",
    "presentation",
    "welcome",
    "agenda",
    "outline",
    "physics",
    "mathematics",
    "science",
    "sciences",
    "engineering",
    "statistics",
    "computer",
    "computing",
    "information",
    "problem",
    "problems",
    "random",
    "linear",
    "nonlinear",
    "general",
    "new",
    "how",
    "what",
    "why",
    "when",
    "is",
    "are",
    "inside",
];

/// Parts that sit among names and are neither names nor evidence against
/// the block: IEEE grades, degrees, the word that joins.
const IGNORABLE: &[&str] = &[
    "member",
    "senior member",
    "student member",
    "life member",
    "fellow",
    "life fellow",
    "ieee",
    "member ieee",
    "senior member ieee",
    "student member ieee",
    "fellow ieee",
    "life fellow ieee",
    "phd",
    "ph.d.",
    "ph.d",
    "md",
    "m.d.",
    "by",
    "and",
    "&",
    "et al.",
    "jr.",
    "jr",
];

const PARTICLES: &[&str] = &[
    "van", "von", "der", "den", "de", "del", "della", "di", "du", "le", "la", "da", "dos", "das",
    "ten", "ter", "bin", "al", "el", "zu", "y",
];

const SUFFIXES: &[&str] = &["jr.", "jr", "sr.", "sr", "ii", "iii", "iv"];

/// Headings that end the front matter.
const SECTION_HEADINGS: &[&str] = &[
    "abstract",
    "introduction",
    "keywords",
    "key words",
    "contents",
    "summary",
    "index terms",
    "table of contents",
];

#[derive(Default)]
struct AuthorBlock {
    authors: Vec<Author>,
    /// Parts in name rows that were neither names, markers, nor
    /// affiliations.
    rejected: usize,
}

/// One row of the page: lines standing side by side, left to right.
struct Row<'a> {
    lines: Vec<&'a TextLine>,
    top: f32,
    bottom: f32,
    size: f32,
    bold: bool,
}

impl Row<'_> {
    fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn rows_below<'a>(kept: &[&'a TextLine], title: &Title, page_height: f32) -> Vec<Row<'a>> {
    let mut below: Vec<&TextLine> = kept
        .iter()
        .enumerate()
        .filter(|(i, line)| !title.lines.contains(i) && line.top >= title.bottom - 0.25 * line.size)
        .map(|(_, line)| *line)
        .filter(|line| line.top < 0.8 * page_height)
        .collect();
    below.sort_by(|a, b| a.top.total_cmp(&b.top).then(a.left.total_cmp(&b.left)));
    let mut rows: Vec<Row> = Vec::new();
    for line in below {
        match rows.last_mut() {
            Some(row) if same_row(row.lines[0], line) => {
                row.lines.push(line);
                row.bottom = row.bottom.max(line.bottom);
            }
            _ => rows.push(Row {
                lines: vec![line],
                top: line.top,
                bottom: line.bottom,
                size: 0.0,
                bold: false,
            }),
        }
    }
    for row in &mut rows {
        row.lines.sort_by(|a, b| a.left.total_cmp(&b.left));
        let chars: usize = row.lines.iter().map(|l| l.text.chars().count()).sum();
        let weighted: f32 = row
            .lines
            .iter()
            .map(|l| l.size * l.text.chars().count() as f32)
            .sum();
        row.size = weighted / chars.max(1) as f32;
        let bold_chars: usize = row
            .lines
            .iter()
            .filter(|l| l.bold)
            .map(|l| l.text.chars().count())
            .sum();
        row.bold = bold_chars * 2 > chars;
    }
    rows
}

/// The author block: rows directly under the title, before the first
/// affiliation, email, section heading, or paragraph. When the names are
/// set in a style the affiliations are not (bold, or a different size),
/// rows in the names' style after an affiliation are read too - the grid
/// a NeurIPS or ICML title page lays its authors out in.
///
/// Older papers stack one author at a time: a name, its affiliation lines,
/// then "AND" alone on a line, the next name, its affiliations. A
/// standalone connector carries the block past the affiliations it would
/// otherwise stop at: the next name-shaped row is another author.
fn find_authors(
    kept: &[&TextLine],
    title: &Title,
    page_height: f32,
    body: f32,
    sparse: bool,
) -> AuthorBlock {
    let rows = rows_below(kept, title, page_height);
    let mut block = AuthorBlock::default();
    let mut name_style: Option<(f32, bool)> = None;
    let mut prev_bottom = title.bottom;
    let mut after_by = false;
    let mut skipped_before_names = 0;
    let mut past_names = false;
    // Past where the block used to end: only a connector row reopens it.
    let mut stalled = false;
    // A connector row was read; rows skipped while waiting for the name.
    let mut connector: Option<usize> = None;

    for row in rows.iter().take(24) {
        let gap = row.top - prev_bottom;
        let limit = if block.authors.is_empty() {
            let first = (4.0 * body).max(0.12 * page_height);
            // A cover or report title page spreads out what a paper's
            // first page packs together.
            if after_by || sparse {
                first.max(0.25 * page_height)
            } else {
                first
            }
        } else {
            (3.0 * row.size).max(2.5 * body)
        };
        if gap > limit || row.top > 0.7 * page_height {
            break;
        }
        let text = row.text();
        if is_heading(&text) || looks_like_prose(&text) {
            break;
        }
        prev_bottom = row.bottom;
        if text.trim().eq_ignore_ascii_case("by") {
            after_by = true;
            continue;
        }
        if !block.authors.is_empty() && is_connector(&text) {
            connector = Some(0);
            stalled = false;
            continue;
        }
        if let Some(skipped) = connector {
            let parsed = parse_row(row);
            if !parsed.names.is_empty() && parsed.rejected == 0 {
                push_names(&mut block, parsed.names);
                connector = None;
                // Its affiliations follow; only another connector goes on.
                stalled = true;
                continue;
            }
            // The previous author's last affiliation lines, at most two.
            if skipped >= 2 {
                break;
            }
            connector = Some(skipped + 1);
            continue;
        }
        if stalled {
            continue;
        }

        if let Some((size, bold)) = name_style {
            let same_style = (row.size - size).abs() <= 0.5 && row.bold == bold;
            if !same_style || past_names {
                // Not the names' style, or past the first affiliation: an
                // affiliation, address or note row, or the next row of a
                // grid - which only a distinct name style can tell.
                let distinct = row.bold != bold || (row.size - size).abs() >= 0.5;
                if !same_style {
                    past_names = true;
                    if !distinct {
                        stalled = true;
                    }
                    continue;
                }
                let parsed = parse_row(row);
                if parsed.names.is_empty() || parsed.rejected > 0 {
                    continue;
                }
                push_names(&mut block, parsed.names);
                continue;
            }
        }

        let parsed = parse_row(row);
        if parsed.names.is_empty() {
            if !block.authors.is_empty() {
                past_names = true;
                // An affiliation row in the names' own style: nothing to
                // tell the grid's next names from more affiliations.
                if name_style
                    .is_some_and(|(size, bold)| (row.size - size).abs() < 0.5 && row.bold == bold)
                {
                    stalled = true;
                }
                continue;
            }
            if parsed.affiliation {
                break;
            }
            skipped_before_names += 1;
            if skipped_before_names > 2 {
                break;
            }
            continue;
        }
        if name_style.is_none() {
            name_style = Some((row.size, row.bold));
        }
        block.rejected += parsed.rejected;
        push_names(&mut block, parsed.names);
        if parsed.affiliation {
            past_names = true;
        }
    }
    block
}

/// "AND", "and", "&" or "with" alone on a line, between two authors.
fn is_connector(text: &str) -> bool {
    let word = text.trim().to_lowercase();
    matches!(word.as_str(), "and" | "&" | "with")
}

fn push_names(block: &mut AuthorBlock, names: Vec<Author>) {
    for name in names {
        if block.authors.len() >= MAX_AUTHORS {
            return;
        }
        if !block.authors.contains(&name) {
            block.authors.push(name);
        }
    }
}

fn is_heading(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    // Less a section number: "1 Introduction", "I. INTRODUCTION".
    let lower = match lower.split_once(' ') {
        Some((first, rest))
            if first
                .trim_end_matches('.')
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, 'i' | 'v' | 'x')) =>
        {
            rest.trim_start()
        }
        _ => lower.as_str(),
    };
    SECTION_HEADINGS.iter().any(|heading| {
        lower.starts_with(heading)
            && lower[heading.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphabetic())
    })
}

/// A sentence, not a name line: long, or mostly lowercase words. "A
/// dissertation submitted in partial satisfaction..." ends a thesis's
/// author block the way an abstract ends a paper's.
fn looks_like_prose(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if text.chars().count() > 160 {
        return true;
    }
    let lowercase = words
        .iter()
        .filter(|w| w.starts_with(|c: char| c.is_lowercase()))
        .filter(|w| !PARTICLES.contains(w) && !matches!(**w, "and" | "by"))
        .count();
    words.len() >= 6 && lowercase * 10 >= words.len() * 4
}

struct ParsedRow {
    names: Vec<Author>,
    rejected: usize,
    /// Whether the row holds an affiliation or email, alone or after the
    /// names.
    affiliation: bool,
}

fn parse_row(row: &Row) -> ParsedRow {
    let mut parsed = ParsedRow {
        names: Vec::new(),
        rejected: 0,
        affiliation: false,
    };
    for line in &row.lines {
        let segment = parse_names(&line.text);
        parsed.names.extend(segment.names);
        parsed.rejected += segment.rejected;
        parsed.affiliation |= segment.affiliation;
    }
    parsed
}

/// Names in one line of an author block. Footnote and affiliation markers
/// separate names as commas do; the line is cut at the first affiliation
/// word or email.
fn parse_names(text: &str) -> ParsedRow {
    let mut parsed = ParsedRow {
        names: Vec::new(),
        rejected: 0,
        affiliation: false,
    };
    let marked = mark_separators(text);
    let marked = marked.strip_prefix("by ").unwrap_or(&marked).to_string();

    // "Family, G." throughout, or "Given Family" - decided per line.
    let mut parts: Vec<String> = Vec::new();
    for chunk in marked.split(['|', ';']) {
        let pieces: Vec<String> = chunk
            .split(',')
            .map(strip_conjunction)
            .filter(|p| !p.is_empty())
            .collect();
        if is_inverted(&pieces) {
            for pair in pieces.chunks(2) {
                parts.push(format!("{} {}", pair[1], pair[0]));
            }
            continue;
        }
        for piece in pieces {
            for part in split_conjunctions(&piece) {
                parts.push(part);
            }
        }
    }

    for part in parts {
        let lower = part.to_lowercase();
        if is_affiliation(&lower) {
            parsed.affiliation = true;
            break;
        }
        if IGNORABLE.contains(&lower.trim_end_matches('.')) || IGNORABLE.contains(&lower.as_str()) {
            continue;
        }
        match parse_name(&part) {
            Some(name) => parsed.names.push(name),
            None => parsed.rejected += 1,
        }
    }
    parsed
}

/// Markers ("Smith¹", "Zhong*", "Smith1,2") replaced by `|`, a separator
/// as strong as a comma.
fn mark_separators(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut in_marker = false;
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let marker = is_marker(c)
            || (c.is_ascii_digit() && (in_marker || prev.is_some_and(char::is_alphabetic)))
            || (in_marker
                && c == ','
                && chars
                    .get(i + 1)
                    .is_some_and(|n| n.is_ascii_digit() || is_marker(*n)));
        if marker {
            if !in_marker {
                out.push('|');
            }
            in_marker = true;
        } else {
            in_marker = false;
            out.push(c);
        }
    }
    out
}

fn strip_conjunction(piece: &str) -> String {
    let mut piece = piece.trim();
    for prefix in ["and ", "AND ", "& ", "And "] {
        if let Some(rest) = piece.strip_prefix(prefix) {
            piece = rest.trim();
        }
    }
    piece.to_string()
}

fn split_conjunctions(piece: &str) -> Vec<String> {
    let mut parts = vec![piece.to_string()];
    for sep in [" and ", " AND ", " & ", " And "] {
        parts = parts
            .into_iter()
            .flat_map(|p| p.split(sep).map(str::to_string).collect::<Vec<_>>())
            .collect();
    }
    parts
        .into_iter()
        .map(|p| strip_conjunction(&p))
        .filter(|p| !p.is_empty())
        .collect()
}

/// Whether comma-separated pieces read "Family, G., Family, G." all the
/// way through: an even count, every second piece initials only.
fn is_inverted(pieces: &[String]) -> bool {
    pieces.len() >= 2
        && pieces.len() % 2 == 0
        && pieces.chunks(2).all(|pair| {
            let family_words: Vec<&str> = pair[0].split_whitespace().collect();
            !family_words.is_empty()
                && family_words.len() <= 3
                && family_words
                    .iter()
                    .all(|w| is_name_word(w) || is_particle(w))
                && family_words.iter().any(|w| !is_initial(w))
                && pair[1].split_whitespace().all(is_initial)
        })
}

fn is_affiliation(lower: &str) -> bool {
    if lower.contains('@') {
        return true;
    }
    // A postal code: five or more digits together.
    let mut run = 0;
    for c in lower.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 5 {
            return true;
        }
    }
    lower
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '/'))
        .any(|word| AFFILIATION_WORDS.contains(&word))
}

fn is_particle(word: &str) -> bool {
    PARTICLES.contains(&word)
}

/// "G.", "J.-P.", "J.K.", "R" - a given name reduced to letters.
fn is_initial(word: &str) -> bool {
    let core = word.trim_end_matches(',');
    !core.is_empty()
        && core.split('-').all(|piece| {
            let letters: Vec<char> = piece.chars().filter(|c| *c != '.').collect();
            !letters.is_empty()
                && letters.len() <= 3
                && letters.iter().all(|c| c.is_uppercase())
                && (piece.contains('.') || letters.len() == 1)
        })
}

/// A capitalized word of letters, hyphens and apostrophes: "Jean-Pierre",
/// "O'Neil", "DeWeese", "SMITH".
fn is_name_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_uppercase()
        && word.chars().count() <= 25
        && word
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, '-' | '\'' | '\u{2019}' | '.'))
        && !NON_NAME_WORDS.contains(&word.to_lowercase().as_str())
}

/// One name, "Given Middle Family" with particles kept in the family
/// ("Johannes van der Waals" -> van der Waals, Johannes), or `None` when
/// the part is not name-shaped.
fn parse_name(part: &str) -> Option<Author> {
    // A period set apart from its initial ("Philip M . Long") rejoins it.
    let mut words: Vec<String> = Vec::new();
    for word in part.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '(' | ')'));
        match (word, words.last_mut()) {
            ("", _) => {}
            (".", Some(last)) => last.push('.'),
            _ => words.push(word.to_string()),
        }
    }
    let mut words: Vec<&str> = words.iter().map(String::as_str).collect();
    while words
        .last()
        .is_some_and(|w| SUFFIXES.contains(&w.to_lowercase().as_str()))
    {
        words.pop();
    }
    let core = words.iter().filter(|w| !is_particle(w)).count();
    if !(2..=5).contains(&core) || words.len() > 7 {
        return None;
    }
    let last = *words.last()?;
    if is_initial(last) || is_particle(last) || is_particle(words[0]) {
        return None;
    }
    if !words
        .iter()
        .all(|w| is_name_word(w) || is_initial(w) || is_particle(w))
    {
        return None;
    }
    // Capitals throughout, or a family name in capitals after given names
    // that are not ("Luc DEVROYE"); a word in capitals anywhere else is an
    // acronym ("UC Berkeley").
    let caps = |w: &str| {
        !is_initial(w)
            && w.chars().filter(|c| c.is_alphabetic()).count() >= 2
            && w.chars()
                .filter(|c| c.is_alphabetic())
                .all(char::is_uppercase)
    };
    let caps_count = words.iter().filter(|w| caps(w)).count();
    let all_caps = caps_count
        == words
            .iter()
            .filter(|w| !is_initial(w) && !is_particle(w))
            .count();
    if caps_count > 0 && !all_caps && !(caps_count == 1 && caps(last)) {
        return None;
    }
    let mut family_start = words.len() - 1;
    while family_start > 1 && is_particle(words[family_start - 1]) {
        family_start -= 1;
    }
    let recase = |w: &&str| {
        if caps(w) {
            capitalize_word(w)
        } else {
            (*w).to_string()
        }
    };
    let given: Vec<String> = words[..family_start].iter().map(recase).collect();
    let family: Vec<String> = words[family_start..].iter().map(recase).collect();
    Some(Author {
        family: family.join(" "),
        given: given.join(" "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 612.0;
    const H: f32 = 792.0;

    /// A line of `text` at `size`, its top at `top`, starting at `left`,
    /// each character half an em wide.
    fn line(text: &str, size: f32, top: f32, left: f32) -> TextLine {
        TextLine {
            text: text.to_string(),
            size,
            weight: 400,
            bold: false,
            top,
            bottom: top + size,
            left,
            right: left + 0.5 * size * text.chars().count() as f32,
        }
    }

    fn bold(mut l: TextLine) -> TextLine {
        l.bold = true;
        l.weight = 700;
        l
    }

    /// Body text below `top`: enough characters to make the page a
    /// paper's, set at `size`.
    fn body(top: f32, size: f32) -> Vec<TextLine> {
        (0..12)
            .map(|i| {
                line(
                    "we study the dynamics of learning in deep linear networks and show that",
                    size,
                    top + i as f32 * (size + 2.0),
                    72.0,
                )
            })
            .collect()
    }

    fn names(page: &FirstPage) -> Vec<(String, String)> {
        page.authors
            .iter()
            .map(|a| (a.given.clone(), a.family.clone()))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(g, f)| (g.to_string(), f.to_string()))
            .collect()
    }

    #[test]
    fn arxiv_preprint_with_margin_stamp() {
        let mut lines = vec![
            // The stamp, if PDFium hands it over as a line at all.
            line(
                "arXiv:1201.4166v2 [cond-mat.stat-mech] 8 Mar 2012",
                20.0,
                250.0,
                12.0,
            ),
            line("A Theory of Saddle Escape in Deep", 17.0, 90.0, 120.0),
            line("Nonlinear Networks", 17.0, 110.0, 200.0),
            line(
                "Alice Smith,¹ Bob Jones,² and Carol Lee¹",
                11.0,
                150.0,
                150.0,
            ),
            line(
                "¹Department of Physics, Stanford University",
                9.0,
                168.0,
                150.0,
            ),
            line(
                "²Google Research, Mountain View, CA 94043",
                9.0,
                180.0,
                150.0,
            ),
            bold(line("Abstract", 10.0, 210.0, 280.0)),
        ];
        lines.extend(body(230.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            page.title.as_deref(),
            Some("A Theory of Saddle Escape in Deep Nonlinear Networks")
        );
        assert_eq!(
            names(&page),
            pairs(&[("Alice", "Smith"), ("Bob", "Jones"), ("Carol", "Lee")])
        );
        assert_eq!(page.title_confidence, Confidence::High);
        assert_eq!(page.authors_confidence, Confidence::High);
    }

    #[test]
    fn aps_paper_with_banner_and_starred_authors() {
        let mut lines =
            vec![
            line("PHYSICAL REVIEW LETTERS 133, 057102 (2024)", 9.0, 40.0, 72.0),
            line("Editors' Suggestion", 9.0, 62.0, 72.0),
            bold(line(
                "Beyond Linear Response: Equivalence between Thermodynamic",
                12.0,
                80.0,
                90.0,
            )),
            bold(line(
                "Geometry and Optimal Transport",
                12.0,
                95.0,
                190.0,
            )),
            line("Adrianne Zhong* and Michael R. DeWeese†", 10.0, 120.0, 180.0),
            line(
                "Department of Physics, University of California, Berkeley, California 94720, USA",
                8.0,
                134.0,
                100.0,
            ),
            line(
                "(Received 1 December 2023; accepted 3 June 2024; published 1 August 2024)",
                8.0,
                150.0,
                100.0,
            ),
        ];
        lines.extend(body(170.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            page.title.as_deref(),
            Some("Beyond Linear Response: Equivalence between Thermodynamic Geometry and Optimal Transport")
        );
        assert_eq!(
            names(&page),
            pairs(&[("Adrianne", "Zhong"), ("Michael R.", "DeWeese")])
        );
        assert_eq!(page.authors_confidence, Confidence::High);
    }

    #[test]
    fn elsevier_scan_with_banner_and_capitals_title() {
        let mut lines = vec![
            line(
                "JOURNAL OF COMPUTER AND SYSTEM SCIENCES 36, 137-157 (1988)",
                11.0,
                60.0,
                90.0,
            ),
            line("THE COMPLEXITY OF MANY CELLS IN", 14.0, 110.0, 150.0),
            line("ARRANGEMENTS OF PLANES", 14.0, 128.0, 180.0),
            line("JOHN E. HOPCROFT AND JEFFREY D. ULLMAN", 10.0, 160.0, 150.0),
            line(
                "Cornell University, Ithaca, New York 14853",
                9.0,
                176.0,
                170.0,
            ),
            line("Received January 5, 1987", 9.0, 200.0, 200.0),
        ];
        lines.extend(body(230.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            page.title.as_deref(),
            Some("The Complexity of Many Cells in Arrangements of Planes")
        );
        assert_eq!(
            names(&page),
            pairs(&[("John E.", "Hopcroft"), ("Jeffrey D.", "Ullman")])
        );
    }

    /// The 1988 JCSS layout: one author at a time, each followed by its
    /// affiliation lines, "AND" alone between them.
    fn jcss_stacked_authors(name_size: f32, rest_size: f32) -> Vec<TextLine> {
        let mut lines = vec![
            line(
                "JOURNAL OF COMPUTER AND SYSTEM SCIENCES 36, 254-276 (1988)",
                9.0,
                60.0,
                90.0,
            ),
            line(
                "ARTHUR-MERLIN GAMES: A RANDOMIZED PROOF",
                14.0,
                110.0,
                130.0,
            ),
            line(
                "SYSTEM, AND A HIERARCHY OF COMPLEXITY CLASSES",
                14.0,
                128.0,
                110.0,
            ),
            line("LASZLO BABAI", name_size, 160.0, 250.0),
            line(
                "Department of Algebra, Eotvos University,",
                rest_size,
                176.0,
                170.0,
            ),
            line("Budapest, Hungary H-1088, and", rest_size, 188.0, 200.0),
            line(
                "Department of Computer Science, University of Chicago,",
                rest_size,
                200.0,
                140.0,
            ),
            line("Chicago, Illinois 60637", rest_size, 212.0, 230.0),
            line("AND", rest_size, 230.0, 295.0),
            line("SHLOMO MORAN", name_size, 248.0, 245.0),
            line(
                "Department of Computer Science, Technion,",
                rest_size,
                264.0,
                170.0,
            ),
            line("Haifa 32000, Israel", rest_size, 276.0, 240.0),
            line("Received October 10, 1986", rest_size, 300.0, 220.0),
        ];
        lines.extend(body(330.0, 10.0));
        lines
    }

    #[test]
    fn and_alone_between_stacked_authors() {
        // Names and affiliations in the same type, as an OCR'd scan
        // reports them; then names set larger than the affiliations.
        for (name_size, rest_size) in [(10.0, 10.0), (11.0, 9.0)] {
            let page = extract(&jcss_stacked_authors(name_size, rest_size), W, H);
            assert_eq!(
                page.title.as_deref(),
                Some("Arthur-Merlin Games: a Randomized Proof System, and a Hierarchy of Complexity Classes"),
                "{name_size}/{rest_size}"
            );
            assert_eq!(
                names(&page),
                pairs(&[("Laszlo", "Babai"), ("Shlomo", "Moran")]),
                "{name_size}/{rest_size}"
            );
        }
    }

    #[test]
    fn and_alone_between_bold_authors_and_three_of_them() {
        let mut lines = vec![
            bold(line("On Stacked Author Blocks", 16.0, 100.0, 180.0)),
            bold(line("ALICE SMITH", 10.0, 140.0, 250.0)),
            line("University of Toronto", 10.0, 154.0, 230.0),
            line("and", 10.0, 170.0, 295.0),
            bold(line("BOB JONES", 10.0, 186.0, 255.0)),
            line("Bell Laboratories", 10.0, 200.0, 240.0),
            line("Murray Hill, New Jersey 07974", 10.0, 212.0, 210.0),
            line("&", 10.0, 228.0, 300.0),
            bold(line("CAROL LEE", 10.0, 244.0, 255.0)),
            line("IBM Research", 10.0, 258.0, 245.0),
        ];
        lines.extend(body(300.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            names(&page),
            pairs(&[("Alice", "Smith"), ("Bob", "Jones"), ("Carol", "Lee")])
        );
    }

    #[test]
    fn and_alone_does_not_swallow_the_abstract() {
        let mut lines = vec![
            bold(line(
                "Sparse Attention Is All You Need Sometimes",
                16.0,
                100.0,
                130.0,
            )),
            line("Alice Smith", 12.0, 140.0, 260.0),
            line("and", 12.0, 156.0, 295.0),
            line(
                "We show that sparse attention matches dense attention on long",
                10.0,
                180.0,
                72.0,
            ),
            line(
                "documents while using a fraction of the memory and compute.",
                10.0,
                192.0,
                72.0,
            ),
            line("Bob Jones Carol Lee", 10.0, 204.0, 72.0),
        ];
        lines.extend(body(230.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(names(&page), pairs(&[("Alice", "Smith")]));
    }

    #[test]
    fn neurips_grid_of_names_and_affiliations() {
        let mut lines = vec![
            line("Preprint. Under review.", 9.0, 30.0, 72.0),
            bold(line(
                "Scaling Laws for Sparse Mixture Models",
                17.0,
                100.0,
                130.0,
            )),
            bold(line("Alice Smith", 10.0, 150.0, 120.0)),
            bold(line("Bob Jones", 10.0, 150.0, 330.0)),
            line("Department of Computer Science", 10.0, 162.0, 90.0),
            line("Google DeepMind", 10.0, 162.0, 320.0),
            line("Stanford University", 10.0, 174.0, 110.0),
            line("London, UK", 10.0, 174.0, 330.0),
            line("alice@stanford.edu", 10.0, 186.0, 110.0),
            line("bob@google.com", 10.0, 186.0, 330.0),
            bold(line("Carol Lee", 10.0, 210.0, 120.0)),
            bold(line("Dan O'Brien", 10.0, 210.0, 330.0)),
            line("Meta AI", 10.0, 222.0, 120.0),
            line("Courant Institute", 10.0, 222.0, 330.0),
            bold(line("Abstract", 12.0, 250.0, 280.0)),
        ];
        lines.extend(body(270.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            page.title.as_deref(),
            Some("Scaling Laws for Sparse Mixture Models")
        );
        assert_eq!(
            names(&page),
            pairs(&[
                ("Alice", "Smith"),
                ("Bob", "Jones"),
                ("Carol", "Lee"),
                ("Dan", "O'Brien")
            ])
        );
        assert_eq!(page.authors_confidence, Confidence::High);
    }

    #[test]
    fn thesis_title_page() {
        let lines = vec![
            line("Geometric Methods in Nonequilibrium", 20.0, 150.0, 100.0),
            line("Statistical Mechanics", 20.0, 176.0, 170.0),
            line("by", 12.0, 260.0, 300.0),
            line("Adrianne Zhong", 14.0, 300.0, 250.0),
            line(
                "A dissertation submitted in partial satisfaction of the requirements for the degree of",
                12.0,
                380.0,
                72.0,
            ),
            line("Doctor of Philosophy", 12.0, 400.0, 240.0),
            line("in", 12.0, 420.0, 300.0),
            line("Physics", 12.0, 440.0, 280.0),
            line("in the", 12.0, 460.0, 290.0),
            line("Graduate Division", 12.0, 480.0, 250.0),
            line("of the", 12.0, 500.0, 290.0),
            line("University of California, Berkeley", 12.0, 520.0, 200.0),
        ];
        let page = extract(&lines, W, H);
        assert_eq!(
            page.title.as_deref(),
            Some("Geometric Methods in Nonequilibrium Statistical Mechanics")
        );
        assert_eq!(names(&page), pairs(&[("Adrianne", "Zhong")]));
    }

    #[test]
    fn one_author_then_email() {
        let mut lines = vec![
            bold(line(
                "On the Convergence of Adam and Beyond",
                16.0,
                100.0,
                140.0,
            )),
            line("Sashank J. Reddi", 12.0, 140.0, 240.0),
            line("sashank@google.com", 10.0, 156.0, 240.0),
            line("Mountain View, CA", 10.0, 170.0, 240.0),
        ];
        lines.extend(body(220.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(names(&page), pairs(&[("Sashank J.", "Reddi")]));
        assert_eq!(page.authors_confidence, Confidence::High);
    }

    #[test]
    fn particles_belong_to_the_family_name() {
        let mut lines = vec![
            line("On the Continuity of the Gaseous State", 16.0, 100.0, 140.0),
            line(
                "Johannes van der Waals and Ludwig de Broglie",
                11.0,
                130.0,
                150.0,
            ),
            line("Universiteit Leiden", 9.0, 146.0, 200.0),
        ];
        lines.extend(body(200.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            names(&page),
            pairs(&[("Johannes", "van der Waals"), ("Ludwig", "de Broglie")])
        );
    }

    #[test]
    fn inverted_names_and_hyphenated_given_names() {
        let parsed = parse_names("Dupont, J.-P., Martin, A. B.");
        let got: Vec<_> = parsed
            .names
            .iter()
            .map(|a| (a.given.as_str(), a.family.as_str()))
            .collect();
        assert_eq!(got, [("J.-P.", "Dupont"), ("A. B.", "Martin")]);
        let parsed = parse_names("Jean-Pierre Serre1,2, Wei Xu2");
        let got: Vec<_> = parsed
            .names
            .iter()
            .map(|a| (a.given.as_str(), a.family.as_str()))
            .collect();
        assert_eq!(got, [("Jean-Pierre", "Serre"), ("Wei", "Xu")]);
        assert_eq!(parsed.rejected, 0);
    }

    #[test]
    fn inline_affiliations_cut_the_names() {
        let parsed = parse_names("Alice Smith, Bob Jones, MIT, Cambridge");
        assert_eq!(parsed.names.len(), 2);
        assert!(parsed.affiliation);
        let parsed = parse_names("Alice Smith, Member, IEEE, and Bob Jones, Fellow, IEEE");
        assert_eq!(parsed.names.len(), 2);
        assert_eq!(parsed.rejected, 0);
    }

    #[test]
    fn a_manual_has_no_authors() {
        let mut lines = vec![
            line("Acme Widget 3000 User Manual", 24.0, 120.0, 120.0),
            line("Version 2.3", 14.0, 160.0, 250.0),
            line("Acme Corporation", 12.0, 190.0, 250.0),
            line(
                "Copyright 2021 Acme Corporation. All rights reserved.",
                9.0,
                700.0,
                100.0,
            ),
        ];
        lines.extend(body(260.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(page.title.as_deref(), Some("Acme Widget 3000 User Manual"));
        assert!(page.authors.is_empty());
        assert_eq!(page.authors_confidence, Confidence::Low);
    }

    #[test]
    fn a_slide_deck_has_no_title_confidence() {
        // Landscape, little text, everything large.
        let lines = vec![
            line("Lecture 3: Gradient Descent", 32.0, 120.0, 100.0),
            line(
                "CS 189 Introduction to Machine Learning",
                20.0,
                200.0,
                100.0,
            ),
            line("Jonathan Shewchuk", 20.0, 240.0, 100.0),
        ];
        let page = extract(&lines, 720.0, 540.0);
        assert_eq!(page.title_confidence, Confidence::Low);
        assert_eq!(page.authors_confidence, Confidence::Low);
    }

    #[test]
    fn a_large_journal_banner_is_not_the_title() {
        let mut lines = vec![
            line(
                "Statistics & Probability Letters 8 (1989) 1-5",
                18.0,
                60.0,
                72.0,
            ),
            line(
                "152 Nuclear Instruments and Methods in Physics Research",
                18.0,
                80.0,
                72.0,
            ),
            line(
                "A Central Limit Theorem for Mixing Arrays",
                14.0,
                120.0,
                120.0,
            ),
            line("Magda Peligrad", 11.0, 150.0, 200.0),
            line("University of Cincinnati, OH 45221, USA", 9.0, 166.0, 180.0),
        ];
        lines.extend(body(220.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(
            page.title.as_deref(),
            Some("A Central Limit Theorem for Mixing Arrays")
        );
        assert_eq!(names(&page), pairs(&[("Magda", "Peligrad")]));
    }

    #[test]
    fn running_text_patterns() {
        for banner in [
            "arXiv:1201.4166v2 [cond-mat.stat-mech] 8 Mar 2012",
            "PHYSICAL REVIEW LETTERS 133, 057102 (2024)",
            "JOURNAL OF COMPUTER AND SYSTEM SCIENCES 36, 137-157 (1988)",
            "JOURNAL OF COMPUTER AND SYSTEM SCIENCES 36",
            "Statistics & Probability Letters 8 (1989",
            "152 Nuclear Instruments and Methods in Physics Research",
            "Contents lists available at ScienceDirect",
            "Vol. 12, No. 3",
            "pp. 137-157",
            "https://doi.org/10.1103/PhysRevLett.133.057102",
            "DOI: 10.1016/0022-0000(88)90028-1",
            "ISSN 0022-0000",
            "© 2024 American Physical Society",
            "Copyright 1988 Academic Press, Inc.",
            "Received 12 March 2023; accepted 4 June 2023",
            "Preprint. Under review.",
            "Under review as a conference paper at ICLR 2025",
            "Published as a conference paper at ICLR 2024",
            "Proceedings of the 41st International Conference on Machine Learning",
            "alice@stanford.edu",
            "www.elsevier.com/locate/jcss",
            "137",
            "xii",
            "- 3 -",
        ] {
            assert!(is_running_text(banner), "{banner}");
        }
        for title in [
            "A Theory of Saddle Escape in Deep Nonlinear Networks",
            "Attention Is All You Need",
            "Beyond Linear Response: Equivalence between Thermodynamic Geometry",
            "Review of Particle Physics",
            "Adrianne Zhong and Michael R. DeWeese",
        ] {
            assert!(!is_running_text(title), "{title}");
        }
    }

    #[test]
    fn header_band_drops_small_type_only() {
        let mut lines = vec![
            line("Some Running Head of a Journal Article", 9.0, 20.0, 72.0),
            line("A Short Title Here", 18.0, 30.0, 150.0),
            line("Ann Barker", 11.0, 60.0, 200.0),
        ];
        lines.extend(body(150.0, 10.0));
        let page = extract(&lines, W, H);
        assert_eq!(page.title.as_deref(), Some("A Short Title Here"));
        assert_eq!(names(&page), pairs(&[("Ann", "Barker")]));
    }

    #[test]
    fn title_cleanup() {
        assert_eq!(
            clean_title("Learning  Gaussians in   Polynomial Time*†"),
            "Learning Gaussians in Polynomial Time"
        );
        assert_eq!(clean_title("Deep Linear Networks¹"), "Deep Linear Networks");
        assert_eq!(
            clean_title("ON THE THEORY OF 3D SU(2) GAUGE FIELDS"),
            "On the Theory of 3D SU(2) Gauge Fields"
        );
        assert_eq!(
            clean_title("Mixed CASE Title Stays"),
            "Mixed CASE Title Stays"
        );
    }

    #[test]
    fn non_names_are_rejected() {
        for part in [
            "University of Chicago",
            "applications to learning",
            "March 2024",
            "Deep Learning",
            "Smith",
            "A. B.",
        ] {
            assert!(parse_name(part).is_none(), "{part}");
        }
        let name = parse_name("ALICE B. SMITH").unwrap();
        assert_eq!(
            (name.given.as_str(), name.family.as_str()),
            ("Alice B.", "Smith")
        );
        let name = parse_name("Luc DEVROYE").unwrap();
        assert_eq!(
            (name.given.as_str(), name.family.as_str()),
            ("Luc", "Devroye")
        );
        let name = parse_name("Philip M . Long").unwrap();
        assert_eq!(
            (name.given.as_str(), name.family.as_str()),
            ("Philip M.", "Long")
        );
        assert!(parse_name("UC Berkeley").is_none());
        let name = parse_name("Martin Luther King Jr.").unwrap();
        assert_eq!(name.family, "King");
    }
}

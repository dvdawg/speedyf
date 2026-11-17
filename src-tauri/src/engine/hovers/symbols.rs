//! Where a math symbol is defined: hovering theta_t, a calligraphic L or a
//! bold W previews the sentence or display that introduces it, the way a
//! "Theorem 3" previews the theorem.
//!
//! A symbol is a base letter set in mathematics with what is attached to
//! it: its subscript and superscript and an accent over it. Two symbols are
//! the same when their glyphs are - the PDF character and the font family
//! it is set in, bold or not - so a bold W, a calligraphic W and a plain W
//! are three symbols, and theta_t is another symbol than theta. A power or
//! a transpose (x^2, W^T) is not part of the symbol.
//!
//! A symbol is defined where the prose says so: "let X be", "we denote by
//! X", "where X is the", "X := ...", "define X", "X denotes", "with X the",
//! or a row of a "Notation" section or table. An occurrence points at the
//! nearest such definition before it - a letter a later proof lets be
//! something else points at its newer meaning - or, for a distinctive
//! symbol used before it is defined (in an abstract), at the first one
//! after it. A symbol with none falls back to its base (theta_t to theta,
//! for a Greek, accented or styled letter with a running index) or to a
//! sibling (X_k to "let X_1, ..., X_n be"), and a distinctive symbol
//! (scripted, accented, or in a font of its own) to its first occurrence
//! when that sets it equal to something ("Phi = ..."). A plain single
//! letter - every x and i - points only at a definition the paper states,
//! and only from the next few pages. A wrong definition is worse than
//! none, so every rule leans towards offering nothing.
//!
//! Occurrences are collected page by page as the hover index reads the
//! document (`IndexBuilder::add_page`), keeping only small facts per
//! occurrence and the region of those that could be a definition; the
//! spans are made when the index is finished.
//!
//! PDFium-free.

use super::page::{same_column, Line, Page, Script};
use super::settle;
use super::targets::{line_rect, pad, union};
use super::{HoverSpanDto, HoverTargetDto, Rect};
use std::collections::{HashMap, HashSet};

/// What a symbol is: its glyph and what is attached to it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// The base glyph's character as the PDF gives it (not normalised: a
    /// mathematical bold W is another character than a W).
    pub base: char,
    /// The base glyph's font family: its name, lowercased, without sizes.
    pub family: String,
    pub bold: bool,
    pub accent: Option<&'static str>,
    pub sub: String,
    pub sup: String,
}

impl Key {
    fn scripted(&self) -> bool {
        !self.sub.is_empty() || !self.sup.is_empty()
    }

    /// The symbol without its scripts: what theta_t falls back to.
    pub fn bare(&self) -> Option<Key> {
        self.scripted().then(|| Key {
            sub: String::new(),
            sup: String::new(),
            ..self.clone()
        })
    }

    /// Set apart from a plain letter by more than its font: scripted,
    /// accented, bold or in a style of its own.
    fn marked(&self) -> bool {
        self.scripted()
            || self.accent.is_some()
            || self.bold
            || style_of(&self.family, self.base).is_some()
    }

    /// What a scripted symbol falls back to when nothing defines it: its
    /// bare letter, for a running index on a letter that is a name of its
    /// own (theta_t to theta, cal L_i to cal L) - not for a numbered
    /// instance (h_2) or a Latin letter (w_j, whose w is too often
    /// something else), nor across a prime or star (c' is not c).
    pub fn base_fallback(&self) -> Option<Key> {
        let index = self.sub.chars().count() == 1 && self.sub.chars().all(|c| c.is_alphabetic());
        let named = !self.base.is_ascii()
            || self.accent.is_some()
            || self.bold
            || style_of(&self.family, self.base).is_some();
        (index && self.sup.is_empty() && named)
            .then(|| self.bare())
            .flatten()
    }

    /// For a letter with a simple index (x_1, x_i, x_n): the letter with
    /// its index replaced by a placeholder, which x_1 and x_k share.
    pub fn sibling_family(&self) -> Option<Key> {
        let simple = !self.sub.is_empty()
            && self.sub.chars().count() <= 2
            && self.sub.chars().all(|c| c.is_ascii_alphanumeric())
            && (self.sub.chars().count() == 1 || self.sub.chars().all(|c| c.is_ascii_digit()));
        simple.then(|| Key {
            sub: "#".into(),
            ..self.clone()
        })
    }

    /// An ASCII name for screen readers and the preview's alt text:
    /// "theta_t", "hat x", "cal L", "bold W_k".
    pub fn label(&self) -> String {
        let mut out = String::new();
        if let Some(accent) = self.accent {
            out.push_str(accent);
            out.push(' ');
        }
        if let Some(style) = style_of(&self.family, self.base) {
            out.push_str(style);
            out.push(' ');
        } else if self.bold {
            out.push_str("bold ");
        }
        out.push_str(&ascii_name(self.base));
        if !self.sub.is_empty() {
            out.push('_');
            out.push_str(&ascii_text(&self.sub));
        }
        if self.sup.chars().all(|c| c == '\'') {
            out.push_str(&self.sup);
        } else {
            out.push('^');
            out.push_str(&ascii_text(&self.sup));
        }
        out
    }
}

/// Why an occurrence counts as the symbol's definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// A line of a "Notation" section, paragraph or table.
    Notation,
    /// "let X", "let X be".
    Let,
    /// "denote by X", "denoted X", "X denotes", "X represents".
    Denote,
    /// "define X", "X is defined as", "we write X", "we use X to denote".
    Define,
    /// "where X is", "with X the", "and X is" after one of those.
    Where,
    /// "X := ...", "X \triangleq ...".
    Assign,
}

impl Cue {
    pub fn name(self) -> &'static str {
        match self {
            Cue::Notation => "notation",
            Cue::Let => "let",
            Cue::Denote => "denote",
            Cue::Define => "define",
            Cue::Where => "where",
            Cue::Assign => "assign",
        }
    }
}

/// One occurrence of a symbol on a page.
#[derive(Clone, Debug)]
pub struct Occurrence {
    pub key: Key,
    pub page: u32,
    /// The base and its scripts.
    pub rect: Rect,
    pub cue: Option<Cue>,
    /// What a preview of this occurrence as a definition shows: its
    /// sentence, or its display. Kept only for occurrences that could be
    /// one - those with a cue, and the first of each symbol.
    pub region: Option<Rect>,
    /// Set equal to something where it stands ("X = ...", "f(x) \propto
    /// ..."), heading its own expression: what makes a symbol's first
    /// occurrence its introduction when no words say so.
    pub equated: bool,
}

/// The definition a symbol's occurrences point at.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub page: u32,
    pub rect: Rect,
    pub region: Rect,
    pub cue: Option<Cue>,
    pub via: Via,
}

/// How a definition was found for a symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// A cue on the symbol itself.
    Own,
    /// A cue on the symbol without its scripts (theta for theta_t).
    Base,
    /// A cue on the same letter with another simple index ("let X_1,
    /// ..., X_n be" for X_k).
    Sibling,
    /// No cue: the symbol's first occurrence.
    First,
}

impl Via {
    pub fn name(self) -> &'static str {
        match self {
            Via::Own => "own",
            Via::Base => "base",
            Via::Sibling => "sibling",
            Via::First => "first",
        }
    }
}

/// How many pages after its definition a plain letter's occurrence still
/// points at it: a letter a proof lets be something is that thing for a
/// while, not for the rest of the paper.
const PLAIN_REACH_PAGES: u32 = 3;

/// How many lines a "Notation" section is read for at most.
const NOTATION_SECTION_LINES: u32 = 90;
/// How many lines after a "Notation" table's caption are its rows.
const NOTATION_TABLE_LINES: u32 = 45;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NotationMode {
    #[default]
    Off,
    /// Until the next heading.
    Section,
    /// A run-in "Notation." paragraph: until the paragraph ends.
    Paragraph,
    /// The rows under a table's caption.
    Table,
}

/// Collects occurrences as the index reads the document.
#[derive(Default)]
pub struct Collector {
    occurrences: Vec<Occurrence>,
    seen: HashSet<Key>,
    notation: NotationMode,
    notation_left: u32,
}

impl Collector {
    pub fn new() -> Collector {
        Collector::default()
    }

    pub fn occurrences(&self) -> &[Occurrence] {
        &self.occurrences
    }

    /// Read one page, classified (`regions::classify`): its running heads,
    /// bibliography and contents are skipped.
    pub fn add_page(&mut self, page: &Page, src: u32) {
        let notation = self.notation_lines(page);
        if page.body_font != 0 && is_math_family(page, page.body_font) {
            // Prose set in a math font: nothing tells a symbol apart.
            return;
        }
        let in_display = display_lines(page);
        let accents = accents(page);
        let mut last_line: Option<usize> = None;
        let mut at = 0;
        while at < page.text.len() {
            let Some(found) = base_at(page, at) else {
                at += 1;
                continue;
            };
            let (key, end, powered) = attach(page, at, &accents);
            at = end.max(at + 1);
            let Some(line) = page.line_of.get(found).map(|&l| l as usize) else {
                continue;
            };
            let rect = page
                .rects(found, end)
                .into_iter()
                .next()
                .unwrap_or(page.glyphs[page.raw_of[found]].rect);
            let first_on_line = last_line != Some(line)
                && page.text[page.lines[line].start..found]
                    .iter()
                    .all(|c| c.is_whitespace() || matches!(c, '(' | '\u{2022}' | '-'));
            last_line = Some(line);
            // In notation, a symbol is defined where it heads a row; a plain
            // letter in its prose is only used there.
            let listed = match notation.get(line).copied().unwrap_or_default() {
                NotationMode::Off => false,
                NotationMode::Paragraph => key.marked(),
                _ => first_on_line || key.marked(),
            };
            let cue = if listed {
                Some(Cue::Notation)
            } else {
                cue_at(page, found, end, in_display[line], powered)
            };
            let first = self.seen.insert(key.clone());
            let region = (cue.is_some() || first).then(|| {
                defining_region(page, &in_display, found, end, cue == Some(Cue::Notation))
            });
            let equated = heads(page, found) && equated(&words_after(page, end));
            self.occurrences.push(Occurrence {
                key,
                page: src,
                rect,
                cue,
                region,
                equated,
            });
        }
    }

    /// Which of the page's lines are part of a notation section, paragraph
    /// or table, carrying a section over into the next page.
    fn notation_lines(&mut self, page: &Page) -> Vec<NotationMode> {
        let mut out = vec![NotationMode::Off; page.lines.len()];
        for (i, line) in page.lines.iter().enumerate() {
            if line.excluded() {
                continue;
            }
            let text = page.line_text(i);
            if let Some(mode) = notation_opener(page, i, &text) {
                self.notation = mode;
                self.notation_left = match mode {
                    NotationMode::Table => NOTATION_TABLE_LINES,
                    _ => NOTATION_SECTION_LINES,
                };
                // A run-in heading's own line is notation already.
                if mode == NotationMode::Paragraph {
                    out[i] = mode;
                }
                continue;
            }
            let ends = match self.notation {
                NotationMode::Off => true,
                NotationMode::Section => is_heading(page, i, &text),
                NotationMode::Table => {
                    is_heading(page, i, &text) || text.trim_start().starts_with("Table")
                }
                NotationMode::Paragraph => {
                    !page.continues_previous(i) && !line.is_display() && line.math_share < 0.5
                }
            };
            if ends || self.notation_left == 0 {
                self.notation = NotationMode::Off;
                continue;
            }
            self.notation_left -= 1;
            out[i] = self.notation;
        }
        out
    }

    /// Each occurrence's definition, with the occurrence it is: the
    /// nearest cued occurrence before it (a letter redefined later points
    /// at its newer meaning), else - for a distinctive symbol - the first
    /// cued one after it; then the same through its bare symbol and its
    /// siblings; then a distinctive symbol's first occurrence.
    pub fn resolve(&self) -> Vec<Option<(usize, Definition)>> {
        let dominant = dominant_family(&self.occurrences);
        let mut first: HashMap<&Key, usize> = HashMap::new();
        let mut cued: HashMap<&Key, Vec<usize>> = HashMap::new();
        let mut siblings: HashMap<Key, Vec<usize>> = HashMap::new();
        for (i, o) in self.occurrences.iter().enumerate() {
            first.entry(&o.key).or_insert(i);
            if o.cue.is_some() && o.region.is_some() {
                cued.entry(&o.key).or_default().push(i);
                if let Some(family) = o.key.sibling_family() {
                    siblings.entry(family).or_default().push(i);
                }
            }
        }
        let plain_of = |key: &Key| !distinctive(key, dominant.as_deref());
        // The cued occurrence of `list` that occurrence `i` points at.
        let pick = |list: &[usize], i: usize, plain: bool| -> Option<usize> {
            let before = list.partition_point(|&c| c <= i);
            if before > 0 {
                let c = list[before - 1];
                let (o, d) = (&self.occurrences[i], &self.occurrences[c]);
                let near = o.page <= d.page + PLAIN_REACH_PAGES;
                if !plain || near || d.cue == Some(Cue::Notation) {
                    return Some(c);
                }
                return None;
            }
            (!plain).then(|| list.first().copied()).flatten()
        };
        let to_definition = |c: usize, via: Via| -> Option<(usize, Definition)> {
            let o = &self.occurrences[c];
            Some((
                c,
                Definition {
                    page: o.page,
                    rect: o.rect,
                    region: o.region?,
                    cue: o.cue,
                    via,
                },
            ))
        };
        self.occurrences
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let key = &o.key;
                let plain = plain_of(key);
                if let Some(c) = cued.get(key).and_then(|l| pick(l, i, plain)) {
                    return to_definition(c, Via::Own);
                }
                if let Some(bare) = key.base_fallback() {
                    let bare_plain = plain_of(&bare);
                    let said = |c: &usize| {
                        matches!(
                            self.occurrences[*c].cue,
                            Some(Cue::Let | Cue::Denote | Cue::Notation | Cue::Where)
                        )
                    };
                    if let Some(c) = cued
                        .get(&bare)
                        .and_then(|l| pick(l, i, bare_plain))
                        .filter(said)
                    {
                        return to_definition(c, Via::Base);
                    }
                }
                if let Some(family) = key.sibling_family() {
                    if let Some(c) = siblings.get(&family).and_then(|l| pick(l, i, plain)) {
                        if self.occurrences[c].key != *key {
                            return to_definition(c, Via::Sibling);
                        }
                    }
                }
                let at = *first.get(key)?;
                if !plain && self.occurrences[at].equated {
                    return to_definition(at, Via::First);
                }
                None
            })
            .collect()
    }

    /// The hover spans: every occurrence of a symbol with a definition,
    /// but the definition itself and any inside what it previews.
    pub fn spans(&self) -> HashMap<u32, Vec<HoverSpanDto>> {
        let resolved = self.resolve();
        let mut out: HashMap<u32, Vec<HoverSpanDto>> = HashMap::new();
        for (i, o) in self.occurrences.iter().enumerate() {
            let Some((at, d)) = &resolved[i] else {
                continue;
            };
            if *at == i || (o.page == d.page && centre_in(o.rect, d.region)) {
                continue;
            }
            out.entry(o.page).or_default().push(HoverSpanDto {
                kind: "symbol",
                rects: vec![o.rect],
                label: o.key.label(),
                target: HoverTargetDto {
                    page: d.page,
                    x: Some(d.region[0]),
                    y: d.region[1] + d.region[3],
                    region: Some(d.region),
                    continues: false,
                    mark: Some(d.rect),
                    terms: Vec::new(),
                },
            });
        }
        out
    }
}

pub(super) fn centre_in(r: Rect, region: Rect) -> bool {
    let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    cx >= region[0] && cx <= region[0] + region[2] && cy >= region[1] && cy <= region[1] + region[3]
}

/// A font's family: its name lowercased, without sizes ("CMMI10" and
/// "CMMI7" are one family), or its page-local id when the page has no
/// names.
fn family(page: &Page, font: u16) -> String {
    match page.font_name(font) {
        Some(name) if !name.is_empty() => name
            .chars()
            .filter(|c| !c.is_ascii_digit())
            .flat_map(char::to_lowercase)
            .collect(),
        _ => format!("#{font}"),
    }
}

fn is_math_family(page: &Page, font: u16) -> bool {
    page.glyphs.iter().any(|g| g.font == font && g.math)
}

/// A family a variable is set in for a reason of its own: script,
/// calligraphic, blackboard or fraktur.
fn style_of(family: &str, base: char) -> Option<&'static str> {
    let code = base as u32;
    if family.contains("msbm") || family.contains("bbold") || family.contains("dsrom") {
        return Some("bb");
    }
    if (0x1D538..=0x1D56B).contains(&code)
        || "\u{2102}\u{210D}\u{2115}\u{2119}\u{211A}\u{211D}\u{2124}".contains(base)
    {
        return Some("bb");
    }
    if family.contains("eufm") || family.contains("frak") || (0x1D504..=0x1D537).contains(&code) {
        return Some("frak");
    }
    if family.contains("cmsy")
        || family.contains("rsfs")
        || family.contains("eusm")
        || family.contains("script")
        || family.contains("calligra")
        || (0x1D49C..=0x1D4CF).contains(&code)
    {
        return Some("cal");
    }
    None
}

const GREEK: &[(char, &str)] = &[
    ('\u{03B1}', "alpha"),
    ('\u{03B2}', "beta"),
    ('\u{03B3}', "gamma"),
    ('\u{03B4}', "delta"),
    ('\u{03B5}', "epsilon"),
    ('\u{03F5}', "epsilon"),
    ('\u{03B6}', "zeta"),
    ('\u{03B7}', "eta"),
    ('\u{03B8}', "theta"),
    ('\u{03D1}', "vartheta"),
    ('\u{03B9}', "iota"),
    ('\u{03BA}', "kappa"),
    ('\u{03BB}', "lambda"),
    ('\u{03BC}', "mu"),
    ('\u{03BD}', "nu"),
    ('\u{03BE}', "xi"),
    ('\u{03BF}', "omicron"),
    ('\u{03C0}', "pi"),
    ('\u{03D6}', "varpi"),
    ('\u{03C1}', "rho"),
    ('\u{03F1}', "varrho"),
    ('\u{03C2}', "varsigma"),
    ('\u{03C3}', "sigma"),
    ('\u{03C4}', "tau"),
    ('\u{03C5}', "upsilon"),
    ('\u{03C6}', "phi"),
    ('\u{03D5}', "phi"),
    ('\u{03C7}', "chi"),
    ('\u{03C8}', "psi"),
    ('\u{03C9}', "omega"),
    ('\u{0393}', "Gamma"),
    ('\u{0394}', "Delta"),
    ('\u{0398}', "Theta"),
    ('\u{039B}', "Lambda"),
    ('\u{039E}', "Xi"),
    ('\u{03A0}', "Pi"),
    ('\u{03A3}', "Sigma"),
    ('\u{03A5}', "Upsilon"),
    ('\u{03A6}', "Phi"),
    ('\u{03A8}', "Psi"),
    ('\u{03A9}', "Omega"),
];

/// A letter as ASCII: itself, a Greek letter's name, a mathematical
/// alphanumeric's plain letter.
fn ascii_name(c: char) -> String {
    use unicode_normalization::UnicodeNormalization;
    if c.is_ascii() {
        return c.to_string();
    }
    let plain: Vec<char> = std::iter::once(c).nfkc().collect();
    if let [p] = plain.as_slice() {
        if p.is_ascii() {
            return p.to_string();
        }
        if let Some((_, name)) = GREEK.iter().find(|(g, _)| g == p) {
            return (*name).to_string();
        }
    }
    String::new()
}

fn ascii_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if matches!(c, '\u{22C6}' | '\u{2217}' | '\u{2605}') {
                "*".to_string()
            } else if c.is_ascii_graphic() {
                c.to_string()
            } else {
                ascii_name(c)
            }
        })
        .collect()
}

/// Whether the character at `at` starts a symbol: a letter set in a math
/// font (or a mathematical alphanumeric), at the line's own level, not
/// part of a word. A lone bold letter in running text (`\mathbf{W}` is
/// set in the text's bold face) counts too.
fn base_at(page: &Page, at: usize) -> Option<usize> {
    let raw = *page.raw_of.get(at)?;
    let g = page.glyph_at(at)?;
    if at > 0 && page.raw_of[at - 1] == raw {
        return None;
    }
    if page.script[at] != Script::Normal || !g.ch.is_alphabetic() || !g.has_box() {
        return None;
    }
    let line = page.line_at(at)?;
    if line.excluded() {
        return None;
    }
    let math_alnum = (0x1D400..=0x1D7FF).contains(&(g.ch as u32));
    if !(g.math || math_alnum || (g.bold && lone_bold(page, at))) {
        return None;
    }
    // Big operators: a sum or integral sign whose font maps it to a
    // letter ("P", "X", "Z") is no variable.
    let fam = family(page, g.font);
    if ["cmex", "esint", "mtex", "stmary", "msam", "extension"]
        .iter()
        .any(|f| fam.contains(f))
    {
        return None;
    }
    // A differential: "dx", "du".
    if g.ch == 'd'
        && page
            .text
            .get(at + 1)
            .is_some_and(|c| c.is_alphabetic() && page.script[at + 1] == Script::Normal)
    {
        return None;
    }
    if in_word(page, at) {
        return None;
    }
    Some(at)
}

fn is_math_char(page: &Page, at: usize) -> bool {
    page.glyph_at(at).is_some_and(|g| {
        g.math || (0x1D400..=0x1D7FF).contains(&(g.ch as u32)) || super::page::is_math_symbol(g.ch)
    })
}

/// A math letter set against other letters with no space: a word typed in
/// math ("softmax"), or prose. Two letters side by side are a product
/// ("Wx"); three or more are read as a word.
fn in_word(page: &Page, at: usize) -> bool {
    let letter = |k: usize| {
        page.text[k].is_alphabetic()
            && page.script[k] == Script::Normal
            && page.raw_of[k] != super::page::NO_RAW
    };
    let mut run = 1;
    let mut k = at;
    while k > 0 && letter(k - 1) {
        k -= 1;
        if !is_math_char(page, k) {
            return true;
        }
        run += 1;
    }
    let mut k = at + 1;
    while k < page.text.len() && letter(k) {
        if !is_math_char(page, k) {
            return true;
        }
        run += 1;
        k += 1;
    }
    // Two capitals set close ("OPT", "ID"): an acronym's letters.
    let capital = |k: usize| page.text.get(k).is_some_and(|c| c.is_uppercase());
    let pair = run == 2
        && capital(at)
        && ((at > 0 && letter(at - 1) && capital(at - 1)) || (letter(at + 1) && capital(at + 1)));
    run >= 3 || pair
}

/// A bold letter standing alone in a line that is not itself bold: no
/// letter either side, not a run-in label ("A." opening a line).
fn lone_bold(page: &Page, at: usize) -> bool {
    // Bold by its font's name, not only by a weight PDFium guesses.
    let named = page
        .glyph_at(at)
        .and_then(|g| page.font_name(g.font))
        .is_none_or(|n| n.is_empty() || super::extract::bold_name(n));
    if !named {
        return false;
    }
    let text = &page.text;
    let alone_before = at == 0 || !text[at - 1].is_alphanumeric();
    let alone_after = at + 1 >= text.len() || !text[at + 1].is_alphanumeric();
    if !(alone_before && alone_after) {
        return false;
    }
    let Some(line) = page.line_at(at) else {
        return false;
    };
    if page.all_bold(line.start, line.end) {
        return false;
    }
    // Letter-spaced text ("F i r s t"): every letter stands alone.
    let line_text: String = text[line.start..line.end].iter().collect();
    let words: Vec<&str> = line_text.split_whitespace().collect();
    let single = words.iter().filter(|w| w.chars().count() == 1).count();
    if single * 4 > words.len() {
        return false;
    }
    let bold_share = {
        let letters: Vec<bool> = (line.start..line.end)
            .filter(|&k| text[k].is_alphanumeric())
            .filter_map(|k| page.glyph_at(k).map(|g| g.bold))
            .collect();
        letters.iter().filter(|b| **b).count() as f32 / letters.len().max(1) as f32
    };
    if bold_share > 0.3 {
        return false;
    }
    // "A." or "(a)" at the start of a line: a label.
    let opens_line = text[line.start..at]
        .iter()
        .all(|c| c.is_whitespace() || *c == '(');
    let label_after = matches!(text.get(at + 1), Some('.' | ')' | ':'));
    !(opens_line && label_after)
}

/// Accent glyphs on the page: their horizontal centre, bottom and name.
fn accents(page: &Page) -> Vec<(f32, f32, &'static str)> {
    page.glyphs
        .iter()
        .filter(|g| g.has_box())
        .filter_map(|g| {
            let name = match g.ch {
                '\u{02C6}' | '^' | '\u{0302}' => "hat",
                '\u{02DC}' | '~' | '\u{0303}' => "tilde",
                '\u{00AF}' | '\u{02C9}' | '\u{0304}' | '\u{0305}' => "bar",
                '\u{02D9}' | '\u{0307}' => "dot",
                '\u{00A8}' | '\u{0308}' => "ddot",
                '\u{02C7}' | '\u{030C}' => "check",
                '\u{02D8}' | '\u{0306}' => "breve",
                '\u{20D7}' | '\u{2192}' => "vec",
                _ => return None,
            };
            Some((g.rect[0] + g.rect[2] / 2.0, g.rect[1], name))
        })
        .collect()
}

/// The symbol starting at `at`, where its scripts end, and whether a
/// power (or other exponent) was taken of it.
fn attach(page: &Page, at: usize, accents: &[(f32, f32, &'static str)]) -> (Key, usize, bool) {
    let g = &page.glyphs[page.raw_of[at]];
    let [x, y, w, h] = g.rect;
    let em = g.size.max(h).max(1.0);
    let accent = accents
        .iter()
        .find(|(cx, bottom, name)| {
            // An arrow is a vector accent only when small and over the letter.
            let small = *name != "vec" || w > 0.0;
            small
                && *cx >= x - 0.15 * w
                && *cx <= x + 1.15 * w
                && *bottom >= y + 0.45 * h
                && *bottom <= y + h + 0.5 * em
        })
        .map(|(_, _, name)| *name);
    let raw = page.raw_of[at];
    let mut end = at + 1;
    while end < page.text.len() && page.raw_of[end] == raw {
        end += 1;
    }
    let mut sub = String::new();
    let mut sup = String::new();
    // A prime set level with the letter.
    while end < page.text.len() && page.text[end] == '\'' {
        sup.push('\'');
        end += 1;
    }
    while end < page.text.len() && page.text[end] != ' ' && page.script[end] != Script::Normal {
        let c = page.text[end];
        match page.script[end] {
            Script::Sub => sub.push(c),
            _ => sup.push(c),
        }
        end += 1;
    }
    // A power, a transpose or an exponent is not part of the symbol; a
    // star, a prime or a bracketed index ("x^(k)") is.
    // A label in letters ("N^hyp") is too.
    let marks = |c: char| matches!(c, '\'' | '*' | '\u{22C6}' | '\u{2217}');
    let bracketed = sup.starts_with('(') && sup.ends_with(')') && sup.chars().count() <= 5;
    let named =
        (2..=5).contains(&sup.chars().count()) && sup.chars().all(|c| c.is_ascii_alphabetic());
    let powered = !(sup.chars().all(marks) || bracketed || named);
    if powered {
        sup.clear();
    }
    // An index runs from a letter or digit to one ("t+1", "i,j"); what
    // starts otherwise is no index, what trails is punctuation.
    if !sub.starts_with(|c: char| c.is_alphanumeric()) {
        sub.clear();
    }
    while sub.ends_with(|c: char| !c.is_alphanumeric() && c != '\'' && c != ')') {
        sub.pop();
    }
    let cap = |s: &mut String| {
        if s.chars().count() > 12 {
            *s = s.chars().take(12).collect();
        }
    };
    cap(&mut sub);
    cap(&mut sup);
    (
        Key {
            base: g.ch,
            family: family(page, g.font),
            bold: page
                .font_name(g.font)
                .filter(|n| !n.is_empty())
                .map_or(g.bold, super::extract::bold_name),
            accent,
            sub,
            sup,
        },
        end,
        powered,
    )
}

/// The text before `at` back to the start of its sentence (at most a few
/// lines), lowercased.
fn sentence_before(page: &Page, at: usize) -> String {
    let from = sentence_start(page, at, None);
    page.text[from..at]
        .iter()
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Whether `text[k]` ends a sentence: a full stop, question or exclamation
/// mark followed by a space and a capital, not an abbreviation's stop.
fn ends_sentence(text: &[char], k: usize) -> bool {
    if !matches!(text[k], '.' | '?' | '!') {
        return false;
    }
    if text.get(k + 1) != Some(&' ') {
        return k + 1 >= text.len();
    }
    if !text
        .get(k + 2)
        .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
    {
        return false;
    }
    let word: String = text[..k]
        .iter()
        .rev()
        .take_while(|c| c.is_alphabetic() || **c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
        .to_lowercase();
    !matches!(
        word.as_str(),
        "e.g"
            | "i.e"
            | "fig"
            | "figs"
            | "eq"
            | "eqs"
            | "cf"
            | "al"
            | "resp"
            | "sec"
            | "thm"
            | "vs"
            | "no"
            | "ref"
            | "refs"
            | "def"
            | "prop"
            | "lem"
            | "cor"
            | "approx"
            | "ch"
            | "p"
    )
}

/// Where the sentence holding `at` starts: after the last sentence end
/// before it, or the first line of its paragraph within reach, never
/// across a display or into another column.
fn sentence_start(page: &Page, at: usize, in_display: Option<&[bool]>) -> usize {
    let own = page.line_of[at] as usize;
    let mut k = at;
    let limit = at.saturating_sub(320);
    while k > limit {
        let line = page.line_of[k - 1] as usize;
        if line != own {
            let l = &page.lines[line];
            let next = page.line_of[k] as usize;
            if line + 3 < own
                || !same_column(l, &page.lines[own])
                || in_display.is_some_and(|d| d[line] && !d[own])
                || l.excluded()
                || (next != line && !page.continues_previous(next))
            {
                break;
            }
        }
        if ends_sentence(&page.text, k - 1) {
            break;
        }
        k -= 1;
    }
    while k < at && page.text[k] == ' ' {
        k += 1;
    }
    k
}

/// Where the sentence holding `end` ends, likewise.
fn sentence_end(page: &Page, end: usize, in_display: &[bool]) -> usize {
    let own = page.line_of[end.saturating_sub(1)] as usize;
    let mut k = end;
    let limit = (end + 320).min(page.text.len());
    while k < limit {
        let line = page.line_of[k] as usize;
        if line != own {
            let l = &page.lines[line];
            let previous = page.line_of[k - 1] as usize;
            if line > own + 3
                || !same_column(l, &page.lines[own])
                || (in_display[line] && !in_display[own])
                || l.excluded()
                || (previous != line && !page.continues_previous(line))
            {
                break;
            }
        }
        if ends_sentence(&page.text, k) {
            return k + 1;
        }
        k += 1;
    }
    k
}

/// Which lines are part of a display equation.
fn display_lines(page: &Page) -> Vec<bool> {
    let mut v = vec![false; page.lines.len()];
    for d in 0..page.displays.len() {
        for i in settle::members(page, d) {
            v[i] = true;
        }
    }
    for (i, l) in page.lines.iter().enumerate() {
        // A display the detector missed: a line of mathematics set apart.
        if l.is_display() && l.math_share >= 0.6 {
            v[i] = true;
        }
    }
    v
}

/// What an occurrence previews as a definition: its display, or the
/// sentence it is in, settled whole.
fn defining_region(page: &Page, in_display: &[bool], at: usize, end: usize, row: bool) -> Rect {
    let own = page.line_of[at] as usize;
    if row {
        // A row of a notation list: the row.
        return settle::settle(page, pad(line_rect(&page.lines[own]), 1.0, page));
    }
    if let Some(d) = settle::display_of(page, own) {
        return settle::settle(page, pad(page.displays[d], 1.0, page));
    }
    if in_display[own] {
        return settle::settle(page, pad(line_rect(&page.lines[own]), 1.0, page));
    }
    let from = sentence_start(page, at, Some(in_display));
    let to = sentence_end(page, end, in_display);
    let first = page.line_of[from] as usize;
    let last = page.line_of[to.saturating_sub(1).max(from)] as usize;
    let mut rect = line_rect(&page.lines[own]);
    for i in first.min(own)..=last.max(own) {
        let l = &page.lines[i];
        if l.excluded() || !same_column(l, &page.lines[own]) || l.end <= l.start {
            continue;
        }
        if i + 2 < own || i > own + 3 {
            continue;
        }
        rect = union(rect, line_rect(l));
    }
    settle::settle(page, pad(rect, 1.0, page))
}

/// The words after an occurrence: each lowercased, and whether it is
/// mathematics (any character of it set in a math font or a math sign).
fn words_after(page: &Page, end: usize) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut word = String::new();
    let mut math = false;
    let limit = (end + 90).min(page.text.len());
    for k in end..limit {
        let c = page.text[k];
        if c == ' ' {
            if !word.is_empty() {
                out.push((std::mem::take(&mut word), math));
            }
            math = false;
            if out.len() >= 8 {
                break;
            }
            continue;
        }
        if is_math_char(page, k) || page.script[k] != Script::Normal {
            math = true;
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() && out.len() < 8 {
        out.push((word, math));
    }
    out
}

/// Whether the occurrence at `at..end` is said to be defined there.
fn cue_at(page: &Page, at: usize, end: usize, displayed: bool, powered: bool) -> Option<Cue> {
    let before = sentence_before(page, at);
    let after = words_after(page, end);
    // "f(X) := ..." defines f, not X; "Si denotes" (an index the page
    // set level) is about S.
    if !heads(page, at) {
        return None;
    }
    // A letter glued on after it ("pa", an index the page set level) or
    // a power taken of it ("D^2n denote"): the words after are not about
    // the symbol, only those before are.
    let glued = page.text.get(end).is_some_and(|c| c.is_alphanumeric());
    if glued || powered {
        return cue(&before, &[], displayed);
    }
    cue(&before, &after, displayed)
}

/// Whether the symbol at `at` heads what follows it: not an argument or a
/// list item ("f(x", ", x") nor a subscript's piece.
fn heads(page: &Page, at: usize) -> bool {
    if at > 0 && page.text[at - 1].is_alphanumeric() {
        return false;
    }
    let before = page.text[..at].iter().rev().find(|c| **c != ' ');
    !matches!(before, Some('(' | ',' | '[' | '{' | '|' | '_' | '^'))
}

/// Whether the words after a symbol set it equal to something: "= ...",
/// "\propto ...", "\sim ...", ":= ...", past an argument list ("(x) =").
fn equated(after: &[(String, bool)]) -> bool {
    let opens = |w: &str| {
        w.starts_with([
            '=', '\u{221D}', '\u{223C}', '\u{2261}', '\u{2254}', '\u{225C}',
        ]) || w.starts_with(":=")
    };
    match after.first() {
        Some((w, _)) if opens(w) => true,
        Some((w, _)) if w.starts_with('(') => {
            let tail = w
                .trim_start_matches(|c: char| c != ')')
                .trim_start_matches(')');
            opens(tail) || after.get(1).is_some_and(|(n, _)| opens(n))
        }
        _ => false,
    }
}

/// Whether "where X <next> <second>" says what X is: "where X is the",
/// "where X denotes" - not "where X is fixed".
fn says_what(next: &str, second: &str) -> bool {
    match next {
        "denotes" | "denote" | "represents" | "represent" | "stands" | "stand" | "indicates"
        | "indicate" | "refers" | "refer" | "corresponds" | "correspond" | "means" => true,
        "is" | "are" | "be" | "being" => matches!(
            second,
            "the"
                | "a"
                | "an"
                | "our"
                | "its"
                | "their"
                | "defined"
                | "given"
                | "called"
                | "drawn"
                | "sampled"
                | "chosen"
                | "some"
                | "any"
                | "each"
        ),
        _ => false,
    }
}

/// Whether what follows "let X" introduces X: "be", "denote", "=", an
/// element sign, a colon (a map), a comma (a list) - not "let x <= y".
fn introduces(after: &[(String, bool)]) -> bool {
    let Some((w, _)) = after.first() else {
        return false;
    };
    let w = w.as_str();
    if matches!(w, "be" | "denote" | "denotes" | "and" | "represent") {
        return true;
    }
    let opens = |w: &str| {
        w.starts_with([
            '=', ',', ':', '\u{2208}', '\u{223C}', '\u{2286}', '\u{2282}', '\u{2254}', '\u{225C}',
        ])
    };
    if opens(w) {
        return true;
    }
    // "let f(x) = ..."
    w.starts_with('(') && after.get(1).is_some_and(|(n, _)| opens(n))
}

/// Read the words around an occurrence. `before` is its sentence up to it,
/// lowercased; `after` the words after it, each marked whether it is
/// mathematics.
pub fn cue(before: &str, after: &[(String, bool)], displayed: bool) -> Option<Cue> {
    // "X := ...", "X(t) := ...", "X \triangleq ..."
    let assign = |w: &str| {
        let split = w == ":" && after.get(1).is_some_and(|(n, _)| n.starts_with('='));
        split
            || w.starts_with(":=")
            || w.starts_with('\u{225C}')
            || w.starts_with('\u{225D}')
            || w.starts_with('\u{2254}')
    };
    for (i, (w, _)) in after.iter().take(2).enumerate() {
        let tail = w.trim_start_matches(|c: char| {
            c != ':' && c != '\u{225C}' && c != '\u{225D}' && c != '\u{2254}'
        });
        if assign(w) || (i == 0 && !tail.is_empty() && assign(tail)) {
            return Some(Cue::Assign);
        }
    }
    if displayed {
        // A display's words are mathematics; only an assignment says it
        // defines.
        return None;
    }
    // The words after, past a little mathematics ("where X \in R^d is").
    let mut rest = after
        .iter()
        .map(|(w, m)| (w.trim_end_matches([',', '.', ';', ':']), *m));
    let mut skipped = 0;
    let mut next_word = None;
    for (w, math) in rest.by_ref() {
        // A letter glued on ("where N hyp", "S i") is part of the symbol
        // the page could not read, not mathematics to look past.
        let glued = skipped == 0 && w.starts_with(char::is_alphabetic);
        if !glued && (math || w.is_empty() || !w.chars().any(char::is_alphabetic)) {
            skipped += 1;
            if skipped > 3 {
                break;
            }
            continue;
        }
        next_word = Some(w.to_string());
        break;
    }
    let second = rest.next().map(|(w, _)| w.to_string()).unwrap_or_default();
    let third = rest.next().map(|(w, _)| w.to_string()).unwrap_or_default();
    let next = next_word.as_deref().unwrap_or("");
    let first_after = after.first().map(|(w, _)| w.as_str()).unwrap_or("");
    let comma_after = first_after.ends_with(',');

    let words: Vec<&str> = before.split_whitespace().collect();
    let last = words.last().copied().unwrap_or("");
    let last2 = if words.len() >= 2 {
        format!("{} {}", words[words.len() - 2], last)
    } else {
        String::new()
    };
    // A word right before the symbol, with no punctuation in between.
    let joined = before.trim_end().ends_with(|c: char| c.is_alphabetic());
    // "the loss, denoted L," "is called X", "we denote by X"
    if joined {
        if last == "let" && introduces(after) {
            return Some(Cue::Let);
        }
        if matches!(
            last2.as_str(),
            "denote by" | "denoted by" | "denoted as" | "denote as" | "written as" | "write as"
        ) || matches!(last, "denoted" | "called" | "termed" | "dubbed")
        {
            return Some(Cue::Denote);
        }
        if matches!(last, "define" | "defines") {
            return Some(Cue::Define);
        }
        // "we write X for ...", "we write X = ..."
        if last2 == "we write" && (next == "for" && skipped == 0 || introduces(after)) {
            return Some(Cue::Define);
        }
        if last == "use"
            && matches!(next, "to" | "for")
            && matches!(second.as_str(), "denote" | "represent" | "the" | "")
        {
            return Some(Cue::Define);
        }
        if last == "where" && says_what(next, &second) {
            return Some(Cue::Where);
        }
        if last == "with"
            && matches!(
                next,
                "the" | "a" | "an" | "being" | "denoting" | "representing"
            )
            && !comma_after
        {
            return Some(Cue::Where);
        }
        if last == "and" && says_what(next, &second) {
            // "where X is ... and Y is ...": the and must continue a
            // "where" or "let" of the same sentence.
            let opened = words.iter().any(|w| matches!(*w, "where" | "let"));
            if opened {
                return Some(Cue::Where);
            }
        }
    }
    // "X denotes ...", "X is defined as ...", "X is called"
    // ... with the verb right after the symbol, or after its arguments.
    let direct =
        skipped == 0 || (skipped == 1 && after.first().is_some_and(|(w, _)| w.starts_with('(')));
    if direct {
        match (next, second.as_str()) {
            ("denotes" | "represents" | "stands", _) => return Some(Cue::Denote),
            ("denote" | "represent", _) if !before.trim_end().ends_with("to") => {
                return Some(Cue::Denote)
            }
            ("is" | "are", "defined")
                if matches!(
                    third.as_str(),
                    "as" | "by"
                        | "to"
                        | "via"
                        | "through"
                        | "for"
                        | "in"
                        | "on"
                        | "similarly"
                        | "analogously"
                        | "below"
                        | "above"
                        | "recursively"
                        | "using"
                        | "with"
                ) =>
            {
                return Some(Cue::Define)
            }
            ("is" | "are", "called" | "termed") => return Some(Cue::Define),
            ("will" | "shall", "denote" | "represent") => return Some(Cue::Denote),
            ("refers", "to") => return Some(Cue::Denote),
            _ => {}
        }
    }
    None
}

/// Whether a line opens notation: a heading "Notation" (numbered or not),
/// a run-in "Notation." paragraph, a table captioned with notation.
fn notation_opener(page: &Page, i: usize, text: &str) -> Option<NotationMode> {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let numbered = words.first().is_some_and(|w| {
        w.trim_end_matches('.')
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
            || (w.len() <= 2 && w.ends_with('.'))
    });
    let body: Vec<&str> = if numbered && words.len() > 1 {
        words[1..].to_vec()
    } else {
        words.clone()
    };
    let head = body.join(" ");
    let head = head.trim_end_matches([':', '.']);
    const HEADS: &[&str] = &[
        "notation",
        "notations",
        "nomenclature",
        "list of symbols",
        "symbols",
        "notation and preliminaries",
        "preliminaries and notation",
        "preliminaries and notations",
        "notations and preliminaries",
        "notation and definitions",
        "definitions and notation",
        "notation and setup",
        "setup and notation",
        "notation and background",
        "background and notation",
        "symbols and notation",
        "summary of notation",
        "table of notation",
        "table of notations",
        "basic notation",
        "general notation",
        "mathematical notation",
    ];
    if trimmed.chars().count() <= 48 && HEADS.contains(&head) && !page.continues_previous(i) {
        return Some(NotationMode::Section);
    }
    // A run-in "Notation." or "Notations:" opening a paragraph.
    let first = body.first().copied().unwrap_or("");
    if matches!(
        first,
        "notation." | "notations." | "notation:" | "notations:"
    ) && body.len() > 2
        && !page.continues_previous(i)
    {
        return Some(NotationMode::Paragraph);
    }
    // "Table 1: Notation", "Table 2. Summary of notation used".
    if words.first() == Some(&"table")
        && words.len() <= 12
        && words
            .iter()
            .any(|w| w.starts_with("notation") || w.starts_with("symbol"))
    {
        return Some(NotationMode::Table);
    }
    None
}

/// Whether a line is a heading that ends a notation section: a numbered
/// heading, or a short line in bold, with no mathematics in it.
fn is_heading(page: &Page, i: usize, text: &str) -> bool {
    let l: &Line = &page.lines[i];
    if l.math_count > 0 || page.continues_previous(i) || text.chars().count() > 70 {
        return false;
    }
    let trimmed = text.trim();
    let mut words = trimmed.split_whitespace();
    let first = words.next().unwrap_or("");
    let numbered = first.chars().next().is_some_and(|c| c.is_ascii_digit())
        && first
            .trim_end_matches('.')
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
        && words
            .next()
            .is_some_and(|w| w.chars().next().is_some_and(char::is_uppercase));
    let bold = page.all_bold(l.start, l.end) && trimmed.split_whitespace().count() <= 8;
    bold || (numbered && super::regions::styled_apart(page, i))
}

/// The family most variables are set in: what a plain letter is.
fn dominant_family(occurrences: &[Occurrence]) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for o in occurrences {
        if !o.key.scripted() && o.key.base.is_ascii_lowercase() {
            *counts.entry(o.key.family.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(f, _)| f.to_string())
}

/// Whether a symbol is distinctive enough that its first appearance is
/// where it is introduced: scripted, accented, bold, or in a style of its
/// own (calligraphic, fraktur) - not a plain letter, not one of the
/// standard sets (R, N, E for expectation...).
fn distinctive(key: &Key, dominant: Option<&str>) -> bool {
    let style = style_of(&key.family, key.base);
    if style == Some("bb") {
        return false;
    }
    // Big O.
    if style == Some("cal") && matches!(key.base, 'O') && !key.scripted() {
        return false;
    }
    let plain_family = dominant.is_none_or(|d| d == key.family);
    let plain =
        !key.scripted() && key.accent.is_none() && !key.bold && plain_family && style.is_none();
    if plain {
        return false;
    }
    // A plain letter with only a numeric or single-letter index (x_1, x_i,
    // a_n) is an element of a sequence, not something introduced.
    if key.accent.is_none() && !key.bold && plain_family && style.is_none() && key.sup.is_empty() {
        let simple = key.sub.chars().count() == 1;
        if simple {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::*;

    fn words(text: &str, math: &[usize]) -> Vec<(String, bool)> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| (w.to_string(), math.contains(&i)))
            .collect()
    }

    #[test]
    fn cues_read_the_words_around_a_symbol() {
        let none: &[usize] = &[];
        assert_eq!(
            cue("let", &words("be the parameters", none), false),
            Some(Cue::Let)
        );
        assert_eq!(
            cue("we denote by", &words("the loss", none), false),
            Some(Cue::Denote)
        );
        assert_eq!(
            cue("the loss, denoted", &words(", is", none), false),
            Some(Cue::Denote)
        );
        assert_eq!(
            cue("where", &words("is the step size", none), false),
            Some(Cue::Where)
        );
        assert_eq!(
            cue("where", &words("\u{2208} r^d is the input", &[0, 1]), false),
            Some(Cue::Where)
        );
        assert_eq!(
            cue("with", &words("the learning rate", none), false),
            Some(Cue::Where)
        );
        assert_eq!(
            cue("we define", &words("as", none), false),
            Some(Cue::Define)
        );
        assert_eq!(cue("", &words(":= e[x]", &[1]), true), Some(Cue::Assign));
        assert_eq!(
            cue("", &words("denotes the set", none), false),
            Some(Cue::Denote)
        );
        assert_eq!(
            cue("", &words("is defined as", none), false),
            Some(Cue::Define)
        );
        assert_eq!(
            cue(
                "where x is the input and",
                &words("is the output", none),
                false
            ),
            Some(Cue::Where)
        );
    }

    #[test]
    fn plain_mentions_are_not_cues() {
        let none: &[usize] = &[];
        assert_eq!(cue("since", &words("is bounded", none), false), None);
        assert_eq!(cue("then", &words("= 0", &[0, 1]), false), None);
        assert_eq!(cue("and", &words("is positive", none), false), None);
        assert_eq!(cue("we use", &words("in the proof", none), false), None);
        // An equals sign in a display is not an assignment.
        assert_eq!(cue("", &words("= a + b", &[0, 1, 2, 3]), true), None);
        // "where" with no verb after it.
        assert_eq!(cue("where", &words("\u{2192} 0", &[0, 1]), false), None);
    }

    /// A page: prose with math letters set in font 4 (MATH), scripts in
    /// font 7 (a smaller math font).
    const MSCRIPT: Style = Style {
        size: 7.0,
        font: 7,
        bold: false,
        math: true,
    };

    /// Prose then a math symbol with an optional subscript, then prose; the
    /// x after the line.
    fn sym(b: &mut Builder, x: f32, y: f32, base: &str, sub: Option<&str>) -> f32 {
        let x = b.text(x, y, MATH, base);
        match sub {
            Some(s) => b.text(x, y - 2.5, MSCRIPT, s) + 2.0,
            None => x + 2.5,
        }
    }

    fn prose(b: &mut Builder, x: f32, y: f32, text: &str) -> f32 {
        b.text(x, y, BODY, text)
    }

    fn filler(b: &mut Builder, from: f32, rows: usize) {
        for i in 0..rows {
            b.line(
                from - 12.0 * i as f32,
                BODY,
                "Running text of the paper goes on and on here.",
            );
        }
    }

    fn collect(pages: Vec<Page>) -> Collector {
        let mut c = Collector::new();
        for (i, mut p) in pages.into_iter().enumerate() {
            let mut state = Default::default();
            super::super::regions::classify(&mut p, &mut state);
            c.add_page(&p, i as u32);
        }
        c
    }

    fn defining_page() -> Page {
        let mut b = Builder::new();
        filler(&mut b, 720.0, 4);
        // "Let theta_t be the parameters at step t."
        let x = prose(&mut b, 72.0, 660.0, "Let ");
        let x = sym(&mut b, x, 660.0, "\u{03B8}", Some("t"));
        prose(&mut b, x, 660.0, " be the parameters at step t.");
        b.newline();
        // "We write L for the loss." with a calligraphic L in font 8.
        let x = prose(&mut b, 72.0, 648.0, "The model has a loss ");
        let x = b.text(x, 648.0, Style { font: 8, ..MATH }, "L");
        prose(&mut b, x + 2.5, 648.0, " = the mean loss over the data.");
        b.newline();
        // a plain x, never defined
        let x = prose(&mut b, 72.0, 636.0, "Since ");
        let x = sym(&mut b, x, 636.0, "x", None);
        prose(&mut b, x, 636.0, " is bounded we are done here.");
        b.newline();
        filler(&mut b, 600.0, 3);
        b.page()
    }

    fn using_page() -> Page {
        let mut b = Builder::new();
        filler(&mut b, 720.0, 3);
        let x = prose(&mut b, 72.0, 680.0, "The update of ");
        let x = sym(&mut b, x, 680.0, "\u{03B8}", Some("t"));
        let x = prose(&mut b, x, 680.0, " and of ");
        let x = sym(&mut b, x, 680.0, "\u{03B8}", None);
        let x = prose(&mut b, x, 680.0, " uses ");
        let x = b.text(x, 680.0, Style { font: 8, ..MATH }, "L");
        let x = prose(&mut b, x + 2.5, 680.0, " and ");
        let x = sym(&mut b, x, 680.0, "x", None);
        prose(&mut b, x, 680.0, " again.");
        b.newline();
        filler(&mut b, 660.0, 3);
        b.page()
    }

    #[test]
    fn a_symbol_points_at_the_sentence_that_defines_it() {
        let c = collect(vec![defining_page(), using_page()]);
        let spans = c.spans();
        let on_1 = &spans[&1];
        let labels: Vec<&str> = on_1.iter().map(|s| s.label.as_str()).collect();
        // theta_t by its cue, L in a font of its own by its first
        // occurrence; plain x and theta (no cue of their own) offer nothing.
        assert!(labels.contains(&"theta_t"), "{labels:?}");
        assert!(!labels.contains(&"x"), "{labels:?}");
        assert!(!labels.contains(&"theta"), "{labels:?}");
        assert!(labels.contains(&"L"), "{labels:?}");
        let theta = on_1.iter().find(|s| s.label == "theta_t").unwrap();
        assert_eq!(theta.target.page, 0);
        let region = theta.target.region.unwrap();
        // The defining line, not the page.
        assert!(
            region[1] < 660.0 && region[1] + region[3] > 667.0,
            "{region:?}"
        );
        assert!(region[3] < 30.0, "{region:?}");
        let mark = theta.target.mark.unwrap();
        assert!((mark[1] - 660.0).abs() < 4.0);
        // The definition itself is no span.
        assert!(spans
            .get(&0)
            .is_none_or(|s| s.iter().all(|s| s.label != "theta_t")));
    }

    #[test]
    fn a_scripted_symbol_falls_back_to_its_base() {
        let mut b = Builder::new();
        filler(&mut b, 720.0, 3);
        let x = prose(&mut b, 72.0, 680.0, "Let ");
        let x = sym(&mut b, x, 680.0, "\u{03B8}", None);
        prose(&mut b, x, 680.0, " be the parameters of the model.");
        b.newline();
        filler(&mut b, 660.0, 3);
        let first = b.page();
        let mut b = Builder::new();
        filler(&mut b, 720.0, 3);
        let x = prose(&mut b, 72.0, 680.0, "At step one ");
        let x = sym(&mut b, x, 680.0, "\u{03B8}", Some("k"));
        prose(&mut b, x, 680.0, " is updated once more.");
        b.newline();
        let second = b.page();
        let c = collect(vec![first, second]);
        let resolved = c.resolve();
        let at = c
            .occurrences()
            .iter()
            .position(|o| o.key.sub == "k")
            .unwrap();
        let (_, d) = resolved[at].clone().unwrap();
        assert_eq!(d.via, Via::Base);
        assert_eq!(d.page, 0);
        assert_eq!(d.cue, Some(Cue::Let));
    }

    #[test]
    fn glyph_identity_tells_symbols_apart() {
        let mut b = Builder::new();
        filler(&mut b, 720.0, 3);
        let x = prose(&mut b, 72.0, 680.0, "Let ");
        let x = b.text(x, 680.0, MATH, "W");
        prose(&mut b, x + 2.5, 680.0, " be a weight matrix of the layer.");
        b.newline();
        let x = prose(&mut b, 72.0, 668.0, "Then the bold ");
        let x = b.text(
            x,
            668.0,
            Style {
                font: 9,
                bold: true,
                ..MATH
            },
            "W",
        );
        prose(&mut b, x + 2.5, 668.0, " is another thing entirely here.");
        b.newline();
        let c = collect(vec![b.page()]);
        let keys: Vec<&Key> = c.occurrences().iter().map(|o| &o.key).collect();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1]);
        assert_eq!(c.occurrences()[0].cue, Some(Cue::Let));
        assert_eq!(c.occurrences()[1].cue, None);
    }

    #[test]
    fn powers_and_transposes_are_not_part_of_a_symbol() {
        let mut b = Builder::new();
        let x = prose(&mut b, 72.0, 700.0, "Then ");
        let x = b.text(x, 700.0, MATH, "W");
        let x = b.text(x, 703.5, MSCRIPT, "T");
        let x = prose(&mut b, x + 2.0, 700.0, " and ");
        let x = b.text(x, 700.0, MATH, "W");
        let x = b.text(x, 703.5, MSCRIPT, "*");
        prose(&mut b, x + 2.0, 700.0, " differ.");
        b.newline();
        let c = collect(vec![b.page()]);
        let o = c.occurrences();
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].key.sup, "");
        assert_eq!(o[1].key.sup, "*");
    }

    #[test]
    fn a_word_typed_in_math_is_no_symbol() {
        let mut b = Builder::new();
        let x = prose(&mut b, 72.0, 700.0, "Apply ");
        let x = b.text(x, 700.0, MATH, "softmax");
        prose(&mut b, x + 2.5, 700.0, " to the scores.");
        b.newline();
        let c = collect(vec![b.page()]);
        assert!(c.occurrences().is_empty());
    }

    #[test]
    fn a_notation_section_defines_what_it_lists() {
        let mut b = Builder::new();
        filler(&mut b, 740.0, 2);
        b.line(700.0, BOLD, "2 Notation");
        let x = sym(&mut b, 72.0, 680.0, "x", None);
        prose(&mut b, x, 680.0, " the input vector");
        b.newline();
        b.line(640.0, BOLD, "3 Method");
        let x = prose(&mut b, 72.0, 620.0, "We feed ");
        let x = sym(&mut b, x, 620.0, "x", None);
        prose(&mut b, x, 620.0, " to the network.");
        b.newline();
        let c = collect(vec![b.page()]);
        let o = c.occurrences();
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].cue, Some(Cue::Notation));
        assert_eq!(o[1].cue, None);
        let spans = c.spans();
        assert_eq!(spans[&0].len(), 1);
        assert_eq!(spans[&0][0].label, "x");
    }

    #[test]
    fn labels_are_ascii() {
        let key = Key {
            base: '\u{03B8}',
            family: "cmmi".into(),
            bold: false,
            accent: Some("hat"),
            sub: "t\u{2212}1".into(),
            sup: String::new(),
        };
        let label = key.label();
        assert!(label.is_ascii(), "{label}");
        assert!(label.starts_with("hat theta_t"), "{label}");
        let cal = Key {
            base: 'L',
            family: "cmsy".into(),
            bold: false,
            accent: None,
            sub: String::new(),
            sup: String::new(),
        };
        assert_eq!(cal.label(), "cal L");
    }
}

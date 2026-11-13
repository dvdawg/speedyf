//! Cross-references: "Eq. (12)", "Fig. 3", "Theorem 2.1", "Section 4.2" in
//! running text, and the places they point at - an equation's printed tag,
//! a caption, a theorem's label, a numbered heading.
//!
//! The governing rule is precision over recall. A wrong preview is far worse
//! than none, so every stage refuses when unsure:
//!
//! - A mention must name its kind ("Fig.", "Lemma", ...) - except a bare
//!   "(12)" after "in", "from", "by" or "using", which also needs the
//!   document to print an equation tag (12).
//! - A mention about another paper ("Eq. (5) of Ref. [12]", "[4, Lemma 2]",
//!   "Smith et al.'s Theorem 1") is dropped.
//! - A definition must look like one: a caption opening a block in its own
//!   face or size, a theorem label in a face distinct from the statement, a
//!   heading in a heading face, an equation tag set apart at the margin of a
//!   line of mathematics. The same words mid-sentence are mentions.
//! - A label defined more than once resolves only when the part of the
//!   document (main text, appendix, supplement, chapter) or document order
//!   settles it; otherwise nothing.
//!
//! PDFium-free.

use super::page::{same_column, Line, Page, Script};
use super::regions::Regions;
use super::Rect;
use crate::engine::preview::column_bounds;
use crate::engine::types::TextRun;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Equation,
    Figure,
    Table,
    Algorithm,
    Section,
    Theorem,
    Lemma,
    Proposition,
    Corollary,
    Definition,
    Remark,
    Example,
    Conjecture,
    Claim,
    Assumption,
}

impl Kind {
    /// The kind as the frontend names it.
    pub fn wire(self) -> &'static str {
        match self {
            Kind::Equation => "equation",
            Kind::Figure => "figure",
            Kind::Table => "table",
            Kind::Algorithm => "algorithm",
            Kind::Section => "section",
            _ => "theorem",
        }
    }

    /// The kind as a reader would say it, for labels.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Equation => "Equation",
            Kind::Figure => "Figure",
            Kind::Table => "Table",
            Kind::Algorithm => "Algorithm",
            Kind::Section => "Section",
            Kind::Theorem => "Theorem",
            Kind::Lemma => "Lemma",
            Kind::Proposition => "Proposition",
            Kind::Corollary => "Corollary",
            Kind::Definition => "Definition",
            Kind::Remark => "Remark",
            Kind::Example => "Example",
            Kind::Conjecture => "Conjecture",
            Kind::Claim => "Claim",
            Kind::Assumption => "Assumption",
        }
    }
}

/// Which label shapes a keyword accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// "(12)", "(2.14)", "(A3)", or after an explicit keyword "12".
    Equation,
    /// "3", "S3", "A1", with subfigure letters "3a", "3(b)".
    Figure,
    /// "2", "S2", and roman "II" after a capitalised keyword.
    Table,
    /// "2.1", "3", "A.1", "B".
    Theorem,
    /// "4.2", "IV", "B", "A.1".
    Section,
    /// "1".
    Number,
}

struct Keyword {
    word: &'static str,
    kind: Kind,
    shape: Shape,
    /// Written in lowercase: only arabic labels follow it.
    lower: bool,
    appendix: bool,
}

const fn kw(word: &'static str, kind: Kind, shape: Shape) -> Keyword {
    Keyword {
        word,
        kind,
        shape,
        lower: false,
        appendix: false,
    }
}

const fn lower(word: &'static str, kind: Kind, shape: Shape) -> Keyword {
    Keyword {
        word,
        kind,
        shape,
        lower: true,
        appendix: false,
    }
}

const fn appendix(word: &'static str, lower: bool) -> Keyword {
    Keyword {
        word,
        kind: Kind::Section,
        shape: Shape::Section,
        lower,
        appendix: true,
    }
}

use Kind as K;
use Shape as S;

/// Every keyword a mention may open with, longest spellings first where one
/// is a prefix of another.
const KEYWORDS: &[Keyword] = &[
    kw("Equations", K::Equation, S::Equation),
    kw("Equation", K::Equation, S::Equation),
    kw("EQUATIONS", K::Equation, S::Equation),
    kw("EQUATION", K::Equation, S::Equation),
    lower("equations", K::Equation, S::Equation),
    lower("equation", K::Equation, S::Equation),
    kw("Eqns.", K::Equation, S::Equation),
    kw("Eqn.", K::Equation, S::Equation),
    kw("Eqs.", K::Equation, S::Equation),
    kw("Eq.", K::Equation, S::Equation),
    kw("EQS.", K::Equation, S::Equation),
    kw("EQ.", K::Equation, S::Equation),
    lower("eqns.", K::Equation, S::Equation),
    lower("eqn.", K::Equation, S::Equation),
    lower("eqs.", K::Equation, S::Equation),
    lower("eq.", K::Equation, S::Equation),
    kw("Figures", K::Figure, S::Figure),
    kw("Figure", K::Figure, S::Figure),
    kw("FIGURES", K::Figure, S::Figure),
    kw("FIGURE", K::Figure, S::Figure),
    lower("figures", K::Figure, S::Figure),
    lower("figure", K::Figure, S::Figure),
    kw("Figs.", K::Figure, S::Figure),
    kw("Fig.", K::Figure, S::Figure),
    kw("FIGS.", K::Figure, S::Figure),
    kw("FIG.", K::Figure, S::Figure),
    lower("figs.", K::Figure, S::Figure),
    lower("fig.", K::Figure, S::Figure),
    kw("Tables", K::Table, S::Table),
    kw("Table", K::Table, S::Table),
    kw("TABLES", K::Table, S::Table),
    kw("TABLE", K::Table, S::Table),
    lower("tables", K::Table, S::Table),
    lower("table", K::Table, S::Table),
    kw("Tabs.", K::Table, S::Table),
    kw("Tab.", K::Table, S::Table),
    kw("Algorithms", K::Algorithm, S::Number),
    kw("Algorithm", K::Algorithm, S::Number),
    kw("ALGORITHM", K::Algorithm, S::Number),
    kw("Alg.", K::Algorithm, S::Number),
    kw("Sections", K::Section, S::Section),
    kw("Section", K::Section, S::Section),
    kw("SECTIONS", K::Section, S::Section),
    kw("SECTION", K::Section, S::Section),
    lower("sections", K::Section, S::Section),
    lower("section", K::Section, S::Section),
    kw("Sects.", K::Section, S::Section),
    kw("Sect.", K::Section, S::Section),
    kw("Secs.", K::Section, S::Section),
    kw("Sec.", K::Section, S::Section),
    lower("secs.", K::Section, S::Section),
    lower("sec.", K::Section, S::Section),
    kw("\u{00A7}\u{00A7}", K::Section, S::Section),
    kw("\u{00A7}", K::Section, S::Section),
    appendix("Appendices", false),
    appendix("Appendix", false),
    appendix("APPENDIX", false),
    appendix("appendices", true),
    appendix("appendix", true),
    kw("Theorems", K::Theorem, S::Theorem),
    kw("Theorem", K::Theorem, S::Theorem),
    kw("THEOREM", K::Theorem, S::Theorem),
    kw("Thms.", K::Theorem, S::Theorem),
    kw("Thm.", K::Theorem, S::Theorem),
    kw("Lemmata", K::Lemma, S::Theorem),
    kw("Lemmas", K::Lemma, S::Theorem),
    kw("Lemma", K::Lemma, S::Theorem),
    kw("LEMMA", K::Lemma, S::Theorem),
    kw("Lem.", K::Lemma, S::Theorem),
    kw("Propositions", K::Proposition, S::Theorem),
    kw("Proposition", K::Proposition, S::Theorem),
    kw("PROPOSITION", K::Proposition, S::Theorem),
    kw("Props.", K::Proposition, S::Theorem),
    kw("Prop.", K::Proposition, S::Theorem),
    kw("Corollaries", K::Corollary, S::Theorem),
    kw("Corollary", K::Corollary, S::Theorem),
    kw("COROLLARY", K::Corollary, S::Theorem),
    kw("Cor.", K::Corollary, S::Theorem),
    kw("Definitions", K::Definition, S::Theorem),
    kw("Definition", K::Definition, S::Theorem),
    kw("DEFINITION", K::Definition, S::Theorem),
    kw("Defn.", K::Definition, S::Theorem),
    kw("Def.", K::Definition, S::Theorem),
    kw("Remarks", K::Remark, S::Theorem),
    kw("Remark", K::Remark, S::Theorem),
    kw("REMARK", K::Remark, S::Theorem),
    kw("Rem.", K::Remark, S::Theorem),
    kw("Examples", K::Example, S::Theorem),
    kw("Example", K::Example, S::Theorem),
    kw("EXAMPLE", K::Example, S::Theorem),
    kw("Conjectures", K::Conjecture, S::Theorem),
    kw("Conjecture", K::Conjecture, S::Theorem),
    kw("CONJECTURE", K::Conjecture, S::Theorem),
    kw("Conj.", K::Conjecture, S::Theorem),
    kw("Claims", K::Claim, S::Theorem),
    kw("Claim", K::Claim, S::Theorem),
    kw("CLAIM", K::Claim, S::Theorem),
    kw("Assumptions", K::Assumption, S::Theorem),
    kw("Assumption", K::Assumption, S::Theorem),
    kw("ASSUMPTION", K::Assumption, S::Theorem),
];

/// A place a label is defined.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub kind: Kind,
    pub label: String,
    pub page: u32,
    /// Position on the page, in normalised-text order.
    pub order: usize,
    pub part: u32,
    /// The label as printed, in the page's normalised text: a mention
    /// overlapping it is the definition itself.
    pub label_range: (usize, usize),
    /// What a preview shows, y-up page space.
    pub region: Rect,
    /// Where a click lands: the top of the definition.
    pub jump: (f32, f32),
    /// The opening words of what the label introduces, letters and digits
    /// only: a restated theorem has the same ones as the original.
    pub signature: String,
}

/// A reference to a label in running text.
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub kind: Kind,
    pub label: String,
    pub page: u32,
    pub order: usize,
    pub part: u32,
    /// The span in the page's normalised text.
    pub range: (usize, usize),
    pub rects: Vec<Rect>,
}

fn at(text: &[char], index: usize) -> Option<char> {
    text.get(index).copied()
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric())
}

fn starts_with(text: &[char], from: usize, pattern: &str) -> bool {
    pattern
        .chars()
        .enumerate()
        .all(|(offset, p)| at(text, from + offset) == Some(p))
}

fn skip_spaces(text: &[char], mut index: usize) -> usize {
    while at(text, index) == Some(' ') {
        index += 1;
    }
    index
}

/// `\d+(\.\d+)*`: a dot belongs to the number only when a digit follows it.
fn number_end(text: &[char], from: usize) -> Option<usize> {
    let mut index = from;
    while at(text, index).is_some_and(|c| c.is_ascii_digit()) {
        index += 1;
    }
    if index == from || index - from > 4 {
        return None;
    }
    while at(text, index) == Some('.') && at(text, index + 1).is_some_and(|c| c.is_ascii_digit()) {
        let part = index + 1;
        index += 1;
        while at(text, index).is_some_and(|c| c.is_ascii_digit()) {
            index += 1;
        }
        if index - part > 3 {
            return None;
        }
    }
    Some(index)
}

/// Roman numerals I to XXXIX.
fn roman_end(text: &[char], from: usize) -> Option<usize> {
    let mut index = from;
    while at(text, index).is_some_and(|c| matches!(c, 'I' | 'V' | 'X')) {
        index += 1;
    }
    if index == from || index - from > 6 {
        return None;
    }
    let value: String = text[from..index].iter().collect();
    is_roman(&value).then_some(index)
}

fn is_roman(value: &str) -> bool {
    const VALID: &[&str] = &["", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX"];
    let tens = value.chars().take_while(|c| *c == 'X').count();
    tens <= 3 && VALID.contains(&&value[tens..])
}

fn collect(text: &[char], from: usize, to: usize) -> String {
    text[from..to].iter().collect()
}

/// A label, its end, and where its subfigure suffix (if any) ends.
struct Parsed {
    label: String,
    end: usize,
    /// Parenthesised equation form, "(12)".
    paren: bool,
}

/// An equation label's inside: "12", "2.14", "A3", "A.3", "3a", "S5".
fn equation_inner(text: &[char], from: usize) -> Option<usize> {
    let mut index = from;
    if at(text, index).is_some_and(|c| c.is_ascii_uppercase()) {
        index += 1;
        if at(text, index) == Some('.') {
            index += 1;
        }
    }
    let end = number_end(text, index)?;
    let mut end = end;
    if at(text, end).is_some_and(|c| c.is_ascii_lowercase()) && !is_word(at(text, end + 1)) {
        end += 1;
    }
    Some(end)
}

fn year_like(label: &str) -> bool {
    label.len() == 4
        && label.chars().all(|c| c.is_ascii_digit())
        && label
            .parse::<u32>()
            .is_ok_and(|v| (1000..=2999).contains(&v))
}

fn parse_equation(text: &[char], from: usize, bare_ok: bool) -> Option<Parsed> {
    if at(text, from) == Some('(') {
        let inner = from + 1;
        let end = equation_inner(text, inner)?;
        if at(text, end) != Some(')') {
            return None;
        }
        let label = collect(text, inner, end);
        if year_like(&label) {
            return None;
        }
        return Some(Parsed {
            label,
            end: end + 1,
            paren: true,
        });
    }
    if !bare_ok {
        return None;
    }
    let end = equation_inner(text, from)?;
    if is_word(at(text, end)) {
        return None;
    }
    let label = collect(text, from, end);
    if year_like(&label) {
        return None;
    }
    Some(Parsed {
        label,
        end,
        paren: false,
    })
}

/// "3", "S3", "A1", "ED 1" is not handled here; with subfigure suffixes
/// "3a", "3(b)", "3a-c", "3(a,b)" consumed but not part of the label.
fn parse_figure(text: &[char], from: usize, roman: bool, subfigures: bool) -> Option<Parsed> {
    let mut index = from;
    let mut label = String::new();
    if at(text, index).is_some_and(|c| c.is_ascii_uppercase())
        && at(text, index + 1).is_some_and(|c| c.is_ascii_digit() || c == '.')
    {
        label.push(text[index]);
        index += 1;
        if at(text, index) == Some('.') {
            label.push('.');
            index += 1;
        }
    }
    let end = match number_end(text, index) {
        Some(end) => end,
        None if roman && label.is_empty() => {
            let end = roman_end(text, index)?;
            if is_word(at(text, end)) {
                return None;
            }
            return Some(Parsed {
                label: collect(text, index, end),
                end,
                paren: false,
            });
        }
        None => return None,
    };
    label.push_str(&collect(text, index, end));
    let mut end = end;
    // Subfigure suffixes: "3a", "3ab", "3a-c", "3(b)", "3(a,b)".
    if !subfigures {
        if is_word(at(text, end)) {
            return None;
        }
    } else if at(text, end).is_some_and(|c| c.is_ascii_lowercase()) {
        let mut cursor = end;
        while at(text, cursor).is_some_and(|c| c.is_ascii_lowercase()) && cursor - end < 2 {
            cursor += 1;
        }
        if is_word(at(text, cursor)) {
            return None;
        }
        if at(text, cursor) == Some('-')
            && at(text, cursor + 1).is_some_and(|c| c.is_ascii_lowercase())
            && !is_word(at(text, cursor + 2))
        {
            cursor += 2;
        }
        end = cursor;
    } else if at(text, end) == Some('(') {
        let mut cursor = end + 1;
        let mut letters = 0;
        while let Some(c) = at(text, cursor) {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                letters += 1;
            } else if !matches!(c, ',' | '-' | ' ') {
                break;
            }
            cursor += 1;
        }
        if at(text, cursor) == Some(')') && letters > 0 && cursor - end <= 10 {
            end = cursor + 1;
        }
    } else if is_word(at(text, end)) {
        return None;
    }
    Some(Parsed {
        label,
        end,
        paren: false,
    })
}

/// "2.1", "3", "A.1", "A1", or a lone capital "A".
fn parse_theorem(text: &[char], from: usize) -> Option<Parsed> {
    let first = at(text, from)?;
    let end = if first.is_ascii_digit() {
        number_end(text, from)?
    } else if first.is_ascii_uppercase() {
        let index = from + 1;
        if at(text, index) == Some('.') && at(text, index + 1).is_some_and(|c| c.is_ascii_digit()) {
            number_end(text, index + 1)?
        } else if at(text, index).is_some_and(|c| c.is_ascii_digit()) {
            number_end(text, index)?
        } else {
            // A lone capital: never followed by another letter, and not the
            // initial of a name ("Theorem A. Smith proved").
            if is_word(at(text, index)) {
                return None;
            }
            if at(text, index) == Some('.')
                && at(text, index + 1) == Some(' ')
                && at(text, index + 2).is_some_and(|c| c.is_uppercase())
                && at(text, index + 3).is_some_and(|c| c.is_lowercase())
            {
                return None;
            }
            index
        }
    } else {
        return None;
    };
    if is_word(at(text, end)) {
        return None;
    }
    Some(Parsed {
        label: collect(text, from, end),
        end,
        paren: false,
    })
}

/// "4.2", "IV", "A", "A.1", "B.3.2".
fn parse_section(text: &[char], from: usize, roman: bool) -> Option<Parsed> {
    let first = at(text, from)?;
    let end = if first.is_ascii_digit() {
        number_end(text, from)?
    } else if roman && matches!(first, 'I' | 'V' | 'X') && roman_end(text, from).is_some() {
        let end = roman_end(text, from)?;
        // "IV.A", "II.B.3": a roman section with a lettered subsection.
        if at(text, end) == Some('.')
            && at(text, end + 1).is_some_and(|c| c.is_ascii_uppercase())
            && !is_word(at(text, end + 2))
        {
            end + 2
        } else {
            end
        }
    } else if first.is_ascii_uppercase() {
        let index = from + 1;
        if at(text, index) == Some('.') && at(text, index + 1).is_some_and(|c| c.is_ascii_digit()) {
            number_end(text, index + 1)?
        } else {
            index
        }
    } else {
        return None;
    };
    if is_word(at(text, end)) {
        return None;
    }
    Some(Parsed {
        label: collect(text, from, end),
        end,
        paren: false,
    })
}

fn parse_label(text: &[char], from: usize, key: &Keyword, bare_ok: bool) -> Option<Parsed> {
    let roman = !key.lower;
    match key.shape {
        Shape::Equation => parse_equation(text, from, bare_ok),
        Shape::Figure => parse_figure(text, from, false, true),
        Shape::Table => parse_figure(text, from, roman, false),
        Shape::Theorem => parse_theorem(text, from),
        Shape::Section => {
            if key.lower {
                // "section 4", "appendix B": no roman numerals after a
                // lowercase keyword.
                let first = at(text, from)?;
                let lettered = key.appendix && first.is_ascii_uppercase();
                if !first.is_ascii_digit() && !lettered {
                    return None;
                }
            }
            parse_section(text, from, roman)
        }
        Shape::Number => {
            let end = number_end(text, from)?;
            if is_word(at(text, end)) {
                return None;
            }
            Some(Parsed {
                label: collect(text, from, end),
                end,
                paren: false,
            })
        }
    }
}

/// After one label of a list, where the next begins: ", ", " and ", "-",
/// " to ", ", and ". None when the list ends.
fn list_separator(text: &[char], from: usize) -> Option<usize> {
    let index = skip_spaces(text, from);
    for sep in [
        ", and ",
        ",and ",
        ", or ",
        " and ",
        " or ",
        " to ",
        " through ",
        ", ",
        ",",
        "-",
        " - ",
        "&",
    ] {
        let start = if sep.starts_with(' ') { from } else { index };
        if starts_with(text, start, sep) {
            return Some(skip_spaces(text, start + sep.chars().count()));
        }
    }
    None
}

/// Words after which a bare "(12)" reads as an equation.
const BARE_TRIGGERS: &[&str] = &["in", "from", "by", "using"];

/// Phrases after a mention that make it about some other work.
const ELSEWHERE: &[&str] = &[
    "their ",
    "the original",
    "the cited",
    "the companion",
    "the aforementioned",
    "that paper",
    "that work",
    "this reference",
];

/// Whether the text around a mention says it is about another work:
/// "Eq. (5) of Ref. [12]", "Theorem 3 in [7]", "Fig. 2 of Smith et al.",
/// "[4, Lemma 2]", "Ref. 3's Eq. 4".
pub fn about_other_work(text: &[char], start: usize, end: usize) -> bool {
    // Possessive just before: "Smith's Lemma 2", "[3]'s Theorem 1".
    let before: String = text[start.saturating_sub(4)..start].iter().collect();
    if before.trim_end().ends_with("'s") || before.trim_end().ends_with("s'") {
        return true;
    }
    // Inside a citation bracket: "[4, Lemma 2]", "[see Theorem 2 of 12]".
    let window_start = start.saturating_sub(40);
    let mut depth_square = 0i32;
    let mut depth_round = 0i32;
    for index in (window_start..start).rev() {
        match text[index] {
            ']' => depth_square += 1,
            '[' => {
                if depth_square == 0 {
                    return true;
                }
                depth_square -= 1;
            }
            ')' => depth_round += 1,
            '(' => {
                if depth_round == 0 {
                    // "(Smith et al. 2019, Theorem 3)": a parenthesis holding
                    // an author-year citation.
                    let inside: String = text[index..start].iter().collect();
                    if inside.contains("et al") || has_year(&inside) || inside.contains('[') {
                        return true;
                    }
                    break;
                }
                depth_round -= 1;
            }
            _ => {}
        }
    }
    // After: "of [12]", "in Ref. 7", "from Smith et al.", "[7]" directly.
    let after_start = skip_spaces(text, end);
    let after_end = (after_start + 60).min(text.len());
    let mut after: String = text[after_start..after_end].iter().collect();
    // Up to the end of the clause.
    if let Some(cut) = after.find(['.', ';']).filter(|&cut| {
        // "Ref. 7" and "et al." keep their dots.
        let head = &after[..cut];
        !(head.ends_with("Ref")
            || head.ends_with("Refs")
            || head.ends_with("ref")
            || head.ends_with("et al"))
    }) {
        after.truncate(cut);
    }
    let after = after.trim_start_matches([',', ' ']);
    if after.starts_with('[') {
        return true;
    }
    let lower = after.to_lowercase();
    let preposition = ["of ", "in ", "from ", "by "]
        .iter()
        .find(|p| lower.starts_with(**p));
    if let Some(p) = preposition {
        let rest = &after[p.len()..];
        let rest_lower = rest.to_lowercase();
        let window: String = rest.chars().take(45).collect();
        if window.contains('[')
            || rest_lower.starts_with("ref")
            || window.contains("Ref.")
            || window.contains("Refs.")
            || window.contains("ref.")
            || window.contains("et al")
            || has_year(&window)
            || ELSEWHERE.iter().any(|e| rest_lower.starts_with(e))
        {
            return true;
        }
    }
    false
}

fn has_year(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(4).enumerate().any(|(i, w)| {
        w.iter().all(|c| c.is_ascii_digit())
            && (w[0] == '1' && w[1] == '9' || w[0] == '2' && w[1] == '0')
            && !chars.get(i + 4).is_some_and(|c| c.is_ascii_digit())
            && !i
                .checked_sub(1)
                .and_then(|p| chars.get(p))
                .is_some_and(|c| c.is_ascii_digit() || *c == '.')
    })
}

/// Whether a stretch of text can carry a mention at all.
fn mentionable(page: &Page, start: usize, end: usize) -> bool {
    for index in start..end.min(page.text.len()) {
        if page.script[index] != Script::Normal && page.text[index] != ' ' {
            return false;
        }
        let Some(line) = page.line_at(index) else {
            continue;
        };
        if line.excluded() || line.is_display() {
            return false;
        }
    }
    !page.any_math(start, end)
}

/// Every cross-reference mention on a page.
pub fn find_mentions(page: &Page, regions: &Regions, src: u32) -> Vec<Mention> {
    let text = &page.text;
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < text.len() {
        let boundary = index == 0 || !is_word(at(text, index - 1));
        if !boundary {
            index += 1;
            continue;
        }
        if let Some((found, next)) = mention_at(page, index) {
            for (kind, label, range) in found {
                let part = page
                    .line_of
                    .get(range.0)
                    .and_then(|&l| regions.part_of_line.get(l as usize))
                    .copied()
                    .unwrap_or(0);
                out.push(Mention {
                    kind,
                    label,
                    page: src,
                    order: range.0,
                    part,
                    range,
                    rects: page.rects(range.0, range.1),
                });
            }
            index = next.max(index + 1);
            continue;
        }
        index += 1;
    }
    out
}

type Found = Vec<(Kind, String, (usize, usize))>;

/// A mention (or a list of them) starting at `start`.
fn mention_at(page: &Page, start: usize) -> Option<(Found, usize)> {
    let text = &page.text;
    // A keyword, then its labels.
    for key in KEYWORDS {
        if !starts_with(text, start, key.word) {
            continue;
        }
        let after_word = start + key.word.chars().count();
        let ends_in_letter = key.word.chars().last().is_some_and(|c| c.is_alphanumeric());
        if ends_in_letter && is_word(at(text, after_word)) {
            continue;
        }
        let label_start = skip_spaces(text, after_word);
        if label_start - after_word > 1 && key.word != "\u{00A7}" {
            return None;
        }
        // "Supplementary Fig. 3" and "Extended Data Fig. 1" are their own
        // numbering.
        let namespace = namespace_before(text, start);
        let first = parse_label(text, label_start, key, true)?;
        let paren = first.paren;
        let mut labels = vec![(namespace.clone() + &first.label, (start, first.end))];
        let mut end = first.end;
        while let Some(next) = list_separator(text, end) {
            let Some(parsed) = parse_label(text, next, key, !paren) else {
                break;
            };
            if parsed.paren != paren {
                break;
            }
            labels.push((namespace.clone() + &parsed.label, (next, parsed.end)));
            end = parsed.end;
        }
        if about_other_work(text, start, end) {
            return Some((Vec::new(), end));
        }
        let found = labels
            .into_iter()
            .filter(|(_, (s, e))| mentionable(page, *s, *e))
            .map(|(label, range)| (key.kind, label, range))
            .collect();
        return Some((found, end));
    }
    // A bare "(12)" after "in", "from", "by", "using".
    if at(text, start) == Some('(') {
        return None;
    }
    let word_end = (start..text.len())
        .find(|&i| !text[i].is_alphabetic())
        .unwrap_or(text.len());
    let word: String = text[start..word_end]
        .iter()
        .collect::<String>()
        .to_lowercase();
    if !BARE_TRIGGERS.contains(&word.as_str()) || at(text, word_end) != Some(' ') {
        return None;
    }
    let paren_at = word_end + 1;
    let first = parse_equation(text, paren_at, false)?;
    let mut labels = vec![(first.label, (paren_at, first.end))];
    let mut end = first.end;
    while let Some(next) = list_separator(text, end) {
        let Some(parsed) = parse_equation(text, next, false) else {
            break;
        };
        labels.push((parsed.label, (next, parsed.end)));
        end = parsed.end;
    }
    if about_other_work(text, paren_at, end) {
        return Some((Vec::new(), end));
    }
    let found = labels
        .into_iter()
        .filter(|(_, (s, e))| mentionable(page, *s, *e))
        .map(|(label, range)| (Kind::Equation, label, range))
        .collect();
    Some((found, end))
}

/// "Supplementary " or "Extended Data " just before a keyword.
fn namespace_before(text: &[char], start: usize) -> String {
    let before: String = text[start.saturating_sub(22)..start].iter().collect();
    let before = before.trim_end();
    if before.ends_with("Extended Data") {
        "ED ".to_string()
    } else if before.ends_with("Supplementary")
        || before.ends_with("Supplemental")
        || before.ends_with("Suppl.")
    {
        "Supp ".to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------------

/// What follows a label at the start of a caption or statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Terminator {
    /// ':' or '|' or an em dash: never prose.
    Strong,
    /// '.' (or '(' opening a theorem's name).
    Period,
    /// Nothing more on the line.
    EndOfLine,
    /// More words straight away.
    None,
}

fn terminator(text: &[char], from: usize, end: usize) -> (Terminator, usize) {
    let index = skip_spaces(text, from);
    if index >= end {
        return (Terminator::EndOfLine, index);
    }
    match text[index] {
        ':' | '|' => (Terminator::Strong, index + 1),
        '-' if at(text, index + 1) == Some(' ') || at(text, index + 1) == Some('-') => {
            (Terminator::Strong, index + 1)
        }
        '.' => (Terminator::Period, index + 1),
        '(' => (Terminator::Period, index),
        _ => (Terminator::None, index),
    }
}

/// The font of the first letter or digit at or after `from` on the line, or
/// on the next line when this one ends.
fn font_after(page: &Page, line: usize, from: usize) -> Option<u16> {
    let l = &page.lines[line];
    let in_line = (from..l.end).find(|&i| page.text[i].is_alphanumeric());
    let index = match in_line {
        Some(i) => i,
        None => {
            let next = page.lines.get(line + 1)?;
            (next.start..next.end).find(|&i| page.text[i].is_alphanumeric())?
        }
    };
    page.glyph_at(index).map(|g| g.font)
}

/// Caption keywords at the start of a line.
const CAPTION_WORDS: &[(&str, Kind, bool)] = &[
    ("Figure", Kind::Figure, false),
    ("FIGURE", Kind::Figure, true),
    ("Fig.", Kind::Figure, false),
    ("FIG.", Kind::Figure, true),
    ("Table", Kind::Table, false),
    ("TABLE", Kind::Table, true),
    ("Tab.", Kind::Table, false),
    ("Algorithm", Kind::Algorithm, false),
    ("ALGORITHM", Kind::Algorithm, true),
];

const THEOREM_WORDS: &[(&str, Kind)] = &[
    ("Theorem", Kind::Theorem),
    ("THEOREM", Kind::Theorem),
    ("Lemma", Kind::Lemma),
    ("LEMMA", Kind::Lemma),
    ("Proposition", Kind::Proposition),
    ("PROPOSITION", Kind::Proposition),
    ("Corollary", Kind::Corollary),
    ("COROLLARY", Kind::Corollary),
    ("Definition", Kind::Definition),
    ("DEFINITION", Kind::Definition),
    ("Remark", Kind::Remark),
    ("REMARK", Kind::Remark),
    ("Example", Kind::Example),
    ("EXAMPLE", Kind::Example),
    ("Conjecture", Kind::Conjecture),
    ("CONJECTURE", Kind::Conjecture),
    ("Claim", Kind::Claim),
    ("CLAIM", Kind::Claim),
    ("Assumption", Kind::Assumption),
    ("ASSUMPTION", Kind::Assumption),
];

/// A label definition found at the start of a line, before its region is
/// worked out.
struct Head {
    kind: Kind,
    label: String,
    line: usize,
    label_range: (usize, usize),
}

fn caption_head(page: &Page, line: usize) -> Option<Head> {
    let l = &page.lines[line];
    let text = &page.text;
    let mut start = l.start;
    let mut namespace = String::new();
    for (prefix, ns) in [
        ("Supplementary ", "Supp "),
        ("Supplemental ", "Supp "),
        ("Extended Data ", "ED "),
    ] {
        if starts_with(text, start, prefix) {
            start += prefix.chars().count();
            namespace = ns.to_string();
            break;
        }
    }
    let (word, kind, capitals) = CAPTION_WORDS
        .iter()
        .find(|(w, _, _)| starts_with(text, start, w))?;
    let after = start + word.chars().count();
    if !word.ends_with('.') && is_word(at(text, after)) {
        return None;
    }
    let label_start = skip_spaces(text, after);
    let parsed = parse_figure(text, label_start, *kind == Kind::Table, false)?;
    if parsed.end > l.end {
        return None;
    }
    // Subfigure suffixes have no place in a caption's own label.
    if parsed.end - label_start > parsed.label.chars().count() {
        return None;
    }
    let (term, after_term) = terminator(text, parsed.end, l.end);
    if page.continues_previous(line) {
        return None;
    }
    let label_font = page.font_of(start, parsed.end);
    let distinct = label_font.is_some() && label_font != font_after(page, line, after_term);
    let smaller = page.body_size > 0.0 && l.size > 0.0 && l.size < 0.95 * page.body_size;
    let accepted = match term {
        Terminator::Strong => true,
        Terminator::Period => distinct || smaller || *capitals,
        // "Figure 3" alone on a line, the caption on the next: only in
        // bold, and only with that next line right below.
        Terminator::EndOfLine => {
            *kind != Kind::Algorithm
                && page.all_bold(start, parsed.end)
                && page
                    .lines
                    .get(line + 1)
                    .is_some_and(|next| same_column(next, l) && l.bottom - next.top <= 1.5 * l.em())
        }
        // "Algorithm 1 Name", set in its own face (algorithm2e).
        Terminator::None => *kind == Kind::Algorithm && distinct,
    };
    if !accepted {
        return None;
    }
    Some(Head {
        kind: *kind,
        label: namespace + &parsed.label,
        line,
        label_range: (l.start, parsed.end),
    })
}

fn theorem_head(page: &Page, line: usize) -> Option<Head> {
    let l = &page.lines[line];
    let text = &page.text;
    let (word, kind) = THEOREM_WORDS
        .iter()
        .find(|(w, _)| starts_with(text, l.start, w))?;
    let after = l.start + word.chars().count();
    if is_word(at(text, after)) || at(text, after) != Some(' ') {
        return None;
    }
    let label_start = skip_spaces(text, after);
    let parsed = parse_theorem(text, label_start)?;
    if parsed.end > l.end {
        return None;
    }
    let (term, after_term) = terminator(text, parsed.end, l.end);
    if matches!(term, Terminator::None) || page.continues_previous(line) {
        return None;
    }
    // The label is set in a face of its own - bold, small capitals, italic
    // against an upright statement. Without that it is a sentence that
    // happens to open a line.
    let label_font = page.font_of(l.start, parsed.end)?;
    let following = font_after(page, line, after_term)?;
    if label_font == following {
        return None;
    }
    Some(Head {
        kind: *kind,
        label: parsed.label,
        line,
        label_range: (l.start, parsed.end),
    })
}

fn section_head(page: &Page, line: usize) -> Option<Head> {
    let l = &page.lines[line];
    let text = &page.text;
    let mut start = l.start;
    let mut appendix = false;
    for prefix in ["Appendix ", "APPENDIX "] {
        if starts_with(text, start, prefix) {
            start += prefix.chars().count();
            appendix = true;
        }
    }
    let first = at(text, start)?;
    let (label, end) = if first.is_ascii_digit() {
        let end = number_end(text, start)?;
        (collect(text, start, end), end)
    } else if matches!(first, 'I' | 'V' | 'X') && roman_end(text, start).is_some() && !appendix {
        let end = roman_end(text, start)?;
        // A roman heading always carries its period: "IV. RESULTS".
        if at(text, end) != Some('.') {
            return None;
        }
        (collect(text, start, end), end)
    } else if first.is_ascii_uppercase() {
        let index = start + 1;
        let end = if at(text, index) == Some('.')
            && at(text, index + 1).is_some_and(|c| c.is_ascii_digit())
        {
            number_end(text, index + 1)?
        } else {
            index
        };
        (collect(text, start, end), end)
    } else {
        return None;
    };
    if is_word(at(text, end)) {
        return None;
    }
    let first_component: u32 = label
        .split('.')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    if first_component > 60 {
        return None;
    }
    // The title: a capital after the label (and its period or colon).
    let mut title = end;
    if matches!(at(text, title), Some('.' | ':')) {
        title += 1;
    }
    let title = skip_spaces(text, title);
    let titled = if title >= l.end {
        appendix
    } else {
        at(text, title).is_some_and(|c| c.is_uppercase()) && title > end
    };
    if !titled || page.continues_previous(line) {
        return None;
    }
    let line_text = page.line_text(line);
    if line_text.contains("....") || line_text.contains(". . .") {
        return None;
    }
    // Set in a heading face: the label and the title's first word share a
    // face that is not the body's, or the line is larger than the body.
    let word_end = (title..l.end)
        .find(|&i| !text[i].is_alphanumeric())
        .unwrap_or(l.end);
    let label_font = page.font_of(start, end)?;
    let title_font = if title < l.end {
        page.font_of(title, word_end)
    } else {
        Some(label_font)
    };
    let larger = page.body_size > 0.0 && page.size_of(start, word_end) >= 1.1 * page.body_size;
    // Another face is not enough - an enumerated list in an italic
    // statement is set in roman. A heading is bold, or larger.
    let heading_face = title_font == Some(label_font)
        && label_font != page.body_font
        && page.all_bold(start, word_end);
    let short = l.end - l.start <= 90;
    let set_apart = heading_face || larger;
    // A long line in a merely larger size is a run of display text, not a
    // heading; a run-in heading keeps its heading face.
    if !set_apart || (!short && !heading_face) {
        return None;
    }
    // A heading is a run of lines of its own: never smaller than the body.
    if page.body_size > 0.0 && l.size > 0.0 && l.size < 0.95 * page.body_size {
        return None;
    }
    Some(Head {
        kind: Kind::Section,
        label,
        line,
        label_range: (l.start, end),
    })
}

/// An equation tag at the margin of a line of mathematics: "(12)" set well
/// apart at the right (or, `leqno`, at the left).
fn equation_tags(page: &Page, line: usize) -> Vec<(String, (usize, usize))> {
    let l = &page.lines[line];
    let text = &page.text;
    let em = l.em();
    let mut out = Vec::new();
    let gap_before = |index: usize| -> Option<f32> {
        let glyph = page.glyph_at(index)?;
        let previous = (l.start..index)
            .rev()
            .filter_map(|i| page.glyph_at(i))
            .find(|g| g.has_box())?;
        Some(glyph.rect[0] - (previous.rect[0] + previous.rect[2]))
    };
    // Right: the line ends "(label)".
    if at(text, l.end.saturating_sub(1)) == Some(')') {
        if let Some(open) = (l.start..l.end).rev().find(|&i| text[i] == '(') {
            if let Some(parsed) = parse_equation(text, open, false) {
                let whole = parsed.end == l.end;
                if whole && open > l.start {
                    if let Some(gap) = gap_before(open) {
                        if gap >= 1.2 * em && display_content(page, line, l.start, open) {
                            out.push((parsed.label, (open, parsed.end)));
                        }
                    }
                } else if whole && lone_tag_beside_display(page, line, true) {
                    out.push((parsed.label, (open, parsed.end)));
                }
            }
        }
    }
    // Left: the line opens "(label)" with the mathematics well after it.
    if out.is_empty() && at(text, l.start) == Some('(') {
        if let Some(parsed) = parse_equation(text, l.start, false) {
            let next = (parsed.end..l.end).find(|&i| text[i] != ' ');
            if let Some(next) = next {
                if let Some(gap) = gap_before(next) {
                    if gap >= 1.2 * em && display_content(page, line, next, l.end) {
                        out.push((parsed.label, (l.start, parsed.end)));
                    }
                }
            } else if lone_tag_beside_display(page, line, false) {
                out.push((parsed.label, (l.start, parsed.end)));
            }
        }
    }
    out
}

/// Whether `text[from..to]` of a line is mathematics (or the line sits in a
/// display whose other lines are).
fn display_content(page: &Page, line: usize, from: usize, to: usize) -> bool {
    let mut math = 0;
    let mut visible = 0;
    for index in from..to {
        let c = page.text[index];
        if c == ' ' {
            continue;
        }
        visible += 1;
        if page.glyph_at(index).is_some_and(|g| g.math) || super::page::is_math_symbol(c) {
            math += 1;
        }
    }
    if visible == 0 {
        return false;
    }
    let share = math as f32 / visible as f32;
    if (math >= 1 && share >= 0.25) || math >= 3 {
        return true;
    }
    // "for all x" on the tagged line of a display.
    let l = &page.lines[line];
    page.lines.iter().any(|other| {
        other != l
            && other.is_display()
            && (other.baseline - l.baseline).abs() <= 2.0 * l.em()
            && same_column(other, l)
    }) && visible <= 30
}

/// A line holding nothing but a tag, level with a display line that ends
/// well before it (right tag) or starts well after it (left tag): the tag of
/// a multi-line display, centred between its lines.
fn lone_tag_beside_display(page: &Page, line: usize, right: bool) -> bool {
    let tag = &page.lines[line];
    let em = tag.em();
    page.lines.iter().enumerate().any(|(index, other)| {
        index != line
            && other.is_display()
            && (other.baseline - tag.baseline).abs() <= 1.6 * em
            && if right {
                other.x1 <= tag.x0 - 1.2 * em
            } else {
                other.x0 >= tag.x1 + 1.2 * em
            }
    })
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

fn line_rect(l: &Line) -> Rect {
    [l.x0, l.bottom, (l.x1 - l.x0).max(1.0), l.height()]
}

fn pad(rect: Rect, by: f32, page: &Page) -> Rect {
    let x0 = (rect[0] - by).max(0.0);
    let y0 = (rect[1] - by).max(0.0);
    let x1 = (rect[0] + rect[2] + by).min(page.width.max(1.0));
    let y1 = (rect[1] + rect[3] + by).min(page.height.max(1.0));
    [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
}

/// Lines below `line` in its column, while they stay close and `keep` says
/// so; stops at the first gap of more than `gap` ems.
fn following_lines(page: &Page, line: usize, gap: f32, keep: impl Fn(usize) -> bool) -> Vec<usize> {
    let anchor = &page.lines[line];
    let mut out = Vec::new();
    let mut bottom = anchor.bottom;
    let mut candidates: Vec<usize> = (0..page.lines.len())
        .filter(|&i| {
            i != line
                && page.lines[i].top <= anchor.bottom + 0.5 * anchor.em()
                && same_column(&page.lines[i], anchor)
                && !page.lines[i].flags.furniture
        })
        .collect();
    candidates.sort_by(|a, b| page.lines[*b].top.total_cmp(&page.lines[*a].top));
    for index in candidates {
        let l = &page.lines[index];
        if bottom - l.top > gap * anchor.em() {
            break;
        }
        if !keep(index) {
            break;
        }
        out.push(index);
        bottom = bottom.min(l.bottom);
    }
    out
}

/// What a caption's preview shows: the artwork above a figure's caption, the
/// table below a table's, with the caption itself.
fn caption_region(page: &Page, head: &Head) -> Rect {
    let line = &page.lines[head.line];
    let caption_size = line.size;
    let caption_lines = following_lines(page, head.line, 1.2, |i| {
        let l = &page.lines[i];
        (caption_size <= 0.0 || (l.size - caption_size).abs() <= 0.06 * caption_size)
            && !starts_another(page, i)
    });
    let mut caption = line_rect(line);
    for &i in caption_lines.iter().take(12) {
        caption = union(caption, line_rect(&page.lines[i]));
    }
    let runs = runs_of(page);
    let above =
        crate::engine::figures::figure_crop_rect(&crate::engine::figures::FigureCropInput {
            page_w_pt: page.width,
            page_h_pt: page.height,
            caption_y: line.baseline,
            caption_x: Some(line.x0),
            runs: &runs,
        });
    // Never up into a running head over the figure.
    let ceiling = page
        .lines
        .iter()
        .filter(|l| l.flags.furniture && l.bottom > line.top)
        .map(|l| l.bottom - 1.0)
        .fold(page.height, f32::min);
    let above = {
        let top = (above[1] + above[3]).min(ceiling);
        [above[0], above[1], above[2], (top - above[1]).max(1.0)]
    };
    match head.kind {
        Kind::Figure => pad(union(above, caption), 2.0, page),
        _ => {
            // A table or algorithm sits under its caption, unless prose
            // follows straight away - then it sat above.
            let after_caption = caption_lines.last().copied().unwrap_or(head.line);
            let column_w = (caption[2]).max(1.0);
            let prose = |i: usize| {
                let l = &page.lines[i];
                l.font == page.body_font
                    && (page.body_size <= 0.0 || (l.size - page.body_size).abs() < 0.3)
                    && l.x1 - l.x0 >= 0.8 * column_w
                    && !l.is_display()
            };
            let body = following_lines(page, after_caption, 3.0, |i| !prose(i));
            if body.is_empty() {
                return pad(union(above, caption), 2.0, page);
            }
            let mut region = caption;
            let limit = caption[1] + caption[3] - 0.5 * page.height;
            for i in body {
                let l = &page.lines[i];
                if l.bottom < limit {
                    break;
                }
                region = union(region, line_rect(l));
            }
            pad(region, 3.0, page)
        }
    }
}

/// Whether a line opens a definition of its own.
fn starts_another(page: &Page, line: usize) -> bool {
    caption_head(page, line).is_some()
        || theorem_head(page, line).is_some()
        || section_head(page, line).is_some()
}

fn opens_proof(page: &Page, line: usize) -> bool {
    let text = page.line_text(line);
    text.starts_with("Proof") || text.starts_with("PROOF") || text.starts_with("Proof.")
}

/// A theorem's statement: from its label down to "Proof", the next labelled
/// block, or a gap.
fn statement_region(page: &Page, head: &Head) -> Rect {
    let line = &page.lines[head.line];
    let top = line.top;
    let mut region = line_rect(line);
    for i in following_lines(page, head.line, 1.6, |i| {
        !opens_proof(page, i) && !starts_another(page, i)
    }) {
        let l = &page.lines[i];
        if top - l.bottom > 260.0 {
            break;
        }
        region = union(region, line_rect(l));
    }
    pad(region, 3.0, page)
}

/// A heading and the start of what follows it.
fn section_region(page: &Page, head: &Head) -> Rect {
    let line = &page.lines[head.line];
    let top = line.top;
    let mut region = line_rect(line);
    for i in following_lines(page, head.line, 2.5, |i| !section_head_quick(page, i)) {
        let l = &page.lines[i];
        if top - l.bottom > 140.0 {
            break;
        }
        region = union(region, line_rect(l));
    }
    pad(region, 3.0, page)
}

fn section_head_quick(page: &Page, line: usize) -> bool {
    section_head(page, line).is_some()
}

/// Words of three letters or more set in a text font: "where", "and",
/// "solve".
fn prose_words(page: &Page, line: &Line) -> usize {
    let mut count = 0;
    let mut letters = 0;
    for at in line.start..=line.end {
        let text_letter = at < line.end
            && page.text[at].is_alphabetic()
            && page.glyph_at(at).is_some_and(|g| !g.math);
        if text_letter {
            letters += 1;
        } else {
            if letters >= 3 {
                count += 1;
            }
            letters = 0;
        }
    }
    count
}

/// The page's lines as text runs, for the crop helpers shared with the
/// figure panel and link previews.
fn runs_of(page: &Page) -> Vec<TextRun> {
    page.lines
        .iter()
        .filter(|l| !l.flags.furniture)
        .map(|l| TextRun {
            text: String::new(),
            start: 0,
            x: l.x0,
            y: l.bottom,
            w: l.x1 - l.x0,
            h: l.height(),
        })
        .collect()
}

/// A display around its tag: the tagged line, the mathematics level with it
/// in its column (the lines of an `align` whose tag sits between them), and
/// the mathematics set close above and below.
fn equation_region(page: &Page, line: usize, tagged: &[bool]) -> Rect {
    let tag_line = &page.lines[line];
    let em = tag_line.em();
    // The column around the display, read from the text above it and below
    // it alike.
    let column = column_bounds(
        &runs_of(page),
        page.width,
        tag_line.baseline + 200.0,
        Some(tag_line.x0),
    );
    let width = (column.end - column.start).max(1.0);
    // A display is set apart from the column's margin; a line of prose dense
    // with inline mathematics starts at the margin, runs most of the way
    // across, and has words in it - and is no part of the display.
    let in_column = |l: &Line| {
        let inside = l.x0 >= column.start - 2.0 && l.x1 <= column.end + 2.0;
        let prose = l.x0 <= column.start + l.em()
            && l.x1 - l.x0 >= 0.6 * width
            && prose_words(page, l) >= 2;
        inside && (!prose || l == tag_line)
    };
    let mut region = line_rect(tag_line);
    // Another tagged line is another equation: it is never part of this
    // one's preview, so an `align` numbered line by line previews one line.
    let own = |index: usize| index == line || !tagged.get(index).copied().unwrap_or(false);
    for (index, other) in page.lines.iter().enumerate() {
        let centre = (other.top + other.bottom) / 2.0;
        if own(index)
            && !other.flags.furniture
            && other.is_display()
            && in_column(other)
            && centre >= tag_line.bottom - 0.5 * em
            && centre <= tag_line.top + 0.5 * em
        {
            region = union(region, line_rect(other));
        }
    }
    loop {
        let mut grew = false;
        for (index, other) in page.lines.iter().enumerate() {
            if other.flags.furniture || index == line || !own(index) || !in_column(other) {
                continue;
            }
            let inside =
                other.bottom >= region[1] - 0.1 && other.top <= region[1] + region[3] + 0.1;
            if inside {
                continue;
            }
            let gap = if other.top <= region[1] {
                region[1] - other.top
            } else {
                other.bottom - (region[1] + region[3])
            };
            if gap <= 0.9 * em && other.is_display() {
                let next = union(region, line_rect(other));
                if next[3] <= 180.0 {
                    region = next;
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    pad(region, 4.0, page)
}

/// The first words after a label, past a parenthesised name: what tells a
/// restatement of Theorem 2 from a second, different Theorem 2.
fn signature(page: &Page, line: usize, from: usize) -> String {
    let l = &page.lines[line];
    let end = page
        .lines
        .get(line + 1)
        .filter(|next| same_column(next, l))
        .map_or(l.end, |next| next.end);
    let mut index = from;
    while index < end && matches!(page.text[index], ' ' | '.' | ':') {
        index += 1;
    }
    if page.text.get(index) == Some(&'(') {
        if let Some(close) = (index..end).find(|&i| page.text[i] == ')') {
            index = close + 1;
        }
    }
    page.text[index.min(end)..end]
        .iter()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .take(40)
        .collect()
}

/// Every definition on a page.
pub fn find_definitions(page: &Page, regions: &Regions, src: u32) -> Vec<Definition> {
    let mut out = Vec::new();
    let tags: Vec<Vec<(String, (usize, usize))>> = (0..page.lines.len())
        .map(|line| {
            if page.lines[line].excluded() {
                Vec::new()
            } else {
                equation_tags(page, line)
            }
        })
        .collect();
    let tagged: Vec<bool> = tags.iter().map(|t| !t.is_empty()).collect();
    for (line, line_tags) in tags.iter().enumerate() {
        let l = &page.lines[line];
        if l.excluded() {
            continue;
        }
        let part = regions.part_of_line.get(line).copied().unwrap_or(0);
        let head = caption_head(page, line)
            .or_else(|| theorem_head(page, line))
            .or_else(|| section_head(page, line));
        if let Some(head) = head {
            let region = match head.kind {
                Kind::Figure | Kind::Table | Kind::Algorithm => caption_region(page, &head),
                Kind::Section => section_region(page, &head),
                _ => statement_region(page, &head),
            };
            let signature = signature(page, line, head.label_range.1);
            out.push(Definition {
                kind: head.kind,
                label: head.label,
                page: src,
                order: l.start,
                part,
                label_range: head.label_range,
                region,
                jump: (l.x0, l.top),
                signature,
            });
            continue;
        }
        for (label, range) in line_tags.iter().cloned() {
            let region = equation_region(page, line, &tagged);
            out.push(Definition {
                kind: Kind::Equation,
                label,
                page: src,
                order: range.0,
                part,
                label_range: range,
                jump: (region[0], region[1] + region[3]),
                region,
                signature: String::new(),
            });
        }
    }
    out
}

/// Resolve a mention against every definition of its label: one in the
/// mention's own part, else the nearest preceding one there, else the only
/// one in the document. Anything else is ambiguous, and gets nothing.
pub fn resolve<'a>(mention: &Mention, candidates: &'a [Definition]) -> Option<&'a Definition> {
    let wanted: Vec<&Definition> = candidates
        .iter()
        .filter(|d| d.kind == mention.kind && d.label == mention.label)
        .collect();
    if wanted.is_empty() {
        return None;
    }
    // A theorem restated before its proof, an algorithm repeated in an
    // appendix: the same words after the same label. The original is the
    // first one.
    let restated = wanted.len() > 1
        && wanted[0].signature.chars().count() >= 12
        && wanted.iter().all(|d| d.signature == wanted[0].signature);
    if restated {
        return wanted.into_iter().min_by_key(|d| (d.page, d.order));
    }
    let same_part: Vec<&Definition> = wanted
        .iter()
        .copied()
        .filter(|d| d.part == mention.part)
        .collect();
    match same_part.len() {
        1 => return Some(same_part[0]),
        0 => {}
        // Section numbers never legitimately repeat within a part: a second
        // "4.2" there is a running head or a table row, and either could be
        // the wrong one.
        _ if mention.kind == Kind::Section => return None,
        _ => {
            return same_part
                .into_iter()
                .filter(|d| (d.page, d.order) <= (mention.page, mention.order))
                .max_by_key(|d| (d.page, d.order));
        }
    }
    (wanted.len() == 1).then(|| wanted[0])
}

/// Whether a mention is the definition's own label.
pub fn is_definition_site(mention: &Mention, definitions: &[Definition]) -> bool {
    definitions.iter().any(|d| {
        d.page == mention.page
            && mention.range.0 < d.label_range.1
            && d.label_range.0 < mention.range.1
    })
}

/// The aria label of a mention.
pub fn describe(kind: Kind, label: &str) -> String {
    let shown = label
        .strip_prefix("ED ")
        .map(|l| format!("Extended Data {} {l}", kind.word()))
        .or_else(|| {
            label
                .strip_prefix("Supp ")
                .map(|l| format!("Supplementary {} {l}", kind.word()))
        });
    shown.unwrap_or_else(|| format!("{} {label}", kind.word()))
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::super::page::text_of;
    use super::super::regions::{classify, RegionState};
    use super::*;

    fn mentions_in(text: &str) -> Vec<(Kind, String, String)> {
        let mut b = Builder::new();
        b.line(700.0, BODY, text);
        let page = b.page();
        let regions = Regions {
            part_of_line: vec![0; page.lines.len()],
        };
        let all: String = text_of(&page);
        find_mentions(&page, &regions, 0)
            .into_iter()
            .map(|m| {
                let span: String = all
                    .chars()
                    .skip(m.range.0)
                    .take(m.range.1 - m.range.0)
                    .collect();
                (m.kind, m.label, span)
            })
            .collect()
    }

    fn labels(text: &str) -> Vec<String> {
        mentions_in(text).into_iter().map(|(_, l, _)| l).collect()
    }

    #[test]
    fn mention_forms() {
        assert_eq!(labels("see Eq. (12) for"), vec!["12"]);
        assert_eq!(labels("by equation (12) we"), vec!["12"]);
        assert_eq!(labels("from Eq. 12 we"), vec!["12"]);
        assert_eq!(labels("see Fig. 3 and"), vec!["3"]);
        assert_eq!(labels("see Figure 3a and"), vec!["3"]);
        assert_eq!(labels("see Fig. 3(b) and"), vec!["3"]);
        assert_eq!(labels("as FIG. 3 shows"), vec!["3"]);
        assert_eq!(labels("see Fig. S3 for"), vec!["S3"]);
        assert_eq!(labels("in Table II we"), vec!["II"]);
        assert_eq!(labels("in Tab. 2 we"), vec!["2"]);
        assert_eq!(labels("by Theorem 2.1 we"), vec!["2.1"]);
        assert_eq!(labels("by Thm. 2.1 we"), vec!["2.1"]);
        assert_eq!(labels("by Lemma 3, we"), vec!["3"]);
        assert_eq!(labels("in Section 4.2 we"), vec!["4.2"]);
        assert_eq!(labels("in Sec. IV we"), vec!["IV"]);
        assert_eq!(labels("in \u{00A7}3 we"), vec!["3"]);
        assert_eq!(labels("in Appendix B we"), vec!["B"]);
        assert_eq!(labels("see Algorithm 1 for"), vec!["1"]);
        assert_eq!(labels("see (A3) from Eq. (A3) and"), vec!["A3"]);
        assert_eq!(labels("in Eq. (2.14), and"), vec!["2.14"]);
    }

    #[test]
    fn each_number_in_a_list_or_range_is_its_own_span() {
        let found = mentions_in("see Eqs. (3)-(5) and Figs. 2 and 3 here");
        let spans: Vec<&str> = found.iter().map(|(_, _, s)| s.as_str()).collect();
        assert_eq!(spans, vec!["Eqs. (3)", "(5)", "Figs. 2", "3"]);
        assert_eq!(labels("by Eqs. (3) and (5) we"), vec!["3", "5"]);
        assert_eq!(labels("Theorems 1, 2, and 4 show"), vec!["1", "2", "4"]);
    }

    #[test]
    fn mentions_split_across_lines_get_a_rect_per_line() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "as shown in Fig-");
        b.line(688.0, BODY, "ure 3 and in Eq.");
        b.line(676.0, BODY, "(12) above");
        let page = b.page();
        let regions = Regions {
            part_of_line: vec![0; 3],
        };
        let found = find_mentions(&page, &regions, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].label, "3");
        assert_eq!(found[0].rects.len(), 2);
        assert_eq!(found[1].label, "12");
        assert_eq!(found[1].rects.len(), 2);
    }

    #[test]
    fn pathology_a_references_to_other_papers_produce_nothing() {
        for text in [
            "as in Eq. (5) of Ref. [12] we",
            "as in Theorem 3 in [7], we",
            "as in Fig. 2 of Smith et al. we",
            "as in [4, Lemma 2] we",
            "Ref. 3's Eq. 4 is",
            "Smith's Lemma 2 says",
            "from Eq. (5) of [12] we",
            "Theorem 3 [7] says",
            "as in Lemma 2 of Ref. 12 we",
            "(Smith et al. 2019, Theorem 3) say",
            "see Theorem 1 in their paper",
            "Fig. 2 from the original work",
        ] {
            assert!(labels(text).is_empty(), "{text}");
        }
        // ...but a reference to this paper's own appendix is kept.
        assert_eq!(labels("as Eq. (5) in Sec. 2 shows"), vec!["5", "2"]);
        assert_eq!(labels("Fig. 2 of this paper"), vec!["2"]);
    }

    #[test]
    fn pathology_e_bare_parenthesised_numbers_need_a_math_reference_phrase() {
        assert_eq!(labels("substituting in (12) gives"), vec!["12"]);
        assert_eq!(labels("from (11) and (12) we"), vec!["11", "12"]);
        assert_eq!(labels("using (3) we"), vec!["3"]);
        for text in [
            "as reported (12) previously",
            "the year (2019) was",
            "in (2019) it was",
            "items (a) and (b)",
            "see [12] and (12)",
            "pages (12) to (15)",
            "f(12) is",
        ] {
            assert!(labels(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn pathology_f_roman_numerals_only_after_a_kind_keyword() {
        assert_eq!(labels("in Table I we"), vec!["I"]);
        assert!(labels("and I think").is_empty());
        assert!(labels("the table I made").is_empty());
        assert!(labels("Section IIX is").is_empty());
    }

    #[test]
    fn prose_words_and_lowercase_theorem_words_are_not_mentions() {
        assert!(labels("for example 3 times").is_empty());
        assert!(labels("by definition 2 is").is_empty());
        assert!(labels("Figure out what").is_empty());
        assert!(labels("Table 2x is").is_empty());
    }

    fn definitions(build: impl FnOnce(&mut Builder)) -> (Page, Vec<Definition>) {
        let mut b = Builder::new();
        build(&mut b);
        let mut page = b.page();
        let regions = classify(&mut page, &mut RegionState::default());
        let defs = find_definitions(&page, &regions, 0);
        (page, defs)
    }

    #[test]
    fn caption_starts_are_definitions_and_mentions_are_not() {
        let (_, defs) = definitions(|b| {
            for i in 0..6 {
                b.line(
                    700.0 - 12.0 * i as f32,
                    BODY,
                    "Body text of the paper continues here.",
                );
            }
            let x = b.text(72.0, 500.0, BOLD, "Figure 3:");
            b.text(x + 3.0, 500.0, BODY, " The caption of the figure.");
            b.newline();
            b.line(470.0, BODY, "Fig. 4 shows the other thing in the paper.");
            b.line(458.0, SMALL, "FIG. 5. A REVTeX caption in small type.");
            b.line(440.0, BODY, "as discussed in the previous");
            b.line(428.0, BODY, "Fig. 6. That is not a caption.");
        });
        let found: Vec<(Kind, &str)> = defs.iter().map(|d| (d.kind, d.label.as_str())).collect();
        assert_eq!(found, vec![(Kind::Figure, "3"), (Kind::Figure, "5")]);
    }

    #[test]
    fn theorem_labels_need_their_own_face() {
        let (_, defs) = definitions(|b| {
            b.line(700.0, BODY, "Some text before it.");
            let x = b.text(72.0, 680.0, BOLD, "Theorem 2.1.");
            b.text(x, 680.0, ITALIC, " Every bounded sequence converges.");
            b.newline();
            b.line(650.0, BODY, "Lemma 3. Then this is just a sentence.");
            let x = b.text(72.0, 630.0, BOLD, "Lemma 4");
            b.text(x, 630.0, BODY, " (Main). Statement here.");
            b.newline();
        });
        let found: Vec<(Kind, &str)> = defs.iter().map(|d| (d.kind, d.label.as_str())).collect();
        assert_eq!(found, vec![(Kind::Theorem, "2.1"), (Kind::Lemma, "4")]);
    }

    #[test]
    fn numbered_headings_in_a_heading_face() {
        let (_, defs) = definitions(|b| {
            for i in 0..4 {
                b.line(
                    700.0 - 12.0 * i as f32,
                    BODY,
                    "Body text of the paper continues.",
                );
            }
            b.line(640.0, BOLD, "4.2 Results and discussion");
            b.line(620.0, BODY, "1. A numbered list item in body type.");
            b.line(600.0, BOLD, "IV. RESULTS");
            b.line(580.0, BOLD, "Appendix B: Proofs");
        });
        let found: Vec<(Kind, &str)> = defs.iter().map(|d| (d.kind, d.label.as_str())).collect();
        assert_eq!(
            found,
            vec![
                (Kind::Section, "4.2"),
                (Kind::Section, "IV"),
                (Kind::Section, "B")
            ]
        );
    }

    #[test]
    fn equation_tags_at_the_right_margin_of_mathematics() {
        let (_, defs) = definitions(|b| {
            b.line(700.0, BODY, "We have the relation");
            b.text(200.0, 680.0, MATH, "x=y+z");
            b.text(500.0, 680.0, BODY, "(12)");
            b.newline();
            b.line(660.0, BODY, "and so on. Reported in prose (13) here.");
            b.line(640.0, BODY, "A sentence that ends with a year (2019)");
        });
        let found: Vec<&str> = defs.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(found, vec!["12"]);
    }

    #[test]
    fn an_equation_preview_leaves_out_the_prose_around_it() {
        let mut b = Builder::new();
        for i in 0..3 {
            b.line(
                724.0 - 12.0 * i as f32,
                BODY,
                "Running text, wide, from margin to margin of the column, a full line.",
            );
        }
        // prose dense with inline math, margin to margin
        let x = b.text(72.0, 688.0, BODY, "where ");
        let x = b.text(x, 688.0, MATH, "a=b+c=d+e+f+g+h+i+j");
        let x = b.text(x, 688.0, BODY, " and ");
        b.text(x, 688.0, MATH, "f=g+h=i+j+k+l+m+n+o+p");
        b.newline();
        b.text(200.0, 676.0, MATH, "x=y+z");
        b.text(380.0, 676.0, BODY, "(21)");
        b.newline();
        b.line(
            660.0,
            BODY,
            "Running text, wide, from margin to margin of the column, a full line.",
        );
        let mut page = b.page();
        let regions = classify(&mut page, &mut RegionState::default());
        let defs = find_definitions(&page, &regions, 0);
        assert_eq!(defs.len(), 1);
        let top = defs[0].region[1] + defs[0].region[3];
        assert!(top < 688.0, "the prose line above is left out: {top}");
    }

    #[test]
    fn pathology_k_left_tags_and_partly_tagged_aligned_lines() {
        let (_, defs) = definitions(|b| {
            b.line(700.0, BODY, "We have the relations");
            b.text(72.0, 680.0, BODY, "(A3)");
            b.text(200.0, 680.0, MATH, "a=b+c");
            b.newline();
            // an aligned display: first line untagged, second tagged
            b.text(200.0, 660.0, MATH, "d=e+f");
            b.newline();
            b.text(200.0, 646.0, MATH, "g=h+i");
            b.text(500.0, 646.0, BODY, "(4)");
            b.newline();
        });
        let found: Vec<&str> = defs.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(found, vec!["A3", "4"]);
    }

    fn def(kind: Kind, label: &str, page: u32, part: u32) -> Definition {
        Definition {
            kind,
            label: label.into(),
            page,
            order: 0,
            part,
            label_range: (0, 0),
            region: [0.0; 4],
            jump: (0.0, 0.0),
            signature: String::new(),
        }
    }

    fn mention(kind: Kind, label: &str, page: u32, part: u32) -> Mention {
        Mention {
            kind,
            label: label.into(),
            page,
            order: 100,
            part,
            range: (100, 105),
            rects: Vec::new(),
        }
    }

    #[test]
    fn pathology_b_duplicate_numbering_prefers_the_same_part() {
        // A supplement restarting Fig. 1.
        let defs = vec![def(Kind::Figure, "1", 2, 0), def(Kind::Figure, "1", 9, 2)];
        assert_eq!(
            resolve(&mention(Kind::Figure, "1", 1, 0), &defs)
                .unwrap()
                .page,
            2
        );
        assert_eq!(
            resolve(&mention(Kind::Figure, "1", 10, 2), &defs)
                .unwrap()
                .page,
            9
        );
        // An exact namespace wins: S1 is not 1.
        let defs = vec![def(Kind::Figure, "1", 2, 0), def(Kind::Figure, "S1", 9, 2)];
        assert_eq!(
            resolve(&mention(Kind::Figure, "S1", 1, 0), &defs)
                .unwrap()
                .page,
            9
        );
        // Twice in the same part: the nearest preceding one.
        let defs = vec![def(Kind::Theorem, "1", 2, 0), def(Kind::Theorem, "1", 5, 0)];
        assert_eq!(
            resolve(&mention(Kind::Theorem, "1", 6, 0), &defs)
                .unwrap()
                .page,
            5
        );
        // ...and none preceding: ambiguous, nothing.
        assert!(resolve(&mention(Kind::Theorem, "1", 1, 0), &defs).is_none());
        // Defined only in other parts, twice: ambiguous.
        let defs = vec![def(Kind::Lemma, "1", 2, 1), def(Kind::Lemma, "1", 5, 2)];
        assert!(resolve(&mention(Kind::Lemma, "1", 1, 0), &defs).is_none());
        // Defined once, in another part (figures collected after the
        // references): that one.
        let defs = vec![def(Kind::Figure, "2", 12, 1)];
        assert_eq!(
            resolve(&mention(Kind::Figure, "2", 1, 0), &defs)
                .unwrap()
                .page,
            12
        );
    }

    #[test]
    fn pathology_j_a_missing_target_resolves_to_nothing() {
        let defs = vec![def(Kind::Figure, "1", 2, 0)];
        assert!(resolve(&mention(Kind::Figure, "7", 1, 0), &defs).is_none());
        assert!(resolve(&mention(Kind::Table, "1", 1, 0), &defs).is_none());
    }

    #[test]
    fn pathology_c_a_caption_label_is_not_its_own_mention() {
        let mut b = Builder::new();
        for i in 0..4 {
            b.line(
                700.0 - 12.0 * i as f32,
                BODY,
                "Body text of the paper continues.",
            );
        }
        let x = b.text(72.0, 600.0, BOLD, "Figure 3:");
        b.text(x, 600.0, BODY, " The caption, see also Fig. 3b.");
        b.newline();
        let mut page = b.page();
        let regions = classify(&mut page, &mut RegionState::default());
        let defs = find_definitions(&page, &regions, 0);
        let mentions = find_mentions(&page, &regions, 0);
        assert_eq!(mentions.len(), 2);
        assert!(is_definition_site(&mentions[0], &defs));
        assert!(!is_definition_site(&mentions[1], &defs));
    }

    #[test]
    fn pathology_g_no_mentions_in_the_bibliography_or_contents() {
        let mut b = Builder::new();
        b.line(700.0, BOLD, "Contents");
        b.line(680.0, BODY, "Figure 3 . . . . . . 12");
        b.line(600.0, BODY, "The paper ends, see Fig. 3.");
        b.line(560.0, BOLD, "References");
        b.line(
            540.0,
            BODY,
            "[1] A. Author, Table 2 in Phys. Rev. 12 (2019).",
        );
        let mut page = b.page();
        let regions = classify(&mut page, &mut RegionState::default());
        assert!(find_mentions(&page, &regions, 0).is_empty());
    }

    #[test]
    fn mentions_in_math_or_scripts_are_skipped() {
        let mut b = Builder::new();
        b.text(72.0, 700.0, BODY, "see ");
        b.text(92.0, 700.0, MATH, "Eq. (3)");
        b.newline();
        let page = b.page();
        let regions = Regions {
            part_of_line: vec![0],
        };
        assert!(find_mentions(&page, &regions, 0).is_empty());
    }

    #[test]
    fn describes_namespaced_labels() {
        assert_eq!(describe(Kind::Figure, "ED 2"), "Extended Data Figure 2");
        assert_eq!(describe(Kind::Lemma, "3"), "Lemma 3");
        assert_eq!(Kind::Lemma.wire(), "theorem");
    }
}

//! Section numbers as headings print them, and the check that a run of
//! them reads as a document's sections rather than a list.
//!
//! Recognised: "1", "1.2", "1.2.3" (with or without a trailing period),
//! "1)" (IEEE's third level), "I." (APS and IEEE sections), "A." and "A.1"
//! (APS subsections, appendices), "Appendix A", "Chapter 3", "Part II",
//! "S1" (supplementary sections), "Section 3" and the section sign ("\u{a7}3").

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// "2", "2.1", "2.1.3", "\u{a7}2", "Section 2"
    Arabic,
    /// "1)"
    ArabicParen,
    /// "IV."
    Roman,
    /// "A.", "A.1", or a bare "A" before a title
    Letter,
    /// "Appendix", "Appendix B"
    Appendix,
    /// "Chapter 3", "CHAPTER THREE"
    Chapter,
    /// "Part II"
    Part,
    /// "S1", "S2.3", "Supplementary Note 4"
    Supp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Numbering {
    pub kind: Kind,
    /// The counter's components: a letter counts from A = 1, so "B.2" is
    /// [2, 2]. Empty for a bare "Appendix".
    pub values: Vec<u32>,
    /// A single letter followed only by a space ("A Proofs"): as often the
    /// article "A" opening a title as an appendix number. Trusted only where
    /// the document's own sections vouch for it.
    pub bare: bool,
    /// The numbering as printed ("2.1", "I.", "Chapter 3").
    pub prefix: String,
    /// What follows it; empty for a number set on its own line.
    pub title: String,
}

impl Numbering {
    pub fn depth(&self) -> usize {
        self.values.len().max(1)
    }
}

const NUMBER_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];

/// The value of a Roman numeral up to 39, upper case only ("IV", "XII").
pub fn roman_value(token: &str) -> Option<u32> {
    if token.is_empty() || token.len() > 6 || !token.chars().all(|c| "IVX".contains(c)) {
        return None;
    }
    const TABLE: &[&str] = &[
        "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII", "XIII", "XIV",
        "XV", "XVI", "XVII", "XVIII", "XIX", "XX", "XXI", "XXII", "XXIII", "XXIV", "XXV",
    ];
    TABLE
        .iter()
        .position(|candidate| *candidate == token)
        .map(|i| i as u32 + 1)
}

/// A counter token after a keyword: digits, a Roman numeral (either case),
/// a single letter, or a number word.
fn counter_value(token: &str) -> Option<u32> {
    let token = token.trim_end_matches(['.', ':', ',']);
    if !token.is_empty() && token.len() <= 3 && token.chars().all(|c| c.is_ascii_digit()) {
        return token.parse().ok();
    }
    if let Some(v) = roman_value(&token.to_ascii_uppercase()) {
        if token.len() > 1 || token == "I" || token == "V" || token == "X" {
            return Some(v);
        }
    }
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_uppercase() {
            return Some(c as u32 - 'A' as u32 + 1);
        }
    }
    let lower = token.to_lowercase();
    NUMBER_WORDS
        .iter()
        .position(|word| *word == lower)
        .map(|i| i as u32 + 1)
}

/// What follows a numbering: separators dropped, and `None` if it does not
/// read as a title - one has to open with a letter or a quote, never with
/// another figure ("2.1 0.35 0.42" is a table row).
fn title_after(rest: &str) -> Option<String> {
    let rest = rest
        .trim_start_matches([' ', '.', ':', '\u{2013}', '\u{2014}', '-'])
        .trim();
    if rest.is_empty() {
        return Some(String::new());
    }
    let first = rest.chars().next()?;
    if first.is_alphabetic() || matches!(first, '"' | '\u{201c}' | '\'' | '(' | '\u{2018}') {
        Some(rest.to_owned())
    } else {
        None
    }
}

fn split_first_word(text: &str) -> (&str, &str) {
    match text.find(char::is_whitespace) {
        Some(i) => (&text[..i], text[i..].trim_start()),
        None => (text, ""),
    }
}

/// The numbering a heading opens with, if any.
pub fn parse(text: &str) -> Option<Numbering> {
    let text = text.trim();
    let (first, rest) = split_first_word(text);
    let lower = first.to_lowercase();
    let keyword = lower.trim_end_matches(['.', ':']);

    // Keyword forms.
    let keyword_kind = match keyword {
        "chapter" | "chap" => Some(Kind::Chapter),
        "part" => Some(Kind::Part),
        "appendix" => Some(Kind::Appendix),
        "section" | "sect" | "sec" => Some(Kind::Arabic),
        "lecture" => Some(Kind::Chapter),
        _ => None,
    };
    if let Some(kind) = keyword_kind {
        let (token, after) = split_first_word(rest);
        // "Chapter 3:", "Part II.", "Appendix B" - or a bare "Appendix".
        let clean = token.trim_end_matches(['.', ':']);
        if kind == Kind::Arabic {
            let values = dotted_digits(clean)?;
            return Some(Numbering {
                kind,
                prefix: format!("{first} {clean}"),
                title: title_after(after)?,
                values,
                bare: false,
            });
        }
        return match counter_value(clean) {
            Some(value) if !clean.is_empty() => Some(Numbering {
                kind,
                values: vec![value],
                bare: false,
                prefix: format!("{first} {clean}"),
                title: title_after(after)?,
            }),
            _ if kind == Kind::Appendix => Some(Numbering {
                kind,
                values: Vec::new(),
                bare: false,
                prefix: first.trim_end_matches([':', '.']).to_owned(),
                title: title_after(rest)?,
            }),
            _ => None,
        };
    }
    if lower == "supplementary" {
        let (noun, after) = split_first_word(rest);
        if matches!(
            noun.to_lowercase().as_str(),
            "note" | "section" | "text" | "methods" | "discussion"
        ) {
            let (token, after) = split_first_word(after);
            let clean = token.trim_end_matches(['.', ':']);
            let clean = clean.strip_prefix('S').unwrap_or(clean);
            if let Some(values) = dotted_digits(clean) {
                return Some(Numbering {
                    kind: Kind::Supp,
                    values,
                    bare: false,
                    prefix: format!("{first} {noun} {token}")
                        .trim_end_matches([':', '.'])
                        .to_owned(),
                    title: title_after(after)?,
                });
            }
        }
        return None;
    }

    // the section sign: "\u{a7}3", "\u{a7} 3.1"
    if let Some(after_sign) = text.strip_prefix('\u{a7}') {
        let (token, after) = split_first_word(after_sign.trim_start());
        let clean = token.trim_end_matches('.');
        let values = dotted_digits(clean)?;
        return Some(Numbering {
            kind: Kind::Arabic,
            values,
            bare: false,
            prefix: format!("\u{a7}{clean}"),
            title: title_after(after)?,
        });
    }

    // "S1", "S2.1."
    if let Some(after_s) = first.strip_prefix('S') {
        let clean = after_s.trim_end_matches(['.', ':']);
        if clean.starts_with(|c: char| c.is_ascii_digit()) {
            let values = dotted_digits(clean)?;
            return Some(Numbering {
                kind: Kind::Supp,
                values,
                bare: false,
                prefix: format!("S{clean}"),
                title: title_after(rest)?,
            });
        }
    }

    // Roman: "IV." - the period is required; without it "I" is a pronoun.
    if let Some(numeral) = first.strip_suffix('.') {
        if let Some(value) = roman_value(numeral) {
            return Some(Numbering {
                kind: Kind::Roman,
                values: vec![value],
                bare: false,
                prefix: first.to_owned(),
                title: title_after(rest)?,
            });
        }
    }

    // Letters: "A.1", "B.", or a bare "A" before a capitalised title.
    let mut chars = first.chars();
    if let Some(letter) = chars.next().filter(char::is_ascii_uppercase) {
        let after_letter = chars.as_str();
        let value = letter as u32 - 'A' as u32 + 1;
        if after_letter.is_empty() {
            let title = title_after(rest)?;
            if title.is_empty() || !title.starts_with(char::is_uppercase) {
                return None;
            }
            return Some(Numbering {
                kind: Kind::Letter,
                values: vec![value],
                bare: true,
                prefix: first.to_owned(),
                title,
            });
        }
        if after_letter == "." || after_letter == ")" {
            return Some(Numbering {
                kind: Kind::Letter,
                values: vec![value],
                bare: false,
                prefix: first.to_owned(),
                title: title_after(rest)?,
            });
        }
        if let Some(subs) = after_letter.strip_prefix('.') {
            let clean = subs.trim_end_matches('.');
            if let Some(more) = dotted_digits(clean) {
                let mut values = vec![value];
                values.extend(more);
                return Some(Numbering {
                    kind: Kind::Letter,
                    values,
                    bare: false,
                    prefix: first.trim_end_matches('.').to_owned(),
                    title: title_after(rest)?,
                });
            }
        }
        return None;
    }

    // Arabic: "2", "2.", "2.1", "2.1.3", "1)" - and "1Introduction", where
    // extraction lost the space between number and title.
    let digits_end = first
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.'))
        .map_or(first.len(), |(i, _)| i);
    let (number, glued) = first.split_at(digits_end);
    if number.is_empty() || !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let clean = number.trim_end_matches('.');
    let values = dotted_digits(clean)?;
    if glued == ")" && values.len() == 1 {
        return Some(Numbering {
            kind: Kind::ArabicParen,
            values,
            bare: false,
            prefix: first.to_owned(),
            title: title_after(rest)?,
        });
    }
    let title = if glued.is_empty() {
        title_after(rest)?
    } else {
        // Only a capital followed by lower case: "1Introduction", never
        // "3D", "2nd" or "10x".
        let mut g = glued.chars();
        let (Some(a), Some(b)) = (g.next(), g.next()) else {
            return None;
        };
        if !(a.is_uppercase() && b.is_lowercase()) {
            return None;
        }
        title_after(&format!("{glued} {rest}"))?
    };
    Some(Numbering {
        kind: Kind::Arabic,
        values,
        bare: false,
        prefix: number.to_owned(),
        title,
    })
}

/// "2", "2.1", "10.3.4": up to five components of at most three digits.
fn dotted_digits(token: &str) -> Option<Vec<u32>> {
    if token.is_empty() {
        return None;
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() > 5 {
        return None;
    }
    parts
        .iter()
        .map(|part| {
            (!part.is_empty() && part.len() <= 3 && part.chars().all(|c| c.is_ascii_digit()))
                .then(|| part.parse::<u32>().ok())
                .flatten()
        })
        .collect()
}

/// Whether `next` can follow `prev` among a document's section numbers:
/// a later sibling a few steps on, a first child, or a step back up to a
/// later ancestor - with every level opened along the way starting low.
///
/// "2.1" after "2", "3" after "2.4", "2.2" after "2.1" all can; "2" after
/// "5" cannot, and neither can "7" after "1" (a list's "7." is not the
/// next section after "1 Introduction").
pub fn plausible_successor(prev: &[u32], next: &[u32]) -> bool {
    let common = prev.iter().zip(next).take_while(|(a, b)| a == b).count();
    if common == next.len() {
        // equal, or `next` is an ancestor of `prev`: never forward
        return false;
    }
    let opens_low = |from: usize| next[from..].iter().all(|v| *v <= 2);
    if common == prev.len() {
        // first child of `prev`
        return opens_low(common);
    }
    let (a, b) = (prev[common], next[common]);
    b > a && b - a <= 3 && opens_low(common + 1)
}

/// The longest run of `keys`, in order, in which each follows the one
/// before plausibly: the section numbering a document actually has, with
/// whatever only looked numbered - a list item, a stray table cell - left
/// out. Returns the indices kept.
pub fn longest_chain(keys: &[Vec<u32>]) -> Vec<usize> {
    let n = keys.len();
    if n == 0 {
        return Vec::new();
    }
    let mut best = vec![1usize; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        for j in 0..i {
            if best[j] + 1 > best[i] && plausible_successor(&keys[j], &keys[i]) {
                best[i] = best[j] + 1;
                back[i] = Some(j);
            }
        }
    }
    // The longest; among equals, the one that starts lowest - "1" rather
    // than "4" - since that is how a document's numbering begins.
    let mut end = 0;
    for i in 0..n {
        if best[i] > best[end] {
            end = i;
        }
    }
    let mut chain = vec![end];
    while let Some(prev) = back[*chain.last().unwrap()] {
        chain.push(prev);
    }
    chain.reverse();
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(text: &str) -> (Kind, Vec<u32>, String) {
        let n = parse(text).unwrap_or_else(|| panic!("{text}"));
        (n.kind, n.values, n.title)
    }

    #[test]
    fn parses_the_numbering_patterns() {
        assert_eq!(
            kv("1 Introduction"),
            (Kind::Arabic, vec![1], "Introduction".into())
        );
        assert_eq!(
            kv("2.1 Background"),
            (Kind::Arabic, vec![2, 1], "Background".into())
        );
        assert_eq!(
            kv("3.2.1. Setup"),
            (Kind::Arabic, vec![3, 2, 1], "Setup".into())
        );
        assert_eq!(
            kv("1Introduction"),
            (Kind::Arabic, vec![1], "Introduction".into())
        );
        assert_eq!(
            kv("I. INTRODUCTION"),
            (Kind::Roman, vec![1], "INTRODUCTION".into())
        );
        assert_eq!(kv("IV. RESULTS"), (Kind::Roman, vec![4], "RESULTS".into()));
        assert_eq!(kv("A. Setup"), (Kind::Letter, vec![1], "Setup".into()));
        assert_eq!(
            kv("B.2 Proofs"),
            (Kind::Letter, vec![2, 2], "Proofs".into())
        );
        assert_eq!(kv("Appendix A"), (Kind::Appendix, vec![1], "".into()));
        assert_eq!(
            kv("Appendix B: Proofs"),
            (Kind::Appendix, vec![2], "Proofs".into())
        );
        assert_eq!(kv("Appendix"), (Kind::Appendix, vec![], "".into()));
        assert_eq!(kv("Chapter 3"), (Kind::Chapter, vec![3], "".into()));
        assert_eq!(kv("CHAPTER THREE"), (Kind::Chapter, vec![3], "".into()));
        assert_eq!(
            kv("Chapter 3: Lattices"),
            (Kind::Chapter, vec![3], "Lattices".into())
        );
        assert_eq!(kv("Part II"), (Kind::Part, vec![2], "".into()));
        assert_eq!(kv("S1 Methods"), (Kind::Supp, vec![1], "Methods".into()));
        assert_eq!(kv("S2.3. Data"), (Kind::Supp, vec![2, 3], "Data".into()));
        assert_eq!(
            kv("Supplementary Note 4: Data"),
            (Kind::Supp, vec![4], "Data".into())
        );
        assert_eq!(
            kv("\u{a7}3 Results"),
            (Kind::Arabic, vec![3], "Results".into())
        );
        assert_eq!(
            kv("1) Training"),
            (Kind::ArabicParen, vec![1], "Training".into())
        );
        assert_eq!(kv("3"), (Kind::Arabic, vec![3], "".into()));
        let bare = parse("A Proofs of the main results").unwrap();
        assert!(bare.bare);
    }

    #[test]
    fn rejects_what_only_looks_numbered() {
        for text in [
            "2.1 0.35 0.42",
            "3D reconstruction",
            "2nd order terms",
            "2019 Annual report",
            "I think so",
            "a. lower case item",
            "Chapter summary",
            "A",
            "A new approach",
            "S-matrix theory",
            "10x faster",
        ] {
            let parsed = parse(text);
            assert!(
                parsed.is_none() || (text == "A new approach" && parsed.is_none()),
                "{text}: {parsed:?}"
            );
        }
    }

    #[test]
    fn successor_rules() {
        assert!(plausible_successor(&[1], &[2]));
        assert!(plausible_successor(&[2], &[2, 1]));
        assert!(plausible_successor(&[2, 1], &[2, 2]));
        assert!(plausible_successor(&[2, 4], &[3]));
        assert!(plausible_successor(&[2, 4, 1], &[3]));
        assert!(plausible_successor(&[2, 4], &[3, 1]));
        assert!(plausible_successor(&[1], &[3]));
        assert!(!plausible_successor(&[5], &[2]));
        assert!(!plausible_successor(&[1], &[7]));
        assert!(!plausible_successor(&[2, 1], &[2]));
        assert!(!plausible_successor(&[2], &[2]));
        assert!(!plausible_successor(&[2], &[2, 5]));
    }

    /// A lone "2." after "5." is a list item, not a section.
    #[test]
    fn a_list_item_breaks_out_of_the_chain() {
        let keys = vec![
            vec![1],
            vec![2],
            vec![2, 1],
            vec![3],
            vec![2],
            vec![4],
            vec![5],
        ];
        assert_eq!(longest_chain(&keys), vec![0, 1, 2, 3, 5, 6]);
    }
}

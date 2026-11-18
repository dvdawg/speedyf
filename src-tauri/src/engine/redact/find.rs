//! Search-and-redact: every occurrence of a phrase, or of a kind of personal
//! data, on a page's text.
//!
//! Runs on the page's raw character stream (one char per PDFium character),
//! with whitespace runs collapsed so a phrase or a phone number broken across
//! a line still matches. Matches come back as character ranges; the worker
//! turns them into rectangles with the same box merge search uses.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FindQuery {
    /// A literal phrase; empty for none.
    #[serde(default)]
    pub phrase: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_words: bool,
    /// Pattern kinds: "email", "phone", "ssn", "card", "date".
    #[serde(default)]
    pub patterns: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedactMatchDto {
    pub page: u32,
    pub start: u32,
    pub len: u32,
    /// The matched text, whitespace collapsed.
    pub text: String,
    /// "phrase" or the pattern kind.
    pub kind: String,
    /// Display-space rectangles `[x, y, w, h]`, y up, one per line.
    pub rects: Vec<[f32; 4]>,
}

/// A match on one page: character range into the raw text, and its kind.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub start: usize,
    pub len: usize,
    pub text: String,
    pub kind: &'static str,
}

fn pattern(kind: &str) -> Option<&'static Regex> {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    static PHONE: OnceLock<Regex> = OnceLock::new();
    static SSN: OnceLock<Regex> = OnceLock::new();
    static CARD: OnceLock<Regex> = OnceLock::new();
    static DATE: OnceLock<Regex> = OnceLock::new();
    let build = |re: &str| Regex::new(re).expect("valid pattern");
    Some(match kind {
        "email" => EMAIL.get_or_init(|| {
            build(r"[A-Za-z0-9._%+\-]+ ?@ ?[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}")
        }),
        "phone" => PHONE.get_or_init(|| {
            build(r"(?:\+?1[ .\-]?)?(?:\(\d{3}\) ?|\d{3}[ .\-])\d{3}[ .\-]\d{4}|\+\d{1,3}(?:[ .\-]\d{2,4}){2,4}")
        }),
        "ssn" => SSN.get_or_init(|| build(r"\d{3}-\d{2}-\d{4}|\d{3} \d{2} \d{4}")),
        "card" => CARD.get_or_init(|| build(r"\d(?:[ \-]?\d){12,18}")),
        "date" => DATE.get_or_init(|| {
            let month = "(?:Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|June?|July?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?)\\.?";
            build(&format!(
                r"\d{{1,2}}[/\-.]\d{{1,2}}[/\-.](?:\d{{4}}|\d{{2}})|\d{{4}}-\d{{2}}-\d{{2}}|{month} \d{{1,2}}(?:st|nd|rd|th)?,? \d{{4}}|\d{{1,2}} {month},? \d{{4}}"
            ))
        }),
        _ => return None,
    })
}

/// Luhn checksum over the digits of `s`.
pub fn luhn(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                d
            }
        })
        .sum();
    sum % 10 == 0
}

fn ssn_valid(s: &str) -> bool {
    let d: String = s.chars().filter(char::is_ascii_digit).collect();
    if d.len() != 9 {
        return false;
    }
    let (area, group, serial) = (&d[0..3], &d[3..5], &d[5..9]);
    area != "000" && area != "666" && !area.starts_with('9') && group != "00" && serial != "0000"
}

fn date_valid(s: &str) -> bool {
    // Numeric dates: some field must be a month (1-12) and some a day (1-31).
    let parts: Vec<u32> = s
        .split(['/', '-', '.'])
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    if parts.len() != 3 || !s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    if s.len() == 10 && s.as_bytes()[4] == b'-' {
        return (1..=12).contains(&parts[1]) && (1..=31).contains(&parts[2]);
    }
    let (a, b) = (parts[0], parts[1]);
    ((1..=12).contains(&a) && (1..=31).contains(&b))
        || ((1..=31).contains(&a) && (1..=12).contains(&b))
}

/// The page text with whitespace collapsed, and for each kept char its index
/// in `raw`.
fn normalize(raw: &str) -> (String, Vec<usize>) {
    let mut norm = String::with_capacity(raw.len());
    let mut map = Vec::with_capacity(raw.len());
    let mut last_space = true;
    for (i, c) in raw.chars().enumerate() {
        let space = c.is_whitespace() || c == '\u{a0}' || c == '\u{0}';
        if space {
            if !last_space {
                norm.push(' ');
                map.push(i);
            }
            last_space = true;
        } else {
            norm.push(c);
            map.push(i);
            last_space = false;
        }
    }
    (norm, map)
}

/// Every match of `query` in a page's raw text.
pub fn find_in_text(raw: &str, query: &FindQuery) -> Vec<Hit> {
    let (norm, map) = normalize(raw);
    // Byte offset -> char index in `norm`.
    let char_at: Vec<usize> = {
        let mut v = vec![0; norm.len() + 1];
        let mut ci = 0;
        for (bi, _) in norm.char_indices() {
            v[bi] = ci;
            ci += 1;
        }
        v[norm.len()] = ci;
        v
    };
    let mut hits: Vec<Hit> = Vec::new();
    let push = |m: regex::Match<'_>, kind: &'static str, hits: &mut Vec<Hit>| {
        let (s, e) = (char_at[m.start()], char_at[m.end()]);
        if e <= s {
            return;
        }
        let start = map[s];
        let end = map[e - 1] + 1;
        hits.push(Hit {
            start,
            len: end - start,
            text: m.as_str().to_string(),
            kind,
        });
    };
    let boundary_ok = |m: &regex::Match<'_>, word: bool| {
        let before = norm[..m.start()].chars().next_back();
        let after = norm[m.end()..].chars().next();
        let bad = |c: Option<char>| {
            c.is_some_and(|c| {
                if word {
                    c.is_alphanumeric() || c == '_'
                } else {
                    c.is_ascii_digit()
                }
            })
        };
        !bad(before) && !bad(after)
    };

    let phrase = query
        .phrase
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !phrase.is_empty() {
        let re = RegexBuilder::new(&regex::escape(&phrase))
            .case_insensitive(!query.case_sensitive)
            .build();
        if let Ok(re) = re {
            for m in re.find_iter(&norm) {
                if !query.whole_words || boundary_ok(&m, true) {
                    push(m, "phrase", &mut hits);
                }
            }
        }
    }
    for kind in &query.patterns {
        let kind: &'static str = match kind.as_str() {
            "email" => "email",
            "phone" => "phone",
            "ssn" => "ssn",
            "card" => "card",
            "date" => "date",
            _ => continue,
        };
        let Some(re) = pattern(kind) else { continue };
        for m in re.find_iter(&norm) {
            let ok = match kind {
                "email" => boundary_ok(&m, false),
                "card" => boundary_ok(&m, false) && luhn(m.as_str()),
                "ssn" => boundary_ok(&m, false) && ssn_valid(m.as_str()),
                "date" => boundary_ok(&m, false) && date_valid(m.as_str()),
                _ => boundary_ok(&m, false),
            };
            if ok {
                push(m, kind, &mut hits);
            }
        }
    }
    hits.sort_by_key(|h| (h.start, std::cmp::Reverse(h.len)));
    // Drop matches wholly inside an earlier one (a phone number inside a
    // card number, say).
    let mut out: Vec<Hit> = Vec::with_capacity(hits.len());
    for h in hits {
        if out
            .last()
            .is_some_and(|p| h.start >= p.start && h.start + h.len <= p.start + p.len)
        {
            continue;
        }
        out.push(h);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(raw: &str, patterns: &[&str]) -> Vec<(String, &'static str)> {
        let q = FindQuery {
            patterns: patterns.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        find_in_text(raw, &q)
            .into_iter()
            .map(|h| (raw.chars().skip(h.start).take(h.len).collect(), h.kind))
            .collect()
    }

    #[test]
    fn patterns_find_personal_data() {
        let raw = "Mail jane.doe@example.org or call (510) 555-1234.\r\nSSN 123-45-6789, card 4111 1111 1111 1111, born March 3, 1990 and 2020-01-31.";
        let found = kinds(raw, &["email", "phone", "ssn", "card", "date"]);
        let texts: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
        assert!(texts.contains(&"jane.doe@example.org"), "{found:?}");
        assert!(texts.contains(&"(510) 555-1234"), "{found:?}");
        assert!(texts.contains(&"123-45-6789"), "{found:?}");
        assert!(texts.contains(&"4111 1111 1111 1111"), "{found:?}");
        assert!(texts.contains(&"March 3, 1990"), "{found:?}");
        assert!(texts.contains(&"2020-01-31"), "{found:?}");
    }

    #[test]
    fn luhn_rejects_random_digits() {
        assert!(luhn("4111111111111111"));
        assert!(!luhn("4111111111111112"));
        let found = kinds("id 1234 5678 9012 3456 end", &["card"]);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn ssn_rules() {
        assert!(kinds("000-12-3456", &["ssn"]).is_empty());
        assert!(kinds("x123456789y 12-345-6789", &["ssn"]).is_empty());
    }

    #[test]
    fn phrase_across_line_break_maps_to_raw_indices() {
        let raw = "Dear John\r\nSmith, hello";
        let q = FindQuery {
            phrase: "john smith".into(),
            ..Default::default()
        };
        let hits = find_in_text(raw, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start, 5);
        let s: String = raw.chars().skip(hits[0].start).take(hits[0].len).collect();
        assert_eq!(s, "John\r\nSmith");
    }

    #[test]
    fn whole_words() {
        let q = FindQuery {
            phrase: "cat".into(),
            whole_words: true,
            ..Default::default()
        };
        assert_eq!(find_in_text("concatenate the cat", &q).len(), 1);
    }
}

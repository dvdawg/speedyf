//! Cite keys: `{family}{year}{word}`, as in `jarzynski1997nonequilibrium`.
//!
//! Generation is a pure function of the metadata so the same paper gets the
//! same key on every machine. Uniqueness is the caller's: it passes a `taken`
//! predicate and gets a suffixed key back when the base collides. When a key
//! is *frozen* is a store rule (projects.md §2), not this module's.

use super::types::{Author, PaperMeta};

/// Words that make a useless third segment: `the1997the` says nothing.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "on", "of", "in", "for", "to", "and", "or", "at", "by", "from", "with",
    "into", "about", "toward", "towards", "via", "is", "are", "be", "as", "its", "their", "some",
    "how", "what", "when", "why", "where", "which", "who", "do", "does", "can", "we", "you", "new",
    "note", "notes", "towards", "vs",
];

/// ASCII letters and digits only, lower-case; accents stripped by taking the
/// base letter of common Latin forms, anything else dropped. BibTeX keys may
/// hold more than this, but a key the user can type without thinking about
/// encodings is worth more than fidelity to `Ø`.
fn fold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let folded: Option<char> = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => Some('a'),
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' => Some('a'),
            'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => Some('c'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => Some('e'),
            'È' | 'É' | 'Ê' | 'Ë' | 'Ē' => Some('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'Ì' | 'Í' | 'Î' | 'Ï' => Some('i'),
            'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' => Some('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => Some('o'),
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' => Some('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'Ù' | 'Ú' | 'Û' | 'Ü' => Some('u'),
            'ý' | 'ÿ' | 'Ý' => Some('y'),
            'ß' => {
                out.push_str("ss");
                None
            }
            'ś' | 'š' | 'ş' | 'Ś' | 'Š' | 'Ş' => Some('s'),
            'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => Some('z'),
            'ł' | 'Ł' => Some('l'),
            'đ' | 'Đ' => Some('d'),
            'ř' | 'Ř' => Some('r'),
            'ť' | 'Ť' => Some('t'),
            _ => None,
        };
        if let Some(c) = folded {
            out.push(c);
        }
    }
    out
}

/// The last word of the family name, so "van der Waals" gives `waals` and a
/// double-barrelled name gives its final part. Particles are not part of a key
/// anyone remembers.
fn family_segment(authors: &[Author]) -> String {
    let family = authors
        .first()
        .map(|author| author.family.as_str())
        .unwrap_or("");
    let folded = family
        .split(|c: char| c.is_whitespace() || c == '-')
        .map(fold)
        .rfind(|part| !part.is_empty())
        .unwrap_or_default();
    if folded.is_empty() {
        "anon".to_owned()
    } else {
        folded
    }
}

fn year_segment(year: Option<i32>) -> String {
    match year {
        Some(year) if (1000..=9999).contains(&year) => year.to_string(),
        _ => "nd".to_owned(),
    }
}

/// The first title word worth keeping: not a stop word, not a bare number,
/// at least two letters after folding.
fn word_segment(title: &str) -> String {
    title
        .split(|c: char| !c.is_alphanumeric())
        .map(fold)
        .find(|word| {
            word.chars().count() >= 2
                && !STOP_WORDS.contains(&word.as_str())
                && word.chars().any(|c| c.is_ascii_alphabetic())
        })
        .unwrap_or_default()
}

/// The key the metadata implies, before any collision handling.
pub fn base_key(meta: &PaperMeta) -> String {
    let authors = meta
        .authors
        .as_ref()
        .map(|field| field.value.as_slice())
        .unwrap_or(&[]);
    let year = meta.year.as_ref().map(|field| field.value);
    let title = meta
        .title
        .as_ref()
        .map(|field| field.value.as_str())
        .unwrap_or("");
    format!(
        "{}{}{}",
        family_segment(authors),
        year_segment(year),
        word_segment(title)
    )
}

/// `base` if free, else `basea`, `baseb`, … `basez`, `baseaa`, … — the
/// first one `taken` does not claim.
pub fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    for n in 0.. {
        let candidate = format!("{base}{}", suffix(n));
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!("the suffix sequence is infinite")
}

/// 0 → "a", 25 → "z", 26 → "aa", 27 → "ab", …
fn suffix(mut n: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.iter().rev().collect()
}

/// `fold` with the first letter upper-cased: "Waals", "Gartner".
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// The part of a family name an alpha label reads: the last
/// whitespace-separated word, folded, so particles ("van der") drop out.
fn alpha_family(family: &str) -> Option<String> {
    family
        .split_whitespace()
        .map(fold)
        .rfind(|part| !part.is_empty())
        .map(|part| capitalized(&part))
}

/// A BibTeX `alpha.bst`-style label: "Jar97", "ZD24", "ABCD20", "ABC+19",
/// or, with no authors, the first significant title word ("Int21").
/// Collisions ("ZD24a") are the caller's: they depend on what else is shown.
pub fn alpha_label(authors: &[Author], year: Option<i32>, title: &str) -> String {
    let families: Vec<String> = authors
        .iter()
        .filter_map(|author| alpha_family(&author.family))
        .collect();
    let initial = |family: &String| family.chars().next().into_iter().collect::<String>();
    let head = match families.as_slice() {
        [] => {
            let word = word_segment(title);
            if word.is_empty() {
                "Anon".to_owned()
            } else {
                capitalized(&word.chars().take(3).collect::<String>())
            }
        }
        [one] => one.chars().take(3).collect(),
        some if some.len() <= 4 => some.iter().map(initial).collect(),
        many => many[..3].iter().map(initial).collect::<String>() + "+",
    };
    match year {
        Some(year) if (1000..=9999).contains(&year) => format!("{head}{:02}", year % 100),
        _ => head,
    }
}

/// A free key for this metadata.
pub fn generate(meta: &PaperMeta, taken: impl Fn(&str) -> bool) -> String {
    unique(&base_key(meta), taken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{MetaField, MetaSource};

    fn meta(family: &str, year: Option<i32>, title: &str) -> PaperMeta {
        PaperMeta {
            title: Some(MetaField {
                value: title.to_owned(),
                source: MetaSource::Crossref,
            }),
            authors: (!family.is_empty()).then(|| MetaField {
                value: vec![Author {
                    family: family.to_owned(),
                    given: "C.".to_owned(),
                }],
                source: MetaSource::Crossref,
            }),
            year: year.map(|value| MetaField {
                value,
                source: MetaSource::Crossref,
            }),
            ..PaperMeta::default()
        }
    }

    #[test]
    fn the_canonical_shape() {
        assert_eq!(
            base_key(&meta(
                "Jarzynski",
                Some(1997),
                "Nonequilibrium Equality for Free Energy Differences"
            )),
            "jarzynski1997nonequilibrium"
        );
    }

    #[test]
    fn skips_stop_words_and_folds_accents() {
        assert_eq!(
            base_key(&meta(
                "Fodor",
                Some(2016),
                "How Far from Equilibrium Is Active Matter?"
            )),
            "fodor2016far"
        );
        assert_eq!(
            base_key(&meta("Gärtner", Some(2020), "Über die Dinge")),
            "gartner2020uber"
        );
    }

    #[test]
    fn particles_are_dropped_and_missing_parts_degrade_in_place() {
        assert_eq!(
            base_key(&meta("van der Waals", Some(1873), "On the continuity")),
            "waals1873continuity"
        );
        assert_eq!(base_key(&meta("", None, "")), "anonnd");
        assert_eq!(
            base_key(&meta("Kubo", Some(0), "The 1966 Paper")),
            "kubondpaper"
        );
    }

    #[test]
    fn collisions_take_letter_suffixes_in_order() {
        let taken = |key: &str| matches!(key, "seifert2012stochastic" | "seifert2012stochastica");
        assert_eq!(
            unique("seifert2012stochastic", taken),
            "seifert2012stochasticb"
        );
        assert_eq!(unique("free", |_| false), "free");
        assert_eq!(suffix(25), "z");
        assert_eq!(suffix(26), "aa");
        assert_eq!(suffix(27), "ab");
    }

    fn people(families: &[&str]) -> Vec<Author> {
        families
            .iter()
            .map(|family| Author {
                family: (*family).to_owned(),
                given: String::new(),
            })
            .collect()
    }

    #[test]
    fn alpha_labels_by_author_count() {
        assert_eq!(
            alpha_label(&people(&["Jarzynski"]), Some(1997), ""),
            "Jar97"
        );
        assert_eq!(alpha_label(&people(&["Li"]), Some(2005), ""), "Li05");
        assert_eq!(
            alpha_label(&people(&["Zhang", "Doe"]), Some(2024), ""),
            "ZD24"
        );
        assert_eq!(
            alpha_label(&people(&["Evans", "Cohen", "Morriss"]), Some(1993), ""),
            "ECM93"
        );
        assert_eq!(
            alpha_label(
                &people(&["Adams", "Brown", "Clark", "Davis"]),
                Some(2020),
                ""
            ),
            "ABCD20"
        );
        assert_eq!(
            alpha_label(
                &people(&["Adams", "Brown", "Clark", "Davis", "Evans"]),
                Some(2019),
                ""
            ),
            "ABC+19"
        );
        assert_eq!(alpha_label(&people(&["Kubo"]), Some(2000), ""), "Kub00");
    }

    #[test]
    fn alpha_labels_drop_particles_and_fold_accents() {
        assert_eq!(
            alpha_label(&people(&["van der Waals"]), Some(1873), ""),
            "Waa73"
        );
        assert_eq!(
            alpha_label(&people(&["de Gennes", "van Kampen"]), Some(1979), ""),
            "GK79"
        );
        assert_eq!(alpha_label(&people(&["Gärtner"]), Some(2020), ""), "Gar20");
        assert_eq!(
            alpha_label(&people(&["Ørsted", "Émile"]), Some(1820), ""),
            "OE20"
        );
        assert_eq!(
            alpha_label(&people(&["  ", "Kubo"]), Some(1966), ""),
            "Kub66"
        );
    }

    #[test]
    fn alpha_labels_without_year_or_authors() {
        assert_eq!(alpha_label(&people(&["Jarzynski"]), None, ""), "Jar");
        assert_eq!(alpha_label(&people(&["Jarzynski"]), Some(0), ""), "Jar");
        assert_eq!(
            alpha_label(
                &[],
                Some(2021),
                "An Introduction to Stochastic Thermodynamics"
            ),
            "Int21"
        );
        assert_eq!(alpha_label(&[], None, "The Über Paper"), "Ube");
        assert_eq!(alpha_label(&[], None, ""), "Anon");
    }
}

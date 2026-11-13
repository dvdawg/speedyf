//! What one printed reference entry says about the work it cites: its
//! authors, year, title, venue, volume and first page (projects.md sec. 4,
//! "From citing papers").
//!
//! Cutting a bibliography into entries is the reference provider's job and
//! stays in the extension; this reads one entry the engine already holds
//! (`StoredRef::text`) for the fields `consensus` compares across citing
//! papers. It is deliberately narrow: it knows the shapes the common styles
//! print and gives up (an empty field) on anything else, because a wrong
//! reading is a false witness and a missing one costs nothing.
//!
//! The author list is read in one of four shapes, whichever names the most
//! people:
//!
//! - **initials first**, comma separated (APS, IEEE, Elsevier):
//!   `A. Zhong and M. R. DeWeese, Title, Phys. Rev. Lett. 133, 057102 (2024).`
//! - **family first** (Springer, Nature, APA, Harvard):
//!   `Zhong, A., DeWeese, M.R.: Title. Venue 133, 057102 (2024)`,
//!   `Zhong, A. & DeWeese, M. R. Title. ...`, `Zhong, A., & DeWeese, M. R. (2024). Title. ...`
//! - **Vancouver**: `Zhong A, DeWeese MR. Title. Venue. 2024;133:057102.`
//! - **full given names** (ACM), only when the list ends in the year:
//!   `Adrianne Zhong and Michael R. DeWeese. 2024. Title. In Proceedings ...`

use super::types::Author;

/// One entry's reading. Every field is optional; `et_al` says the author
/// list was cut short, so it is only a prefix of the real one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Witness {
    pub authors: Vec<Author>,
    pub et_al: bool,
    pub year: Option<i32>,
    pub title: Option<String>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub pages: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    /// "A. B. Family", fields separated by commas.
    Initials,
    /// "Family, A. B.", fields separated by sentences.
    Inverted,
    /// "Family AB", fields separated by sentences.
    Vancouver,
    /// "Given Family. 2019.", fields separated by sentences.
    FullName,
}

const PARTICLES: &[&str] = &[
    "van", "von", "der", "den", "de", "del", "della", "di", "da", "du", "dos", "des", "la", "le",
    "ten", "ter", "zu", "bin", "al", "el", "y",
];
/// Words a venue name may hold in lower case.
const VENUE_STOPWORDS: &[&str] = &[
    "of", "the", "and", "in", "on", "for", "&", "de", "der", "und", "fur", "a", "an", "to",
];
const MAX_AUTHORS: usize = 60;
/// A name has at most this many words, particles included.
const MAX_NAME_WORDS: usize = 6;

/// Read one reference entry. `year` is the provider's reading of it, which
/// wins over the engine's own.
pub fn parse(text: &str, year: Option<i32>) -> Witness {
    let cleaned = clean(text);
    let words: Vec<&str> = cleaned.split(' ').filter(|w| !w.is_empty()).collect();
    let mut best: Option<(Style, Vec<Author>, bool, usize)> = None;
    for style in [
        Style::Inverted,
        Style::Initials,
        Style::Vancouver,
        Style::FullName,
    ] {
        let parsed = match style {
            Style::Initials => direct(&words, false),
            Style::FullName => direct(&words, true),
            Style::Inverted => inverted(&words),
            Style::Vancouver => vancouver(&words),
        };
        let Some((authors, et_al, used)) = parsed else {
            continue;
        };
        if authors.is_empty() {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(_, current, _, _)| authors.len() > current.len())
        {
            best = Some((style, authors, et_al, used));
        }
    }
    let (style, authors, et_al, used) = match best {
        Some(best) => best,
        None => (Style::Initials, Vec::new(), false, 0),
    };
    let rest = words[used..].join(" ");
    let mut witness = tail_fields(&rest, style);
    witness.authors = authors;
    witness.et_al = et_al;
    witness.year = year
        .filter(|y| (1400..=2100).contains(y))
        .or(witness.year)
        .or_else(|| any_year(&cleaned));
    witness
}

/* ---- Text ---- */

/// Quotes and dashes in their ASCII forms, whitespace collapsed, a leading
/// label ("[12]", "12.", "(12)") taken off.
fn clean(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| match c {
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{00ab}' | '\u{00bb}' => '"',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{2010}' | '\u{2011}' | '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect();
    let collapsed = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    strip_label(&collapsed).to_owned()
}

fn strip_label(text: &str) -> &str {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix('[') {
        if let Some(close) = rest.find(']').filter(|close| *close <= 12) {
            return rest[close + 1..].trim_start();
        }
    }
    if let Some(rest) = t.strip_prefix('(') {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=4).contains(&digits) && rest[digits..].starts_with(')') {
            return rest[digits + 1..].trim_start();
        }
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=4).contains(&digits) {
        let after = &t[digits..];
        if let Some(rest) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            return rest;
        }
    }
    t
}

/// A word without the separator after it.
fn body(word: &str) -> &str {
    word.trim_end_matches([',', ';', ':'])
}

/// "J.", "J.-P.", "A.B.", "Ch.", "J": a given name reduced to letters.
fn is_initials(word: &str) -> bool {
    let word = body(word);
    if word.is_empty() || word.chars().count() > 9 {
        return false;
    }
    word.split('-').all(|piece| {
        let chars: Vec<char> = piece.chars().collect();
        if chars.is_empty() {
            return false;
        }
        let (mut at, mut groups) = (0, 0);
        while at < chars.len() {
            if !chars[at].is_uppercase() {
                return false;
            }
            at += 1;
            let lower = at < chars.len() && chars[at].is_lowercase();
            if lower {
                at += 1;
            }
            if at < chars.len() && chars[at] == '.' {
                at += 1;
            } else if lower || at < chars.len() {
                return false;
            }
            groups += 1;
        }
        (1..=3).contains(&groups)
    })
}

/// A capitalized word of letters, hyphens and apostrophes: "DeWeese",
/// "O'Neil", "Jean-Pierre", "SMITH". Not an initial.
fn is_name_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let count = word.chars().count();
    first.is_uppercase()
        && (2..=30).contains(&count)
        && word
            .chars()
            .all(|c| c.is_alphabetic() || matches!(c, '-' | '\''))
        && word.chars().filter(|c| c.is_alphabetic()).count() >= 2
}

fn is_particle(word: &str) -> bool {
    PARTICLES.contains(&word)
}

fn is_separator(word: &str) -> bool {
    matches!(word, "and" | "&" | "," | "AND" | "und")
}

/// "et al." at `at`, and how many words it takes.
fn et_al_at(words: &[&str], at: usize) -> Option<usize> {
    let word = words.get(at)?.trim_end_matches([',', ';', ':', '.']);
    if word == "et" {
        let next = words.get(at + 1)?;
        if next.starts_with("al") {
            return Some(2);
        }
    }
    if word == "etal" || word == "et.al" {
        return Some(1);
    }
    None
}

/// "M.R." and "M. R." as "M. R."; Vancouver's "MR" as "M. R." too.
fn tidy_initials(words: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    for word in words {
        for piece in body(word).split_inclusive('.') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            if let (true, Some(last)) = (piece.starts_with('-'), out.last_mut()) {
                last.push_str(piece);
            } else if piece.contains('-') || piece.ends_with('.') {
                out.push(piece.to_owned());
            } else {
                out.push(format!("{piece}."));
            }
        }
    }
    out.join(" ")
}

/* ---- Authors ---- */

/// Initials (or full given names, `full`) before the family name, people
/// separated by commas, "and" and "&". The words used, and whether the
/// list ended in "et al.".
fn direct(words: &[&str], full: bool) -> Option<(Vec<Author>, bool, usize)> {
    let mut authors = Vec::new();
    let mut et_al = false;
    let mut at = 0;
    let mut sentence_end = false;
    loop {
        while at < words.len() && is_separator(words[at]) && !authors.is_empty() {
            at += 1;
        }
        if at >= words.len() {
            break;
        }
        if let Some(taken) = et_al_at(words, at).filter(|_| !authors.is_empty()) {
            et_al = true;
            at += taken;
            sentence_end = words[at - 1].ends_with('.');
            break;
        }
        let start = at;
        let mut end = at;
        let mut stop: Option<char> = None;
        while end < words.len() && end - start < MAX_NAME_WORDS {
            let word = words[end];
            if end > start
                && (is_separator(word)
                    || et_al_at(words, end).is_some()
                    || word.starts_with(['"', '(']))
            {
                break;
            }
            end += 1;
            if let Some(last) = word.chars().last().filter(|c| matches!(c, ',' | ';' | ':')) {
                stop = Some(last);
                break;
            }
            if word.ends_with('.') && !is_initials(word) {
                stop = Some('.');
                break;
            }
        }
        let Some(author) = direct_name(&words[start..end], full) else {
            at = if authors.is_empty() { 0 } else { start };
            break;
        };
        authors.push(author);
        at = end;
        if matches!(stop, Some('.') | Some(':')) {
            sentence_end = true;
            break;
        }
        if authors.len() >= MAX_AUTHORS {
            break;
        }
    }
    if full {
        // Full names are only trusted where the list visibly ends: in a
        // sentence followed by the year ("Jones. 2019."), or by "(2019)".
        let next = words.get(at).map(|w| w.trim_matches(['(', ')', '.', ',']));
        let year_next = next.is_some_and(|w| {
            w.len() >= 4 && w[..4].bytes().all(|b| b.is_ascii_digit()) && w.len() <= 5
        });
        if !(year_next && (sentence_end || words[at].starts_with('('))) {
            return None;
        }
    }
    Some((authors, et_al, at))
}

/// One "A. B. Family" (or "Given B. Family") name.
fn direct_name(chunk: &[&str], full: bool) -> Option<Author> {
    if chunk.len() < 2 || chunk.len() > MAX_NAME_WORDS {
        return None;
    }
    let mut words: Vec<&str> = chunk.iter().map(|w| body(w)).collect();
    let last = words.len() - 1;
    words[last] = words[last].trim_end_matches('.');
    let given_len = if full {
        // The family is the last word and the particles before it.
        let mut g = last;
        while g > 1 && is_particle(words[g - 1]) {
            g -= 1;
        }
        if !is_name_word(words[0]) {
            return None;
        }
        g
    } else {
        words.iter().take_while(|w| is_initials(w)).count()
    };
    if given_len == 0 || given_len > last {
        return None;
    }
    let family = &words[given_len..];
    if family.is_empty() || family.len() > 3 {
        return None;
    }
    let (family_last, particles) = family.split_last()?;
    if !is_name_word(family_last) || !particles.iter().all(|w| is_particle(w) || is_name_word(w)) {
        return None;
    }
    if full
        && words[1..given_len]
            .iter()
            .any(|w| !is_initials(w) && !is_name_word(w))
    {
        return None;
    }
    let given = if full {
        words[..given_len].join(" ")
    } else {
        tidy_initials(&words[..given_len])
    };
    Some(Author {
        family: family.join(" "),
        given,
    })
}

/// "Family, A. B." people, separated by commas, "and" and "&".
fn inverted(words: &[&str]) -> Option<(Vec<Author>, bool, usize)> {
    let mut authors = Vec::new();
    let mut et_al = false;
    let mut at = 0;
    loop {
        let resume = at;
        while at < words.len() && is_separator(words[at]) && !authors.is_empty() {
            at += 1;
        }
        if at >= words.len() {
            break;
        }
        if let Some(taken) = et_al_at(words, at).filter(|_| !authors.is_empty()) {
            et_al = true;
            at += taken;
            break;
        }
        let start = at;
        let mut end = at;
        while end < words.len() && end - start < 3 {
            end += 1;
            if words[end - 1].ends_with(',') {
                break;
            }
        }
        if !words[end - 1].ends_with(',') {
            at = resume;
            break;
        }
        let family: Vec<&str> = words[start..end].iter().map(|w| body(w)).collect();
        let (last, particles) = family.split_last()?;
        if !is_name_word(last) || !particles.iter().all(|w| is_particle(w) || is_name_word(w)) {
            at = resume;
            break;
        }
        let mut initials_end = end;
        while initials_end < words.len() && is_initials(words[initials_end]) {
            initials_end += 1;
            if words[initials_end - 1].ends_with([',', ';', ':']) {
                break;
            }
        }
        if initials_end == end {
            at = resume;
            break;
        }
        authors.push(Author {
            family: family.join(" "),
            given: tidy_initials(&words[end..initials_end]),
        });
        at = initials_end;
        let tail = words[initials_end - 1];
        if tail.ends_with(':') || authors.len() >= MAX_AUTHORS {
            break;
        }
        if tail.ends_with([',', ';']) {
            continue;
        }
        // "A. B." with nothing after it: another person only if a
        // separator or "et al." follows.
        let next = words.get(at).copied().unwrap_or_default();
        if !is_separator(next) && et_al_at(words, at).is_none() {
            break;
        }
    }
    Some((authors, et_al, at))
}

/// "Family AB" people, separated by commas, the list ended by a period.
fn vancouver(words: &[&str]) -> Option<(Vec<Author>, bool, usize)> {
    let mut authors = Vec::new();
    let mut et_al = false;
    let mut at = 0;
    loop {
        if at >= words.len() {
            break;
        }
        if let Some(taken) = et_al_at(words, at).filter(|_| !authors.is_empty()) {
            et_al = true;
            at += taken;
            break;
        }
        let start = at;
        let mut end = at;
        while end < words.len()
            && end - start < 3
            && (is_name_word(words[end]) || is_particle(words[end]))
        {
            end += 1;
        }
        let Some(initials) = words.get(end) else {
            break;
        };
        let letters = initials.trim_end_matches([',', '.']);
        let plain = (1..=3).contains(&letters.len())
            && letters.bytes().all(|b| b.is_ascii_uppercase())
            && initials.len() - letters.len() <= 1;
        if end == start || !plain || !is_name_word(words[end - 1]) {
            at = start;
            break;
        }
        let given: Vec<String> = letters.chars().map(|c| format!("{c}.")).collect();
        authors.push(Author {
            family: words[start..end].join(" "),
            given: given.join(" "),
        });
        at = end + 1;
        if initials.ends_with('.') || authors.len() >= MAX_AUTHORS {
            break;
        }
        if !initials.ends_with(',') {
            break;
        }
    }
    // A list that never ended in its period is not one.
    let ended = at > 0 && words[at - 1].ends_with('.');
    ended.then_some((authors, et_al, at))
}

/* ---- Title, venue, volume, pages ---- */

/// What follows the authors.
fn tail_fields(rest: &str, style: Style) -> Witness {
    let mut witness = Witness::default();
    let mut r = rest.trim_start_matches([',', ':', ';', '.', ' ']);
    // "(2019). " or "(2019a), " (author-year), "2019. " (ACM).
    if let Some((year, after)) = leading_year(r) {
        witness.year = Some(year);
        r = after.trim_start_matches([',', ':', ';', '.', ' ']);
    }
    let sentences = !matches!(style, Style::Initials);
    let mut after: &str = r;
    let mut title: Option<String> = None;
    if let Some(inner) = r.strip_prefix('"') {
        if let Some(close) = inner.find('"') {
            title = Some(inner[..close].trim_end_matches([',', '.', ' ']).to_owned());
            after = inner[close + 1..].trim_start_matches([',', '.', ' ']);
        }
    } else if sentences {
        if let Some((end, skip)) = sentence_end(r) {
            title = Some(r[..end].trim().to_owned());
            after = r[end + skip..].trim_start();
        }
    }
    let venue = venue_fields(after);
    if let Some(found) = &venue {
        // Comma style: the title is whatever stands before the venue.
        if title.is_none() && !sentences && found.start > 0 {
            let before = after[..found.start].trim_end_matches([',', ' ', '.']);
            if before.split(' ').count() >= 2 {
                title = Some(before.to_owned());
            }
        }
        witness.venue = Some(found.venue.clone());
        witness.volume = found.volume.clone();
        witness.pages = found.pages.clone();
        if witness.year.is_none() {
            witness.year = found.year;
        }
    } else if let Some(proceedings) = after
        .strip_prefix("In ")
        .or_else(|| after.strip_prefix("In: "))
    {
        let cut = [" (", ". ", ", "]
            .iter()
            .filter_map(|stop| proceedings.find(stop))
            .min()
            .unwrap_or(proceedings.len());
        let name = proceedings[..cut].trim_end_matches(['.', ',']).trim();
        if name.split(' ').count() >= 2 {
            witness.venue = Some(name.to_owned());
        }
    }
    witness.title = title
        .map(|t| t.trim_matches([' ', ',', '.', ':']).to_owned())
        .filter(|t| t.split(' ').count() >= 2 && t.chars().any(char::is_alphabetic));
    if witness
        .venue
        .as_deref()
        .is_some_and(|v| v.to_ascii_lowercase().contains("arxiv"))
    {
        witness.venue = None;
        witness.volume = None;
        witness.pages = None;
    }
    witness
}

/// A year that opens the text: "(2019).", "(2019a),", "2019.".
fn leading_year(text: &str) -> Option<(i32, &str)> {
    let (inner, close) = match text.strip_prefix('(') {
        Some(inner) => (inner, true),
        None => (text, false),
    };
    let digits = inner.bytes().take_while(u8::is_ascii_digit).count();
    if digits != 4 {
        return None;
    }
    let year: i32 = inner[..4].parse().ok()?;
    if !(1400..=2100).contains(&year) {
        return None;
    }
    let mut rest = &inner[4..];
    if rest.starts_with(|c: char| c.is_ascii_lowercase()) {
        rest = &rest[1..];
    }
    if close {
        rest = rest.strip_prefix(')')?;
    } else if !rest.starts_with(['.', ',']) {
        return None;
    }
    Some((year, rest))
}

/// Where the first sentence of `text` ends: ". ", "? ", "! ", and how much
/// to skip. A period after a single capital ("U. S.") ends nothing.
fn sentence_end(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    for at in 1..bytes.len().saturating_sub(1) {
        if bytes[at + 1] != b' ' {
            continue;
        }
        match bytes[at] {
            b'.' => {
                let word_start = text[..at].rfind(' ').map_or(0, |s| s + 1);
                let word = &text[word_start..at];
                if word.chars().count() <= 1 {
                    continue;
                }
                return Some((at, 1));
            }
            b'?' | b'!' => return Some((at + 1, 1)),
            _ => {}
        }
    }
    None
}

struct VenueFields {
    /// Byte offset of the venue in the text it was read from.
    start: usize,
    venue: String,
    volume: Option<String>,
    pages: Option<String>,
    year: Option<i32>,
}

/// "Phys. Rev. Lett. 133, 057102 (2024)", "Journal of Things, 12(3),
/// 45-67", "J. Comput. Phys. 227 (2008) 1-20", "IEEE Trans. Inf. Theory,
/// vol. 52, no. 4, pp. 1289-1306", "J Things. 2019;12(3):45-67".
fn venue_fields(text: &str) -> Option<VenueFields> {
    let words: Vec<(usize, &str)> = word_offsets(text);
    // IEEE: "Venue, vol. 52, ... pp. 1-10".
    if let Some(k) = words
        .iter()
        .position(|(_, w)| w.eq_ignore_ascii_case("vol.") || w.eq_ignore_ascii_case("vol"))
    {
        let volume = words
            .get(k + 1)
            .map(|(_, w)| w.trim_end_matches([',', '.']))
            .filter(|w| is_volume(w))?;
        let (start, venue) = venue_before(text, &words, k, true)?;
        let pages = words
            .iter()
            .skip(k + 2)
            .position(|(_, w)| matches!(*w, "pp." | "p." | "pp" | "Art."))
            .and_then(|p| words.get(k + 3 + p))
            .and_then(|(_, w)| page_token(w));
        return Some(VenueFields {
            start,
            venue,
            volume: Some(volume.to_owned()),
            pages,
            year: None,
        });
    }
    for k in 1..words.len() {
        let word = words[k].1;
        // Vancouver: "2019;12(3):45-67."
        if let Some((year, volume, pages)) = vancouver_numbers(word) {
            let (start, venue) = venue_before(text, &words, k, false)?;
            // Vancouver ends the venue with a sentence: "J Things. 2019;...".
            let venue = venue.trim_end_matches('.').to_owned();
            return Some(VenueFields {
                start,
                venue,
                volume: Some(volume),
                pages,
                year: Some(year),
            });
        }
        let Some((volume, issue)) = volume_token(word) else {
            continue;
        };
        // A number is a volume when what follows reads like one: a
        // separator or an issue on it, or a year or page right after.
        let next = words.get(k + 1).map(|(_, w)| *w);
        let anchored = issue
            || word.ends_with([',', ':', ';'])
            || next.is_some_and(|n| paren_year(n).is_some() || page_token(n).is_some());
        if !anchored {
            continue;
        }
        let Some((start, venue)) = venue_before(text, &words, k, false) else {
            continue;
        };
        let (pages, year) = pages_after(&words[k + 1..]);
        return Some(VenueFields {
            start,
            venue,
            volume: Some(volume),
            pages,
            year,
        });
    }
    None
}

fn word_offsets(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c == ' ' {
            if let Some(s) = start.take() {
                out.push((s, &text[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out
}

fn is_volume(word: &str) -> bool {
    (1..=4).contains(&word.len()) && word.bytes().all(|b| b.is_ascii_digit())
}

/// "133,", "12(3),", "227": a volume, and whether an issue was on it.
fn volume_token(word: &str) -> Option<(String, bool)> {
    let word = word.trim_end_matches([',', ':', ';', '.']);
    let digits = word.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=4).contains(&digits) {
        return None;
    }
    let rest = &word[digits..];
    if rest.is_empty() {
        return Some((word.to_owned(), false));
    }
    let issue = rest.strip_prefix('(')?.strip_suffix(')')?;
    (!issue.is_empty() && issue.len() <= 9).then(|| (word[..digits].to_owned(), true))
}

/// "2019;12(3):45-67." as (year, volume, pages).
fn vancouver_numbers(word: &str) -> Option<(i32, String, Option<String>)> {
    let word = word.trim_end_matches('.');
    let (year, rest) = word.split_once(';')?;
    let year: i32 = year.parse().ok().filter(|y| (1400..=2100).contains(y))?;
    let (volume_part, pages) = match rest.split_once(':') {
        Some((volume, pages)) => (volume, page_token(pages)),
        None => (rest, None),
    };
    let digits = volume_part.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=4).contains(&digits) {
        return None;
    }
    Some((year, volume_part[..digits].to_owned(), pages))
}

/// "(2024)", "(2024).", "(2024),": the year.
fn paren_year(word: &str) -> Option<i32> {
    let inner = word
        .trim_end_matches([',', '.', ';', ':'])
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let inner = inner.trim_end_matches(|c: char| c.is_ascii_lowercase());
    (inner.len() == 4)
        .then(|| inner.parse().ok())
        .flatten()
        .filter(|y| (1400..=2100).contains(y))
}

/// "057102", "L052103", "436-444", "e1001": a page or article number.
fn page_token(word: &str) -> Option<String> {
    let word = word.trim_end_matches([',', '.', ';', ')']);
    let word = word
        .strip_prefix("pp.")
        .or_else(|| word.strip_prefix("p."))
        .unwrap_or(word);
    let ok = |piece: &str| {
        let digits = piece.trim_start_matches(|c: char| c.is_ascii_alphabetic());
        piece.len() - digits.len() <= 1
            && (1..=7).contains(&digits.len())
            && digits.bytes().all(|b| b.is_ascii_digit())
    };
    let mut pieces = word.split('-').filter(|p| !p.is_empty());
    let first = pieces.next()?;
    let second = pieces.next();
    if pieces.next().is_some() || !ok(first) || second.is_some_and(|s| !ok(s)) {
        return None;
    }
    Some(match second {
        Some(second) => format!("{first}-{second}"),
        None => first.to_owned(),
    })
}

/// The pages after a volume: issues "(3)" and years "(2008)" skipped, an
/// ACM issue ("6 (2017), 84-90") too. The year if one was passed.
fn pages_after(words: &[(usize, &str)]) -> (Option<String>, Option<i32>) {
    let mut year = None;
    let mut at = 0;
    while at < words.len() && at < 6 {
        let word = words[at].1;
        if let Some(found) = paren_year(word) {
            year = Some(found);
            at += 1;
            continue;
        }
        if word.starts_with('(') || matches!(word, "no." | "No." | "pp." | "p.") {
            at += 1;
            continue;
        }
        if let Some(page) = page_token(word) {
            // A number followed by the year is an issue: "6 (2017), 84-90".
            if words
                .get(at + 1)
                .is_some_and(|(_, n)| paren_year(n).is_some())
                && words
                    .get(at + 2)
                    .is_some_and(|(_, n)| page_token(n).is_some())
            {
                at += 1;
                continue;
            }
            return (Some(page), year);
        }
        break;
    }
    (None, year)
}

/// The venue name ending just before word `k`: capitalized words and
/// lower-case joining words, back to a comma, a colon, a quote or a
/// lower-case word. `comma_ok` lets the word just before `k` end in a comma
/// ("Journal of Things, 12(3)" and IEEE's "Venue, vol.").
fn venue_before(
    text: &str,
    words: &[(usize, &str)],
    k: usize,
    comma_ok: bool,
) -> Option<(usize, String)> {
    let mut first = k;
    while first > 0 {
        let word = words[first - 1].1;
        let immediate = first == k;
        if word.ends_with(['"']) || word.starts_with('"') {
            break;
        }
        if word.ends_with([',', ';']) && !(immediate && (comma_ok || word.ends_with(','))) {
            break;
        }
        let core = word.trim_end_matches([',', ';', '.', ':']);
        if word.ends_with(':') && !(core.len() == 1 && core.chars().all(|c| c.is_uppercase())) {
            break;
        }
        let capitalized = core.chars().next().is_some_and(|c| c.is_uppercase());
        if !capitalized && !VENUE_STOPWORDS.contains(&core.to_lowercase().as_str()) {
            break;
        }
        first -= 1;
        // A full sentence's last word ("stuff.") is not a venue's. An
        // abbreviation is short; a long word with a period ends a title.
        if first > 0 {
            let previous = words[first - 1].1;
            let previous_core = previous.trim_end_matches('.');
            if previous.ends_with('.')
                && previous_core.chars().count() >= 8
                && previous_core.chars().all(char::is_alphabetic)
            {
                break;
            }
        }
        if k - first > 12 {
            return None;
        }
    }
    // Lower-case joining words do not open a venue.
    while first < k
        && !words[first]
            .1
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase())
    {
        first += 1;
    }
    if first == k {
        return None;
    }
    let start = words[first].0;
    let end = words[k - 1].0 + words[k - 1].1.len();
    let venue = text[start..end]
        .trim_end_matches([',', ';', ' '])
        .to_owned();
    let venue = venue.strip_prefix("In ").unwrap_or(&venue).to_owned();
    Some((start, venue)).filter(|(_, v)| v.chars().any(char::is_alphabetic))
}

/// The year in a text with no better reading: the last four-digit number
/// in parentheses, else the last standalone one.
fn any_year(text: &str) -> Option<i32> {
    let words = word_offsets(text);
    if let Some(year) = words.iter().rev().find_map(|(_, w)| paren_year(w)) {
        return Some(year);
    }
    words.iter().rev().find_map(|(_, w)| {
        let core = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        (core.len() == 4)
            .then(|| core.parse::<i32>().ok())
            .flatten()
            .filter(|y| (1900..=2100).contains(y))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(witness: &Witness) -> Vec<String> {
        witness
            .authors
            .iter()
            .map(|a| format!("{}|{}", a.family, a.given))
            .collect()
    }

    #[test]
    fn aps_style() {
        let w = parse("A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).", Some(2024));
        assert_eq!(names(&w), ["Zhong|A.", "DeWeese|M. R."]);
        assert_eq!(w.year, Some(2024));
        assert_eq!(
            w.title.as_deref(),
            Some("Beyond linear response: Equivalence between thermodynamic geometry and optimal transport")
        );
        assert_eq!(w.venue.as_deref(), Some("Phys. Rev. Lett."));
        assert_eq!(w.volume.as_deref(), Some("133"));
        assert_eq!(w.pages.as_deref(), Some("057102"));
        assert!(!w.et_al);

        // PRL's own style prints no title.
        let w = parse(
            "[7] T. Schmiedl and U. Seifert, Phys. Rev. Lett. 98, 108301 (2007).",
            None,
        );
        assert_eq!(names(&w), ["Schmiedl|T.", "Seifert|U."]);
        assert_eq!(w.title, None);
        assert_eq!(w.venue.as_deref(), Some("Phys. Rev. Lett."));
        assert_eq!(w.year, Some(2007));

        let w = parse(
            "J.-P. Eckmann, C. Pillet, and L. Rey-Bellet, Entropy production in nonlinear, thermally driven Hamiltonian systems, J. Stat. Phys. 95, 305 (1999).",
            Some(1999),
        );
        assert_eq!(names(&w), ["Eckmann|J.-P.", "Pillet|C.", "Rey-Bellet|L."]);
        assert_eq!(
            w.title.as_deref(),
            Some("Entropy production in nonlinear, thermally driven Hamiltonian systems")
        );
        assert_eq!(w.venue.as_deref(), Some("J. Stat. Phys."));
    }

    #[test]
    fn ieee_style() {
        let w = parse(
            "J. Smith, A. B. Jones, and C. van der Berg, \u{201c}Deep things for stuff,\u{201d} IEEE Trans. Inf. Theory, vol. 52, no. 4, pp. 1289\u{2013}1306, Apr. 2006.",
            Some(2006),
        );
        assert_eq!(names(&w), ["Smith|J.", "Jones|A. B.", "van der Berg|C."]);
        assert_eq!(w.title.as_deref(), Some("Deep things for stuff"));
        assert_eq!(w.venue.as_deref(), Some("IEEE Trans. Inf. Theory"));
        assert_eq!(w.volume.as_deref(), Some("52"));
        assert_eq!(w.pages.as_deref(), Some("1289-1306"));
    }

    #[test]
    fn acm_style() {
        let w = parse(
            "John Smith and Alice B. Jones. 2019. Deep things for stuff. In Proceedings of the 2019 CHI Conference on Human Factors in Computing Systems (CHI '19). ACM, New York, NY, USA, 1-12.",
            None,
        );
        assert_eq!(names(&w), ["Smith|John", "Jones|Alice B."]);
        assert_eq!(w.year, Some(2019));
        assert_eq!(w.title.as_deref(), Some("Deep things for stuff"));
        assert_eq!(
            w.venue.as_deref(),
            Some("Proceedings of the 2019 CHI Conference on Human Factors in Computing Systems")
        );
        let w = parse(
            "Jeffrey Dean and Sanjay Ghemawat. 2008. MapReduce: simplified data processing on large clusters. Commun. ACM 51, 1 (2008), 107-113.",
            None,
        );
        assert_eq!(names(&w), ["Dean|Jeffrey", "Ghemawat|Sanjay"]);
        assert_eq!(w.venue.as_deref(), Some("Commun. ACM"));
        assert_eq!(w.volume.as_deref(), Some("51"));
        assert_eq!(w.pages.as_deref(), Some("107-113"));
    }

    #[test]
    fn springer_and_nature_styles() {
        let w = parse(
            "Zhong, A., DeWeese, M.R.: Beyond linear response: equivalence between thermodynamic geometry and optimal transport. Phys. Rev. Lett. 133, 057102 (2024)",
            None,
        );
        assert_eq!(names(&w), ["Zhong|A.", "DeWeese|M. R."]);
        assert_eq!(w.year, Some(2024));
        assert_eq!(
            w.title.as_deref(),
            Some("Beyond linear response: equivalence between thermodynamic geometry and optimal transport")
        );
        assert_eq!(w.venue.as_deref(), Some("Phys. Rev. Lett."));
        assert_eq!(w.pages.as_deref(), Some("057102"));

        let w = parse(
            "LeCun, Y., Bengio, Y. & Hinton, G. Deep learning. Nature 521, 436\u{2013}444 (2015).",
            None,
        );
        assert_eq!(names(&w), ["LeCun|Y.", "Bengio|Y.", "Hinton|G."]);
        assert_eq!(w.title.as_deref(), Some("Deep learning"));
        assert_eq!(w.venue.as_deref(), Some("Nature"));
        assert_eq!(w.volume.as_deref(), Some("521"));
        assert_eq!(w.pages.as_deref(), Some("436-444"));
        assert_eq!(w.year, Some(2015));
    }

    #[test]
    fn author_year_and_vancouver_styles() {
        let w = parse(
            "Smith, J., & Jones, A. B. (2019a). Title of the paper. Journal of Things, 12(3), 45-67.",
            None,
        );
        assert_eq!(names(&w), ["Smith|J.", "Jones|A. B."]);
        assert_eq!(w.year, Some(2019));
        assert_eq!(w.title.as_deref(), Some("Title of the paper"));
        assert_eq!(w.venue.as_deref(), Some("Journal of Things"));
        assert_eq!(w.volume.as_deref(), Some("12"));
        assert_eq!(w.pages.as_deref(), Some("45-67"));

        let w = parse(
            "1. Smith JA, Jones B, Doe C. Title of the paper. J Things. 2019;12(3):45-67.",
            None,
        );
        assert_eq!(names(&w), ["Smith|J. A.", "Jones|B.", "Doe|C."]);
        assert_eq!(w.title.as_deref(), Some("Title of the paper"));
        assert_eq!(w.venue.as_deref(), Some("J Things"));
        assert_eq!(w.volume.as_deref(), Some("12"));
        assert_eq!(w.pages.as_deref(), Some("45-67"));
        assert_eq!(w.year, Some(2019));
    }

    #[test]
    fn elsevier_style_and_et_al() {
        let w = parse(
            "J. Smith, A.B. Jones, Deep things for stuff, J. Comput. Phys. 227 (2008) 1-20.",
            None,
        );
        assert_eq!(names(&w), ["Smith|J.", "Jones|A. B."]);
        assert_eq!(w.title.as_deref(), Some("Deep things for stuff"));
        assert_eq!(w.venue.as_deref(), Some("J. Comput. Phys."));
        assert_eq!(w.volume.as_deref(), Some("227"));
        assert_eq!(w.pages.as_deref(), Some("1-20"));
        assert_eq!(w.year, Some(2008));

        let w = parse("A. Zhong et al., Phys. Rev. E 105, L052103 (2022).", None);
        assert_eq!(names(&w), ["Zhong|A."]);
        assert!(w.et_al);
        assert_eq!(w.venue.as_deref(), Some("Phys. Rev. E"));
        assert_eq!(w.pages.as_deref(), Some("L052103"));
        let w = parse("Smith, J., et al. (2020). A title here. Nature 1, 2.", None);
        assert_eq!(names(&w), ["Smith|J."]);
        assert!(w.et_al);
    }

    #[test]
    fn what_is_not_a_name_is_not_read_as_one() {
        // A title-cased title is not a list of full names unless the year
        // closes it.
        let w = parse("Deep Learning, Nature Reviews 12, 1 (2015).", None);
        assert!(w.authors.is_empty(), "{:?}", w.authors);
        let w = parse("ATLAS Collaboration, Phys. Lett. B 716, 1 (2012).", None);
        assert!(w.authors.is_empty(), "{:?}", w.authors);
        let w = parse("arXiv:2404.01286", None);
        assert!(w.authors.is_empty());
        assert_eq!(w.venue, None);
        let w = parse(
            "A. Zhong, Some title, arXiv preprint arXiv:2404.01286 (2024).",
            None,
        );
        assert_eq!(names(&w), ["Zhong|A."]);
        assert_eq!(w.venue, None);
    }
}

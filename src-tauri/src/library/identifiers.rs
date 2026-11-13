//! What an identifier says on its own, with no network (projects.md §4).
//!
//! An arXiv id encodes the month it was submitted; the stamp arXiv prints
//! down the first page's margin adds the primary category and the date of
//! the version. A DOI from a few publisher families carries its year in a
//! fixed place. An Elsevier PII — what an old ScienceDirect scan prints in
//! place of a DOI, often as the file's only identifier — carries its year
//! in the parentheses and converts to the article's DOI, which is what
//! unlocks a Crossref lookup for a 1988 paper.
//!
//! Everything here is a pure function over a string. Nothing guesses: a
//! bare four-digit run in a DOI is never taken for a year, only the places
//! where a family's convention puts one.

use crate::engine::citation::find_explicit_citations;
use crate::engine::types::CitationIdDto;

/// The first new-style arXiv id was 0704.0001; a `YYMM` below that is not
/// an id of that style.
const FIRST_NEW_STYLE_YYMM: u32 = 704;

/// A `vN` suffix off an id, when it has one.
fn strip_version(id: &str) -> &str {
    match id.rfind(['v', 'V']) {
        Some(at)
            if at > 0
                && !id[at + 1..].is_empty()
                && id[at + 1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            &id[..at]
        }
        _ => id,
    }
}

fn strip_arxiv_label(id: &str) -> &str {
    let id = id.trim();
    id.get(..6)
        .filter(|head| head.eq_ignore_ascii_case("arxiv:"))
        .map_or(id, |_| id[6..].trim())
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `(YY, MM, legacy)` of an arXiv id: new style `YYMM.NNNNN`, or the old
/// `archive/YYMMNNN`. Tolerant of an `arXiv:` label and a version suffix,
/// which the normalizer strips but a caller might not have.
fn arxiv_yymm(id: &str) -> Option<(u32, u32, bool)> {
    let core = strip_version(strip_arxiv_label(id));
    if let Some((archive, number)) = core.rsplit_once('/') {
        if archive.is_empty() || number.len() != 7 || !all_digits(number) {
            return None;
        }
        let yy: u32 = number[..2].parse().ok()?;
        let mm: u32 = number[2..4].parse().ok()?;
        return (1..=12).contains(&mm).then_some((yy, mm, true));
    }
    let (yymm, number) = core.split_once('.')?;
    if yymm.len() != 4 || !all_digits(yymm) || !(4..=5).contains(&number.len()) {
        return None;
    }
    if !all_digits(number) {
        return None;
    }
    let yy: u32 = yymm[..2].parse().ok()?;
    let mm: u32 = yymm[2..].parse().ok()?;
    if !(1..=12).contains(&mm) || yy * 100 + mm < FIRST_NEW_STYLE_YYMM {
        return None;
    }
    Some((yy, mm, false))
}

/// The year an arXiv id was submitted: `2401.12345` → 2024,
/// `hep-th/9901001` → 1999, `cond-mat/0601234` → 2006. The old scheme ran
/// from 1991 to March 2007, so a two-digit year of 91 or more is in the
/// nineteen-hundreds.
pub fn year_from_arxiv(id: &str) -> Option<i32> {
    let (yy, _, legacy) = arxiv_yymm(id)?;
    let century = if legacy && yy >= 91 { 1900 } else { 2000 };
    Some(century + yy as i32)
}

/// The month (1–12) an arXiv id was submitted. Nothing stores it yet; it
/// sits beside the year for when a record wants it.
#[cfg_attr(not(test), allow(dead_code))] // read by the tests
pub fn month_from_arxiv(id: &str) -> Option<u8> {
    arxiv_yymm(id).map(|(_, mm, _)| mm as u8)
}

/// The stamp arXiv prints down the margin of a first page:
/// `arXiv:1706.03762v5 [cs.CL] 6 Dec 2017`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArxivStamp {
    /// The id, without its version.
    pub id: String,
    /// The bracketed primary category, as printed (`cs.CL`, `hep-th`).
    pub category: Option<String>,
    /// The version's date as (year, month, day).
    pub date: Option<(i32, u8, u8)>,
}

/// The most whitespace the stamp's parts may be apart. The parts sit on
/// one line with a space or two between them; more is another line.
const MAX_STAMP_GAP: usize = 4;

fn skip_gap(chars: &[char], mut at: usize) -> usize {
    let limit = at + MAX_STAMP_GAP;
    while at < limit && chars.get(at).is_some_and(|c| c.is_whitespace()) {
        at += 1;
    }
    at
}

fn category_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-')
}

/// `[category]` at `at`: the category and where it ends.
fn bracketed_category(chars: &[char], at: usize) -> Option<(String, usize)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let start = at + 1;
    let mut end = start;
    while end - start < 24 && chars.get(end).is_some_and(|c| category_char(*c)) {
        end += 1;
    }
    if end == start || chars.get(end) != Some(&']') || !chars[start].is_ascii_alphabetic() {
        return None;
    }
    Some((chars[start..end].iter().collect(), end + 1))
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn number_at(chars: &[char], at: usize, max_digits: usize) -> Option<(u32, usize)> {
    let mut end = at;
    while end - at < max_digits && chars.get(end).is_some_and(char::is_ascii_digit) {
        end += 1;
    }
    if end == at || chars.get(end).is_some_and(char::is_ascii_digit) {
        return None;
    }
    let value: u32 = chars[at..end].iter().collect::<String>().parse().ok()?;
    Some((value, end))
}

/// `D Mon YYYY` at `at`, the month by its first three letters.
fn stamp_date(chars: &[char], at: usize) -> Option<(i32, u8, u8)> {
    let (day, at) = number_at(chars, at, 2)?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let at = skip_gap(chars, at);
    let mut end = at;
    while chars.get(end).is_some_and(|c| c.is_ascii_alphabetic()) {
        end += 1;
    }
    if end - at < 3 {
        return None;
    }
    let name: String = chars[at..at + 3]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    let month = MONTHS.iter().position(|m| *m == name)? as u8 + 1;
    let at = skip_gap(chars, end);
    let (year, _) = number_at(chars, at, 4)?;
    if !(1991..=2100).contains(&year) {
        return None;
    }
    Some((year as i32, month, day as u8))
}

/// The first arXiv stamp in page text: an explicitly labelled arXiv id
/// followed by its bracketed category, and the version date when it is
/// there. An id without the brackets is a citation, not a stamp.
pub fn arxiv_stamp(text: &str) -> Option<ArxivStamp> {
    let chars: Vec<char> = text.chars().collect();
    for found in find_explicit_citations(text) {
        let CitationIdDto::ArXiv(id) = found.id else {
            continue;
        };
        let after = skip_gap(&chars, (found.start + found.len) as usize);
        let Some((category, end)) = bracketed_category(&chars, after) else {
            continue;
        };
        let date = stamp_date(&chars, skip_gap(&chars, end));
        return Some(ArxivStamp {
            id,
            category: Some(category),
            date,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Elsevier PIIs
// ---------------------------------------------------------------------------

fn is_check_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, 'x' | 'X')
}

/// A PII in its printed form at `at`: `S0022-0000(88)90028-1`, the S
/// optional. Returns where it ends.
fn printed_pii_at(chars: &[char], at: usize) -> Option<usize> {
    let mut index = at;
    if chars.get(index).is_some_and(|c| matches!(c, 's' | 'S')) {
        index += 1;
    }
    let mut expect = |want: &dyn Fn(char) -> bool, count: usize| -> Option<()> {
        for _ in 0..count {
            if !chars.get(index).is_some_and(|c| want(*c)) {
                return None;
            }
            index += 1;
        }
        Some(())
    };
    let digit = |c: char| c.is_ascii_digit();
    expect(&digit, 4)?;
    expect(&|c| c == '-', 1)?;
    expect(&digit, 3)?;
    expect(&is_check_char, 1)?;
    expect(&|c| c == '(', 1)?;
    expect(&digit, 2)?;
    expect(&|c| c == ')', 1)?;
    expect(&digit, 5)?;
    expect(&|c| c == '-', 1)?;
    expect(&is_check_char, 1)?;
    if chars.get(index).is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(index)
}

/// A PII written without its punctuation, the way a packet or a URL
/// carries it (`S0022000088900281`), expanded to the printed form. Only
/// taken when it is the whole string: sixteen digits in running text are
/// something else.
fn compact_pii(text: &str) -> Option<String> {
    let text = text.trim();
    let (s, body) = match text.strip_prefix(['s', 'S']) {
        Some(body) => (&text[..1], body),
        None => ("", text),
    };
    let chars: Vec<char> = body.chars().collect();
    if chars.len() != 16
        || !chars[..7].iter().all(char::is_ascii_digit)
        || !is_check_char(chars[7])
        || !chars[8..15].iter().all(char::is_ascii_digit)
        || !is_check_char(chars[15])
    {
        return None;
    }
    let part = |range: std::ops::Range<usize>| chars[range].iter().collect::<String>();
    Some(format!(
        "{s}{}-{}({}){}-{}",
        part(0..4),
        part(4..8),
        part(8..10),
        part(10..15),
        part(15..16)
    ))
}

/// The first Elsevier PII in `text`, as printed: labelled ("PII:
/// S0022-0000(88)90028-1", "PII 0022-0000(88)90028-1") or bare, the
/// pattern being distinctive enough on its own. A compact one that is the
/// whole string is expanded.
pub fn find_pii(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for at in 0..chars.len() {
        if at > 0 && chars[at - 1].is_ascii_alphanumeric() {
            // Inside a longer token; the optional S is the one exception
            // and is handled by starting the match there.
            continue;
        }
        if let Some(end) = printed_pii_at(&chars, at) {
            return Some(chars[at..end].iter().collect());
        }
    }
    compact_pii(text)
}

/// The ISSN a PII starts with, as printed (`0022-0000`).
pub fn issn_of_pii(pii: &str) -> Option<&str> {
    let body = pii.strip_prefix(['s', 'S']).unwrap_or(pii);
    let issn = body.get(..9)?;
    let chars: Vec<char> = issn.chars().collect();
    (chars.len() == 9
        && chars[..4].iter().all(char::is_ascii_digit)
        && chars[4] == '-'
        && chars[5..8].iter().all(char::is_ascii_digit)
        && is_check_char(chars[8]))
    .then_some(issn)
}

/// Whether an ISSN's check digit holds: the digits weighted 8 down to 2
/// plus the check (X for ten) sum to a multiple of eleven. The hyphen is
/// optional.
pub fn valid_issn(issn: &str) -> bool {
    let chars: Vec<char> = issn
        .trim()
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    if chars.len() != 8 || !chars[..7].iter().all(char::is_ascii_digit) || !is_check_char(chars[7])
    {
        return false;
    }
    let sum: u32 = chars[..7]
        .iter()
        .enumerate()
        .map(|(index, c)| c.to_digit(10).unwrap_or(0) * (8 - index as u32))
        .sum();
    let check = match chars[7] {
        'x' | 'X' => 10,
        c => c.to_digit(10).unwrap_or(0),
    };
    (sum + check) % 11 == 0
}

/// Whether the PII's ISSN checks out — the bar a PII must clear before it
/// may stand in for a DOI and merge files. A mis-OCR'd digit would
/// otherwise merge two articles.
pub fn pii_has_valid_issn(pii: &str) -> bool {
    issn_of_pii(pii).is_some_and(valid_issn)
}

/// Elsevier's DOI for a PII: `10.1016/` and the PII as printed. Articles
/// from before the scheme had their DOIs made from the bare PII, later
/// ones keep the S, and the file prints whichever it has.
pub fn doi_from_pii(pii: &str) -> String {
    format!("10.1016/{}", pii.trim())
}

/// The year in a PII's parentheses: `(88)` → 1988, `(05)` → 2005.
pub fn year_from_pii(pii: &str) -> Option<i32> {
    let open = pii.find('(')?;
    let yy = pii.get(open + 1..open + 3)?;
    if !all_digits(yy) || pii.get(open + 3..open + 4) != Some(")") {
        return None;
    }
    let yy: i32 = yy.parse().ok()?;
    Some(if yy >= 50 { 1900 + yy } else { 2000 + yy })
}

// ---------------------------------------------------------------------------
// DOIs
// ---------------------------------------------------------------------------

/// A four-digit year at the start of `s`, when it is one a paper could
/// carry.
fn year_at_start(s: &str) -> Option<i32> {
    let digits = s.get(..4)?;
    if !all_digits(digits) {
        return None;
    }
    digits
        .parse::<i32>()
        .ok()
        .filter(|year| (1900..=2100).contains(year))
}

/// `<segment>.YYYY.` — the year a journal's DOIs carry after the journal
/// code, as Elsevier (`j.physa.2020.124578`) and IEEE (`tpami.2019.291`)
/// write them.
fn year_after_first_segment(rest: &str) -> Option<i32> {
    let (_, after) = rest.split_once('.')?;
    let year = year_at_start(after)?;
    (after.as_bytes().get(4) == Some(&b'.')).then_some(year)
}

/// The year a DOI carries, for the families whose convention puts one in
/// a fixed place: arXiv's own DOIs (the id), Elsevier (`j.<journal>.YYYY.`
/// and the PII form), Springer Nature (`s<journal>-0YY-` or `-9YY-`),
/// IEEE (`<journal>.YYYY.`) and Wiley (`<journal>.YYYYNNNNN`). Any other
/// DOI says nothing: `10.1103/PhysRevLett.78.2690` is volume and page.
pub fn year_from_doi(doi: &str) -> Option<i32> {
    let lower = doi.trim().to_ascii_lowercase();
    let mut doi = lower.as_str();
    for prefix in ["https://doi.org/", "http://doi.org/", "doi:"] {
        if let Some(rest) = doi.strip_prefix(prefix) {
            doi = rest.trim();
        }
    }
    if let Some(id) = doi.strip_prefix("10.48550/arxiv.") {
        return year_from_arxiv(id);
    }
    if let Some(rest) = doi.strip_prefix("10.1016/") {
        if let Some(journal) = rest.strip_prefix("j.") {
            return year_after_first_segment(journal);
        }
        return find_pii(rest).as_deref().and_then(year_from_pii);
    }
    for prefix in ["10.1007/s", "10.1038/s"] {
        if let Some(rest) = doi.strip_prefix(prefix) {
            let (journal, after) = rest.split_once('-')?;
            let yyy = after.get(..3)?;
            if !all_digits(journal) || !all_digits(yyy) || after.as_bytes().get(3) != Some(&b'-') {
                return None;
            }
            let yy: i32 = yyy[1..].parse().ok()?;
            return match yyy.as_bytes()[0] {
                b'0' => Some(2000 + yy),
                b'9' => Some(1900 + yy),
                _ => None,
            };
        }
    }
    if let Some(rest) = doi.strip_prefix("10.1109/") {
        return year_after_first_segment(rest);
    }
    if let Some(rest) = doi.strip_prefix("10.1002/") {
        let (_, after) = rest.split_once('.')?;
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits < 9 || !(after.starts_with("19") || after.starts_with("20")) {
            return None;
        }
        return year_at_start(after);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_style_arxiv_ids_carry_their_year_and_month() {
        assert_eq!(year_from_arxiv("2401.12345"), Some(2024));
        assert_eq!(year_from_arxiv("1706.03762"), Some(2017));
        assert_eq!(
            year_from_arxiv("1706.03762v5"),
            Some(2017),
            "version tolerated"
        );
        assert_eq!(
            year_from_arxiv("arXiv:1706.03762"),
            Some(2017),
            "label tolerated"
        );
        assert_eq!(year_from_arxiv("0704.0001"), Some(2007), "the first id");
        assert_eq!(year_from_arxiv("0712.9999"), Some(2007));
        assert_eq!(year_from_arxiv("0703.0001"), None, "before the scheme");
        assert_eq!(year_from_arxiv("2400.12345"), None, "month zero");
        assert_eq!(year_from_arxiv("2413.12345"), None, "month thirteen");
        assert_eq!(year_from_arxiv("2401.123"), None, "too few digits");
        assert_eq!(year_from_arxiv("10.1000/x"), None);
        assert_eq!(year_from_arxiv(""), None);
        assert_eq!(month_from_arxiv("2401.12345"), Some(1));
        assert_eq!(month_from_arxiv("1712.00001"), Some(12));
        assert_eq!(month_from_arxiv("2413.12345"), None);
    }

    #[test]
    fn legacy_arxiv_ids_carry_their_year_across_the_century() {
        assert_eq!(year_from_arxiv("hep-th/9901001"), Some(1999));
        assert_eq!(
            year_from_arxiv("hep-th/9108001"),
            Some(1991),
            "the first year"
        );
        assert_eq!(year_from_arxiv("cond-mat/0601234"), Some(2006));
        assert_eq!(year_from_arxiv("math.GT/0309136v1"), Some(2003));
        assert_eq!(
            year_from_arxiv("solv-int/9901001"),
            Some(1999),
            "a v in the archive"
        );
        assert_eq!(year_from_arxiv("hep-th/9913001"), None, "month thirteen");
        assert_eq!(year_from_arxiv("hep-th/990100"), None, "six digits");
        assert_eq!(month_from_arxiv("cond-mat/0601234"), Some(1), "YYMMNNN");
        assert_eq!(month_from_arxiv("cond-mat/0606001"), Some(6));
    }

    #[test]
    fn the_stamp_gives_the_category_and_the_version_date() {
        let stamp = arxiv_stamp("arXiv:1706.03762v5 [cs.CL] 6 Dec 2017\nAttention Is All You Need")
            .expect("stamp");
        assert_eq!(stamp.id, "1706.03762");
        assert_eq!(stamp.category.as_deref(), Some("cs.CL"));
        assert_eq!(stamp.date, Some((2017, 12, 6)));

        let legacy = arxiv_stamp("arXiv:hep-th/9901001v2  [hep-th]  5 Jan 1999").expect("stamp");
        assert_eq!(legacy.id, "hep-th/9901001");
        assert_eq!(legacy.category.as_deref(), Some("hep-th"));
        assert_eq!(legacy.date, Some((1999, 1, 5)));

        let dotted = arxiv_stamp("arXiv:2401.00001v2 [cond-mat.stat-mech] 31 Jan 2024").unwrap();
        assert_eq!(dotted.category.as_deref(), Some("cond-mat.stat-mech"));
        assert_eq!(dotted.date, Some((2024, 1, 31)));

        let undated = arxiv_stamp("arXiv:1706.03762v5 [cs.CL]").expect("stamp");
        assert_eq!(undated.category.as_deref(), Some("cs.CL"));
        assert_eq!(undated.date, None);
        let bad_date = arxiv_stamp("arXiv:1706.03762 [cs.CL] 6 Foo 2017").expect("stamp");
        assert_eq!(bad_date.date, None);

        // A citation is not a stamp; the stamp is the first bracketed id.
        assert_eq!(arxiv_stamp("see arXiv:1706.03762 and doi:10.1/x"), None);
        let second = arxiv_stamp("cf. arXiv:1111.11111. arXiv:1706.03762 [cs.CL] 6 Dec 2017")
            .expect("stamp");
        assert_eq!(second.id, "1706.03762");
        assert_eq!(arxiv_stamp("arXiv:1706.03762 [] 6 Dec 2017"), None);
        assert_eq!(arxiv_stamp(""), None);
    }

    #[test]
    fn piis_are_found_labelled_bare_in_an_info_title_or_compact() {
        assert_eq!(
            find_pii("PII: 0022-0000(88)90028-1").as_deref(),
            Some("0022-0000(88)90028-1")
        );
        assert_eq!(
            find_pii("PII S0022-0000(96)00001-X").as_deref(),
            Some("S0022-0000(96)00001-X")
        );
        assert_eq!(
            find_pii("PII:S0370-2693(97)00412-1").as_deref(),
            Some("S0370-2693(97)00412-1")
        );
        assert_eq!(
            find_pii(
                "Journal of Computer and System Sciences 37, 1988\n0022-0000(88)90028-1 Elsevier"
            )
            .as_deref(),
            Some("0022-0000(88)90028-1"),
            "bare"
        );
        assert_eq!(
            find_pii("S0022000088900281").as_deref(),
            Some("S0022-0000(88)90028-1"),
            "a packet's compact form"
        );
        assert_eq!(
            find_pii("0022000088900281").as_deref(),
            Some("0022-0000(88)90028-1")
        );
        assert_eq!(find_pii("A title with 1234-5678 in it"), None);
        assert_eq!(find_pii("x0022-0000(88)90028-1"), None, "inside a token");
        assert_eq!(find_pii("0022-0000(88)90028-12"), None, "runs on");
        assert_eq!(
            find_pii("card 1234567890123456 please"),
            None,
            "compact only alone"
        );
        assert_eq!(find_pii(""), None);
    }

    #[test]
    fn a_pii_converts_to_elseviers_doi_and_carries_its_year() {
        assert_eq!(
            doi_from_pii("0022-0000(88)90028-1"),
            "10.1016/0022-0000(88)90028-1",
            "no S printed, none added"
        );
        assert_eq!(
            doi_from_pii("S0370-2693(97)00412-1"),
            "10.1016/S0370-2693(97)00412-1",
            "the S stays"
        );
        assert_eq!(year_from_pii("0022-0000(88)90028-1"), Some(1988));
        assert_eq!(year_from_pii("S0022-0000(05)00001-X"), Some(2005));
        assert_eq!(year_from_pii("S0022-0000(50)00001-X"), Some(1950));
        assert_eq!(year_from_pii("S0022-0000(49)00001-X"), Some(2049));
        assert_eq!(year_from_pii("no parens"), None);
        assert_eq!(year_from_pii("(8)"), None);
    }

    #[test]
    fn issns_are_checked_before_a_pii_becomes_an_identity() {
        assert!(valid_issn("0022-0000"));
        assert!(valid_issn("00220000"));
        assert!(valid_issn("0370-2693"));
        assert!(valid_issn("2049-3630"), "X-free");
        assert!(valid_issn("0028-646X"), "X check");
        assert!(valid_issn("0028-646x"));
        assert!(!valid_issn("0378-595X"), "X where a 5 belongs");
        assert!(!valid_issn("0022-0001"), "one digit off");
        assert!(!valid_issn("0023-0000"));
        assert!(!valid_issn("0022-000"));
        assert!(!valid_issn("X022-0000"));
        assert!(!valid_issn(""));
        assert_eq!(issn_of_pii("S0022-0000(88)90028-1"), Some("0022-0000"));
        assert_eq!(issn_of_pii("0022-0000(88)90028-1"), Some("0022-0000"));
        assert_eq!(issn_of_pii("garbage"), None);
        assert!(pii_has_valid_issn("0022-0000(88)90028-1"));
        assert!(
            !pii_has_valid_issn("0022-0001(88)90028-1"),
            "a mangled digit"
        );
    }

    #[test]
    fn dois_give_their_year_only_where_the_family_puts_one() {
        // arXiv's own DOIs.
        assert_eq!(year_from_doi("10.48550/arXiv.2605.01288"), Some(2026));
        assert_eq!(year_from_doi("10.48550/arxiv.hep-th/9901001"), Some(1999));
        // Elsevier.
        assert_eq!(year_from_doi("10.1016/j.physa.2020.124578"), Some(2020));
        assert_eq!(year_from_doi("10.1016/j.cell.2020.01.001"), Some(2020));
        assert_eq!(year_from_doi("10.1016/0022-0000(88)90028-1"), Some(1988));
        assert_eq!(year_from_doi("10.1016/s0370-2693(97)00412-1"), Some(1997));
        assert_eq!(year_from_doi("10.1016/j.physa"), None);
        assert_eq!(year_from_doi("10.1016/b978-0-12-345678-9.00001-2"), None);
        // Springer Nature.
        assert_eq!(year_from_doi("10.1007/s11263-015-0816-y"), Some(2015));
        assert_eq!(year_from_doi("10.1007/s00220-999-01234-5"), Some(1999));
        assert_eq!(year_from_doi("10.1038/s41586-020-2649-2"), Some(2020));
        assert_eq!(year_from_doi("10.1038/nature12373"), None);
        assert_eq!(year_from_doi("10.1038/srep12345"), None);
        assert_eq!(year_from_doi("10.1007/978-3-030-12345-6_7"), None);
        assert_eq!(year_from_doi("10.1007/bf01234567"), None);
        // IEEE.
        assert_eq!(year_from_doi("10.1109/TPAMI.2019.2913372"), Some(2019));
        assert_eq!(year_from_doi("10.1109/cvpr.2016.90"), Some(2016));
        assert_eq!(year_from_doi("10.1109/5.771073"), None);
        // Wiley.
        assert_eq!(year_from_doi("10.1002/anie.201912345"), Some(2019));
        assert_eq!(year_from_doi("10.1002/adma.202001234"), Some(2020));
        assert_eq!(year_from_doi("10.1002/cpa.3160390305"), None);
        assert_eq!(year_from_doi("10.1002/anie.2019123"), None, "too short");
        // Prefixes are tolerated.
        assert_eq!(
            year_from_doi("https://doi.org/10.1016/j.physa.2020.124578"),
            Some(2020)
        );
        assert_eq!(year_from_doi("doi:10.1109/cvpr.2016.90"), Some(2016));
        // Nothing is guessed from a number that happens to look like a year.
        assert_eq!(year_from_doi("10.1103/PhysRevLett.78.2690"), None);
        assert_eq!(year_from_doi("10.1145/3292500.3330701"), None);
        assert_eq!(year_from_doi("10.1088/1742-5468/abc61e"), None);
        assert_eq!(year_from_doi("10.1000/2019"), None);
        assert_eq!(year_from_doi(""), None);
    }
}

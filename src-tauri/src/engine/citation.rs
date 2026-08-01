//! Citation identifier parsing shared by link extraction and the local-library
//! scanner. The parser is deliberately hand-written: identifiers are small,
//! ASCII grammars and adding a regex dependency would buy little here.

use super::types::CitationIdDto;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitationOccurrence {
    pub id: CitationIdDto,
    /// Character index in PDFium's raw character stream.
    pub start: u32,
    pub len: u32,
}

fn starts_ascii_case_insensitive(chars: &[char], at: usize, prefix: &str) -> bool {
    for (index, expected) in (at..).zip(prefix.chars()) {
        let Some(actual) = chars.get(index) else {
            return false;
        };
        if !actual.eq_ignore_ascii_case(&expected) {
            return false;
        }
    }
    true
}

fn doi_suffix_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ';' | '(' | ')' | '/' | ':' | '-' | ',')
}

fn parse_doi_at(chars: &[char], at: usize) -> Option<(usize, String)> {
    if chars.get(at) != Some(&'1')
        || chars.get(at + 1) != Some(&'0')
        || chars.get(at + 2) != Some(&'.')
    {
        return None;
    }
    let mut index = at + 3;
    let digits_start = index;
    while chars.get(index).is_some_and(char::is_ascii_digit) && index - digits_start < 9 {
        index += 1;
    }
    let digits = index - digits_start;
    if !(4..=9).contains(&digits)
        || chars.get(index).is_some_and(char::is_ascii_digit)
        || chars.get(index) != Some(&'/')
    {
        return None;
    }
    index += 1;
    let suffix_start = index;
    while chars.get(index).is_some_and(|ch| doi_suffix_char(*ch)) {
        index += 1;
    }
    while index > suffix_start && matches!(chars[index - 1], '.' | ',' | ')' | ';') {
        index -= 1;
    }
    if index == suffix_start {
        return None;
    }
    let value: String = chars[at..index]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    Some((index, value))
}

fn parse_modern_arxiv_at(chars: &[char], at: usize) -> Option<(usize, String)> {
    if !(0..4).all(|offset| chars.get(at + offset).is_some_and(char::is_ascii_digit))
        || chars.get(at + 4) != Some(&'.')
    {
        return None;
    }
    let mut index = at + 5;
    let digits_start = index;
    while chars.get(index).is_some_and(char::is_ascii_digit) && index - digits_start < 5 {
        index += 1;
    }
    let digits = index - digits_start;
    if !(4..=5).contains(&digits) || chars.get(index).is_some_and(char::is_ascii_digit) {
        return None;
    }
    let normalized_end = index;
    if chars.get(index).is_some_and(|ch| *ch == 'v' || *ch == 'V') {
        let version_start = index + 1;
        index = version_start;
        while chars.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        if index == version_start {
            return None;
        }
    }
    if chars
        .get(index)
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
    {
        return None;
    }
    Some((index, chars[at..normalized_end].iter().collect()))
}

fn legacy_category_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-')
}

fn parse_legacy_arxiv_at(chars: &[char], at: usize) -> Option<(usize, String)> {
    let mut slash = at;
    while slash < chars.len() && slash - at <= 24 && legacy_category_char(chars[slash]) {
        slash += 1;
    }
    if slash == at || chars.get(slash) != Some(&'/') {
        return None;
    }
    let digits_start = slash + 1;
    if !(0..7).all(|offset| {
        chars
            .get(digits_start + offset)
            .is_some_and(char::is_ascii_digit)
    }) || chars
        .get(digits_start + 7)
        .is_some_and(char::is_ascii_digit)
    {
        return None;
    }
    let normalized_end = digits_start + 7;
    let mut end = normalized_end;
    if chars.get(end).is_some_and(|ch| *ch == 'v' || *ch == 'V') {
        let version_start = end + 1;
        end = version_start;
        while chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        if end == version_start {
            return None;
        }
    }
    if chars
        .get(end)
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
    {
        return None;
    }
    Some((end, chars[at..normalized_end].iter().collect()))
}

fn parse_arxiv_at(chars: &[char], at: usize) -> Option<(usize, String)> {
    parse_modern_arxiv_at(chars, at).or_else(|| parse_legacy_arxiv_at(chars, at))
}

fn prefixed_at(chars: &[char], at: usize) -> Option<(usize, CitationIdDto)> {
    const DOI_PREFIXES: [&str; 4] = [
        "https://doi.org/",
        "http://doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ];
    for prefix in DOI_PREFIXES {
        if starts_ascii_case_insensitive(chars, at, prefix) {
            let value_at = at + prefix.chars().count();
            if let Some((end, value)) = parse_doi_at(chars, value_at) {
                return Some((end, CitationIdDto::Doi(value)));
            }
        }
    }

    const ARXIV_PREFIXES: [&str; 7] = [
        "https://arxiv.org/abs/",
        "http://arxiv.org/abs/",
        "arxiv.org/abs/",
        "https://arxiv.org/pdf/",
        "http://arxiv.org/pdf/",
        "arxiv.org/pdf/",
        "arxiv:",
    ];
    for prefix in ARXIV_PREFIXES {
        if starts_ascii_case_insensitive(chars, at, prefix) {
            let value_at = at + prefix.chars().count();
            if let Some((end, value)) = parse_arxiv_at(chars, value_at) {
                return Some((end, CitationIdDto::ArXiv(value)));
            }
        }
    }
    None
}

/// Finds only unambiguous, explicitly-prefixed identifiers in page text.
pub fn find_explicit_citations(text: &str) -> Vec<CitationOccurrence> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some((end, id)) = prefixed_at(&chars, index) {
            out.push(CitationOccurrence {
                id,
                start: index as u32,
                len: end.saturating_sub(index) as u32,
            });
            index = end.max(index + 1);
        } else {
            index += 1;
        }
    }
    out
}

/// Parses a citation from a URI, metadata string, or normalized bare id.
pub fn parse_citation_id(input: &str) -> Option<CitationIdDto> {
    let trimmed = input
        .trim()
        .trim_matches(|ch| matches!(ch, '<' | '>' | '"' | '\''));
    if let Some(found) = find_explicit_citations(trimmed).into_iter().next() {
        return Some(found.id);
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if let Some((_, value)) = parse_doi_at(&chars, 0) {
        return Some(CitationIdDto::Doi(value));
    }
    parse_arxiv_at(&chars, 0).map(|(_, value)| CitationIdDto::ArXiv(value))
}

/// Fast filename recognition used before opening a library PDF.
pub fn citation_from_filename(name: &str) -> Option<CitationIdDto> {
    let stem = name
        .strip_suffix(".pdf")
        .or_else(|| name.strip_suffix(".PDF"))
        .unwrap_or(name);
    let candidate = stem
        .strip_prefix("arXiv-")
        .or_else(|| stem.strip_prefix("arxiv-"))
        .or_else(|| stem.strip_prefix("arXiv_"))
        .or_else(|| stem.strip_prefix("arxiv_"))
        .unwrap_or(stem);
    if let Some(CitationIdDto::ArXiv(value)) = parse_citation_id(candidate) {
        return Some(CitationIdDto::ArXiv(value));
    }

    let chars: Vec<char> = stem.chars().collect();
    for at in 0..chars.len().saturating_sub(3) {
        if chars.get(at) != Some(&'1')
            || chars.get(at + 1) != Some(&'0')
            || chars.get(at + 2) != Some(&'.')
        {
            continue;
        }
        let mut separator = at + 3;
        while chars.get(separator).is_some_and(char::is_ascii_digit) {
            separator += 1;
        }
        if chars.get(separator) != Some(&'_') {
            continue;
        }
        let mut doi_chars = chars[at..].to_vec();
        doi_chars[separator - at] = '/';
        if let Some((_, value)) = parse_doi_at(&doi_chars, 0) {
            return Some(CitationIdDto::Doi(value));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_doi_and_strips_sentence_punctuation() {
        assert_eq!(
            parse_citation_id("doi:10.1145/1234567."),
            Some(CitationIdDto::Doi("10.1145/1234567".into()))
        );
    }

    #[test]
    fn parses_doi_url_and_rejects_incomplete_registrant() {
        assert_eq!(
            parse_citation_id("https://doi.org/10.1000/ABC_def"),
            Some(CitationIdDto::Doi("10.1000/abc_def".into()))
        );
        assert_eq!(parse_citation_id("10.114/nope"), None);
        assert_eq!(parse_citation_id("10.1145"), None);
    }

    #[test]
    fn strips_arxiv_versions_for_modern_and_legacy_ids() {
        assert_eq!(
            parse_citation_id("arXiv:2401.12345v2"),
            Some(CitationIdDto::ArXiv("2401.12345".into()))
        );
        assert_eq!(
            parse_citation_id("https://arxiv.org/abs/math.GT/0309136v1"),
            Some(CitationIdDto::ArXiv("math.GT/0309136".into()))
        );
        assert_eq!(parse_citation_id("arXiv:2401.12345version"), None);
    }

    #[test]
    fn reports_pdfium_character_ranges_for_explicit_text_ids() {
        let found = find_explicit_citations("See doi:10.1000/example and arXiv:2401.12345v3.");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].start, 4);
        assert_eq!(found[0].len, 19);
        assert_eq!(found[1].id, CitationIdDto::ArXiv("2401.12345".into()));
    }

    #[test]
    fn recognizes_common_library_filename_patterns() {
        assert_eq!(
            citation_from_filename("arXiv-2401.12345v2.pdf"),
            Some(CitationIdDto::ArXiv("2401.12345".into()))
        );
        assert_eq!(
            citation_from_filename("10.1145_1234567.pdf"),
            Some(CitationIdDto::Doi("10.1145/1234567".into()))
        );
    }

    #[test]
    fn citation_ids_use_the_frontend_tag_shape() {
        assert_eq!(
            serde_json::to_string(&CitationIdDto::Doi("10.1000/test".into())).unwrap(),
            r#"{"scheme":"doi","value":"10.1000/test"}"#
        );
        assert_eq!(
            serde_json::to_string(&CitationIdDto::ArXiv("2401.12345".into())).unwrap(),
            r#"{"scheme":"arXiv","value":"2401.12345"}"#
        );
    }
}

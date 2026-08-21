//! Which documents are worth reading at all.
//!
//! A library search reads every sidecar to find the few that match. At a
//! thousand documents that is ~100 MB a query and over a second, nearly all of
//! it spent on files that contain nothing. This narrows the field first.
//!
//! Each document gets a bitmap of the character trigrams it contains. A query
//! can only appear in a document that holds *all* of the query's trigrams, so
//! a distinctive phrase rules out almost everything without opening a file.
//!
//! It is a filter, not an index: a false positive costs one wasted read, which
//! the ordinary scan then rejects. A false negative would be a wrong answer,
//! so the construction never has one — anything the mapping cannot represent
//! makes the filter answer "maybe" instead of "no".
//!
//! Deliberately not a trie. Tries index token prefixes and would answer a
//! different question than the substring-and-phrase matching the rest of
//! search already guarantees, where "file" matches "profile" and a phrase
//! spans a line break.

/// Symbols the alphabet folds to: 26 letters, one for all digits, one for
/// whitespace, one for everything else, padded to a power of two so an index
/// is two shifts.
const SYMBOLS: usize = 32;
const TRIGRAMS: usize = SYMBOLS * SYMBOLS * SYMBOLS;
/// 32768 bits.
pub const FILTER_BYTES: usize = TRIGRAMS / 8;

const DIGIT: u8 = 26;
const SPACE: u8 = 27;
const OTHER: u8 = 28;

/// Fold a character into the alphabet.
///
/// Collapsing digits together and everything unusual into one symbol keeps the
/// bitmap small. It costs precision — "x1" and "x2" look alike — which makes
/// the filter admit documents it need not; it never makes it reject one.
fn symbol(ch: char) -> u8 {
    match ch {
        'a'..='z' => ch as u8 - b'a',
        'A'..='Z' => ch.to_ascii_lowercase() as u8 - b'a',
        '0'..='9' => DIGIT,
        c if c.is_whitespace() => SPACE,
        _ => OTHER,
    }
}

/// A document's trigrams.
#[derive(Clone)]
pub struct Filter {
    bits: Vec<u8>,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            bits: vec![0; FILTER_BYTES],
        }
    }
}

fn index_of(a: u8, b: u8, c: u8) -> usize {
    (a as usize) << 10 | (b as usize) << 5 | (c as usize)
}

/// Every trigram in `text`, as (index) values.
fn trigrams(text: &str, mut visit: impl FnMut(usize)) {
    let mut window = [0u8; 3];
    let mut filled = 0usize;
    for ch in text.chars() {
        window[0] = window[1];
        window[1] = window[2];
        window[2] = symbol(ch);
        filled += 1;
        if filled >= 3 {
            visit(index_of(window[0], window[1], window[2]));
        }
    }
}

impl Filter {
    /// Test-only: real documents are folded in page by page with `add`.
    #[cfg(test)]
    pub fn from_text(text: &str) -> Self {
        let mut filter = Filter::default();
        filter.add(text);
        filter
    }

    /// Fold another page into the same document filter.
    pub fn add(&mut self, text: &str) {
        trigrams(text, |index| {
            self.bits[index / 8] |= 1 << (index % 8);
        });
    }

    fn holds(&self, index: usize) -> bool {
        self.bits[index / 8] & (1 << (index % 8)) != 0
    }

    /// Whether this document could contain `query`.
    ///
    /// A query shorter than a trigram carries no information here, so it is
    /// admitted rather than guessed at — the scan behind this will settle it.
    pub fn might_contain(&self, query: &str) -> bool {
        if query.chars().count() < 3 {
            return true;
        }
        let mut possible = true;
        trigrams(query, |index| {
            if !self.holds(index) {
                possible = false;
            }
        });
        possible
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    /// Rebuild from stored bytes, or nothing if they are not a filter.
    pub fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        (bytes.len() == FILTER_BYTES).then_some(Filter { bits: bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_text_it_was_built_from() {
        let filter = Filter::from_text("variance collapse under Rao-Blackwellization");
        for query in [
            "variance",
            "collapse",
            "variance collapse",
            "Rao-Blackwell",
            "under",
        ] {
            assert!(filter.might_contain(query), "{query}");
        }
    }

    #[test]
    fn rejects_what_cannot_be_there() {
        // The whole point: a distinctive phrase rules a document out without
        // anyone reading it.
        let filter = Filter::from_text("variance collapse under smoothing");
        for query in ["quaternion", "zzzznotinanypaper", "hyperbolic geometry"] {
            assert!(!filter.might_contain(query), "{query}");
        }
    }

    #[test]
    fn never_rejects_something_present_however_odd() {
        // A false negative is a wrong answer, so every one of these must be
        // admitted — punctuation, maths, mixed scripts and all.
        let text = "σ² ≤ ∇_M log q(x) — see §4.1 (Eq. 12), naïve ﬁle";
        let filter = Filter::from_text(text);
        for query in ["σ² ≤", "∇_M log", "§4.1", "(Eq. 12)", "naïve", "ﬁle"] {
            assert!(filter.might_contain(query), "{query:?} is in the text");
        }
    }

    #[test]
    fn case_and_query_length_never_cause_a_miss() {
        let filter = Filter::from_text("Variance Collapse");
        assert!(filter.might_contain("variance collapse"));
        assert!(filter.might_contain("VARIANCE"));
        // Under three characters carries no trigram, so it must be admitted
        // rather than rejected on no evidence.
        assert!(filter.might_contain("va"));
        assert!(filter.might_contain(""));
    }

    #[test]
    fn pages_accumulate_into_one_document_filter() {
        let mut filter = Filter::default();
        filter.add("first page about manifolds");
        filter.add("second page about quaternions");
        assert!(filter.might_contain("manifolds"));
        assert!(filter.might_contain("quaternions"));
        assert!(!filter.might_contain("zzzznotinanypaper"));
    }

    #[test]
    fn survives_a_round_trip_through_bytes() {
        let filter = Filter::from_text("variance collapse");
        let restored = Filter::from_bytes(filter.as_bytes().to_vec()).expect("same length");
        assert!(restored.might_contain("variance collapse"));
        assert!(!restored.might_contain("quaternion"));
        assert_eq!(filter.as_bytes().len(), FILTER_BYTES);
        // Anything that is not a filter is refused rather than read as one.
        assert!(Filter::from_bytes(vec![0; 10]).is_none());
    }

    #[test]
    fn an_empty_document_admits_only_short_queries() {
        let filter = Filter::default();
        assert!(!filter.might_contain("anything"));
        assert!(filter.might_contain("an"));
    }
}

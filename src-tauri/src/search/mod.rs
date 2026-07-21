//! Incremental, byte-budgeted search index.
//!
//! Pages are normalized once (per-char NFKC → ligatures/fullwidth folded,
//! curly quotes straightened, whitespace runs collapsed) with a map back to
//! PDFium character indices so match highlights can be resolved to page
//! rects. Queries run over whatever subset is indexed so far — results appear
//! before the whole document is processed.

use crate::engine::types::{DocId, MatchDto, PageMatchesDto, TextRun};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

pub struct PageEntry {
    /// normalized characters (case preserved; fold at query time if needed)
    pub norm: Vec<char>,
    /// norm index → PDFium char index
    pub map: Vec<u32>,
    /// cached layout runs for the text-selection layer
    pub runs: Vec<TextRun>,
    pub char_count: u32,
    pub cost: u64,
}

#[derive(Default)]
pub struct DocIndex {
    pub pages: HashMap<u16, PageEntry>,
    pub truncated: bool,
}

pub struct SearchStore {
    docs: HashMap<DocId, DocIndex>,
    budget: u64,
    used: u64,
}

/// Normalize one raw character stream. Returns (norm chars, norm→raw map).
pub fn normalize(raw: &str) -> (Vec<char>, Vec<u32>) {
    let mut norm: Vec<char> = Vec::with_capacity(raw.len());
    let mut map: Vec<u32> = Vec::with_capacity(raw.len());
    let mut in_ws = false;
    for (i, c) in raw.chars().enumerate() {
        let idx = i as u32;
        if c.is_whitespace() {
            if !in_ws && !norm.is_empty() {
                norm.push(' ');
                map.push(idx);
            }
            in_ws = true;
            continue;
        }
        in_ws = false;
        let straightened = match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => '"',
            _ => c,
        };
        if straightened != c {
            norm.push(straightened);
            map.push(idx);
            continue;
        }
        // per-char NFKC folds ligatures (ﬁ→fi) and fullwidth forms; every
        // expansion char maps back to the same raw index
        for n in c.nfkc() {
            norm.push(n);
            map.push(idx);
        }
    }
    (norm, map)
}

/// Case-insensitive single-char fold (first lowercase mapping).
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Find matches of `query` (raw user input) inside a normalized page.
/// Returns (pdfium_start, pdfium_len, snippet) triples.
pub fn find_matches(
    entry_norm: &[char],
    entry_map: &[u32],
    query: &str,
    case_sensitive: bool,
    limit: usize,
) -> Vec<(u32, u32, String)> {
    let (mut q, _) = normalize(query);
    if q.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = if case_sensitive {
        entry_norm.to_vec()
    } else {
        q = q.into_iter().map(fold).collect();
        entry_norm.iter().map(|c| fold(*c)).collect()
    };
    let n = hay.len();
    let m = q.len();
    let mut out = Vec::new();
    if n < m {
        return out;
    }
    let mut i = 0;
    while i + m <= n && out.len() < limit {
        if hay[i..i + m] == q[..] {
            let start = entry_map[i];
            let len = entry_map[i + m - 1] - start + 1;
            let s0 = i.saturating_sub(30);
            let s1 = (i + m + 30).min(n);
            let snippet: String = entry_norm[s0..s1].iter().collect();
            out.push((start, len, snippet));
            i += m;
        } else {
            i += 1;
        }
    }
    out
}

impl SearchStore {
    pub fn new(budget: u64) -> Self {
        SearchStore {
            docs: HashMap::new(),
            budget,
            used: 0,
        }
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Store an extracted page. Returns false (and marks the doc truncated)
    /// when the budget is exhausted — indexing should stop.
    pub fn store_page(
        &mut self,
        doc: DocId,
        src: u16,
        raw: &str,
        runs: Vec<TextRun>,
        char_count: u32,
    ) -> bool {
        let (norm, map) = normalize(raw);
        let cost = (norm.len() * 8 + map.len() * 4 + runs.len() * 48 + raw.len()) as u64;
        let entry = PageEntry {
            norm,
            map,
            runs,
            char_count,
            cost,
        };
        let idx = self.docs.entry(doc).or_default();
        if self.used + cost > self.budget {
            idx.truncated = true;
            return false;
        }
        if let Some(old) = idx.pages.insert(src, entry) {
            self.used = self.used.saturating_sub(old.cost);
        }
        self.used += cost;
        true
    }

    pub fn page(&self, doc: DocId, src: u16) -> Option<&PageEntry> {
        self.docs.get(&doc).and_then(|d| d.pages.get(&src))
    }

    pub fn doc(&self, doc: DocId) -> Option<&DocIndex> {
        self.docs.get(&doc)
    }

    pub fn indexed_count(&self, doc: DocId) -> u32 {
        self.docs.get(&doc).map(|d| d.pages.len() as u32).unwrap_or(0)
    }

    pub fn is_truncated(&self, doc: DocId) -> bool {
        self.docs.get(&doc).map(|d| d.truncated).unwrap_or(false)
    }

    pub fn remove_doc(&mut self, doc: DocId) {
        if let Some(d) = self.docs.remove(&doc) {
            for (_, e) in d.pages {
                self.used = self.used.saturating_sub(e.cost);
            }
        }
    }

    /// Query the indexed subset of a document, page-ordered.
    pub fn query(
        &self,
        doc: DocId,
        query: &str,
        case_sensitive: bool,
        per_page_limit: usize,
    ) -> Vec<PageMatchesDto> {
        let Some(d) = self.docs.get(&doc) else {
            return Vec::new();
        };
        let mut pages: Vec<&u16> = d.pages.keys().collect();
        pages.sort();
        let mut out = Vec::new();
        for src in pages {
            let e = &d.pages[src];
            let found = find_matches(&e.norm, &e.map, query, case_sensitive, per_page_limit);
            if !found.is_empty() {
                out.push(PageMatchesDto {
                    src: *src as u32,
                    matches: found
                        .into_iter()
                        .map(|(start, len, snippet)| MatchDto { start, len, snippet })
                        .collect(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &str, q: &str, cs: bool) -> Vec<(u32, u32, String)> {
        let (norm, map) = normalize(text);
        find_matches(&norm, &map, q, cs, 100)
    }

    #[test]
    fn normalize_folds_ligatures_via_nfkc() {
        let (norm, map) = normalize("ﬁle");
        assert_eq!(norm.iter().collect::<String>(), "file");
        // both expansion chars of ﬁ map back to raw char 0
        assert_eq!(map[0], 0);
        assert_eq!(map[1], 0);
        assert_eq!(map[2], 1);
    }

    #[test]
    fn normalize_collapses_whitespace_runs() {
        let (norm, _) = normalize("hello \t\n  world");
        assert_eq!(norm.iter().collect::<String>(), "hello world");
    }

    #[test]
    fn normalize_straightens_curly_quotes() {
        let (norm, _) = normalize("it’s “fine”");
        assert_eq!(norm.iter().collect::<String>(), "it's \"fine\"");
    }

    #[test]
    fn finds_case_insensitive_matches_with_pdfium_ranges() {
        let found = matches("The Quick brown the END", "the", false);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, 0); // "The" at raw char 0
        assert_eq!(found[0].1, 3);
        assert_eq!(found[1].0, 16); // "the" at raw char 16
    }

    #[test]
    fn case_sensitive_toggle_respects_case() {
        let found = matches("The Quick brown the END", "the", true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 16);
    }

    #[test]
    fn query_spanning_collapsed_whitespace_matches() {
        let found = matches("hello   \n world", "hello world", false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 0);
        // match length in RAW chars covers the whole whitespace run
        assert_eq!(found[0].1, 15);
    }

    #[test]
    fn ligature_text_matches_plain_query() {
        let found = matches("my ﬁle here", "file", false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 3);
        assert_eq!(found[0].1, 3, "ﬁ+l+e is 3 raw chars");
    }

    #[test]
    fn snippets_surround_the_match() {
        let found = matches("aaaa needle bbbb", "needle", false);
        assert!(found[0].2.contains("needle"));
    }

    #[test]
    fn store_respects_budget_and_reports_truncation() {
        let mut s = SearchStore::new(200);
        assert!(s.store_page(1, 0, "small", Vec::new(), 5));
        let big = "x".repeat(10_000);
        assert!(!s.store_page(1, 1, &big, Vec::new(), 10_000));
        assert!(s.is_truncated(1));
        assert_eq!(s.indexed_count(1), 1);
        s.remove_doc(1);
        assert_eq!(s.used(), 0);
    }

    #[test]
    fn query_returns_page_ordered_results_from_indexed_subset() {
        let mut s = SearchStore::new(1_000_000);
        s.store_page(1, 2, "beta beta", Vec::new(), 9);
        s.store_page(1, 0, "alpha beta", Vec::new(), 10);
        let r = s.query(1, "beta", false, 10);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].src, 0);
        assert_eq!(r[1].src, 2);
        assert_eq!(r[1].matches.len(), 2);
    }
}

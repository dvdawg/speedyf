//! The library's own search index: one text sidecar per document, on disk.
//!
//! Separate from `SearchStore`, which cannot serve this. That store is keyed by
//! the ephemeral `DocId` a file gets when it is opened, holds ~2.1× the raw
//! text inside a 64 MiB budget shared with every open tab, and is dropped when
//! a document closes. A library of a few hundred papers is more text than the
//! budget allows and has to survive quitting, so it needs a different home.
//!
//! What is *not* duplicated is the matching. `PageEntry`, `prepare_query` and
//! `find_prepared_matches` come from the parent module unchanged: they define
//! what "matches" means here — NFKC, collapsed whitespace, straightened
//! quotes, folded ligatures — and the offset map they carry is the bridge back
//! to raw PDFium character indices, which is what lets a hit found here be
//! highlighted later by the ordinary per-document path.
//!
//! Sidecars hold text already normalized *and* case-folded. Storing only the
//! normalized form was the first design, on the reasoning that folding is
//! cheap — measurement disagreed: re-folding the library cost ~770ms a query,
//! because it allocates a lowercased copy of every page whether or not the
//! page can match. The folded copy costs a few megabytes of disk and removes
//! that entirely.

use super::gramindex::{signature_of, Candidates, GramIndex};
use super::trigram::{Filter, FILTER_BYTES};
use super::{find_prepared_matches, prepare_query, PageEntry};
use crate::engine::types::{LibraryHitDto, MatchDto, PageMatchesDto};
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Bumped when the sidecar shape changes. Anything else is discarded and
/// re-extracted rather than misread.
const SIDECAR_VERSION: u32 = 1;
/// Refuse to load something implausible as one document's text.
const MAX_SIDECAR_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct StoredPage {
    src: u32,
    /// normalized text — what snippets are cut from
    text: String,
    /// case-folded text — what a case-insensitive query scans
    folded: String,
    /// offset-map breakpoints, mapping a hit back to a raw PDFium index
    breaks: Vec<(u32, i32)>,
    char_count: u32,
}

#[derive(Serialize, Deserialize)]
struct Sidecar {
    version: u32,
    /// The document this came from and the state it was in. Both are checked
    /// on load: a sidecar is valid only for the exact bytes it was built from,
    /// and the stored path catches a hash collision in the file name rather
    /// than trusting it.
    path: PathBuf,
    mtime_ms: u64,
    size: u64,
    pages: Vec<StoredPage>,
}

/// One document's pages, ready to search.
pub struct IndexedDocument {
    pub pages: Vec<(u32, PageEntry)>,
}

/// Where a document's scan copy lives: its folded text and nothing else.
///
/// The JSON sidecar carries the normalized text and offset maps a *match*
/// needs, which makes it ~3x the size of the text and expensive to parse. A
/// search reads every candidate but matches in almost none of them, so the
/// scanning path gets its own file holding only what scanning uses.
pub fn scan_path(app_data_dir: &Path, document: &Path) -> PathBuf {
    sidecar_path(app_data_dir, document).with_extension("fold")
}

/// Where a document's trigram filter lives — 4 KB beside the text it stands
/// for, so a search can rule the document out without reading the text.
pub fn filter_path(app_data_dir: &Path, document: &Path) -> PathBuf {
    sidecar_path(app_data_dir, document).with_extension("tri")
}

/// Where a document's sidecar lives.
///
/// Named from a hash of the path rather than the path itself: paths hold
/// separators and characters a file name cannot, and run longer than most file
/// systems allow.
pub fn sidecar_path(app_data_dir: &Path, document: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in document.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    app_data_dir.join("text").join(format!("{hash:016x}.json"))
}

/// Write a document's extracted text beside the library index.
///
/// `pages` arrives as the raw PDFium stream; normalizing here means every
/// reader of a sidecar sees the same form.
pub fn write_sidecar(
    app_data_dir: &Path,
    document: &Path,
    mtime_ms: u64,
    size: u64,
    pages: &[(u32, String)],
) -> AppResult<()> {
    let stored: Vec<StoredPage> = pages
        .iter()
        .map(|(src, raw)| {
            let count = raw.chars().count().min(u32::MAX as usize) as u32;
            let entry = PageEntry::new_indexed(raw, count);
            let (text, folded, breaks, char_count) = entry.indexed_parts();
            StoredPage {
                src: *src,
                text: text.to_owned(),
                folded: folded.to_owned(),
                breaks: breaks.to_vec(),
                char_count,
            }
        })
        .collect();

    let sidecar = Sidecar {
        version: SIDECAR_VERSION,
        path: document.to_path_buf(),
        mtime_ms,
        size,
        pages: stored,
    };
    let target = sidecar_path(app_data_dir, document);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Io(format!("cannot create text index directory: {e}")))?;
    }
    let bytes = serde_json::to_vec(&sidecar)
        .map_err(|e| AppError::Internal(format!("cannot serialize text index: {e}")))?;
    std::fs::write(&target, bytes)
        .map_err(|e| AppError::Io(format!("cannot write text index: {e}")))?;

    // The filter is written second and read first: if writing it fails the
    // document is simply always read, which is slower and still correct.
    let mut filter = Filter::default();
    for page in &sidecar.pages {
        filter.add(&page.folded);
    }
    let _ = std::fs::write(filter_path(app_data_dir, document), filter.as_bytes());

    // The scan copy: every page's folded text run together. Page boundaries do
    // not matter here — this file only ever answers "is the phrase anywhere in
    // this document", and the sidecar settles where.
    let scan: String = sidecar
        .pages
        .iter()
        .map(|page| page.folded.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(scan_path(app_data_dir, document), scan);
    Ok(())
}

/// Read a sidecar's bytes, or nothing if there is no usable file there.
///
/// Split from parsing so a search can reject a document on its raw bytes —
/// see `worth_parsing` — without ever building the strings inside it.
fn read_filter(app_data_dir: &Path, document: &Path) -> Option<Filter> {
    let path = filter_path(app_data_dir, document);
    if std::fs::metadata(&path).ok()?.len() != FILTER_BYTES as u64 {
        return None;
    }
    Filter::from_bytes(std::fs::read(&path).ok()?)
}

fn read_sidecar_bytes(app_data_dir: &Path, document: &Path) -> Option<Vec<u8>> {
    let target = sidecar_path(app_data_dir, document);
    if std::fs::metadata(&target).ok()?.len() > MAX_SIDECAR_BYTES {
        return None;
    }
    std::fs::read(&target).ok()
}

/// Whether a document is worth parsing at all.
///
/// The sidecar stores the folded text verbatim, so if the prepared query does
/// not appear anywhere in the file's bytes it cannot appear in any page, and
/// the document can be skipped without building a single `String`. Parsing is
/// what a library search actually spends its time on, so skipping it for the
/// documents that cannot match is most of the speed.
///
/// Only sound when the query survives JSON escaping unchanged: a `"` in the
/// text is stored as `\"`, so a query containing one could be missed. Those
/// queries fall back to parsing, which is correct, just slower.
fn worth_parsing(bytes: &[u8], prepared: &str) -> bool {
    if prepared
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control())
    {
        return true;
    }
    // The file is JSON, so it is valid UTF-8; validating it is a fast scan and
    // buys `str::contains`, which is a proper substring search rather than a
    // naive window comparison.
    std::str::from_utf8(bytes).is_ok_and(|text| text.contains(prepared))
}

fn parse_sidecar(
    bytes: &[u8],
    document: &Path,
    mtime_ms: u64,
    size: u64,
) -> Option<IndexedDocument> {
    let sidecar: Sidecar = serde_json::from_slice(bytes).ok()?;
    if sidecar.version != SIDECAR_VERSION
        || sidecar.path != document
        || sidecar.mtime_ms != mtime_ms
        || sidecar.size != size
    {
        return None;
    }
    Some(IndexedDocument {
        pages: sidecar
            .pages
            .into_iter()
            .map(|page| {
                (
                    page.src,
                    PageEntry::from_indexed(page.text, page.folded, page.breaks, page.char_count),
                )
            })
            .collect(),
    })
}

/// Delete a document's sidecar. Best effort: a leftover only wastes disk, and
/// is rejected on load anyway once its fingerprint stops matching.
pub fn remove_sidecar(app_data_dir: &Path, document: &Path) {
    let _ = std::fs::remove_file(sidecar_path(app_data_dir, document));
    let _ = std::fs::remove_file(filter_path(app_data_dir, document));
    let _ = std::fs::remove_file(scan_path(app_data_dir, document));
}

/// Matches within one document.
pub fn search_document(
    document: &IndexedDocument,
    prepared: &str,
    case_sensitive: bool,
    limit: usize,
) -> Vec<PageMatchesDto> {
    let mut out = Vec::new();
    let mut budget = limit;
    for (src, entry) in &document.pages {
        if budget == 0 {
            break;
        }
        let found = find_prepared_matches(entry, prepared, case_sensitive, budget.min(20));
        if found.is_empty() {
            continue;
        }
        budget = budget.saturating_sub(found.len());
        out.push(PageMatchesDto {
            src: *src,
            matches: found
                .into_iter()
                .map(|(start, len, snippet)| MatchDto {
                    start,
                    len,
                    snippet,
                })
                .collect(),
        });
    }
    out
}

/// Where the library's inverted gram index lives — one file for the whole
/// library, unlike the per-document sidecars beside it.
pub fn gram_index_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("text").join("grams.idx")
}

/// The identity of the current document set, so an index built for a different
/// one can be spotted rather than trusted.
pub fn library_signature(documents: &[LibraryDocument]) -> u64 {
    let fingerprints: Vec<(PathBuf, u64, u64)> = documents
        .iter()
        .map(|document| (document.path.clone(), document.mtime_ms, document.size))
        .collect();
    signature_of(&fingerprints)
}

/// Build the inverted index by reading every document's scan copy.
///
/// This is the expensive operation in the whole feature — it reads the
/// library's entire folded text once — so it is done off any hot path and its
/// result persisted. A document whose scan copy is unreadable is passed
/// through as unknown rather than dropped.
pub fn build_gram_index(app_data_dir: &Path, documents: &[LibraryDocument]) -> GramIndex {
    let texts: Vec<(PathBuf, Option<Vec<u8>>)> = documents
        .iter()
        .map(|document| {
            let text = std::fs::read(scan_path(app_data_dir, &document.path)).ok();
            (document.path.clone(), text)
        })
        .collect();
    GramIndex::build(texts, library_signature(documents))
}

/// Read the persisted index, but only if it still describes this library.
pub fn load_gram_index(app_data_dir: &Path, signature: u64) -> Option<GramIndex> {
    let bytes = std::fs::read(gram_index_path(app_data_dir)).ok()?;
    let index = GramIndex::from_bytes(&bytes)?;
    (index.signature() == signature).then_some(index)
}

/// Persist the index. Best effort: failing to save costs a rebuild next
/// launch, never a wrong answer.
pub fn save_gram_index(app_data_dir: &Path, index: &GramIndex) {
    let path = gram_index_path(app_data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, index.to_bytes());
}

/// A document to search: where it is, what it is called, and the state it was
/// in when indexed.
pub struct LibraryDocument {
    pub path: PathBuf,
    pub title: Option<String>,
    pub mtime_ms: u64,
    pub size: u64,
}

/// Search a whole library.
///
/// A document with no valid sidecar is skipped silently: "not indexed yet" is
/// the normal state during the background pass, not a failure.
///
/// Three filters stand in front of the actual matching, each cheaper than the
/// one behind it:
///
/// 1. **The gram index**, when one has been built. It answers "which documents
///    could contain this" without touching the documents at all, which is the
///    only step whose cost does not grow with the library. On a real
///    1017-paper corpus a distinctive phrase leaves fewer than seventy.
/// 2. **The trigram filter** — 4 KB per document, against ~100 KB of text.
/// 3. **The scan copy** — the folded text with nothing else, a third the size
///    of the sidecar and needing no parsing.
///
/// Only a document that survives all three pays to have its sidecar parsed.
/// Every one of them is allowed to admit a document it need not; none may
/// reject one it should keep.
pub fn search_library(
    app_data_dir: &Path,
    documents: &[LibraryDocument],
    index: Option<&GramIndex>,
    query: &str,
    case_sensitive: bool,
    per_document_limit: usize,
    document_limit: usize,
) -> (Vec<LibraryHitDto>, bool) {
    let prepared = prepare_query(query, case_sensitive);
    if prepared.is_empty() {
        return (Vec::new(), false);
    }
    // The filters all work against folded text, so they are asked the folded
    // question whatever the search's own case sensitivity. Folding widens what
    // matches, so this can only ever admit extra documents — asking a
    // case-sensitive query of folded text would wrongly reject them.
    let folded = prepare_query(query, false);

    // Documents the index has ruled in. `None` means it could not narrow this
    // query — every gram in it was too common — and everything is considered,
    // which is the right answer for a phrase that common anyway.
    let allowed: Option<HashSet<&Path>> = index.and_then(|index| match index.candidates(&folded) {
        Candidates::Unknown => None,
        Candidates::Only(slots) => Some(
            slots
                .iter()
                .filter_map(|slot| index.document(*slot))
                .collect(),
        ),
    });

    let mut hits: Vec<LibraryHitDto> = Vec::new();
    let mut truncated = false;
    for document in documents {
        if hits.len() >= document_limit {
            truncated = true;
            break;
        }
        if allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(document.path.as_path()))
        {
            continue;
        }
        // A document with no filter — written before filters existed, or whose
        // write failed — is read as before.
        if let Some(filter) = read_filter(app_data_dir, &document.path) {
            if !filter.might_contain(&folded) {
                continue;
            }
        }
        // No scan copy — indexed before this existed, or the write failed —
        // falls through to the sidecar, which is slower and correct.
        if let Ok(scan) = std::fs::read_to_string(scan_path(app_data_dir, &document.path)) {
            if !scan.contains(&folded) {
                continue;
            }
        }
        let Some(bytes) = read_sidecar_bytes(app_data_dir, &document.path) else {
            continue;
        };
        if !worth_parsing(&bytes, &prepared) {
            continue;
        }
        let Some(indexed) = parse_sidecar(&bytes, &document.path, document.mtime_ms, document.size)
        else {
            continue;
        };
        let pages = search_document(&indexed, &prepared, case_sensitive, per_document_limit);
        if pages.is_empty() {
            continue;
        }
        let total: u32 = pages
            .iter()
            .map(|page| page.matches.len().min(u32::MAX as usize) as u32)
            .sum();
        hits.push(LibraryHitDto {
            path: document.path.to_string_lossy().into_owned(),
            name: document
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            title: document.title.clone(),
            total_matches: total,
            pages,
        });
    }

    // Most matches first: among hundreds of papers, the one that mentions a
    // phrase repeatedly is nearly always the one being looked for.
    hits.sort_by(|a, b| {
        b.total_matches
            .cmp(&a.total_matches)
            .then_with(|| a.name.cmp(&b.name))
    });
    (hits, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Load a document exactly the way a search does.
    fn load(dir: &Path, doc: &LibraryDocument) -> Option<IndexedDocument> {
        let bytes = read_sidecar_bytes(dir, &doc.path)?;
        parse_sidecar(&bytes, &doc.path, doc.mtime_ms, doc.size)
    }

    fn document(dir: &Path, name: &str, pages: &[(u32, &str)]) -> LibraryDocument {
        let path = dir.join(name);
        std::fs::write(&path, b"%PDF-1.7\n").expect("write pdf");
        let meta = std::fs::metadata(&path).expect("stat");
        let owned: Vec<(u32, String)> = pages
            .iter()
            .map(|(src, text)| (*src, (*text).to_string()))
            .collect();
        write_sidecar(dir, &path, 1_000, meta.len(), &owned).expect("sidecar");
        LibraryDocument {
            path,
            title: Some(name.to_string()),
            mtime_ms: 1_000,
            size: meta.len(),
        }
    }

    #[test]
    fn a_sidecar_round_trips_and_keeps_raw_offsets() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, "The quick  brown fox")]);
        let indexed = load(dir.path(), &doc).expect("valid sidecar");
        let prepared = prepare_query("brown", false);
        let found = search_document(&indexed, &prepared, false, 10);
        assert_eq!(found.len(), 1);
        // The double space is collapsed when normalized, so a raw index that
        // ignored the offset map would land one character early.
        assert_eq!(found[0].matches[0].start, 11);
        assert_eq!(found[0].matches[0].len, 5);
    }

    #[test]
    fn a_sidecar_for_different_bytes_is_refused() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, "hello")]);
        // Same path, different fingerprint: the file changed under us.
        let bytes = read_sidecar_bytes(dir.path(), &doc.path).expect("bytes");
        assert!(parse_sidecar(&bytes, &doc.path, doc.mtime_ms + 1, doc.size).is_none());
        assert!(parse_sidecar(&bytes, &doc.path, doc.mtime_ms, doc.size + 1).is_none());
        assert!(parse_sidecar(&bytes, &doc.path, doc.mtime_ms, doc.size).is_some());
    }

    #[test]
    fn a_corrupt_sidecar_is_skipped_rather_than_fatal() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, "hello")]);
        std::fs::write(sidecar_path(dir.path(), &doc.path), b"{ not json").expect("clobber");
        assert!(load(dir.path(), &doc).is_none());
        // ...and a search over it simply finds nothing, rather than failing.
        let (hits, _) = search_library(dir.path(), &[doc], None, "hello", false, 10, 10);
        assert!(hits.is_empty());
    }

    #[test]
    fn a_missing_sidecar_means_not_indexed_yet_not_an_error() {
        let dir = tempfile::tempdir().expect("dir");
        let absent = LibraryDocument {
            path: dir.path().join("never-indexed.pdf"),
            title: None,
            mtime_ms: 1,
            size: 1,
        };
        let (hits, truncated) =
            search_library(dir.path(), &[absent], None, "anything", false, 10, 10);
        assert!(hits.is_empty() && !truncated);
    }

    #[test]
    fn results_lead_with_the_document_that_says_it_most() {
        let dir = tempfile::tempdir().expect("dir");
        let sparse = document(dir.path(), "sparse.pdf", &[(0, "manifold")]);
        let dense = document(
            dir.path(),
            "dense.pdf",
            &[(0, "manifold manifold"), (3, "manifold again")],
        );
        let (hits, _) = search_library(
            dir.path(),
            &[sparse, dense],
            None,
            "manifold",
            false,
            20,
            10,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "dense.pdf");
        assert_eq!(hits[0].total_matches, 3);
        // Page numbers survive, so a hit can be opened where it actually is.
        assert_eq!(hits[0].pages[1].src, 3);
    }

    #[test]
    fn a_phrase_broken_across_lines_still_matches() {
        // The normalization the parent module already does is what makes this
        // work; the test is here to catch a sidecar that stored the raw form.
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, "variance\ncollapse on")]);
        let (hits, _) =
            search_library(dir.path(), &[doc], None, "variance collapse", false, 10, 10);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn a_query_with_a_quote_still_searches() {
        // The byte prefilter is unsound for anything JSON escapes, so those
        // queries must fall back to parsing rather than silently miss.
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, r#"he said "hello" loudly"#)]);
        let (hits, _) = search_library(dir.path(), &[doc], None, r#""hello""#, false, 10, 10);
        assert_eq!(hits.len(), 1, "a quoted phrase must still be findable");
    }

    #[test]
    fn the_gram_index_narrows_without_losing_anything() {
        let dir = tempfile::tempdir().expect("dir");
        let docs = vec![
            document(
                dir.path(),
                "a.pdf",
                &[(0, "variance collapse under smoothing")],
            ),
            document(
                dir.path(),
                "b.pdf",
                &[(0, "quaternion algebra and smoothing")],
            ),
            document(dir.path(), "c.pdf", &[(2, "the variance of the estimator")]),
        ];
        let index = build_gram_index(dir.path(), &docs);

        // Same results with the index as without it — the index decides what
        // is read, never what matches.
        for query in [
            "variance",
            "collapse",
            "quaternion",
            "smoothing",
            "estimator",
        ] {
            let (with, _) = search_library(dir.path(), &docs, Some(&index), query, false, 10, 10);
            let (without, _) = search_library(dir.path(), &docs, None, query, false, 10, 10);
            let names = |hits: &[LibraryHitDto]| {
                hits.iter().map(|hit| hit.name.clone()).collect::<Vec<_>>()
            };
            assert_eq!(names(&with), names(&without), "{query}");
        }

        // A phrase nothing contains costs no document reads at all.
        let (hits, _) = search_library(dir.path(), &docs, Some(&index), "zzzznope", false, 10, 10);
        assert!(hits.is_empty());
    }

    #[test]
    fn a_saved_index_is_reused_only_for_the_library_it_describes() {
        let dir = tempfile::tempdir().expect("dir");
        let docs = vec![document(dir.path(), "a.pdf", &[(0, "variance collapse")])];
        let signature = library_signature(&docs);
        save_gram_index(dir.path(), &build_gram_index(dir.path(), &docs));

        assert!(load_gram_index(dir.path(), signature).is_some());
        // A library that has changed since must not be searched through an
        // index describing the old one — it would rule out the new files.
        assert!(load_gram_index(dir.path(), signature ^ 1).is_none());
    }

    #[test]
    fn a_capitalized_query_still_finds_lowercase_text() {
        // The prefilters read folded text, so they have to be asked the folded
        // question however the search itself treats case.
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, "Variance Collapse")]);
        let docs = vec![doc];
        let index = build_gram_index(dir.path(), &docs);
        let (hits, _) = search_library(
            dir.path(),
            &docs,
            Some(&index),
            "Variance Collapse",
            true,
            10,
            10,
        );
        assert_eq!(
            hits.len(),
            1,
            "a case-sensitive match must survive the filters"
        );
    }

    #[test]
    fn an_empty_query_matches_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        let doc = document(dir.path(), "a.pdf", &[(0, "text")]);
        let (hits, _) = search_library(dir.path(), &[doc], None, "   ", false, 10, 10);
        assert!(hits.is_empty());
    }

    #[test]
    fn the_document_limit_reports_itself() {
        let dir = tempfile::tempdir().expect("dir");
        let docs: Vec<LibraryDocument> = (0..4)
            .map(|i| document(dir.path(), &format!("{i}.pdf"), &[(0, "shared term")]))
            .collect();
        let (hits, truncated) = search_library(dir.path(), &docs, None, "shared", false, 10, 2);
        assert_eq!(hits.len(), 2);
        assert!(truncated, "a capped result set must say so");
    }
}

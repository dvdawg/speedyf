//! Which documents could contain a phrase, without asking any of them.
//!
//! Per-document filters plateau because they are consulted *per document*: a
//! query reads something for every file, so the work grows with the library no
//! matter how sharp the filter is. Sharpening it makes that worse — a document
//! holds ~50k distinct 5-grams, and filtering those needs far more per document
//! than the 4 KB a trigram bitmap costs.
//!
//! Inverting the question fixes the complexity rather than the constant. Here a
//! gram names the documents that contain it, so a query reads only the posting
//! lists for its own grams — a few kilobytes — and never touches the rest of
//! the library. Work becomes proportional to the query.
//!
//! Two details make it small and honest:
//!
//! * **Common grams are dropped.** A gram in most of the library filters
//!   nothing and costs the most storage. On a real 1017-document corpus the 1%
//!   of grams above the threshold held a third of all postings.
//! * **Dropped is not the same as absent.** A gram that was dropped is still
//!   recorded, marked as carrying no information, because "in no document" and
//!   "in too many to care" must not look alike: the first means the answer is
//!   empty, the second means this gram cannot help.
//!
//! A document whose text could not be read is not silently absent: it is
//! recorded as one the index knows nothing about, and offered as a candidate
//! for every query. Leaving it out would make it unfindable forever.
//!
//! Grams are bytes, not characters. A multi-byte character is split the same
//! way when indexing and when querying, so matching is unaffected.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Bytes per gram. Four is short enough that a phrase yields several and long
/// enough to be selective — "olla" is rare where "col" is everywhere.
const GRAM: usize = 4;
/// A gram in more of the library than this is dropped: it cannot narrow
/// anything, and it is what makes the index large.
const COMMON_FRACTION: f64 = 0.15;
/// Marks a dropped gram in the directory.
const DROPPED: u32 = u32::MAX;

const MAGIC: u32 = 0x5350_4749; // "SPGI"
const VERSION: u32 = 1;

/// What the index can say about a query.
#[derive(Debug, PartialEq)]
pub enum Candidates {
    /// These documents, and no others, can contain the phrase.
    Only(Vec<u32>),
    /// The index cannot narrow this query — every gram in it was too common.
    /// The caller scans, which is the right answer anyway for a phrase that
    /// appears in most of the library.
    Unknown,
}

pub struct GramIndex {
    /// Documents by slot, in the order postings refer to them.
    documents: Vec<PathBuf>,
    /// (gram, start, len) sorted by gram; `len == DROPPED` marks a gram that
    /// was too common to keep.
    directory: Vec<(u32, u32, u32)>,
    postings: Vec<u32>,
    /// Slots with no indexed text, which therefore must be considered for
    /// every query — the index has nothing to say about them, and saying
    /// nothing must not read as saying no.
    always: Vec<u32>,
    /// Identifies the document set this was built from, so an index that no
    /// longer describes the library can be spotted rather than trusted.
    signature: u64,
}

fn grams_of(text: &[u8], mut visit: impl FnMut(u32)) {
    for window in text.windows(GRAM) {
        visit(u32::from_le_bytes([
            window[0], window[1], window[2], window[3],
        ]));
    }
}

/// A cheap identity for a set of documents: order-independent, so a rescan
/// that returns the same files in a different order does not look like a
/// change.
pub fn signature_of(documents: &[(PathBuf, u64, u64)]) -> u64 {
    let mut total: u64 = 0;
    for (path, mtime_ms, size) in documents {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in path.to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= mtime_ms.rotate_left(17) ^ size.rotate_left(31);
        total = total.wrapping_add(hash);
    }
    total
}

impl GramIndex {
    /// Build from every document's scan text. A document whose text is `None`
    /// could not be read, and becomes a permanent candidate rather than a
    /// document that quietly stops being findable.
    pub fn build(documents: Vec<(PathBuf, Option<Vec<u8>>)>, signature: u64) -> Self {
        let count = documents.len();
        let cutoff = ((count as f64) * COMMON_FRACTION).ceil().max(1.0) as usize;

        let mut postings_by_gram: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut paths = Vec::with_capacity(count);
        let mut always = Vec::new();
        for (slot, (path, text)) in documents.into_iter().enumerate() {
            match text {
                Some(text) => {
                    let mut seen = HashSet::new();
                    grams_of(&text, |gram| {
                        seen.insert(gram);
                    });
                    for gram in seen {
                        postings_by_gram.entry(gram).or_default().push(slot as u32);
                    }
                }
                None => always.push(slot as u32),
            }
            paths.push(path);
        }

        let mut directory = Vec::with_capacity(postings_by_gram.len());
        let mut postings = Vec::new();
        let mut grams: Vec<u32> = postings_by_gram.keys().copied().collect();
        grams.sort_unstable();
        for gram in grams {
            let mut docs = postings_by_gram.remove(&gram).unwrap_or_default();
            if docs.len() > cutoff {
                // Recorded, but with nothing to say. Forgetting it entirely
                // would make it indistinguishable from a gram no document has,
                // which means "no results".
                directory.push((gram, 0, DROPPED));
                continue;
            }
            docs.sort_unstable();
            let start = postings.len() as u32;
            let len = docs.len() as u32;
            postings.extend(docs);
            directory.push((gram, start, len));
        }

        GramIndex {
            documents: paths,
            directory,
            postings,
            always,
            signature,
        }
    }

    pub fn signature(&self) -> u64 {
        self.signature
    }

    pub fn document(&self, slot: u32) -> Option<&Path> {
        self.documents.get(slot as usize).map(PathBuf::as_path)
    }

    fn lookup(&self, gram: u32) -> Option<(u32, u32)> {
        let found = self
            .directory
            .binary_search_by_key(&gram, |(candidate, _, _)| *candidate)
            .ok()?;
        let (_, start, len) = self.directory[found];
        Some((start, len))
    }

    /// The documents that could contain `query`.
    pub fn candidates(&self, query: &str) -> Candidates {
        let bytes = query.as_bytes();
        if bytes.len() < GRAM {
            return Candidates::Unknown;
        }

        // Take the most selective grams first: intersecting a short list into
        // a long one costs the length of the short one.
        let mut usable: Vec<(u32, u32)> = Vec::new();
        let mut missing = false;
        grams_of(bytes, |gram| {
            if missing {
                return;
            }
            match self.lookup(gram) {
                // No document holds this gram, so none holds the phrase.
                None => missing = true,
                Some((_, DROPPED)) => {}
                Some((start, len)) => usable.push((start, len)),
            }
        });
        if missing {
            return Candidates::Only(self.always.clone());
        }
        if usable.is_empty() {
            return Candidates::Unknown;
        }

        usable.sort_by_key(|(_, len)| *len);
        let (start, len) = usable[0];
        let mut result: Vec<u32> = self.postings[start as usize..(start + len) as usize].to_vec();
        for (start, len) in usable.iter().skip(1) {
            if result.is_empty() {
                break;
            }
            let list = &self.postings[*start as usize..(*start + *len) as usize];
            result.retain(|slot| list.binary_search(slot).is_ok());
        }
        result.extend(self.always.iter().copied());
        result.sort_unstable();
        result.dedup();
        Candidates::Only(result)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(MAGIC.to_le_bytes());
        out.extend(VERSION.to_le_bytes());
        out.extend(self.signature.to_le_bytes());

        out.extend((self.documents.len() as u32).to_le_bytes());
        for path in &self.documents {
            let bytes = path.to_string_lossy();
            let bytes = bytes.as_bytes();
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(bytes);
        }

        out.extend((self.directory.len() as u32).to_le_bytes());
        for (gram, start, len) in &self.directory {
            out.extend(gram.to_le_bytes());
            out.extend(start.to_le_bytes());
            out.extend(len.to_le_bytes());
        }

        out.extend((self.postings.len() as u32).to_le_bytes());
        for slot in &self.postings {
            out.extend(slot.to_le_bytes());
        }

        out.extend((self.always.len() as u32).to_le_bytes());
        for slot in &self.always {
            out.extend(slot.to_le_bytes());
        }
        out
    }

    /// Parse an index, or nothing if it is not one this build understands.
    /// Every length is checked against what is actually there — the file is
    /// read back from disk and must not be trusted to be well formed.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut at = 0usize;
        let u32_at = |at: &mut usize| -> Option<u32> {
            let end = at.checked_add(4)?;
            let value = u32::from_le_bytes(bytes.get(*at..end)?.try_into().ok()?);
            *at = end;
            Some(value)
        };
        if u32_at(&mut at)? != MAGIC || u32_at(&mut at)? != VERSION {
            return None;
        }
        let signature = {
            let end = at.checked_add(8)?;
            let value = u64::from_le_bytes(bytes.get(at..end)?.try_into().ok()?);
            at = end;
            value
        };

        let document_count = u32_at(&mut at)? as usize;
        let mut documents = Vec::with_capacity(document_count.min(100_000));
        for _ in 0..document_count {
            let len = u32_at(&mut at)? as usize;
            let end = at.checked_add(len)?;
            let text = std::str::from_utf8(bytes.get(at..end)?).ok()?;
            documents.push(PathBuf::from(text));
            at = end;
        }

        let directory_count = u32_at(&mut at)? as usize;
        let mut directory = Vec::with_capacity(directory_count.min(4_000_000));
        for _ in 0..directory_count {
            let gram = u32_at(&mut at)?;
            let start = u32_at(&mut at)?;
            let len = u32_at(&mut at)?;
            directory.push((gram, start, len));
        }

        let postings_count = u32_at(&mut at)? as usize;
        let mut postings = Vec::with_capacity(postings_count.min(64_000_000));
        for _ in 0..postings_count {
            postings.push(u32_at(&mut at)?);
        }

        let always_count = u32_at(&mut at)? as usize;
        let mut always = Vec::with_capacity(always_count.min(1_000_000));
        for _ in 0..always_count {
            always.push(u32_at(&mut at)?);
        }

        // Every posting range must actually be inside the postings, or a
        // malformed file would panic on a slice later.
        let total = postings.len() as u64;
        let sound = directory
            .iter()
            .all(|(_, start, len)| *len == DROPPED || u64::from(*start) + u64::from(*len) <= total);
        if !sound {
            return None;
        }

        Some(GramIndex {
            documents,
            directory,
            postings,
            always,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus() -> Vec<(PathBuf, Option<Vec<u8>>)> {
        vec![
            (
                PathBuf::from("/a.pdf"),
                Some(b"variance collapse under smoothing".to_vec()),
            ),
            (
                PathBuf::from("/b.pdf"),
                Some(b"quaternion algebra and smoothing".to_vec()),
            ),
            (
                PathBuf::from("/c.pdf"),
                Some(b"variance of the estimator, smoothing".to_vec()),
            ),
        ]
    }

    #[test]
    fn narrows_to_the_documents_that_can_match() {
        let index = GramIndex::build(corpus(), 1);
        let Candidates::Only(slots) = index.candidates("collapse") else {
            panic!("expected a narrowed set");
        };
        assert_eq!(slots, vec![0]);
        assert_eq!(index.document(0).unwrap(), Path::new("/a.pdf"));
    }

    #[test]
    fn a_gram_no_document_has_means_no_results() {
        // The case worth being fast at: a phrase nothing contains should cost
        // a lookup, not a search.
        let index = GramIndex::build(corpus(), 1);
        assert_eq!(
            index.candidates("zzzznotinanypaper"),
            Candidates::Only(Vec::new())
        );
    }

    #[test]
    fn a_gram_in_everything_narrows_nothing_but_never_lies() {
        // "smoothing" is in all three, so its grams are dropped as common and
        // the index admits it cannot help — which must not be confused with
        // saying no document matches.
        let index = GramIndex::build(corpus(), 1);
        assert_eq!(index.candidates("smoothing"), Candidates::Unknown);
    }

    #[test]
    fn intersects_across_grams() {
        let index = GramIndex::build(corpus(), 1);
        let Candidates::Only(slots) = index.candidates("variance collapse") else {
            panic!("expected a narrowed set");
        };
        // Both documents contain "variance"; only one continues "collapse".
        assert_eq!(slots, vec![0]);
    }

    #[test]
    fn never_loses_a_document_that_does_contain_the_phrase() {
        // A false negative is a wrong answer. Every substring of every
        // document must survive the index.
        let index = GramIndex::build(corpus(), 1);
        for (slot, (_, text)) in corpus().into_iter().enumerate() {
            let text = String::from_utf8(text.expect("indexed")).unwrap();
            for len in [4, 7, 12] {
                if text.len() < len {
                    continue;
                }
                for start in 0..=text.len() - len {
                    let Some(phrase) = text.get(start..start + len) else {
                        continue;
                    };
                    match index.candidates(phrase) {
                        Candidates::Unknown => {}
                        Candidates::Only(slots) => assert!(
                            slots.contains(&(slot as u32)),
                            "{phrase:?} is in document {slot} but was ruled out"
                        ),
                    }
                }
            }
        }
    }

    #[test]
    fn a_query_shorter_than_a_gram_cannot_be_judged() {
        let index = GramIndex::build(corpus(), 1);
        assert_eq!(index.candidates("of"), Candidates::Unknown);
        assert_eq!(index.candidates(""), Candidates::Unknown);
    }

    #[test]
    fn survives_a_round_trip_through_bytes() {
        let index = GramIndex::build(corpus(), 42);
        let restored = GramIndex::from_bytes(&index.to_bytes()).expect("valid index");
        assert_eq!(restored.signature(), 42);
        assert_eq!(restored.candidates("collapse"), Candidates::Only(vec![0]));
        assert_eq!(restored.document(1).unwrap(), Path::new("/b.pdf"));
    }

    #[test]
    fn refuses_anything_that_is_not_an_index() {
        assert!(GramIndex::from_bytes(b"").is_none());
        assert!(GramIndex::from_bytes(b"not an index at all").is_none());
        // Truncation must be refused rather than read past.
        let index = GramIndex::build(corpus(), 1);
        let bytes = index.to_bytes();
        assert!(GramIndex::from_bytes(&bytes[..bytes.len() / 2]).is_none());
    }

    #[test]
    fn a_document_with_no_text_is_always_a_candidate() {
        // Never indexed, or its text could not be read. Excluding it would
        // make it permanently unfindable, which is worse than reading it.
        let mut documents = corpus();
        documents.push((PathBuf::from("/unread.pdf"), None));
        let index = GramIndex::build(documents, 1);

        let Candidates::Only(slots) = index.candidates("collapse") else {
            panic!("expected a narrowed set");
        };
        assert_eq!(slots, vec![0, 3]);

        // Even a phrase nothing indexed contains has to leave it in.
        assert_eq!(
            index.candidates("zzzznotinanypaper"),
            Candidates::Only(vec![3])
        );
    }

    #[test]
    fn the_signature_changes_when_the_documents_do() {
        let a = signature_of(&[(PathBuf::from("/a.pdf"), 1, 10)]);
        let b = signature_of(&[(PathBuf::from("/a.pdf"), 2, 10)]);
        let c = signature_of(&[(PathBuf::from("/b.pdf"), 1, 10)]);
        assert_ne!(a, b, "an edited file must invalidate the index");
        assert_ne!(a, c, "a different file must invalidate the index");
        // Order is not part of the identity: a rescan may return them anyhow.
        let one = signature_of(&[
            (PathBuf::from("/a.pdf"), 1, 10),
            (PathBuf::from("/b.pdf"), 2, 20),
        ]);
        let other = signature_of(&[
            (PathBuf::from("/b.pdf"), 2, 20),
            (PathBuf::from("/a.pdf"), 1, 10),
        ]);
        assert_eq!(one, other);
    }
}

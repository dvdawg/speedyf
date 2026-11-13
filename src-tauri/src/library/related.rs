//! Related items: the other library items the citation graph ties one to,
//! computed offline from what the library already holds (projects.md sec. 8).
//!
//! Two signals, each normalized with Salton's cosine so a hub does not
//! dominate:
//!
//! - bibliographic coupling, the works both items cite, over
//!   `sqrt(|works of a| * |works of b|)`;
//! - co-citation, the items citing both, over
//!   `sqrt(|citers of a| * |citers of b|)`.
//!
//! A work is named by the keys `suggest::works_by_paper` gives it: `p:<index>`
//! for a citation that resolves to a library item, else its DOI and arXiv
//! keys and its reference-text key. An item's works are first folded so one
//! work read twice (a DOI from the text and the same DOI in a reference
//! entry) counts once.
//!
//! The score is the sum of the two cosines. An item shows only with at least
//! [`MIN_SHARED`] shared works or [`MIN_CO_CITED`] co-citations, so one
//! coincidence never does. A direct citation either way is already in Cites
//! and Cited by: such an item still shows when it qualifies on its own, but
//! at [`DIRECT_WEIGHT`] of its score, so it does not crowd out what those
//! lists do not show. Never listed: the item itself (its merged records and
//! aliases resolve to it), an item sharing one of its identifiers, and an
//! item with a byte-identical file. Ties break by item id, so the order is
//! the same on every call.

use super::lookup::NetworkEdges;
use super::references::DocRefs;
use super::store::PaperStore;
use super::suggest::works_by_paper;
use super::types::RelatedItemDto;
use super::LibraryEntry;
use std::collections::{HashMap, HashSet};

/// The most related items one answer carries.
pub const MAX_RELATED: usize = 5;
/// Works both must cite before coupling alone lists an item.
pub const MIN_SHARED: u32 = 2;
/// Items that must cite both before co-citation alone lists an item.
pub const MIN_CO_CITED: u32 = 2;
/// What a direct citation either way leaves of an item's score.
pub const DIRECT_WEIGHT: f32 = 0.5;

/// One item's works, each a set of keys, a work read twice folded into one.
fn fold(works: &[Vec<String>]) -> Vec<Vec<&str>> {
    let mut folded: Vec<Vec<&str>> = Vec::new();
    for work in works {
        let keys: Vec<&str> = work.iter().map(String::as_str).collect();
        match folded
            .iter_mut()
            .find(|seen| keys.iter().any(|key| seen.contains(key)))
        {
            Some(seen) => {
                for key in keys {
                    if !seen.contains(&key) {
                        seen.push(key);
                    }
                }
            }
            None => folded.push(keys),
        }
    }
    folded
}

/// "shares 6 references, cited together by 3 items".
fn reason(shared: u32, co_cited: u32) -> String {
    let mut parts: Vec<String> = Vec::new();
    if shared > 0 {
        let noun = if shared == 1 {
            "reference"
        } else {
            "references"
        };
        parts.push(format!("shares {shared} {noun}"));
    }
    if co_cited > 0 {
        let noun = if co_cited == 1 { "item" } else { "items" };
        parts.push(format!("cited together by {co_cited} {noun}"));
    }
    parts.join(", ")
}

#[derive(Default)]
struct Tally {
    shared: u32,
    co_cited: u32,
}

impl PaperStore {
    /// The items most related to `paper_id` by the citation graph, at most
    /// [`MAX_RELATED`], strongest first (see the module notes for the score
    /// and the thresholds). An id that names no item answers empty. Built
    /// once per call from the same sources as `citation_links`, plus the
    /// network's _cited by_ edges, which make the citing item cite.
    pub fn related_items(
        &self,
        paper_id: &str,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> Vec<RelatedItemDto> {
        let Ok(own) = self.live_index(paper_id) else {
            return Vec::new();
        };
        let tables = self.citation_tables();
        let mut works = works_by_paper(self, &tables, entries, refs, network);
        // What the network says cites each item, as the citer's own work.
        for (index, paper) in self.papers.iter().enumerate() {
            for id in paper.citation_ids() {
                for citing in network.cites.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                    if let Some(&source) = tables.by_id.get(citing) {
                        if source != index {
                            works[source].push(vec![format!("p:{index}")]);
                        }
                    }
                }
            }
        }
        let folded: Vec<Vec<Vec<&str>>> = works.iter().map(|w| fold(w)).collect();

        // Key -> the items citing a work under it, each once, in index order.
        let mut holders: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, item) in folded.iter().enumerate() {
            for key in item.iter().flatten() {
                let list = holders.entry(key).or_default();
                if list.last() != Some(&index) {
                    list.push(index);
                }
            }
        }
        let citers = |index: usize| -> &[usize] {
            holders
                .get(format!("p:{index}").as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[])
        };

        let mut tally: HashMap<usize, Tally> = HashMap::new();
        // Coupling: each of the item's works once per other item citing it.
        for work in &folded[own] {
            let mut with: HashSet<usize> = HashSet::new();
            for key in work {
                with.extend(holders.get(key).into_iter().flatten().copied());
            }
            for other in with {
                tally.entry(other).or_default().shared += 1;
            }
        }
        // Co-citation: each item citing this one, once per other item it cites.
        let own_citers = citers(own);
        for &citer in own_citers {
            let cited: HashSet<usize> = folded[citer]
                .iter()
                .flatten()
                .filter_map(|key| key.strip_prefix("p:")?.parse::<usize>().ok())
                .filter(|&cited| cited != citer)
                .collect();
            for cited in cited {
                tally.entry(cited).or_default().co_cited += 1;
            }
        }

        let own_paper = &self.papers[own];
        let own_ids = own_paper.citation_ids();
        let own_prints: HashSet<&str> = own_paper
            .files
            .iter()
            .filter_map(|file| file.fingerprint.as_deref())
            .collect();
        let own_key = format!("p:{own}");
        let cites_key = |index: usize, key: &str| folded[index].iter().flatten().any(|k| *k == key);

        let mut out: Vec<(usize, f32, u32, u32)> = Vec::new();
        for (other, Tally { shared, co_cited }) in tally {
            if other == own || (shared < MIN_SHARED && co_cited < MIN_CO_CITED) {
                continue;
            }
            let paper = &self.papers[other];
            let same_work = paper.citation_ids().iter().any(|id| own_ids.contains(id))
                || paper
                    .files
                    .iter()
                    .filter_map(|file| file.fingerprint.as_deref())
                    .any(|print| own_prints.contains(print));
            if same_work {
                continue;
            }
            let cosine = |n: u32, a: usize, b: usize| {
                if n == 0 || a == 0 || b == 0 {
                    0.0
                } else {
                    n as f32 / ((a as f32) * (b as f32)).sqrt()
                }
            };
            let mut score = cosine(shared, folded[own].len(), folded[other].len())
                + cosine(co_cited, own_citers.len(), citers(other).len());
            let direct = cites_key(own, &format!("p:{other}")) || cites_key(other, &own_key);
            if direct {
                score *= DIRECT_WEIGHT;
            }
            out.push((other, score, shared, co_cited));
        }
        out.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.papers[a.0].id.cmp(&self.papers[b.0].id))
        });
        out.truncate(MAX_RELATED);
        out.into_iter()
            .filter_map(|(index, score, shared, co_cited)| {
                let row = self.rows_in_list_order(vec![&self.papers[index]]).pop()?;
                Some(RelatedItemDto {
                    row,
                    score,
                    shared_refs: shared,
                    co_cited,
                    reason: reason(shared, co_cited),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::CitationIdDto::{self, Doi};
    use crate::library::papers::PaperRecord;
    use crate::library::tests::indexed_entry;
    use crate::library::types::{MetaField, MetaSource, PaperFile};
    use std::path::Path;

    fn store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        std::mem::forget(dir);
        store
    }

    /// A paper with one file at `/l/<name>.pdf` and DOI `10.1/<name>`.
    fn add(store: &mut PaperStore, name: &str) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = Some(MetaField {
            value: format!("Library paper {name} on some topic"),
            source: MetaSource::Crossref,
        });
        paper.ids.doi = Some(format!("10.1/{name}"));
        paper.files.push(PaperFile {
            path: format!("/l/{name}.pdf"),
            name: format!("{name}.pdf"),
            size: 1,
            mtime_ms: 1,
            fingerprint: Some(name.to_owned()),
            missing: false,
            ..Default::default()
        });
        paper.cite_key = name.to_owned();
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    fn entry(name: &str, cites: &[&str]) -> LibraryEntry {
        let mut entry = indexed_entry(Path::new(&format!("/l/{name}.pdf")), 1, 1);
        entry.fingerprint = Some(name.to_owned());
        entry.cites = Some(
            cites
                .iter()
                .map(|doi| Doi((*doi).to_owned()))
                .collect::<Vec<CitationIdDto>>(),
        );
        entry
    }

    fn related(store: &PaperStore, id: &str, entries: &[LibraryEntry]) -> Vec<RelatedItemDto> {
        store.related_items(id, entries, &DocRefs::new(), &NetworkEdges::default())
    }

    fn ids(got: &[RelatedItemDto]) -> Vec<&str> {
        got.iter().map(|r| r.row.id.as_str()).collect()
    }

    #[test]
    fn coupling_lists_items_citing_the_same_works() {
        let mut store = store();
        let t = add(&mut store, "t");
        let a = add(&mut store, "a");
        let b = add(&mut store, "b");
        let lib = add(&mut store, "lib");
        let entries = vec![
            // Two works outside the library and one item in it.
            entry("t", &["10.9/r1", "10.9/r2", "10.1/lib"]),
            entry("a", &["10.9/r1", "10.1/lib", "10.9/x"]),
            // One shared work: below the threshold.
            entry("b", &["10.9/r2"]),
        ];
        let got = related(&store, &t, &entries);
        assert_eq!(ids(&got), [a.as_str()]);
        assert_eq!((got[0].shared_refs, got[0].co_cited), (2, 0));
        assert_eq!(got[0].reason, "shares 2 references");
        // 2 / sqrt(3 * 3)
        assert!((got[0].score - 2.0 / 3.0).abs() < 1e-4);
        assert!(!ids(&got).contains(&b.as_str()));
        assert!(!ids(&got).contains(&lib.as_str()), "cited, not related");
        assert!(related(&store, "nope", &entries).is_empty());
    }

    #[test]
    fn co_citation_lists_items_cited_together() {
        let mut store = store();
        let t = add(&mut store, "t");
        let a = add(&mut store, "a");
        let b = add(&mut store, "b");
        add(&mut store, "c1");
        add(&mut store, "c2");
        add(&mut store, "c3");
        let entries = vec![
            entry("c1", &["10.1/t", "10.1/a", "10.1/b"]),
            entry("c2", &["10.1/t", "10.1/a"]),
            entry("c3", &["10.1/a"]),
        ];
        let got = related(&store, &t, &entries);
        // a: cited with t by c1 and c2; b: by c1 only, below the threshold.
        assert_eq!(ids(&got), [a.as_str()]);
        assert_eq!((got[0].shared_refs, got[0].co_cited), (0, 2));
        assert_eq!(got[0].reason, "cited together by 2 items");
        // 2 / sqrt(2 * 3)
        assert!((got[0].score - 2.0 / 6f32.sqrt()).abs() < 1e-4);
        assert!(!ids(&got).contains(&b.as_str()));
    }

    #[test]
    fn a_hub_with_a_long_bibliography_does_not_dominate() {
        let mut store = store();
        let t = add(&mut store, "t");
        let hub = add(&mut store, "hub");
        let small = add(&mut store, "small");
        let mut hub_cites: Vec<String> = (1..=4).map(|i| format!("10.9/r{i}")).collect();
        hub_cites.extend((0..200).map(|i| format!("10.8/other{i}")));
        let hub_refs: Vec<&str> = hub_cites.iter().map(String::as_str).collect();
        let entries = vec![
            entry("t", &["10.9/r1", "10.9/r2", "10.9/r3", "10.9/r4"]),
            entry("hub", &hub_refs),
            entry("small", &["10.9/r1", "10.9/r2", "10.9/y"]),
        ];
        let got = related(&store, &t, &entries);
        // The hub shares four works, the small item two, out of three.
        assert_eq!(ids(&got), [small.as_str(), hub.as_str()]);
        assert_eq!(got[1].shared_refs, 4);
        assert!(got[0].score > 2.0 * got[1].score);
    }

    #[test]
    fn direct_citations_rank_below_and_never_alone() {
        let mut store = store();
        let t = add(&mut store, "t");
        let cited = add(&mut store, "cited");
        let peer = add(&mut store, "peer");
        let only = add(&mut store, "only");
        let entries = vec![
            entry("t", &["10.9/r1", "10.9/r2", "10.1/cited", "10.1/only"]),
            // Cited by t and coupled to it as strongly as peer.
            entry("cited", &["10.9/r1", "10.9/r2"]),
            entry("peer", &["10.9/r1", "10.9/r2"]),
            entry("only", &[]),
        ];
        let got = related(&store, &t, &entries);
        assert_eq!(ids(&got), [peer.as_str(), cited.as_str()]);
        assert!((got[1].score - DIRECT_WEIGHT * got[0].score).abs() < 1e-4);
        assert!(!ids(&got).contains(&only.as_str()));
    }

    #[test]
    fn the_item_its_duplicates_and_its_aliases_are_never_listed() {
        let mut store = store();
        let t = add(&mut store, "t");
        // Same bytes as t, under another record.
        let copy = add(&mut store, "copy");
        store.papers[1].files[0].fingerprint = Some("t".into());
        // Another record under t's DOI.
        let twin = add(&mut store, "twin");
        store.papers[2].ids.doi = Some("10.1/t".into());
        let peer = add(&mut store, "peer");
        // An alias of t's: a citation of it is a citation of t.
        store.papers[0]
            .alias_ids
            .push(CitationIdDto::ArXiv("2601.00001".into()));
        let mut c = entry("c", &[]);
        c.cites = Some(vec![
            CitationIdDto::ArXiv("2601.00001".into()),
            Doi("10.1/t".into()),
        ]);
        add(&mut store, "c");
        let entries = vec![
            entry("t", &["10.9/r1", "10.9/r2"]),
            entry("copy", &["10.9/r1", "10.9/r2"]),
            entry("twin", &["10.9/r1", "10.9/r2"]),
            entry("peer", &["10.9/r1", "10.9/r2"]),
            c,
        ];
        let got = related(&store, &t, &entries);
        assert_eq!(ids(&got), [peer.as_str()]);
        assert!(!ids(&got).contains(&copy.as_str()));
        assert!(!ids(&got).contains(&twin.as_str()));
    }

    #[test]
    fn the_order_is_deterministic_and_capped() {
        let mut store = store();
        let t = add(&mut store, "t");
        let peers: Vec<String> = (0..8).map(|i| add(&mut store, &format!("p{i}"))).collect();
        let mut entries = vec![entry("t", &["10.9/r1", "10.9/r2"])];
        for i in 0..8 {
            entries.push(entry(&format!("p{i}"), &["10.9/r1", "10.9/r2"]));
        }
        let first = related(&store, &t, &entries);
        assert_eq!(first.len(), MAX_RELATED);
        let mut sorted = peers.clone();
        sorted.sort();
        assert_eq!(
            ids(&first),
            sorted[..MAX_RELATED]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        entries[1..].reverse();
        for _ in 0..5 {
            assert_eq!(related(&store, &t, &entries), first);
        }
    }

    #[test]
    fn the_reason_is_plain_words() {
        assert_eq!(
            reason(6, 3),
            "shares 6 references, cited together by 3 items"
        );
        assert_eq!(reason(0, 1), "cited together by 1 item");
        assert!(reason(6, 3).is_ascii());
    }
}

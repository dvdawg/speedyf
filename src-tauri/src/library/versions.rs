//! Version spotting (projects.md sec. 2, "Merge suggestions"): a preprint
//! and its journal version cite nearly the same works, so two items whose
//! reference lists overlap almost entirely are probably one work.
//!
//! Each reference is named by keys: the library item it resolves to, the
//! identifiers it prints, its title as `witness` reads it, its volume and
//! first page, and its first author and year when it prints no title.
//! Two references are the same work when they share a key. An inverted
//! index from key to items gives every item's overlap with every other in
//! one pass over the references, never comparing all pairs.
//!
//! A pair is spotted when both lists have at least [`MIN_REFS`] entries,
//! the Jaccard index of the two lists is at least [`MIN_JACCARD`], and one
//! weak signal agrees: similar titles, the same first-author family, or
//! known years at most [`YEAR_SLACK`] apart. Surveys of one field share
//! perhaps a third of their references, and a follow-up by the same
//! authors about half: Jaccard 0.2 and 0.33, well short. Spotted pairs are
//! offered as merge suggestions ("nearly the same 48 references"), never
//! merged on this signal alone.

use super::consensus::family_key;
use super::missing::{id_key, text_key, MIN_KEY_CHARS, PREFIX_CHARS};
use super::papers::{CitationTables, PaperRecord};
use super::references::{DocRefs, StoredRef};
use super::store::PaperStore;
use super::{titlematch, witness, LibraryEntry};
use std::collections::HashMap;
use std::sync::Arc;

/// Lists shorter than this say too little to call two papers one.
pub const MIN_REFS: usize = 12;
/// Shared references over all references, at least.
pub const MIN_JACCARD: f64 = 0.7;
/// Title similarity (`titlematch::similarity`) that corroborates.
pub const MIN_TITLE_SIMILARITY: f32 = 0.5;
/// Known years this close corroborate.
pub const YEAR_SLACK: i32 = 2;

/// Two records (store indices, `a < b`) and how many references they
/// share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionPair {
    pub a: usize,
    pub b: usize,
    pub shared: usize,
}

/// The keys one reference of `holder` is known by. Empty for a citation
/// of `holder` itself, and for an entry too thin to name anything.
pub(super) fn reference_keys(
    reference: &StoredRef,
    holder: usize,
    tables: &CitationTables<'_>,
) -> Vec<String> {
    let mut keys = Vec::new();
    match tables.resolve_reference(reference, holder) {
        Some(target) if target == holder => return keys,
        Some(target) => keys.push(format!("p:{target}")),
        None => {}
    }
    keys.extend(reference.ids.iter().map(id_key));
    let read = witness::parse(&reference.text, reference.year);
    let title = read
        .title
        .as_deref()
        .map(|t| titlematch::normalize(t).replace(' ', ""))
        .filter(|t| t.len() >= 16);
    if let Some(title) = &title {
        keys.push(format!("t:{title}"));
    }
    if let (Some(volume), Some(pages)) = (&read.volume, &read.pages) {
        let page = pages
            .split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        keys.push(format!("v:{volume}:{page}"));
    }
    if title.is_none() {
        if let (Some(first), Some(year)) = (read.authors.first(), read.year) {
            let family = family_key(&first.family);
            if !family.is_empty() {
                keys.push(format!("a:{family}:{year}"));
            }
        }
    }
    if keys.is_empty() {
        let text: String = text_key(reference).chars().take(PREFIX_CHARS).collect();
        if text.len() >= MIN_KEY_CHARS {
            keys.push(format!("x:{text}"));
        }
    }
    keys
}

/// Whether something besides the references says the two are one work.
fn corroborated(a: &PaperRecord, b: &PaperRecord) -> bool {
    if let (Some(x), Some(y)) = (a.title(), b.title()) {
        if titlematch::similarity(x, y) >= MIN_TITLE_SIMILARITY {
            return true;
        }
    }
    if let (Some(x), Some(y)) = (a.authors().first(), b.authors().first()) {
        let (x, y) = (family_key(&x.family), family_key(&y.family));
        if !x.is_empty() && x == y {
            return true;
        }
    }
    matches!((a.year(), b.year()), (Some(x), Some(y)) if (x - y).abs() <= YEAR_SLACK)
}

/// Every version pair among the records with reference lists (`lists`,
/// indexed like `store.papers`), in index order.
pub(super) fn spot(
    store: &PaperStore,
    tables: &CitationTables<'_>,
    lists: &[Option<&Arc<Vec<StoredRef>>>],
) -> Vec<VersionPair> {
    // (record, its references' keys), long enough lists only.
    let items: Vec<(usize, Vec<Vec<String>>)> = lists
        .iter()
        .enumerate()
        .filter_map(|(holder, list)| {
            let list = (*list)?;
            if list.len() < MIN_REFS {
                return None;
            }
            let keyed: Vec<Vec<String>> = list
                .iter()
                .map(|reference| reference_keys(reference, holder, tables))
                .filter(|keys| !keys.is_empty())
                .collect();
            (keyed.len() >= MIN_REFS).then_some((holder, keyed))
        })
        .collect();
    let mut index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (at, (_, keyed)) in items.iter().enumerate() {
        for keys in keyed {
            for key in keys {
                let posting = index.entry(key.as_str()).or_default();
                if posting.last() != Some(&at) {
                    posting.push(at);
                }
            }
        }
    }
    let mut pairs = Vec::new();
    for (at, (holder, keyed)) in items.iter().enumerate() {
        // For each later item, how many of this one's references it has.
        let mut shared: HashMap<usize, usize> = HashMap::new();
        let mut seen: Vec<usize> = Vec::new();
        for keys in keyed {
            seen.clear();
            for key in keys {
                for &other in index.get(key.as_str()).map(Vec::as_slice).unwrap_or(&[]) {
                    if other > at && !seen.contains(&other) {
                        seen.push(other);
                        *shared.entry(other).or_default() += 1;
                    }
                }
            }
        }
        let mut found: Vec<(usize, usize)> = shared.into_iter().collect();
        found.sort_unstable();
        for (other, count) in found {
            let (other_holder, other_keyed) = &items[other];
            let count = count.min(other_keyed.len());
            let union = keyed.len() + other_keyed.len() - count;
            if (count as f64) < MIN_JACCARD * union as f64 {
                continue;
            }
            if !corroborated(&store.papers[*holder], &store.papers[*other_holder]) {
                continue;
            }
            pairs.push(VersionPair {
                a: (*holder).min(*other_holder),
                b: (*holder).max(*other_holder),
                shared: count,
            });
        }
    }
    pairs.sort_by_key(|pair| (pair.a, pair.b));
    pairs
}

impl PaperStore {
    /// The version pairs of the library, as (id, id, shared references),
    /// for the duplicate suggestions.
    pub fn version_pairs(
        &self,
        entries: &[LibraryEntry],
        refs: &DocRefs,
    ) -> Vec<(String, String, usize)> {
        let tables = self.citation_tables();
        let lists = super::consensus::lists_by_record(self, &tables, entries, refs);
        spot(self, &tables, &lists)
            .into_iter()
            .map(|pair| {
                (
                    self.papers[pair.a].id.clone(),
                    self.papers[pair.b].id.clone(),
                    pair.shared,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::consensus::fixtures::{entry, id_of, refs, store, Listed};
    use super::*;
    use std::collections::HashMap;

    /// The `n`th work of a made-up field, printed in APS style with a
    /// title, as every list here prints it.
    fn work(n: usize) -> (String, Option<i32>) {
        const WORDS: [&str; 10] = [
            "entropy",
            "transport",
            "geometry",
            "response",
            "control",
            "fluctuations",
            "engines",
            "information",
            "dissipation",
            "noise",
        ];
        let year = 1990 + (n % 30) as i32;
        (
            format!(
                "A. Author{n} and B. Coauthor, On the {} of {} in model {n}, Phys. Rev. E {}, {:06} ({year}).",
                WORDS[n % 10],
                WORDS[(n / 10) % 10],
                40 + n % 60,
                1000 + n
            ),
            Some(year),
        )
    }

    /// (path, title, first-page authors, reference list).
    type Item<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)], Listed);

    fn list(works: impl IntoIterator<Item = usize>) -> Listed {
        works.into_iter().map(work).collect()
    }

    fn spotted(library: &[Item<'_>]) -> (PaperStore, Vec<(String, String, usize)>) {
        let mut store = store();
        let entries: Vec<LibraryEntry> = library
            .iter()
            .map(|(path, title, authors, _)| entry(path, title, authors))
            .collect();
        for e in &entries {
            store.reconcile_file(e);
        }
        let lists: Vec<(&str, Listed)> = library
            .iter()
            .map(|(path, _, _, list)| (*path, list.clone()))
            .collect();
        let pairs = store.version_pairs(&entries, &refs(&lists));
        (store, pairs)
    }

    const SMITH: &[(&str, &str)] = &[("J.", "Smith"), ("A.", "Jones")];
    const OTHERS: &[(&str, &str)] = &[("K.", "Brown")];

    #[test]
    fn a_preprint_and_its_journal_version_are_spotted() {
        let (store, pairs) = spotted(&[
            (
                "/l/preprint.pdf",
                "Fast optimal protocols for small engines",
                SMITH,
                list(0..48),
            ),
            (
                "/l/journal.pdf",
                "Fast optimal protocols for small stochastic engines",
                SMITH,
                list((0..48).chain(48..50)),
            ),
        ]);
        let (p, j) = (
            id_of(&store, "/l/preprint.pdf"),
            id_of(&store, "/l/journal.pdf"),
        );
        assert_eq!(pairs.len(), 1, "{pairs:?}");
        let (a, b, shared) = &pairs[0];
        assert_eq!(
            [a, b]
                .map(String::as_str)
                .iter()
                .filter(|x| **x == p || **x == j)
                .count(),
            2
        );
        assert_eq!(*shared, 48);
        // Through the duplicate suggestions, with its reason.
        let groups = store.duplicates_with_versions(&HashMap::new(), &pairs);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].reason, "nearly the same 48 references");
        // Never merged on its own.
        assert_eq!(store.papers.len(), 2);
    }

    #[test]
    fn surveys_of_one_field_and_a_follow_up_are_not() {
        // Two surveys sharing 30% of their references.
        let (_, pairs) = spotted(&[
            (
                "/l/s1.pdf",
                "A survey of stochastic thermodynamics",
                SMITH,
                list(0..40),
            ),
            (
                "/l/s2.pdf",
                "A review of stochastic thermodynamics",
                SMITH,
                list((28..40).chain(100..128)),
            ),
        ]);
        assert!(pairs.is_empty(), "{pairs:?}");
        // A follow-up by the same authors sharing half its references.
        let (_, pairs) = spotted(&[
            (
                "/l/base.pdf",
                "Optimal protocols for small engines",
                SMITH,
                list(0..40),
            ),
            (
                "/l/next.pdf",
                "Optimal protocols for small engines II",
                SMITH,
                list((20..40).chain(200..220)),
            ),
        ]);
        assert!(pairs.is_empty(), "{pairs:?}");
    }

    #[test]
    fn overlap_alone_needs_a_corroborator_and_enough_references() {
        // The same list, nothing else in common: different titles, first
        // authors, and years unknown.
        let (_, pairs) = spotted(&[
            (
                "/l/a.pdf",
                "Fast optimal protocols for small engines",
                SMITH,
                list(0..30),
            ),
            (
                "/l/b.pdf",
                "Quantum error correction with surface codes",
                OTHERS,
                list(0..30),
            ),
        ]);
        assert!(pairs.is_empty(), "{pairs:?}");
        // Identical but short lists.
        let (_, pairs) = spotted(&[
            (
                "/l/a.pdf",
                "Fast optimal protocols for small engines",
                SMITH,
                list(0..11),
            ),
            (
                "/l/b.pdf",
                "Fast optimal protocols for small engines v2",
                SMITH,
                list(0..11),
            ),
        ]);
        assert!(pairs.is_empty(), "{pairs:?}");
    }

    #[test]
    fn a_reference_matches_by_any_of_its_keys() {
        // The journal version prints DOIs where the preprint printed
        // titles; the volume and page still name the same works.
        let journal: Vec<(String, Option<i32>)> = (0..20)
            .map(|n| {
                (
                    format!(
                        "A. Author{n} and B. Coauthor, Phys. Rev. E {}, {:06} ({}).",
                        40 + n % 60,
                        1000 + n,
                        1990 + n % 30
                    ),
                    Some(1990 + n % 30),
                )
            })
            .collect();
        let (_, pairs) = spotted(&[
            (
                "/l/pre.pdf",
                "Fast optimal protocols for small engines",
                SMITH,
                list(0..20),
            ),
            (
                "/l/pub.pdf",
                "Fast optimal protocols for small engines",
                SMITH,
                journal,
            ),
        ]);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].2, 20);
    }

    /// The inverted index finds exactly what comparing every pair finds.
    #[test]
    fn the_index_agrees_with_comparing_every_pair() {
        let mut library = Vec::new();
        let mut seed = 7u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let paths: Vec<String> = (0..9).map(|i| format!("/l/p{i}.pdf")).collect();
        let titles: Vec<String> = (0..9)
            .map(|i| format!("Optimal protocols for small engines part {i}"))
            .collect();
        let base: Vec<usize> = (0..40).collect();
        for i in 0..9 {
            // Each list keeps a random share of a common base and adds
            // its own works.
            let keep = 28 + (next() % 13) as usize;
            let mut works: Vec<usize> = base
                .iter()
                .copied()
                .filter(|_| (next() % 40) < keep as u64)
                .collect();
            works.extend((0..(next() % 5) as usize).map(|k| 1000 + i * 100 + k));
            library.push((paths[i].as_str(), titles[i].as_str(), SMITH, list(works)));
        }
        let (store, pairs) = spotted(&library);
        // Every pair, the slow way.
        let tables = store.citation_tables();
        let lists = refs(
            &library
                .iter()
                .map(|(p, _, _, l)| (*p, l.clone()))
                .collect::<Vec<_>>(),
        );
        let keyed: Vec<(String, Vec<Vec<String>>)> = paths
            .iter()
            .map(|path| {
                let holder = store
                    .papers
                    .iter()
                    .position(|p| p.files[0].path == *path)
                    .expect("holder");
                let keys = lists[&std::path::PathBuf::from(path)]
                    .iter()
                    .map(|r| reference_keys(r, holder, &tables))
                    .filter(|k| !k.is_empty())
                    .collect();
                (store.papers[holder].id.clone(), keys)
            })
            .collect();
        let mut slow = Vec::new();
        for a in 0..keyed.len() {
            for b in a + 1..keyed.len() {
                let (ka, kb) = (&keyed[a].1, &keyed[b].1);
                if ka.len() < MIN_REFS || kb.len() < MIN_REFS {
                    continue;
                }
                let shared = ka
                    .iter()
                    .filter(|keys| {
                        kb.iter()
                            .any(|other| keys.iter().any(|k| other.contains(k)))
                    })
                    .count();
                let union = ka.len() + kb.len() - shared;
                if shared as f64 >= MIN_JACCARD * union as f64 {
                    let mut ids = [keyed[a].0.clone(), keyed[b].0.clone()];
                    ids.sort();
                    slow.push((ids, shared));
                }
            }
        }
        let mut fast: Vec<([String; 2], usize)> = pairs
            .into_iter()
            .map(|(a, b, shared)| {
                let mut ids = [a, b];
                ids.sort();
                (ids, shared)
            })
            .collect();
        slow.sort();
        fast.sort();
        assert!(!slow.is_empty(), "the fixture should hold some pairs");
        assert!(slow.len() < 36, "and some that are not");
        assert_eq!(fast, slow);
    }
}

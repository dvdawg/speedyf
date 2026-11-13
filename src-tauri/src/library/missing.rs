//! Works the view's items cite that are not in the library (projects.md sec. 8):
//! the ghost nodes of the graph, and a reading list.
//!
//! Per citing item, from the same sources the graph reads: the identifiers
//! its text cites (`cites`), each reference entry of its list (the active
//! provider's refs sidecar), and the network's references for its own
//! identifiers. A citation is missing when it resolves to no record, by
//! identifier (aliases included) or, for a reference entry, by a library
//! title in it; an identifier printed in an entry that resolved by title is
//! not missing either.
//!
//! **Grouping** across citing items, conservative - two printings of one
//! work left apart is fine, two works put together is not:
//!
//! - by identifier: citations sharing a DOI or an arXiv id (lower-case) are
//!   one work, and an entry printing several identifiers joins them;
//! - by text, for reference entries: each entry's key is its compact text
//!   with its year removed; two entries are one work when their keys are
//!   equal, or when one key is at least [`PREFIX_CHARS`] long and its first
//!   [`PREFIX_CHARS`] characters occur in the other's key. Text never joins
//!   two groups that both carry identifiers (they would be distinct works
//!   by identifier), so an entry without an identifier joins the group of
//!   one printing the same text with an identifier, and that is all. An
//!   entry without an identifier whose key is shorter than
//!   [`MIN_KEY_CHARS`] is too little to name a work and is dropped.

use super::lookup::NetworkEdges;
use super::papers::PaperRecord;
use super::references::{compact, DocRefs, StoredRef};
use super::store::PaperStore;
use super::types::MissingWorkDto;
use super::{identifiers, LibraryEntry};
use crate::engine::types::CitationIdDto;
use crate::errors::AppResult;
use std::collections::{HashMap, HashSet};

/// The length of the reference-text prefix two entries are matched on.
pub const PREFIX_CHARS: usize = 40;
/// Shorter keys ("ibid", "ref 12") name nothing.
pub const MIN_KEY_CHARS: usize = 24;
/// The most works one answer lists.
pub const MAX_MISSING: usize = 200;

/// One citation of something not in the library.
struct Cited<'a> {
    /// Position of the citing item in the view.
    node: usize,
    ids: Vec<CitationIdDto>,
    entry: Option<&'a StoredRef>,
}

pub(super) fn id_key(id: &CitationIdDto) -> String {
    match id {
        CitationIdDto::Doi(doi) => format!("doi:{}", doi.to_ascii_lowercase()),
        CitationIdDto::ArXiv(arxiv) => format!("arxiv:{}", arxiv.to_ascii_lowercase()),
    }
}

/// An entry's text key: its compact text with its year taken out.
pub(super) fn text_key(entry: &StoredRef) -> String {
    match entry.year {
        Some(year) => entry.compact.replace(&year.to_string(), ""),
        None => entry.compact.clone(),
    }
}

struct Sets {
    parent: Vec<usize>,
    has_ids: Vec<bool>,
}

impl Sets {
    fn find(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }

    /// Join two sets; `by_text` refuses when both carry identifiers.
    fn join(&mut self, a: usize, b: usize, by_text: bool) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb || (by_text && self.has_ids[ra] && self.has_ids[rb]) {
            return;
        }
        let (keep, gone) = (ra.min(rb), ra.max(rb));
        self.parent[gone] = keep;
        self.has_ids[keep] |= self.has_ids[gone];
    }
}

impl PaperStore {
    /// Works cited by the view's items and absent from the library, cited
    /// by at least `min_cited_by` of them (see the module doc). The view is
    /// the focus paper's neighbourhood when `focus` is set (`depth` steps,
    /// as `focus_graph`), else the project's members, else every item.
    /// Sorted by how many items cite them, then by year, newest first; at
    /// most [`MAX_MISSING`].
    #[allow(clippy::too_many_arguments)]
    pub fn missing_works(
        &self,
        project_id: Option<&str>,
        focus: Option<&str>,
        depth: u32,
        min_cited_by: u32,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> AppResult<Vec<MissingWorkDto>> {
        let view: Vec<&PaperRecord> = match (focus, project_id) {
            (Some(focus), _) => self.focus_nodes(focus, depth, entries, refs, network)?,
            (None, Some(project)) => self.project_members(project)?,
            (None, None) => {
                self.in_list_order(self.papers.iter().filter(|p| p.is_paper()).collect())
            }
        };
        let mut works = self.missing_over(view, min_cited_by, entries, refs, network);
        works.truncate(MAX_MISSING);
        Ok(works)
    }

    /// `missing_works` over a view the caller chose, every group kept:
    /// the co-citation suggestions (`cocitation.rs`) read the whole
    /// library's missing works through here, so their keys are the keys
    /// this list uses.
    pub(super) fn missing_over(
        &self,
        view: Vec<&PaperRecord>,
        min_cited_by: u32,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> Vec<MissingWorkDto> {
        let tables = self.citation_tables();
        let paper_index: HashMap<&str, usize> = self
            .papers
            .iter()
            .enumerate()
            .map(|(index, paper)| (paper.id.as_str(), index))
            .collect();
        let position: HashMap<usize, usize> = view
            .iter()
            .enumerate()
            .map(|(at, paper)| (paper_index[paper.id.as_str()], at))
            .collect();

        let mut cited: Vec<Cited> = Vec::new();
        for entry in entries {
            let Some(cites) = entry.cites.as_ref() else {
                continue;
            };
            let path = entry.path.to_string_lossy();
            let Some(&holder) = tables.by_path.get(path.as_ref()) else {
                continue;
            };
            let Some(&node) = position.get(&holder) else {
                continue;
            };
            let mut resolved: HashSet<&CitationIdDto> = HashSet::new();
            for reference in refs.get(&entry.path).map(|l| l.as_slice()).unwrap_or(&[]) {
                match tables.resolve_reference(reference, holder) {
                    Some(_) => resolved.extend(reference.ids.iter()),
                    None => cited.push(Cited {
                        node,
                        ids: reference.ids.clone(),
                        entry: Some(reference),
                    }),
                }
            }
            for id in cites {
                if !tables.by_id.contains_key(id) && !resolved.contains(id) {
                    cited.push(Cited {
                        node,
                        ids: vec![id.clone()],
                        entry: None,
                    });
                }
            }
        }
        for (node, paper) in view.iter().enumerate() {
            for id in paper.citation_ids() {
                for reference in network.refs.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                    if !tables.by_id.contains_key(reference) {
                        cited.push(Cited {
                            node,
                            ids: vec![reference.clone()],
                            entry: None,
                        });
                    }
                }
            }
        }
        // Too little to name a work.
        let keys: Vec<Option<String>> = cited
            .iter()
            .map(|c| c.entry.map(text_key).filter(|k| k.len() >= MIN_KEY_CHARS))
            .collect();
        let usable: Vec<bool> = cited
            .iter()
            .zip(&keys)
            .map(|(c, key)| !c.ids.is_empty() || key.is_some())
            .collect();

        let mut sets = Sets {
            parent: (0..cited.len()).collect(),
            has_ids: cited.iter().map(|c| !c.ids.is_empty()).collect(),
        };
        let mut by_id: HashMap<String, usize> = HashMap::new();
        for (at, c) in cited.iter().enumerate() {
            for id in &c.ids {
                match by_id.get(&id_key(id)) {
                    Some(&first) => sets.join(first, at, false),
                    None => {
                        by_id.insert(id_key(id), at);
                    }
                }
            }
        }
        let mut whole: HashMap<&str, usize> = HashMap::new();
        let mut prefixes: HashMap<&str, Vec<usize>> = HashMap::new();
        for (at, key) in keys.iter().enumerate() {
            let Some(key) = key else { continue };
            match whole.get(key.as_str()) {
                Some(&first) => sets.join(first, at, true),
                None => {
                    whole.insert(key, at);
                }
            }
            if key.len() >= PREFIX_CHARS {
                prefixes.entry(&key[..PREFIX_CHARS]).or_default().push(at);
            }
        }
        for (at, key) in keys.iter().enumerate() {
            let Some(key) = key else { continue };
            if key.len() < PREFIX_CHARS {
                continue;
            }
            for start in 0..=key.len() - PREFIX_CHARS {
                if let Some(others) = prefixes.get(&key[start..start + PREFIX_CHARS]) {
                    for &other in others {
                        if other != at {
                            sets.join(other, at, true);
                        }
                    }
                }
            }
        }

        // Gather each group, in citation order.
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut group_of: HashMap<usize, usize> = HashMap::new();
        for (at, usable) in usable.iter().enumerate() {
            if !usable {
                continue;
            }
            let root = sets.find(at);
            let group = *group_of.entry(root).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[group].push(at);
        }

        let mut used_keys: HashSet<String> = HashSet::new();
        let mut works: Vec<MissingWorkDto> = Vec::new();
        for members in groups {
            let mut nodes: Vec<usize> = members.iter().map(|&m| cited[m].node).collect();
            nodes.sort_unstable();
            nodes.dedup();
            if nodes.len() < min_cited_by.max(1) as usize {
                continue;
            }
            let ids = members.iter().flat_map(|&m| cited[m].ids.iter());
            let doi = ids.clone().find_map(|id| match id {
                CitationIdDto::Doi(doi) => Some(doi.to_ascii_lowercase()),
                CitationIdDto::ArXiv(_) => None,
            });
            let arxiv = ids.clone().find_map(|id| match id {
                CitationIdDto::ArXiv(arxiv) => Some(arxiv.to_ascii_lowercase()),
                CitationIdDto::Doi(_) => None,
            });
            let printed: Vec<&StoredRef> = members.iter().filter_map(|&m| cited[m].entry).collect();
            let text = printed
                .iter()
                .map(|entry| entry.text.as_str())
                .filter(|text| !text.is_empty())
                .fold(None::<&str>, |best, text| match best {
                    Some(best) if best.chars().count() >= text.chars().count() => Some(best),
                    _ => Some(text),
                })
                .map(str::to_owned);
            let mut years: HashMap<i32, usize> = HashMap::new();
            for entry in &printed {
                if let Some(year) = entry.year {
                    *years.entry(year).or_default() += 1;
                }
            }
            let year = years
                .into_iter()
                .max_by_key(|&(year, count)| (count, std::cmp::Reverse(year)))
                .map(|(year, _)| year)
                .or_else(|| arxiv.as_deref().and_then(identifiers::year_from_arxiv));
            let base = match (&doi, &arxiv) {
                (Some(doi), _) => format!("doi:{doi}"),
                (None, Some(arxiv)) => format!("arxiv:{arxiv}"),
                (None, None) => {
                    let key = members
                        .iter()
                        .find_map(|&m| keys[m].as_deref())
                        .map(|key| key.chars().take(PREFIX_CHARS).collect::<String>())
                        .unwrap_or_else(|| compact(text.as_deref().unwrap_or("")));
                    format!("ref:{key}")
                }
            };
            let mut key = base.clone();
            let mut n = 2;
            while !used_keys.insert(key.clone()) {
                key = format!("{base}#{n}");
                n += 1;
            }
            works.push(MissingWorkDto {
                key,
                text,
                year,
                doi,
                arxiv,
                cited_by: nodes.iter().map(|&node| view[node].id.clone()).collect(),
            });
        }
        works.sort_by(|a, b| {
            b.cited_by
                .len()
                .cmp(&a.cited_by.len())
                .then_with(|| b.year.cmp(&a.year))
                .then_with(|| a.key.cmp(&b.key))
        });
        works
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::references::{stored_refs, ProvidedRef};
    use crate::library::tests::indexed_entry;
    use crate::library::types::{MetaField, MetaSource, PaperFile};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use CitationIdDto::{ArXiv, Doi};

    fn store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        std::mem::forget(dir);
        store
    }

    /// A paper with one file at `/l/<name>.pdf` and the given title.
    fn add(store: &mut PaperStore, name: &str, title: &str, year: i32) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = Some(MetaField {
            value: title.to_owned(),
            source: MetaSource::Crossref,
        });
        paper.meta.year = Some(MetaField {
            value: year,
            source: MetaSource::Crossref,
        });
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

    fn entry(name: &str, cites: Vec<CitationIdDto>) -> LibraryEntry {
        let mut entry = indexed_entry(Path::new(&format!("/l/{name}.pdf")), 1, 1);
        entry.fingerprint = Some(name.to_owned());
        entry.cites = Some(cites);
        entry
    }

    fn list(refs: &mut DocRefs, name: &str, texts: &[(&str, i32)]) {
        let provided: Vec<ProvidedRef> = texts
            .iter()
            .map(|(text, year)| ProvidedRef {
                label: None,
                text: (*text).to_owned(),
                year: Some(*year),
            })
            .collect();
        refs.insert(
            PathBuf::from(format!("/l/{name}.pdf")),
            Arc::new(stored_refs(&provided, None, None)),
        );
    }

    const MISSING_LONG: &str = "J. Doe and R. Roe, A rather long title about a work nobody here has, J. Phys. A 12, 345 (2001).";
    const MISSING_LONGER: &str = "J. Doe and R. Roe, A rather long title about a work nobody here has, J. Phys. A 12, 345 (2001), and its erratum.";
    const MISSING_INSIDE: &str = "See also: Q. Moe, Another quite long title that the library lacks entirely, Nature 500, 1 (2013).";
    const MISSING_BARE: &str =
        "Q. Moe, Another quite long title that the library lacks entirely, Nature 500, 1 (2013).";
    const OTHER_WORK: &str = "J. Doe and R. Roe, A different work by the same pair of authors entirely, J. Phys. A 13, 1 (2002).";

    #[test]
    fn works_are_grouped_by_identifier_and_by_reference_text() {
        let mut store = store();
        let x = add(&mut store, "x", "Paper X about some topic here", 2020);
        let y = add(&mut store, "y", "Paper Y about some other topic", 2021);
        let z = add(
            &mut store,
            "z",
            "A library paper on optimal transport geometry",
            2015,
        );
        store.papers[2].ids.doi = Some("10.1000/z".into());
        store.papers[2].alias_ids = vec![ArXiv("1501.00001".into())];
        let entries = vec![
            entry(
                "x",
                vec![
                    Doi("10.1234/m".into()),
                    Doi("10.1000/z".into()),
                    ArXiv("1501.00001".into()),
                    ArXiv("2101.00042".into()),
                ],
            ),
            entry("y", vec![Doi("10.1234/m".into())]),
            entry("z", Vec::new()),
        ];
        let mut refs = DocRefs::new();
        list(
            &mut refs,
            "x",
            &[
                (MISSING_LONG, 2001),
                (MISSING_INSIDE, 2013),
                ("A. Author, A library paper on optimal transport geometry, doi:10.1000/z (2015).", 2015),
                ("Ibid.", 2001),
                ("M. Em, The work with a DOI that both cite in full, doi:10.1234/m (2019).", 2019),
            ],
        );
        list(
            &mut refs,
            "y",
            &[
                (MISSING_LONGER, 2001),
                (MISSING_BARE, 2013),
                (OTHER_WORK, 2002),
            ],
        );
        let network = NetworkEdges::default();
        let works = store
            .missing_works(None, None, 1, 1, &entries, &refs, &network)
            .unwrap();
        let by_key = |key: &str| works.iter().find(|w| w.key == key);

        let m = by_key("doi:10.1234/m").expect("the DOI both cite");
        assert_eq!(m.cited_by, [x.clone(), y.clone()]);
        assert_eq!(m.year, Some(2019));
        assert!(m.text.as_deref().unwrap().contains("both cite in full"));
        assert_eq!(works[0].key, "doi:10.1234/m", "most cited first");

        // Text: one's prefix in the other, both ways round.
        let long = works
            .iter()
            .find(|w| {
                w.text
                    .as_deref()
                    .is_some_and(|t| t.contains("rather long title"))
            })
            .unwrap();
        assert_eq!(long.cited_by.len(), 2);
        assert_eq!(
            long.text.as_deref(),
            Some(MISSING_LONGER),
            "the longest printing"
        );
        assert_eq!(long.year, Some(2001));
        assert!(long.key.starts_with("ref:jdoeandrroe"));
        let inside = works
            .iter()
            .find(|w| {
                w.text
                    .as_deref()
                    .is_some_and(|t| t.contains("Another quite long"))
            })
            .unwrap();
        assert_eq!(inside.cited_by.len(), 2);
        // Same authors, another work: kept apart.
        let other = works
            .iter()
            .find(|w| w.text.as_deref() == Some(OTHER_WORK))
            .unwrap();
        assert_eq!(other.cited_by, std::slice::from_ref(&y));

        // In the library, by DOI, by alias, by title: never missing.
        assert!(works.iter().all(|w| w.doi.as_deref() != Some("10.1000/z")));
        assert!(works
            .iter()
            .all(|w| w.arxiv.as_deref() != Some("1501.00001")));
        assert!(works.iter().all(|w| !w
            .text
            .as_deref()
            .unwrap_or("")
            .contains("optimal transport")));
        assert!(works.iter().all(|w| !w.cited_by.contains(&z)));
        let arxiv = by_key("arxiv:2101.00042").expect("an arXiv id only the text names");
        assert_eq!(arxiv.text, None);
        assert_eq!(arxiv.year, Some(2021));
        assert!(works.iter().all(|w| !w.key.contains("ibid")), "too short");
        assert_eq!(works.len(), 5);

        // At least two citing items.
        let works = store
            .missing_works(None, None, 1, 2, &entries, &refs, &network)
            .unwrap();
        assert_eq!(works.len(), 3);
        assert!(works.iter().all(|w| w.cited_by.len() == 2));

        // A project narrows the citing items.
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&y, &project, true).unwrap();
        let works = store
            .missing_works(Some(&project), None, 1, 1, &entries, &refs, &network)
            .unwrap();
        assert!(works.iter().all(|w| w.cited_by == [y.clone()]));
        assert_eq!(works.len(), 4);
    }

    /// A -> B -> C -> D, and E on its own in a project.
    fn chain() -> (PaperStore, Vec<String>, Vec<LibraryEntry>) {
        let mut store = store();
        let names = ["a", "b", "c", "d", "e"];
        let ids: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let id = add(
                    &mut store,
                    name,
                    &format!("Paper {name} of the chain"),
                    2000 + i as i32,
                );
                store.papers[i].ids.doi = Some(format!("10.1/{name}"));
                id
            })
            .collect();
        let entries = vec![
            entry("a", vec![Doi("10.1/b".into()), Doi("10.9/ghost-a".into())]),
            entry("b", vec![Doi("10.1/c".into())]),
            entry("c", vec![Doi("10.1/d".into()), Doi("10.9/ghost-c".into())]),
            entry("d", vec![Doi("10.9/ghost-d".into())]),
            entry("e", Vec::new()),
        ];
        (store, ids, entries)
    }

    #[test]
    fn the_focus_graph_reaches_depth_steps_either_way_across_the_library() {
        let (mut store, ids, entries) = chain();
        let refs = DocRefs::new();
        let network = NetworkEdges::default();
        let node_ids = |graph: &crate::library::types::GraphDto| {
            let mut nodes: Vec<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
            nodes.sort();
            nodes
        };
        let sorted = |picked: &[usize]| {
            let mut out: Vec<String> = picked.iter().map(|&i| ids[i].clone()).collect();
            out.sort();
            out
        };
        let one = store
            .focus_graph(&ids[1], 1, &entries, &refs, &network)
            .unwrap();
        assert_eq!(node_ids(&one), sorted(&[0, 1, 2]));
        assert_eq!(one.edges.len(), 2);
        assert_eq!(one.external, 3, "a's ghost, c's ghost, and d");
        let two = store
            .focus_graph(&ids[1], 2, &entries, &refs, &network)
            .unwrap();
        assert_eq!(node_ids(&two), sorted(&[0, 1, 2, 3]));
        assert_eq!(two.edges.len(), 3);
        let clamped = store
            .focus_graph(&ids[1], 9, &entries, &refs, &network)
            .unwrap();
        assert_eq!(node_ids(&clamped), node_ids(&two));
        let zero = store
            .focus_graph(&ids[0], 0, &entries, &refs, &network)
            .unwrap();
        assert_eq!(node_ids(&zero), sorted(&[0, 1]), "at least one step");
        let alone = store
            .focus_graph(&ids[4], 2, &entries, &refs, &network)
            .unwrap();
        assert_eq!(node_ids(&alone), sorted(&[4]));
        assert!(store
            .focus_graph("nope", 1, &entries, &refs, &network)
            .is_err());

        // Missing works around a focus, which overrides the project.
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&ids[4], &project, true).unwrap();
        let works = store
            .missing_works(
                Some(&project),
                Some(&ids[1]),
                1,
                1,
                &entries,
                &refs,
                &network,
            )
            .unwrap();
        let mut keys: Vec<&str> = works.iter().map(|w| w.key.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["doi:10.9/ghost-a", "doi:10.9/ghost-c"]);
        let works = store
            .missing_works(Some(&project), None, 1, 1, &entries, &refs, &network)
            .unwrap();
        assert!(works.is_empty());
    }
}

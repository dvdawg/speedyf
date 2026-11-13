//! Works often cited alongside a project's members (projects.md sec. 8,
//! "Cited with these"): what the library's papers cite together with the
//! members, that is not in the project.
//!
//! A candidate is a library item outside the project, or a missing work -
//! a citation that resolves to no record, grouped and keyed exactly as the
//! missing-works list groups and keys them (`missing.rs`, read here over
//! the whole library). The citing side is every paper in the library (and
//! every member), with records that are versions of one another - sharing
//! a DOI or an arXiv id (aliases included), or printing the same title
//! under the same first author - counted as one citing work.
//!
//! **Score.** For a candidate X, the citing works whose bibliography holds
//! X and at least one member are its co-citers. Each co-citer contributes
//! the number of members it cites; the sum is divided by the square root
//! of how many citing works in the whole library cite X, so a textbook
//! everybody cites does not win on reach alone. A candidate is shown when
//! at least [`MIN_CITERS`] co-citers and at least `max(MIN_MEMBERS,
//! min_members)` distinct members are behind it. Members never are, nor are
//! versions of members, nor the missing works the "cited by these" list
//! already shows (cited by at least `min_cited_by` members), so the two
//! lists never repeat each other. Sorted by score, then members, then
//! co-citers, then key; at most [`MAX_COCITED`].
//!
//! One pass over every bibliography builds the cited-work sets and the
//! inverted index from a work to the works citing it; only the co-citers
//! of members are then visited.

use super::lookup::NetworkEdges;
use super::missing::id_key;
use super::papers::PaperRecord;
use super::references::{compact, DocRefs};
use super::store::PaperStore;
use super::LibraryEntry;
use crate::errors::AppResult;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

/// The most candidates one answer lists.
pub const MAX_COCITED: usize = 20;
/// Distinct citing works a candidate needs.
pub const MIN_CITERS: usize = 2;
/// Distinct members a candidate must be cited with, at the least.
pub const MIN_MEMBERS: u32 = 2;
/// Titles shorter than this (compacted) do not make two records one work.
const MIN_TITLE_CHARS: usize = 20;

/// A work often cited with a project's members.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CoCitedWorkDto {
    /// `item:<record id>` for a library item; a missing work's key as the
    /// missing-works list gives it otherwise.
    pub key: String,
    /// The library item, when it is one.
    pub paper_id: Option<String>,
    /// The item's title.
    pub title: Option<String>,
    /// A missing work's reference as printed.
    pub text: Option<String>,
    pub year: Option<i32>,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    /// The members it is cited with, in list order.
    pub members: Vec<String>,
    /// Distinct citing works that cite it with a member.
    pub citers: u32,
    /// Distinct citing works in the library that cite it at all.
    pub cited_in_library: u32,
    pub score: f64,
}

/// A cited work: a library item (its version class) or a missing work
/// (its index in the library-wide missing list).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Work {
    Item(usize),
    Missing(usize),
}

#[derive(Default)]
struct Tally {
    citers: u32,
    weight: u32,
    members: BTreeSet<usize>,
}

/// Union-find roots: records that are versions of one another share one.
fn version_classes(papers: &[PaperRecord]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..papers.len()).collect();
    fn find(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut first: HashMap<String, usize> = HashMap::new();
    for (index, paper) in papers.iter().enumerate() {
        let mut keys: Vec<String> = paper.citation_ids().iter().map(id_key).collect();
        if let Some(title) = paper.title().map(compact) {
            if title.len() >= MIN_TITLE_CHARS {
                let author = paper
                    .authors()
                    .first()
                    .map(|a| compact(&a.family))
                    .unwrap_or_default();
                keys.push(format!("t:{title}|{author}"));
            }
        }
        for key in keys {
            match first.get(&key) {
                Some(&other) => {
                    let (a, b) = (find(&mut parent, other), find(&mut parent, index));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
                None => {
                    first.insert(key, index);
                }
            }
        }
    }
    (0..papers.len()).map(|i| find(&mut parent, i)).collect()
}

impl PaperStore {
    /// Works the library cites alongside `project_id`'s members (see the
    /// module doc). `min_members` raises the members a candidate needs
    /// above [`MIN_MEMBERS`]; `min_cited_by` is the missing-works list's
    /// threshold, whose works are left out here.
    #[allow(clippy::too_many_arguments)]
    pub fn cocited_works(
        &self,
        project_id: &str,
        min_members: u32,
        min_cited_by: u32,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> AppResult<Vec<CoCitedWorkDto>> {
        let members = self.project_members(project_id)?;
        let paper_index: HashMap<&str, usize> = self
            .papers
            .iter()
            .enumerate()
            .map(|(index, paper)| (paper.id.as_str(), index))
            .collect();
        let member_indices: HashSet<usize> = members
            .iter()
            .map(|paper| paper_index[paper.id.as_str()])
            .collect();
        if member_indices.is_empty() {
            return Ok(Vec::new());
        }
        let class = version_classes(&self.papers);
        let member_classes: HashSet<usize> = member_indices.iter().map(|&m| class[m]).collect();
        let citing: Vec<bool> = self
            .papers
            .iter()
            .enumerate()
            .map(|(index, paper)| paper.is_paper() || member_indices.contains(&index))
            .collect();

        // What each citing work cites, by class root.
        let mut cited: Vec<HashSet<Work>> = vec![HashSet::new(); self.papers.len()];
        let tables = self.citation_tables();
        let mut cite_item = |from: usize, to: usize| {
            if citing[from] && class[from] != class[to] {
                cited[class[from]].insert(Work::Item(class[to]));
            }
        };
        for entry in entries {
            let Some(ids) = entry.cites.as_ref() else {
                continue;
            };
            let path = entry.path.to_string_lossy();
            let Some(&holder) = tables.by_path.get(path.as_ref()) else {
                continue;
            };
            for reference in refs.get(&entry.path).map(|l| l.as_slice()).unwrap_or(&[]) {
                if let Some(target) = tables.resolve_reference(reference, holder) {
                    cite_item(holder, target);
                }
            }
            for id in ids {
                if let Some(&target) = tables.by_id.get(id) {
                    cite_item(holder, target);
                }
            }
        }
        for (index, paper) in self.papers.iter().enumerate() {
            for id in paper.citation_ids() {
                for reference in network.refs.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                    if let Some(&target) = tables.by_id.get(reference) {
                        cite_item(index, target);
                    }
                }
                for source in network.cites.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                    if let Some(&from) = tables.by_id.get(source) {
                        cite_item(from, index);
                    }
                }
            }
        }
        drop(tables);

        // Missing works over every citing record, keyed as the list keys
        // them; those the "cited by these" list shows are left out.
        let view: Vec<&PaperRecord> = self
            .papers
            .iter()
            .enumerate()
            .filter(|(index, _)| citing[*index])
            .map(|(_, paper)| paper)
            .collect();
        let missing = self.missing_over(view, 1, entries, refs, network);
        let shown: HashSet<String> = self
            .missing_works(
                Some(project_id),
                None,
                1,
                min_cited_by,
                entries,
                refs,
                network,
            )?
            .into_iter()
            .map(|work| work.key)
            .collect();
        let floor = min_cited_by.max(1) as usize;
        let mut excluded: HashSet<usize> = HashSet::new();
        for (at, work) in missing.iter().enumerate() {
            let by_members = work
                .cited_by
                .iter()
                .filter(|id| member_indices.contains(&paper_index[id.as_str()]))
                .count();
            if by_members >= floor || shown.contains(&work.key) {
                excluded.insert(at);
            }
            for id in &work.cited_by {
                cited[class[paper_index[id.as_str()]]].insert(Work::Missing(at));
            }
        }

        // The inverted index: who cites each work.
        let mut citers_of: HashMap<Work, Vec<usize>> = HashMap::new();
        for (from, works) in cited.iter().enumerate() {
            for &work in works {
                citers_of.entry(work).or_default().push(from);
            }
        }
        let mut co_citers: Vec<usize> = member_classes
            .iter()
            .flat_map(|&m| citers_of.get(&Work::Item(m)).into_iter().flatten().copied())
            .collect();
        co_citers.sort_unstable();
        co_citers.dedup();

        let mut tally: HashMap<Work, Tally> = HashMap::new();
        for from in co_citers {
            let with: Vec<usize> = member_classes
                .iter()
                .copied()
                .filter(|&m| cited[from].contains(&Work::Item(m)))
                .collect();
            if with.is_empty() {
                continue;
            }
            for &work in &cited[from] {
                let candidate = match work {
                    Work::Item(c) => !member_classes.contains(&c) && self.papers[c].is_paper(),
                    Work::Missing(at) => !excluded.contains(&at),
                };
                if !candidate {
                    continue;
                }
                let entry = tally.entry(work).or_default();
                entry.citers += 1;
                entry.weight += with.len() as u32;
                entry.members.extend(with.iter().copied());
            }
        }

        let need_members = min_members.max(MIN_MEMBERS) as usize;
        let mut out: Vec<CoCitedWorkDto> = Vec::new();
        for (work, t) in tally {
            if (t.citers as usize) < MIN_CITERS || t.members.len() < need_members {
                continue;
            }
            let total = citers_of.get(&work).map_or(1, Vec::len).max(1);
            let score = t.weight as f64 / (total as f64).sqrt();
            let member_ids: Vec<String> = members
                .iter()
                .filter(|paper| t.members.contains(&class[paper_index[paper.id.as_str()]]))
                .map(|paper| paper.id.clone())
                .collect();
            let base = CoCitedWorkDto {
                key: String::new(),
                paper_id: None,
                title: None,
                text: None,
                year: None,
                doi: None,
                arxiv: None,
                members: member_ids,
                citers: t.citers,
                cited_in_library: total as u32,
                score,
            };
            out.push(match work {
                Work::Item(c) => {
                    let paper = &self.papers[c];
                    CoCitedWorkDto {
                        key: format!("item:{}", paper.id),
                        paper_id: Some(paper.id.clone()),
                        title: paper.title().map(str::to_owned),
                        year: paper.year(),
                        doi: paper.ids.doi.clone(),
                        arxiv: paper.ids.arxiv.clone(),
                        ..base
                    }
                }
                Work::Missing(at) => {
                    let missing = &missing[at];
                    CoCitedWorkDto {
                        key: missing.key.clone(),
                        text: missing.text.clone(),
                        year: missing.year,
                        doi: missing.doi.clone(),
                        arxiv: missing.arxiv.clone(),
                        ..base
                    }
                }
            });
        }
        out.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| b.members.len().cmp(&a.members.len()))
                .then_with(|| b.citers.cmp(&a.citers))
                .then_with(|| a.key.cmp(&b.key))
        });
        out.truncate(MAX_COCITED);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::CitationIdDto::{self, Doi};
    use crate::library::tests::indexed_entry;
    use crate::library::types::{Author, MetaField, MetaSource, PaperFile};
    use std::path::Path;

    fn store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        std::mem::forget(dir);
        store
    }

    /// A paper with one file at `/l/<name>.pdf` and the DOI `10.1/<name>`.
    fn add(store: &mut PaperStore, name: &str) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = Some(MetaField {
            value: format!("Paper {name}"),
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
        paper.ids.doi = Some(format!("10.1/{name}"));
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
                .map(|doi| CitationIdDto::Doi((*doi).to_owned()))
                .collect(),
        );
        entry
    }

    struct Fixture {
        store: PaperStore,
        project: String,
        ids: HashMap<String, String>,
        entries: Vec<LibraryEntry>,
    }

    /// `members` in project P; `cites` as (citing name, cited DOIs).
    fn fixture(members: &[&str], others: &[&str], cites: &[(&str, &[&str])]) -> Fixture {
        let mut store = store();
        let mut ids = HashMap::new();
        for name in members.iter().chain(others) {
            ids.insert((*name).to_owned(), add(&mut store, name));
        }
        let project = store.create_project("P").unwrap().id;
        for name in members {
            store.set_membership(&ids[*name], &project, true).unwrap();
        }
        let entries = members
            .iter()
            .chain(others)
            .map(|name| {
                let list = cites
                    .iter()
                    .find(|(citing, _)| citing == name)
                    .map(|(_, list)| *list)
                    .unwrap_or(&[]);
                entry(name, list)
            })
            .collect();
        Fixture {
            store,
            project,
            ids,
            entries,
        }
    }

    fn run(f: &Fixture, min_members: u32, min_cited_by: u32) -> Vec<CoCitedWorkDto> {
        f.store
            .cocited_works(
                &f.project,
                min_members,
                min_cited_by,
                &f.entries,
                &DocRefs::new(),
                &NetworkEdges::default(),
            )
            .unwrap()
    }

    fn keys(works: &[CoCitedWorkDto]) -> Vec<&str> {
        works.iter().map(|w| w.key.as_str()).collect()
    }

    #[test]
    fn needs_two_citers_and_two_members_and_names_items_and_missing_works() {
        let f = fixture(
            &["m1", "m2", "m3"],
            &["c1", "c2", "c3", "x", "y"],
            &[
                ("c1", &["10.1/m1", "10.1/m2", "10.1/x", "10.9/g"]),
                ("c2", &["10.1/m1", "10.1/x", "10.9/g"]),
                ("c3", &["10.1/m3", "10.1/y"]),
            ],
        );
        let works = run(&f, 2, 2);
        let x = format!("item:{}", f.ids["x"]);
        let mut got = keys(&works);
        got.sort();
        let mut want = vec!["doi:10.9/g", x.as_str()];
        want.sort();
        assert_eq!(got, want, "y has one citer; members are never candidates");
        let item = works.iter().find(|w| w.key == x).unwrap();
        assert_eq!(item.paper_id.as_deref(), Some(f.ids["x"].as_str()));
        assert_eq!(item.title.as_deref(), Some("Paper x"));
        assert_eq!(item.citers, 2);
        let mut members = item.members.clone();
        members.sort();
        let mut want = vec![f.ids["m1"].clone(), f.ids["m2"].clone()];
        want.sort();
        assert_eq!(members, want);
        assert!((item.score - 3.0 / 2f64.sqrt()).abs() < 1e-9);
        let ghost = works.iter().find(|w| w.key == "doi:10.9/g").unwrap();
        assert_eq!(ghost.doi.as_deref(), Some("10.9/g"));
        assert_eq!(ghost.paper_id, None);

        // A higher member floor: two members are not enough.
        assert!(run(&f, 3, 2).is_empty());
    }

    #[test]
    fn a_work_cited_everywhere_does_not_dominate() {
        let mut cites: Vec<(&str, &[&str])> = vec![
            ("c1", &["10.1/m1", "10.1/m2", "10.9/niche", "10.9/textbook"]),
            ("c2", &["10.1/m1", "10.1/m2", "10.9/niche", "10.9/textbook"]),
            ("c3", &["10.1/m1", "10.9/textbook"]),
        ];
        let others: Vec<String> = (0..12).map(|i| format!("o{i}")).collect();
        let only: &[&str] = &["10.9/textbook"];
        for name in &others {
            cites.push((name.as_str(), only));
        }
        let mut names = vec!["c1", "c2", "c3"];
        names.extend(others.iter().map(String::as_str));
        let f = fixture(&["m1", "m2"], &names, &cites);
        let works = run(&f, 2, 2);
        assert_eq!(keys(&works), ["doi:10.9/niche", "doi:10.9/textbook"]);
        let textbook = &works[1];
        assert_eq!(textbook.citers, 3, "more co-citers than the niche work");
        assert_eq!(textbook.cited_in_library, 15);
        assert!(works[0].score > textbook.score);
    }

    #[test]
    fn versions_of_one_citing_work_count_once() {
        let mut f = fixture(
            &["m1", "m2"],
            &["c1", "c1v", "x"],
            &[
                ("c1", &["10.1/m1", "10.1/m2", "10.1/x"]),
                ("c1v", &["10.1/m1", "10.1/m2", "10.1/x"]),
            ],
        );
        // Not versions yet: two citers.
        assert_eq!(run(&f, 2, 2).len(), 1);
        // The preprint answers to the published version's DOI as an alias.
        let at = f
            .store
            .papers
            .iter()
            .position(|p| p.id == f.ids["c1v"])
            .unwrap();
        f.store.papers[at].alias_ids = vec![Doi("10.1/c1".into())];
        assert!(run(&f, 2, 2).is_empty());
        // Same title and first author: one work too.
        f.store.papers[at].alias_ids.clear();
        for name in ["c1", "c1v"] {
            let at = f
                .store
                .papers
                .iter()
                .position(|p| p.id == f.ids[name])
                .unwrap();
            let paper = &mut f.store.papers[at];
            paper.meta.title = Some(MetaField {
                value: "A long shared title of one work".into(),
                source: MetaSource::Crossref,
            });
            paper.meta.authors = Some(MetaField {
                value: vec![Author {
                    family: "Doe".into(),
                    given: String::new(),
                }],
                source: MetaSource::Crossref,
            });
        }
        assert!(run(&f, 2, 2).is_empty());
    }

    #[test]
    fn leaves_out_what_the_cited_by_list_shows_and_versions_of_members() {
        let f = fixture(
            &["m1", "m2"],
            &["c1", "c2", "m1v"],
            &[
                ("m1", &["10.9/h"]),
                ("m2", &["10.9/h"]),
                ("c1", &["10.1/m1", "10.1/m2", "10.9/h", "10.1/m1v"]),
                ("c2", &["10.1/m1", "10.1/m2", "10.9/h", "10.1/m1v"]),
            ],
        );
        // m1v is a different record, not a version yet.
        let works = run(&f, 2, 2);
        assert_eq!(keys(&works), [format!("item:{}", f.ids["m1v"])]);
        // At a threshold the list does not reach, h is fair game.
        let works = run(&f, 2, 3);
        assert!(keys(&works).contains(&"doi:10.9/h"));

        let mut f = f;
        let at = f
            .store
            .papers
            .iter()
            .position(|p| p.id == f.ids["m1v"])
            .unwrap();
        f.store.papers[at].alias_ids = vec![Doi("10.1/m1".into())];
        assert!(run(&f, 2, 2).is_empty(), "a version of a member");
    }

    #[test]
    fn the_order_is_deterministic() {
        let f = fixture(
            &["m1", "m2"],
            &["c1", "c2", "a", "b"],
            &[
                (
                    "c1",
                    &["10.1/m1", "10.1/m2", "10.9/z", "10.9/y", "10.1/a", "10.1/b"],
                ),
                (
                    "c2",
                    &["10.1/m1", "10.1/m2", "10.9/z", "10.9/y", "10.1/a", "10.1/b"],
                ),
            ],
        );
        let first = run(&f, 2, 2);
        assert_eq!(first.len(), 4);
        for _ in 0..5 {
            assert_eq!(run(&f, 2, 2), first);
        }
        let mut sorted = keys(&first);
        sorted.sort();
        assert_eq!(keys(&first), sorted, "equal scores fall back to the key");
    }
}

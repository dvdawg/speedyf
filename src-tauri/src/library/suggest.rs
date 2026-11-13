//! Projects an item might be filed in, read off the citation graph
//! (projects.md sec. 6): for each project the item is not in, how many members
//! it cites, how many cite it, and how many works it and the members both
//! cite.
//!
//! A work is named by keys: `p:<index>` for a citation that resolves to a
//! library record, else the identifier keys and the reference-text key the
//! missing-works grouping uses (`missing::id_key`, `missing::text_key`,
//! exact match only). One of the item's citations is shared with a project
//! when any of its keys is among the keys of the members' citations, and is
//! counted once under the first such key, so a DOI read from the text and
//! the same DOI in a reference entry are one work.

use super::lookup::NetworkEdges;
use super::missing::{id_key, text_key, MIN_KEY_CHARS};
use super::papers::CitationTables;
use super::references::DocRefs;
use super::store::PaperStore;
use super::types::ProjectSuggestionDto;
use super::LibraryEntry;
use std::collections::HashSet;

/// The most suggestions one answer carries.
pub const MAX_SUGGESTIONS: usize = 3;
/// Shared references count up to this many toward the score.
pub const SHARED_CAP: u32 = 10;
/// With no citation either way, this many shared references suggest.
pub const MIN_SHARED: u32 = 3;

/// Each paper's citations as works, a work being its keys in preference
/// order. Indexed like `PaperStore::papers`.
fn works_by_paper(
    store: &PaperStore,
    tables: &CitationTables<'_>,
    entries: &[LibraryEntry],
    refs: &DocRefs,
    network: &NetworkEdges,
) -> Vec<Vec<Vec<String>>> {
    let mut works: Vec<Vec<Vec<String>>> = vec![Vec::new(); store.papers.len()];
    let resolved = |index: usize| vec![format!("p:{index}")];
    for entry in entries {
        let Some(cites) = entry.cites.as_ref() else {
            continue;
        };
        let path = entry.path.to_string_lossy();
        let Some(&holder) = tables.by_path.get(path.as_ref()) else {
            continue;
        };
        for reference in refs.get(&entry.path).map(|l| l.as_slice()).unwrap_or(&[]) {
            match tables.resolve_reference(reference, holder) {
                Some(target) if target == holder => {}
                Some(target) => works[holder].push(resolved(target)),
                None => {
                    let mut keys: Vec<String> = reference.ids.iter().map(id_key).collect();
                    let text = text_key(reference);
                    if text.len() >= MIN_KEY_CHARS {
                        keys.push(format!("t:{text}"));
                    }
                    if !keys.is_empty() {
                        works[holder].push(keys);
                    }
                }
            }
        }
        for id in cites {
            match tables.by_id.get(id) {
                Some(&target) if target == holder => {}
                Some(&target) => works[holder].push(resolved(target)),
                None => works[holder].push(vec![id_key(id)]),
            }
        }
    }
    for (index, paper) in store.papers.iter().enumerate() {
        for id in paper.citation_ids() {
            for cited in network.refs.get(&id).map(Vec::as_slice).unwrap_or(&[]) {
                match tables.by_id.get(cited) {
                    Some(&target) if target == index => {}
                    Some(&target) => works[index].push(resolved(target)),
                    None => works[index].push(vec![id_key(cited)]),
                }
            }
        }
    }
    works
}

/// "cites 2 papers in it, cited by 1 paper in it, shares 4 references with it".
fn reason(cites: u32, cited_by: u32, shared: u32) -> String {
    let papers = |n: u32| if n == 1 { "paper" } else { "papers" };
    let mut parts: Vec<String> = Vec::new();
    if cites > 0 {
        parts.push(format!("cites {cites} {} in it", papers(cites)));
    }
    if cited_by > 0 {
        parts.push(format!("cited by {cited_by} {} in it", papers(cited_by)));
    }
    if shared > 0 {
        let noun = if shared == 1 {
            "reference"
        } else {
            "references"
        };
        parts.push(format!("shares {shared} {noun} with it"));
    }
    parts.join(", ")
}

impl PaperStore {
    /// The projects `paper_id` is not in that the citation graph ties it
    /// to, strongest first, at most [`MAX_SUGGESTIONS`]. Score:
    /// `2 * (cites + cited_by) + min(shared_refs, SHARED_CAP)`, divided by
    /// the square root of the project's size so a big project does not win
    /// on size alone. A project is suggested when the item cites or is
    /// cited by a member, or shares at least [`MIN_SHARED`] references
    /// with the members. An id that names no item answers empty. The
    /// tables are built once per call.
    pub fn suggest_projects(
        &self,
        paper_id: &str,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> Vec<ProjectSuggestionDto> {
        let Ok(own) = self.live_index(paper_id) else {
            return Vec::new();
        };
        let own_id = self.papers[own].id.as_str();
        let tables = self.citation_tables();
        let (cites, cited_by, _) = self.link_indices(own, &tables, entries, refs, network);
        let works = works_by_paper(self, &tables, entries, refs, network);
        let index_of: std::collections::HashMap<&str, usize> = self
            .papers
            .iter()
            .enumerate()
            .map(|(index, paper)| (paper.id.as_str(), index))
            .collect();

        let mut projects: Vec<_> = self.projects.iter().collect();
        projects.sort_by_key(|project| project.order);
        let mut out: Vec<ProjectSuggestionDto> = Vec::new();
        for project in projects {
            if project.paper_ids.iter().any(|id| id == own_id) {
                continue;
            }
            let members: HashSet<usize> = project
                .paper_ids
                .iter()
                .filter_map(|id| index_of.get(id.as_str()).copied())
                .filter(|&index| index != own)
                .collect();
            if members.is_empty() {
                continue;
            }
            let count = |list: &[usize]| list.iter().filter(|i| members.contains(i)).count() as u32;
            let n_cites = count(&cites);
            let n_cited_by = count(&cited_by);
            let member_keys: HashSet<&str> = members
                .iter()
                .flat_map(|&m| works[m].iter().flatten().map(String::as_str))
                .collect();
            let mut shared: HashSet<&str> = HashSet::new();
            for work in &works[own] {
                if let Some(key) = work.iter().find(|key| member_keys.contains(key.as_str())) {
                    shared.insert(key.as_str());
                }
            }
            let n_shared = shared.len() as u32;
            if n_cites + n_cited_by == 0 && n_shared < MIN_SHARED {
                continue;
            }
            let raw = 2 * (n_cites + n_cited_by) + n_shared.min(SHARED_CAP);
            let score = raw as f32 / (members.len() as f32).sqrt();
            out.push(ProjectSuggestionDto {
                project_id: project.id.clone(),
                name: project.name.clone(),
                score,
                cites: n_cites,
                cited_by: n_cited_by,
                shared_refs: n_shared,
                reason: reason(n_cites, n_cited_by, n_shared),
            });
        }
        // Stable: equal scores keep the projects' stored order.
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(MAX_SUGGESTIONS);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::CitationIdDto::{self, Doi};
    use crate::library::papers::PaperRecord;
    use crate::library::references::{stored_refs, ProvidedRef};
    use crate::library::tests::indexed_entry;
    use crate::library::types::{MetaField, MetaSource, PaperFile};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

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

    fn list(refs: &mut DocRefs, name: &str, texts: &[&str]) {
        let provided: Vec<ProvidedRef> = texts
            .iter()
            .map(|text| ProvidedRef {
                label: None,
                text: (*text).to_owned(),
                year: Some(2001),
            })
            .collect();
        refs.insert(
            PathBuf::from(format!("/l/{name}.pdf")),
            Arc::new(stored_refs(&provided, None, None)),
        );
    }

    fn project(store: &mut PaperStore, name: &str, members: &[&String]) -> String {
        let id = store.create_project(name).unwrap().id;
        for member in members {
            store.set_membership(member, &id, true).unwrap();
        }
        id
    }

    const SHARED_TEXT: &str =
        "J. Doe and R. Roe, A rather long title about a work nobody here has, J. Phys. A 12, 345 (2001).";

    #[test]
    fn projects_are_suggested_by_citations_and_shared_references() {
        let mut store = store();
        let t = add(&mut store, "t");
        let a1 = add(&mut store, "a1");
        let b1 = add(&mut store, "b1");
        let b2 = add(&mut store, "b2");
        let c: Vec<String> = (1..=4).map(|i| add(&mut store, &format!("c{i}"))).collect();
        let d1 = add(&mut store, "d1");
        let e1 = add(&mut store, "e1");
        let f: Vec<String> = (1..=9).map(|i| add(&mut store, &format!("f{i}"))).collect();

        let a = project(&mut store, "A", &[&a1]);
        let b = project(&mut store, "B", &[&b1, &b2]);
        let cp = project(&mut store, "C", &c.iter().collect::<Vec<_>>());
        let d = project(&mut store, "D", &[&t, &d1]);
        let e = project(&mut store, "E", &[&e1]);
        let fp = project(&mut store, "F", &f.iter().collect::<Vec<_>>());

        let entries = vec![
            // t cites a1, c1 and d1, and three works outside the library,
            // one of them only by its reference text.
            entry(
                "t",
                &[
                    "10.1/a1", "10.1/c1", "10.1/d1", "10.9/r1", "10.9/r2", "10.9/r4",
                ],
            ),
            entry("b1", &["10.9/r1", "10.9/r2"]),
            entry("b2", &[]),
            // E shares two references: below the threshold.
            entry("e1", &["10.9/r1", "10.9/r4"]),
            // f1 cites t.
            entry("f1", &["10.1/t"]),
        ];
        let mut refs = DocRefs::new();
        list(&mut refs, "t", &[SHARED_TEXT]);
        list(&mut refs, "b2", &[SHARED_TEXT]);
        let network = NetworkEdges::default();

        let got = store.suggest_projects(&t, &entries, &refs, &network);
        let ids: Vec<&str> = got.iter().map(|s| s.project_id.as_str()).collect();
        // B: 3 shared / sqrt(2) = 2.12; A: 2 / 1; C: 2 / sqrt(4) = 1;
        // F: 2 / sqrt(9), fourth, cut.
        assert_eq!(ids, [b.as_str(), a.as_str(), cp.as_str()]);
        assert!(!ids.contains(&d.as_str()), "already in D");
        assert!(!ids.contains(&e.as_str()), "two shared references only");
        assert!(!ids.contains(&fp.as_str()), "at most three");

        assert_eq!(
            (got[0].cites, got[0].cited_by, got[0].shared_refs),
            (0, 0, 3)
        );
        assert_eq!(got[0].name, "B");
        assert_eq!(got[0].reason, "shares 3 references with it");
        assert!((got[0].score - 3.0 / 2f32.sqrt()).abs() < 1e-4);
        assert_eq!(
            (got[1].cites, got[1].cited_by, got[1].shared_refs),
            (1, 0, 0)
        );
        assert_eq!(got[1].reason, "cites 1 paper in it");
        assert!((got[1].score - 2.0).abs() < 1e-4);
        assert!((got[2].score - 1.0).abs() < 1e-4, "normalized by size");

        // Without C and A, F's citation shows: cited by one member.
        store.set_membership(&t, &a, true).unwrap();
        store.delete_project(&cp).unwrap();
        let got = store.suggest_projects(&t, &entries, &refs, &network);
        let ids: Vec<&str> = got.iter().map(|s| s.project_id.as_str()).collect();
        assert_eq!(ids, [b.as_str(), fp.as_str()]);
        assert_eq!(got[1].reason, "cited by 1 paper in it");

        assert!(store
            .suggest_projects("nope", &entries, &refs, &network)
            .is_empty());
    }

    #[test]
    fn the_reason_is_plain_words() {
        assert_eq!(
            reason(2, 1, 4),
            "cites 2 papers in it, cited by 1 paper in it, shares 4 references with it"
        );
        assert_eq!(reason(0, 0, 1), "shares 1 reference with it");
    }
}

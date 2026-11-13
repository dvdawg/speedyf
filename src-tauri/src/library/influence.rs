//! "Cited here": for each item of a scope, how many items of that same
//! scope cite it (projects.md sec. 5, sorting). The edges are the citation
//! graph's (`graph_over`), built once over the scope, so the counts agree
//! with the arrows the graph view draws.

use super::lookup::NetworkEdges;
use super::references::DocRefs;
use super::store::PaperStore;
use super::types::PaperFilterDto;
use super::LibraryEntry;
use std::collections::HashMap;

impl PaperStore {
    /// Item id -> how many items in the scope `filter` names (its query
    /// ignored: a search narrows the rows, not who counts as citing) cite
    /// it. Items no one in the scope cites are left out.
    pub fn cited_here(
        &self,
        filter: &PaperFilterDto,
        entries: &[LibraryEntry],
        refs: &DocRefs,
        network: &NetworkEdges,
    ) -> HashMap<String, u32> {
        let scope = PaperFilterDto {
            query: String::new(),
            ..filter.clone()
        };
        let ids: std::collections::HashSet<String> = self
            .paper_rows(&scope)
            .rows
            .into_iter()
            .map(|row| row.id)
            .collect();
        let nodes: Vec<_> = self.papers.iter().filter(|p| ids.contains(&p.id)).collect();
        let (_, edges) = self.graph_over(nodes.clone(), entries, refs, network);
        let mut counts: HashMap<String, u32> = HashMap::new();
        for (_, to) in edges {
            *counts.entry(nodes[to].id.clone()).or_default() += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::super::consensus::fixtures::store;
    use super::super::papers::PaperRecord;
    use super::super::projects::ProjectRecord;
    use super::super::types::{MetaField, MetaSource, PaperFile};
    use super::*;
    use crate::engine::types::CitationIdDto::Doi;
    use crate::library::tests::indexed_entry;
    use std::path::Path;

    fn add(store: &mut PaperStore, name: &str) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = Some(MetaField {
            value: format!("Paper {name} about a subject"),
            source: MetaSource::Crossref,
        });
        paper.ids.doi = Some(format!("10.1/{name}"));
        paper.files.push(PaperFile {
            path: format!("/l/{name}.pdf"),
            name: format!("{name}.pdf"),
            size: 1,
            mtime_ms: 1,
            ..Default::default()
        });
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    fn citing(name: &str, cites: &[&str]) -> LibraryEntry {
        let mut entry = indexed_entry(Path::new(&format!("/l/{name}.pdf")), 1, 1);
        entry.cites = Some(cites.iter().map(|c| Doi(format!("10.1/{c}"))).collect());
        entry
    }

    #[test]
    fn counts_citers_within_the_scope_only() {
        let mut s = store();
        let a = add(&mut s, "a");
        let b = add(&mut s, "b");
        let c = add(&mut s, "c");
        s.projects = vec![ProjectRecord {
            id: "p".into(),
            name: "P".into(),
            order: 0,
            paper_ids: vec![a.clone(), b.clone()],
            bib_path: None,
            created_at: String::new(),
        }];
        let entries = vec![
            citing("a", &["b", "c"]),
            citing("b", &[]),
            citing("c", &["b", "b"]),
        ];
        let net = NetworkEdges::default();
        let refs = DocRefs::default();

        let all = s.cited_here(&PaperFilterDto::default(), &entries, &refs, &net);
        assert_eq!(all.get(&b), Some(&2));
        assert_eq!(all.get(&c), Some(&1));
        assert_eq!(all.get(&a), None);

        let project = PaperFilterDto {
            project: Some("p".into()),
            query: "unrelated words".into(),
            ..Default::default()
        };
        let here = s.cited_here(&project, &entries, &refs, &net);
        assert_eq!(here.get(&b), Some(&1));
        assert_eq!(here.get(&c), None);
    }
}

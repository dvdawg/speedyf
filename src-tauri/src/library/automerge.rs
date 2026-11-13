//! Automatic merging of duplicate items (projects.md §2, identity).
//!
//! Two item records (`is_paper`) that are plainly one work are merged
//! without asking, through `merge_papers`, so Split restores them exactly:
//!
//! - **Same identifier.** A shared DOI or arXiv id, aliases included; an
//!   arXiv DOI (`10.48550/arxiv.X`) counts as arXiv id X.
//! - **Same title, first author and year.** Equal normalized titles of four
//!   words or more, the same first-author family name (case and accents
//!   folded), known years at most one apart, and no conflicting
//!   identifiers: two different DOIs, or two different arXiv ids, are two
//!   works.
//!
//! Never for a pair the user dismissed (`not_duplicates`, which merging
//! carries over, so a dismissal reaches through a third record), for a
//! non-item, or in a read-only store. Triggered whenever an identifier or a
//! lookup lands on a record, after a scan reconciles a file, and once over
//! the whole store on load. The keeper is the record with more project
//! memberships, then a pinned key, then user edits, then more files, then
//! the earlier `added_at`, then the smaller id. The reason is kept on the
//! keeper; splitting an automatic merge records the records as not
//! duplicates, so they stay apart.

use super::papers::{fold, PaperRecord};
use super::store::PaperStore;
use super::titlematch;
use super::types::MetaSource;
use crate::engine::types::CitationIdDto;
use std::collections::{HashMap, HashSet};

/// A title needs this many words before it can make two records one.
const MIN_TITLE_WORDS: usize = 4;
const ARXIV_DOI_PREFIX: &str = "10.48550/arxiv.";

/// The identifiers a record answers to, aliases included, as (is arXiv,
/// value): an arXiv DOI is read as its arXiv id. May repeat one.
fn key_iter(paper: &PaperRecord) -> impl Iterator<Item = (bool, &str)> {
    let own = [
        paper.ids.doi.as_deref().map(|doi| (false, doi)),
        paper.ids.arxiv.as_deref().map(|arxiv| (true, arxiv)),
    ];
    let aliases = paper.alias_ids.iter().map(|id| match id {
        CitationIdDto::Doi(doi) => (false, doi.as_str()),
        CitationIdDto::ArXiv(arxiv) => (true, arxiv.as_str()),
    });
    own.into_iter()
        .flatten()
        .chain(aliases)
        .map(
            |(arxiv, value)| match value.strip_prefix(ARXIV_DOI_PREFIX) {
                Some(rest) if !arxiv => (true, rest),
                _ => (arxiv, value),
            },
        )
}

/// The same, each once.
fn identity_keys(paper: &PaperRecord) -> Vec<(bool, &str)> {
    let mut keys: Vec<(bool, &str)> = Vec::new();
    for key in key_iter(paper) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn normalized_title(paper: &PaperRecord) -> Option<String> {
    let title = titlematch::normalize(paper.title()?);
    (title.split(' ').count() >= MIN_TITLE_WORDS).then_some(title)
}

fn first_family(paper: &PaperRecord) -> Option<String> {
    let family: String = fold(paper.authors().first()?.family.trim())
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    (!family.is_empty()).then_some(family)
}

/// Two DOIs, or two arXiv ids, and none in common: two works.
fn conflicting(a: &[(bool, &str)], b: &[(bool, &str)]) -> bool {
    [false, true].into_iter().any(|arxiv| {
        let left: Vec<&str> = a.iter().filter(|k| k.0 == arxiv).map(|k| k.1).collect();
        let right: Vec<&str> = b.iter().filter(|k| k.0 == arxiv).map(|k| k.1).collect();
        !left.is_empty() && !right.is_empty() && !left.iter().any(|x| right.contains(x))
    })
}

/// What a record is matched by, worked out once per subject.
struct Subject<'a> {
    keys: Vec<(bool, &'a str)>,
    title: Option<String>,
    family: Option<String>,
    year: Option<i32>,
}

impl<'a> Subject<'a> {
    fn of(paper: &'a PaperRecord) -> Self {
        Self {
            keys: identity_keys(paper),
            title: normalized_title(paper),
            family: first_family(paper),
            year: paper.year(),
        }
    }

    /// Why `other` is the same work, if it is. Cheap checks first: this
    /// runs against every record each time a scan changes one.
    fn reason(&self, other: &PaperRecord) -> Option<String> {
        if let Some((arxiv, value)) = key_iter(other).find(|key| self.keys.contains(key)) {
            return Some(if arxiv {
                format!("same arXiv id {value}")
            } else {
                format!("same DOI {value}")
            });
        }
        let (Some(title), Some(family), Some(year)) = (&self.title, &self.family, self.year) else {
            return None;
        };
        let other_year = other.year()?;
        if (year - other_year).abs() > 1 || other.title().is_none() {
            return None;
        }
        if first_family(other).as_ref() != Some(family)
            || normalized_title(other).as_ref() != Some(title)
            || conflicting(&self.keys, &identity_keys(other))
        {
            return None;
        }
        Some("same title, first author and year".to_owned())
    }
}

fn dismissed(a: &PaperRecord, b: &PaperRecord) -> bool {
    a.not_duplicates.contains(&b.id) || b.not_duplicates.contains(&a.id)
}

fn user_edited(paper: &PaperRecord) -> bool {
    let user = |source: Option<MetaSource>| source == Some(MetaSource::User);
    let (ids, meta) = (&paper.ids, &paper.meta);
    paper.paper_override.is_some()
        || user(ids.doi_source)
        || user(ids.arxiv_source)
        || user(ids.isbn_source)
        || user(meta.title.as_ref().map(|f| f.source))
        || user(meta.authors.as_ref().map(|f| f.source))
        || user(meta.year.as_ref().map(|f| f.source))
        || user(meta.venue.as_ref().map(|f| f.source))
        || user(meta.volume.as_ref().map(|f| f.source))
        || user(meta.issue.as_ref().map(|f| f.source))
        || user(meta.pages.as_ref().map(|f| f.source))
        || user(meta.publisher.as_ref().map(|f| f.source))
        || user(meta.abstract_.as_ref().map(|f| f.source))
        || user(meta.kind.as_ref().map(|f| f.source))
}

impl PaperStore {
    /// Which of two records keeps the merge: more memberships, a pinned
    /// key, user edits, more files, added earlier, the smaller id.
    fn keeper_of(&self, a: usize, b: usize) -> usize {
        let rank = |index: usize| {
            let paper = &self.papers[index];
            (
                std::cmp::Reverse(self.project_ids_for(&paper.id).len()),
                !paper.cite_key_pinned,
                !user_edited(paper),
                std::cmp::Reverse(paper.files.len()),
                paper.added_at.clone(),
                paper.id.clone(),
            )
        };
        if rank(a) <= rank(b) {
            a
        } else {
            b
        }
    }

    /// The first record, in store order, that is the same work as the one
    /// at `index`, with the reason.
    fn auto_partner(&self, index: usize) -> Option<(usize, String)> {
        let paper = &self.papers[index];
        if !paper.is_paper() {
            return None;
        }
        let subject = Subject::of(paper);
        self.papers.iter().enumerate().find_map(|(at, other)| {
            if at == index || !other.is_paper() || dismissed(paper, other) {
                return None;
            }
            subject.reason(other).map(|reason| (at, reason))
        })
    }

    /// Merge every record that is the same work as `id` into one, one pair
    /// at a time so each merge is judged against the record it made.
    /// Returns the surviving record's id (`id` itself when nothing merged,
    /// or when `id` is unknown) and whether anything merged.
    pub fn auto_merge_around(&mut self, id: &str) -> (String, bool) {
        let mut current = id.to_owned();
        let mut merged = false;
        if !self.writable() {
            return (current, merged);
        }
        while let Ok(index) = self.index_of(&current) {
            let Some((other, reason)) = self.auto_partner(index) else {
                break;
            };
            let keep = self.keeper_of(index, other);
            let gone = if keep == index { other } else { index };
            let (keep_id, gone_id) = (self.papers[keep].id.clone(), self.papers[gone].id.clone());
            if self
                .merge_papers(&keep_id, std::slice::from_ref(&gone_id))
                .is_err()
            {
                break;
            }
            let keep = self
                .index_of(&keep_id)
                .expect("the keeper survives a merge");
            let paper = &mut self.papers[keep];
            if let Some(trace) = paper
                .merged_from
                .iter_mut()
                .find(|snapshot| snapshot.id == gone_id)
                .and_then(|snapshot| snapshot.merge_trace.as_mut())
            {
                trace.auto = true;
            }
            if !paper.auto_merge_reasons.contains(&reason) {
                paper.auto_merge_reasons.push(reason);
            }
            log::info!("merged library record {gone_id} into {keep_id} automatically");
            current = keep_id;
            merged = true;
        }
        (current, merged)
    }

    /// Merge every pair of records that are the same work, in a fixed
    /// order (added first, then id). Returns whether anything merged. The
    /// candidates are found through tables first, so a library with
    /// nothing to merge costs one pass.
    pub fn auto_merge_all(&mut self) -> bool {
        if !self.writable() {
            return false;
        }
        let mut by_key: HashMap<(bool, &str), Vec<usize>> = HashMap::new();
        let mut by_title: HashMap<(String, String), Vec<usize>> = HashMap::new();
        for (index, paper) in self.papers.iter().enumerate() {
            if !paper.is_paper() {
                continue;
            }
            for key in identity_keys(paper) {
                by_key.entry(key).or_default().push(index);
            }
            if let (Some(title), Some(family), Some(_)) =
                (normalized_title(paper), first_family(paper), paper.year())
            {
                by_title.entry((title, family)).or_default().push(index);
            }
        }
        let involved: HashSet<usize> = by_key
            .into_values()
            .chain(by_title.into_values())
            .filter(|list| list.len() > 1)
            .flatten()
            .collect();
        if involved.is_empty() {
            return false;
        }
        let mut order: Vec<(String, String)> = involved
            .into_iter()
            .map(|index| {
                let paper = &self.papers[index];
                (paper.added_at.clone(), paper.id.clone())
            })
            .collect();
        order.sort();
        let mut merged = false;
        for (_, id) in order {
            merged |= self.auto_merge_around(&id).1;
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::indexed_entry;
    use crate::library::types::{Author, LookupCandidateDto, MetaField, MetaPatchDto, PaperFile};
    use crate::library::LibraryEntry;
    use std::path::Path;

    fn store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        std::mem::forget(dir);
        store
    }

    fn field<T>(value: T, source: MetaSource) -> Option<MetaField<T>> {
        Some(MetaField { value, source })
    }

    fn file(path: &str) -> PaperFile {
        PaperFile {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            size: 100,
            mtime_ms: 1,
            fingerprint: Some(path.to_owned()),
            missing: false,
            ..Default::default()
        }
    }

    /// A record pushed straight into the store, as a load would find it.
    fn add(
        store: &mut PaperStore,
        title: &str,
        family: &str,
        year: Option<i32>,
        paths: &[&str],
        added_at: &str,
    ) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = field(title.to_owned(), MetaSource::FirstPage);
        if !family.is_empty() {
            paper.meta.authors = field(
                vec![Author {
                    family: family.to_owned(),
                    given: String::new(),
                }],
                MetaSource::FirstPage,
            );
        }
        paper.meta.year = year.and_then(|y| field(y, MetaSource::FirstPage));
        for path in paths {
            paper.files.push(file(path));
        }
        paper.added_at = added_at.to_owned();
        paper.cite_key = store.free_key(&paper.meta, None);
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    fn get<'a>(store: &'a PaperStore, id: &str) -> &'a PaperRecord {
        &store.papers[store.index_of(id).unwrap()]
    }

    fn get_mut<'a>(store: &'a mut PaperStore, id: &str) -> &'a mut PaperRecord {
        let index = store.index_of(id).unwrap();
        &mut store.papers[index]
    }

    fn paths(store: &PaperStore, id: &str) -> Vec<String> {
        get(store, id)
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect()
    }

    const MOT: &str = "Majority-of-Three is Optimal";
    const LONG: &str = "Majority of three is optimal for learning";

    /// The case that prompted this: the preprint and its copies on one
    /// record, the review copy on another; a lookup gives the second the
    /// first's arXiv id.
    #[test]
    fn a_lookup_that_lands_anothers_arxiv_id_merges_the_two() {
        let mut store = store();
        let first = add(
            &mut store,
            MOT,
            "Rawal",
            Some(2026),
            &["/l/2606.13614v1.pdf", "/l/mot.pdf", "/l/mot copy.pdf"],
            "2026-06-01T00:00:00Z",
        );
        get_mut(&mut store, &first).ids.arxiv = Some("2606.13614".into());
        let second = add(
            &mut store,
            "Anonymous submission",
            "",
            None,
            &["/l/mot_tmlr.pdf"],
            "2026-06-02T00:00:00Z",
        );
        let candidate = LookupCandidateDto {
            source: MetaSource::Arxiv,
            title: MOT.into(),
            authors: vec![Author {
                family: "Rawal".into(),
                given: "Divit".into(),
            }],
            year: Some(2026),
            venue: None,
            volume: None,
            issue: None,
            pages: None,
            publisher: None,
            abstract_: None,
            kind: None,
            doi: None,
            arxiv: Some("2606.13614".into()),
            isbn: None,
        };
        let dto = store.apply_candidate(&second, &candidate).unwrap();
        assert_eq!(dto.id, first, "the record with more files keeps");
        assert_eq!(store.papers.len(), 1);
        assert_eq!(dto.auto_merged, ["same arXiv id 2606.13614"]);
        assert_eq!(dto.files.len(), 4);
        assert_eq!(
            dto.files[0].path, "/l/2606.13614v1.pdf",
            "keeper's main file"
        );
        // The merged-away id still finds the item.
        store.mark_looked_up(&second).unwrap();
        assert_eq!(store.paper_dto(&second).unwrap().id, first);
    }

    #[test]
    fn a_patched_identifier_merges_and_an_arxiv_doi_counts_as_the_arxiv_id() {
        let mut store = store();
        let a = add(
            &mut store,
            "One",
            "",
            None,
            &["/l/a.pdf"],
            "2026-01-01T00:00:00Z",
        );
        get_mut(&mut store, &a).ids.doi = Some("10.48550/arxiv.2401.00001".into());
        let b = add(
            &mut store,
            "Two",
            "",
            None,
            &["/l/b.pdf"],
            "2026-01-02T00:00:00Z",
        );
        let dto = store
            .apply_patch(
                &b,
                MetaPatchDto {
                    arxiv: Some(Some("2401.00001v2".into())),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        assert_eq!(store.papers.len(), 1);
        // b's identifier is the user's: user edits beat an earlier record.
        assert_eq!(dto.id, b);
        assert_eq!(dto.auto_merged, ["same arXiv id 2401.00001"]);
        assert!(
            get(&store, &b).merged_from[0]
                .merge_trace
                .as_ref()
                .unwrap()
                .auto
        );
    }

    fn entry(path: &str, arxiv: Option<&str>) -> LibraryEntry {
        let mut entry = indexed_entry(Path::new(path), 1, 100);
        entry.fingerprint = Some(path.to_owned());
        entry.arxiv = arxiv.map(str::to_owned);
        entry
    }

    #[test]
    fn a_rescan_that_finds_an_identifier_merges() {
        let mut store = store();
        store.reconcile_file(&entry("/l/a.pdf", None));
        store.reconcile_file(&entry("/l/b.pdf", None));
        assert_eq!(store.papers.len(), 2);
        // The second holds the preprint's arXiv DOI (a lookup's); the first
        // file now reads as the preprint itself.
        let b = store.papers[1].id.clone();
        get_mut(&mut store, &b).ids.doi = Some("10.48550/arxiv.2401.00002".into());
        store.reconcile_file(&entry("/l/a.pdf", Some("2401.00002v1")));
        assert_eq!(store.papers.len(), 1);
        let paper = &store.papers[0];
        assert_eq!(paper.files.len(), 2);
        assert_eq!(paper.auto_merge_reasons, ["same arXiv id 2401.00002"]);
    }

    #[test]
    fn same_title_first_author_and_year_merge_and_near_misses_do_not() {
        let mut store = store();
        let at = "2026-01-01T00:00:00Z";
        let a = add(&mut store, LONG, "Rawal", Some(2025), &["/l/a.pdf"], at);
        let b = add(
            &mut store,
            "Majority of Three is Optimal, for Learning.",
            "RAWAL",
            Some(2026),
            &["/l/b.pdf"],
            at,
        );
        // Different first author; two years apart; a conflicting DOI; a
        // three-word title.
        let other_author = add(
            &mut store,
            LONG,
            "Zhivotovskiy",
            Some(2025),
            &["/l/c.pdf"],
            at,
        );
        let far = add(&mut store, LONG, "Rawal", Some(2023), &["/l/d.pdf"], at);
        let short_a = add(
            &mut store,
            "Majority is optimal",
            "Rawal",
            Some(2025),
            &["/l/e.pdf"],
            at,
        );
        let short_b = add(
            &mut store,
            "Majority is optimal",
            "Rawal",
            Some(2025),
            &["/l/f.pdf"],
            at,
        );
        let doi_a = add(
            &mut store,
            "A study of many other things",
            "Ng",
            Some(2020),
            &["/l/g.pdf"],
            at,
        );
        let doi_b = add(
            &mut store,
            "A study of many other things",
            "Ng",
            Some(2020),
            &["/l/h.pdf"],
            at,
        );
        get_mut(&mut store, &doi_a).ids.doi = Some("10.1/one".into());
        get_mut(&mut store, &doi_b).ids.doi = Some("10.1/two".into());
        // Different arXiv ids are different works too.
        let ax_a = add(
            &mut store,
            "Yet another title of words",
            "Li",
            Some(2020),
            &["/l/i.pdf"],
            at,
        );
        let ax_b = add(
            &mut store,
            "Yet another title of words",
            "Li",
            Some(2020),
            &["/l/j.pdf"],
            at,
        );
        get_mut(&mut store, &ax_a).ids.arxiv = Some("2001.00001".into());
        get_mut(&mut store, &ax_b).ids.arxiv = Some("2001.00002".into());

        assert!(store.auto_merge_all());
        let survivors: Vec<&str> = store.papers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(survivors.len(), 9, "only a and b merge");
        let kept = if survivors.contains(&a.as_str()) {
            &a
        } else {
            &b
        };
        assert!(!survivors.contains(&(if kept == &a { &b } else { &a }).as_str()));
        assert_eq!(
            get(&store, kept).auto_merge_reasons,
            ["same title, first author and year"]
        );
        for id in [
            &other_author,
            &far,
            &short_a,
            &short_b,
            &doi_a,
            &doi_b,
            &ax_a,
            &ax_b,
        ] {
            assert!(store.index_of(id).is_ok());
        }
        // Idempotent.
        assert!(!store.auto_merge_all());
    }

    #[test]
    fn dismissed_pairs_stay_apart_directly_and_through_a_third() {
        let mut store = store();
        let at = "2026-01-01T00:00:00Z";
        let a = add(&mut store, "A", "", None, &["/l/a.pdf"], at);
        let b = add(&mut store, "B", "", None, &["/l/b.pdf"], at);
        let c = add(&mut store, "C", "", None, &["/l/c.pdf"], at);
        for id in [&a, &b, &c] {
            get_mut(&mut store, id).ids.doi = Some("10.1/same".into());
        }
        store.mark_not_duplicates(&[a.clone(), c.clone()]).unwrap();
        store.auto_merge_all();
        // a and b merge (or b and c); never a with c, through b or not.
        assert_eq!(store.papers.len(), 2);
        let holder = |id: &str| {
            store
                .papers
                .iter()
                .position(|p| p.id == id || p.merged_from.iter().any(|s| s.id == id))
                .unwrap()
        };
        assert_ne!(holder(&a), holder(&c));
        // A non-item never merges either.
        let d = add(&mut store, "D", "", None, &["/l/d.pdf"], at);
        get_mut(&mut store, &d).ids.doi = Some("10.1/same".into());
        get_mut(&mut store, &d).paper_override = Some(false);
        assert!(!store.auto_merge_all());
        assert_eq!(store.papers.len(), 3);
    }

    #[test]
    fn the_keeper_has_projects_then_a_pinned_key_then_edits_then_files_then_age() {
        let mut store = store();
        let pair =
            |store: &mut PaperStore, files_a: &[&str], files_b: &[&str], at_a: &str, at_b: &str| {
                let a = add(store, "A", "", None, files_a, at_a);
                let b = add(store, "B", "", None, files_b, at_b);
                for id in [&a, &b] {
                    get_mut(store, id).ids.doi = Some(format!("10.1/{}", files_a[0]));
                }
                (a, b)
            };
        let early = "2026-01-01T00:00:00Z";
        let late = "2026-02-01T00:00:00Z";
        let survivor = |store: &PaperStore, a: &str, b: &str| {
            assert!(store.index_of(a).is_err() || store.index_of(b).is_err());
            if store.index_of(a).is_ok() {
                a.to_owned()
            } else {
                b.to_owned()
            }
        };

        // Age, then id.
        let (a, b) = pair(&mut store, &["/1a"], &["/1b"], late, early);
        store.auto_merge_all();
        assert_eq!(survivor(&store, &a, &b), b);
        // Files beat age.
        let (a, b) = pair(&mut store, &["/2a", "/2c"], &["/2b"], late, early);
        store.auto_merge_all();
        assert_eq!(survivor(&store, &a, &b), a);
        // User edits beat files.
        let (a, b) = pair(&mut store, &["/3a", "/3c"], &["/3b"], early, early);
        get_mut(&mut store, &b).meta.venue = field("V".into(), MetaSource::User);
        store.auto_merge_all();
        assert_eq!(survivor(&store, &a, &b), b);
        // A pinned key beats edits, and stays the key.
        let (a, b) = pair(&mut store, &["/4a"], &["/4b"], early, early);
        get_mut(&mut store, &a).meta.venue = field("V".into(), MetaSource::User);
        get_mut(&mut store, &b).cite_key = "pinned2026".into();
        get_mut(&mut store, &b).cite_key_pinned = true;
        store.auto_merge_all();
        assert_eq!(survivor(&store, &a, &b), b);
        assert_eq!(get(&store, &b).cite_key, "pinned2026");
        // Memberships beat everything; the other pinned key is an alias.
        let (a, b) = pair(&mut store, &["/5a"], &["/5b"], late, early);
        get_mut(&mut store, &b).cite_key = "bkey2026".into();
        get_mut(&mut store, &b).cite_key_pinned = true;
        get_mut(&mut store, &a).cite_key = "akey2026".into();
        get_mut(&mut store, &a).cite_key_pinned = true;
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&a, &project, true).unwrap();
        store.auto_merge_all();
        assert_eq!(survivor(&store, &a, &b), a);
        assert_eq!(get(&store, &a).cite_key, "akey2026");
        assert_eq!(get(&store, &a).alias_keys, ["bkey2026"]);
    }

    #[test]
    fn the_load_sweep_merges_once_and_split_keeps_them_apart() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = PaperStore::load(dir.path().to_path_buf());
        let at = "2026-01-01T00:00:00Z";
        let a = add(&mut store, "A", "", None, &["/l/a.pdf"], at);
        let b = add(
            &mut store,
            "B",
            "",
            None,
            &["/l/b.pdf"],
            "2026-01-02T00:00:00Z",
        );
        for id in [&a, &b] {
            get_mut(&mut store, id).ids.arxiv = Some("2606.13614".into());
        }
        let before: Vec<serde_json::Value> = [&a, &b]
            .iter()
            .map(|id| {
                let mut value = serde_json::to_value(get(&store, id)).unwrap();
                value.as_object_mut().unwrap().remove("updatedAt");
                value
            })
            .collect();
        store.save().unwrap();

        let mut loaded = PaperStore::load(dir.path().to_path_buf());
        assert_eq!(loaded.papers.len(), 1, "merged on load");
        assert_eq!(loaded.papers[0].id, a);
        assert_eq!(
            loaded.papers[0].auto_merge_reasons,
            ["same arXiv id 2606.13614"]
        );
        let again = PaperStore::load(dir.path().to_path_buf());
        assert_eq!(again.papers.len(), 1, "saved, and idempotent");

        // Split: both back as they were, now known not to be duplicates.
        loaded.unmerge(&a).unwrap();
        assert_eq!(loaded.papers.len(), 2);
        for (id, was) in [&a, &b].iter().zip(&before) {
            let mut value = serde_json::to_value(get(&loaded, id)).unwrap();
            let object = value.as_object_mut().unwrap();
            object.remove("updatedAt");
            object.remove("notDuplicates");
            assert_eq!(&value, was);
        }
        assert_eq!(get(&loaded, &a).not_duplicates, std::slice::from_ref(&b));
        assert!(!loaded.auto_merge_all());
        loaded.save().unwrap();
        let reloaded = PaperStore::load(dir.path().to_path_buf());
        assert_eq!(reloaded.papers.len(), 2, "the sweep does not re-merge");
    }

    #[test]
    fn a_read_only_store_never_merges() {
        let mut store = store();
        let a = add(&mut store, "A", "", None, &["/l/a.pdf"], "x");
        let b = add(&mut store, "B", "", None, &["/l/b.pdf"], "y");
        for id in [&a, &b] {
            get_mut(&mut store, id).ids.doi = Some("10.1/x".into());
        }
        store.block_writes_for_tests();
        assert!(!store.auto_merge_all());
        assert_eq!(store.papers.len(), 2);
    }

    #[test]
    fn the_main_file_is_chosen_and_kept_across_rescans() {
        let mut store = store();
        store.reconcile_file(&entry("/l/a.pdf", Some("2401.00009")));
        store.reconcile_file(&entry("/l/b.pdf", Some("2401.00009")));
        assert_eq!(store.papers.len(), 1);
        let id = store.papers[0].id.clone();
        assert!(store.set_primary_file(&id, "/l/nope.pdf").is_err());
        let dto = store.set_primary_file(&id, "/l/b.pdf").unwrap();
        assert_eq!(dto.files[0].path, "/l/b.pdf");
        // Moved away and back: still the main one.
        store.mark_missing(&HashSet::new());
        store.reconcile_file(&entry("/l/a.pdf", Some("2401.00009")));
        let mut moved = entry("/l/moved/b.pdf", Some("2401.00009"));
        moved.fingerprint = Some("/l/b.pdf".into());
        store.reconcile_file(&moved);
        assert_eq!(paths(&store, &id), ["/l/moved/b.pdf", "/l/a.pdf"]);
        // A file detached and reattached comes back first too.
        get_mut(&mut store, &id)
            .files
            .retain(|f| f.path != "/l/moved/b.pdf");
        store.reconcile_file(&moved);
        assert_eq!(paths(&store, &id)[0], "/l/moved/b.pdf");
        // And it survives a save and a load.
        store.save().unwrap();
        assert_eq!(
            get(&store, &id).primary_path.as_deref(),
            Some("/l/moved/b.pdf")
        );
    }
}

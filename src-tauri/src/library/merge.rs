//! Duplicates, merging and unmerging (projects.md sec. 2, identity).
//!
//! Two records can be one work: the arXiv preprint and the journal version
//! scanned before either knew of the other, or the same file saved twice
//! with different bytes. `duplicates` suggests such groups; `merge_papers`
//! folds records into one, keeping each merged record whole as a snapshot
//! on the survivor so `unmerge` can split them back.
//!
//! **Duplicate rules**, strongest first (the group's reason is its strongest
//! pair's): a shared DOI or arXiv id, aliases included ("same DOI", "same
//! arXiv id", or "arXiv X is the preprint of D" when only one side has a
//! journal DOI); an arXiv id the lookup cache says was published as a DOI
//! another record holds; an identifier citing papers print for one record
//! that the other holds ("cited as the same DOI by 3 papers",
//! `consensus::CitedAs`); equal normalized titles of four words or more with
//! years at most one apart or unknown ("same title"); a near-identical title
//! (`titlematch::judge` Near) with known years at most one apart; nearly
//! the same reference list (`versions`, weakest: "nearly the same 48
//! references"), only suggested, never merged on its own. Pairs the
//! user dismissed (`not_duplicates`) are never joined, directly or through a
//! third record. Groups are disjoint, members and groups in list order.

use super::citekey;
use super::papers::{MergeTrace, PaperRecord};
use super::store::{now_rfc3339, PaperStore};
use super::titlematch::{self, MatchQuality};
use super::types::{DuplicateGroupDto, MetaField, PaperDto, PaperMeta};
use crate::engine::types::CitationIdDto;
use crate::errors::{AppError, AppResult};
use std::collections::{HashMap, HashSet};

/// A title needs this many words before it can make two records one.
const MIN_TITLE_WORDS: usize = 4;

/// Disjoint groups over `items`, each with the reason of its strongest pair.
struct Groups<'a> {
    items: &'a [&'a PaperRecord],
    parent: Vec<usize>,
    members: Vec<Vec<usize>>,
    /// Per root: (strength, reason); lower strength is stronger.
    reason: Vec<Option<(u8, String)>>,
}

impl<'a> Groups<'a> {
    fn new(items: &'a [&'a PaperRecord]) -> Self {
        Self {
            items,
            parent: (0..items.len()).collect(),
            members: (0..items.len()).map(|i| vec![i]).collect(),
            reason: vec![None; items.len()],
        }
    }

    fn find(&mut self, mut at: usize) -> usize {
        while self.parent[at] != at {
            self.parent[at] = self.parent[self.parent[at]];
            at = self.parent[at];
        }
        at
    }

    fn dismissed(&self, a: usize, b: usize) -> bool {
        let (a, b) = (self.items[a], self.items[b]);
        a.not_duplicates.contains(&b.id) || b.not_duplicates.contains(&a.id)
    }

    /// Join the groups of `a` and `b`, unless the user said some member of
    /// one is not a duplicate of some member of the other.
    fn join(&mut self, a: usize, b: usize, strength: u8, reason: impl FnOnce() -> String) {
        if a == b {
            return;
        }
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let refused = self.members[ra]
                .iter()
                .any(|&x| self.members[rb].iter().any(|&y| self.dismissed(x, y)));
            if refused {
                return;
            }
        } else if self.dismissed(a, b) {
            return;
        }
        let (keep, gone) = if ra <= rb { (ra, rb) } else { (rb, ra) };
        if keep != gone {
            self.parent[gone] = keep;
            let moved = std::mem::take(&mut self.members[gone]);
            self.members[keep].extend(moved);
            let other = self.reason[gone].take();
            if other
                .as_ref()
                .is_some_and(|(s, _)| self.reason[keep].as_ref().is_none_or(|(k, _)| s < k))
            {
                self.reason[keep] = other;
            }
        }
        if self.reason[keep]
            .as_ref()
            .is_none_or(|(current, _)| strength < *current)
        {
            self.reason[keep] = Some((strength, reason()));
        }
    }
}

fn is_arxiv_doi(doi: &str) -> bool {
    doi.starts_with("10.48550/")
}

fn journal_doi(paper: &PaperRecord) -> Option<&str> {
    paper.ids.doi.as_deref().filter(|doi| !is_arxiv_doi(doi))
}

fn years_close(a: Option<i32>, b: Option<i32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= 1,
        _ => true,
    }
}

/// Keep's field unless the merged record's comes from a strictly stronger
/// source; so a user value on either side survives, keep's on a tie.
fn merge_field<T: Clone>(keep: &mut Option<MetaField<T>>, gone: &Option<MetaField<T>>) {
    if let Some(gone) = gone {
        if keep
            .as_ref()
            .is_none_or(|kept| gone.source.rank() > kept.source.rank())
        {
            *keep = Some(gone.clone());
        }
    }
}

fn merge_meta(keep: &mut PaperMeta, gone: &PaperMeta) {
    merge_field(&mut keep.title, &gone.title);
    merge_field(&mut keep.authors, &gone.authors);
    merge_field(&mut keep.year, &gone.year);
    merge_field(&mut keep.venue, &gone.venue);
    merge_field(&mut keep.volume, &gone.volume);
    merge_field(&mut keep.issue, &gone.issue);
    merge_field(&mut keep.pages, &gone.pages);
    merge_field(&mut keep.publisher, &gone.publisher);
    merge_field(&mut keep.abstract_, &gone.abstract_);
    merge_field(&mut keep.kind, &gone.kind);
}

/// Put a field back as it was before a merge, unless the user has set it
/// since.
fn restore_field<T: Clone + PartialEq>(
    current: &mut Option<MetaField<T>>,
    before: &Option<MetaField<T>>,
) {
    let user_since = current
        .as_ref()
        .is_some_and(|field| field.source == super::types::MetaSource::User)
        && current != before;
    if !user_since {
        *current = before.clone();
    }
}

fn restore_meta(current: &mut PaperMeta, before: &PaperMeta) {
    restore_field(&mut current.title, &before.title);
    restore_field(&mut current.authors, &before.authors);
    restore_field(&mut current.year, &before.year);
    restore_field(&mut current.venue, &before.venue);
    restore_field(&mut current.volume, &before.volume);
    restore_field(&mut current.issue, &before.issue);
    restore_field(&mut current.pages, &before.pages);
    restore_field(&mut current.publisher, &before.publisher);
    restore_field(&mut current.abstract_, &before.abstract_);
    restore_field(&mut current.kind, &before.kind);
}

fn restore_id(
    current: &mut Option<String>,
    current_source: &mut Option<super::types::MetaSource>,
    before: &Option<String>,
    before_source: Option<super::types::MetaSource>,
) {
    let user_since = *current_source == Some(super::types::MetaSource::User) && current != before;
    if !user_since {
        *current = before.clone();
        *current_source = before_source;
    }
}

impl PaperStore {
    /// Every arXiv id a record answers to, aliases included, for asking
    /// the lookup cache what each was published as.
    pub fn arxiv_ids(&self) -> Vec<String> {
        let mut out = Vec::new();
        for paper in &self.papers {
            for id in paper.citation_ids() {
                if let CitationIdDto::ArXiv(arxiv) = id {
                    out.push(arxiv);
                }
            }
        }
        out
    }

    /// Groups of items (`is_paper`) that look like one work (see the module
    /// doc). `preprint_dois` maps an arXiv id to the journal DOI the lookup
    /// cache's arXiv answer names for it.
    pub fn duplicates(&self, preprint_dois: &HashMap<String, String>) -> Vec<DuplicateGroupDto> {
        self.duplicates_with_versions(preprint_dois, &[])
    }

    /// `duplicates`, plus the version pairs `version_pairs` spotted from
    /// reference lists, as (id, id, shared references).
    pub fn duplicates_with_versions(
        &self,
        preprint_dois: &HashMap<String, String>,
        versions: &[(String, String, usize)],
    ) -> Vec<DuplicateGroupDto> {
        let items: Vec<&PaperRecord> =
            self.in_list_order(self.papers.iter().filter(|p| p.is_paper()).collect());
        let mut groups = Groups::new(&items);

        // Identifiers, aliases included.
        let mut holders: HashMap<CitationIdDto, Vec<usize>> = HashMap::new();
        let mut order: Vec<CitationIdDto> = Vec::new();
        for (at, item) in items.iter().enumerate() {
            for id in item.citation_ids() {
                let list = holders.entry(id.clone()).or_default();
                if list.is_empty() {
                    order.push(id);
                }
                if !list.contains(&at) {
                    list.push(at);
                }
            }
        }
        for id in &order {
            let list = &holders[id];
            for &other in &list[1..] {
                let first = list[0];
                match id {
                    CitationIdDto::Doi(_) => groups.join(first, other, 0, || "same DOI".into()),
                    CitationIdDto::ArXiv(arxiv) => {
                        let (a, b) = (journal_doi(items[first]), journal_doi(items[other]));
                        let reason = match (a, b) {
                            (Some(doi), None) | (None, Some(doi)) => {
                                format!("arXiv {arxiv} is the preprint of {doi}")
                            }
                            _ => "same arXiv id".to_owned(),
                        };
                        groups.join(first, other, 0, || reason);
                    }
                }
            }
        }

        // An arXiv id the lookup cache says was published as a DOI another
        // record holds.
        for (at, item) in items.iter().enumerate() {
            for id in item.citation_ids() {
                let CitationIdDto::ArXiv(arxiv) = &id else {
                    continue;
                };
                let Some(doi) = preprint_dois.get(arxiv).filter(|doi| !is_arxiv_doi(doi)) else {
                    continue;
                };
                for &other in holders
                    .get(&CitationIdDto::Doi(doi.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    groups.join(at, other, 1, || {
                        format!("arXiv {arxiv} is the preprint of {doi}")
                    });
                }
            }
        }

        // An identifier citing papers print for one that the other holds.
        let position: HashMap<&str, usize> = items
            .iter()
            .enumerate()
            .map(|(at, item)| (item.id.as_str(), at))
            .collect();
        for (at, item) in items.iter().enumerate() {
            let Some(cited) = &item.citation_facts.cited_as else {
                continue;
            };
            if let Some(&other) = position.get(cited.paper_id.as_str()) {
                groups.join(at, other, 1, || cited.reason());
            }
        }

        // Titles.
        let titles: Vec<Option<String>> = items
            .iter()
            .map(|item| {
                let normalized = titlematch::normalize(item.title()?);
                (normalized.split(' ').count() >= MIN_TITLE_WORDS).then_some(normalized)
            })
            .collect();
        let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
        for (at, title) in titles.iter().enumerate() {
            if let Some(title) = title {
                by_title.entry(title.replace(' ', "")).or_default().push(at);
            }
        }
        let mut buckets: Vec<&Vec<usize>> = by_title.values().filter(|b| b.len() > 1).collect();
        buckets.sort_by_key(|bucket| bucket[0]);
        for bucket in buckets {
            for (i, &a) in bucket.iter().enumerate() {
                for &b in &bucket[i + 1..] {
                    if years_close(items[a].year(), items[b].year()) {
                        groups.join(a, b, 2, || "same title".into());
                    }
                }
            }
        }
        let mut by_year: HashMap<i32, Vec<usize>> = HashMap::new();
        for (at, item) in items.iter().enumerate() {
            if let (Some(year), Some(_)) = (item.year(), &titles[at]) {
                by_year.entry(year).or_default().push(at);
            }
        }
        for (a, item) in items.iter().enumerate() {
            let (Some(year), Some(title)) = (item.year(), titles[a].as_deref()) else {
                continue;
            };
            for other_year in [year, year + 1] {
                for &b in by_year.get(&other_year).map(Vec::as_slice).unwrap_or(&[]) {
                    if b == a || (other_year == year && b < a) {
                        continue;
                    }
                    let other = titles[b].as_deref().unwrap_or_default();
                    if titlematch::judge(title, Some(year), other, Some(year)) == MatchQuality::Near
                    {
                        groups.join(a, b, 3, || format!("near-identical title, {year}"));
                    }
                }
            }
        }

        // Nearly the same reference list.
        for (a, b, shared) in versions {
            if let (Some(&a), Some(&b)) = (position.get(a.as_str()), position.get(b.as_str())) {
                groups.join(a, b, 4, || format!("nearly the same {shared} references"));
            }
        }

        let mut out: Vec<(usize, DuplicateGroupDto)> = Vec::new();
        for root in 0..items.len() {
            if groups.find(root) != root || groups.members[root].len() < 2 {
                continue;
            }
            let mut members = groups.members[root].clone();
            members.sort_unstable();
            let reason = groups.reason[root]
                .as_ref()
                .map(|(_, reason)| reason.clone())
                .unwrap_or_default();
            out.push((
                members[0],
                DuplicateGroupDto {
                    paper_ids: members.iter().map(|&m| items[m].id.clone()).collect(),
                    reason,
                },
            ));
        }
        out.sort_by_key(|(first, _)| *first);
        out.into_iter().map(|(_, group)| group).collect()
    }

    /// The user says these records are not duplicates of one another: each
    /// pair is left out of `duplicates` from now on.
    pub fn mark_not_duplicates(&mut self, paper_ids: &[String]) -> AppResult<()> {
        self.ensure_writable()?;
        for id in paper_ids {
            self.index_of(id)?;
        }
        for id in paper_ids {
            let index = self.index_of(id)?;
            let paper = &mut self.papers[index];
            let mut changed = false;
            for other in paper_ids {
                if other != id && !paper.not_duplicates.contains(other) {
                    paper.not_duplicates.push(other.clone());
                    changed = true;
                }
            }
            if changed {
                paper.updated_at = now_rfc3339();
            }
        }
        Ok(())
    }

    /// Fold `merge_ids` into `keep_id`, in that order. Refused for an
    /// unknown id, `keep_id` among `merge_ids`, or a read-only store.
    ///
    /// Each merged record is kept whole on `keep.merged_from` (its own
    /// snapshots moved up beside it), then: its files are appended (one per
    /// path); keep joins every project it was in, at its place, and it
    /// leaves them; each metadata field goes to the stronger source, keep's
    /// on a tie; identifiers keep lacks are filled with their source and a
    /// conflicting DOI or arXiv id becomes an alias; keep adopts its pinned
    /// key when keep's is not pinned, and any other pinned key becomes an
    /// alias key; candidates and declined matches are unioned; keep's say
    /// on paperhood wins, else the merged one's. The merged records go.
    pub fn merge_papers(&mut self, keep_id: &str, merge_ids: &[String]) -> AppResult<PaperDto> {
        self.ensure_writable()?;
        self.index_of(keep_id)?;
        let mut merge: Vec<&String> = Vec::new();
        for id in merge_ids {
            if id == keep_id {
                return Err(AppError::Internal(
                    "a paper cannot be merged into itself".into(),
                ));
            }
            self.index_of(id)?;
            if !merge.contains(&id) {
                merge.push(id);
            }
        }
        for id in merge {
            self.merge_one(keep_id, id)?;
        }
        self.mark_dirty();
        self.paper_dto(keep_id)
    }

    fn merge_one(&mut self, keep_id: &str, gone_id: &str) -> AppResult<()> {
        let gone_index = self.index_of(gone_id)?;
        let mut gone = self.papers.remove(gone_index);
        let keep_index = self.index_of(keep_id)?;

        let mut was_in = Vec::new();
        let mut added = Vec::new();
        for project in &mut self.projects {
            let Some(at) = project.paper_ids.iter().position(|id| id == gone_id) else {
                continue;
            };
            was_in.push(project.id.clone());
            if project.paper_ids.iter().any(|id| id == keep_id) {
                project.paper_ids.remove(at);
            } else {
                project.paper_ids[at] = keep_id.to_owned();
                added.push(project.id.clone());
            }
            project.paper_ids.retain({
                let mut seen = HashSet::new();
                move |id| seen.insert(id.clone())
            });
        }
        for paper in &mut self.papers {
            if paper.id != keep_id
                && paper.not_duplicates.iter().any(|id| id == gone_id)
                && !paper.not_duplicates.iter().any(|id| id == keep_id)
            {
                paper.not_duplicates.push(keep_id.to_owned());
            }
        }

        let taken: HashSet<String> = self
            .papers
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != keep_index)
            .flat_map(|(_, paper)| {
                std::iter::once(paper.cite_key.clone()).chain(paper.alias_keys.iter().cloned())
            })
            .collect();
        let keep = &mut self.papers[keep_index];
        let trace = MergeTrace {
            into: keep_id.to_owned(),
            projects: was_in,
            added_target_to: added,
            target_meta: keep.meta.clone(),
            target_ids: keep.ids.clone(),
            target_cite_key: keep.cite_key.clone(),
            target_cite_key_pinned: keep.cite_key_pinned,
            target_alias_ids: keep.alias_ids.clone(),
            target_alias_keys: keep.alias_keys.clone(),
            auto: false,
        };
        let nested = std::mem::take(&mut gone.merged_from);
        let mut snapshot = gone.clone();
        snapshot.merge_trace = Some(trace);
        keep.merged_from.push(snapshot);
        keep.merged_from.extend(nested);

        for file in gone.files {
            if !keep.files.iter().any(|kept| kept.path == file.path) {
                keep.files.push(file);
            }
        }
        // The user's main file, from either side, keep's first; else the
        // rules choose among all of them (`mainfile`).
        if keep.primary_path.is_none() {
            keep.primary_path = gone.primary_path;
        }
        keep.order_files();
        let base_before = citekey::base_key(&keep.meta);
        merge_meta(&mut keep.meta, &gone.meta);

        let ids = &mut keep.ids;
        let mut aliases: Vec<CitationIdDto> = Vec::new();
        match (&ids.doi, gone.ids.doi) {
            (None, Some(doi)) => {
                ids.doi = Some(doi);
                ids.doi_source = gone.ids.doi_source;
            }
            (Some(kept), Some(doi)) if *kept != doi => aliases.push(CitationIdDto::Doi(doi)),
            _ => {}
        }
        match (&ids.arxiv, gone.ids.arxiv) {
            (None, Some(arxiv)) => {
                ids.arxiv = Some(arxiv);
                ids.arxiv_source = gone.ids.arxiv_source;
            }
            (Some(kept), Some(arxiv)) if *kept != arxiv => {
                aliases.push(CitationIdDto::ArXiv(arxiv))
            }
            _ => {}
        }
        if ids.isbn.is_none() && gone.ids.isbn.is_some() {
            ids.isbn = gone.ids.isbn;
            ids.isbn_source = gone.ids.isbn_source;
        }
        aliases.extend(gone.alias_ids);
        for alias in aliases {
            let own = match &alias {
                CitationIdDto::Doi(doi) => keep.ids.doi.as_ref() == Some(doi),
                CitationIdDto::ArXiv(arxiv) => keep.ids.arxiv.as_ref() == Some(arxiv),
            };
            if !own && !keep.alias_ids.contains(&alias) {
                keep.alias_ids.push(alias);
            }
        }

        let mut pinned: Vec<String> = Vec::new();
        if gone.cite_key_pinned && !gone.cite_key.is_empty() {
            pinned.push(gone.cite_key.clone());
        }
        pinned.extend(gone.alias_keys);
        if !keep.cite_key_pinned && gone.cite_key_pinned && !gone.cite_key.is_empty() {
            keep.cite_key = gone.cite_key.clone();
            keep.cite_key_pinned = true;
        }
        for key in pinned {
            if key != keep.cite_key && !keep.alias_keys.contains(&key) {
                keep.alias_keys.push(key);
            }
        }
        if !keep.cite_key_pinned && citekey::base_key(&keep.meta) != base_before {
            let own_aliases = keep.alias_keys.clone();
            keep.cite_key = citekey::generate(&keep.meta, |key| {
                taken.contains(key) || own_aliases.iter().any(|alias| alias == key)
            });
        }

        for candidate in gone.candidates {
            if !keep.candidates.contains(&candidate) {
                keep.candidates.push(candidate);
            }
        }
        for declined in gone.declined {
            if !keep.declined.contains(&declined) {
                keep.declined.push(declined);
            }
        }
        if keep.paper_override.is_none() {
            keep.paper_override = gone.paper_override;
        }
        if keep.arxiv_category.is_none() {
            keep.arxiv_category = gone.arxiv_category;
        }
        for id in gone.not_duplicates {
            if id != keep.id && !keep.not_duplicates.contains(&id) {
                keep.not_duplicates.push(id);
            }
        }
        keep.updated_at = now_rfc3339();
        Ok(())
    }

    /// Split a merged record back: every snapshot becomes its own record
    /// again (same id), takes its files back from the record it was merged
    /// into, and rejoins the projects it was in (those that still exist).
    /// Each record merged into is put back as it was before its first
    /// merge - metadata and identifiers the user has not set since, its
    /// aliases, its key unless it was pinned to its own since - and leaves
    /// the projects it joined only through a merge. Fields its files supply
    /// are re-derived by the next scan. Returns the restored records, then
    /// `paper_id`'s.
    pub fn unmerge(&mut self, paper_id: &str) -> AppResult<Vec<PaperDto>> {
        self.ensure_writable()?;
        let keep_index = self.index_of(paper_id)?;
        let snapshots = std::mem::take(&mut self.papers[keep_index].merged_from);
        if snapshots.is_empty() {
            return Ok(vec![self.paper_dto(paper_id)?]);
        }
        let mut restored: Vec<(String, HashSet<String>, Option<MergeTrace>)> = Vec::new();
        for mut snapshot in snapshots {
            let trace = snapshot.merge_trace.take();
            if self.papers.iter().any(|paper| paper.id == snapshot.id) {
                continue;
            }
            let paths = snapshot.files.iter().map(|f| f.path.clone()).collect();
            restored.push((snapshot.id.clone(), paths, trace));
            snapshot.updated_at = now_rfc3339();
            self.papers.push(snapshot);
        }
        let restored_ids: HashSet<String> = restored.iter().map(|(id, _, _)| id.clone()).collect();

        // Each target as before its first merge.
        let mut seen: HashSet<String> = HashSet::new();
        for (_, _, trace) in &restored {
            let Some(trace) = trace else { continue };
            if !seen.insert(trace.into.clone()) {
                continue;
            }
            let Ok(target) = self.index_of(&trace.into) else {
                continue;
            };
            let paper = &mut self.papers[target];
            restore_meta(&mut paper.meta, &trace.target_meta);
            let before = &trace.target_ids;
            restore_id(
                &mut paper.ids.doi,
                &mut paper.ids.doi_source,
                &before.doi,
                before.doi_source,
            );
            restore_id(
                &mut paper.ids.arxiv,
                &mut paper.ids.arxiv_source,
                &before.arxiv,
                before.arxiv_source,
            );
            restore_id(
                &mut paper.ids.isbn,
                &mut paper.ids.isbn_source,
                &before.isbn,
                before.isbn_source,
            );
            paper.alias_ids = trace.target_alias_ids.clone();
            paper.alias_keys = trace.target_alias_keys.clone();
            let current = paper.cite_key.clone();
            let clash = self.papers.iter().enumerate().any(|(index, other)| {
                index != target
                    && restored_ids.contains(&other.id)
                    && (other.cite_key == current || other.alias_keys.contains(&current))
            });
            let paper = &self.papers[target];
            if clash || !paper.cite_key_pinned || current == trace.target_cite_key {
                let wanted = trace.target_cite_key.clone();
                let free = !wanted.is_empty()
                    && !self.papers.iter().enumerate().any(|(index, other)| {
                        index != target
                            && (other.cite_key == wanted || other.alias_keys.contains(&wanted))
                    });
                if free {
                    let paper = &mut self.papers[target];
                    paper.cite_key = wanted;
                    paper.cite_key_pinned = trace.target_cite_key_pinned;
                } else if clash {
                    let meta = paper.meta.clone();
                    let key = self.free_key(&meta, Some(target));
                    let paper = &mut self.papers[target];
                    paper.cite_key = key;
                    paper.cite_key_pinned = false;
                }
            }
        }

        // Files back to their records; projects as they were.
        for (id, paths, trace) in &restored {
            let Some(trace) = trace else { continue };
            if let Ok(target) = self.index_of(&trace.into) {
                let paper = &mut self.papers[target];
                paper.files.retain(|file| !paths.contains(&file.path));
                if paper
                    .primary_path
                    .as_ref()
                    .is_some_and(|primary| paths.contains(primary))
                {
                    paper.primary_path = None;
                }
                paper.order_files();
                paper.updated_at = now_rfc3339();
            }
            for project in &mut self.projects {
                if trace.added_target_to.contains(&project.id) {
                    project.paper_ids.retain(|member| *member != trace.into);
                }
                if trace.projects.contains(&project.id)
                    && !project.paper_ids.iter().any(|member| member == id)
                {
                    project.paper_ids.push(id.clone());
                }
            }
        }

        // Splitting an automatic merge says the records are not one work:
        // every record it joined stays apart from the others from now on.
        let mut auto_ids: Vec<String> = Vec::new();
        for (id, _, trace) in &restored {
            if let Some(trace) = trace.as_ref().filter(|trace| trace.auto) {
                for id in [id, &trace.into] {
                    if !auto_ids.contains(id) && self.index_of(id).is_ok() {
                        auto_ids.push(id.clone());
                    }
                }
            }
        }
        if auto_ids.len() > 1 {
            self.mark_not_duplicates(&auto_ids)?;
        }
        for (id, _, _) in &restored {
            let index = self.index_of(id)?;
            self.papers[index].auto_merge_reasons.clear();
        }
        let index = self.index_of(paper_id)?;
        self.papers[index].auto_merge_reasons.clear();

        // A restored record whose unpinned key someone took meanwhile gets
        // a fresh one.
        for (id, _, _) in &restored {
            let index = self.index_of(id)?;
            let key = self.papers[index].cite_key.clone();
            let taken = self.papers.iter().enumerate().any(|(other, paper)| {
                other != index && (paper.cite_key == key || paper.alias_keys.contains(&key))
            });
            if taken && !self.papers[index].cite_key_pinned {
                let meta = self.papers[index].meta.clone();
                let key = self.free_key(&meta, Some(index));
                self.papers[index].set_cite_key(key);
            }
        }

        self.mark_dirty();
        let mut out = Vec::new();
        for (id, _, _) in &restored {
            out.push(self.paper_dto(id)?);
        }
        out.push(self.paper_dto(paper_id)?);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::lookup::NetworkEdges;
    use crate::library::references::DocRefs;
    use crate::library::tests::indexed_entry;
    use crate::library::types::{MetaPatchDto, MetaSource, PaperFile};
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

    fn file(path: &str, fingerprint: &str) -> PaperFile {
        PaperFile {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            size: 100,
            mtime_ms: 1,
            fingerprint: Some(fingerprint.to_owned()),
            missing: false,
            ..Default::default()
        }
    }

    /// A paper with a title (from the first page), a year, and one file.
    fn add(store: &mut PaperStore, title: &str, year: Option<i32>, path: &str) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = field(title.to_owned(), MetaSource::FirstPage);
        paper.meta.year = year.and_then(|y| field(y, MetaSource::FirstPage));
        paper.files.push(file(path, path));
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

    /// What unmerge must give back: the record as JSON without its
    /// modification stamp, and its projects.
    fn essence(store: &PaperStore, id: &str) -> (serde_json::Value, Vec<String>) {
        let mut value = serde_json::to_value(get(store, id)).unwrap();
        value.as_object_mut().unwrap().remove("updatedAt");
        (value, store.project_ids_for(id))
    }

    fn entry_at(path: &str, fingerprint: &str) -> LibraryEntry {
        let mut entry = indexed_entry(Path::new(path), 1, 100);
        entry.fingerprint = Some(fingerprint.to_owned());
        entry
    }

    #[test]
    fn a_merge_moves_files_memberships_identifiers_and_metadata() {
        let mut store = store();
        let journal = add(
            &mut store,
            "Beyond linear response in thermodynamics",
            None,
            "/l/j.pdf",
        );
        let preprint = add(
            &mut store,
            "Beyond linear response in thermodynamics (preprint)",
            Some(2023),
            "/l/p.pdf",
        );
        {
            let j = get_mut(&mut store, &journal);
            j.ids.doi = Some("10.1103/physrevlett.133.057102".into());
            j.ids.doi_source = Some(MetaSource::Crossref);
            j.ids.arxiv = Some("2404.11111".into());
            j.ids.arxiv_source = Some(MetaSource::Crossref);
            j.meta.title = field("Beyond Linear Response".into(), MetaSource::Crossref);
            j.meta.venue = field("Phys. Rev. Lett.".into(), MetaSource::User);
        }
        {
            let p = get_mut(&mut store, &preprint);
            p.ids.arxiv = Some("2404.01286".into());
            p.ids.arxiv_source = Some(MetaSource::FirstPage);
            p.ids.isbn = Some("9780000000002".into());
            p.ids.isbn_source = Some(MetaSource::User);
            p.meta.authors = field(
                vec![super::super::types::Author {
                    family: "Zhong".into(),
                    given: "A.".into(),
                }],
                MetaSource::Arxiv,
            );
            p.meta.venue = field("arXiv".into(), MetaSource::Arxiv);
            p.meta.pages = field("1-10".into(), MetaSource::User);
        }
        let a = store.create_project("A").unwrap().id;
        let b = store.create_project("B").unwrap().id;
        store.set_membership(&preprint, &a, true).unwrap();
        store.set_membership(&journal, &b, true).unwrap();
        store.set_membership(&preprint, &b, true).unwrap();

        let dto = store
            .merge_papers(&journal, std::slice::from_ref(&preprint))
            .expect("merge");
        assert_eq!(dto.merged_count, 1);
        assert_eq!(store.papers.len(), 1);
        let keep = get(&store, &journal);
        let paths: Vec<&str> = keep.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["/l/j.pdf", "/l/p.pdf"]);
        assert_eq!(store.project_ids_for(&journal), [a.clone(), b.clone()]);
        assert!(store
            .projects
            .iter()
            .all(|p| !p.paper_ids.contains(&preprint)));
        assert_eq!(
            store.projects.iter().find(|p| p.id == b).unwrap().paper_ids,
            std::slice::from_ref(&journal),
            "once"
        );
        // Identifiers: keep's stay, a conflicting arXiv id is an alias, a
        // missing one is filled with its source.
        assert_eq!(keep.ids.arxiv.as_deref(), Some("2404.11111"));
        assert_eq!(keep.alias_ids, [CitationIdDto::ArXiv("2404.01286".into())]);
        assert_eq!(keep.ids.isbn.as_deref(), Some("9780000000002"));
        assert_eq!(keep.ids.isbn_source, Some(MetaSource::User));
        // Metadata: the stronger source, field by field; a user value on
        // either side survives.
        assert_eq!(keep.title(), Some("Beyond Linear Response"));
        assert_eq!(keep.authors()[0].family, "Zhong");
        assert_eq!(keep.year(), Some(2023));
        assert_eq!(keep.venue(), Some("Phys. Rev. Lett."));
        assert_eq!(
            keep.meta
                .pages
                .as_ref()
                .map(|f| (f.value.as_str(), f.source)),
            Some(("1-10", MetaSource::User))
        );

        // Citations of the preprint's arXiv id resolve to the merged record.
        let mut citing = PaperRecord::new();
        citing.files.push(file("/l/c.pdf", "c"));
        citing.cite_key = "citing".into();
        let citing_id = citing.id.clone();
        store.papers.push(citing);
        let mut entries = vec![entry_at("/l/c.pdf", "c")];
        entries[0].cites = Some(vec![CitationIdDto::ArXiv("2404.01286".into())]);
        let links = store
            .citation_links(
                &journal,
                &entries,
                &DocRefs::new(),
                &NetworkEdges::default(),
            )
            .unwrap();
        assert_eq!(links.cited_by.len(), 1);
        assert_eq!(links.cited_by[0].id, citing_id);
        let graph = store
            .citation_graph(None, &entries, &DocRefs::new(), &NetworkEdges::default())
            .unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.external, 0);
    }

    #[test]
    fn a_rescan_of_a_merged_away_file_attaches_to_the_survivor() {
        let mut store = store();
        let keep = add(
            &mut store,
            "Journal version of a work",
            Some(2024),
            "/l/j.pdf",
        );
        let gone = add(
            &mut store,
            "Preprint version of a work",
            Some(2023),
            "/l/p.pdf",
        );
        get_mut(&mut store, &keep).ids.arxiv = Some("2404.11111".into());
        get_mut(&mut store, &gone).ids.arxiv = Some("2404.01286".into());
        store
            .merge_papers(&keep, std::slice::from_ref(&gone))
            .unwrap();
        let count = store.papers.len();

        // By path: the same file, unchanged.
        store.reconcile_file(&entry_at("/l/p.pdf", "/l/p.pdf"));
        // By fingerprint: moved.
        store.reconcile_file(&entry_at("/l/moved/p.pdf", "/l/p.pdf"));
        // By alias: new bytes, new place, the preprint's arXiv id.
        let mut fresh = entry_at("/l/p-v2.pdf", "v2");
        fresh.arxiv = Some("2404.01286v2".into());
        store.reconcile_file(&fresh);

        assert_eq!(store.papers.len(), count, "no new record");
        let record = get(&store, &keep);
        let paths: Vec<&str> = record.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/l/moved/p.pdf"));
        assert!(paths.contains(&"/l/p-v2.pdf"));
        assert_eq!(
            record.ids.arxiv.as_deref(),
            Some("2404.11111"),
            "the alias stays an alias"
        );
    }

    #[test]
    fn a_pinned_key_is_adopted_and_the_others_are_kept_as_aliases() {
        let mut store = store();
        let keep = add(
            &mut store,
            "Keep this one around please",
            Some(2020),
            "/l/k.pdf",
        );
        let a = add(
            &mut store,
            "First merged record here",
            Some(2021),
            "/l/a.pdf",
        );
        let b = add(
            &mut store,
            "Second merged record here",
            Some(2022),
            "/l/b.pdf",
        );
        let other = add(
            &mut store,
            "Somebody else entirely now",
            Some(2019),
            "/l/o.pdf",
        );
        let keep_key = get(&store, &keep).cite_key.clone();
        for (id, key) in [(&a, "alpha2021"), (&b, "beta2022")] {
            let record = get_mut(&mut store, id);
            record.cite_key = key.into();
            record.cite_key_pinned = true;
        }
        store.merge_papers(&keep, &[a.clone(), b.clone()]).unwrap();
        let record = get(&store, &keep);
        assert_eq!(record.cite_key, "alpha2021");
        assert!(record.cite_key_pinned);
        assert_eq!(record.alias_keys, ["beta2022"]);
        // Still taken.
        let clash = store.apply_patch(
            &other,
            MetaPatchDto {
                cite_key: Some(Some("beta2022".into())),
                ..MetaPatchDto::default()
            },
        );
        assert!(clash.is_err());
        // BibTeX names the record by its key only.
        let (bib, _) = store.bibtex_for(std::slice::from_ref(&keep)).unwrap();
        assert!(bib.contains("alpha2021") && !bib.contains("beta2022"));

        store.unmerge(&keep).unwrap();
        assert_eq!(get(&store, &keep).cite_key, keep_key);
        assert!(get(&store, &keep).alias_keys.is_empty());
        assert_eq!(get(&store, &a).cite_key, "alpha2021");
        assert_eq!(get(&store, &b).cite_key, "beta2022");
    }

    #[test]
    fn unmerge_restores_exactly_and_nested_merges_flatten() {
        let mut store = store();
        let a = add(&mut store, "Record a of three", Some(2020), "/l/a.pdf");
        let b = add(&mut store, "Record b of three", Some(2021), "/l/b.pdf");
        let c = add(&mut store, "Record c of three", None, "/l/c.pdf");
        get_mut(&mut store, &a).ids.doi = Some("10.1/a".into());
        get_mut(&mut store, &b).ids.doi = Some("10.1/b".into());
        get_mut(&mut store, &c).ids.arxiv = Some("2401.00003".into());
        get_mut(&mut store, &c).meta.venue = field("Venue C".into(), MetaSource::Crossref);
        let p = store.create_project("P").unwrap().id;
        let q = store.create_project("Q").unwrap().id;
        store.set_membership(&b, &p, true).unwrap();
        store.set_membership(&c, &q, true).unwrap();
        store.set_membership(&a, &q, true).unwrap();
        let before: Vec<_> = [&a, &b, &c].iter().map(|id| essence(&store, id)).collect();

        store.merge_papers(&b, std::slice::from_ref(&c)).unwrap();
        assert_eq!(get(&store, &b).merged_from.len(), 1);
        assert_eq!(store.project_ids_for(&b), [p.clone(), q.clone()]);
        store.merge_papers(&a, std::slice::from_ref(&b)).unwrap();
        let record = get(&store, &a);
        assert_eq!(record.merged_from.len(), 2, "flattened");
        assert!(record.merged_from.iter().all(|s| s.merged_from.is_empty()));
        assert_eq!(store.paper_dto(&a).unwrap().merged_count, 2);
        assert_eq!(record.files.len(), 3);
        assert_eq!(record.ids.arxiv.as_deref(), Some("2401.00003"));
        assert_eq!(record.alias_ids, [CitationIdDto::Doi("10.1/b".into())]);
        assert_eq!(record.venue(), Some("Venue C"));

        let restored = store.unmerge(&a).unwrap();
        let ids: Vec<&str> = restored.iter().map(|dto| dto.id.as_str()).collect();
        assert_eq!(ids, [b.as_str(), c.as_str(), a.as_str()]);
        assert_eq!(store.papers.len(), 3);
        let after: Vec<_> = [&a, &b, &c].iter().map(|id| essence(&store, id)).collect();
        assert_eq!(after, before);
    }

    #[test]
    fn merging_is_refused_for_unknown_ids_and_into_itself() {
        let mut store = store();
        let a = add(&mut store, "One", None, "/l/a.pdf");
        let b = add(&mut store, "Two", None, "/l/b.pdf");
        assert!(store.merge_papers(&a, &["nope".into()]).is_err());
        assert!(store
            .merge_papers("nope", std::slice::from_ref(&b))
            .is_err());
        assert!(store.merge_papers(&a, &[b.clone(), a.clone()]).is_err());
        assert_eq!(store.papers.len(), 2);
    }

    #[test]
    fn duplicates_are_found_for_each_reason_and_dismissed_pairs_are_not() {
        let mut store = store();
        let doi_a = add(&mut store, "Alpha one", Some(2001), "/l/1.pdf");
        let doi_b = add(&mut store, "Alpha two", Some(2005), "/l/2.pdf");
        for id in [&doi_a, &doi_b] {
            get_mut(&mut store, id).ids.doi = Some("10.1/same".into());
        }
        let pre = add(&mut store, "Beta one", Some(2010), "/l/3.pdf");
        let pub_ = add(&mut store, "Beta two", Some(2011), "/l/4.pdf");
        get_mut(&mut store, &pre).ids.arxiv = Some("2404.01286".into());
        get_mut(&mut store, &pub_).ids.arxiv = Some("2404.01286".into());
        get_mut(&mut store, &pub_).ids.doi = Some("10.1103/x.1.2".into());
        let cached_pre = add(&mut store, "Gamma one", Some(2012), "/l/5.pdf");
        let cached_pub = add(&mut store, "Gamma two", Some(2013), "/l/6.pdf");
        get_mut(&mut store, &cached_pre).ids.arxiv = Some("2301.00001".into());
        get_mut(&mut store, &cached_pub).ids.doi = Some("10.1/journal".into());
        let same_a = add(
            &mut store,
            "A study of things in many places",
            Some(2018),
            "/l/7.pdf",
        );
        let same_b = add(
            &mut store,
            "A Study of Things in Many Places.",
            None,
            "/l/8.pdf",
        );
        let same_c = add(
            &mut store,
            "A study of things in many places",
            Some(2019),
            "/l/9.pdf",
        );
        let near_a = add(
            &mut store,
            "Thermodynamic geometry of minimum dissipation driven barrier crossing",
            Some(2016),
            "/l/10.pdf",
        );
        let near_b = add(
            &mut store,
            "The thermodynamic geometry of minimum dissipation driven barrier crossing",
            Some(2017),
            "/l/11.pdf",
        );
        let short_a = add(&mut store, "Optimal transport", None, "/l/12.pdf");
        let _short_b = add(&mut store, "Optimal transport", None, "/l/13.pdf");
        let not_paper = add(&mut store, "Alpha three", None, "/l/14.pdf");
        get_mut(&mut store, &not_paper).ids.doi = Some("10.1/same".into());
        get_mut(&mut store, &not_paper).paper_override = Some(false);
        let _ = short_a;

        let preprints: HashMap<String, String> =
            [("2301.00001".to_owned(), "10.1/journal".to_owned())].into();
        let groups = store.duplicates(&preprints);
        let find = |id: &str| {
            groups
                .iter()
                .find(|g| g.paper_ids.iter().any(|p| p == id))
                .cloned()
        };
        let sorted = |ids: &[&String]| {
            let mut ids: Vec<String> = ids.iter().map(|s| (*s).clone()).collect();
            ids.sort();
            ids
        };
        let members = |id: &str| {
            let mut ids = find(id).unwrap().paper_ids;
            ids.sort();
            ids
        };
        assert_eq!(find(&doi_a).unwrap().reason, "same DOI");
        assert_eq!(
            members(&doi_a),
            sorted(&[&doi_a, &doi_b]),
            "never the non-paper"
        );
        assert_eq!(
            find(&pre).unwrap().reason,
            "arXiv 2404.01286 is the preprint of 10.1103/x.1.2"
        );
        assert_eq!(
            find(&cached_pre).unwrap().reason,
            "arXiv 2301.00001 is the preprint of 10.1/journal"
        );
        assert_eq!(members(&cached_pre), sorted(&[&cached_pre, &cached_pub]));
        assert_eq!(find(&same_a).unwrap().reason, "same title");
        // An unknown year joins either.
        assert_eq!(members(&same_a), sorted(&[&same_a, &same_b, &same_c]));
        assert_eq!(find(&near_a).unwrap().reason, "near-identical title, 2016");
        assert_eq!(members(&near_a), sorted(&[&near_a, &near_b]));
        assert!(find(&short_a).is_none(), "too short a title to say");
        assert_eq!(groups.len(), 5);
        // Members in list order, deterministic.
        assert_eq!(store.duplicates(&preprints), groups);

        // Dismissed: never paired again, directly or through a third.
        store
            .mark_not_duplicates(&[same_a.clone(), same_c.clone()])
            .unwrap();
        store
            .mark_not_duplicates(&[near_a.clone(), near_b.clone()])
            .unwrap();
        let groups = store.duplicates(&preprints);
        let find = |id: &str| groups.iter().find(|g| g.paper_ids.iter().any(|p| p == id));
        let mut left = find(&same_a).unwrap().paper_ids.clone();
        left.sort();
        assert_eq!(
            left,
            sorted(&[&same_a, &same_b]),
            "c cannot join a through b"
        );
        assert!(find(&same_c).is_none());
        assert!(find(&near_a).is_none());
        assert!(store.mark_not_duplicates(&["nope".into()]).is_err());
    }
}

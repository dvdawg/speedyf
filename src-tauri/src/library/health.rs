//! Library health: what SpeedyF suspects about the items, in one place
//! (projects.md sec. 5, "Health"). Every detector reads what the store and
//! the scan already hold; nothing here goes online or reads a file. The
//! duplicate groups are handed in, since the command computes them once
//! per library change for the duplicate line as well.
//!
//! A finding the user set aside (`review::IgnoredFinding`) is never
//! returned again; a group with nothing left is left out.

use super::papers::{on_unmounted_volume, PaperRecord, MISSING_GRACE};
use super::store::{parse_rfc3339, PaperStore};
use super::titlematch;
use super::types::{DuplicateGroupDto, MetaSource};
use super::LibraryEntry;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Moves on every `library:changed`, so what is derived from the library
/// can be computed once per change however many readers ask.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn bump_generation() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// Below this similarity a confident first-page title and the recorded
/// one are taken to name different works.
pub const TITLE_MISMATCH: f32 = 0.3;

pub const KIND_DUPLICATE: &str = "duplicate";
pub const KIND_YEAR: &str = "year";
pub const KIND_MISSING: &str = "missing";
pub const KIND_WEAK: &str = "weak";
pub const KIND_TITLE: &str = "title";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HealthFindingDto {
    pub kind: String,
    /// The item the finding is about; the one an Ignore is recorded on.
    pub paper_id: String,
    /// For a duplicate: every record of the group, this one first.
    pub paper_ids: Vec<String>,
    pub title: String,
    /// One muted line: why.
    pub reason: String,
    /// The value the one-click fix writes: the year, or the page's title.
    pub suggestion: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HealthGroupDto {
    pub kind: String,
    pub label: String,
    pub findings: Vec<HealthFindingDto>,
}

fn finding(kind: &str, paper: &PaperRecord, reason: String) -> HealthFindingDto {
    HealthFindingDto {
        kind: kind.to_owned(),
        paper_id: paper.id.clone(),
        paper_ids: vec![paper.id.clone()],
        title: paper.display_title(),
        reason,
        suggestion: None,
    }
}

fn plural(n: u64, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

impl PaperStore {
    /// Every group with something in it, in the order the screen lists
    /// them. `now` is unix seconds.
    pub fn health(
        &self,
        duplicates: &[DuplicateGroupDto],
        entries: &[LibraryEntry],
        now: u64,
    ) -> Vec<HealthGroupDto> {
        let groups = [
            (
                KIND_DUPLICATE,
                "Likely duplicates or versions",
                self.duplicate_findings(duplicates),
            ),
            (KIND_YEAR, "Year may be wrong", self.year_findings()),
            (KIND_MISSING, "Missing files", self.missing_findings(now)),
            (KIND_WEAK, "Weak metadata", self.weak_findings()),
            (KIND_TITLE, "Title mismatch", self.title_findings(entries)),
        ];
        groups
            .into_iter()
            .filter(|(_, _, findings)| !findings.is_empty())
            .map(|(kind, label, findings)| HealthGroupDto {
                kind: kind.to_owned(),
                label: label.to_owned(),
                findings,
            })
            .collect()
    }

    fn by_id(&self) -> HashMap<&str, &PaperRecord> {
        self.papers.iter().map(|p| (p.id.as_str(), p)).collect()
    }

    /// The items the user would look at here: papers, not the other PDFs,
    /// in list order.
    fn health_items(&self, kind: &str) -> Vec<&PaperRecord> {
        let items = self
            .papers
            .iter()
            .filter(|paper| paper.is_paper() && !self.review.is_ignored(&paper.id, kind))
            .collect();
        self.in_list_order(items)
    }

    fn duplicate_findings(&self, groups: &[DuplicateGroupDto]) -> Vec<HealthFindingDto> {
        let by_id = self.by_id();
        groups
            .iter()
            .filter(|group| {
                !group
                    .paper_ids
                    .iter()
                    .any(|id| self.review.is_ignored(id, KIND_DUPLICATE))
            })
            .filter_map(|group| {
                let papers: Vec<&PaperRecord> = group
                    .paper_ids
                    .iter()
                    .filter_map(|id| by_id.get(id.as_str()).copied())
                    .collect();
                let (first, rest) = papers.split_first()?;
                let others = match rest {
                    [] => return None,
                    [one] => format!("\"{}\"", one.display_title()),
                    more => plural(more.len() as u64, "other item"),
                };
                let mut found = finding(
                    KIND_DUPLICATE,
                    first,
                    format!("same work as {others}? {}", group.reason),
                );
                found.paper_ids = papers.iter().map(|p| p.id.clone()).collect();
                Some(found)
            })
            .collect()
    }

    fn year_findings(&self) -> Vec<HealthFindingDto> {
        self.health_items(KIND_YEAR)
            .into_iter()
            .filter_map(|paper| {
                let hint = paper.year_hint()?;
                let floor = paper.citation_facts.year_floor?;
                let year = paper.year()?;
                let mut found = finding(KIND_YEAR, paper, format!("{year}: {hint}"));
                found.suggestion = Some(floor.to_string());
                Some(found)
            })
            .collect()
    }

    fn missing_findings(&self, now: u64) -> Vec<HealthFindingDto> {
        self.health_items(KIND_MISSING)
            .into_iter()
            .filter(|paper| !paper.files.is_empty() && paper.is_missing())
            .map(|paper| {
                let reason = if paper.files.iter().any(|f| on_unmounted_volume(&f.path)) {
                    "on a drive that is not connected".to_owned()
                } else {
                    let since = paper
                        .files
                        .iter()
                        .filter_map(|f| f.missing_since.as_deref().and_then(parse_rfc3339))
                        .max();
                    match since {
                        Some(since) => {
                            let days = now.saturating_sub(since) / 86_400;
                            let grace = MISSING_GRACE.as_secs() / 86_400;
                            let left = grace.saturating_sub(days).max(1);
                            let gone = if days == 0 {
                                "file gone since today".to_owned()
                            } else {
                                format!("file gone for {}", plural(days, "day"))
                            };
                            format!(
                                "{gone}; leaves the library in {} unless it comes back",
                                plural(left, "day")
                            )
                        }
                        None => "file gone".to_owned(),
                    }
                };
                finding(KIND_MISSING, paper, reason)
            })
            .collect()
    }

    fn weak_findings(&self) -> Vec<HealthFindingDto> {
        // Projects that keep a .bib, by member.
        let mut bib_projects: HashMap<&str, Vec<&str>> = HashMap::new();
        for project in self.projects.iter().filter(|p| p.bib_path.is_some()) {
            for id in &project.paper_ids {
                bib_projects
                    .entry(id.as_str())
                    .or_default()
                    .push(project.name.as_str());
            }
        }
        self.health_items(KIND_WEAK)
            .into_iter()
            .filter(|paper| !paper.is_missing())
            .filter_map(|paper| {
                let no_title = paper.title().is_none()
                    || paper
                        .meta
                        .title
                        .as_ref()
                        .is_some_and(|t| t.source == MetaSource::Filename);
                let no_authors = paper.authors().is_empty();
                let reason = match (no_title, no_authors) {
                    (true, true) => "no title beyond the file name, no authors".to_owned(),
                    (true, false) => "no title beyond the file name".to_owned(),
                    (false, true) => "no authors".to_owned(),
                    (false, false) => {
                        let projects = bib_projects.get(paper.id.as_str())?;
                        let missing = paper.incomplete();
                        if missing.is_empty() {
                            return None;
                        }
                        format!(
                            "BibTeX for {} lacks {}",
                            projects.join(", "),
                            missing.join(", ")
                        )
                    }
                };
                Some(finding(KIND_WEAK, paper, reason))
            })
            .collect()
    }

    fn title_findings(&self, entries: &[LibraryEntry]) -> Vec<HealthFindingDto> {
        let by_path: HashMap<String, &LibraryEntry> = entries
            .iter()
            .map(|e| (e.path.to_string_lossy().into_owned(), e))
            .collect();
        self.health_items(KIND_TITLE)
            .into_iter()
            .filter_map(|paper| {
                let recorded = paper.meta.title.as_ref()?;
                if matches!(
                    recorded.source,
                    MetaSource::User | MetaSource::FirstPage | MetaSource::Filename
                ) {
                    return None;
                }
                let file = paper.files.first().filter(|f| !f.missing)?;
                let entry = by_path.get(&file.path)?;
                if !entry.first_page_title_confident {
                    return None;
                }
                let page = entry.title.as_deref()?.trim();
                if !titles_disagree(&recorded.value, page) {
                    return None;
                }
                let mut found =
                    finding(KIND_TITLE, paper, format!("the first page says \"{page}\""));
                found.suggestion = Some(page.to_owned());
                Some(found)
            })
            .collect()
    }
}

/// Both titles say something and share too little to be one work's: not
/// one inside the other (a subtitle dropped), and less alike than
/// [`TITLE_MISMATCH`].
pub fn titles_disagree(recorded: &str, page: &str) -> bool {
    let (a, b) = (titlematch::normalize(recorded), titlematch::normalize(page));
    if a.is_empty() || b.is_empty() || a.contains(&b) || b.contains(&a) {
        return false;
    }
    titlematch::similarity(recorded, page) < TITLE_MISMATCH
}

#[cfg(test)]
mod tests {
    use super::super::consensus::fixtures::{entry, store};
    use super::super::projects::ProjectRecord;
    use super::super::store::format_rfc3339;
    use super::*;
    use crate::library::types::{Author, MetaField, PaperFile};

    const NOW: u64 = 1_800_000_000;

    fn add(store: &mut PaperStore, name: &str, title: Option<(&str, MetaSource)>) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = title.map(|(value, source)| MetaField {
            value: value.to_owned(),
            source,
        });
        paper.meta.authors = Some(MetaField {
            value: vec![Author {
                family: "Author".into(),
                given: "A.".into(),
            }],
            source: MetaSource::Crossref,
        });
        paper.meta.year = Some(MetaField {
            value: 2020,
            source: MetaSource::Crossref,
        });
        paper.meta.venue = Some(MetaField {
            value: "Journal".into(),
            source: MetaSource::Crossref,
        });
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

    fn paper<'a>(store: &'a mut PaperStore, id: &str) -> &'a mut PaperRecord {
        store.papers.iter_mut().find(|p| p.id == id).unwrap()
    }

    fn ids(groups: &[HealthGroupDto], kind: &str) -> Vec<String> {
        groups
            .iter()
            .find(|g| g.kind == kind)
            .map(|g| g.findings.iter().map(|f| f.paper_id.clone()).collect())
            .unwrap_or_default()
    }

    const GOOD: Option<(&str, MetaSource)> = Some((
        "Stochastic thermodynamics of small systems",
        MetaSource::Crossref,
    ));

    #[test]
    fn a_healthy_library_has_no_groups() {
        let mut s = store();
        add(&mut s, "ok", GOOD);
        assert!(s.health(&[], &[], NOW).is_empty());
    }

    #[test]
    fn duplicates_come_from_the_groups_and_an_ignore_on_any_member_hides_them() {
        let mut s = store();
        let a = add(&mut s, "a", GOOD);
        let b = add(
            &mut s,
            "b",
            Some(("Stochastic thermodynamics, small systems", MetaSource::Xmp)),
        );
        let groups = vec![DuplicateGroupDto {
            paper_ids: vec![a.clone(), b.clone()],
            reason: "same title".into(),
        }];
        let health = s.health(&groups, &[], NOW);
        assert_eq!(health[0].kind, KIND_DUPLICATE);
        assert_eq!(health[0].findings[0].paper_ids, vec![a.clone(), b.clone()]);
        assert!(health[0].findings[0].reason.contains("same title"));
        s.ignore_finding(&b, KIND_DUPLICATE).unwrap();
        assert!(s.health(&groups, &[], NOW).is_empty());
    }

    #[test]
    fn a_year_well_below_the_floor_is_flagged_with_the_floor_to_use() {
        let mut s = store();
        let low = add(&mut s, "low", GOOD);
        let near = add(&mut s, "near", GOOD);
        paper(&mut s, &low).citation_facts.year_floor = Some(2023);
        paper(&mut s, &near).citation_facts.year_floor = Some(2021);
        let health = s.health(&[], &[], NOW);
        assert_eq!(ids(&health, KIND_YEAR), vec![low.clone()]);
        let found = &health[0].findings[0];
        assert_eq!(found.suggestion.as_deref(), Some("2023"));
        s.ignore_finding(&low, KIND_YEAR).unwrap();
        assert!(ids(&s.health(&[], &[], NOW), KIND_YEAR).is_empty());
    }

    #[test]
    fn missing_files_say_how_long() {
        let mut s = store();
        let gone = add(&mut s, "gone", GOOD);
        let partly = add(&mut s, "partly", GOOD);
        {
            let p = paper(&mut s, &gone);
            p.files[0].missing = true;
            p.files[0].missing_since = Some(format_rfc3339(NOW - 3 * 86_400 - 60));
        }
        {
            let p = paper(&mut s, &partly);
            p.files.push(PaperFile {
                path: "/l/partly2.pdf".into(),
                name: "partly2.pdf".into(),
                missing: true,
                ..Default::default()
            });
        }
        let health = s.health(&[], &[], NOW);
        assert_eq!(ids(&health, KIND_MISSING), vec![gone.clone()]);
        let reason = &health[0].findings[0].reason;
        assert!(reason.contains("3 days"), "{reason}");
        assert!(reason.contains("in 4 days"), "{reason}");
        s.ignore_finding(&gone, KIND_MISSING).unwrap();
        assert!(s.health(&[], &[], NOW).is_empty());
    }

    #[test]
    fn weak_metadata_no_title_no_authors_or_incomplete_for_a_bib() {
        let mut s = store();
        let untitled = add(&mut s, "untitled", Some(("untitled", MetaSource::Filename)));
        let anonymous = add(&mut s, "anon", GOOD);
        paper(&mut s, &anonymous).meta.authors = None;
        let no_venue = add(&mut s, "novenue", GOOD);
        paper(&mut s, &no_venue).meta.venue = None;
        let no_venue_unsynced = add(&mut s, "novenue2", GOOD);
        paper(&mut s, &no_venue_unsynced).meta.venue = None;
        let project = |name: &str, order: u32, bib: Option<&str>, ids: Vec<String>| ProjectRecord {
            id: format!("p{order}"),
            name: name.to_owned(),
            order,
            paper_ids: ids,
            bib_path: bib.map(str::to_owned),
            created_at: String::new(),
        };
        s.projects = vec![
            project("Thesis", 0, Some("/tmp/t.bib"), vec![no_venue.clone()]),
            project("Notes", 1, None, vec![no_venue_unsynced.clone()]),
        ];

        let health = s.health(&[], &[], NOW);
        let mut weak = ids(&health, KIND_WEAK);
        weak.sort();
        let mut want = vec![untitled.clone(), anonymous.clone(), no_venue.clone()];
        want.sort();
        assert_eq!(weak, want);
        let reason = |id: &str| {
            health[0]
                .findings
                .iter()
                .find(|f| f.paper_id == id)
                .unwrap()
                .reason
                .clone()
        };
        assert_eq!(reason(&untitled), "no title beyond the file name");
        assert_eq!(reason(&anonymous), "no authors");
        assert_eq!(reason(&no_venue), "BibTeX for Thesis lacks venue");
        s.ignore_finding(&untitled, KIND_WEAK).unwrap();
        assert!(!ids(&s.health(&[], &[], NOW), KIND_WEAK).contains(&untitled));
    }

    #[test]
    fn a_confident_page_title_that_disagrees_is_flagged_unless_the_user_typed_the_title() {
        let mut s = store();
        let wrong = add(&mut s, "wrong", GOOD);
        let edited = add(
            &mut s,
            "edited",
            Some(("My own name for it", MetaSource::User)),
        );
        let subtitle = add(&mut s, "sub", GOOD);
        let unsure = add(&mut s, "unsure", GOOD);
        let page = "Optimal transport of active matter in narrow channels";
        let mut unsure_entry = entry("/l/unsure.pdf", page, &[]);
        unsure_entry.first_page_title_confident = false;
        let entries = vec![
            entry("/l/wrong.pdf", page, &[]),
            entry("/l/edited.pdf", page, &[]),
            entry(
                "/l/sub.pdf",
                "Stochastic thermodynamics of small systems: a review",
                &[],
            ),
            unsure_entry,
        ];
        let health = s.health(&[], &entries, NOW);
        assert_eq!(ids(&health, KIND_TITLE), vec![wrong.clone()]);
        assert_eq!(health[0].findings[0].suggestion.as_deref(), Some(page));
        let _ = (edited, subtitle, unsure);
        s.ignore_finding(&wrong, KIND_TITLE).unwrap();
        assert!(s.health(&[], &entries, NOW).is_empty());
    }

    #[test]
    fn titles_disagree_only_when_clearly_different() {
        assert!(titles_disagree(
            "Stochastic thermodynamics of small systems",
            "Deep residual learning for image recognition"
        ));
        assert!(!titles_disagree(
            "Stochastic thermodynamics of small systems",
            "Stochastic thermodynamics of small systems: a review"
        ));
        assert!(!titles_disagree("", "Anything at all here"));
        assert!(!titles_disagree(
            "Stochastic thermodynamics of small systems",
            "Stochastic thermodynamics of tiny systems"
        ));
    }
}

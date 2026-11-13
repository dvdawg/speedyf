//! What the user has looked at and decided, kept in `library.json` beside
//! the records: which new arrivals were triaged, and which health findings
//! were set aside (projects.md sec. 5, "New" and "Health").
//!
//! One versioned section, `review`, so it can change shape without the
//! whole file's version moving. A file without it is a library from before
//! the section existed: `since` is set to the moment it was first loaded,
//! so items that were already there never count as new.

use super::papers::PaperRecord;
use super::store::{now_rfc3339, parse_rfc3339, PaperStore};
use super::types::{PaperFilterDto, PaperRowDto};
use crate::errors::AppResult;
use serde::{Deserialize, Serialize};

pub const REVIEW_VERSION: u32 = 1;

/// How long an item counts as new after it first appeared.
pub const NEW_WINDOW: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// A finding the user said not to show again: one item, one kind.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredFinding {
    pub paper_id: String,
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewState {
    pub version: u32,
    /// Items added before this (RFC 3339) are never new. `None` only on a
    /// state not yet stamped; `PaperStore::load` stamps it.
    #[serde(default)]
    pub since: Option<String>,
    /// New items the user triaged (filed, or marked done).
    #[serde(default)]
    pub reviewed: Vec<String>,
    /// Health findings set aside.
    #[serde(default)]
    pub ignored: Vec<IgnoredFinding>,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            version: REVIEW_VERSION,
            since: None,
            reviewed: Vec::new(),
            ignored: Vec::new(),
        }
    }
}

impl ReviewState {
    /// A fresh state counting arrivals from now on.
    pub fn starting_now() -> Self {
        Self {
            since: Some(now_rfc3339()),
            ..Self::default()
        }
    }

    pub fn is_ignored(&self, paper_id: &str, kind: &str) -> bool {
        self.ignored
            .iter()
            .any(|entry| entry.paper_id == paper_id && entry.kind == kind)
    }
}

impl PaperStore {
    /// Whether the item arrived within [`NEW_WINDOW`] of `now` (unix
    /// seconds), after the review state began, and is not triaged yet.
    pub(super) fn is_new_item(&self, paper: &PaperRecord, now: u64) -> bool {
        if !paper.is_paper() || self.review.reviewed.contains(&paper.id) {
            return false;
        }
        let Some(since) = self.review.since.as_deref().and_then(parse_rfc3339) else {
            return false;
        };
        let Some(added) = parse_rfc3339(&paper.added_at) else {
            return false;
        };
        added >= since && added <= now && now - added <= NEW_WINDOW.as_secs()
    }

    /// The new items matching `query`, newest first.
    pub fn new_items(&self, query: &str, now: u64) -> Vec<PaperRowDto> {
        let added: std::collections::HashMap<&str, u64> = self
            .papers
            .iter()
            .filter(|paper| self.is_new_item(paper, now))
            .map(|paper| {
                (
                    paper.id.as_str(),
                    parse_rfc3339(&paper.added_at).unwrap_or(0),
                )
            })
            .collect();
        if added.is_empty() {
            return Vec::new();
        }
        let filter = PaperFilterDto {
            query: query.to_owned(),
            ..Default::default()
        };
        let mut rows: Vec<PaperRowDto> = self
            .paper_rows(&filter)
            .rows
            .into_iter()
            .filter(|row| added.contains_key(row.id.as_str()))
            .collect();
        // Stable: equal times keep the list's own order.
        rows.sort_by_key(|row| std::cmp::Reverse(added[row.id.as_str()]));
        rows
    }

    /// Mark items triaged. Ids of items no longer new are dropped as the
    /// list is rewritten, so it never grows past what the window holds.
    pub fn mark_reviewed(&mut self, ids: &[String], now: u64) -> AppResult<()> {
        self.ensure_writable()?;
        let mut reviewed: Vec<String> = Vec::new();
        for id in self.review.reviewed.iter().chain(ids) {
            if reviewed.contains(id) {
                continue;
            }
            let recent = self.papers.iter().any(|paper| {
                &paper.id == id
                    && parse_rfc3339(&paper.added_at)
                        .is_some_and(|added| now.saturating_sub(added) <= NEW_WINDOW.as_secs())
            });
            if recent {
                reviewed.push(id.clone());
            }
        }
        self.review.reviewed = reviewed;
        self.mark_dirty();
        Ok(())
    }

    /// Never show this finding about this item again.
    pub fn ignore_finding(&mut self, paper_id: &str, kind: &str) -> AppResult<()> {
        self.ensure_writable()?;
        if !self.review.is_ignored(paper_id, kind) {
            self.review.ignored.push(IgnoredFinding {
                paper_id: paper_id.to_owned(),
                kind: kind.to_owned(),
            });
            self.mark_dirty();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::consensus::fixtures::store;
    use super::super::store::format_rfc3339;
    use super::*;
    use crate::library::types::{MetaField, MetaSource};

    const DAY: u64 = 24 * 60 * 60;
    const NOW: u64 = 1_800_000_000;

    fn add(store: &mut PaperStore, title: &str, added: u64) -> String {
        let mut paper = PaperRecord::new();
        paper.meta.title = Some(MetaField {
            value: title.to_owned(),
            source: MetaSource::Crossref,
        });
        paper.added_at = format_rfc3339(added);
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    fn titles(rows: &[PaperRowDto]) -> Vec<&str> {
        rows.iter().map(|row| row.title.as_str()).collect()
    }

    #[test]
    fn lists_recent_arrivals_newest_first_and_never_older_ones() {
        let mut store = store();
        store.review.since = Some(format_rfc3339(NOW - 30 * DAY));
        add(&mut store, "Old one", NOW - 8 * DAY);
        add(&mut store, "Middle", NOW - 3 * DAY);
        add(&mut store, "Newest", NOW - 60);
        add(&mut store, "Before the feature", NOW - 31 * DAY);
        assert_eq!(titles(&store.new_items("", NOW)), ["Newest", "Middle"]);
        assert_eq!(titles(&store.new_items("midd", NOW)), ["Middle"]);
    }

    #[test]
    fn items_from_before_the_review_state_are_never_new() {
        let mut store = store();
        add(&mut store, "Existing", NOW - DAY);
        store.review.since = Some(format_rfc3339(NOW - 60));
        assert!(store.new_items("", NOW).is_empty());
        store.review.since = None;
        assert!(store.new_items("", NOW).is_empty());
    }

    #[test]
    fn reviewed_items_leave_and_the_mark_persists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = PaperStore::load(dir.path().to_path_buf());
        store.review.since = Some(format_rfc3339(NOW - 30 * DAY));
        let a = add(&mut store, "A", NOW - DAY);
        let old = add(&mut store, "Old", NOW - 20 * DAY);
        add(&mut store, "B", NOW - 2 * DAY);
        store.mark_reviewed(&[a.clone(), old], NOW).unwrap();
        // An id past the window is not kept.
        assert_eq!(store.review.reviewed, vec![a.clone()]);
        assert_eq!(titles(&store.new_items("", NOW)), ["B"]);
        store.ignore_finding(&a, "year").unwrap();
        store.save().unwrap();
        let reloaded = PaperStore::load(dir.path().to_path_buf());
        assert_eq!(reloaded.review.reviewed, vec![a.clone()]);
        assert!(reloaded.review.is_ignored(&a, "year"));
        assert!(!reloaded.review.is_ignored(&a, "title"));
        assert_eq!(reloaded.review.version, REVIEW_VERSION);
    }

    #[test]
    fn a_library_without_the_section_starts_counting_at_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("library.json"),
            r#"{"version":1,"papers":[],"projects":[]}"#,
        )
        .unwrap();
        let store = PaperStore::load(dir.path().to_path_buf());
        let since = store.review.since.as_deref().and_then(parse_rfc3339);
        assert!(since.is_some());
    }
}

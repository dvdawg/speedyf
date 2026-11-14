//! Undo for what the user does to the library (projects.md sec. 14).
//!
//! Each user action that changes records runs through `journaled`: the
//! records, projects and review marks are cloned before it, compared after
//! it, and only what the action changed is kept, as it was before. That is
//! a step, with a short description ("Removed from pac_learning"). The last
//! [`UNDO_LIMIT`] steps are kept in memory, never on disk.
//!
//! Undo puts back exactly the records, projects and marks of its step, as
//! one ordinary save. Everything else is left as it is now, so what a
//! scan, a lookup or the citation consensus wrote meanwhile to other
//! records survives. Before restoring, the same keys are captured as they
//! are now: that is the redo step, and redoing works the same way back. A
//! new action clears the redo steps.
//!
//! Scans, lookups and background merges never come through here, so they
//! are never undone.

use super::papers::PaperRecord;
use super::projects::ProjectRecord;
use super::review::{IgnoredFinding, ReviewState};
use super::store::PaperStore;
use crate::errors::{AppError, AppResult};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// How many steps undo reaches back.
pub const UNDO_LIMIT: usize = 20;

/// A record as it was: absent, or present at a position in the list.
#[derive(Clone, Debug, PartialEq)]
enum Slot<T> {
    Absent,
    At(usize, T),
}

/// A project as it was. A project only renumbered by the action keeps its
/// order alone, so its members as they are now are never put back.
#[derive(Clone, Debug, PartialEq)]
enum ProjectSlot {
    Whole(Slot<ProjectRecord>),
    Order(u32),
}

/// The review section's lists: which new items were triaged, which
/// findings set aside. Only user actions write them, so a step keeps them
/// whole.
#[derive(Clone, Debug, PartialEq)]
struct Marks {
    reviewed: Vec<String>,
    ignored: Vec<IgnoredFinding>,
}

impl Marks {
    fn of(review: &ReviewState) -> Self {
        Self {
            reviewed: review.reviewed.clone(),
            ignored: review.ignored.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct Step {
    /// Rises with each new action; a redo keeps the step's own, so a window
    /// can tell a new action from a redone one.
    seq: u64,
    description: String,
    papers: Vec<(String, Slot<PaperRecord>)>,
    projects: Vec<(String, ProjectSlot)>,
    marks: Option<Marks>,
}

impl Step {
    fn is_empty(&self) -> bool {
        self.papers.is_empty() && self.projects.is_empty() && self.marks.is_none()
    }
}

#[derive(Default)]
pub struct Journal {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    seq: u64,
}

/// The next undo and redo, for the palette and the status bar.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UndoStateDto {
    pub undo: Option<UndoEntryDto>,
    pub redo: Option<UndoEntryDto>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UndoEntryDto {
    pub seq: u64,
    pub description: String,
}

/// What an undo or redo did, for the command to finish with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Undone {
    pub description: String,
    /// Projects it brought back, whose set-aside files come back too.
    pub projects_restored: Vec<String>,
    /// Projects it took away, whose files are set aside as a delete does.
    pub projects_removed: Vec<String>,
}

/// The state an action can change, cloned before it runs.
struct Before {
    papers: Vec<PaperRecord>,
    projects: Vec<ProjectRecord>,
    review: ReviewState,
}

/// `before` against `after`, by id: each record the action added (absent
/// before), changed or removed, as it was.
fn diff_records<T: Clone + PartialEq>(
    before: &[T],
    after: &[T],
    id: impl Fn(&T) -> &str,
) -> Vec<(String, Slot<T>)> {
    let old: HashMap<&str, (usize, &T)> = before
        .iter()
        .enumerate()
        .map(|(at, record)| (id(record), (at, record)))
        .collect();
    let now: HashMap<&str, &T> = after.iter().map(|record| (id(record), record)).collect();
    let mut out = Vec::new();
    for record in after {
        match old.get(id(record)) {
            None => out.push((id(record).to_owned(), Slot::Absent)),
            Some((at, was)) if *was != record => {
                out.push((id(record).to_owned(), Slot::At(*at, (*was).clone())));
            }
            Some(_) => {}
        }
    }
    for (at, record) in before.iter().enumerate() {
        if !now.contains_key(id(record)) {
            out.push((id(record).to_owned(), Slot::At(at, record.clone())));
        }
    }
    out
}

impl PaperStore {
    /// Run a user action as one undoable step. A step that changed nothing
    /// is not kept; an action that fails keeps nothing either.
    pub fn journaled<T>(
        &mut self,
        description: impl Into<String>,
        action: impl FnOnce(&mut Self) -> AppResult<T>,
    ) -> AppResult<T> {
        self.journaled_creating(description, &[], action)
    }

    /// `journaled`, for an action whose records were made just before it,
    /// outside it (a bundle import scans its PDFs first): undoing it removes
    /// those records too.
    pub fn journaled_creating<T>(
        &mut self,
        description: impl Into<String>,
        created: &[String],
        action: impl FnOnce(&mut Self) -> AppResult<T>,
    ) -> AppResult<T> {
        let before = Before {
            papers: self.papers.clone(),
            projects: self.projects.clone(),
            review: self.review.clone(),
        };
        let result = action(self)?;
        let mut step = self.diff(before, description.into());
        for id in created {
            if !self.papers.iter().any(|paper| &paper.id == id) {
                continue;
            }
            match step.papers.iter_mut().find(|(known, _)| known == id) {
                Some((_, slot)) => *slot = Slot::Absent,
                None => step.papers.push((id.clone(), Slot::Absent)),
            }
        }
        if !step.is_empty() {
            let journal = &mut self.journal;
            journal.seq += 1;
            step.seq = journal.seq;
            journal.undo.push_back(step);
            while journal.undo.len() > UNDO_LIMIT {
                journal.undo.pop_front();
            }
            journal.redo.clear();
        }
        Ok(result)
    }

    fn diff(&self, before: Before, description: String) -> Step {
        let papers = diff_records(&before.papers, &self.papers, |p| p.id.as_str());
        let projects = diff_records(&before.projects, &self.projects, |p| p.id.as_str())
            .into_iter()
            .map(|(id, slot)| {
                let now = self.projects.iter().find(|p| p.id == id);
                let slot = match (slot, now) {
                    (Slot::At(at, was), Some(now)) => {
                        let only_order = ProjectRecord {
                            order: now.order,
                            ..was.clone()
                        } == *now;
                        if only_order {
                            ProjectSlot::Order(was.order)
                        } else {
                            ProjectSlot::Whole(Slot::At(at, was))
                        }
                    }
                    (slot, _) => ProjectSlot::Whole(slot),
                };
                (id, slot)
            })
            .collect();
        let was = Marks::of(&before.review);
        let marks = (was != Marks::of(&self.review)).then_some(was);
        Step {
            seq: 0,
            description,
            papers,
            projects,
            marks,
        }
    }

    /// Every record's id, for an action that makes records before it runs
    /// (`journaled_creating`).
    pub fn record_ids(&self) -> std::collections::HashSet<String> {
        self.papers.iter().map(|paper| paper.id.clone()).collect()
    }

    /// The next undo and redo.
    pub fn undo_state(&self) -> UndoStateDto {
        let entry = |step: &Step| UndoEntryDto {
            seq: step.seq,
            description: step.description.clone(),
        };
        UndoStateDto {
            undo: self.journal.undo.back().map(entry),
            redo: self.journal.redo.last().map(entry),
        }
    }

    /// Put back the latest step. The caller saves, which also rewrites the
    /// synced `.bib` of every project it touched.
    pub fn undo(&mut self) -> AppResult<Undone> {
        self.ensure_writable()?;
        let step = self
            .journal
            .undo
            .pop_back()
            .ok_or_else(|| AppError::Unsupported("nothing to undo in the library".into()))?;
        let (inverse, undone) = self.restore(step);
        self.journal.redo.push(inverse);
        Ok(undone)
    }

    /// Do again the step undo last put back.
    pub fn redo(&mut self) -> AppResult<Undone> {
        self.ensure_writable()?;
        let step = self
            .journal
            .redo
            .pop()
            .ok_or_else(|| AppError::Unsupported("nothing to redo in the library".into()))?;
        let (inverse, undone) = self.restore(step);
        self.journal.undo.push_back(inverse);
        Ok(undone)
    }

    /// Forget every step: the records were replaced wholesale (a backup
    /// restored, another data folder adopted).
    pub fn clear_journal(&mut self) {
        self.journal.undo.clear();
        self.journal.redo.clear();
    }

    /// Put the step's keys back as it holds them; the same keys as they
    /// were just before are the step that undoes this.
    fn restore(&mut self, step: Step) -> (Step, Undone) {
        let mut undone = Undone {
            description: step.description.clone(),
            ..Undone::default()
        };
        let papers = swap_in(&mut self.papers, step.papers, |p| p.id.as_str());
        let mut orders = Vec::new();
        let mut wholes = Vec::new();
        for (id, slot) in step.projects {
            match slot {
                ProjectSlot::Order(order) => orders.push((id, order)),
                ProjectSlot::Whole(slot) => wholes.push((id, slot)),
            }
        }
        let mut projects = Vec::with_capacity(orders.len() + wholes.len());
        for (id, order) in orders {
            if let Some(project) = self.projects.iter_mut().find(|p| p.id == id) {
                projects.push((id, ProjectSlot::Order(project.order)));
                project.order = order;
            }
        }
        let wanted: Vec<(String, bool)> = wholes
            .iter()
            .map(|(id, slot)| (id.clone(), matches!(slot, Slot::At(..))))
            .collect();
        let current = swap_in(&mut self.projects, wholes, |p| p.id.as_str());
        for ((id, present), (_, was)) in wanted.into_iter().zip(&current) {
            match (was, present) {
                (Slot::Absent, true) => undone.projects_restored.push(id),
                (Slot::At(..), false) => undone.projects_removed.push(id),
                _ => {}
            }
        }
        projects.extend(
            current
                .into_iter()
                .map(|(id, slot)| (id, ProjectSlot::Whole(slot))),
        );
        self.renumber();
        let marks = step.marks.map(|wanted| {
            let now = Marks::of(&self.review);
            self.review.reviewed = wanted.reviewed;
            self.review.ignored = wanted.ignored;
            now
        });
        self.mark_dirty();
        let inverse = Step {
            seq: step.seq,
            description: step.description,
            papers,
            projects,
            marks,
        };
        (inverse, undone)
    }
}

/// Put these records back as the slots hold them, each at its place in
/// the list (or where it now ends), and say how each was just before.
fn swap_in<T: Clone>(
    list: &mut Vec<T>,
    wanted: Vec<(String, Slot<T>)>,
    id: impl Fn(&T) -> &str,
) -> Vec<(String, Slot<T>)> {
    let current: Vec<(String, Slot<T>)> = wanted
        .iter()
        .map(|(key, _)| {
            let slot = match list.iter().position(|record| id(record) == key) {
                Some(at) => Slot::At(at, list[at].clone()),
                None => Slot::Absent,
            };
            (key.clone(), slot)
        })
        .collect();
    list.retain(|record| !wanted.iter().any(|(key, _)| key == id(record)));
    let mut puts: Vec<(usize, T)> = wanted
        .into_iter()
        .filter_map(|(_, slot)| match slot {
            Slot::At(at, record) => Some((at, record)),
            Slot::Absent => None,
        })
        .collect();
    puts.sort_by_key(|(at, _)| *at);
    for (at, record) in puts {
        let at = at.min(list.len());
        list.insert(at, record);
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{MetaField, MetaPatchDto, MetaSource, PaperFile};

    fn store() -> (tempfile::TempDir, PaperStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        (dir, store)
    }

    fn add_paper(store: &mut PaperStore, key: &str, title: &str) -> String {
        let mut paper = PaperRecord::new();
        paper.cite_key = key.into();
        paper.meta.title = Some(MetaField {
            value: title.into(),
            source: MetaSource::FirstPage,
        });
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    /// Everything a step can touch, as one value to compare.
    fn state(store: &PaperStore) -> serde_json::Value {
        serde_json::json!({
            "papers": store.papers,
            "projects": store.projects,
            "review": store.review,
        })
    }

    /// Two items, two projects, the first item in the first project.
    fn library() -> (tempfile::TempDir, PaperStore, [String; 2], [String; 2]) {
        let (dir, mut store) = store();
        let a = add_paper(&mut store, "smith2020", "On Things");
        let b = add_paper(&mut store, "jones2021", "On Things, Again");
        let p = store.create_project("pac_learning").unwrap().id;
        let q = store.create_project("thesis").unwrap().id;
        store.set_membership(&a, &p, true).unwrap();
        (dir, store, [a, b], [p, q])
    }

    /// Run `action` journaled, then check undo gives back exactly what was
    /// before and redo exactly what it made.
    fn round_trip(
        store: &mut PaperStore,
        description: &str,
        action: impl FnOnce(&mut PaperStore) -> AppResult<()>,
    ) {
        let before = state(store);
        store.journaled(description, action).expect("action");
        let after = state(store);
        assert_ne!(before, after, "{description} changed nothing");
        assert_eq!(
            store.undo_state().undo.map(|e| e.description).as_deref(),
            Some(description)
        );
        assert_eq!(store.undo().unwrap().description, description);
        assert_eq!(state(store), before, "undo of {description}");
        assert_eq!(
            store.undo_state().redo.map(|e| e.description).as_deref(),
            Some(description)
        );
        store.redo().unwrap();
        assert_eq!(state(store), after, "redo of {description}");
    }

    fn patch(edit: impl FnOnce(&mut MetaPatchDto)) -> MetaPatchDto {
        let mut patch: MetaPatchDto = serde_json::from_str("{}").unwrap();
        edit(&mut patch);
        patch
    }

    #[test]
    fn every_covered_action_undoes_and_redoes_exactly() {
        let (_dir, mut store, [a, b], [p, q]) = library();
        round_trip(&mut store, "Removed from pac_learning", |s| {
            s.set_membership(&a, &p, false)
        });
        round_trip(&mut store, "Added to thesis", |s| {
            s.set_membership(&a, &q, true)
        });
        round_trip(&mut store, "Created drafts", |s| {
            s.create_project("drafts").map(|_| ())
        });
        round_trip(&mut store, "Renamed", |s| s.rename_project(&p, "pac"));
        let ids = vec![q.clone(), p.clone()];
        round_trip(&mut store, "Reordered projects", |s| {
            s.reorder_projects(&ids)
        });
        round_trip(&mut store, "Deleted pac", |s| s.delete_project(&p));
        round_trip(&mut store, "Marked as not an item", |s| {
            s.apply_patch(&a, patch(|p| p.is_paper = Some(Some(false))))
                .map(|_| ())
        });
        round_trip(&mut store, "Edited year", |s| {
            s.apply_patch(&a, patch(|p| p.year = Some(Some(2019))))
                .map(|_| ())
        });
        round_trip(&mut store, "Changed cite key", |s| {
            s.apply_patch(&a, patch(|p| p.cite_key = Some(Some("smiththings".into()))))
                .map(|_| ())
        });
        round_trip(&mut store, "Pinned cite key", |s| {
            s.apply_patch(&b, patch(|p| p.cite_key = Some(Some("jones2021".into()))))
                .map(|_| ())
        });
        round_trip(&mut store, "Ignored a finding", |s| {
            s.ignore_finding(&a, "year")
        });
        let ids = vec![a.clone(), b.clone()];
        round_trip(&mut store, "Marked 2 items done", |s| {
            s.mark_reviewed(&ids, crate::library::store::now_unix_secs())
        });
        round_trip(&mut store, "Marked as not duplicates", |s| {
            s.mark_not_duplicates(&ids)
        });
        round_trip(&mut store, "Merged 2 items", |s| {
            s.merge_papers(&a, std::slice::from_ref(&b)).map(|_| ())
        });
        round_trip(&mut store, "Split item", |s| s.unmerge(&a).map(|_| ()));
    }

    #[test]
    fn a_background_change_to_another_record_survives_an_undo() {
        let (_dir, mut store, [a, b], _) = library();
        store
            .journaled("Edited title", |s| {
                s.apply_patch(&a, patch(|p| p.title = Some(Some("Renamed".into()))))
            })
            .unwrap();
        // A lookup lands on the other item meanwhile, outside the journal.
        let index = store.papers.iter().position(|p| p.id == b).unwrap();
        store.papers[index].meta.venue = Some(MetaField {
            value: "JMLR".into(),
            source: MetaSource::Crossref,
        });
        store.undo().unwrap();
        let a_now = store.papers.iter().find(|p| p.id == a).unwrap();
        assert_eq!(a_now.title(), Some("On Things"));
        let b_now = store.papers.iter().find(|p| p.id == b).unwrap();
        assert_eq!(b_now.meta.venue.as_ref().unwrap().value, "JMLR");
    }

    #[test]
    fn only_the_last_twenty_steps_are_kept_and_a_new_action_clears_redo() {
        let (_dir, mut store, [a, _], [p, _]) = library();
        for n in 0..25 {
            store
                .journaled(format!("step {n}"), |s| {
                    s.set_membership(&a, &p, n % 2 == 0)
                })
                .unwrap();
        }
        for _ in 0..UNDO_LIMIT {
            store.undo().unwrap();
        }
        assert!(store.undo().is_err(), "the 21st undo has nothing left");
        assert_eq!(
            store.undo_state().redo.map(|e| e.description).as_deref(),
            Some("step 5")
        );
        store
            .journaled("Created drafts", |s| s.create_project("drafts"))
            .unwrap();
        assert!(store.undo_state().redo.is_none());
        assert!(store.redo().is_err());
        // An action that changes nothing is not a step.
        store
            .journaled("nothing", |s| s.set_membership(&a, &p, true))
            .unwrap();
        assert_eq!(
            store.undo_state().undo.map(|e| e.description).as_deref(),
            Some("Created drafts")
        );
    }

    #[test]
    fn a_redo_keeps_its_sequence_number_and_a_new_action_gets_a_higher_one() {
        let (_dir, mut store, [a, _], [p, _]) = library();
        store
            .journaled("Removed", |s| s.set_membership(&a, &p, false))
            .unwrap();
        let first = store.undo_state().undo.unwrap().seq;
        store.undo().unwrap();
        store.redo().unwrap();
        assert_eq!(store.undo_state().undo.unwrap().seq, first);
        store
            .journaled("Added", |s| s.set_membership(&a, &p, true))
            .unwrap();
        assert!(store.undo_state().undo.unwrap().seq > first);
    }

    #[test]
    fn undoing_a_bundle_import_removes_its_project_and_records_but_keeps_the_files() {
        let (dir, mut store, [a, _], _) = library();
        let pdf = dir.path().join("inbox").join("bundled.pdf");
        std::fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        std::fs::write(&pdf, b"%PDF-1.4").unwrap();
        let before = state(&store);
        let before_ids = store.record_ids();
        // The scan of the copied PDF made a record before the import ran.
        let made = add_paper(&mut store, "lee2022", "Bundled");
        let index = store.papers.iter().position(|p| p.id == made).unwrap();
        store.papers[index].files.push(PaperFile {
            path: pdf.to_string_lossy().into_owned(),
            name: "bundled.pdf".into(),
            ..PaperFile::default()
        });
        let members = vec![
            (made.clone(), "lee22".to_owned()),
            (a.clone(), "old".to_owned()),
        ];
        let created: Vec<String> = members
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| !before_ids.contains(id))
            .collect();
        let project = store
            .journaled_creating("Imported Thesis", &created, |s| {
                s.import_project("Thesis", &members)
            })
            .unwrap();
        let after = state(&store);
        let undone = store.undo().unwrap();
        assert_eq!(undone.projects_removed, vec![project.id.clone()]);
        assert_eq!(state(&store), before);
        assert!(pdf.is_file(), "the PDF copied into the inbox stays");
        let redone = store.redo().unwrap();
        assert_eq!(redone.projects_restored, vec![project.id]);
        assert_eq!(state(&store), after);
    }

    #[test]
    fn undo_and_redo_with_nothing_to_do_are_refused() {
        let (_dir, mut store) = store();
        assert!(store.undo().is_err());
        assert!(store.redo().is_err());
        assert_eq!(store.undo_state(), UndoStateDto::default());
    }
}

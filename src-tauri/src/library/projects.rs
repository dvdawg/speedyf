//! Projects and memberships: a name, an ordered position, the ids of its
//! papers, and an optional synced `.bib` path. Flat, many-to-many, held on
//! the project as a list of paper ids (projects.md §2).
//!
//! Deleting a project removes memberships and nothing else; files and papers
//! are untouched.
//!
//! A project's `.bib` is written here too (projects.md §7): once, on export,
//! or on every save while the project has a synced path. SpeedyF owns a
//! synced file — hand edits to it are lost by design, and the file says so
//! in its first line.

use super::bibtex;
use super::papers::PaperRecord;
use super::store::{now_rfc3339, PaperStore};
use super::types::{PaperDto, ProjectDto};
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub order: u32,
    #[serde(default)]
    pub paper_ids: Vec<String>,
    #[serde(default)]
    pub bib_path: Option<String>,
    pub created_at: String,
}

fn clean_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Internal("project name cannot be empty".into()));
    }
    Ok(name.to_owned())
}

impl PaperStore {
    fn project_index(&self, id: &str) -> AppResult<usize> {
        self.projects
            .iter()
            .position(|project| project.id == id)
            .ok_or_else(|| AppError::Internal("project not found".into()))
    }

    /// Projects in their stored order.
    fn sorted_projects(&self) -> Vec<&ProjectRecord> {
        let mut projects: Vec<&ProjectRecord> = self.projects.iter().collect();
        projects.sort_by_key(|project| project.order);
        projects
    }

    /// Give every project a distinct position 0..n in its current order, so
    /// a gap left by a delete or a tie left by an older file never shows.
    fn renumber(&mut self) {
        let mut order: Vec<(u32, usize)> = self
            .projects
            .iter()
            .enumerate()
            .map(|(index, project)| (project.order, index))
            .collect();
        order.sort();
        for (position, (_, index)) in order.into_iter().enumerate() {
            self.projects[index].order = position as u32;
        }
    }

    fn refuse_duplicate_name(&self, name: &str, except: Option<usize>) -> AppResult<()> {
        let wanted = name.to_lowercase();
        let clash =
            self.projects.iter().enumerate().any(|(index, project)| {
                Some(index) != except && project.name.to_lowercase() == wanted
            });
        if clash {
            return Err(AppError::Internal(format!(
                "a project named \"{name}\" already exists"
            )));
        }
        Ok(())
    }

    fn project_dto_for(&self, project: &ProjectRecord) -> ProjectDto {
        let mut paper_count = 0u32;
        let mut incomplete_count = 0u32;
        for id in &project.paper_ids {
            if let Some(paper) = self.papers.iter().find(|paper| &paper.id == id) {
                paper_count += 1;
                if !paper.incomplete().is_empty() {
                    incomplete_count += 1;
                }
            }
        }
        ProjectDto {
            id: project.id.clone(),
            name: project.name.clone(),
            order: project.order,
            paper_count,
            bib_path: project.bib_path.clone(),
            incomplete_count,
            created_at: project.created_at.clone(),
        }
    }

    pub fn projects(&self) -> Vec<ProjectDto> {
        self.sorted_projects()
            .into_iter()
            .map(|project| self.project_dto_for(project))
            .collect()
    }

    pub fn project_dto(&self, id: &str) -> AppResult<ProjectDto> {
        let project = &self.projects[self.project_index(id)?];
        Ok(self.project_dto_for(project))
    }

    /// Paper id → ids of the projects holding it, in project order. Papers
    /// in no project are absent.
    pub(super) fn membership_map(&self) -> HashMap<&str, Vec<String>> {
        let mut map: HashMap<&str, Vec<String>> = HashMap::new();
        for project in self.sorted_projects() {
            for paper_id in &project.paper_ids {
                map.entry(paper_id.as_str())
                    .or_default()
                    .push(project.id.clone());
            }
        }
        map
    }

    pub(super) fn project_ids_for(&self, paper_id: &str) -> Vec<String> {
        self.sorted_projects()
            .into_iter()
            .filter(|project| project.paper_ids.iter().any(|id| id == paper_id))
            .map(|project| project.id.clone())
            .collect()
    }

    pub fn create_project(&mut self, name: &str) -> AppResult<ProjectDto> {
        self.ensure_writable()?;
        let name = clean_name(name)?;
        self.refuse_duplicate_name(&name, None)?;
        let order = self
            .projects
            .iter()
            .map(|project| project.order + 1)
            .max()
            .unwrap_or(0);
        let project = ProjectRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            order,
            paper_ids: Vec::new(),
            bib_path: None,
            created_at: now_rfc3339(),
        };
        let dto = self.project_dto_for(&project);
        self.projects.push(project);
        self.renumber();
        Ok(dto)
    }

    pub fn rename_project(&mut self, id: &str, name: &str) -> AppResult<()> {
        self.ensure_writable()?;
        let index = self.project_index(id)?;
        let name = clean_name(name)?;
        self.refuse_duplicate_name(&name, Some(index))?;
        self.projects[index].name = name;
        Ok(())
    }

    pub fn delete_project(&mut self, id: &str) -> AppResult<()> {
        self.ensure_writable()?;
        let index = self.project_index(id)?;
        self.projects.remove(index);
        self.renumber();
        Ok(())
    }

    /// `ids` in this order first; ids it does not name keep their old order
    /// after them; ids it names that do not exist are ignored.
    pub fn reorder_projects(&mut self, ids: &[String]) -> AppResult<()> {
        self.ensure_writable()?;
        let mut placed: Vec<usize> = Vec::new();
        for id in ids {
            if let Some(index) = self.projects.iter().position(|project| &project.id == id) {
                if !placed.contains(&index) {
                    placed.push(index);
                }
            }
        }
        let mut rest: Vec<usize> = (0..self.projects.len())
            .filter(|index| !placed.contains(index))
            .collect();
        rest.sort_by_key(|index| self.projects[*index].order);
        for (position, index) in placed.into_iter().chain(rest).enumerate() {
            self.projects[index].order = position as u32;
        }
        Ok(())
    }

    pub fn set_membership(
        &mut self,
        paper_id: &str,
        project_id: &str,
        member: bool,
    ) -> AppResult<()> {
        self.ensure_writable()?;
        self.index_of(paper_id)?;
        let index = self.project_index(project_id)?;
        let project = &mut self.projects[index];
        let present = project.paper_ids.iter().any(|id| id == paper_id);
        match (member, present) {
            (true, false) => project.paper_ids.push(paper_id.to_owned()),
            (false, true) => project.paper_ids.retain(|id| id != paper_id),
            _ => {}
        }
        Ok(())
    }

    /// Record where the project's `.bib` goes, or that it goes nowhere. The
    /// file itself is written by the next save (`sync_project_bibs`), which
    /// the command that calls this performs at once.
    pub fn set_project_bib(&mut self, project_id: &str, path: Option<String>) -> AppResult<()> {
        self.ensure_writable()?;
        let index = self.project_index(project_id)?;
        self.projects[index].bib_path = path
            .map(|path| path.trim().to_owned())
            .filter(|path| !path.is_empty());
        Ok(())
    }

    /// The project's papers in list order — the order `library_papers`
    /// shows them — so a `.bib` reads like the screen. Ids that name no
    /// paper are skipped.
    pub fn project_members(&self, project_id: &str) -> AppResult<Vec<&PaperRecord>> {
        let project = &self.projects[self.project_index(project_id)?];
        let by_id: HashMap<&str, &PaperRecord> = self
            .papers
            .iter()
            .map(|paper| (paper.id.as_str(), paper))
            .collect();
        let members = project
            .paper_ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .collect();
        Ok(self.in_list_order(members))
    }

    /// The files on disk of every paper in a project: what a full-text
    /// search scoped to it reads. Missing files are left out; an unknown
    /// project is an error.
    pub fn project_files(&self, project_id: &str) -> AppResult<HashSet<PathBuf>> {
        Ok(self
            .project_members(project_id)?
            .into_iter()
            .flat_map(|paper| paper.files.iter())
            .filter(|file| !file.missing)
            .map(|file| PathBuf::from(&file.path))
            .collect())
    }

    /// The project's `.bib`, header included, and the ids of the papers in
    /// it, in file order.
    fn render_project_bib(&self, project_id: &str) -> AppResult<(String, Vec<String>)> {
        let members = self.project_members(project_id)?;
        let ids = members.iter().map(|paper| paper.id.clone()).collect();
        let dtos: Vec<PaperDto> = members.iter().map(|paper| self.dto_for(paper)).collect();
        Ok((bibtex::render(&dtos, true), ids))
    }

    /// Export .bib…: the project's entries written to `path` once. The
    /// file is written before the keys are pinned, so a write that fails
    /// pins nothing.
    pub fn export_project_bib(&mut self, project_id: &str, path: &Path) -> AppResult<()> {
        self.ensure_writable()?;
        let (text, ids) = self.render_project_bib(project_id)?;
        write_bib_atomically(path, &text)?;
        self.freeze_keys(&ids);
        Ok(())
    }

    /// Bring every synced `.bib` up to date with the records: rendered,
    /// compared with the file, written only when the bytes differ, and its
    /// keys pinned. Called from every save, so the file never lags the
    /// data. A file that cannot be written is reported by project and does
    /// not stop the others or the save — the user's action was a change to
    /// their library, and that must land whatever the state of a folder
    /// that may be unmounted.
    ///
    /// Comparing first is what makes "on every save" affordable: a scan
    /// flushes once a second, and a sync service watching the folder, like
    /// an editor with the file open, reacts to a write and not to a read.
    pub fn sync_project_bibs(&mut self) -> Vec<(String, AppError)> {
        let synced: Vec<(String, String)> = self
            .projects
            .iter()
            .filter_map(|project| Some((project.id.clone(), project.bib_path.clone()?)))
            .collect();
        let mut failures = Vec::new();
        for (project_id, path) in synced {
            let (text, ids) = match self.render_project_bib(&project_id) {
                Ok(rendered) => rendered,
                Err(error) => {
                    failures.push((project_id, error));
                    continue;
                }
            };
            let path = Path::new(&path);
            if fs::read(path).ok().as_deref() != Some(text.as_bytes()) {
                if let Err(error) = write_bib_atomically(path, &text) {
                    failures.push((project_id, error));
                    continue;
                }
            }
            self.freeze_keys(&ids);
        }
        failures
    }
}

/// A temp file beside the target, then a rename over it, so a `bibtex` run
/// that happens to be reading never sees half a file.
fn write_bib_atomically(path: &Path, text: &str) -> AppResult<()> {
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| AppError::Io(format!("{} has no parent folder", path.display())))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".speedyf-")
        .suffix(".bib.tmp")
        .tempfile_in(dir)
        .map_err(|error| AppError::Io(format!("cannot write in {}: {error}", dir.display())))?;
    temp.write_all(text.as_bytes())
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| AppError::Io(format!("cannot write {}: {error}", path.display())))?;
    temp.persist(path).map_err(|error| {
        AppError::Io(format!(
            "cannot replace {}: {}",
            path.display(),
            error.error
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::citekey;
    use crate::library::types::{Author, MetaField, MetaPatchDto, MetaSource};
    use std::time::{Duration, SystemTime};

    fn store() -> PaperStore {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = PaperStore::load(dir.path().to_path_buf());
        std::mem::forget(dir);
        store
    }

    fn add_paper(store: &mut PaperStore) -> String {
        let paper = PaperRecord::new();
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    /// A complete paper, as a lookup would have left it, with its key.
    fn add_paper_with(store: &mut PaperStore, family: &str, year: i32, title: &str) -> String {
        fn field<T>(value: T) -> Option<MetaField<T>> {
            Some(MetaField {
                value,
                source: MetaSource::Crossref,
            })
        }
        let mut paper = PaperRecord::new();
        paper.meta.title = field(title.to_owned());
        paper.meta.authors = field(vec![Author {
            family: family.to_owned(),
            given: "A.".to_owned(),
        }]);
        paper.meta.year = field(year);
        paper.meta.venue = field("Journal".to_owned());
        paper.cite_key = citekey::generate(&paper.meta, |_| false);
        let id = paper.id.clone();
        store.papers.push(paper);
        id
    }

    #[test]
    fn export_writes_the_header_and_the_entries_in_list_order() {
        let mut store = store();
        let zed = add_paper_with(&mut store, "Zed", 2001, "Alpha work");
        let abel = add_paper_with(&mut store, "Abel", 1999, "Zeta work");
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&zed, &project, true).unwrap();
        store.set_membership(&abel, &project, true).unwrap();
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("refs.bib");

        store.export_project_bib(&project, &path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("% Written by SpeedyF."), "{text}");
        let abel_at = text.find("@article{abel1999zeta,").expect("abel");
        let zed_at = text.find("@article{zed2001alpha,").expect("zed");
        assert!(
            abel_at < zed_at,
            "list order, not membership order:\n{text}"
        );
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "the temp file is gone after persist"
        );
        assert!(
            store.papers.iter().all(|p| p.cite_key_pinned),
            "written: frozen"
        );

        // Nowhere to write: an error, and nothing frozen by it.
        let mut other = self::tests::store();
        let paper = add_paper_with(&mut other, "Solo", 2000, "Only work");
        let project2 = other.create_project("Q").unwrap().id;
        other.set_membership(&paper, &project2, true).unwrap();
        let missing = dir.path().join("no-such-folder").join("refs.bib");
        assert!(other.export_project_bib(&project2, &missing).is_err());
        assert!(other
            .export_project_bib(&project2, Path::new("refs.bib"))
            .is_err());
        assert!(!other.papers[0].cite_key_pinned);
        assert!(other.export_project_bib("nope", &path).is_err());
    }

    #[test]
    fn a_synced_bib_follows_every_save_and_is_rewritten_only_when_it_differs() {
        let app = tempfile::tempdir().expect("app data");
        let mut store = PaperStore::load(app.path().to_path_buf());
        let abel = add_paper_with(&mut store, "Abel", 1999, "Zeta work");
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&abel, &project, true).unwrap();
        let sync_dir = app.path().join("sync");
        fs::create_dir_all(&sync_dir).unwrap();
        let path = sync_dir.join("refs.bib");

        store
            .set_project_bib(&project, Some(path.to_string_lossy().into_owned()))
            .unwrap();
        assert!(
            !path.exists(),
            "recording the path writes nothing by itself"
        );
        store.save().unwrap();
        let first = fs::read_to_string(&path).unwrap();
        assert!(first.starts_with("% Written by SpeedyF."));
        assert!(first.contains("@article{abel1999zeta,"));
        assert!(store.papers[0].cite_key_pinned, "written to a .bib: frozen");

        // The same records again: compared, not rewritten.
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(store.sync_project_bibs().is_empty());
        store.save().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), old);

        // A member's metadata changes: rewritten, under the frozen key.
        store
            .apply_patch(
                &abel,
                MetaPatchDto {
                    year: Some(Some(2000)),
                    ..MetaPatchDto::default()
                },
            )
            .unwrap();
        store.save().unwrap();
        let second = fs::read_to_string(&path).unwrap();
        assert!(second.contains("year = {2000}"), "{second}");
        assert!(second.contains("@article{abel1999zeta,"), "{second}");
        assert_ne!(fs::metadata(&path).unwrap().modified().unwrap(), old);

        // Membership changes: rewritten, and the newcomer's key freezes.
        let baker = add_paper_with(&mut store, "Baker", 2010, "Beta work");
        store.set_membership(&baker, &project, true).unwrap();
        store.save().unwrap();
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("@article{baker2010beta,"));
        assert!(store.papers[1].cite_key_pinned);

        // Hand edits are lost by design.
        fs::write(&path, "hand edited").unwrap();
        store.save().unwrap();
        assert!(fs::read_to_string(&path)
            .unwrap()
            .starts_with("% Written by SpeedyF."));

        // A folder that cannot be written is reported per project and does
        // not fail the save that carries the user's change.
        let gone = app.path().join("gone").join("refs.bib");
        store
            .set_project_bib(&project, Some(gone.to_string_lossy().into_owned()))
            .unwrap();
        let failures = store.sync_project_bibs();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, project);
        store.save().unwrap();
        assert!(!gone.exists());

        // No path: no more syncing, and the file it wrote is left alone.
        store.set_project_bib(&project, None).unwrap();
        fs::write(&path, "left alone").unwrap();
        store.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "left alone");
    }

    #[test]
    fn names_are_trimmed_required_and_unique_ignoring_case() {
        let mut store = store();
        let thesis = store.create_project("  Thesis ").unwrap();
        assert_eq!(thesis.name, "Thesis");
        assert_eq!(thesis.order, 0);
        assert!(store.create_project("   ").is_err());
        let error = store.create_project("thesis").expect_err("duplicate");
        assert!(error.to_string().contains("already exists"), "{error}");
        let active = store.create_project("Active matter").unwrap();
        assert_eq!(active.order, 1);
        assert!(store.rename_project(&active.id, "THESIS").is_err());
        store.rename_project(&active.id, "Active Matter").unwrap();
        store.rename_project(&thesis.id, "thesis").unwrap();
        assert!(store.rename_project("nope", "x").is_err());
        assert_eq!(
            store
                .projects()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["thesis", "Active Matter"]
        );
    }

    #[test]
    fn membership_counts_and_delete_leave_papers_alone() {
        let mut store = store();
        let a = add_paper(&mut store);
        let b = add_paper(&mut store);
        let project = store.create_project("P").unwrap().id;
        let other = store.create_project("Q").unwrap().id;
        store.set_membership(&a, &project, true).unwrap();
        store.set_membership(&a, &project, true).unwrap();
        store.set_membership(&b, &project, true).unwrap();
        store.set_membership(&a, &other, true).unwrap();
        assert!(store.set_membership("nope", &project, true).is_err());
        assert!(store.set_membership(&a, "nope", true).is_err());

        let dto = store.project_dto(&project).unwrap();
        assert_eq!(dto.paper_count, 2);
        assert_eq!(dto.incomplete_count, 2, "empty papers lack everything");
        assert_eq!(store.counts().all, 2);
        assert_eq!(store.counts().unfiled, 0);
        assert_eq!(
            store.paper_dto(&a).unwrap().project_ids,
            vec![project.clone(), other.clone()]
        );

        store.set_membership(&b, &project, false).unwrap();
        assert_eq!(store.project_dto(&project).unwrap().paper_count, 1);
        assert_eq!(store.counts().unfiled, 1);

        store.delete_project(&project).unwrap();
        assert!(store.delete_project(&project).is_err());
        assert_eq!(store.papers.len(), 2, "papers survive");
        assert_eq!(store.projects().len(), 1);
        assert_eq!(store.projects()[0].order, 0, "renumbered");
        assert_eq!(store.paper_dto(&a).unwrap().project_ids, vec![other]);
    }

    #[test]
    fn project_files_are_the_members_files_on_disk() {
        let mut store = store();
        let a = add_paper(&mut store);
        let b = add_paper(&mut store);
        let outside = add_paper(&mut store);
        let file = |path: &str, missing: bool| crate::library::types::PaperFile {
            path: path.to_owned(),
            name: path.to_owned(),
            size: 1,
            mtime_ms: 1,
            fingerprint: None,
            missing,
        };
        let index =
            |store: &PaperStore, id: &str| store.papers.iter().position(|p| p.id == id).unwrap();
        let (ia, ib, io) = (
            index(&store, &a),
            index(&store, &b),
            index(&store, &outside),
        );
        store.papers[ia].files = vec![file("/lib/a.pdf", false), file("/lib/a-v2.pdf", false)];
        store.papers[ib].files = vec![file("/lib/b.pdf", true)];
        store.papers[io].files = vec![file("/lib/o.pdf", false)];
        let project = store.create_project("P").unwrap().id;
        store.set_membership(&a, &project, true).unwrap();
        store.set_membership(&b, &project, true).unwrap();

        let files = store.project_files(&project).unwrap();
        let expected: HashSet<PathBuf> = ["/lib/a.pdf", "/lib/a-v2.pdf"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(files, expected, "members only, missing files left out");
        assert!(store.project_files("nope").is_err());
    }

    #[test]
    fn reorder_ignores_unknown_ids_and_appends_the_rest() {
        let mut store = store();
        let a = store.create_project("A").unwrap().id;
        let b = store.create_project("B").unwrap().id;
        let c = store.create_project("C").unwrap().id;
        store
            .reorder_projects(&[c.clone(), "nope".into(), a.clone()])
            .unwrap();
        let names: Vec<String> = store.projects().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["C", "A", "B"]);
        let orders: Vec<u32> = store.projects().into_iter().map(|p| p.order).collect();
        assert_eq!(orders, vec![0, 1, 2]);
        store.reorder_projects(&[b]).unwrap();
        let names: Vec<String> = store.projects().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["B", "C", "A"]);
    }

    #[test]
    fn bib_path_is_recorded_and_cleared() {
        let mut store = store();
        let id = store.create_project("P").unwrap().id;
        store
            .set_project_bib(&id, Some(" /tmp/refs.bib ".into()))
            .unwrap();
        assert_eq!(
            store.project_dto(&id).unwrap().bib_path.as_deref(),
            Some("/tmp/refs.bib")
        );
        store.set_project_bib(&id, Some("  ".into())).unwrap();
        assert!(store.project_dto(&id).unwrap().bib_path.is_none());
        assert!(store.set_project_bib("nope", None).is_err());
    }
}

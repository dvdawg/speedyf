//! Commands for extensions' project files (`projects:files`, projects.md
//! sec. 13): the open documents every view of a notepad shares, plain file
//! reads and writes, and the hooks that keep the files with their project
//! when it is deleted, restored from a backup, bundled or imported.
//!
//! Every call names the extension it is for and is checked here against
//! the extension's installed grant, as `net.fetch` is: the window's own
//! gate is the first check, this is the second.

use super::library::{read_store, write_store};
use super::EngineState;
use crate::engine::types::Priority;
use crate::engine::Work;
use crate::errors::{AppError, AppResult};
use crate::extensions::Permission;
use crate::library::backup;
use crate::library::bundle;
use crate::library::filing;
use crate::library::projectdocs::{DocEvent, DocKey, Op, OpenedDoc, ProjectDocs};
use crate::library::projectfiles::{self, ProjectFileDto, SnapshotState};
use parking_lot::Mutex;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;

/// The shared side of project files: the open documents, the snapshot
/// series, and the data folder they live in.
pub struct ProjectFiles {
    pub docs: Mutex<ProjectDocs>,
    snapshots: Mutex<SnapshotState>,
    data_dir: Mutex<PathBuf>,
}

pub struct ProjectFilesState(pub Arc<ProjectFiles>);

/// How often due saves are written, and (ten times as long) how often open
/// files are compared with the disk.
const TICK: Duration = Duration::from_millis(100);

fn now_unix() -> u64 {
    crate::library::store::now_unix_secs()
}

impl ProjectFiles {
    /// Start the saver: due saves every `TICK`, a disk check every second,
    /// a snapshot after a save when the hour is up. Every event goes to
    /// every window as `project-file:event`.
    pub fn start(app: tauri::AppHandle, data_dir: PathBuf) -> Arc<Self> {
        let mut docs = ProjectDocs::new();
        let emitter = app.clone();
        docs.set_emitter(Box::new(move |event: DocEvent| {
            let _ = emitter.emit("project-file:event", event);
        }));
        let files = Arc::new(Self {
            docs: Mutex::new(docs),
            snapshots: Mutex::new(SnapshotState::default()),
            data_dir: Mutex::new(data_dir),
        });
        let worker = Arc::clone(&files);
        let spawned = std::thread::Builder::new()
            .name("project-files".into())
            .spawn(move || {
                let mut ticks: u64 = 0;
                loop {
                    std::thread::sleep(TICK);
                    ticks += 1;
                    let dir = worker.data_dir();
                    let saved = worker.docs.lock().save_due(&dir, Instant::now(), false);
                    if !saved.is_empty() {
                        worker.snapshot(false);
                    }
                    if ticks % 10 == 0 {
                        let stamp = backup::stamped_name("", "", now_unix());
                        worker.docs.lock().check_disk(&dir, &stamp);
                    }
                }
            });
        if let Err(error) = spawned {
            log::warn!("cannot start the project files saver: {error}");
        }
        files
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.lock().clone()
    }

    fn snapshot(&self, force: bool) {
        let dir = self.data_dir();
        if let Err(error) = self.snapshots.lock().maybe_write(&dir, now_unix(), force) {
            log::warn!("cannot back up the project files: {error}");
        }
    }

    /// Write every dirty document now.
    pub fn flush(&self) {
        let dir = self.data_dir();
        self.docs.lock().save_due(&dir, Instant::now(), true);
    }

    /// A project was deleted: its open documents are saved and closed, and
    /// every extension's files for it are set aside.
    pub fn project_deleted(&self, project: &str) {
        let dir = self.data_dir();
        self.docs
            .lock()
            .close_where(&dir, |key| key.project_id == project, "deleted");
        if let Err(error) = projectfiles::set_aside(&dir, project) {
            log::warn!("cannot set aside the files of project {project}: {error}");
        }
    }

    /// A library backup written at `at` was restored: put back the project
    /// files nearest that moment (the present snapshotted first) and reload
    /// the open documents.
    pub fn restored(&self, at: u64) {
        self.flush();
        let dir = self.data_dir();
        let restored = {
            let mut snapshots = self.snapshots.lock();
            projectfiles::restore_nearest(&dir, &mut snapshots, at, now_unix())
        };
        if let Err(error) = restored {
            log::warn!("cannot restore the project files: {error}");
        }
        self.docs.lock().reload_all(&dir);
    }

    /// The data folder moved to `to`. Open documents are saved and closed;
    /// when the library was copied there (rather than adopted), its files
    /// are copied too.
    pub fn moved(&self, to: PathBuf, copied_library: bool) {
        let from = self.data_dir();
        self.docs.lock().close_where(&from, |_| true, "moved");
        if copied_library && from != to {
            if let Err(error) = projectfiles::copy_tree_if_absent(&from, &to) {
                log::warn!("cannot copy the project files: {error}");
            }
        }
        *self.data_dir.lock() = to;
        self.snapshots.lock().reset();
    }
}

fn key(extension_id: String, project_id: String, file: String) -> DocKey {
    DocKey {
        extension_id,
        project_id,
        file,
    }
}

/// The extension may use project files now, and the project exists.
async fn check(
    app: &tauri::AppHandle,
    engine: &EngineState,
    extension_id: &str,
    project_id: &str,
    what: &str,
) -> AppResult<()> {
    super::granted_manifest(app, extension_id, Permission::ProjectsFiles, what)?;
    let id = project_id.to_owned();
    let exists = read_store(engine.0.clone(), move |store| {
        Ok(store.projects().iter().any(|project| project.id == id))
    })
    .await?;
    if !exists {
        return Err(AppError::Unsupported(format!(
            "there is no project \"{project_id}\""
        )));
    }
    Ok(())
}

/// Open a project text file for editing (or join it): its text and the
/// revision it is at.
#[tauri::command]
pub async fn project_file_open(
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
    file: String,
) -> AppResult<OpenedDoc> {
    check(&app, &engine, &extension_id, &project_id, "an editor").await?;
    let dir = files.0.data_dir();
    files
        .0
        .docs
        .lock()
        .open(&dir, &key(extension_id, project_id, file))
}

/// One edit from a view, against the revision it last saw. The edit as
/// applied comes back to every window as a `project-file:event`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn project_file_edit(
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
    file: String,
    base_rev: u64,
    from: usize,
    to: usize,
    insert: String,
    origin: String,
) -> AppResult<u64> {
    let (rev, _) = files.0.docs.lock().edit(
        &key(extension_id, project_id, file),
        base_rev,
        Op { from, to, insert },
        &origin,
    )?;
    Ok(rev)
}

#[tauri::command]
pub async fn project_file_close(
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
    file: String,
) -> AppResult<()> {
    files
        .0
        .docs
        .lock()
        .close(&key(extension_id, project_id, file));
    Ok(())
}

/// Save every open document now: a window is closing.
#[tauri::command]
pub async fn project_files_flush(files: tauri::State<'_, ProjectFilesState>) -> AppResult<()> {
    files.0.flush();
    Ok(())
}

/// A file's content: the live text when it is open in an editor.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBody {
    text: Option<String>,
    base64: Option<String>,
}

#[tauri::command]
pub async fn project_file_read(
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
    path: String,
    kind: String,
) -> AppResult<Option<FileBody>> {
    use base64::Engine as _;
    check(
        &app,
        &engine,
        &extension_id,
        &project_id,
        "projectFiles.read",
    )
    .await?;
    let k = key(extension_id, project_id, path);
    if kind == "text" {
        if let Some(text) = files.0.docs.lock().text(&k) {
            return Ok(Some(FileBody {
                text: Some(text),
                base64: None,
            }));
        }
    }
    let dir = files.0.data_dir();
    let Some(bytes) = projectfiles::read(&k.path(&dir)?)? else {
        return Ok(None);
    };
    Ok(Some(if kind == "text" {
        FileBody {
            text: Some(String::from_utf8_lossy(&bytes).into_owned()),
            base64: None,
        }
    } else {
        FileBody {
            text: None,
            base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        }
    }))
}

/// Write a file, text or bytes. A file open in an editor is changed
/// through it, so every view sees the change and keeps its place.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn project_file_write(
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
    path: String,
    text: Option<String>,
    base64: Option<String>,
) -> AppResult<usize> {
    use base64::Engine as _;
    check(
        &app,
        &engine,
        &extension_id,
        &project_id,
        "projectFiles.write",
    )
    .await?;
    let k = key(extension_id, project_id, path);
    let dir = files.0.data_dir();
    let target = k.path(&dir)?;
    let bytes = match (text, base64) {
        (Some(text), None) => {
            let mut docs = files.0.docs.lock();
            if docs.is_open(&k) {
                let written = text.len();
                docs.replace(&k, &text, "extension")?;
                return Ok(written);
            }
            text.into_bytes()
        }
        (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(|_| AppError::Malformed("bytes must be base64".into()))?,
        _ => {
            return Err(AppError::Malformed(
                "write needs exactly one of text or bytes".into(),
            ))
        }
    };
    let written = bytes.len();
    tauri::async_runtime::spawn_blocking(move || projectfiles::write_atomic(&target, &bytes))
        .await
        .map_err(|error| AppError::Internal(format!("write task failed: {error}")))??;
    files.0.snapshot(false);
    Ok(written)
}

#[tauri::command]
pub async fn project_file_list(
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
) -> AppResult<Vec<ProjectFileDto>> {
    check(
        &app,
        &engine,
        &extension_id,
        &project_id,
        "projectFiles.list",
    )
    .await?;
    projectfiles::list(&files.0.data_dir(), &extension_id, &project_id)
}

#[tauri::command]
pub async fn project_file_remove(
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
    files: tauri::State<'_, ProjectFilesState>,
    extension_id: String,
    project_id: String,
    path: String,
) -> AppResult<bool> {
    check(
        &app,
        &engine,
        &extension_id,
        &project_id,
        "projectFiles.remove",
    )
    .await?;
    let k = key(extension_id, project_id, path);
    if files.0.docs.lock().is_open(&k) {
        return Err(AppError::Unsupported(
            "that file is open in an editor".into(),
        ));
    }
    projectfiles::remove(&k.path(&files.0.data_dir())?)
}

/// What an import made, for the inline result.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BundleImportDto {
    pub project_id: String,
    pub project_name: String,
    pub items: u32,
    pub files: u32,
    /// PDFs that could not be filed, by their name in the zip.
    pub failed: Vec<String>,
}

/// Import a project bundle: each PDF is filed into the inbox (copy, then
/// scan, like a dropped file), a project is made with the bundle's name
/// holding them, each bundle cite key the item does not carry is
/// remembered as a former key, and the bundle's project files are written
/// under the new project's id (projects.md sec. 13).
#[tauri::command]
pub async fn library_import_bundle(
    engine: tauri::State<'_, EngineState>,
    files: tauri::State<'_, ProjectFilesState>,
    path: String,
    inbox_dir: String,
) -> AppResult<BundleImportDto> {
    let engine = engine.0.clone();
    let temp = tempfile::tempdir().map_err(|e| AppError::Io(format!("temp dir: {e}")))?;
    let temp_path = temp.path().to_path_buf();
    let source = PathBuf::from(&path);
    let contents =
        tauri::async_runtime::spawn_blocking(move || bundle::read_bundle(&source, &temp_path))
            .await
            .map_err(|error| AppError::Internal(format!("import task failed: {error}")))??;

    let roots = engine.0.library.lock().roots();
    let mut members: Vec<(String, String)> = Vec::new();
    let mut failed = Vec::new();
    for item in &contents.manifest.items {
        let Some(pdf) = item.pdf.as_ref() else {
            continue;
        };
        let Some((_, extracted)) = contents.pdfs.iter().find(|(name, _)| name == pdf) else {
            failed.push(pdf.clone());
            continue;
        };
        let inbox = PathBuf::from(&inbox_dir);
        let from = extracted.clone();
        let roots = roots.clone();
        let filed = tauri::async_runtime::spawn_blocking(move || {
            filing::copy_into_inbox(&from, &inbox, &roots)
        })
        .await
        .map_err(|error| AppError::Internal(format!("import task failed: {error}")))?;
        let filed = match filed {
            Ok(filed) => filed,
            Err(error) => {
                // No inbox is every item failing for one reason: say it once.
                if failed.is_empty() && members.is_empty() {
                    return Err(error);
                }
                failed.push(pdf.clone());
                continue;
            }
        };
        let scanned = engine
            .call_async(Priority::HoverPreview, 0, move |respond| {
                Work::LibraryScanFile {
                    path: filed,
                    respond,
                }
            })
            .await;
        match scanned {
            Ok(paper) => members.push((paper.id, item.cite_key.clone())),
            Err(_) => failed.push(pdf.clone()),
        }
    }

    let name = contents.manifest.project.clone();
    let project = write_store(engine.clone(), move |store| {
        store.import_project(&name, &members)
    })
    .await?;

    let dir = files.0.data_dir();
    let project_id = project.id.clone();
    let entries = contents.files;
    let written = tauri::async_runtime::spawn_blocking(move || {
        let mut written = 0u32;
        for (name, bytes) in entries {
            if let Some(target) = projectfiles::bundle_target(&dir, &project_id, &name) {
                if projectfiles::write_atomic(&target, &bytes).is_ok() {
                    written += 1;
                }
            }
        }
        written
    })
    .await
    .map_err(|error| AppError::Internal(format!("import task failed: {error}")))?;
    files.0.snapshot(false);
    drop(temp);
    Ok(BundleImportDto {
        project_id: project.id,
        project_name: project.name,
        items: project.paper_count,
        files: written,
        failed,
    })
}

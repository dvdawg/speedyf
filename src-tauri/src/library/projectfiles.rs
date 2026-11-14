//! Files extensions keep per project (`projects:files`, projects.md sec. 13).
//!
//! The built-in Notepad keeps a project's notepad here; any extension that
//! holds the permission gets the same treatment. The extension only ever
//! names a relative path; everything a user's data needs is done here, the
//! same way for all of them:
//!
//! - **Where:** `<library data dir>/project-files/<extension id>/<project
//!   id>/<path>`, beside `library.json`, so the files move and sync with the
//!   library.
//! - **Atomic writes:** a temp file in the same folder, renamed over.
//! - **A deleted project** has its folders moved to `<extension
//!   id>/deleted/<project id>/`, never removed.
//! - **Backups:** `backups/project-files-YYYYMMDD-HHMMSS.zip`, a full
//!   snapshot of the tree, at most hourly and only when something changed,
//!   rotated like the library's own backups.
//! - **Bundles:** a project's files travel in its bundle and come back on
//!   import under the new project's id.

use super::backup;
use crate::errors::{AppError, AppResult};
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const ROOT_DIR: &str = "project-files";
/// Inside each extension's folder: the files of projects that were deleted.
pub const DELETED_DIR: &str = "deleted";
/// Largest file an extension may keep. A long notepad is a megabyte; an
/// image of a page region a few hundred kilobytes.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const SNAPSHOT_PREFIX: &str = "project-files-";
const SNAPSHOT_SUFFIX: &str = ".zip";
/// Inflating past this is not one of our snapshots.
const MAX_SNAPSHOT_INFLATED: u64 = 2 * 1024 * 1024 * 1024;

/// One file, for `projectFiles.list`.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFileDto {
    pub path: String,
    pub size: u64,
}

pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join(ROOT_DIR)
}

/// A project id as the library makes them (UUID v4), or anything of the
/// same harmless alphabet; never the name of the deleted folder.
pub fn is_valid_project_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != DELETED_DIR
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `<root>/<extension>/<project>`, after checking both names.
pub fn project_dir(data_dir: &Path, extension: &str, project: &str) -> AppResult<PathBuf> {
    if !crate::extensions::is_valid_id(extension) {
        return Err(AppError::Malformed(format!(
            "bad extension id \"{extension}\""
        )));
    }
    if !is_valid_project_id(project) {
        return Err(AppError::Malformed(format!("bad project id \"{project}\"")));
    }
    Ok(root(data_dir).join(extension).join(project))
}

/// The file `rel` names in an extension's folder for a project. `rel` is
/// checked by `extensions::is_valid_project_file`: plain names only, so it
/// cannot leave the folder.
pub fn file_path(data_dir: &Path, extension: &str, project: &str, rel: &str) -> AppResult<PathBuf> {
    if !crate::extensions::is_valid_project_file(rel) {
        return Err(AppError::Malformed(format!(
            "\"{rel}\" is not a project file path (plain names of letters, digits, . _ -, at most four parts)"
        )));
    }
    let mut path = project_dir(data_dir, extension, project)?;
    for part in rel.split('/') {
        path.push(part);
    }
    Ok(path)
}

/// The bytes of a file, `None` when there is none.
pub fn read(path: &Path) -> AppResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Io(format!(
            "cannot read {}: {error}",
            path.display()
        ))),
    }
}

/// Write `bytes` to `path` atomically: a temp file in the same folder, then
/// a rename over the old one, so a crash leaves the old file or the new one
/// and never half of either.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::Unsupported(format!(
            "a project file is limited to {} MB",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    let dir = path
        .parent()
        .ok_or_else(|| AppError::Io(format!("{} has no parent folder", path.display())))?;
    fs::create_dir_all(dir)
        .map_err(|error| AppError::Io(format!("cannot create {}: {error}", dir.display())))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".speedyf-")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|error| AppError::Io(format!("cannot write in {}: {error}", dir.display())))?;
    temp.write_all(bytes)
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

/// Remove one file; a file that is not there is not an error.
pub fn remove(path: &Path) -> AppResult<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::Io(format!(
            "cannot remove {}: {error}",
            path.display()
        ))),
    }
}

/// Every regular file under `dir`, as (`a/b.png`, absolute path), sorted.
/// Symbolic links and dot files (our own temp files) are skipped.
fn walk(dir: &Path) -> Vec<(String, PathBuf)> {
    fn go(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                go(&entry.path(), &rel, out);
            } else if kind.is_file() {
                out.push((rel, entry.path()));
            }
        }
    }
    let mut out = Vec::new();
    go(dir, "", &mut out);
    out.sort();
    out
}

/// The files an extension keeps for a project.
pub fn list(data_dir: &Path, extension: &str, project: &str) -> AppResult<Vec<ProjectFileDto>> {
    let dir = project_dir(data_dir, extension, project)?;
    Ok(walk(&dir)
        .into_iter()
        .map(|(path, abs)| ProjectFileDto {
            size: fs::metadata(abs).map(|m| m.len()).unwrap_or(0),
            path,
        })
        .collect())
}

/// Set a deleted project's files aside: every extension's folder for it
/// moves to `<extension>/deleted/<project>` (`-2`, `-3`... on a clash).
/// Returns how many folders moved.
pub fn set_aside(data_dir: &Path, project: &str) -> AppResult<usize> {
    if !is_valid_project_id(project) {
        return Ok(0);
    }
    let Ok(extensions) = fs::read_dir(root(data_dir)) else {
        return Ok(0);
    };
    let mut moved = 0;
    for entry in extensions.flatten() {
        let from = entry.path().join(project);
        if !from.is_dir() {
            continue;
        }
        let deleted = entry.path().join(DELETED_DIR);
        fs::create_dir_all(&deleted).map_err(|error| {
            AppError::Io(format!("cannot create {}: {error}", deleted.display()))
        })?;
        let mut to = deleted.join(project);
        let mut n = 2;
        while to.exists() {
            to = deleted.join(format!("{project}-{n}"));
            n += 1;
        }
        fs::rename(&from, &to)
            .map_err(|error| AppError::Io(format!("cannot move {}: {error}", from.display())))?;
        moved += 1;
    }
    Ok(moved)
}

/// FNV over every file's path and bytes: whether the tree changed since
/// the last snapshot.
pub fn tree_hash(data_dir: &Path) -> u64 {
    let mut joined: Vec<u8> = Vec::new();
    for (rel, abs) in walk(&root(data_dir)) {
        joined.extend_from_slice(rel.as_bytes());
        joined.push(0);
        let bytes = fs::read(&abs).unwrap_or_default();
        joined.extend_from_slice(&backup::content_hash(&bytes).to_le_bytes());
    }
    backup::content_hash(&joined)
}

fn zip_error(error: zip::result::ZipError) -> AppError {
    AppError::Io(format!("zip: {error}"))
}

/// The whole tree as a zip, in memory: what a snapshot holds.
pub fn tree_zip(data_dir: &Path) -> AppResult<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (rel, abs) in walk(&root(data_dir)) {
            let bytes = fs::read(&abs)
                .map_err(|error| AppError::Io(format!("cannot read {}: {error}", abs.display())))?;
            zip.start_file(rel.as_str(), options).map_err(zip_error)?;
            zip.write_all(&bytes)
                .map_err(|error| AppError::Io(format!("zip: {error}")))?;
        }
        zip.finish().map_err(zip_error)?;
    }
    Ok(cursor.into_inner())
}

/// Whether a path read out of a zip is one of ours: `<extension id>/` and
/// then plain names, never `..`, never absolute.
fn safe_tree_path(name: &str) -> bool {
    let mut parts = name.split('/');
    let Some(first) = parts.next() else {
        return false;
    };
    let rest: Vec<&str> = parts.collect();
    crate::extensions::is_valid_id(first)
        && !rest.is_empty()
        && rest.len() <= 6
        && rest.iter().all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

/// Unpack a snapshot over the tree: the old tree is moved aside, the
/// snapshot written in its place, and the old one removed only once the new
/// one is whole. An entry that is not a path of ours is skipped.
pub fn restore_tree(data_dir: &Path, zip_bytes: &[u8]) -> AppResult<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(zip_error)?;
    let staging = data_dir.join(format!(".{ROOT_DIR}-restoring"));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
        if entry.is_dir() || !safe_tree_path(&name) {
            continue;
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| AppError::Io(format!("zip: {error}")))?;
        total += bytes.len() as u64;
        if bytes.len() > MAX_FILE_BYTES || total > MAX_SNAPSHOT_INFLATED {
            let _ = fs::remove_dir_all(&staging);
            return Err(AppError::Io(
                "the snapshot is larger than any of ours".into(),
            ));
        }
        let mut path = staging.clone();
        for part in name.split('/') {
            path.push(part);
        }
        write_atomic(&path, &bytes)?;
    }
    let live = root(data_dir);
    let old = data_dir.join(format!(".{ROOT_DIR}-replaced"));
    if old.exists() {
        let _ = fs::remove_dir_all(&old);
    }
    if live.exists() {
        fs::rename(&live, &old)
            .map_err(|error| AppError::Io(format!("cannot move {}: {error}", live.display())))?;
    }
    if staging.exists() {
        fs::rename(&staging, &live)
            .map_err(|error| AppError::Io(format!("cannot restore {}: {error}", live.display())))?;
    }
    if old.exists() {
        let _ = fs::remove_dir_all(&old);
    }
    Ok(())
}

/// Snapshots in `dir`, newest first.
pub fn snapshots_in(dir: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            backup::parse_stamped(SNAPSHOT_PREFIX, SNAPSHOT_SUFFIX, &name).map(|at| (name, at))
        })
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    found
}

fn rotate_snapshots(dir: &Path, now: u64) {
    let snapshots = snapshots_in(dir);
    let times: Vec<u64> = snapshots.iter().map(|(_, at)| *at).collect();
    let keep = backup::keep_set(&times, now);
    for (name, at) in snapshots {
        if !keep.contains(&at) {
            if let Err(error) = fs::remove_file(dir.join(&name)) {
                log::warn!("cannot remove the old project-files backup {name}: {error}");
            }
        }
    }
}

/// Write the tree as the snapshot for `at` (a second later per clash).
fn write_snapshot(dir: &Path, at: u64, bytes: &[u8]) -> AppResult<String> {
    let mut at = at;
    while dir
        .join(backup::stamped_name(SNAPSHOT_PREFIX, SNAPSHOT_SUFFIX, at))
        .exists()
    {
        at += 1;
    }
    let name = backup::stamped_name(SNAPSHOT_PREFIX, SNAPSHOT_SUFFIX, at);
    let path = dir.join(&name);
    fs::create_dir_all(dir)
        .map_err(|error| AppError::Io(format!("cannot create {}: {error}", dir.display())))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".project-files-backup")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|error| AppError::Io(format!("cannot write in {}: {error}", dir.display())))?;
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| AppError::Io(format!("cannot write a backup: {error}")))?;
    temp.persist(&path).map_err(|error| {
        AppError::Io(format!("cannot write {}: {}", path.display(), error.error))
    })?;
    Ok(name)
}

/// The project files' side of the backups: when the last snapshot was
/// written and what it held, so one is written at most hourly and never
/// twice for the same content.
#[derive(Default)]
pub struct SnapshotState {
    primed: bool,
    last_at: Option<u64>,
    last_hash: Option<u64>,
}

impl SnapshotState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Snapshot the tree of `data_dir` into its backups folder if the last
    /// snapshot is an hour old (or `force`) and the tree changed since.
    /// Nothing is written for an empty tree that never had a snapshot.
    pub fn maybe_write(
        &mut self,
        data_dir: &Path,
        now: u64,
        force: bool,
    ) -> AppResult<Option<String>> {
        let dir = backup::dir_of(data_dir);
        if !self.primed {
            self.primed = true;
            if let Some((name, at)) = snapshots_in(&dir).into_iter().next() {
                self.last_at = Some(at);
                self.last_hash = fs::read(dir.join(name)).ok().and_then(|bytes| {
                    // The hash is of the tree, so restore it into nothing to
                    // compare would be wasteful; the zip's own entries are
                    // hashed the same way instead.
                    zip_tree_hash(&bytes)
                });
            }
        }
        let recent = self
            .last_at
            .is_some_and(|at| at <= now && now - at < backup::BACKUP_INTERVAL_SECS);
        if recent && !force {
            return Ok(None);
        }
        let files = walk(&root(data_dir));
        if files.is_empty() && self.last_at.is_none() {
            return Ok(None);
        }
        let hash = tree_hash(data_dir);
        if self.last_hash == Some(hash) {
            return Ok(None);
        }
        let bytes = tree_zip(data_dir)?;
        let name = write_snapshot(&dir, now, &bytes)?;
        self.last_at = Some(now);
        self.last_hash = Some(hash);
        rotate_snapshots(&dir, now);
        Ok(Some(name))
    }
}

/// `tree_hash` computed from a snapshot's entries rather than the disk.
fn zip_tree_hash(zip_bytes: &[u8]) -> Option<u64> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).ok()?;
    let mut entries: Vec<(String, u64)> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).ok()?;
        if entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        entries.push((
            entry.name().map(|n| n.into_owned()).unwrap_or_default(),
            backup::content_hash(&bytes),
        ));
    }
    entries.sort();
    let mut joined: Vec<u8> = Vec::new();
    for (rel, hash) in entries {
        joined.extend_from_slice(rel.as_bytes());
        joined.push(0);
        joined.extend_from_slice(&hash.to_le_bytes());
    }
    Some(backup::content_hash(&joined))
}

/// Put back the snapshot nearest in time to `at` (the moment of a library
/// backup being restored), after snapshotting the present so the restore can
/// itself be undone. Returns the snapshot used, `None` when there is none.
pub fn restore_nearest(
    data_dir: &Path,
    state: &mut SnapshotState,
    at: u64,
    now: u64,
) -> AppResult<Option<String>> {
    let dir = backup::dir_of(data_dir);
    let Some((name, _)) = snapshots_in(&dir)
        .into_iter()
        .min_by_key(|(_, written)| (written.abs_diff(at), *written))
    else {
        return Ok(None);
    };
    let bytes = fs::read(dir.join(&name))
        .map_err(|error| AppError::Io(format!("cannot read {name}: {error}")))?;
    // Before touching anything: the present, whatever the hour.
    state.maybe_write(data_dir, now, true)?;
    restore_tree(data_dir, &bytes)?;
    Ok(Some(name))
}

/// A project's files for its bundle: (`project-files/<extension>/<path>`,
/// absolute path), every extension's.
pub fn bundle_entries(data_dir: &Path, project: &str) -> Vec<(String, PathBuf)> {
    if !is_valid_project_id(project) {
        return Vec::new();
    }
    let Ok(extensions) = fs::read_dir(root(data_dir)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut names: Vec<String> = extensions
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| crate::extensions::is_valid_id(name))
        .collect();
    names.sort();
    for extension in names {
        let dir = root(data_dir).join(&extension).join(project);
        for (rel, abs) in walk(&dir) {
            out.push((format!("{ROOT_DIR}/{extension}/{rel}"), abs));
        }
    }
    out
}

/// Where a bundle entry named `project-files/<extension>/<path>` goes for
/// `project`; `None` for any other name or a path that is not a project
/// file's.
pub fn bundle_target(data_dir: &Path, project: &str, entry: &str) -> Option<PathBuf> {
    let rest = entry.strip_prefix(&format!("{ROOT_DIR}/"))?;
    let (extension, rel) = rest.split_once('/')?;
    file_path(data_dir, extension, project, rel).ok()
}

/// Copy the whole tree from one data folder to another that has none: the
/// data folder moved and the library was copied, so its files come too.
pub fn copy_tree_if_absent(from: &Path, to: &Path) -> AppResult<usize> {
    let target = root(to);
    if target.exists() {
        return Ok(0);
    }
    let mut copied = 0;
    for (rel, abs) in walk(&root(from)) {
        let bytes = fs::read(&abs)
            .map_err(|error| AppError::Io(format!("cannot read {}: {error}", abs.display())))?;
        let mut path = target.clone();
        for part in rel.split('/') {
            path.push(part);
        }
        write_atomic(&path, &bytes)?;
        copied += 1;
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXT: &str = "dev.speedyf.notepad";
    const PROJECT: &str = "3b9d0c1e-0000-4000-8000-000000000001";

    #[test]
    fn paths_stay_inside_the_project_folder() {
        let data = Path::new("/data");
        let ok = file_path(data, EXT, PROJECT, "assets/fig-1.png").unwrap();
        assert_eq!(
            ok,
            data.join(ROOT_DIR)
                .join(EXT)
                .join(PROJECT)
                .join("assets")
                .join("fig-1.png")
        );
        for bad in [
            "../x.md",
            "/etc/passwd",
            "a//b",
            ".hidden",
            "a/../b",
            "",
            "x y.md",
        ] {
            assert!(file_path(data, EXT, PROJECT, bad).is_err(), "{bad}");
        }
        assert!(file_path(data, "Bad", PROJECT, "a.md").is_err());
        assert!(file_path(data, EXT, "deleted", "a.md").is_err());
        assert!(file_path(data, EXT, "../x", "a.md").is_err());
    }

    #[test]
    fn writes_are_atomic_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_path(dir.path(), EXT, PROJECT, "notepad.md").unwrap();
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(read(&path).unwrap().unwrap(), b"two");
        // No temp file is left beside it.
        assert_eq!(
            list(dir.path(), EXT, PROJECT).unwrap(),
            vec![ProjectFileDto {
                path: "notepad.md".into(),
                size: 3
            }]
        );
        let big = vec![b'x'; MAX_FILE_BYTES + 1];
        assert!(write_atomic(&path, &big).is_err());
        assert_eq!(read(&path).unwrap().unwrap(), b"two");
    }

    #[test]
    fn a_deleted_projects_files_are_set_aside_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_path(dir.path(), EXT, PROJECT, "notepad.md").unwrap();
        write_atomic(&path, b"kept").unwrap();
        assert_eq!(set_aside(dir.path(), PROJECT).unwrap(), 1);
        assert!(!path.exists());
        let kept = root(dir.path())
            .join(EXT)
            .join(DELETED_DIR)
            .join(PROJECT)
            .join("notepad.md");
        assert_eq!(fs::read(&kept).unwrap(), b"kept");
        // A second deletion of the same id does not overwrite the first.
        write_atomic(&path, b"again").unwrap();
        set_aside(dir.path(), PROJECT).unwrap();
        let second = root(dir.path())
            .join(EXT)
            .join(DELETED_DIR)
            .join(format!("{PROJECT}-2"))
            .join("notepad.md");
        assert_eq!(fs::read(second).unwrap(), b"again");
        assert_eq!(fs::read(kept).unwrap(), b"kept");
    }

    #[test]
    fn snapshots_are_hourly_changed_only_and_restore_the_nearest() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let note = file_path(data, EXT, PROJECT, "notepad.md").unwrap();
        let asset = file_path(data, EXT, PROJECT, "assets/eq.png").unwrap();
        let mut state = SnapshotState::default();
        let now = 1_800_000_000;
        // Nothing to back up yet.
        assert_eq!(state.maybe_write(data, now, false).unwrap(), None);
        write_atomic(&note, "v1 \u{3b1}".as_bytes()).unwrap();
        write_atomic(&asset, &[0x89, b'P', b'N', b'G']).unwrap();
        let first = state.maybe_write(data, now, false).unwrap().unwrap();
        // Within the hour: nothing.
        write_atomic(&note, b"v2").unwrap();
        assert!(state.maybe_write(data, now + 60, false).unwrap().is_none());
        // Later and changed: a second one.
        let second = state.maybe_write(data, now + 7200, false).unwrap().unwrap();
        assert_ne!(first, second);
        // Later, unchanged: nothing, also for a fresh state reading the disk.
        assert!(state
            .maybe_write(data, now + 14_400, false)
            .unwrap()
            .is_none());
        let mut fresh = SnapshotState::default();
        assert!(fresh
            .maybe_write(data, now + 14_400, false)
            .unwrap()
            .is_none());

        write_atomic(&note, b"v3").unwrap();
        // Restore the one nearest the first library backup's time.
        let used = restore_nearest(data, &mut fresh, now + 10, now + 20_000)
            .unwrap()
            .unwrap();
        assert_eq!(used, first);
        assert_eq!(fs::read_to_string(&note).unwrap(), "v1 \u{3b1}");
        assert_eq!(fs::read(&asset).unwrap(), vec![0x89, b'P', b'N', b'G']);
        // The present was snapshotted first, so v3 can come back.
        let newest = snapshots_in(&backup::dir_of(data)).remove(0);
        restore_tree(
            data,
            &fs::read(backup::dir_of(data).join(newest.0)).unwrap(),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&note).unwrap(), "v3");
    }

    #[test]
    fn a_hostile_snapshot_cannot_write_outside_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for name in ["../escape.md", "dev.speedyf.notepad/../../x", "Bad/x.md"] {
                zip.start_file(name, options).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.start_file("dev.speedyf.notepad/p1/notepad.md", options)
                .unwrap();
            zip.write_all(b"ok").unwrap();
            zip.finish().unwrap();
        }
        restore_tree(dir.path(), &cursor.into_inner()).unwrap();
        assert!(!dir.path().join("escape.md").exists());
        assert!(!dir.path().parent().unwrap().join("x").exists());
        assert_eq!(
            fs::read_to_string(root(dir.path()).join(EXT).join("p1").join("notepad.md")).unwrap(),
            "ok"
        );
    }

    #[test]
    fn bundle_entries_round_trip_to_a_new_project() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        write_atomic(
            &file_path(data, EXT, PROJECT, "notepad.md").unwrap(),
            b"# n",
        )
        .unwrap();
        write_atomic(
            &file_path(data, EXT, PROJECT, "assets/a.png").unwrap(),
            b"png",
        )
        .unwrap();
        let entries = bundle_entries(data, PROJECT);
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "project-files/dev.speedyf.notepad/assets/a.png",
                "project-files/dev.speedyf.notepad/notepad.md"
            ]
        );
        let target = bundle_target(data, "p2", names[1]).unwrap();
        assert_eq!(target, root(data).join(EXT).join("p2").join("notepad.md"));
        assert!(bundle_target(data, "p2", "project-files/dev.speedyf.notepad/../x").is_none());
        assert!(bundle_target(data, "p2", "pdfs/a.pdf").is_none());
    }
}

//! Local-only citation library indexing and resolution. Directory traversal
//! and persistence are thread-safe, while PDF parsing is deliberately left to
//! the engine worker that owns PDFium.

use crate::engine::citation::{citation_from_filename, find_explicit_citations};
use crate::engine::types::{CitationIdDto, LibraryStatusDto};
use crate::errors::{AppError, AppResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

const INDEX_FILE: &str = "library-index.json";
const ROOT_FILE: &str = "library-root.json";

pub trait CitationResolver: Send + Sync {
    fn resolve(&self, id: &CitationIdDto) -> Option<PathBuf>;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub mtime_ms: u64,
    pub size: u64,
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    pub title: Option<String>,
    /// Whether this file's full text has been extracted to a sidecar for
    /// library-wide search. Defaulted so an index written before search
    /// existed still loads — it simply reports nothing indexed yet.
    #[serde(default)]
    pub text_indexed: bool,
}

/// The shape written to disk today. Both persisted files carry a version now:
/// they had none, and an unrecognized shape was silently discarded along with
/// the whole index.
const FORMAT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct RootConfig {
    #[serde(default)]
    version: u32,
    /// v2 and later. Several folders, because papers are never all in one.
    #[serde(default)]
    roots: Vec<PathBuf>,
    /// v1. Read on load so an existing library survives the upgrade, never
    /// written.
    #[serde(default)]
    root: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: Vec<LibraryEntry>,
}

#[derive(Clone, Debug)]
pub struct ScanCandidate {
    pub path: PathBuf,
    pub mtime_ms: u64,
    pub size: u64,
    pub epoch: u64,
    pub unchanged: bool,
}

pub struct LibraryState {
    app_data_dir: PathBuf,
    roots: Vec<PathBuf>,
    /// Keyed by path. Was a `Vec` with a linear containment check per scanned
    /// file, which is O(n²) across a scan — tolerable for one folder, not for
    /// several.
    entries: HashMap<PathBuf, LibraryEntry>,
    pending: VecDeque<PathBuf>,
    processed: u32,
    total: u32,
    scanning: bool,
    scan_epoch: u64,
}

impl LibraryState {
    pub fn load(app_data_dir: PathBuf) -> Self {
        if let Err(error) = fs::create_dir_all(&app_data_dir) {
            log::warn!("cannot create app-data directory for citation library: {error}");
        }
        let configured: Vec<PathBuf> = fs::read(app_data_dir.join(ROOT_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RootConfig>(&bytes).ok())
            .map(|config| {
                // A v1 file names a single root; carry it forward rather than
                // making the user pick their folder again.
                let mut roots = config.roots;
                if let Some(legacy) = config.root {
                    if !roots.contains(&legacy) {
                        roots.push(legacy);
                    }
                }
                roots
            })
            .unwrap_or_default()
            .iter()
            .filter_map(|root| canonical_directory(root).ok())
            .collect();

        let stored = match fs::read(app_data_dir.join(INDEX_FILE)) {
            Ok(bytes) => read_index(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                log::warn!("cannot read citation library index: {error}");
                Vec::new()
            }
        };
        // Drop anything that has moved out of the roots or changed on disk, so
        // a stale entry can never resolve a citation or answer a search.
        let entries: HashMap<PathBuf, LibraryEntry> = stored
            .into_iter()
            .filter(|entry| {
                contained_in_any(&configured, &entry.path)
                    .and_then(|path| file_fingerprint(&path).map(|fingerprint| (path, fingerprint)))
                    .is_some_and(|(path, (mtime_ms, size))| {
                        path == entry.path && mtime_ms == entry.mtime_ms && size == entry.size
                    })
            })
            .map(|entry| (entry.path.clone(), entry))
            .collect();

        let mut state = Self {
            app_data_dir,
            roots: configured,
            entries,
            pending: VecDeque::new(),
            processed: 0,
            total: 0,
            scanning: false,
            scan_epoch: 0,
        };
        state.schedule_scan();
        state
    }

    pub fn status(&self) -> LibraryStatusDto {
        let cap = |value: usize| value.min(u32::MAX as usize) as u32;
        LibraryStatusDto {
            roots: self
                .roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect(),
            indexed: if self.scanning {
                self.processed
            } else {
                cap(self.entries.len())
            },
            total: self.total,
            scanning: self.scanning,
            text_indexed: cap(self.entries.values().filter(|e| e.text_indexed).count()),
            text_total: cap(self.entries.len()),
        }
    }

    pub fn app_data_dir(&self) -> PathBuf {
        self.app_data_dir.clone()
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    /// The canonical form of `path` if it lies inside any root, else None.
    /// Re-checked at use time, not only at scan time, so an entry that has
    /// since moved outside the library cannot be opened through it.
    pub fn contains(&self, path: &Path) -> Option<PathBuf> {
        contained_in_any(&self.roots, path)
    }

    /// Add a folder to the library. Re-adding one already present just
    /// restarts an idle scan, which is how a manual refresh is spelled.
    pub fn add_root(&mut self, path: PathBuf) -> AppResult<bool> {
        let root = canonical_directory(&path)?;
        if !self.roots.contains(&root) {
            self.roots.push(root);
            self.persist_root();
        }
        self.schedule_scan();
        Ok(self.scanning)
    }

    /// Remove one folder. Only its own documents go: the whole index used to
    /// be cleared whenever the root changed, which with several folders would
    /// throw away work that is still perfectly good.
    pub fn remove_root(&mut self, path: &Path) -> AppResult<bool> {
        let target = canonical_directory(path).unwrap_or_else(|_| path.to_path_buf());
        self.roots.retain(|root| root != &target);
        let dropped: Vec<PathBuf> = self
            .entries
            .keys()
            .filter(|entry| contained_in_any(&self.roots, entry).is_none())
            .cloned()
            .collect();
        for path in dropped {
            self.entries.remove(&path);
            // The text index outlives the entry otherwise, and nothing would
            // ever come back to clean it up.
            crate::search::librarytext::remove_sidecar(&self.app_data_dir, &path);
        }
        self.persist_root();
        self.persist_index();
        self.schedule_scan();
        Ok(self.scanning)
    }

    fn schedule_scan(&mut self) {
        self.scan_epoch = self.scan_epoch.wrapping_add(1);
        self.pending.clear();
        self.processed = 0;
        let mut files: Vec<PathBuf> = self
            .roots
            .iter()
            .flat_map(|root| collect_pdfs(root))
            .collect();
        // Nested roots would otherwise queue the same file twice.
        files.sort();
        files.dedup();
        self.total = files.len().min(u32::MAX as usize) as u32;
        self.pending = files.into();
        self.scanning = !self.pending.is_empty();
        if !self.scanning {
            self.persist_index();
        }
    }

    pub fn next_candidate(&mut self) -> Option<ScanCandidate> {
        while let Some(path) = self.pending.pop_front() {
            let Some((mtime_ms, size)) = file_fingerprint(&path) else {
                self.processed = self.processed.saturating_add(1);
                continue;
            };
            let unchanged = self
                .entries
                .get(&path)
                .is_some_and(|entry| entry.mtime_ms == mtime_ms && entry.size == size);
            return Some(ScanCandidate {
                path,
                mtime_ms,
                size,
                epoch: self.scan_epoch,
                unchanged,
            });
        }
        self.scanning = false;
        self.persist_index();
        None
    }

    pub fn finish_candidate(&mut self, candidate: &ScanCandidate, entry: Option<LibraryEntry>) {
        if candidate.epoch != self.scan_epoch {
            return;
        }
        if let Some(entry) = entry {
            // A re-scanned file is a changed file: whatever text was extracted
            // from the old bytes no longer describes it. The sidecar would be
            // rejected on load anyway once the fingerprint stops matching, but
            // deleting it now keeps the index from growing a copy per edit.
            crate::search::librarytext::remove_sidecar(&self.app_data_dir, &entry.path);
            self.entries.insert(entry.path.clone(), entry);
        }
        self.processed = self.processed.saturating_add(1).min(self.total);
        if self.processed % 25 == 0 {
            self.persist_index();
        }
        if self.pending.is_empty() {
            self.scanning = false;
            self.persist_index();
        }
    }

    pub fn has_scan_work(&self) -> bool {
        self.scanning && !self.pending.is_empty()
    }

    pub fn entry_for_path(&self, path: &Path) -> Option<LibraryEntry> {
        self.entries.get(path).cloned()
    }

    /// Every document currently in the library, for a search to stream over.
    pub fn all_entries(&self) -> Vec<LibraryEntry> {
        let mut entries: Vec<LibraryEntry> = self.entries.values().cloned().collect();
        // A stable order keeps results from reshuffling between identical
        // queries, which a HashMap alone would not.
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    /// The next document whose text has not been extracted yet.
    pub fn next_text_candidate(&self) -> Option<LibraryEntry> {
        self.all_entries().into_iter().find(|e| !e.text_indexed)
    }

    /// Record that a document's text is now on disk — or that extracting it
    /// failed, which is also final: a file that cannot be read will not start
    /// reading, and retrying it forever would stall every document behind it.
    pub fn finish_text(&mut self, path: &Path) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.text_indexed = true;
        }
        self.persist_index();
    }

    pub fn has_text_work(&self) -> bool {
        self.entries.values().any(|entry| !entry.text_indexed)
    }

    fn persist_root(&self) {
        let data = RootConfig {
            version: FORMAT_VERSION,
            roots: self.roots.clone(),
            root: None,
        };
        match serde_json::to_vec_pretty(&data) {
            Ok(bytes) => {
                if let Err(error) = fs::write(self.app_data_dir.join(ROOT_FILE), bytes) {
                    log::warn!("cannot persist citation library root: {error}");
                }
            }
            Err(error) => log::warn!("cannot serialize citation library root: {error}"),
        }
    }

    fn persist_index(&self) {
        let data = IndexFile {
            version: FORMAT_VERSION,
            entries: self.all_entries(),
        };
        match serde_json::to_vec_pretty(&data) {
            Ok(bytes) => {
                if let Err(error) = fs::write(self.app_data_dir.join(INDEX_FILE), bytes) {
                    log::warn!("cannot persist citation library index: {error}");
                }
            }
            Err(error) => log::warn!("cannot serialize citation library index: {error}"),
        }
    }
}

pub struct LocalLibraryResolver {
    state: Arc<Mutex<LibraryState>>,
}

impl LocalLibraryResolver {
    pub fn new(state: Arc<Mutex<LibraryState>>) -> Self {
        Self { state }
    }
}

impl CitationResolver for LocalLibraryResolver {
    fn resolve(&self, id: &CitationIdDto) -> Option<PathBuf> {
        let state = self.state.lock();
        let entry = state.entries.values().find(|entry| match id {
            CitationIdDto::Doi(value) => entry.doi.as_ref() == Some(value),
            CitationIdDto::ArXiv(value) => entry.arxiv.as_ref() == Some(value),
        })?;
        state.contains(&entry.path)
    }
}

/// Read a persisted index, accepting both the versioned envelope and the bare
/// array that shipped before it.
fn read_index(bytes: &[u8]) -> Vec<LibraryEntry> {
    if let Ok(file) = serde_json::from_slice::<IndexFile>(bytes) {
        if file.version > FORMAT_VERSION {
            // Written by a newer build. Refusing beats guessing at a shape we
            // do not know; a rescan costs time, a misread costs correctness.
            log::warn!(
                "citation library index is version {}; this build understands {FORMAT_VERSION}",
                file.version
            );
            return Vec::new();
        }
        if !file.entries.is_empty() || file.version > 0 {
            return file.entries;
        }
    }
    match serde_json::from_slice::<Vec<LibraryEntry>>(bytes) {
        Ok(entries) => entries,
        Err(error) => {
            log::warn!("discarding corrupt citation library index: {error}");
            Vec::new()
        }
    }
}

/// The canonical form of `path` if it lies inside any of `roots`.
pub fn contained_in_any(roots: &[PathBuf], path: &Path) -> Option<PathBuf> {
    roots
        .iter()
        .find_map(|root| contained_canonical_path(root, path))
}

pub fn entry_from_filename(candidate: &ScanCandidate) -> Option<LibraryEntry> {
    let name = candidate.path.file_name()?.to_string_lossy();
    let id = citation_from_filename(&name)?;
    let (doi, arxiv) = match id {
        CitationIdDto::Doi(value) => (Some(value), None),
        CitationIdDto::ArXiv(value) => (None, Some(value)),
    };
    Some(LibraryEntry {
        path: candidate.path.clone(),
        mtime_ms: candidate.mtime_ms,
        size: candidate.size,
        doi,
        arxiv,
        title: None,
        text_indexed: false,
    })
}

pub fn entry_from_first_page(candidate: &ScanCandidate, text: &str) -> LibraryEntry {
    let mut doi = None;
    let mut arxiv = None;
    for occurrence in find_explicit_citations(text) {
        match occurrence.id {
            CitationIdDto::Doi(value) if doi.is_none() => doi = Some(value),
            CitationIdDto::ArXiv(value) if arxiv.is_none() => arxiv = Some(value),
            _ => {}
        }
        if doi.is_some() && arxiv.is_some() {
            break;
        }
    }
    let title = text
        .lines()
        .map(str::trim)
        .find(|line| line.chars().count() > 12)
        .map(|line| line.chars().take(300).collect());
    LibraryEntry {
        path: candidate.path.clone(),
        mtime_ms: candidate.mtime_ms,
        size: candidate.size,
        doi,
        arxiv,
        title,
        text_indexed: false,
    }
}

fn canonical_directory(path: &Path) -> AppResult<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|error| AppError::Io(format!("cannot open library folder: {error}")))?;
    if !canonical.is_dir() {
        return Err(AppError::Io(format!(
            "not an existing directory: {}",
            path.display()
        )));
    }
    Ok(canonical)
}

pub fn contained_canonical_path(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().ok()?;
    let canonical_candidate = candidate.canonicalize().ok()?;
    (canonical_candidate.is_file() && canonical_candidate.starts_with(&canonical_root))
        .then_some(canonical_candidate)
}

fn file_fingerprint(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mtime_ms = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .min(u64::MAX as u128) as u64;
    Some((mtime_ms, metadata.len()))
}

fn collect_pdfs(root: &Path) -> Vec<PathBuf> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            {
                if let Some(path) = contained_canonical_path(root, &path) {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_the_selected_root() {
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        let inside_file = root.path().join("inside.pdf");
        let outside_file = outside.path().join("outside.pdf");
        fs::write(&inside_file, b"pdf").expect("inside fixture");
        fs::write(&outside_file, b"pdf").expect("outside fixture");

        assert!(contained_canonical_path(root.path(), &inside_file).is_some());
        assert!(contained_canonical_path(root.path(), &outside_file).is_none());
    }

    #[test]
    fn first_page_metadata_extracts_title_and_both_id_schemes() {
        let candidate = ScanCandidate {
            path: PathBuf::from("paper.pdf"),
            mtime_ms: 1,
            size: 2,
            epoch: 3,
            unchanged: false,
        };
        let entry = entry_from_first_page(
            &candidate,
            "A sufficiently descriptive paper title\ndoi:10.1000/ABC\narXiv:2401.12345v2",
        );
        assert_eq!(
            entry.title.as_deref(),
            Some("A sufficiently descriptive paper title")
        );
        assert_eq!(entry.doi.as_deref(), Some("10.1000/abc"));
        assert_eq!(entry.arxiv.as_deref(), Some("2401.12345"));
    }

    #[test]
    fn corrupt_index_is_discarded_and_scheduled_for_rescan() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("library root");
        fs::write(root.path().join("2401.12345.pdf"), b"fixture").expect("library fixture");
        // The v1 shape, written literally: this is what is actually on disk
        // for anyone who used the library before it took several folders.
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "root": root.path() }).to_string(),
        )
        .unwrap();
        fs::write(app_data.path().join(INDEX_FILE), b"{broken json").unwrap();

        let state = LibraryState::load(app_data.path().to_path_buf());

        assert!(state.status().scanning);
        assert_eq!(state.status().total, 1);
        assert!(state.entries.is_empty());
    }

    #[test]
    fn a_single_root_config_survives_the_upgrade() {
        // Losing it would silently empty someone's library and make them go
        // find the folder again.
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("library root");
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "root": root.path() }).to_string(),
        )
        .unwrap();

        let state = LibraryState::load(app_data.path().to_path_buf());
        assert_eq!(state.roots().len(), 1);
        assert_eq!(state.status().roots.len(), 1);
    }

    #[test]
    fn an_index_from_a_newer_build_is_refused_not_misread() {
        let app_data = tempfile::tempdir().expect("app data");
        fs::write(
            app_data.path().join(INDEX_FILE),
            serde_json::json!({ "version": 999, "entries": [] }).to_string(),
        )
        .unwrap();
        let state = LibraryState::load(app_data.path().to_path_buf());
        assert!(state.entries.is_empty());
    }

    #[test]
    fn a_bare_array_index_still_loads() {
        // What every existing install has on disk right now.
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("library root");
        let pdf = root.path().join("paper.pdf");
        fs::write(&pdf, b"fixture").unwrap();
        // Stored entries are always canonical — `collect_pdfs` re-validates
        // every hit through `contained_canonical_path`. On macOS a tempdir is
        // /var/... which canonicalizes to /private/var/..., so a fixture that
        // skipped this would look like a file that had moved away.
        let pdf = pdf.canonicalize().expect("canonical");
        let (mtime_ms, size) = file_fingerprint(&pdf).expect("fingerprint");
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "root": root.path() }).to_string(),
        )
        .unwrap();
        fs::write(
            app_data.path().join(INDEX_FILE),
            serde_json::json!([{
                "path": pdf,
                "mtime_ms": mtime_ms,
                "size": size,
                "doi": null,
                "arxiv": null,
                "title": "A paper"
            }])
            .to_string(),
        )
        .unwrap();

        let state = LibraryState::load(app_data.path().to_path_buf());
        let entry = state.entry_for_path(&pdf).expect("entry survived");
        assert_eq!(entry.title.as_deref(), Some("A paper"));
        // ...and it has no text yet, so the search pass will pick it up.
        assert!(!entry.text_indexed);
    }

    #[test]
    fn each_root_keeps_its_own_documents() {
        let app_data = tempfile::tempdir().expect("app data");
        let first = tempfile::tempdir().expect("first root");
        let second = tempfile::tempdir().expect("second root");
        fs::write(first.path().join("a.pdf"), b"fixture").unwrap();
        fs::write(second.path().join("b.pdf"), b"fixture").unwrap();

        let mut state = LibraryState::load(app_data.path().to_path_buf());
        state.add_root(first.path().to_path_buf()).expect("first");
        state.add_root(second.path().to_path_buf()).expect("second");
        assert_eq!(state.roots().len(), 2);
        assert_eq!(state.status().total, 2);

        // Adding the same folder twice is a refresh, not a duplicate.
        state.add_root(second.path().to_path_buf()).expect("again");
        assert_eq!(state.roots().len(), 2);

        // Pretend both were scanned.
        for (name, dir) in [("a.pdf", first.path()), ("b.pdf", second.path())] {
            let path = dir.join(name);
            let (mtime_ms, size) = file_fingerprint(&path).expect("fingerprint");
            state.entries.insert(
                path.clone(),
                LibraryEntry {
                    path,
                    mtime_ms,
                    size,
                    doi: None,
                    arxiv: None,
                    title: None,
                    text_indexed: true,
                },
            );
        }

        state.remove_root(first.path()).expect("remove");
        assert_eq!(state.roots().len(), 1);
        let remaining = state.all_entries();
        assert_eq!(
            remaining.len(),
            1,
            "only the removed root's files should go"
        );
        assert!(remaining[0].path.ends_with("b.pdf"));
    }

    #[test]
    fn a_path_outside_every_root_is_not_in_the_library() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(root.path().join("in.pdf"), b"fixture").unwrap();
        fs::write(outside.path().join("out.pdf"), b"fixture").unwrap();

        let mut state = LibraryState::load(app_data.path().to_path_buf());
        state.add_root(root.path().to_path_buf()).expect("add");
        assert!(state.contains(&root.path().join("in.pdf")).is_some());
        assert!(state.contains(&outside.path().join("out.pdf")).is_none());
    }
}

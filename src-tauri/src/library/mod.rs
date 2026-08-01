//! Local-only citation library indexing and resolution. Directory traversal
//! and persistence are thread-safe, while PDF parsing is deliberately left to
//! the engine worker that owns PDFium.

use crate::engine::citation::{citation_from_filename, find_explicit_citations};
use crate::engine::types::{CitationIdDto, LibraryStatusDto};
use crate::errors::{AppError, AppResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
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
}

#[derive(Serialize, Deserialize, Default)]
struct RootConfig {
    root: Option<PathBuf>,
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
    root: Option<PathBuf>,
    entries: Vec<LibraryEntry>,
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
        let configured = fs::read(app_data_dir.join(ROOT_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RootConfig>(&bytes).ok())
            .and_then(|config| config.root)
            .and_then(|root| canonical_directory(&root).ok());

        let mut entries = match fs::read(app_data_dir.join(INDEX_FILE)) {
            Ok(bytes) => match serde_json::from_slice::<Vec<LibraryEntry>>(&bytes) {
                Ok(entries) => entries,
                Err(error) => {
                    log::warn!("discarding corrupt citation library index: {error}");
                    Vec::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                log::warn!("cannot read citation library index: {error}");
                Vec::new()
            }
        };
        entries.retain(|entry| {
            configured.as_ref().is_some_and(|root| {
                contained_canonical_path(root, &entry.path)
                    .and_then(|path| file_fingerprint(&path).map(|fingerprint| (path, fingerprint)))
                    .is_some_and(|(path, (mtime_ms, size))| {
                        path == entry.path && mtime_ms == entry.mtime_ms && size == entry.size
                    })
            })
        });

        let mut state = Self {
            app_data_dir,
            root: configured,
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
        LibraryStatusDto {
            root: self
                .root
                .as_ref()
                .map(|root| root.to_string_lossy().into_owned()),
            indexed: if self.scanning {
                self.processed
            } else {
                self.entries.len().min(u32::MAX as usize) as u32
            },
            total: self.total,
            scanning: self.scanning,
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn set_root(&mut self, path: Option<PathBuf>) -> AppResult<bool> {
        let next = match path {
            Some(path) => Some(canonical_directory(&path)?),
            None => None,
        };
        if next == self.root {
            if !self.scanning {
                self.schedule_scan();
            }
            return Ok(self.scanning);
        }
        self.root = next;
        self.entries.clear();
        self.persist_root();
        self.persist_index();
        self.schedule_scan();
        Ok(self.scanning)
    }

    fn schedule_scan(&mut self) {
        self.scan_epoch = self.scan_epoch.wrapping_add(1);
        self.pending.clear();
        self.processed = 0;
        let files = self
            .root
            .as_ref()
            .map(|root| collect_pdfs(root))
            .unwrap_or_default();
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
            let unchanged = self.entries.iter().any(|entry| {
                entry.path == path && entry.mtime_ms == mtime_ms && entry.size == size
            });
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
            self.entries.retain(|existing| existing.path != entry.path);
            self.entries.push(entry);
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
        self.entries
            .iter()
            .find(|entry| entry.path == path)
            .cloned()
    }

    fn persist_root(&self) {
        let data = RootConfig {
            root: self.root.clone(),
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
        match serde_json::to_vec_pretty(&self.entries) {
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
        let root = state.root.as_ref()?;
        let entry = state.entries.iter().find(|entry| match id {
            CitationIdDto::Doi(value) => entry.doi.as_ref() == Some(value),
            CitationIdDto::ArXiv(value) => entry.arxiv.as_ref() == Some(value),
        })?;
        contained_canonical_path(root, &entry.path)
    }
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
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::to_vec(&RootConfig {
                root: Some(root.path().to_path_buf()),
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(app_data.path().join(INDEX_FILE), b"{broken json").unwrap();

        let state = LibraryState::load(app_data.path().to_path_buf());

        assert!(state.status().scanning);
        assert_eq!(state.status().total, 1);
        assert!(state.entries.is_empty());
    }
}

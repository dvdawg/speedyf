//! Local-only citation library indexing and resolution. Directory traversal
//! and persistence are thread-safe, while PDF parsing is deliberately left to
//! the engine worker that owns PDFium.

use crate::engine::citation::{citation_from_filename, find_explicit_citations, fold_arxiv_doi};
use crate::engine::types::{CitationIdDto, LibraryStatusDto};
use crate::errors::{AppError, AppResult};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

pub mod automerge;
pub mod backup;
pub mod bibtex;
pub mod bundle;
pub mod chain;
pub mod citekey;
pub mod filing;
pub mod firstpage;
pub mod identifiers;
pub mod lookup;
pub mod mainfile;
pub mod merge;
pub mod missing;
pub mod ocrtext;
pub mod paperness;
pub mod papers;
pub mod projects;
pub mod ratelimit;
pub mod references;
pub mod sources;
pub mod store;
pub mod suggest;
pub mod titlematch;
pub mod types;
pub mod watch;
pub mod xmp;

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
    /// existed still loads - it simply reports nothing indexed yet.
    #[serde(default)]
    pub text_indexed: bool,
    /// Size plus a hash of the first and last 64 KiB: how a paper record
    /// recognizes its file after a rename. Absent in an index written before
    /// paper records existed, which `next_candidate` treats as "rescan".
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// The Info dictionary's Title, when it is a title rather than a file
    /// name or "Untitled"; Author as written; the year of CreationDate.
    /// These rank above the first-page guess and below any lookup.
    #[serde(default)]
    pub info_title: Option<String>,
    #[serde(default)]
    pub info_author: Option<String>,
    #[serde(default)]
    pub info_year: Option<i32>,
    /// Every DOI and arXiv id the full text cites, less the file's own
    /// (projects.md sec. 8). `None` until extracted - the text pass fills it in,
    /// and a backfill job reads it out of the sidecar for entries indexed
    /// before it existed.
    #[serde(default)]
    pub cites: Option<Vec<CitationIdDto>>,
    /// Whether the file looks like a paper, and why (`paperness::classify`
    /// over the first page and the Info dictionary). `None` in an index
    /// written before the scan classified, until the rescan reaches it.
    #[serde(default)]
    pub paper_verdict: Option<types::PapernessVerdict>,
    #[serde(default)]
    pub paper_reason: Option<String>,
    /// What kind of item the classifier took it for - a thesis, a book, an
    /// article - when it could tell. Offered to the record's `kind` at
    /// first-page rank.
    #[serde(default)]
    pub paper_kind: Option<types::PaperKind>,
    /// What the file's XMP packet said, when it has one worth keeping.
    #[serde(default)]
    pub xmp: Option<xmp::XmpFields>,
    /// An ISBN printed on the first page, checksum-valid, digits only.
    #[serde(default)]
    pub isbn: Option<String>,
    /// The primary category off the arXiv stamp down the first page's
    /// margin (`cs.CL`, `hep-th`), which tells the lookup chain whether
    /// the paper is physics or computer science before any service has
    /// been asked.
    #[serde(default)]
    pub arxiv_category: Option<String>,
    /// Which arXiv version this file is (`v5`), which `arxiv` leaves off:
    /// read off the stamp when it names the file's own id, else off a file
    /// name that gave the id. Ranks the item's main file (`mainfile`).
    #[serde(default)]
    pub arxiv_version: Option<u32>,
    /// Where `doi` and `arxiv` were read: `FirstPage` for the head of the
    /// first page, `Filename` for the file's name, `Identifier` for a DOI
    /// made from a PII. The paper record keeps the source beside the
    /// identifier, which is how it tells the scan's own (re-derived every
    /// rescan) from a lookup's or the user's (kept). `None` in an index
    /// written before the scan recorded it, read as the scan's.
    #[serde(default)]
    pub doi_source: Option<types::MetaSource>,
    #[serde(default)]
    pub arxiv_source: Option<types::MetaSource>,
    /// The authors printed under the title on the first page, read from
    /// its layout (`firstpage`), and whether the title and the authors
    /// were read with confidence: a page that looked like a paper's title
    /// page and parsed cleanly. A reading without confidence is offered
    /// only when the Info dictionary and XMP say nothing.
    #[serde(default)]
    pub first_page_authors: Vec<types::Author>,
    #[serde(default)]
    pub first_page_title_confident: bool,
    #[serde(default)]
    pub first_page_authors_confident: bool,
    /// Whether the scan found the file worth OCR (`ocrtext::wants_ocr`):
    /// one of its first pages has an unusable text layer, or the first is
    /// a scanned image. The text pass OCRs such a file's pages.
    #[serde(default)]
    pub ocr_wanted: bool,
    /// The scan wanted OCR it was not allowed to run - a file scanned on
    /// demand (the inbox), outside the idle jobs - so its first-page
    /// reading is the text layer's. The idle text pass scans it again
    /// with OCR before extracting its text.
    #[serde(default)]
    pub ocr_deferred: bool,
}

/// The shape written to disk today. Both persisted files carry a version now:
/// they had none, and an unrecognized shape was silently discarded along with
/// the whole index.
/// v3: the scan reads the Info dictionary and fingerprints files. An older
/// index is loaded but every entry is rescanned once (see `read_index`), so
/// changes to what the scan extracts reach papers that were indexed before.
/// v4: the scan classifies each file as a paper or not, reads its XMP
/// packet and looks for an ISBN. Same one-time rescan.
/// v5: the classifier takes theses and books for library items, records the
/// kind, and reads a few more pages when the first says little. Same
/// one-time rescan, so the theses and books already indexed get their kind.
/// v6: the scan derives a DOI from an Elsevier PII and reads the arXiv
/// stamp's category (`identifiers`). Same one-time rescan, so the old
/// scans already indexed gain the DOI that makes them lookable-up.
/// v7: the scan reads the file's own identifiers from the head of the
/// first page only, takes an arXiv id for the file's own only off the
/// stamp, and records where each came from. Same one-time rescan, so a
/// paper that was filed under the first work it cites is put right.
/// v8: a document's reference list is cut into entries and kept in a refs
/// sidecar (`references`), and `cites` gains the DOIs APS journal
/// references stand for. No rescan: an older index's `cites` are cleared,
/// so the cheap backfill re-derives both from the text sidecars already on
/// disk, without PDFium.
/// v9: the reference list is read by the reference provider extension, not
/// the engine; `cites` are explicit identifiers and APS journal DOIs found
/// anywhere in the text. Cleared and backfilled again, the same way, so
/// identifiers the engine's own entry parser used to add are dropped and
/// the provider is handed every document (v1 refs sidecars are ignored).
/// v10: the scan reads the first page's layout for its title and authors
/// (`firstpage`). Rescanned once, since that needs PDFium; same bytes keep
/// their text, cites and refs sidecars, so only the metadata pass repeats.
/// v11: the word-spacing repair works. It read PDFium's `[x, y, w, h]`
/// character boxes as `[l, b, r, t]`, measured no advance and silently
/// did nothing, so a LaTeX paper's first-page text and full text ran its
/// words together ("Privateandpolynomialtime..."); it now reads loose boxes
/// in the right form and leaves rotated glyphs (the arXiv stamp) whole.
/// Rescanned once (the scan's identifiers, ISBN and classification come
/// from the repaired first pages) and every document's text extracted
/// again: text sidecars rewritten (v2), `cites` re-derived from the new
/// text as each finishes, and its refs sidecar dropped so the reference
/// provider is handed the new text (`do_library_text_next`).
/// v12: scanned papers are OCR'd (`ocrtext`, macOS Vision). Rescanned
/// once - metadata only - so a scan's first page is read by OCR for its
/// title, authors and identifiers. Text is extracted again only for the
/// files that rescan finds worth OCR (`ocr_wanted`, see
/// `keep_text_state`); every other sidecar stands.
/// v13: the scan keeps the file's arXiv version (`arxiv_version`, off the
/// stamp or the file name), which ranks an item's main file. Rescanned
/// once, metadata only, like v12: same bytes keep every sidecar.
const FORMAT_VERSION: u32 = 13;
/// Indexes older than this are rescanned on upgrade: what changed after
/// it is re-derived from the text sidecars instead.
const RESCAN_BELOW_VERSION: u32 = 13;
/// Indexes older than this have every document's text extracted again.
const TEXT_SINCE_VERSION: u32 = 11;
/// Indexes older than this have their `cites` re-derived by the backfill.
const CITES_SINCE_VERSION: u32 = 9;

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
    /// file, which is O(n²) across a scan - tolerable for one folder, not for
    /// several.
    entries: HashMap<PathBuf, LibraryEntry>,
    pending: VecDeque<PathBuf>,
    processed: u32,
    total: u32,
    scanning: bool,
    scan_epoch: u64,
    /// Backfilled entries since the index was last written; see
    /// `finish_cites`.
    cites_unsaved: u32,
    /// Files the watcher saw settle, to scan one per idle job (`watch`).
    watch_files: VecDeque<PathBuf>,
    watch_queued: HashSet<PathBuf>,
    /// Paths the watcher saw go: files, or folders with files under them.
    watch_gone: Vec<PathBuf>,
    /// Whether a `LibraryWatchNext` job is queued or running, so a second
    /// batch joins its queue rather than starting another chain.
    watch_job: bool,
    /// Entries the watch jobs changed since the index was last written.
    watch_unsaved: u32,
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
                    .and_then(|path| file_stamp(&path).map(|fingerprint| (path, fingerprint)))
                    .is_some_and(|(path, (mtime_ms, size))| {
                        path == entry.path
                            && mtime_ms == entry.mtime_ms
                            && size == entry.size
                            // Indexed before hidden folders (a Trash) and
                            // partial downloads were skipped: not the
                            // library's, so its paper file goes missing.
                            && !watch::is_ignored(&path, &configured, &[])
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
            cites_unsaved: 0,
            watch_files: VecDeque::new(),
            watch_queued: HashSet::new(),
            watch_gone: Vec::new(),
            watch_job: false,
            watch_unsaved: 0,
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
            let Some((mtime_ms, size)) = file_stamp(&path) else {
                self.processed = self.processed.saturating_add(1);
                continue;
            };
            // An entry without a fingerprint was written before paper
            // records existed, so the first run after that upgrade rescans
            // everything once, at idle priority, to fingerprint it and read
            // its Info dictionary. After that the stamp decides as before.
            let unchanged = self.entries.get(&path).is_some_and(|entry| {
                entry.mtime_ms == mtime_ms && entry.size == size && entry.fingerprint.is_some()
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

    /// Record a scanned candidate. Returns whether it was taken: a candidate
    /// from a scan that has since been restarted is dropped, and the caller
    /// should not act on it either.
    pub fn finish_candidate(
        &mut self,
        candidate: &ScanCandidate,
        entry: Option<LibraryEntry>,
    ) -> bool {
        if candidate.epoch != self.scan_epoch {
            return false;
        }
        if let Some(mut entry) = entry {
            // A re-scanned file is a changed file: whatever text was extracted
            // from the old bytes no longer describes it. The sidecar would be
            // rejected on load anyway once the stamp stops matching, but
            // deleting it now keeps the index from growing a copy per edit.
            // The one exception is the fingerprint-only rescan after the
            // upgrade: same bytes, so the sidecar still describes them.
            match self.entries.get(&entry.path) {
                Some(existing)
                    if existing.mtime_ms == entry.mtime_ms && existing.size == entry.size =>
                {
                    keep_text_state(&mut entry, existing);
                }
                _ => crate::search::librarytext::remove_sidecar(&self.app_data_dir, &entry.path),
            }
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
        true
    }

    /// Put one file into the index outside a scan: a file just copied into
    /// the inbox, which the caller wants a paper for now rather than when the
    /// next idle scan reaches it. Same bytes as before keep their sidecar.
    pub fn upsert_entry(&mut self, mut entry: LibraryEntry) {
        match self.entries.get(&entry.path) {
            Some(existing)
                if existing.mtime_ms == entry.mtime_ms && existing.size == entry.size =>
            {
                keep_text_state(&mut entry, existing);
            }
            _ => crate::search::librarytext::remove_sidecar(&self.app_data_dir, &entry.path),
        }
        self.entries.insert(entry.path.clone(), entry);
        self.persist_index();
    }

    /// Every path in the index, for marking paper files missing.
    pub fn paths(&self) -> std::collections::HashSet<PathBuf> {
        self.entries.keys().cloned().collect()
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
        self.all_entries()
            .into_iter()
            .find(|e| !e.text_indexed && !is_dataless(&e.path))
    }

    /// Record that a document's text is now on disk - or that extracting it
    /// failed, which is also final: a file that cannot be read will not start
    /// reading, and retrying it forever would stall every document behind it.
    /// `cites` is what the same text cited, so the backfill never has to
    /// read back a sidecar this pass just wrote.
    pub fn finish_text(&mut self, path: &Path, cites: Vec<CitationIdDto>) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.text_indexed = true;
            entry.cites = Some(cites);
        }
        self.persist_index();
    }

    /// A cloud placeholder (`is_dataless`) is not text work: reading it
    /// would download it. It is once it has been downloaded.
    pub fn has_text_work(&self) -> bool {
        self.entries
            .values()
            .any(|entry| !entry.text_indexed && !is_dataless(&entry.path))
    }

    /// Whether a full scan is under way; the watcher's safety net waits.
    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    /// Every indexed path's stamp, for the watcher's safety net.
    pub fn stamps(&self) -> HashMap<PathBuf, (u64, u64)> {
        self.entries
            .iter()
            .map(|(path, entry)| (path.clone(), (entry.mtime_ms, entry.size)))
            .collect()
    }

    /// A settled batch from the watcher. Returns whether the caller should
    /// queue a `LibraryWatchNext` job: there is work and none is queued.
    pub fn queue_watched(&mut self, files: Vec<PathBuf>, gone: Vec<PathBuf>) -> bool {
        for path in files {
            if self.watch_queued.insert(path.clone()) {
                self.watch_files.push_back(path);
            }
        }
        for path in gone {
            // Gone, so it cannot be canonicalized itself; its folder can.
            let path = match (
                path.parent().and_then(|p| p.canonicalize().ok()),
                path.file_name(),
            ) {
                (Some(parent), Some(name)) => parent.join(name),
                _ => path,
            };
            if !self.watch_gone.contains(&path) {
                self.watch_gone.push(path);
            }
        }
        let work = !self.watch_files.is_empty() || !self.watch_gone.is_empty();
        if work && !self.watch_job {
            self.watch_job = true;
            return true;
        }
        false
    }

    /// The paths the watcher saw go, with every index entry at or below one
    /// that is really gone dropped (and its text sidecar with it). Returns
    /// the paths, for the paper records to mark missing.
    pub fn take_gone(&mut self) -> Vec<PathBuf> {
        let gone = std::mem::take(&mut self.watch_gone);
        if gone.is_empty() {
            return gone;
        }
        let dropped: Vec<PathBuf> = self
            .entries
            .keys()
            .filter(|path| gone.iter().any(|g| path.starts_with(g)) && !path.is_file())
            .cloned()
            .collect();
        for path in &dropped {
            self.entries.remove(path);
            crate::search::librarytext::remove_sidecar(&self.app_data_dir, path);
        }
        if !dropped.is_empty() {
            self.watch_unsaved += 1;
        }
        gone
    }

    /// The next watched file still in a root, as a candidate: unchanged
    /// when the index already has these bytes.
    pub fn next_watched(&mut self) -> Option<ScanCandidate> {
        while let Some(path) = self.watch_files.pop_front() {
            self.watch_queued.remove(&path);
            let Some(path) = contained_in_any(&self.roots, &path) else {
                continue;
            };
            let Some((mtime_ms, size)) = file_stamp(&path) else {
                continue;
            };
            let unchanged = self.entries.get(&path).is_some_and(|entry| {
                entry.mtime_ms == mtime_ms && entry.size == size && entry.fingerprint.is_some()
            });
            return Some(ScanCandidate {
                path,
                mtime_ms,
                size,
                epoch: self.scan_epoch,
                unchanged,
            });
        }
        None
    }

    /// A watched file's scan: like `upsert_entry`, but the index is written
    /// when the batch ends (`finish_watch_step`), not per file.
    pub fn record_watched(&mut self, mut entry: LibraryEntry) {
        match self.entries.get(&entry.path) {
            Some(existing)
                if existing.mtime_ms == entry.mtime_ms && existing.size == entry.size =>
            {
                keep_text_state(&mut entry, existing);
            }
            _ => crate::search::librarytext::remove_sidecar(&self.app_data_dir, &entry.path),
        }
        self.entries.insert(entry.path.clone(), entry);
        self.watch_unsaved += 1;
    }

    /// After one watch job: true when another should follow. Otherwise the
    /// chain ends here and the index is written if the batch changed it.
    pub fn finish_watch_step(&mut self) -> bool {
        if !self.watch_files.is_empty() || !self.watch_gone.is_empty() {
            if self.watch_unsaved >= 25 {
                self.watch_unsaved = 0;
                self.persist_index();
            }
            return true;
        }
        self.watch_job = false;
        if self.watch_unsaved > 0 {
            self.watch_unsaved = 0;
            self.persist_index();
        }
        false
    }

    /// The next document whose text is on disk but whose cited identifiers
    /// were never read out of it: indexed before cites existed.
    pub fn next_cites_candidate(&self) -> Option<LibraryEntry> {
        self.all_entries()
            .into_iter()
            .find(|e| e.text_indexed && e.cites.is_none())
    }

    pub fn has_cites_work(&self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.text_indexed && entry.cites.is_none())
    }

    /// Record a backfilled entry's cites. Written through every 25 entries
    /// and when none are left, as the scan does: a backfill touches a
    /// thousand entries in seconds, and writing the whole index for each
    /// would cost more than the reading did.
    pub fn finish_cites(&mut self, path: &Path, cites: Vec<CitationIdDto>) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.cites = Some(cites);
        }
        self.cites_unsaved += 1;
        if self.cites_unsaved % 25 == 0 || !self.has_cites_work() {
            self.cites_unsaved = 0;
            self.persist_index();
        }
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

/// A rescan of the same bytes keeps the text pass's work - unless this
/// scan is the first to find the file worth OCR, whose text is then
/// extracted again with it (the v12 upgrade's one text pass, limited to
/// scanned papers).
fn keep_text_state(entry: &mut LibraryEntry, existing: &LibraryEntry) {
    entry.text_indexed = existing.text_indexed && (existing.ocr_wanted || !entry.ocr_wanted);
    entry.cites = existing.cites.clone();
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
            let mut entries = file.entries;
            if file.version < RESCAN_BELOW_VERSION {
                // The scan learns more than it did when this index was
                // written. Without a fingerprint `next_candidate` treats an
                // entry as changed, so the whole library is read once more
                // at idle priority; sidecars carry over.
                for entry in &mut entries {
                    entry.fingerprint = None;
                }
            }
            if file.version < TEXT_SINCE_VERSION {
                // The text pass extracts differently now: every document is
                // read again at idle priority, its sidecar rewritten and its
                // cites re-derived. The old cites stand until then.
                for entry in &mut entries {
                    entry.text_indexed = false;
                }
            }
            if file.version < CITES_SINCE_VERSION {
                // What is read out of the text grew (the reference list,
                // the journal DOIs): the backfill reads it again out of
                // the sidecars, which are still good.
                for entry in &mut entries {
                    entry.cites = None;
                }
            }
            return entries;
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

/// A gap wider than this share of the page's median glyph advance is a word
/// space.
///
/// Measured against the glyphs themselves rather than the font size PDFium
/// reports, because that size is not always reported and a zero there silently
/// disables the whole repair. An interword space runs about half a lowercase
/// advance; letters within a word sit flush or kern slightly negative, so there
/// is a wide margin between the two.
const WORD_GAP_RATIO: f32 = 0.3;
/// Baseline separation, as a share of glyph height, above which a character
/// belongs to the next line.
const LINE_BREAK_RATIO: f32 = 0.6;

/// What PDFium reports in place of a hyphen it removed at a line end,
/// having joined the word across the break.
const SOFT_HYPHEN_MARK: char = '\u{2}';

/// What the geometry puts before a character: a word space the file
/// encoded as position, or the start of a new line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Separator {
    Space,
    Line,
}

/// `[x, y_bottom, w, h]` (y-up, as `text::extract_page` and
/// `layout_text::extract_spacing_glyphs` report a character) as
/// `[left, bottom, right, top]`, the form the repair measures in.
///
/// Reading the first form as the second was the bug that silently turned
/// the repair off: `w - x` is negative for nearly every glyph, so no
/// advance was ever measured and every caller got PDFium's own lines back,
/// words run together.
pub fn edges_from_rect([x, y, w, h]: [f32; 4]) -> [f32; 4] {
    [x, y, x + w, y + h]
}

/// The separator before each character, or `None` when the page has too
/// little geometry to measure - the caller then keeps the file's own
/// lines. `boxes` are `[l, b, r, t]`; `upright` says which characters run
/// left to right (missing entries do).
///
/// Characters that do not run left to right - the arXiv stamp up the
/// margin, a rotated axis label - take no part in the geometry. Each run
/// of them is its own line, its text as the file spells it: measured, a
/// stamp's characters stack vertically and every one would read as a new
/// line, one fragment per character. Nor do they count toward the median
/// advance or stand as the "previous" glyph a gap is measured from.
fn separators(
    chars: &[char],
    boxes: &[[f32; 4]],
    upright: &[bool],
) -> Option<Vec<Option<Separator>>> {
    let turned = |index: usize| upright.get(index) == Some(&false);
    // Geometry may be absent or shorter than the text. Characters past it are
    // still emitted - losing text to recover spacing would be a bad trade.
    let measured = chars.len().min(boxes.len());
    let level: Vec<[f32; 4]> = (0..measured)
        .filter(|&index| !turned(index))
        .map(|index| boxes[index])
        .collect();
    let advance = median_advance(&level)?;
    let gap_threshold = WORD_GAP_RATIO * advance;

    let mut out = vec![None; chars.len()];
    let mut previous: Option<[f32; 4]> = None;
    let mut in_turned = false;
    // Whether the line so far ends in a character that is not whitespace:
    // only then can a gap add a space.
    let mut open_word = false;
    // Whether the word so far ends in PDFium's mark for a line-end hyphen it
    // removed (U+0002): the word goes on at the start of the next line, and
    // PDFium has already joined it ("mi\u{2}crostate"). Breaking there would
    // split it, in the search text too.
    let mut hyphenated = false;

    for (index, &ch) in chars.iter().enumerate() {
        let bx = boxes.get(index).copied().unwrap_or([0.0; 4]);
        let has_box = bx[2] > bx[0] && bx[3] > bx[1];
        let printing = !ch.is_whitespace() && !ch.is_control();

        if printing && turned(index) != in_turned {
            in_turned = !in_turned;
            out[index] = Some(Separator::Line);
            previous = None;
            open_word = false;
        }
        if !in_turned && !turned(index) {
            if let (Some(prev), true) = (previous, has_box) {
                let height = (prev[3] - prev[1]).max(bx[3] - bx[1]).max(1.0);
                if (prev[1] - bx[1]).abs() > LINE_BREAK_RATIO * height {
                    out[index] = Some(Separator::Line);
                    open_word = false;
                } else if !ch.is_whitespace() && open_word && bx[0] - prev[2] > gap_threshold {
                    out[index] = Some(Separator::Space);
                }
            }
            if has_box {
                previous = Some(bx);
            }
        }

        if hyphenated && printing {
            out[index] = None;
        }
        if ch == SOFT_HYPHEN_MARK {
            hyphenated = true;
        } else if !ch.is_control() {
            hyphenated = false;
        }
        if ch.is_whitespace() {
            open_word = false;
        } else if !ch.is_control() {
            open_word = true;
        }
    }
    Some(out)
}

/// Rebuild page lines from the character stream and its geometry.
///
/// Works from one entry per PDFium character index in `raw`, `boxes` and
/// `upright`, which therefore line up by construction. An earlier version
/// indexed `boxes` by `run.start + offset` instead, which silently read the
/// wrong glyph's geometry for every character after the run builder skipped
/// one - and produced no spaces at all on the files that needed them.
///
/// Many PDFs, LaTeX output above all, encode word gaps as position rather than
/// as space characters; those files yield titles like
/// "ATheoryofSaddleEscapeinDeepNonlinear" unless the gaps are read back here.
/// `boxes` are `[l, b, r, t]` (`edges_from_rect`).
pub fn lines_from_glyphs(raw: &str, boxes: &[[f32; 4]], upright: &[bool]) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let Some(separators) = separators(&chars, boxes, upright) else {
        return raw
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
    };

    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for (&ch, separator) in chars.iter().zip(separators) {
        match separator {
            Some(Separator::Line) => push_line(&mut lines, &mut line),
            Some(Separator::Space) => line.push(' '),
            None => {}
        }
        if ch == '\n' || ch == '\r' {
            // A break the file stated outright.
            push_line(&mut lines, &mut line);
        } else if !ch.is_control()
            && !(ch.is_whitespace() && (line.is_empty() || line.ends_with(char::is_whitespace)))
        {
            line.push(ch);
        }
    }
    push_line(&mut lines, &mut line);
    lines
}

/// `lines_from_glyphs` for a page whose characters all run left to right.
#[cfg(test)]
pub fn lines_from_chars(raw: &str, boxes: &[[f32; 4]]) -> Vec<String> {
    lines_from_glyphs(raw, boxes, &[])
}

fn push_line(lines: &mut Vec<String>, line: &mut String) {
    let text = line.trim();
    if !text.is_empty() {
        lines.push(text.to_string());
    }
    line.clear();
}

/// Median width of the glyphs that reported one. Median rather than mean so a
/// single wide glyph or a stray zero box cannot move it.
fn median_advance(boxes: &[[f32; 4]]) -> Option<f32> {
    let mut widths: Vec<f32> = boxes
        .iter()
        .map(|b| b[2] - b[0])
        .filter(|w| *w > 0.0)
        .collect();
    if widths.len() < 2 {
        return None;
    }
    widths.sort_by(f32::total_cmp);
    Some(widths[widths.len() / 2])
}

/// Page text with word spacing recovered, lines separated by newlines.
/// `rects` are `[x, y_bottom, w, h]` as PDFium extraction reports them
/// (`layout_text::SpacingGlyphs`).
pub fn text_from_page(raw: &str, rects: &[[f32; 4]], upright: &[bool]) -> String {
    let boxes: Vec<[f32; 4]> = rects.iter().copied().map(edges_from_rect).collect();
    lines_from_glyphs(raw, &boxes, upright).join("\n")
}

/// The character indices before which the geometry puts a word space or a
/// line break, ascending - for the library's search sidecar, which keeps
/// the raw stream (its offsets are how a hit is highlighted) and inserts
/// these as whitespace that maps back to the index it precedes
/// (`librarytext::write_spaced_sidecar`). Empty when the page has no usable
/// geometry. `rects` as for `text_from_page`.
pub fn word_breaks(raw: &str, rects: &[[f32; 4]], upright: &[bool]) -> Vec<u32> {
    let chars: Vec<char> = raw.chars().collect();
    let boxes: Vec<[f32; 4]> = rects.iter().copied().map(edges_from_rect).collect();
    separators(&chars, &boxes, upright)
        .map(|separators| {
            separators
                .iter()
                .enumerate()
                .filter(|(_, separator)| separator.is_some())
                .map(|(index, _)| index.min(u32::MAX as usize) as u32)
                .collect()
        })
        .unwrap_or_default()
}

pub fn entry_from_filename(candidate: &ScanCandidate) -> Option<LibraryEntry> {
    let name = candidate.path.file_name()?.to_string_lossy();
    let id = citation_from_filename(&name)?;
    let (doi, arxiv) = match id {
        CitationIdDto::Doi(value) => (Some(value), None),
        CitationIdDto::ArXiv(value) => (None, Some(value)),
    };
    let source = |value: &Option<String>| value.as_ref().map(|_| types::MetaSource::Filename);
    Some(LibraryEntry {
        path: candidate.path.clone(),
        mtime_ms: candidate.mtime_ms,
        size: candidate.size,
        doi_source: source(&doi),
        arxiv_source: source(&arxiv),
        doi,
        arxiv,
        title: None,
        text_indexed: false,
        fingerprint: None,
        info_title: None,
        info_author: None,
        info_year: None,
        cites: None,
        paper_verdict: None,
        paper_reason: None,
        paper_kind: None,
        xmp: None,
        isbn: None,
        arxiv_category: None,
        arxiv_version: None,
        first_page_authors: Vec::new(),
        first_page_title_confident: false,
        first_page_authors_confident: false,
        ocr_wanted: false,
        ocr_deferred: false,
    })
}

/// The headings a reference list stands under: the whole of a line, less
/// a section number before and a colon or period after, in any case.
const REFERENCE_HEADINGS: &[&str] = &[
    "references",
    "notes and references",
    "bibliography",
    "literature cited",
    "works cited",
];

/// The part of a page's text above its reference list: everything before
/// the first line that is just "References", "Bibliography", "Literature
/// cited" or "Works cited", numbered or not. The document's own
/// identifiers are read from here and nowhere else, because a reference
/// list is a list of other people's: a short paper whose references start
/// on the first page was being filed under the first work it cites. A
/// page without the heading is all head, which is most first pages.
pub fn page_head(text: &str) -> &str {
    let mut consumed = 0;
    for line in text.split_inclusive('\n') {
        if is_reference_heading(line) {
            return &text[..consumed];
        }
        consumed += line.len();
    }
    text
}

/// Whether a line is a reference-list heading and nothing else: the
/// words, with a section number ("6", "6.", "VI.", "A.2") allowed before
/// them and a colon or period after. "References are in the appendix" is
/// prose; "References 12" is a table of contents.
fn is_reference_heading(line: &str) -> bool {
    let mut words = line.split_whitespace().peekable();
    if words.peek().is_some_and(|first| is_section_number(first)) {
        words.next();
    }
    let heading = words
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches([':', '.'])
        .to_lowercase();
    REFERENCE_HEADINGS.contains(&heading.as_str())
}

/// "6", "6.", "6.1", "VI.", "(6)" - what a numbered heading starts with.
fn is_section_number(word: &str) -> bool {
    let core = word.trim_matches(|c: char| matches!(c, '.' | ')' | '('));
    !core.is_empty()
        && core.split('.').all(|part| {
            !part.is_empty()
                && (part.chars().all(|c| c.is_ascii_digit())
                    || part
                        .chars()
                        .all(|c| matches!(c, 'i' | 'v' | 'x' | 'I' | 'V' | 'X')))
        })
}

/// What the pages the classifier read say: the first page, or the first
/// few when the first says little, so a book's ISBN is found on its
/// copyright page.
///
/// The file's own DOI and ISBN are read from the head of the page, above
/// any reference list (`page_head`). Its own arXiv id is only ever the
/// stamp arXiv prints down the margin (`identifiers::arxiv_stamp`): a bare
/// `arXiv:ID` in running text is a citation, wherever it stands. The stamp
/// is looked for on the whole page, since arXiv lays it over the page
/// after the body and PDFium reads it out last - below a reference list,
/// when the page has one - and its bracketed category makes it
/// unmistakable anyway.
pub fn entry_from_first_page(candidate: &ScanCandidate, text: &str) -> LibraryEntry {
    entry_from_page(candidate, text, None)
}

/// `entry_from_first_page`, with the title and authors the first page's
/// layout gave (`firstpage::extract`), when the page could be laid out.
/// Its title replaces the "first long line" guess, which stays as the
/// fallback, unconfident, for a page without one.
pub fn entry_from_page(
    candidate: &ScanCandidate,
    text: &str,
    layout: Option<&firstpage::FirstPage>,
) -> LibraryEntry {
    let head = page_head(text);
    let doi = find_explicit_citations(head)
        .into_iter()
        .find_map(|occurrence| match occurrence.id {
            CitationIdDto::Doi(value) => Some(value),
            CitationIdDto::ArXiv(_) => None,
        });
    let arxiv = identifiers::arxiv_stamp(text).map(|stamp| stamp.id);
    let laid_out = layout.and_then(|page| page.title.clone());
    let title_confident = laid_out.is_some()
        && layout.is_some_and(|page| page.title_confidence == firstpage::Confidence::High);
    let title = laid_out.or_else(|| {
        text.lines()
            .map(str::trim)
            .find(|line| line.chars().count() > 12)
            .map(|line| line.chars().take(300).collect())
    });
    let first_page_authors = layout.map(|page| page.authors.clone()).unwrap_or_default();
    let first_page_authors_confident = !first_page_authors.is_empty()
        && layout.is_some_and(|page| page.authors_confidence == firstpage::Confidence::High);
    let source = |value: &Option<String>| value.as_ref().map(|_| types::MetaSource::FirstPage);
    LibraryEntry {
        path: candidate.path.clone(),
        mtime_ms: candidate.mtime_ms,
        size: candidate.size,
        doi_source: source(&doi),
        arxiv_source: source(&arxiv),
        doi,
        arxiv,
        title,
        text_indexed: false,
        fingerprint: None,
        info_title: None,
        info_author: None,
        info_year: None,
        cites: None,
        paper_verdict: None,
        paper_reason: None,
        paper_kind: None,
        xmp: None,
        isbn: find_isbn(head),
        arxiv_category: None,
        arxiv_version: None,
        first_page_authors,
        first_page_title_confident: title_confident,
        first_page_authors_confident,
        ocr_wanted: false,
        ocr_deferred: false,
    }
}

/// What the file's identifiers say on their own (projects.md sec. 4), once the
/// file name and the pages have had their say. An Elsevier PII - printed
/// in the head of the page, or left in an Info Title the scan rejected as
/// a title ("PII: 0022-0000(88)90028-1") - becomes the article's DOI when
/// there is no other, but only when its ISSN checks out: a PII is as good
/// an identity as a printed DOI, and a mis-OCR'd one would merge two
/// articles. The arXiv stamp's category is kept for the lookup chain when
/// the stamp names this file's own id (the file name may say otherwise).
pub fn derive_from_identifiers(entry: &mut LibraryEntry, text: &str, info_title: Option<&str>) {
    if entry.doi.is_none() {
        entry.doi = [Some(page_head(text)), info_title]
            .into_iter()
            .flatten()
            .filter_map(identifiers::find_pii)
            .find(|pii| identifiers::pii_has_valid_issn(pii))
            .map(|pii| identifiers::doi_from_pii(&pii));
        entry.doi_source = entry.doi.as_ref().map(|_| types::MetaSource::Identifier);
    }
    let stamp = identifiers::arxiv_stamp(text)
        .filter(|stamp| entry.arxiv.as_deref().is_none_or(|id| id == stamp.id));
    let named_version = || {
        (entry.arxiv_source == Some(types::MetaSource::Filename))
            .then(|| entry.path.file_name())
            .flatten()
            .and_then(|name| mainfile::version_from_name(&name.to_string_lossy()))
    };
    entry.arxiv_version = stamp
        .as_ref()
        .and_then(|stamp| stamp.version)
        .or_else(named_version);
    entry.arxiv_category = stamp.and_then(|stamp| stamp.category);
}

/// The separators an ISBN is printed with. Hyphens of every kind and
/// spaces, including the non-breaking one a typesetter uses to keep the
/// number on one line.
fn is_isbn_separator(c: char) -> bool {
    matches!(
        c,
        '-' | ' ' | '\u{a0}' | '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}'
    )
}

/// Is `digits` (10 or 13 characters, digits with a possible trailing X) a
/// real ISBN? ISBN-10 is a weighted sum mod 11 with X for ten; ISBN-13 is
/// the EAN mod-10 check.
fn isbn_checksum_ok(digits: &str) -> bool {
    let values: Vec<u32> = digits
        .chars()
        .map(|c| match c {
            'X' => 10,
            _ => c.to_digit(10).unwrap_or(99),
        })
        .collect();
    if values.contains(&99) {
        return false;
    }
    match values.len() {
        10 => {
            if values[..9].contains(&10) {
                return false;
            }
            let sum: u32 = values
                .iter()
                .enumerate()
                .map(|(index, value)| (10 - index as u32) * value)
                .sum();
            sum % 11 == 0
        }
        13 => {
            if values.contains(&10) {
                return false;
            }
            let sum: u32 = values
                .iter()
                .enumerate()
                .map(|(index, value)| if index % 2 == 0 { *value } else { 3 * value })
                .sum();
            sum % 10 == 0
        }
        _ => false,
    }
}

/// An ISBN as written - `978-0-306-40615-7`, `0 306 40615 2`,
/// `0306406152`, with a trailing `x` - normalized to its digits (and a
/// trailing `X`), or `None` when it is not 10 or 13 long or its checksum
/// fails.
pub fn normalize_isbn(raw: &str) -> Option<String> {
    let digits: String = raw
        .trim()
        .chars()
        .filter(|c| !is_isbn_separator(*c))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    isbn_checksum_ok(&digits).then_some(digits)
}

/// The first valid ISBN announced as one in `text`: "ISBN" (with or
/// without "-10" / "-13" and a colon), then the number, hyphenated or
/// spaced or neither. A copyright page lists several - hardcover, eBook -
/// and the first that checks out is the one.
pub fn find_isbn(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("isbn") {
        let after = from + at + "isbn".len();
        from = after;
        // "isbn" inside a word ("misbnorm") is not the label.
        if text[..after - "isbn".len()]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        {
            continue;
        }
        if let Some(found) = isbn_after_label(&text[after..]) {
            return Some(found);
        }
    }
    None
}

/// The number following an "ISBN" label, tried with the "-13"/"-10" form
/// marker stripped first and as written second, so "ISBN-13: 978..." and
/// "ISBN 1056..." both read right.
fn isbn_after_label(rest: &str) -> Option<String> {
    fn skip_leading(s: &str) -> &str {
        s.trim_start_matches(|c: char| is_isbn_separator(c) || c == ':')
    }
    let unmarked = skip_leading(rest);
    let stripped = ["13", "10"].iter().find_map(|marker| {
        let after = unmarked.strip_prefix(marker)?;
        let next = after.chars().next();
        (next.is_none() || next.is_some_and(|c| is_isbn_separator(c) || c == ':'))
            .then(|| skip_leading(after))
    });
    stripped
        .and_then(isbn_at_start)
        .or_else(|| isbn_at_start(unmarked))
}

/// An ISBN at the very start of `s`: digits run together or separated by
/// one kind of separator, at most 13 of them, a final X allowed in the
/// tenth place. The 13-digit reading is tried first, then the first ten.
fn isbn_at_start(s: &str) -> Option<String> {
    let mut digits = String::new();
    let mut separator: Option<char> = None;
    let mut last_was_separator = false;
    for c in s.chars() {
        if c.is_ascii_digit() || ((c == 'X' || c == 'x') && digits.len() == 9) {
            digits.push(c.to_ascii_uppercase());
            last_was_separator = false;
            if digits.len() == 13 || digits.ends_with('X') {
                break;
            }
        } else if is_isbn_separator(c) && !digits.is_empty() && !last_was_separator {
            // One separator per number: "0-306-40615-2 2004" ends at the
            // space, "0 306 40615 2" does not.
            let kind = if c == ' ' || c == '\u{a0}' { ' ' } else { '-' };
            match separator {
                Some(known) if known != kind => break,
                _ => separator = Some(kind),
            }
            last_was_separator = true;
        } else {
            break;
        }
    }
    if digits.len() >= 13 && isbn_checksum_ok(&digits[..13]) {
        return Some(digits[..13].to_owned());
    }
    if digits.len() >= 10 && isbn_checksum_ok(&digits[..10]) {
        return Some(digits[..10].to_owned());
    }
    None
}

/// The identifiers a document's full text cites, in first-seen order,
/// without repeats and without the document's own: an arXiv paper carries
/// its id in every page header, and a journal article prints its DOI on the
/// first page. arXiv DOIs (`10.48550/arXiv....`) are folded to the arXiv id,
/// as citation resolution does, so a reference list that cites the same
/// preprint both ways yields one edge.
pub fn cited_identifiers(
    text: &str,
    own_doi: Option<&str>,
    own_arxiv: Option<&str>,
) -> Vec<CitationIdDto> {
    let own_doi = own_doi.map(str::to_ascii_lowercase);
    let mut seen: HashSet<CitationIdDto> = HashSet::new();
    let mut out = Vec::new();
    for occurrence in find_explicit_citations(text) {
        let id = fold_arxiv_doi(occurrence.id);
        let own = match &id {
            CitationIdDto::Doi(value) => own_doi.as_deref() == Some(value.as_str()),
            CitationIdDto::ArXiv(value) => own_arxiv == Some(value.as_str()),
        };
        if own || !seen.insert(id.clone()) {
            continue;
        }
        out.push(id);
    }
    out
}

/// What a document's text cites: every explicit DOI and arXiv id in it
/// (`cited_identifiers`) plus the DOIs its APS journal references stand
/// for, wherever they are printed (`references::journal_dois`), the
/// document's own left out - for `LibraryEntry::cites`. Identifiers only:
/// cutting the reference list into entries is the reference provider
/// extension's job (`references`), and with no provider these are all the
/// edges there are. `text` is the sidecar's form
/// (`librarytext::normalized_spaced_text`, pages joined by newlines), so the text
/// pass and the backfill read the same identifiers out of the same
/// document.
pub fn record_citations(entry: &LibraryEntry, text: &str) -> Vec<CitationIdDto> {
    let own_doi = entry.doi.as_deref();
    let own_arxiv = entry.arxiv.as_deref();
    let mut cites = cited_identifiers(text, own_doi, own_arxiv);
    let own_doi = own_doi.map(str::to_ascii_lowercase);
    for doi in references::journal_dois(text) {
        if own_doi.as_deref() == Some(doi.as_str()) {
            continue;
        }
        let id = CitationIdDto::Doi(doi);
        if !cites.contains(&id) {
            cites.push(id);
        }
    }
    cites
}

/// The cited identifiers of a document indexed before they were kept (or
/// before the current way of reading them), read back out of its sidecar.
/// A sidecar that is missing or describes other bytes yields nothing, and
/// that is final for the same reason a text pass that fails is: the next
/// scan of changed bytes starts it over.
pub fn backfill_cites(app_data_dir: &Path, entry: &LibraryEntry) -> Vec<CitationIdDto> {
    crate::search::librarytext::read_sidecar_text(
        app_data_dir,
        &entry.path,
        entry.mtime_ms,
        entry.size,
    )
    .map(|text| record_citations(entry, &text))
    .unwrap_or_default()
}

/// An Info-dictionary Title worth keeping. Authoring tools fill the field
/// with the source file's name or "Untitled" often enough that the rule is
/// needed: those would otherwise outrank the first-page title.
pub fn plausible_info_title(title: &str) -> bool {
    let title = title.trim();
    if title.is_empty() || title.eq_ignore_ascii_case("untitled") {
        return false;
    }
    let lower = title.to_lowercase();
    if [".pdf", ".tex", ".dvi", ".doc", ".docx", ".ps"]
        .iter()
        .any(|extension| lower.ends_with(extension))
    {
        return false;
    }
    // Publisher scans fill Title with the article's identifier: "PII:
    // S0022-0000(88)90028-1", "doi:10.1103/PhysRevLett.78.2690". The first
    // page carries the real title; this would only replace it with a number.
    if [
        "pii:", "pii ", "doi:", "doi ", "arxiv:", "issn", "isbn", "10.",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        return false;
    }
    // A title has letters in it. Identifiers and page labels do not, or have
    // only one or two among the digits.
    if title.chars().filter(|c| c.is_alphabetic()).count() < 3 {
        return false;
    }
    // "jarzynski_1997" or "paper-final": a file stem, not a title.
    title.contains(char::is_whitespace) || !title.contains(['_', '-'])
}

/// The year of a PDF date string, `D:YYYYMMDDHHmmSS...`. Only the year is
/// wanted, and only when it is one a paper could carry.
pub fn year_from_pdf_date(date: &str) -> Option<i32> {
    let digits = date.trim().trim_start_matches("D:");
    let year: String = digits.chars().take(4).collect();
    if year.len() < 4 || !year.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    year.parse::<i32>()
        .ok()
        .filter(|year| (1800..=2100).contains(year))
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

/// (mtime in ms, size): the cheap "has this file changed" check the index
/// runs on every scan.
pub fn file_stamp(path: &Path) -> Option<(u64, u64)> {
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

/// Whether `path` is a cloud placeholder whose bytes are not on this Mac:
/// iCloud Drive, Dropbox or OneDrive with "optimize storage" evicted it.
/// Opening one to fingerprint or index it downloads it, so the scan reads
/// only its stamp and name, and the text pass leaves it until it is
/// downloaded. It is still there: never missing.
#[cfg(target_os = "macos")]
pub fn is_dataless(path: &Path) -> bool {
    use std::os::macos::fs::MetadataExt;
    fs::metadata(path).is_ok_and(|metadata| dataless_flag(metadata.st_flags()))
}

/// Other platforms have no such placeholder in `stat`.
#[cfg(not(target_os = "macos"))]
pub fn is_dataless(_path: &Path) -> bool {
    false
}

/// `SF_DATALESS` from `<sys/stat.h>`.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

#[cfg(target_os = "macos")]
fn dataless_flag(flags: u32) -> bool {
    flags & SF_DATALESS != 0
}

/// Bytes read from each end of a file for its content fingerprint.
const FINGERPRINT_EDGE: u64 = 64 * 1024;

/// `{size:x}-{hash:016x}`: the size and an FNV-1a 64 hash of the first and
/// last 64 KiB (the whole file when it is no longer than both). Stable across
/// renames and moves, different the moment the file is re-saved - cheap
/// enough to compute during the scan, which is why it is the identity a
/// paper record falls back on when there is no identifier (projects.md sec. 2).
pub fn content_fingerprint(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    // Every empty file hashes the same, and a fingerprint match merges files
    // into one paper for good. A failed download has no identity yet.
    if size == 0 {
        return None;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut buffer = vec![0_u8; FINGERPRINT_EDGE as usize];
    if size <= 2 * FINGERPRINT_EDGE {
        let mut whole = Vec::with_capacity(size as usize);
        file.read_to_end(&mut whole).ok()?;
        hash = fnv1a(hash, &whole);
    } else {
        file.read_exact(&mut buffer).ok()?;
        hash = fnv1a(hash, &buffer);
        file.seek(SeekFrom::End(-(FINGERPRINT_EDGE as i64))).ok()?;
        file.read_exact(&mut buffer).ok()?;
        hash = fnv1a(hash, &buffer);
    }
    Some(format!("{size:x}-{hash:016x}"))
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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
            // Hidden files and folders (a Trash among them, when a root
            // holds the home folder) and downloads still arriving are not
            // the library's; the watcher ignores them the same way.
            if watch::ignored_below(root, &path) {
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

    pub(crate) fn indexed_entry(path: &Path, mtime_ms: u64, size: u64) -> LibraryEntry {
        LibraryEntry {
            path: path.to_path_buf(),
            mtime_ms,
            size,
            doi: None,
            arxiv: None,
            title: None,
            text_indexed: true,
            fingerprint: Some("fp".into()),
            info_title: None,
            info_author: None,
            info_year: None,
            cites: None,
            paper_verdict: None,
            paper_reason: None,
            paper_kind: None,
            xmp: None,
            isbn: None,
            arxiv_category: None,
            arxiv_version: None,
            first_page_authors: Vec::new(),
            first_page_title_confident: false,
            first_page_authors_confident: false,
            ocr_wanted: false,
            ocr_deferred: false,
            doi_source: None,
            arxiv_source: None,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_cloud_placeholder_is_told_by_its_dataless_flag_alone() {
        assert!(dataless_flag(0x4000_0000));
        // UF_HIDDEN, UF_COMPRESSED and SF_ARCHIVED say nothing about it.
        assert!(!dataless_flag(0x8000 | 0x20 | 0x1_0000));
        assert!(dataless_flag(0x4000_0000 | 0x8000));
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("here.pdf");
        fs::write(&file, b"%PDF-1.4").unwrap();
        assert!(!is_dataless(&file), "a file on disk is read as usual");
        assert!(!is_dataless(&dir.path().join("gone.pdf")));
    }

    #[test]
    fn the_full_scan_skips_hidden_folders_the_trash_and_partial_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for folder in [".Trash", ".cache", "x.pdf.download", "sub"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        for file in [
            "kept.pdf",
            "sub/also.PDF",
            ".hidden.pdf",
            ".Trash/trashed.pdf",
            ".cache/cached.pdf",
            "x.pdf.download/x.pdf",
            "y.pdf.crdownload",
        ] {
            fs::write(root.join(file), b"%PDF-1.4").unwrap();
        }
        assert_eq!(
            collect_pdfs(&root),
            vec![root.join("kept.pdf"), root.join("sub/also.PDF")]
        );
    }

    #[test]
    fn isbns_are_found_checked_and_normalized() {
        // ISBN-10, hyphenated, spaced, bare, and with the X check digit.
        assert_eq!(
            find_isbn("ISBN 0-306-40615-2").as_deref(),
            Some("0306406152")
        );
        assert_eq!(
            find_isbn("ISBN: 0 306 40615 2").as_deref(),
            Some("0306406152")
        );
        assert_eq!(find_isbn("isbn 0306406152").as_deref(), Some("0306406152"));
        assert_eq!(
            find_isbn("ISBN 0-8044-2957-X").as_deref(),
            Some("080442957X")
        );
        assert_eq!(
            find_isbn("ISBN 0-8044-2957-x (pbk.)").as_deref(),
            Some("080442957X")
        );
        // ISBN-13, with and without the form marker.
        assert_eq!(
            find_isbn("ISBN 978-0-306-40615-7").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(
            find_isbn("ISBN-13: 978-0-306-40615-7").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(
            find_isbn("ISBN 13: 978 0 306 40615 7").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(
            find_isbn("ISBN-10: 0-306-40615-2").as_deref(),
            Some("0306406152")
        );
        assert_eq!(
            find_isbn("ISBN\u{a0}978\u{2011}0\u{2011}13\u{2011}468599\u{2011}1").as_deref(),
            Some("9780134685991")
        );
        // Bad checksums are refused; the next valid one on the page wins.
        assert!(find_isbn("ISBN 0-306-40615-3").is_none());
        assert!(find_isbn("ISBN 978-0-306-40615-8").is_none());
        assert_eq!(
            find_isbn("ISBN 978-0-306-40615-8 (hardcover)\nISBN 978-0-13-468599-1 (eBook)")
                .as_deref(),
            Some("9780134685991")
        );
        // Text after the number does not run into it.
        assert_eq!(
            find_isbn("ISBN 0-306-40615-2 2004 Springer").as_deref(),
            Some("0306406152")
        );
        assert_eq!(
            find_isbn("ISBN 0306406152. Printed in 2004").as_deref(),
            Some("0306406152")
        );
        // Not announced, not a number, inside a word: nothing.
        assert!(find_isbn("0-306-40615-2").is_none());
        assert!(find_isbn("ISBN pending").is_none());
        assert!(find_isbn("ISBN").is_none());
        assert!(find_isbn("the misbn0306406152 code").is_none());
        assert!(find_isbn("").is_none());

        assert_eq!(
            normalize_isbn(" 978-0-306-40615-7 ").as_deref(),
            Some("9780306406157")
        );
        assert_eq!(normalize_isbn("080442957x").as_deref(), Some("080442957X"));
        assert!(normalize_isbn("978-0-306-40615-7-1").is_none(), "14 digits");
        assert!(normalize_isbn("X306406152").is_none(), "X only at the end");
        assert!(normalize_isbn("978030640615X").is_none(), "no X in ISBN-13");
        assert!(normalize_isbn("").is_none());
    }

    #[test]
    fn an_index_without_the_classification_fields_still_loads() {
        let entries = read_index(
            serde_json::json!({
                "version": FORMAT_VERSION,
                "entries": [{
                    "path": "/lib/a.pdf", "mtime_ms": 1, "size": 2,
                    "fingerprint": "2-abc", "text_indexed": true,
                    "info_title": "A paper"
                }]
            })
            .to_string()
            .as_bytes(),
        );
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!(entry.paper_verdict.is_none());
        assert!(entry.paper_reason.is_none());
        assert!(entry.xmp.is_none());
        assert!(entry.isbn.is_none());
        assert!(entry.arxiv_category.is_none());
        assert!(entry.doi_source.is_none());
        assert!(entry.arxiv_source.is_none());
        assert_eq!(entry.info_title.as_deref(), Some("A paper"));
        assert!(entry.first_page_authors.is_empty());
        assert!(!entry.first_page_title_confident);
        assert!(!entry.first_page_authors_confident);

        // The fields round-trip once written.
        let mut written = entry.clone();
        written.doi_source = Some(types::MetaSource::Identifier);
        written.arxiv_source = Some(types::MetaSource::Filename);
        written.paper_verdict = Some(types::PapernessVerdict::Likely);
        written.paper_reason = Some("has an abstract".into());
        written.paper_kind = Some(types::PaperKind::Thesis);
        written.xmp = Some(xmp::XmpFields {
            doi: Some("10.1/x".into()),
            ..xmp::XmpFields::default()
        });
        written.isbn = Some("9780306406157".into());
        written.arxiv_category = Some("cs.CL".into());
        let text = serde_json::to_string(&IndexFile {
            version: FORMAT_VERSION,
            entries: vec![written.clone()],
        })
        .unwrap();
        let back = read_index(text.as_bytes());
        assert_eq!(back[0].paper_verdict, written.paper_verdict);
        assert_eq!(back[0].paper_reason, written.paper_reason);
        assert_eq!(back[0].paper_kind, written.paper_kind);
        assert_eq!(back[0].xmp, written.xmp);
        assert_eq!(back[0].isbn, written.isbn);
        assert_eq!(back[0].arxiv_category, written.arxiv_category);
        assert_eq!(back[0].doi_source, written.doi_source);
        assert_eq!(back[0].arxiv_source, written.arxiv_source);
    }

    fn candidate(path: &str) -> ScanCandidate {
        ScanCandidate {
            path: PathBuf::from(path),
            mtime_ms: 1,
            size: 2,
            epoch: 0,
            unchanged: false,
        }
    }

    /// The user's old Elsevier scans: no DOI anywhere, an Info Title of
    /// "PII: ..." that is no title, and nothing to look up with - until the
    /// PII is read as the DOI it stands for.
    #[test]
    fn a_pii_stands_in_for_a_missing_doi_once_its_issn_checks_out() {
        let page = "JOURNAL OF COMPUTER AND SYSTEM SCIENCES 37, 130-143 (1988)\nA Paper Title";
        const INFO_TITLE: &str = "PII: 0022-0000(88)90028-1";
        let title = Some(INFO_TITLE);
        assert!(!plausible_info_title(INFO_TITLE));
        let mut entry = entry_from_first_page(&candidate("/lib/old.pdf"), page);
        assert!(entry.doi.is_none());
        derive_from_identifiers(&mut entry, page, title);
        assert_eq!(
            entry.doi.as_deref(),
            Some("10.1016/0022-0000(88)90028-1"),
            "from the Info Title the title filter rejected"
        );
        assert_eq!(entry.doi_source, Some(types::MetaSource::Identifier));
        assert!(entry.arxiv_category.is_none());

        // Printed on the page, with the S the newer scheme prints.
        let page = "Physics Letters B 400 (1997) 1-8\nPII: S0370-2693(97)00412-1\nA title";
        let mut entry = entry_from_first_page(&candidate("/lib/new.pdf"), page);
        derive_from_identifiers(&mut entry, page, None);
        assert_eq!(entry.doi.as_deref(), Some("10.1016/S0370-2693(97)00412-1"));

        // A DOI the page or the name gave is left alone.
        let page = "doi:10.1103/PhysRevLett.78.2690\nPII: S0370-2693(97)00412-1";
        let mut entry = entry_from_first_page(&candidate("/lib/doi.pdf"), page);
        derive_from_identifiers(&mut entry, page, None);
        assert_eq!(entry.doi.as_deref(), Some("10.1103/physrevlett.78.2690"));
        assert_eq!(entry.doi_source, Some(types::MetaSource::FirstPage));

        // A PII whose ISSN fails its check digit is a mis-read, not an
        // identity: the page's bad one is passed over for the title's good
        // one, and a file with only the bad one keeps no DOI.
        let page = "PII: 0022-0001(88)90028-1";
        let mut entry = entry_from_first_page(&candidate("/lib/ocr.pdf"), page);
        derive_from_identifiers(&mut entry, page, title);
        assert_eq!(entry.doi.as_deref(), Some("10.1016/0022-0000(88)90028-1"));
        let mut entry = entry_from_first_page(&candidate("/lib/ocr.pdf"), page);
        derive_from_identifiers(&mut entry, page, None);
        assert!(entry.doi.is_none());
    }

    #[test]
    fn the_stamps_category_is_kept_when_it_names_the_files_own_id() {
        let page = "arXiv:1706.03762v5 [cs.CL] 6 Dec 2017\nAttention Is All You Need";
        let mut entry = entry_from_first_page(&candidate("/lib/a.pdf"), page);
        assert_eq!(entry.arxiv.as_deref(), Some("1706.03762"));
        derive_from_identifiers(&mut entry, page, None);
        assert_eq!(entry.arxiv_category.as_deref(), Some("cs.CL"));

        // The file name named another id: the stamp is not this file's.
        entry.arxiv = Some("2401.00001".into());
        derive_from_identifiers(&mut entry, page, None);
        assert!(entry.arxiv_category.is_none());

        // No stamp, no category - and a rescan takes it back.
        let mut entry = entry_from_first_page(&candidate("/lib/b.pdf"), "A title only");
        entry.arxiv_category = Some("stale".into());
        derive_from_identifiers(&mut entry, "A title only", None);
        assert!(entry.arxiv_category.is_none());
    }

    #[test]
    fn cited_identifiers_are_deduped_folded_and_never_the_documents_own() {
        let text = "arXiv:2401.00001v2 [cond-mat.stat-mech] 3 Jan 2024\n\
            Physical Review Letters 78, 2690 doi:10.1000/own\n\
            [1] C. Jarzynski, doi:10.1103/PhysRevLett.78.2690.\n\
            [2] the same, https://doi.org/10.1103/physrevlett.78.2690\n\
            [3] a preprint, doi:10.48550/arXiv.1901.00002\n\
            [4] the same preprint, arxiv.org/abs/1901.00002v1\n\
            [5] ourselves again, arXiv:2401.00001";
        let cites = cited_identifiers(text, Some("10.1000/OWN"), Some("2401.00001"));
        assert_eq!(
            cites,
            vec![
                CitationIdDto::Doi("10.1103/physrevlett.78.2690".into()),
                CitationIdDto::ArXiv("1901.00002".into()),
            ]
        );
        assert!(cited_identifiers("no identifiers here", None, None).is_empty());
    }

    #[test]
    fn the_backfill_reads_cites_out_of_an_existing_sidecar() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("a.pdf");
        fs::write(&path, b"%PDF-1.7\n").expect("pdf");
        let (mtime_ms, size) = file_stamp(&path).expect("stamp");
        crate::search::librarytext::write_sidecar(
            dir.path(),
            &path,
            mtime_ms,
            size,
            &[
                (
                    0,
                    "Ours: arXiv:2401.00009. See doi:10.1000/x and".to_string(),
                ),
                (
                    3,
                    "also arXiv:2401.00003 and doi:10.1000/x again".to_string(),
                ),
            ],
        )
        .expect("sidecar");
        let mut entry = indexed_entry(&path, mtime_ms, size);
        entry.arxiv = Some("2401.00009".into());
        let expected = vec![
            CitationIdDto::Doi("10.1000/x".into()),
            CitationIdDto::ArXiv("2401.00003".into()),
        ];
        assert_eq!(backfill_cites(dir.path(), &entry), expected);

        // A sidecar for other bytes is not trusted: nothing, finally.
        let mut stale = entry.clone();
        stale.size += 1;
        assert!(backfill_cites(dir.path(), &stale).is_empty());

        // The bookkeeping: only indexed entries without cites are handed
        // out, and none once they are filled in.
        let mut state = LibraryState::load(dir.path().to_path_buf());
        let other = dir.path().join("b.pdf");
        state.entries.insert(path.clone(), entry);
        let mut unindexed = indexed_entry(&other, 1, 1);
        unindexed.text_indexed = false;
        state.entries.insert(other.clone(), unindexed);
        assert!(state.has_cites_work());
        assert_eq!(state.next_cites_candidate().expect("candidate").path, path);
        state.finish_cites(&path, expected.clone());
        assert!(
            !state.has_cites_work(),
            "an unindexed entry is the text pass's"
        );
        assert_eq!(state.entries[&path].cites, Some(expected.clone()));

        // The text pass records cites with the text, so there is nothing to
        // backfill behind it.
        state.finish_text(&other, Vec::new());
        assert!(state.entries[&other].text_indexed);
        assert_eq!(state.entries[&other].cites, Some(Vec::new()));
        assert!(!state.has_cites_work());

        // A rescan of the same bytes keeps what was extracted; new bytes
        // start over, like the sidecar.
        let same = ScanCandidate {
            path: path.clone(),
            mtime_ms,
            size,
            epoch: state.scan_epoch,
            unchanged: false,
        };
        assert!(state.finish_candidate(&same, Some(indexed_entry(&path, mtime_ms, size))));
        assert_eq!(state.entries[&path].cites, Some(expected));
        let changed = ScanCandidate {
            size: size + 1,
            ..same
        };
        state.finish_candidate(&changed, Some(indexed_entry(&path, mtime_ms, size + 1)));
        assert_eq!(state.entries[&path].cites, None);
        assert!(state.has_cites_work());
    }

    /// The re-derivation an upgrade relies on: an index older than v9 has
    /// its cites cleared, and the backfill rebuilds them from the text
    /// sidecar alone, without PDFium - explicit identifiers and APS journal
    /// references wherever they are printed, and no refs sidecar, which is
    /// the reference provider's to write.
    #[test]
    fn the_backfill_reads_identifiers_and_journal_dois_and_leaves_refs_to_the_provider() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("2512.24540v3.pdf");
        fs::write(&path, b"%PDF-1.7\n").expect("pdf");
        let (mtime_ms, size) = file_stamp(&path).expect("stamp");
        crate::search::librarytext::write_sidecar(
            dir.path(),
            &path,
            mtime_ms,
            size,
            &[
                (
                    0,
                    "Higher-order response theory. Published as Phys. Rev. E 99, 000001. See [1] and [2]."
                        .to_string(),
                ),
                (
                    9,
                    "Acknowledgments. We thank X. References [1] A. Zhong and M. R. DeWeese, Beyond linear response:\n\
                     Equivalence between thermodynamic geometry and optimal transport,\n\
                     Phys. Rev. Lett. 133, 057102 (2024). [2] B. Two, arXiv:2401.00003 (2024)."
                        .to_string(),
                ),
            ],
        )
        .expect("sidecar");

        let old = read_index(
            serde_json::json!({
                "version": 8,
                "entries": [{
                    "path": path, "mtime_ms": mtime_ms, "size": size,
                    "fingerprint": "fp", "text_indexed": true, "doi": "10.1103/physreve.99.000001",
                    "cites": [{ "scheme": "arXiv", "value": "2401.00003" }]
                }]
            })
            .to_string()
            .as_bytes(),
        );
        assert_eq!(old[0].cites, None, "re-derived");
        assert_eq!(
            old[0].fingerprint, None,
            "and rescanned once, for the first page's layout (v10)"
        );
        assert!(!old[0].text_indexed, "and its text extracted again (v11)");

        let mut state = LibraryState::load(dir.path().to_path_buf());
        // The backfill, for text already on disk whose cites are unknown.
        let mut entry = old[0].clone();
        entry.text_indexed = true;
        state.entries.insert(path.clone(), entry);
        let entry = state.next_cites_candidate().expect("backfill work");
        let cites = backfill_cites(dir.path(), &entry);
        assert_eq!(
            cites,
            vec![
                CitationIdDto::ArXiv("2401.00003".into()),
                CitationIdDto::Doi("10.1103/physrevlett.133.057102".into()),
            ],
            "never its own journal reference"
        );
        state.finish_cites(&path, cites);
        assert!(!state.has_cites_work());
        assert!(
            !crate::search::librarytext::refs_path(dir.path(), &path).exists(),
            "no refs sidecar without the provider"
        );

        // Removing the text sidecar removes a provider's refs with it.
        references::write_refs(dir.path(), &path, mtime_ms, size, "p@1", &[]).expect("refs");
        crate::search::librarytext::remove_sidecar(dir.path(), &path);
        assert!(!crate::search::librarytext::refs_path(dir.path(), &path).exists());
    }

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

    /// One box per character, laid out left to right, with `gap` points of
    /// clear space after each index named in `gaps`.
    fn laid_out(text: &str, gaps: &[usize], gap: f32) -> Vec<[f32; 4]> {
        let advance = 5.0f32;
        let mut boxes = Vec::new();
        let mut x = 72.0f32;
        for (i, _) in text.chars().enumerate() {
            boxes.push([x, 700.0, x + advance, 710.0]);
            x += advance;
            if gaps.contains(&i) {
                x += gap;
            }
        }
        boxes
    }

    /// The bug: PDFium reports a title with no spaces in it, because the file
    /// encodes word gaps as position rather than as characters.
    #[test]
    fn recovers_word_spaces_from_character_geometry() {
        let text = "ATheoryofSaddle";
        let boxes = laid_out(text, &[0, 6, 8], 4.0);
        assert_eq!(lines_from_chars(text, &boxes), vec!["A Theory of Saddle"]);
    }

    /// An earlier version skipped any run containing a space, which left
    /// "Majority-of-Three: OrderedProofs" exactly as broken as before.
    #[test]
    fn repairs_text_that_is_only_partly_spaced() {
        let text = "Three: OrderedProofs";
        let boxes = laid_out(text, &[13], 4.0);
        assert_eq!(
            lines_from_chars(text, &boxes),
            vec!["Three: Ordered Proofs"]
        );
    }

    #[test]
    fn does_not_split_a_word_with_ordinary_letter_spacing() {
        let text = "Saddle";
        assert_eq!(
            lines_from_chars(text, &laid_out(text, &[], 0.0)),
            vec!["Saddle"]
        );
    }

    #[test]
    fn ignores_a_gap_too_small_to_be_a_word_space() {
        let text = "Saddle";
        let boxes = laid_out(text, &[0, 1, 2, 3, 4], 0.5);
        assert_eq!(lines_from_chars(text, &boxes), vec!["Saddle"]);
    }

    #[test]
    fn does_not_double_a_space_the_file_already_supplies() {
        let text = "A Theory";
        let boxes = laid_out(text, &[0, 1], 4.0);
        assert_eq!(lines_from_chars(text, &boxes), vec!["A Theory"]);
    }

    /// Word gaps and line breaks are different signals: a new baseline ends the
    /// line however wide the horizontal gap happens to be.
    #[test]
    fn breaks_lines_on_a_new_baseline() {
        let text = "TitleAuthor";
        let mut boxes = laid_out("Title", &[], 0.0);
        for i in 0..6 {
            let x = 72.0 + i as f32 * 5.0;
            boxes.push([x, 680.0, x + 5.0, 690.0]);
        }
        assert_eq!(lines_from_chars(text, &boxes), vec!["Title", "Author"]);
    }

    /// Geometry shorter than the text must not cost the text: characters past
    /// the boxes are emitted, just without gap detection.
    #[test]
    fn keeps_every_character_when_geometry_runs_out() {
        let text = "ATheoryof";
        let boxes = laid_out(text, &[0], 4.0);
        let out = lines_from_chars(text, &boxes[..4]).join("");
        assert!(out.contains("Theoryof"), "text was truncated: {out:?}");
        assert_eq!(out.replace(' ', ""), text);
    }

    #[test]
    fn falls_back_to_the_files_own_lines_without_geometry() {
        assert_eq!(
            lines_from_chars("Title\ndoi:10.1/x", &[]),
            vec!["Title", "doi:10.1/x"]
        );
    }

    #[test]
    fn no_characters_is_empty() {
        assert!(lines_from_chars("", &[]).is_empty());
        assert_eq!(text_from_page("", &[], &[]), "");
    }

    #[test]
    fn text_from_page_joins_lines_with_newlines() {
        let text = "TitleAuthor";
        let mut boxes = laid_out("Title", &[], 0.0);
        for i in 0..6 {
            let x = 72.0 + i as f32 * 5.0;
            boxes.push([x, 680.0, x + 5.0, 690.0]);
        }
        let rects: Vec<[f32; 4]> = boxes
            .iter()
            .map(|b| [b[0], b[1], b[2] - b[0], b[3] - b[1]])
            .collect();
        assert_eq!(text_from_page(text, &rects, &[]), "Title\nAuthor");
    }

    /// Per-character advances, in ems, of Times-Roman, near enough: what
    /// makes a fixture's median advance look like a real page's.
    fn em_width(ch: char) -> f32 {
        match ch {
            'i' | 'j' | 'l' | 't' | 'f' | 'r' | 'I' => 0.28,
            'm' | 'w' | 'M' | 'W' => 0.78,
            c if c.is_ascii_uppercase() => 0.67,
            c if c.is_ascii_digit() => 0.5,
            _ => 0.47,
        }
    }

    /// A line as PDFium reports it from a LaTeX PDF: no space characters,
    /// each glyph's loose box in the real `[x, y_bottom, w, h]` form,
    /// letters flush, `gap_em` of clear space at each word boundary.
    fn latex_line(
        words: &[&str],
        size: f32,
        left: f32,
        baseline: f32,
        gap_em: f32,
    ) -> (String, Vec<[f32; 4]>) {
        let mut raw = String::new();
        let mut rects = Vec::new();
        let mut x = left;
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                x += gap_em * size;
            }
            for ch in word.chars() {
                let w = em_width(ch) * size;
                raw.push(ch);
                rects.push([x, baseline - 0.22 * size, w, 1.1 * size]);
                x += w;
            }
        }
        (raw, rects)
    }

    /// The bug, end to end: real-form boxes from a LaTeX title with no
    /// spaces in its character stream come back as words.
    #[test]
    fn a_latex_line_in_real_box_form_gets_its_word_spaces() {
        let words = [
            "Private",
            "and",
            "polynomial",
            "time",
            "algorithms",
            "for",
            "learning",
            "Gaussians",
        ];
        let (raw, rects) = latex_line(&words, 17.0, 72.0, 700.0, 0.25);
        assert!(!raw.contains(' '));
        assert_eq!(
            text_from_page(&raw, &rects, &[]),
            "Private and polynomial time algorithms for learning Gaussians"
        );
        // Tightly justified body text: a word space squeezed to 0.17 em.
        let (raw, rects) = latex_line(&["Majority", "of", "Three"], 10.0, 72.0, 600.0, 0.17);
        assert_eq!(text_from_page(&raw, &rects, &[]), "Majority of Three");
    }

    /// The arXiv stamp runs up the left margin. Its characters stack
    /// vertically, so measured like running text each one starts a new line.
    /// Excluded from the geometry, it reads as the file spells it, on its
    /// own line, and the running text around it is unaffected.
    #[test]
    fn a_rotated_stamp_stays_whole_and_apart() {
        let stamp = "arXiv:2103.01234v1 [cs.LG] 1 Mar 2021";
        let mut raw = String::new();
        let mut rects = Vec::new();
        let mut upright = Vec::new();
        let (title, title_rects) =
            latex_line(&["Private", "and", "polynomial"], 17.0, 72.0, 700.0, 0.25);
        raw.push_str(&title);
        rects.extend(title_rects);
        upright.extend(std::iter::repeat_n(true, title.chars().count()));
        let mut y = 250.0f32;
        for ch in stamp.chars() {
            raw.push(ch);
            // Rotated 90 degrees: the glyph's advance runs up the page.
            let advance = em_width(ch) * 20.0;
            rects.push(if ch == ' ' {
                [0.0; 4]
            } else {
                [14.0, y, 22.0, advance]
            });
            upright.push(false);
            y += advance;
        }
        let (body, body_rects) = latex_line(&["We", "study", "learning"], 10.0, 72.0, 600.0, 0.3);
        raw.push_str(&body);
        rects.extend(body_rects);
        upright.extend(std::iter::repeat_n(true, body.chars().count()));

        assert_eq!(
            text_from_page(&raw, &rects, &upright),
            format!("Private and polynomial\n{stamp}\nWe study learning")
        );
        // What the flags prevent: the same page read as all running text
        // shreds the stamp.
        let shredded = text_from_page(&raw, &rects, &[]);
        assert!(shredded.lines().count() > 10, "{shredded}");
    }

    /// PDFium joins a word hyphenated across lines and marks the join with
    /// U+0002; the next line's baseline must not split it again.
    #[test]
    fn a_word_hyphenated_across_lines_stays_whole() {
        let (first, mut rects) = latex_line(&["the", "mi"], 10.0, 72.0, 700.0, 0.3);
        rects.push([0.0; 4]);
        let (second, second_rects) = latex_line(&["crostate", "x"], 10.0, 72.0, 688.0, 0.3);
        rects.extend(second_rects);
        let raw = format!("{first}\u{2}{second}");
        assert_eq!(text_from_page(&raw, &rects, &[]), "the microstate x");
        assert!(!word_breaks(&raw, &rects, &[]).contains(&6));
    }

    #[test]
    fn word_breaks_name_the_index_each_space_goes_before() {
        let (raw, rects) = latex_line(&["A", "Theory", "of"], 10.0, 72.0, 700.0, 0.3);
        assert_eq!(word_breaks(&raw, &rects, &[]), vec![1, 7]);
        // Two lines: the break before the second is a line break.
        let (second, second_rects) = latex_line(&["Saddle"], 10.0, 72.0, 680.0, 0.3);
        let raw = format!("{raw}{second}");
        let rects: Vec<_> = rects.into_iter().chain(second_rects).collect();
        assert_eq!(word_breaks(&raw, &rects, &[]), vec![1, 7, 9]);
        assert!(word_breaks("no geometry", &[], &[]).is_empty());
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
        // The arXiv id must be the stamp's: a bare one is a citation.
        let entry = entry_from_first_page(
            &candidate,
            "A sufficiently descriptive paper title\ndoi:10.1000/ABC\narXiv:2401.12345v2 [cs.LG] 20 Jan 2024",
        );
        assert_eq!(
            entry.title.as_deref(),
            Some("A sufficiently descriptive paper title")
        );
        assert_eq!(entry.doi.as_deref(), Some("10.1000/abc"));
        assert_eq!(entry.doi_source, Some(types::MetaSource::FirstPage));
        assert_eq!(entry.arxiv.as_deref(), Some("2401.12345"));
        assert_eq!(entry.arxiv_source, Some(types::MetaSource::FirstPage));
    }

    #[test]
    fn the_head_of_a_page_ends_at_its_reference_list() {
        let body = "A Title Long Enough\ndoi:10.1000/own\nAbstract\n";
        for heading in [
            "References",
            "REFERENCES",
            "References:",
            "6. References",
            "6 References",
            "VI. References",
            "Bibliography",
            "Literature Cited",
            "Works cited",
            "  References  ",
        ] {
            let text = format!("{body}{heading}\n[1] doi:10.1000/cited");
            assert_eq!(page_head(&text), body, "{heading:?}");
        }
        // No heading: the whole page. A line that only contains the word,
        // or carries a page number, is not the heading.
        for text in [
            "A Title\ndoi:10.1000/own\nReferences are listed at the end.\n[1] doi:10.1000/cited",
            "Contents\nReferences 231\nIndex 240",
            "A Title\nSee the references in [1].",
            "",
        ] {
            assert_eq!(page_head(text), text, "{text:?}");
        }
        // The first heading wins, and a heading on the first line leaves
        // nothing.
        assert_eq!(page_head("Head\nReferences\nBibliography\nmore"), "Head\n");
        assert_eq!(page_head("References\n[1] x"), "");
    }

    /// The demo library's bug: a one-page paper whose own DOI is in the
    /// head and whose reference list cites a preprint by its arXiv id was
    /// filed under the preprint, and the citation extractor then dropped
    /// the preprint as a self-citation.
    #[test]
    fn own_identifiers_come_from_the_head_and_an_arxiv_id_only_from_the_stamp() {
        let page = "A Demonstration of Library Scanning\nJ. Doe\nDOI: 10.5555/speedyf.001\nAbstract\nWe cite things.\nReferences\n[1] A. Vaswani et al. Attention is all you need. arXiv:1706.03762, 2017.\n[2] Someone. doi:10.1000/cited";
        let entry = entry_from_first_page(&candidate("/lib/demo.pdf"), page);
        assert_eq!(entry.doi.as_deref(), Some("10.5555/speedyf.001"));
        assert_eq!(entry.arxiv, None, "a cited id is not the paper's");
        assert_eq!(
            cited_identifiers(page, entry.doi.as_deref(), entry.arxiv.as_deref()),
            vec![
                CitationIdDto::ArXiv("1706.03762".into()),
                CitationIdDto::Doi("10.1000/cited".into()),
            ],
            "and it is cited, not self"
        );

        // A bare arXiv id anywhere - even in the head - is a citation; the
        // stamp, wherever PDFium puts it, is the file's own.
        let cited_in_head = "A Title Long Enough\nWe build on arXiv:1706.03762 here.\nAbstract";
        assert_eq!(
            entry_from_first_page(&candidate("/lib/a.pdf"), cited_in_head).arxiv,
            None
        );
        let stamped_last = "A Title Long Enough\nAbstract\nReferences\n[1] arXiv:1111.11111\narXiv:2401.00001v2 [cs.CL] 3 Jan 2024";
        let entry = entry_from_first_page(&candidate("/lib/b.pdf"), stamped_last);
        assert_eq!(entry.arxiv.as_deref(), Some("2401.00001"));
        assert_eq!(entry.arxiv_source, Some(types::MetaSource::FirstPage));

        // A DOI, a PII or an ISBN below the heading is someone else's.
        let below = "A Title Long Enough\nReferences\n[1] doi:10.1000/cited\n[2] PII: S0370-2693(97)00412-1\n[3] ISBN 0-306-40615-2";
        let mut entry = entry_from_first_page(&candidate("/lib/c.pdf"), below);
        derive_from_identifiers(&mut entry, below, None);
        assert_eq!(entry.doi, None);
        assert_eq!(entry.doi_source, None);
        assert_eq!(entry.isbn, None);

        // The file name's say is recorded as the name's.
        let named = entry_from_filename(&candidate("/lib/arXiv-2401.12345v2.pdf")).unwrap();
        assert_eq!(named.arxiv.as_deref(), Some("2401.12345"));
        assert_eq!(named.arxiv_source, Some(types::MetaSource::Filename));
        assert_eq!(named.doi_source, None);
        let named = entry_from_filename(&candidate("/lib/10.1145_1234567.pdf")).unwrap();
        assert_eq!(named.doi.as_deref(), Some("10.1145/1234567"));
        assert_eq!(named.doi_source, Some(types::MetaSource::Filename));
        assert!(entry_from_filename(&candidate("/lib/notes.pdf")).is_none());
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
    fn an_older_index_loads_but_is_rescanned_once() {
        let entries = read_index(
            serde_json::json!({
                "version": 2,
                "entries": [{
                    "path": "/lib/a.pdf", "mtime_ms": 1, "size": 2,
                    "fingerprint": "2-abc", "text_indexed": true
                }]
            })
            .to_string()
            .as_bytes(),
        );
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].fingerprint.is_none(),
            "no fingerprint means rescan"
        );
        assert!(
            !entries[0].text_indexed,
            "v11: the text is extracted again, with its word spaces"
        );

        let current = read_index(
            serde_json::json!({
                "version": FORMAT_VERSION,
                "entries": [{
                    "path": "/lib/a.pdf", "mtime_ms": 1, "size": 2,
                    "fingerprint": "2-abc", "text_indexed": true
                }]
            })
            .to_string()
            .as_bytes(),
        );
        assert_eq!(current[0].fingerprint.as_deref(), Some("2-abc"));
        assert!(current[0].text_indexed, "a current sidecar is kept");
    }

    fn layout_of(
        title: Option<&str>,
        authors: &[(&str, &str)],
        high: bool,
    ) -> firstpage::FirstPage {
        let confidence = if high {
            firstpage::Confidence::High
        } else {
            firstpage::Confidence::Low
        };
        firstpage::FirstPage {
            title: title.map(str::to_owned),
            authors: authors
                .iter()
                .map(|(given, family)| types::Author {
                    given: (*given).into(),
                    family: (*family).into(),
                })
                .collect(),
            title_confidence: confidence,
            authors_confidence: confidence,
        }
    }

    /// The layout's title replaces the "first long line" guess, which
    /// stays as the unconfident fallback; its authors ride along.
    #[test]
    fn the_first_page_layout_supplies_title_and_authors() {
        let page = "PHYSICAL REVIEW LETTERS 133, 057102 (2024)\nBeyond Linear Response";
        let layout = layout_of(
            Some("Beyond Linear Response"),
            &[("Adrianne", "Zhong"), ("Michael R.", "DeWeese")],
            true,
        );
        let entry = entry_from_page(&candidate("/lib/a.pdf"), page, Some(&layout));
        assert_eq!(entry.title.as_deref(), Some("Beyond Linear Response"));
        assert!(entry.first_page_title_confident);
        assert!(entry.first_page_authors_confident);
        assert_eq!(entry.first_page_authors.len(), 2);

        let without = entry_from_page(
            &candidate("/lib/a.pdf"),
            page,
            Some(&layout_of(None, &[], true)),
        );
        assert_eq!(
            without.title.as_deref(),
            Some("PHYSICAL REVIEW LETTERS 133, 057102 (2024)"),
            "the old rule, as a fallback"
        );
        assert!(!without.first_page_title_confident);
        assert!(
            !without.first_page_authors_confident,
            "no authors, no confidence"
        );
        assert_eq!(
            entry_from_first_page(&candidate("/lib/a.pdf"), page).title,
            without.title
        );
    }

    /// v10 rescans every entry once for the first page's layout, and v11
    /// once more for the repaired first-page text. The bytes have not
    /// changed, so the cites and refs survive the rescan; the text is
    /// queued for extraction again (v11), which is what replaces them.
    #[test]
    fn the_v11_rescan_keeps_cites_and_refs_and_requeues_the_text() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let pdf = root.path().join("paper.pdf");
        fs::write(&pdf, b"fixture").unwrap();
        let pdf = pdf.canonicalize().unwrap();
        let (mtime_ms, size) = file_stamp(&pdf).unwrap();
        crate::search::librarytext::write_sidecar(
            app_data.path(),
            &pdf,
            mtime_ms,
            size,
            &[(0, "Some text".to_owned())],
        )
        .expect("sidecar");
        references::write_refs(app_data.path(), &pdf, mtime_ms, size, "p@1", &[]).expect("refs");
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "version": 2, "roots": [root.path()] }).to_string(),
        )
        .unwrap();
        fs::write(
            app_data.path().join(INDEX_FILE),
            serde_json::json!({ "version": 9, "entries": [{
                "path": pdf, "mtime_ms": mtime_ms, "size": size, "fingerprint": "fp",
                "title": "A paper", "text_indexed": true,
                "cites": [{ "scheme": "arXiv", "value": "2401.00003" }]
            }]})
            .to_string(),
        )
        .unwrap();

        let mut state = LibraryState::load(app_data.path().to_path_buf());
        let loaded = state.entry_for_path(&pdf).unwrap();
        assert!(loaded.cites.is_some(), "v9 cites are current");
        let candidate = state.next_candidate().expect("queued");
        assert!(!candidate.unchanged, "v9: rescanned once");
        let layout = layout_of(Some("A Paper Title"), &[("Ann", "Barker")], true);
        let mut scanned = entry_from_page(&candidate, "", Some(&layout));
        scanned.fingerprint = Some("fp".into());
        assert!(state.finish_candidate(&candidate, Some(scanned)));
        let stored = state.entry_for_path(&pdf).unwrap();
        assert!(!stored.text_indexed, "v11: the text is extracted again");
        assert_eq!(
            state.next_text_candidate().map(|entry| entry.path),
            Some(pdf.clone())
        );
        assert_eq!(
            stored.cites,
            Some(vec![CitationIdDto::ArXiv("2401.00003".into())]),
            "and their cites"
        );
        assert!(crate::search::librarytext::refs_path(app_data.path(), &pdf).exists());
        assert_eq!(stored.first_page_authors.len(), 1);
        assert_eq!(stored.title.as_deref(), Some("A Paper Title"));
    }

    #[test]
    fn the_arxiv_version_comes_off_the_stamp_or_an_arxiv_file_name() {
        let page = "arXiv:1706.03762v5 [cs.CL] 6 Dec 2017\nAttention Is All You Need";
        let mut entry = entry_from_first_page(&candidate("/lib/attention.pdf"), page);
        derive_from_identifiers(&mut entry, page, None);
        assert_eq!(entry.arxiv_version, Some(5));
        assert_eq!(
            mainfile::traits_of(&entry).version,
            Some(5),
            "not in the name"
        );

        // A name that gave the id gives the version, when there is no stamp.
        let mut named = entry_from_filename(&candidate("/lib/1706.03762v3.pdf")).unwrap();
        derive_from_identifiers(&mut named, "No stamp here", None);
        assert_eq!(named.arxiv_version, Some(3));
        // The stamp names the same id: the file says which version it is.
        derive_from_identifiers(&mut named, page, None);
        assert_eq!(named.arxiv_version, Some(5));
        // The name named another id: the stamp is not this file's.
        let mut other = entry_from_filename(&candidate("/lib/2401.00001v2.pdf")).unwrap();
        derive_from_identifiers(&mut other, page, None);
        assert_eq!(other.arxiv_version, Some(2));

        // A "v3" in a name that is no arXiv id is no arXiv version, though
        // the main-file rule still reads it off the name; a rescan without
        // a stamp takes a stale version back.
        let mut plain = entry_from_first_page(&candidate("/lib/paper_v3.pdf"), "A title");
        plain.arxiv_version = Some(9);
        derive_from_identifiers(&mut plain, "A title", None);
        assert_eq!(plain.arxiv_version, None);
        assert_eq!(mainfile::traits_of(&plain).version, Some(3));
    }

    #[test]
    fn a_v12_index_is_rescanned_once_for_the_arxiv_version_keeping_its_text() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let pdf = root.path().join("attention.pdf");
        fs::write(&pdf, b"fixture").unwrap();
        let pdf = pdf.canonicalize().unwrap();
        let (mtime_ms, size) = file_stamp(&pdf).unwrap();
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "version": 2, "roots": [root.path()] }).to_string(),
        )
        .unwrap();
        fs::write(
            app_data.path().join(INDEX_FILE),
            serde_json::json!({ "version": 12, "entries": [{
                "path": pdf, "mtime_ms": mtime_ms, "size": size, "fingerprint": "fp",
                "title": null, "doi": null, "arxiv": "1706.03762", "text_indexed": true,
                "cites": []
            }]})
            .to_string(),
        )
        .unwrap();

        let mut state = LibraryState::load(app_data.path().to_path_buf());
        assert_eq!(state.entry_for_path(&pdf).unwrap().arxiv_version, None);
        let candidate = state.next_candidate().expect("queued");
        assert!(!candidate.unchanged, "v12: rescanned once");
        let page = "arXiv:1706.03762v5 [cs.CL] 6 Dec 2017";
        let mut scanned = entry_from_page(&candidate, page, None);
        derive_from_identifiers(&mut scanned, page, None);
        scanned.fingerprint = Some("fp".into());
        assert!(state.finish_candidate(&candidate, Some(scanned)));
        let stored = state.entry_for_path(&pdf).unwrap();
        assert_eq!(stored.arxiv_version, Some(5));
        assert!(stored.text_indexed, "metadata only: the text stands");
        assert!(state.next_candidate().is_none(), "once");
    }

    #[test]
    fn the_v12_rescan_reextracts_text_only_for_files_worth_ocr() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let mut pdfs = Vec::new();
        for name in ["born-digital.pdf", "scan.pdf"] {
            let pdf = root.path().join(name);
            fs::write(&pdf, name.as_bytes()).unwrap();
            pdfs.push(pdf.canonicalize().unwrap());
        }
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "version": 2, "roots": [root.path()] }).to_string(),
        )
        .unwrap();
        let entries: Vec<_> = pdfs
            .iter()
            .map(|pdf| {
                let (mtime_ms, size) = file_stamp(pdf).unwrap();
                serde_json::json!({
                    "path": pdf, "mtime_ms": mtime_ms, "size": size, "fingerprint": "fp",
                    "title": null, "doi": null, "arxiv": null, "text_indexed": true, "cites": []
                })
            })
            .collect();
        fs::write(
            app_data.path().join(INDEX_FILE),
            serde_json::json!({ "version": 11, "entries": entries }).to_string(),
        )
        .unwrap();

        let mut state = LibraryState::load(app_data.path().to_path_buf());
        assert!(
            state.all_entries().iter().all(|entry| entry.text_indexed),
            "v11 text stands until the rescan says otherwise"
        );
        while let Some(candidate) = state.next_candidate() {
            assert!(!candidate.unchanged, "v11: rescanned once");
            let mut scanned = entry_from_page(&candidate, "", None);
            scanned.fingerprint = Some("fp".into());
            scanned.ocr_wanted = candidate.path.ends_with("scan.pdf");
            assert!(state.finish_candidate(&candidate, Some(scanned)));
        }
        assert!(state.entry_for_path(&pdfs[0]).unwrap().text_indexed);
        assert!(
            !state.entry_for_path(&pdfs[1]).unwrap().text_indexed,
            "the scan's text is extracted again, with OCR"
        );
        assert_eq!(
            state.next_text_candidate().map(|e| e.path),
            Some(pdfs[1].clone())
        );

        // Once OCR'd, a later rescan of the same bytes keeps it.
        state.finish_text(&pdfs[1], Vec::new());
        let mut again = state.entry_for_path(&pdfs[1]).unwrap();
        again.text_indexed = false;
        state.upsert_entry(again);
        assert!(state.entry_for_path(&pdfs[1]).unwrap().text_indexed);
    }

    #[test]
    fn a_bare_array_index_still_loads() {
        // What every existing install has on disk right now.
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("library root");
        let pdf = root.path().join("paper.pdf");
        fs::write(&pdf, b"fixture").unwrap();
        // Stored entries are always canonical - `collect_pdfs` re-validates
        // every hit through `contained_canonical_path`. On macOS a tempdir is
        // /var/... which canonicalizes to /private/var/..., so a fixture that
        // skipped this would look like a file that had moved away.
        let pdf = pdf.canonicalize().expect("canonical");
        let (mtime_ms, size) = file_stamp(&pdf).expect("fingerprint");
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
            let (mtime_ms, size) = file_stamp(&path).expect("fingerprint");
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
                    fingerprint: None,
                    info_title: None,
                    info_author: None,
                    info_year: None,
                    cites: None,
                    paper_verdict: None,
                    paper_reason: None,
                    paper_kind: None,
                    xmp: None,
                    isbn: None,
                    arxiv_category: None,
                    arxiv_version: None,
                    first_page_authors: Vec::new(),
                    first_page_title_confident: false,
                    first_page_authors_confident: false,
                    ocr_wanted: false,
                    ocr_deferred: false,
                    doi_source: None,
                    arxiv_source: None,
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

    #[test]
    fn content_fingerprint_survives_a_rename_and_not_an_edit() {
        let dir = tempfile::tempdir().expect("dir");
        let small = dir.path().join("small.pdf");
        fs::write(&small, b"%PDF small").unwrap();
        let moved = dir.path().join("renamed.pdf");
        fs::copy(&small, &moved).unwrap();
        let fingerprint = content_fingerprint(&small).expect("fingerprint");
        assert_eq!(
            content_fingerprint(&moved).as_deref(),
            Some(fingerprint.as_str())
        );
        assert!(fingerprint.starts_with("a-"), "{fingerprint}");
        fs::write(&moved, b"%PDF smalL").unwrap();
        assert_ne!(content_fingerprint(&moved).unwrap(), fingerprint);

        // Large files hash only their ends: a change in the middle is not
        // seen, a change at either end is.
        let big = dir.path().join("big.pdf");
        let mut bytes = vec![7_u8; 3 * FINGERPRINT_EDGE as usize];
        fs::write(&big, &bytes).unwrap();
        let before = content_fingerprint(&big).unwrap();
        bytes[FINGERPRINT_EDGE as usize + 10] = 8;
        fs::write(&big, &bytes).unwrap();
        assert_eq!(content_fingerprint(&big).unwrap(), before);
        let last = bytes.len() - 1;
        bytes[last] = 8;
        fs::write(&big, &bytes).unwrap();
        assert_ne!(content_fingerprint(&big).unwrap(), before);
        assert!(content_fingerprint(&dir.path().join("nope.pdf")).is_none());
    }

    #[test]
    fn info_titles_that_are_file_names_are_ignored() {
        assert!(plausible_info_title(
            "Nonequilibrium Equality for Free Energy Differences"
        ));
        assert!(plausible_info_title("Entropy"));
        assert!(!plausible_info_title(""));
        assert!(!plausible_info_title("  Untitled "));
        assert!(!plausible_info_title("jarzynski1997.pdf"));
        assert!(!plausible_info_title("main.tex"));
        assert!(!plausible_info_title("Microsoft Word - draft.docx"));
        assert!(!plausible_info_title("jarzynski_1997"));
        assert!(!plausible_info_title("paper-final"));
        assert!(plausible_info_title("A self-contained proof"));
        assert!(!plausible_info_title("PII: 0022-0000(88)90028-1"));
        assert!(!plausible_info_title("doi:10.1103/PhysRevLett.78.2690"));
        assert!(!plausible_info_title("10.1016/j.cell.2020.01.001"));
        assert!(!plausible_info_title("2401.12345"));
        assert!(!plausible_info_title("p. 17"));
    }

    #[test]
    fn pdf_dates_yield_a_year() {
        assert_eq!(year_from_pdf_date("D:19970415120000Z"), Some(1997));
        assert_eq!(year_from_pdf_date("20231103"), Some(2023));
        assert_eq!(year_from_pdf_date("D:0000"), None);
        assert_eq!(year_from_pdf_date("D:abcd"), None);
        assert_eq!(year_from_pdf_date(""), None);
    }

    #[test]
    fn an_index_without_fingerprints_is_rescanned_once() {
        let app_data = tempfile::tempdir().expect("app data");
        let root = tempfile::tempdir().expect("root");
        let pdf = root.path().join("paper.pdf");
        fs::write(&pdf, b"fixture").unwrap();
        let pdf = pdf.canonicalize().unwrap();
        let (mtime_ms, size) = file_stamp(&pdf).unwrap();
        fs::write(
            app_data.path().join(ROOT_FILE),
            serde_json::json!({ "version": 2, "roots": [root.path()] }).to_string(),
        )
        .unwrap();
        fs::write(
            app_data.path().join(INDEX_FILE),
            serde_json::json!({ "version": 2, "entries": [{
                "path": pdf, "mtime_ms": mtime_ms, "size": size,
                "doi": null, "arxiv": null, "title": "A paper", "text_indexed": true
            }]})
            .to_string(),
        )
        .unwrap();

        let mut state = LibraryState::load(app_data.path().to_path_buf());
        let candidate = state.next_candidate().expect("queued");
        assert!(!candidate.unchanged, "no fingerprint yet: rescan");
        let mut entry = state.entry_for_path(&pdf).unwrap();
        entry.fingerprint = content_fingerprint(&pdf);
        assert!(state.finish_candidate(&candidate, Some(entry)));
        assert!(
            !state.entry_for_path(&pdf).unwrap().text_indexed,
            "same bytes keep their text state, which v11 reset for extraction"
        );

        state.add_root(root.path().to_path_buf()).unwrap();
        let candidate = state.next_candidate().expect("queued again");
        assert!(candidate.unchanged, "fingerprinted: the stamp decides");
        // A candidate from a superseded scan is refused.
        state.add_root(root.path().to_path_buf()).unwrap();
        assert!(!state.finish_candidate(&candidate, None));

        // Upserting outside a scan takes effect at once.
        let mut fresh = entry_from_first_page(
            &ScanCandidate {
                path: pdf.clone(),
                mtime_ms,
                size: size + 1,
                epoch: 0,
                unchanged: false,
            },
            "",
        );
        fresh.fingerprint = Some("x".into());
        state.upsert_entry(fresh);
        let stored = state.entry_for_path(&pdf).unwrap();
        assert_eq!(stored.size, size + 1);
        assert!(!stored.text_indexed, "different bytes: the sidecar goes");
        assert!(state.paths().contains(&pdf));
    }
}

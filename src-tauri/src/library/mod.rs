//! Local-only citation library indexing and resolution. Directory traversal
//! and persistence are thread-safe, while PDF parsing is deliberately left to
//! the engine worker that owns PDFium.

use crate::engine::citation::{citation_from_filename, find_explicit_citations};
use crate::engine::types::{CitationIdDto, LibraryStatusDto, TextRun};
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

/// Rebuild page lines from positioned text runs.
///
/// The metadata scan used to read PDFium's raw character stream, which for a
/// great many PDFs — LaTeX output especially — contains no space characters at
/// all. Word gaps in those files are *positions*, not characters, so a title
/// came back as "PrivateandpolynomialtimealgorithmsforlearningGaussians".
///
/// Runs carry geometry, so the gaps can be recovered: runs sharing a baseline
/// are one line, ordered left to right, joined by a space unless one side
/// already supplies the whitespace. Lines run down the page, which is
/// descending y — PDF space is y-up.
pub fn lines_from_runs(runs: &[TextRun]) -> Vec<String> {
    /// Two runs are on the same line when their baselines are within this
    /// fraction of the taller one's height. Generous enough for the sub- and
    /// superscripts that sit slightly off a baseline, tight enough not to merge
    /// consecutive lines of body text.
    const SAME_LINE: f32 = 0.6;

    let mut ordered: Vec<&TextRun> = runs.iter().filter(|r| !r.text.is_empty()).collect();
    // Down the page first — PDF space is y-up — then left to right.
    ordered.sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));

    // Grouped into lines *before* anything is ordered horizontally. Sorting by
    // y alone puts a superscript ahead of the character it belongs to, since it
    // sits a few points higher: "x²" came out as "2 x".
    let mut grouped: Vec<Vec<&TextRun>> = Vec::new();
    for run in ordered {
        match grouped.last_mut() {
            // Measured against the line's first run rather than the previous
            // one, so a run of slightly drifting baselines cannot walk a line
            // arbitrarily far from where it started.
            Some(line) if same_line(line[0], run, SAME_LINE) => line.push(run),
            _ => grouped.push(vec![run]),
        }
    }

    grouped
        .into_iter()
        .filter_map(|mut line| {
            line.sort_by(|a, b| a.x.total_cmp(&b.x));
            let mut text = String::new();
            for run in line {
                if !text.is_empty()
                    && !text.ends_with(char::is_whitespace)
                    && !run.text.starts_with(char::is_whitespace)
                {
                    text.push(' ');
                }
                text.push_str(&run.text);
            }
            let text = text.trim().to_string();
            (!text.is_empty()).then_some(text)
        })
        .collect()
}

fn same_line(first: &TextRun, run: &TextRun, tolerance: f32) -> bool {
    (first.y - run.y).abs() <= tolerance * first.h.max(run.h).max(1.0)
}

/// A gap wider than this share of the run's median glyph advance is a word
/// space.
///
/// Measured against the glyphs themselves rather than the font size PDFium
/// reports, because that size is not always reported — a zero there silently
/// disabled the whole repair on exactly the files that needed it. Median
/// advance is always available and needs no font metadata at all: an interword
/// space runs about half a lowercase advance, while letters within a word sit
/// flush or kern slightly negative.
const WORD_GAP_RATIO: f32 = 0.3;

/// Put the spaces back inside one run using its characters' own geometry.
///
/// `lines_from_runs` can only insert spaces *between* runs, and PDFium happily
/// reports a whole line as a single run whose text contains no spaces at all —
/// which is what a title like "ATheoryofSaddleEscapeinDeepNonlinear" is. The
/// gaps in those files are positions between glyphs, and this is where they are
/// read back.
///
/// Runs that are *partly* spaced are repaired too. An earlier version skipped
/// anything already containing a space, which left
/// "Majority-of-Three: OrderedProofs" exactly as broken as before.
pub fn repair_run_text(run: &TextRun, boxes: &[[f32; 4]], sizes: &[f32]) -> String {
    let count = run.text.chars().count();
    if count < 2 {
        return run.text.clone();
    }
    let start = run.start as usize;
    let Some(slice) = boxes.get(start..start + count) else {
        // Geometry missing or misaligned: pass the text through rather than
        // guess at it.
        return run.text.clone();
    };

    let Some(advance) = median_advance(slice) else {
        return run.text.clone();
    };
    let threshold = WORD_GAP_RATIO * advance;

    let mut out = String::with_capacity(run.text.len() + 8);
    let mut previous_right: Option<f32> = None;
    for (offset, ch) in run.text.chars().enumerate() {
        let [x0, _, x1, _] = slice[offset];
        let has_box = x1 > x0;
        if let Some(right) = previous_right {
            // Never doubled: a space the file already supplies, on either side,
            // is the space — this only fills in the ones it left out.
            let already_spaced = ch.is_whitespace() || out.ends_with(char::is_whitespace);
            if has_box && !already_spaced && x0 - right > threshold {
                out.push(' ');
            }
        }
        out.push(ch);
        if has_box {
            previous_right = Some(x1);
        }
        // A glyph with no box leaves `previous_right` where it was, so the next
        // real glyph is still measured from something meaningful.
        let _ = sizes;
    }
    out
}

/// Median width of the glyphs that reported one, as a stand-in for the font's
/// advance. Median rather than mean so a single wide glyph or a stray zero box
/// cannot move it.
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

/// Page lines, with word spacing recovered from character geometry where the
/// file does not encode it.
pub fn lines_from_page(runs: &[TextRun], boxes: &[[f32; 4]], sizes: &[f32]) -> Vec<String> {
    let repaired: Vec<TextRun> = runs
        .iter()
        .map(|run| TextRun {
            text: repair_run_text(run, boxes, sizes),
            ..run.clone()
        })
        .collect();
    lines_from_runs(&repaired)
}

/// The same as one string, lines separated by newlines.
pub fn text_from_page(runs: &[TextRun], boxes: &[[f32; 4]], sizes: &[f32]) -> String {
    lines_from_page(runs, boxes, sizes).join("\n")
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

    fn run(text: &str, x: f32, y: f32) -> TextRun {
        TextRun {
            text: text.into(),
            start: 0,
            x,
            y,
            w: text.len() as f32 * 5.0,
            h: 10.0,
        }
    }

    /// The bug this exists to fix: a PDF that encodes word gaps as position
    /// rather than as space characters produced
    /// "PrivateandpolynomialtimealgorithmsforlearningGaussians".
    #[test]
    fn puts_the_spaces_back_between_positioned_runs() {
        let runs = vec![
            run("Private", 72.0, 700.0),
            run("and", 130.0, 700.0),
            run("polynomial", 165.0, 700.0),
            run("time", 240.0, 700.0),
        ];
        assert_eq!(lines_from_runs(&runs), vec!["Private and polynomial time"]);
    }

    #[test]
    fn does_not_double_a_space_a_run_already_carries() {
        let runs = vec![run("Private ", 72.0, 700.0), run("and", 130.0, 700.0)];
        assert_eq!(lines_from_runs(&runs), vec!["Private and"]);
        let runs = vec![run("Private", 72.0, 700.0), run(" and", 130.0, 700.0)];
        assert_eq!(lines_from_runs(&runs), vec!["Private and"]);
    }

    #[test]
    fn separates_lines_by_baseline() {
        let runs = vec![
            run("Second", 72.0, 680.0),
            run("First", 72.0, 700.0),
            run("line", 110.0, 700.0),
        ];
        assert_eq!(lines_from_runs(&runs), vec!["First line", "Second"]);
    }

    /// Runs arrive in whatever order PDFium emits them, which is not reading
    /// order.
    #[test]
    fn orders_top_down_then_left_to_right() {
        let runs = vec![
            run("world", 130.0, 700.0),
            run("below", 72.0, 660.0),
            run("Hello", 72.0, 700.0),
        ];
        assert_eq!(lines_from_runs(&runs), vec!["Hello world", "below"]);
    }

    /// A sub- or superscript sits slightly off the baseline and belongs to the
    /// line it was set on, not to a line of its own.
    #[test]
    fn keeps_a_slightly_raised_run_on_its_line() {
        let runs = vec![run("x", 72.0, 700.0), run("2", 80.0, 703.0)];
        assert_eq!(lines_from_runs(&runs), vec!["x 2"]);
    }

    #[test]
    fn ignores_empty_runs_and_blank_lines() {
        let runs = vec![
            run("", 72.0, 700.0),
            run("Title", 80.0, 700.0),
            run("   ", 72.0, 660.0),
        ];
        assert_eq!(lines_from_runs(&runs), vec!["Title"]);
    }

    /// One character box per character, laid out left to right with `gap`
    /// points of clear space wherever `spaces` says a word ends.
    fn glyphs(
        text: &str,
        size: f32,
        gaps: &[usize],
        gap: f32,
    ) -> (TextRun, Vec<[f32; 4]>, Vec<f32>) {
        let mut boxes = Vec::new();
        let mut x = 72.0f32;
        let advance = size * 0.5;
        for (i, _) in text.chars().enumerate() {
            boxes.push([x, 700.0, x + advance, 710.0]);
            x += advance;
            if gaps.contains(&i) {
                x += gap;
            }
        }
        let run = TextRun {
            text: text.into(),
            start: 0,
            x: 72.0,
            y: 700.0,
            w: x - 72.0,
            h: 10.0,
        };
        let sizes = vec![size; text.chars().count()];
        (run, boxes, sizes)
    }

    /// The bug: PDFium reports a whole title as one run with no spaces in it,
    /// because the file encodes word gaps as position rather than characters.
    #[test]
    fn recovers_word_spaces_from_character_geometry() {
        // "ATheoryofSaddle" with real gaps after A, y, of, e
        let (run, boxes, sizes) = glyphs("ATheoryofSaddle", 10.0, &[0, 6, 8], 4.0);
        assert_eq!(repair_run_text(&run, &boxes, &sizes), "A Theory of Saddle");
    }

    #[test]
    fn leaves_text_alone_when_it_already_has_spaces() {
        let (run, boxes, sizes) = glyphs("A Theory of", 10.0, &[], 0.0);
        assert_eq!(repair_run_text(&run, &boxes, &sizes), "A Theory of");
    }

    /// An earlier version skipped any run containing a space, which left
    /// "Majority-of-Three: OrderedProofs" exactly as broken as before.
    #[test]
    fn repairs_a_run_that_is_only_partly_spaced() {
        let (run, boxes, sizes) = glyphs("Three: OrderedProofs", 10.0, &[13], 4.0);
        assert_eq!(
            repair_run_text(&run, &boxes, &sizes),
            "Three: Ordered Proofs"
        );
    }

    /// PDFium does not always report a font size. Basing the threshold on it
    /// meant a zero silently disabled the repair on exactly the files that
    /// needed it.
    #[test]
    fn works_when_the_font_size_is_not_reported() {
        let (run, boxes, _) = glyphs("ATheoryof", 10.0, &[0, 6], 4.0);
        let zeros = vec![0.0; run.text.chars().count()];
        assert_eq!(repair_run_text(&run, &boxes, &zeros), "A Theory of");
    }

    #[test]
    fn does_not_double_a_space_the_file_already_supplies() {
        let (run, boxes, sizes) = glyphs("A Theory", 10.0, &[0], 4.0);
        assert_eq!(repair_run_text(&run, &boxes, &sizes), "A Theory");
    }

    /// Letters within a word sit flush or kern together; nothing there should
    /// be read as a space.
    #[test]
    fn does_not_split_a_word_with_ordinary_letter_spacing() {
        let (run, boxes, sizes) = glyphs("Saddle", 10.0, &[], 0.0);
        assert_eq!(repair_run_text(&run, &boxes, &sizes), "Saddle");
    }

    #[test]
    fn ignores_a_gap_too_small_to_be_a_word_space() {
        // 0.05em of tracking is not a space at 0.15em threshold.
        let (run, boxes, sizes) = glyphs("Saddle", 10.0, &[0, 1, 2, 3, 4], 0.5);
        assert_eq!(repair_run_text(&run, &boxes, &sizes), "Saddle");
    }

    #[test]
    fn passes_text_through_when_geometry_is_missing() {
        let (run, _, sizes) = glyphs("ATheoryof", 10.0, &[0], 4.0);
        assert_eq!(repair_run_text(&run, &[], &sizes), "ATheoryof");
    }

    /// A run whose boxes do not line up with its characters must not be
    /// repaired from whatever happens to sit at those indices.
    #[test]
    fn passes_text_through_when_boxes_are_short() {
        let (run, boxes, sizes) = glyphs("ATheoryof", 10.0, &[0], 4.0);
        assert_eq!(repair_run_text(&run, &boxes[..3], &sizes), "ATheoryof");
    }

    #[test]
    fn a_single_character_run_is_untouched() {
        let (run, boxes, sizes) = glyphs("A", 10.0, &[], 0.0);
        assert_eq!(repair_run_text(&run, &boxes, &sizes), "A");
    }

    #[test]
    fn repairs_runs_when_building_page_lines() {
        let (run, boxes, sizes) = glyphs("ATheoryofSaddle", 10.0, &[0, 6, 8], 4.0);
        assert_eq!(
            lines_from_page(&[run], &boxes, &sizes),
            vec!["A Theory of Saddle"]
        );
    }

    #[test]
    fn text_from_page_joins_lines_with_newlines() {
        let runs = vec![run("Title", 72.0, 700.0), run("doi:10.1/x", 72.0, 660.0)];
        assert_eq!(text_from_page(&runs, &[], &[]), "Title\ndoi:10.1/x");
    }

    #[test]
    fn no_runs_is_empty_rather_than_a_blank_line() {
        assert!(lines_from_runs(&[]).is_empty());
        assert_eq!(text_from_page(&[], &[], &[]), "");
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

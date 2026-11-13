//! `library.json`: papers, projects and memberships - the one file that holds
//! something the user did and cannot be rebuilt from the folders.
//!
//! It lives in the library data dir, which defaults to app data and can be
//! moved so a sync service carries it. Three rules keep it safe (projects.md
//! sec. 3, sec. 12): it is written atomically, it carries a version from day one, and
//! it is never written over when it cannot be read - a corrupt file is set
//! aside under a new name first, and a file from a newer build is left alone
//! while this build runs read-only against an empty library.

use super::backup::{self, BackupState, LibraryBackupDto};
use super::bibtex::BibtexOptions;
use super::papers::PaperRecord;
use super::persons::PersonsSection;
use super::projects::ProjectRecord;
use super::review::ReviewState;
use super::types::LibraryDataInfoDto;
use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const LIBRARY_FILE: &str = "library.json";
/// Where `library.json` lives when it is not in app data. Only this pointer
/// stays in app data, so the data folder itself never has to know which
/// machine it is on.
const POINTER_FILE: &str = "library-data-dir.json";
const FORMAT_VERSION: u32 = 1;
/// How often a scan's reconciles reach the disk and the windows. Commands
/// save on the spot; a scan touching thousands of files must not.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// The records. The version is read on its own first (see
/// `read_library_file`), so it is not repeated here.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LibraryFile {
    #[serde(default)]
    papers: Vec<PaperRecord>,
    #[serde(default)]
    projects: Vec<ProjectRecord>,
    /// How BibTeX is written (projects.md sec. 7). Absent before there were
    /// options: the defaults, which write what was written then.
    #[serde(default)]
    bibtex_options: BibtexOptions,
    /// Triage and dismissals (`review`). Absent before it existed.
    #[serde(default)]
    review: Option<ReviewState>,
    /// Authors as persons (`persons`), with a version of its own.
    #[serde(default)]
    persons: PersonsSection,
}

/// The same shape, borrowed, so a save never clones every record.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryFileRef<'a> {
    version: u32,
    papers: &'a [PaperRecord],
    projects: &'a [ProjectRecord],
    bibtex_options: &'a BibtexOptions,
    review: &'a ReviewState,
    persons: &'a PersonsSection,
}

#[derive(Serialize, Deserialize)]
struct DataDirPointer {
    dir: PathBuf,
}

enum Loaded {
    Missing,
    Ok(LibraryFile),
    Newer(u32),
    Corrupt(String),
    Unreadable(String),
}

pub struct PaperStore {
    app_data_dir: PathBuf,
    data_dir: PathBuf,
    pub(super) papers: Vec<PaperRecord>,
    pub(super) projects: Vec<ProjectRecord>,
    pub(super) bibtex_options: BibtexOptions,
    pub(super) review: ReviewState,
    /// Authors clustered into persons; refreshed before every save.
    pub(super) persons: PersonsSection,
    /// The last backup a save wrote (`backup`).
    backup: BackupState,
    /// Why nothing may be written, when the file on disk is one this build
    /// must not replace: written by a newer build, or unreadable and not
    /// movable out of the way. Nothing was loaded either.
    write_block: Option<String>,
    /// Unsaved reconciles from a scan. Commands save on the spot.
    dirty: bool,
    /// Synced `.bib` files the last save could not write, by project id, so
    /// the command that set the path can tell the user instead of the log.
    bib_errors: Vec<(String, String)>,
    last_flush: Instant,
}

impl PaperStore {
    pub fn load(app_data_dir: PathBuf) -> Self {
        if let Err(error) = fs::create_dir_all(&app_data_dir) {
            log::warn!("cannot create app-data directory for the paper library: {error}");
        }
        let app_data_dir = app_data_dir.canonicalize().unwrap_or(app_data_dir);
        let (data_dir, unavailable) = match read_pointer(&app_data_dir) {
            Pointer::Default => (app_data_dir.clone(), false),
            Pointer::Dir(dir) => (dir, false),
            Pointer::Unavailable(dir) => (dir, true),
        };
        let mut store = Self {
            app_data_dir,
            data_dir,
            papers: Vec::new(),
            projects: Vec::new(),
            bibtex_options: BibtexOptions::default(),
            review: ReviewState::starting_now(),
            persons: PersonsSection::default(),
            backup: BackupState::default(),
            write_block: None,
            dirty: false,
            bib_errors: Vec::new(),
            last_flush: Instant::now(),
        };
        if unavailable {
            // Falling back to the copy left in app data would quietly fork
            // the library: edits made here would be stranded the moment the
            // real folder came back. Read-only, and say so, until the folder
            // returns or the user points somewhere else.
            log::warn!(
                "library data folder {} is not available; papers and projects are read-only",
                store.data_dir.display()
            );
            store.write_block = Some(format!(
                "the library data folder {} is not available (an unmounted drive, or a sync folder not set up here); papers and projects are read-only until it is, or until you choose another folder in Settings",
                store.data_dir.display()
            ));
            return store;
        }
        let path = store.file_path();
        match read_library_file(&path) {
            Loaded::Missing => {}
            Loaded::Ok(file) => {
                store.papers = file.papers;
                store.projects = file.projects;
                store.bibtex_options = file.bibtex_options;
                if let Some(review) = file.review.filter(|r| r.since.is_some()) {
                    store.review = review;
                }
                store.persons = file.persons.loaded();
                // Duplicates from before automatic merging, or from another
                // machine's edits, are merged once here; then every item's
                // main file is chosen again and the items gone for a week
                // are removed.
                let merged = store.auto_merge_all();
                let swept = store.load_sweep(now_unix_secs());
                if merged || swept {
                    if let Err(error) = store.save() {
                        log::warn!("cannot save the library after merging duplicates: {error}");
                        store.mark_dirty();
                    }
                }
            }
            Loaded::Newer(version) => {
                log::warn!(
                    "{} is version {version}; this build understands {FORMAT_VERSION}. Papers and projects are read-only until the app is updated.",
                    path.display()
                );
                store.write_block = Some(format!(
                    "library.json was written by a newer SpeedyF (version {version}); update the app before changing papers or projects"
                ));
            }
            Loaded::Corrupt(error) => {
                // Set the file aside rather than write over it: whatever is
                // in there may still be recoverable by hand, and a fresh
                // empty file is not worth losing it for.
                let aside =
                    path.with_file_name(format!("{LIBRARY_FILE}.corrupt-{}", now_unix_ms()));
                match fs::rename(&path, &aside) {
                    Ok(()) => log::warn!(
                        "cannot read {}: {error}; moved it to {} and starting empty",
                        path.display(),
                        aside.display()
                    ),
                    Err(rename_error) => {
                        log::warn!(
                            "cannot read {}: {error}; and cannot move it aside: {rename_error}",
                            path.display()
                        );
                        store.write_block = Some(format!(
                            "library.json cannot be read ({error}) and could not be moved aside ({rename_error})"
                        ));
                    }
                }
            }
            Loaded::Unreadable(error) => {
                log::warn!("cannot read {}: {error}", path.display());
                store.write_block = Some(format!("library.json cannot be read: {error}"));
            }
        }
        store.refresh_persons();
        store
    }

    fn file_path(&self) -> PathBuf {
        self.data_dir.join(LIBRARY_FILE)
    }

    pub fn ensure_writable(&self) -> AppResult<()> {
        match &self.write_block {
            Some(reason) => Err(AppError::Unsupported(reason.clone())),
            None => Ok(()),
        }
    }

    pub fn writable(&self) -> bool {
        self.write_block.is_none()
    }

    #[cfg(test)]
    pub(super) fn block_writes_for_tests(&mut self) {
        self.write_block = Some("read-only for a test".into());
    }

    /// Something changed in memory that is not on disk yet. Scans call this
    /// through `reconcile_file`, and a lookup batch after each paper;
    /// `flush` decides when it reaches the disk.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Write the whole library now. Atomic: a temp file beside the real one,
    /// then a rename over it, so a crash leaves either the old file or the
    /// new one and never a torn one.
    pub fn save(&mut self) -> AppResult<()> {
        self.save_with(true)
    }

    /// `sync_bibs` is false only for the throttled flush a scan makes from
    /// the engine thread: reading and comparing a `.bib` on a slow or absent
    /// volume there would stall rendering once a second. The scan's final
    /// flush and every command save sync, so the files never lag for long.
    fn save_with(&mut self, sync_bibs: bool) -> AppResult<()> {
        self.ensure_writable()?;
        // Every write passes through here, so this is where the synced
        // `.bib` files follow the records. Before the serialize: writing one
        // pins keys, and the file must hold what the records hold.
        if sync_bibs {
            self.bib_errors = self
                .sync_project_bibs()
                .into_iter()
                .map(|(project_id, error)| {
                    log::warn!("cannot write the synced .bib of project {project_id}: {error}");
                    (project_id, error.to_string())
                })
                .collect();
        }
        self.refresh_persons();
        let path = self.file_path();
        let bytes = self.serialized()?;
        write_atomically(&path, &bytes)?;
        self.dirty = false;
        self.last_flush = Instant::now();
        // A backup is a convenience: one that cannot be written never fails
        // the save that asked for it.
        let dir = backup::dir_of(&self.data_dir);
        if let Err(error) = self
            .backup
            .maybe_write(&dir, &bytes, now_unix_secs(), false)
        {
            log::warn!("cannot back up the library: {error}");
        }
        Ok(())
    }

    /// `library.json` as a save writes it.
    fn serialized(&self) -> AppResult<Vec<u8>> {
        serde_json::to_vec_pretty(&LibraryFileRef {
            version: FORMAT_VERSION,
            papers: &self.papers,
            projects: &self.projects,
            bibtex_options: &self.bibtex_options,
            review: &self.review,
            persons: &self.persons,
        })
        .map_err(|error| AppError::Internal(format!("cannot serialize the library: {error}")))
    }

    pub fn bibtex_options(&self) -> BibtexOptions {
        self.bibtex_options.clone()
    }

    /// Change how BibTeX is written. The caller saves, which rewrites every
    /// synced `.bib` whose bytes the change alters.
    pub fn set_bibtex_options(&mut self, options: BibtexOptions) -> AppResult<BibtexOptions> {
        self.ensure_writable()?;
        if self.bibtex_options != options {
            self.bibtex_options = options;
            self.mark_dirty();
        }
        Ok(self.bibtex_options.clone())
    }

    /// The backups of `library.json`, newest first, with what each holds.
    /// One that cannot be read is left out: it could not be restored.
    pub fn backups(&self) -> Vec<LibraryBackupDto> {
        let dir = backup::dir_of(&self.data_dir);
        backup::backups_in(&dir)
            .into_iter()
            .filter_map(|(file, at)| {
                let path = dir.join(&file);
                let bytes = fs::metadata(&path).ok()?.len();
                let content = backup::read(&path).ok()?;
                let counts = serde_json::from_slice::<BackupCounts>(&content).ok()?;
                let cap = |n: usize| n.min(u32::MAX as usize) as u32;
                Some(LibraryBackupDto {
                    file,
                    at: format_rfc3339(at),
                    bytes,
                    papers: cap(counts.papers.len()),
                    projects: cap(counts.projects.len()),
                })
            })
            .collect()
    }

    /// Put the library back as the backup `file` holds it. Refused for a
    /// name that is not a backup's, a backup that does not parse, and one
    /// written by a newer build. The current state is backed up first, so
    /// a restore can itself be undone; then the records are replaced in
    /// memory and the caller's save writes them to disk.
    pub fn restore_backup(&mut self, file: &str) -> AppResult<()> {
        self.ensure_writable()?;
        if backup::parse_name(file).is_none() {
            return Err(AppError::Internal(format!(
                "{file} is not a library backup"
            )));
        }
        let dir = backup::dir_of(&self.data_dir);
        let bytes = backup::read(&dir.join(file))?;
        let restored = match read_library_bytes(&bytes) {
            Loaded::Ok(restored) => restored,
            Loaded::Newer(version) => {
                return Err(AppError::Unsupported(format!(
                    "that backup was written by a newer SpeedyF (version {version}); update the app to restore it"
                )));
            }
            Loaded::Missing | Loaded::Corrupt(_) | Loaded::Unreadable(_) => {
                return Err(AppError::Io(format!("the backup {file} cannot be read")));
            }
        };
        let current = self.serialized()?;
        self.backup
            .maybe_write(&dir, &current, now_unix_secs(), true)?;
        self.papers = restored.papers;
        self.projects = restored.projects;
        self.bibtex_options = restored.bibtex_options;
        if let Some(review) = restored.review.filter(|r| r.since.is_some()) {
            self.review = review;
        }
        self.persons = restored.persons.loaded();
        self.mark_dirty();
        Ok(())
    }

    /// Save if there is anything unsaved and either `force` or the last save
    /// is more than `FLUSH_INTERVAL` ago. Returns whether a save happened, so
    /// the caller can tell the windows.
    pub fn flush(&mut self, force: bool) -> AppResult<bool> {
        if !self.dirty || !self.writable() {
            return Ok(false);
        }
        if !force && self.last_flush.elapsed() < FLUSH_INTERVAL {
            return Ok(false);
        }
        self.save_with(force)?;
        Ok(true)
    }

    /// Why the project's synced `.bib` was not written by the last save.
    pub fn bib_error_for(&self, project_id: &str) -> Option<String> {
        self.bib_errors
            .iter()
            .find(|(id, _)| id == project_id)
            .map(|(_, error)| error.clone())
    }

    pub fn data_info(&self) -> LibraryDataInfoDto {
        LibraryDataInfoDto {
            dir: self.data_dir.to_string_lossy().into_owned(),
            is_default: self.data_dir == self.app_data_dir,
        }
    }

    /// Point the library at another folder. A folder that already holds a
    /// `library.json` is adopted as it is - that is how a second machine joins
    /// a synced folder - and nothing is copied over it. An empty folder gets a
    /// copy of the current file. The pointer is written last, so a failure
    /// part-way leaves the old location in force.
    pub fn move_data(&mut self, dir: PathBuf) -> AppResult<LibraryDataInfoDto> {
        fs::create_dir_all(&dir)
            .map_err(|error| AppError::Io(format!("cannot create {}: {error}", dir.display())))?;
        let dir = dir
            .canonicalize()
            .map_err(|error| AppError::Io(format!("cannot open {}: {error}", dir.display())))?;
        if dir == self.data_dir {
            return Ok(self.data_info());
        }
        let target = dir.join(LIBRARY_FILE);
        // Nothing in memory changes until the pointer is on disk: a failure
        // part-way must leave the old location in force *and* the old
        // records in memory, or the next save would write the adopted file's
        // contents over the old one.
        let mut adopted = None;
        match read_library_file(&target) {
            Loaded::Ok(file) => {
                adopted = Some((file.papers, file.projects, file.bibtex_options));
            }
            Loaded::Missing => {
                let current = self.file_path();
                if current.is_file() {
                    // Bytes, not a re-serialization: if the current file is
                    // one this build refused to read, the copy must be just
                    // as refusable.
                    fs::copy(&current, &target).map_err(|error| {
                        AppError::Io(format!(
                            "cannot copy library.json to {}: {error}",
                            dir.display()
                        ))
                    })?;
                } else {
                    // Nothing to copy: the old folder is unavailable, or
                    // nothing was ever saved. What is in memory is then
                    // empty or this session's own work, and writing it to a
                    // fresh folder destroys nothing.
                    write_atomically(&target, &self.serialized()?)?;
                }
            }
            Loaded::Newer(version) => {
                return Err(AppError::Unsupported(format!(
                    "the library.json in that folder was written by a newer SpeedyF (version {version}); update the app before switching to it"
                )));
            }
            Loaded::Corrupt(error) | Loaded::Unreadable(error) => {
                return Err(AppError::Io(format!(
                    "the library.json in that folder cannot be read: {error}"
                )));
            }
        }
        write_pointer(&self.app_data_dir, &dir)?;
        self.data_dir = dir;
        self.write_block = None;
        self.backup.reset();
        if let Some((papers, projects, bibtex_options)) = adopted {
            self.papers = papers;
            self.projects = projects;
            self.bibtex_options = bibtex_options;
            self.dirty = false;
        }
        Ok(self.data_info())
    }
}

/// How many records a backup holds, without parsing them.
#[derive(Deserialize)]
struct BackupCounts {
    #[serde(default)]
    papers: Vec<serde::de::IgnoredAny>,
    #[serde(default)]
    projects: Vec<serde::de::IgnoredAny>,
}

fn read_library_file(path: &Path) -> Loaded {
    match fs::read(path) {
        Ok(bytes) => read_library_bytes(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Loaded::Missing,
        Err(error) => Loaded::Unreadable(error.to_string()),
    }
}

fn read_library_bytes(bytes: &[u8]) -> Loaded {
    // The version first, on its own, so a file whose records this build
    // cannot parse is still recognized as newer rather than as corrupt.
    #[derive(Deserialize)]
    struct Envelope {
        #[serde(default)]
        version: u32,
    }
    match serde_json::from_slice::<Envelope>(bytes) {
        Ok(envelope) if envelope.version > FORMAT_VERSION => {
            return Loaded::Newer(envelope.version);
        }
        Ok(_) => {}
        Err(error) => return Loaded::Corrupt(error.to_string()),
    }
    match serde_json::from_slice::<LibraryFile>(bytes) {
        Ok(file) => Loaded::Ok(file),
        Err(error) => Loaded::Corrupt(error.to_string()),
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let dir = path
        .parent()
        .ok_or_else(|| AppError::Io(format!("{} has no parent folder", path.display())))?;
    fs::create_dir_all(dir)
        .map_err(|error| AppError::Io(format!("cannot create {}: {error}", dir.display())))?;
    let mut temp = tempfile::Builder::new()
        .prefix(LIBRARY_FILE)
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

/// Where the pointer file says the data lives.
enum Pointer {
    /// No pointer: app data.
    Default,
    Dir(PathBuf),
    /// Pointed at a folder that is not there right now: an unmounted drive,
    /// a sync folder not yet set up on this machine.
    Unavailable(PathBuf),
}

fn read_pointer(app_data_dir: &Path) -> Pointer {
    let Ok(bytes) = fs::read(app_data_dir.join(POINTER_FILE)) else {
        return Pointer::Default;
    };
    let pointer: DataDirPointer = match serde_json::from_slice(&bytes) {
        Ok(pointer) => pointer,
        Err(error) => {
            log::warn!("ignoring unreadable {POINTER_FILE}: {error}");
            return Pointer::Default;
        }
    };
    match pointer.dir.canonicalize() {
        Ok(dir) if dir.is_dir() => Pointer::Dir(dir),
        _ => Pointer::Unavailable(pointer.dir),
    }
}

fn write_pointer(app_data_dir: &Path, data_dir: &Path) -> AppResult<()> {
    let path = app_data_dir.join(POINTER_FILE);
    if data_dir == app_data_dir {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::Io(format!(
                "cannot remove {}: {error}",
                path.display()
            ))),
        };
    }
    let bytes = serde_json::to_vec_pretty(&DataDirPointer {
        dir: data_dir.to_path_buf(),
    })
    .map_err(|error| AppError::Internal(format!("cannot serialize {POINTER_FILE}: {error}")))?;
    fs::write(&path, bytes)
        .map_err(|error| AppError::Io(format!("cannot write {}: {error}", path.display())))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// The current time as `2026-11-11T20:00:00Z`.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
}

/// RFC 3339 in UTC, whole seconds. Written by hand because this is the only
/// place the app formats a date, and a date crate for one format is a bad
/// trade.
pub fn format_rfc3339(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let seconds_of_day = unix_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

/// The inverse of `format_rfc3339`, for what it writes
/// (`YYYY-MM-DDTHH:MM:SSZ`): unix seconds, or `None` for anything else.
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    let (date, time) = text.trim().split_once('T')?;
    let time = time.strip_suffix('Z')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let day: u32 = date.next()?.parse().ok()?;
    let mut time = time.split(':');
    let hour: u64 = time.next()?.parse().ok()?;
    let minute: u64 = time.next()?.parse().ok()?;
    let second: u64 = time.next()?.parse().ok()?;
    if date.next().is_some()
        || time.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = u64::try_from(super::ratelimit::days_from_civil(year, month, day)).ok()?;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
/// Howard Hinnant's algorithm: it works in 400-year eras and in years that
/// start in March, so leap days land at the end of the year and the month
/// lengths become a single linear formula.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::types::{MetaField, MetaSource};

    fn store_with_one_paper(app_data: &Path) -> PaperStore {
        let mut store = PaperStore::load(app_data.to_path_buf());
        let mut paper = PaperRecord::new();
        paper.meta.title = Some(MetaField {
            value: "A Title".into(),
            source: MetaSource::User,
        });
        paper.cite_key = "anonndtitle".into();
        store.papers.push(paper);
        store.create_project("Thesis").expect("create project");
        store
    }

    #[test]
    fn an_unavailable_data_folder_is_read_only_until_another_is_chosen() {
        let app_data = tempfile::tempdir().expect("app data");
        // The pointer names a folder that is not there (an unmounted drive).
        let gone = app_data.path().join("not-mounted");
        write_pointer(app_data.path(), &gone).expect("pointer");
        // A stale copy in app data must not be adopted behind the user's back.
        fs::write(
            app_data.path().join(LIBRARY_FILE),
            serde_json::json!({ "version": FORMAT_VERSION, "papers": [{
                "id": "stale", "files": [], "ids": {}, "meta": {},
                "citeKey": "stale", "citeKeyPinned": false,
                "addedAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z"
            }], "projects": [] })
            .to_string(),
        )
        .unwrap();

        let mut store = PaperStore::load(app_data.path().to_path_buf());
        assert!(
            store.papers.is_empty(),
            "nothing is adopted from the stale copy"
        );
        assert!(!store.writable());
        assert!(store.create_project("Thesis").is_err());
        assert!(store.data_info().dir.ends_with("not-mounted"));

        // Choosing another folder is the way out, and works while blocked.
        let fresh = tempfile::tempdir().expect("fresh");
        let info = store.move_data(fresh.path().to_path_buf()).expect("move");
        assert!(!info.is_default);
        assert!(store.writable());
        store.create_project("Thesis").expect("writes again");
        assert!(fresh.path().join(LIBRARY_FILE).is_file());
        // The stale copy in app data is untouched.
        let stale = fs::read_to_string(app_data.path().join(LIBRARY_FILE)).unwrap();
        assert!(stale.contains("\"stale\""));
    }

    #[test]
    fn formats_rfc3339_in_utc() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_rfc3339(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_rfc3339(1_699_000_000), "2023-11-03T08:26:40Z");
        assert_eq!(format_rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(-719_468), (0, 3, 1));
        let now = now_rfc3339();
        assert!(now.ends_with('Z') && now.len() == 20, "{now}");
    }

    #[test]
    fn round_trips_through_library_json() {
        let app_data = tempfile::tempdir().expect("app data");
        let mut store = store_with_one_paper(app_data.path());
        let paper_id = store.papers[0].id.clone();
        let project_id = store.projects[0].id.clone();
        store
            .set_membership(&paper_id, &project_id, true)
            .expect("membership");
        store.save().expect("save");
        assert!(app_data.path().join(LIBRARY_FILE).is_file());
        assert!(
            !fs::read_dir(app_data.path())
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")),
            "the temp file must be gone after persist"
        );

        let reloaded = PaperStore::load(app_data.path().to_path_buf());
        assert_eq!(reloaded.papers.len(), 1);
        assert_eq!(reloaded.papers[0].id, paper_id);
        assert_eq!(
            reloaded.papers[0]
                .meta
                .title
                .as_ref()
                .map(|t| t.value.as_str()),
            Some("A Title")
        );
        assert_eq!(reloaded.projects.len(), 1);
        assert_eq!(reloaded.projects[0].paper_ids, vec![paper_id]);

        // The on-disk field names are the ones projects.md sec. 3 fixes.
        let text = fs::read_to_string(app_data.path().join(LIBRARY_FILE)).unwrap();
        for key in [
            "\"version\": 1",
            "\"citeKey\"",
            "\"citeKeyPinned\"",
            "\"addedAt\"",
            "\"paperIds\"",
            "\"bibPath\"",
            "\"ids\"",
        ] {
            assert!(text.contains(key), "missing {key} in {text}");
        }
    }

    #[test]
    fn a_library_written_before_verdicts_and_candidates_still_loads() {
        let app_data = tempfile::tempdir().expect("app data");
        fs::write(
            app_data.path().join(LIBRARY_FILE),
            serde_json::json!({ "version": FORMAT_VERSION, "papers": [{
                "id": "old", "files": [], "ids": { "doi": "10.1/x" },
                "meta": { "title": { "value": "Old", "source": "pdf-info" } },
                "citeKey": "anonndold", "citeKeyPinned": false,
                "addedAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z"
            }], "projects": [] })
            .to_string(),
        )
        .unwrap();
        let mut store = PaperStore::load(app_data.path().to_path_buf());
        assert_eq!(store.papers.len(), 1);
        let paper = &store.papers[0];
        assert_eq!(paper.verdict, None);
        assert_eq!(paper.verdict_reason, "");
        assert_eq!(paper.paper_override, None);
        assert!(paper.candidates.is_empty());
        assert!(paper.ids.isbn.is_none());
        assert_eq!(paper.ids.doi.as_deref(), Some("10.1/x"));
        assert_eq!(
            paper.ids.doi_source, None,
            "written before sources were kept: the scan's to correct"
        );
        assert_eq!(paper.looked_up_at, None);
        assert!(paper.declined.is_empty());
        assert!(paper.is_paper(), "benefit of the doubt");
        assert_eq!(store.counts().others, 0);

        // Once written, the new fields round-trip.
        store.papers[0].verdict = Some(crate::library::types::PapernessVerdict::Unknown);
        store.papers[0].paper_override = Some(true);
        store.papers[0].ids.isbn = Some("9780306406157".into());
        store.papers[0].ids.doi_source = Some(MetaSource::Crossref);
        store.papers[0].ids.isbn_source = Some(MetaSource::User);
        store.papers[0].looked_up_at = Some("2026-02-03T04:05:06Z".into());
        store.papers[0].declined = vec!["doi:10.1/no".into()];
        store.save().unwrap();
        let text = fs::read_to_string(app_data.path().join(LIBRARY_FILE)).unwrap();
        assert!(text.contains("\"paperOverride\": true"), "{text}");
        assert!(text.contains("\"doiSource\": \"crossref\""), "{text}");
        assert!(
            text.contains("\"lookedUpAt\": \"2026-02-03T04:05:06Z\""),
            "{text}"
        );
        let reloaded = PaperStore::load(app_data.path().to_path_buf());
        let paper = &reloaded.papers[0];
        assert_eq!(
            paper.verdict,
            Some(crate::library::types::PapernessVerdict::Unknown)
        );
        assert_eq!(paper.paper_override, Some(true));
        assert_eq!(paper.ids.isbn.as_deref(), Some("9780306406157"));
        assert_eq!(paper.ids.doi_source, Some(MetaSource::Crossref));
        assert_eq!(paper.ids.arxiv_source, None);
        assert_eq!(paper.ids.isbn_source, Some(MetaSource::User));
        assert_eq!(paper.looked_up_at.as_deref(), Some("2026-02-03T04:05:06Z"));
        assert_eq!(paper.declined, vec!["doi:10.1/no".to_string()]);
        assert!(paper.is_paper());
    }

    #[test]
    fn a_corrupt_file_is_set_aside_not_overwritten() {
        let app_data = tempfile::tempdir().expect("app data");
        let path = app_data.path().join(LIBRARY_FILE);
        fs::write(&path, b"{not json").unwrap();

        let mut store = PaperStore::load(app_data.path().to_path_buf());
        assert!(store.papers.is_empty());
        assert!(store.writable());
        store.save().expect("a fresh file can be written");

        let aside: Vec<_> = fs::read_dir(app_data.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("library.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(
            fs::read(app_data.path().join(&aside[0])).unwrap(),
            b"{not json"
        );
    }

    #[test]
    fn a_newer_version_is_refused_and_never_written() {
        let app_data = tempfile::tempdir().expect("app data");
        let path = app_data.path().join(LIBRARY_FILE);
        let newer =
            serde_json::json!({ "version": 99, "papers": [{ "future": true }] }).to_string();
        fs::write(&path, &newer).unwrap();

        let mut store = PaperStore::load(app_data.path().to_path_buf());
        assert!(store.papers.is_empty());
        assert!(!store.writable());
        let error = store.save().expect_err("must refuse");
        assert!(error.to_string().contains("newer"), "{error}");
        assert!(store.create_project("x").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), newer);
    }

    #[test]
    fn moving_data_copies_then_adopts() {
        let app_data = tempfile::tempdir().expect("app data");
        let elsewhere = tempfile::tempdir().expect("elsewhere");
        let mut store = store_with_one_paper(app_data.path());
        store.save().expect("save");
        assert!(store.data_info().is_default);

        let info = store
            .move_data(elsewhere.path().join("sync"))
            .expect("move");
        assert!(!info.is_default);
        assert!(Path::new(&info.dir).join(LIBRARY_FILE).is_file());
        assert!(app_data.path().join(POINTER_FILE).is_file());

        // A fresh load follows the pointer.
        let followed = PaperStore::load(app_data.path().to_path_buf());
        assert_eq!(followed.data_info().dir, info.dir);
        assert_eq!(followed.papers.len(), 1);

        // Pointing at a folder that already has a library adopts it, keeping
        // that folder's contents rather than copying ours over them.
        let other = tempfile::tempdir().expect("other");
        let mut other_store = PaperStore::load(other.path().to_path_buf());
        other_store.create_project("Theirs").unwrap();
        other_store.create_project("Also theirs").unwrap();
        other_store.save().unwrap();
        let mut store = followed;
        store.move_data(other.path().to_path_buf()).expect("adopt");
        assert_eq!(store.projects.len(), 2);
        assert!(store.papers.is_empty());

        // Back to the default removes the pointer.
        store
            .move_data(app_data.path().to_path_buf())
            .expect("back");
        assert!(store.data_info().is_default);
        assert!(!app_data.path().join(POINTER_FILE).exists());
    }

    #[test]
    fn throttled_flushes_save_once_an_interval_and_the_forced_one_the_rest() {
        let app_data = tempfile::tempdir().expect("app data");
        let mut store = store_with_one_paper(app_data.path());
        let turns = 200;
        let started = Instant::now();
        let mut saves = 0;
        for _ in 0..turns {
            store.mark_dirty();
            if store.flush(false).unwrap() {
                saves += 1;
            }
        }
        let allowed =
            (started.elapsed().as_secs_f64() / FLUSH_INTERVAL.as_secs_f64()).ceil() as u32;
        assert!(
            saves <= allowed + 1,
            "{saves} saves for {turns} turns in {:?}",
            started.elapsed()
        );
        assert!(
            store.flush(true).unwrap(),
            "the forced flush writes the rest"
        );
        assert!(!store.flush(true).unwrap(), "nothing left");
        assert!(app_data.path().join(LIBRARY_FILE).is_file());
    }

    #[test]
    fn rfc3339_round_trips_and_rejects_other_shapes() {
        for secs in [0, 951_782_400, 1_700_000_000, 4_102_444_799] {
            assert_eq!(parse_rfc3339(&format_rfc3339(secs)), Some(secs));
        }
        assert_eq!(parse_rfc3339("2026-01-01T00:00:00Z"), Some(1_767_225_600));
        assert_eq!(
            parse_rfc3339(" 2026-01-01T00:00:00Z\n"),
            Some(1_767_225_600)
        );
        for bad in [
            "",
            "2026-01-01",
            "2026-01-01T00:00:00",
            "2026-01-01T00:00:00+01:00",
            "2026-13-01T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:00:00:00Z",
            "1969-12-31T23:59:59Z",
            "yesterday",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad:?}");
        }
    }

    fn backup_names(data: &Path) -> Vec<String> {
        backup::backups_in(&backup::dir_of(data))
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[test]
    fn a_save_backs_up_and_a_backup_restores_after_backing_up_the_present() {
        let app_data = tempfile::tempdir().expect("app data");
        let mut store = store_with_one_paper(app_data.path());
        store.save().expect("save");
        let first = backup_names(app_data.path());
        assert_eq!(first.len(), 1, "the first save backs up");
        // Within the hour, changed: no second backup.
        store.create_project("Second").unwrap();
        store.save().unwrap();
        assert_eq!(backup_names(app_data.path()), first);

        let listed = store.backups();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].papers, listed[0].projects), (1, 1));
        assert_eq!(listed[0].file, first[0]);
        assert!(listed[0].bytes > 0 && listed[0].at.ends_with('Z'));

        // Restore: the present (two projects) is backed up first, then the
        // backup's one project is what memory and disk hold.
        store.restore_backup(&first[0]).expect("restore");
        store.save().unwrap();
        assert_eq!(store.projects.len(), 1);
        let after = store.backups();
        assert_eq!(after.len(), 2, "{after:?}");
        assert_eq!(after[0].projects, 2, "the state before the restore");
        let reloaded = PaperStore::load(app_data.path().to_path_buf());
        assert_eq!(reloaded.projects.len(), 1);
        assert_eq!(reloaded.papers.len(), 1);
        // And that backup restores the two projects back.
        store.restore_backup(&after[0].file).unwrap();
        store.save().unwrap();
        assert_eq!(store.projects.len(), 2);
    }

    #[test]
    fn a_corrupt_newer_or_foreign_backup_is_refused_and_nothing_changes() {
        let app_data = tempfile::tempdir().expect("app data");
        let mut store = store_with_one_paper(app_data.path());
        store.save().unwrap();
        let dir = backup::dir_of(app_data.path());
        let now = 1_700_000_000;
        let corrupt = backup::write(&dir, now, b"{not json").unwrap();
        let newer = backup::write(
            &dir,
            now + 10,
            serde_json::json!({ "version": 99, "papers": [] })
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        fs::write(dir.join("library-20260101-000000.json.gz"), b"not gzip").unwrap();
        let before = fs::read(app_data.path().join(LIBRARY_FILE)).unwrap();
        let count = backup_names(app_data.path()).len();

        let error = store.restore_backup(&newer).unwrap_err();
        assert!(error.to_string().contains("newer"), "{error}");
        assert!(store.restore_backup(&corrupt).is_err());
        assert!(store
            .restore_backup("library-20260101-000000.json.gz")
            .is_err());
        assert!(store.restore_backup("../library.json").is_err());
        assert!(store
            .restore_backup("library-20250101-000000.json.gz")
            .is_err());
        assert_eq!(store.papers.len(), 1);
        assert_eq!(
            fs::read(app_data.path().join(LIBRARY_FILE)).unwrap(),
            before
        );
        assert_eq!(
            backup_names(app_data.path()).len(),
            count,
            "no backup of a refusal"
        );
        // The unreadable ones are not offered.
        let offered: Vec<String> = store.backups().into_iter().map(|b| b.file).collect();
        assert!(!offered.contains(&corrupt));
        assert!(
            offered.contains(&newer),
            "listed; restoring it says why not"
        );
        assert!(!offered
            .iter()
            .any(|f| f == "library-20260101-000000.json.gz"));
    }

    #[test]
    fn bibtex_options_persist_and_rewrite_the_synced_bib() {
        let app_data = tempfile::tempdir().expect("app data");
        let mut store = store_with_one_paper(app_data.path());
        let paper_id = store.papers[0].id.clone();
        let project_id = store.projects[0].id.clone();
        store.set_membership(&paper_id, &project_id, true).unwrap();
        let bib = app_data.path().join("refs.bib");
        store
            .set_project_bib(&project_id, Some(bib.to_string_lossy().into_owned()))
            .unwrap();
        store.save().unwrap();
        assert!(fs::read_to_string(&bib)
            .unwrap()
            .contains("title = {{A Title}}"));

        let options = BibtexOptions {
            protect_title: false,
            ..BibtexOptions::default()
        };
        store.set_bibtex_options(options.clone()).unwrap();
        store.save().unwrap();
        assert!(fs::read_to_string(&bib)
            .unwrap()
            .contains("title = {A Title}"));
        let text = fs::read_to_string(app_data.path().join(LIBRARY_FILE)).unwrap();
        assert!(text.contains("\"bibtexOptions\""), "{text}");
        assert!(text.contains("\"protectTitle\": false"), "{text}");
        let reloaded = PaperStore::load(app_data.path().to_path_buf());
        assert_eq!(reloaded.bibtex_options(), options);

        // A file from before there were options loads with the defaults.
        let old = tempfile::tempdir().unwrap();
        fs::write(
            old.path().join(LIBRARY_FILE),
            serde_json::json!({ "version": FORMAT_VERSION, "papers": [], "projects": [] })
                .to_string(),
        )
        .unwrap();
        let store = PaperStore::load(old.path().to_path_buf());
        assert_eq!(store.bibtex_options(), BibtexOptions::default());
    }
}

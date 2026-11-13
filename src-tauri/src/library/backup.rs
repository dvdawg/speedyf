//! Backups of `library.json` (projects.md sec. 3): gzip-compressed full
//! snapshots in `<library data dir>/backups/library-YYYYMMDD-HHMMSS.json.gz`
//! (UTC). Full rather than diffs so each one restores on its own; a library
//! of a thousand items compresses to a few tens of kilobytes.
//!
//! A save writes one at most once an hour, and only when the content
//! changed since the last one. Rotation keeps every backup from the last
//! day, the newest of each day for two weeks, and the newest of each week
//! for eight weeks; the rest are deleted.

use crate::errors::{AppError, AppResult};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const BACKUP_DIR: &str = "backups";
const PREFIX: &str = "library-";
const SUFFIX: &str = ".json.gz";
/// The least time between two backups a save writes.
pub const BACKUP_INTERVAL_SECS: u64 = 60 * 60;
const DAY: u64 = 24 * 60 * 60;
const KEEP_ALL_SECS: u64 = DAY;
const KEEP_DAILY_SECS: u64 = 14 * DAY;
const KEEP_WEEKLY_SECS: u64 = 8 * 7 * DAY;
/// More than any library.json, less than what would hurt to inflate: a
/// backup that inflates past it is not ours.
const MAX_INFLATED: u64 = 512 * 1024 * 1024;

/// One backup, for Settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryBackupDto {
    /// The file name inside the backups folder; what restore takes.
    pub file: String,
    /// When it was written (RFC 3339, UTC).
    pub at: String,
    /// Its size on disk, compressed.
    pub bytes: u64,
    pub papers: u32,
    pub projects: u32,
}

/// `library-20261112-093000.json.gz` for the moment `at` (unix seconds).
pub fn file_name(at: u64) -> String {
    let stamp = super::store::format_rfc3339(at);
    // `2026-11-12T09:30:00Z` -> `20261112-093000`
    let digits: String = stamp.chars().filter(char::is_ascii_digit).collect();
    format!("{PREFIX}{}-{}{SUFFIX}", &digits[..8], &digits[8..14])
}

/// The moment a backup's name says it was written, `None` for any other
/// name - which is also what keeps a restore inside the backups folder.
pub fn parse_name(name: &str) -> Option<u64> {
    let stamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (date, time) = stamp.split_once('-')?;
    if date.len() != 8
        || time.len() != 6
        || !date.chars().chain(time.chars()).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    super::store::parse_rfc3339(&format!(
        "{}-{}-{}T{}:{}:{}Z",
        &date[..4],
        &date[4..6],
        &date[6..],
        &time[..2],
        &time[2..4],
        &time[4..]
    ))
}

/// FNV-1a 64: stable across runs and builds, which a content comparison
/// against a file written last week needs.
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Write `bytes` compressed as the backup for `at`, a second later per
/// clash so an existing backup is never written over. Atomic, like
/// `library.json` itself. Returns the file name.
pub fn write(dir: &Path, at: u64, bytes: &[u8]) -> AppResult<String> {
    fs::create_dir_all(dir)
        .map_err(|error| AppError::Io(format!("cannot create {}: {error}", dir.display())))?;
    let mut at = at;
    while dir.join(file_name(at)).exists() {
        at += 1;
    }
    let name = file_name(at);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .map_err(|error| AppError::Internal(format!("cannot compress a backup: {error}")))?;
    let compressed = encoder
        .finish()
        .map_err(|error| AppError::Internal(format!("cannot compress a backup: {error}")))?;
    let mut temp = tempfile::Builder::new()
        .prefix(".library-backup")
        .suffix(".tmp")
        .tempfile_in(dir)
        .map_err(|error| AppError::Io(format!("cannot write in {}: {error}", dir.display())))?;
    temp.write_all(&compressed)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| AppError::Io(format!("cannot write a backup: {error}")))?;
    let path = dir.join(&name);
    temp.persist(&path).map_err(|error| {
        AppError::Io(format!("cannot write {}: {}", path.display(), error.error))
    })?;
    Ok(name)
}

/// A backup's `library.json` bytes.
pub fn read(path: &Path) -> AppResult<Vec<u8>> {
    let file = fs::File::open(path)
        .map_err(|error| AppError::Io(format!("cannot open {}: {error}", path.display())))?;
    let mut bytes = Vec::new();
    GzDecoder::new(file)
        .take(MAX_INFLATED)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::Io(format!("cannot read {}: {error}", path.display())))?;
    Ok(bytes)
}

/// Every backup in `dir` by name, with when it was written, newest first.
pub fn backups_in(dir: &Path) -> Vec<(String, u64)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            parse_name(&name).map(|at| (name, at))
        })
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    found
}

/// Which of the backups written at `times` rotation keeps at `now`: all
/// from the last day (and any from the future, a clock set back), the
/// newest of each UTC day up to two weeks back, the newest of each week up
/// to eight weeks back.
pub fn keep_set(times: &[u64], now: u64) -> HashSet<u64> {
    let mut keep = HashSet::new();
    let mut daily: HashMap<u64, u64> = HashMap::new();
    let mut weekly: HashMap<u64, u64> = HashMap::new();
    for &at in times {
        let age = now.saturating_sub(at);
        if at > now || age < KEEP_ALL_SECS {
            keep.insert(at);
        } else if age < KEEP_DAILY_SECS {
            let newest = daily.entry(at / DAY).or_insert(at);
            *newest = (*newest).max(at);
        } else if age < KEEP_WEEKLY_SECS {
            let newest = weekly.entry(at / (7 * DAY)).or_insert(at);
            *newest = (*newest).max(at);
        }
    }
    keep.extend(daily.into_values());
    keep.extend(weekly.into_values());
    keep
}

/// Delete the backups in `dir` that rotation does not keep at `now`.
pub fn rotate(dir: &Path, now: u64) {
    let backups = backups_in(dir);
    let times: Vec<u64> = backups.iter().map(|(_, at)| *at).collect();
    let keep = keep_set(&times, now);
    for (name, at) in backups {
        if !keep.contains(&at) {
            if let Err(error) = fs::remove_file(dir.join(&name)) {
                log::warn!("cannot remove the old library backup {name}: {error}");
            }
        }
    }
}

/// The backups folder of a library data folder.
pub fn dir_of(data_dir: &Path) -> PathBuf {
    data_dir.join(BACKUP_DIR)
}

/// What a save remembers about the last backup, so it neither writes one
/// more than hourly nor writes the same content twice.
#[derive(Default)]
pub struct BackupState {
    /// Read from the newest backup on disk at the first save.
    primed: bool,
    pub last_at: Option<u64>,
    last_hash: Option<u64>,
}

impl BackupState {
    fn prime(&mut self, dir: &Path) {
        if self.primed {
            return;
        }
        self.primed = true;
        if let Some((name, at)) = backups_in(dir).into_iter().next() {
            self.last_at = Some(at);
            self.last_hash = read(&dir.join(name)).ok().map(|bytes| content_hash(&bytes));
        }
    }

    /// Forget what was read: the data folder moved.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Write a backup of `bytes` into `dir` if the last one is an hour old
    /// (or `force`) and holds other content. Returns the file written.
    pub fn maybe_write(
        &mut self,
        dir: &Path,
        bytes: &[u8],
        now: u64,
        force: bool,
    ) -> AppResult<Option<String>> {
        self.prime(dir);
        let recent = self
            .last_at
            .is_some_and(|at| at <= now && now - at < BACKUP_INTERVAL_SECS);
        if recent && !force {
            return Ok(None);
        }
        let hash = content_hash(bytes);
        if self.last_hash == Some(hash) {
            return Ok(None);
        }
        let name = write(dir, now, bytes)?;
        self.last_at = Some(now);
        self.last_hash = Some(hash);
        rotate(dir, now);
        Ok(Some(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_nothing_else_parses() {
        let at = 1_794_476_400; // 2026-11-12T09:40:00Z
        let name = file_name(at);
        assert_eq!(name, "library-20261112-094000.json.gz");
        assert_eq!(parse_name(&name), Some(at));
        for bad in [
            "library.json",
            "library-20261112-094000.json",
            "library-2026111-094000.json.gz",
            "../library-20261112-094000.json.gz",
            "library-20261112-09400a.json.gz",
            "library-20261312-094000.json.gz",
        ] {
            assert_eq!(parse_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn rotation_keeps_the_last_day_then_dailies_then_weeklies() {
        // A backup every hour for 80 days, then rotate as the day ends.
        let now = 1_800_000_000 - 1_800_000_000 % DAY + DAY - 1;
        let hour = 60 * 60;
        let times: Vec<u64> = (0..80 * 24).map(|n| now - n * hour).collect();
        let keep = keep_set(&times, now);
        // Every one of the last 24 hours.
        for n in 0..24 {
            assert!(keep.contains(&(now - n * hour)), "hour {n}");
        }
        // One a day, the newest, for days 1..14.
        for day in 1..14 {
            let in_day: Vec<u64> = times
                .iter()
                .copied()
                .filter(|at| now - at >= DAY && at / DAY == now / DAY - day)
                .collect();
            let kept: Vec<u64> = in_day
                .iter()
                .copied()
                .filter(|at| keep.contains(at))
                .collect();
            assert_eq!(kept, vec![*in_day.iter().max().unwrap()], "day {day}");
        }
        // Nothing older than eight weeks, and at most one per week between.
        assert!(keep.iter().all(|at| now - at < KEEP_WEEKLY_SECS));
        let mut weeks: HashMap<u64, u32> = HashMap::new();
        for at in keep.iter().filter(|at| now - **at >= KEEP_DAILY_SECS) {
            *weeks.entry(at / (7 * DAY)).or_default() += 1;
        }
        assert!(weeks.values().all(|count| *count == 1), "{weeks:?}");
        assert!(weeks.len() >= 6, "{weeks:?}");
        assert_eq!(keep.len(), 24 + 13 + weeks.len());
        // Rotating again keeps the same set.
        let kept: Vec<u64> = keep.iter().copied().collect();
        assert_eq!(keep_set(&kept, now), keep);
    }

    #[test]
    fn rotation_deletes_on_disk_and_writes_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_800_000_000;
        let first = write(dir.path(), now - 30 * DAY, b"old").unwrap();
        let a = write(dir.path(), now, b"a").unwrap();
        let b = write(dir.path(), now, b"b").unwrap();
        assert_ne!(a, b, "a clash moves to the next second");
        assert_eq!(read(&dir.path().join(&a)).unwrap(), b"a");
        assert_eq!(read(&dir.path().join(&b)).unwrap(), b"b");
        write(dir.path(), now - 100 * DAY, b"ancient").unwrap();
        rotate(dir.path(), now);
        let names: Vec<String> = backups_in(dir.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec![b, a, first]);
    }

    #[test]
    fn a_save_backs_up_hourly_and_only_changed_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = BackupState::default();
        let now = 1_800_000_000;
        assert!(state
            .maybe_write(dir.path(), b"v1", now, false)
            .unwrap()
            .is_some());
        // Within the hour: nothing, changed or not.
        assert!(state
            .maybe_write(dir.path(), b"v2", now + 60, false)
            .unwrap()
            .is_none());
        // Later, unchanged: nothing.
        assert!(state
            .maybe_write(dir.path(), b"v1", now + 7200, false)
            .unwrap()
            .is_none());
        assert_eq!(backups_in(dir.path()).len(), 1);
        // Later, changed: one more.
        assert!(state
            .maybe_write(dir.path(), b"v2", now + 7200, false)
            .unwrap()
            .is_some());
        assert_eq!(backups_in(dir.path()).len(), 2);
        // A fresh state reads the newest backup, so a restart does not
        // write the same content again.
        let mut fresh = BackupState::default();
        assert!(fresh
            .maybe_write(dir.path(), b"v2", now + 14_400, false)
            .unwrap()
            .is_none());
        // Forced, within the hour, changed: written.
        assert!(fresh
            .maybe_write(dir.path(), b"v3", now + 14_400, true)
            .unwrap()
            .is_some());
        assert_eq!(backups_in(dir.path()).len(), 3);
    }
}

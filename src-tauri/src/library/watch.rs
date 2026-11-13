//! Live library folder watching (projects.md sec. 3).
//!
//! One background thread holds the platform's recommended watcher (FSEvents
//! on macOS, inotify on Linux, ReadDirectoryChangesW on Windows), recursive
//! on every library root. Raw events only name paths; what happened to a
//! path is read off the disk once it has gone quiet, so a create, a write,
//! a delete and either half of a rename all reduce to "look at this path
//! again". A path that is gone is handed over as removed (its paper file is
//! marked missing now), one that is a stable PDF as changed (scanned at idle
//! and reconciled, which reattaches a renamed file by its fingerprint).
//!
//! Every 10 minutes a stat-only pass compares the folders with the index,
//! for whatever the watcher missed: FSEvents coalescing, network volumes,
//! sleep, a root the watcher could not arm.

use super::{collect_pdfs, file_stamp};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long a path must have had no events before it is looked at.
pub const QUIET: Duration = Duration::from_millis(1500);
/// The gap between the two looks whose size and mtime must agree.
pub const RECHECK: Duration = Duration::from_millis(500);
/// The safety-net pass.
pub const RESCAN_EVERY: Duration = Duration::from_secs(10 * 60);
/// How often the roots are checked for a volume that went or came back.
pub const ARM_CHECK_EVERY: Duration = Duration::from_secs(60);
/// Retry delay for a safety-net pass that found a scan running.
const RESCAN_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug)]
pub struct WatchTiming {
    pub quiet: Duration,
    pub recheck: Duration,
    pub rescan_every: Duration,
    pub arm_check_every: Duration,
}

impl Default for WatchTiming {
    fn default() -> Self {
        Self {
            quiet: QUIET,
            recheck: RECHECK,
            rescan_every: RESCAN_EVERY,
            arm_check_every: ARM_CHECK_EVERY,
        }
    }
}

/// File names a browser writes while a download is still arriving.
const PARTIAL_SUFFIXES: &[&str] = &[
    ".crdownload",
    ".download",
    ".part",
    ".partial",
    ".opdownload",
    ".tmp",
];

fn is_partial_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PARTIAL_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// Whether the file name says PDF, in any case.
pub fn is_pdf_name(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

/// Whether a path below `root` should never enter the library: a hidden
/// file or a file in a hidden folder (which takes in `~/.Trash` and
/// `/Volumes/X/.Trashes` when a root holds them), or a download still in
/// progress (`x.pdf.crdownload`, Safari's `x.pdf.download/` bundle,
/// Firefox's `x.pdf.part`). Only the part below the root counts: a root
/// that itself lives in a hidden folder is still watched. A trash folder
/// anywhere in the path is ignored regardless.
pub fn ignored_below(root: &Path, path: &Path) -> bool {
    let trash = path.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            name == ".Trash" || name == ".Trashes" || name.starts_with(".Trash-")
        }
        _ => false,
    });
    if trash {
        return true;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            is_hidden_name(&name) || is_partial_name(&name)
        }
        _ => true,
    })
}

/// Whether the watcher should drop an event for `path`: outside every
/// root, inside one of `skip_dirs` (our own data folders), or ignored by
/// [`ignored_below`] for its root.
pub fn is_ignored(path: &Path, roots: &[PathBuf], skip_dirs: &[PathBuf]) -> bool {
    if skip_dirs.iter().any(|dir| path.starts_with(dir)) {
        return true;
    }
    match roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.as_os_str().len())
    {
        Some(root) => ignored_below(root, path),
        None => true,
    }
}

/// What a path is on disk when it is finally looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    File(u64, u64),
    Dir,
    Gone,
}

pub fn stat(path: &Path) -> Stat {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Stat::Dir,
        Ok(metadata) if metadata.is_file() => match file_stamp(path) {
            Some((mtime_ms, size)) => Stat::File(mtime_ms, size),
            None => Stat::Gone,
        },
        // A symlink or something stranger: the scan never follows those.
        Ok(_) => Stat::Dir,
        Err(_) => Stat::Gone,
    }
}

struct Pending {
    last_event: Instant,
    /// When it was last looked at once quiet, and what it was then.
    checked: Option<(Instant, Stat)>,
}

/// What a poll found settled.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ready {
    /// Stable PDFs: scan them.
    pub files: Vec<PathBuf>,
    /// Paths that are no longer there, file or folder: their files are gone.
    pub gone: Vec<PathBuf>,
    /// Folders that appeared or changed: the caller lists their PDFs and
    /// touches each, so they wait out their own quiet period.
    pub dirs: Vec<PathBuf>,
}

impl Ready {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.gone.is_empty() && self.dirs.is_empty()
    }
}

/// Coalesces raw events per path and releases a path once it has been
/// quiet for `quiet` and, for a file, its size and mtime agree across two
/// looks `recheck` apart. Pure: the clock and the disk are passed in.
pub struct Debouncer {
    quiet: Duration,
    recheck: Duration,
    pending: HashMap<PathBuf, Pending>,
}

impl Debouncer {
    pub fn new(quiet: Duration, recheck: Duration) -> Self {
        Self {
            quiet,
            recheck,
            pending: HashMap::new(),
        }
    }

    /// An event for `path` at `now`: its quiet period starts over, and a
    /// look already taken no longer counts.
    pub fn touch(&mut self, path: PathBuf, now: Instant) {
        self.pending.insert(
            path,
            Pending {
                last_event: now,
                checked: None,
            },
        );
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// When the next poll could release something.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending
            .values()
            .map(|pending| match pending.checked {
                Some((at, _)) => at + self.recheck,
                None => pending.last_event + self.quiet,
            })
            .min()
    }

    pub fn poll(&mut self, now: Instant, stat: impl Fn(&Path) -> Stat) -> Ready {
        let mut ready = Ready::default();
        let mut settled = Vec::new();
        for (path, pending) in &mut self.pending {
            if now.saturating_duration_since(pending.last_event) < self.quiet {
                continue;
            }
            if let Some((at, _)) = pending.checked {
                if now.saturating_duration_since(at) < self.recheck {
                    continue;
                }
            }
            match stat(path) {
                Stat::Gone => {
                    ready.gone.push(path.clone());
                    settled.push(path.clone());
                }
                Stat::Dir => {
                    ready.dirs.push(path.clone());
                    settled.push(path.clone());
                }
                Stat::File(..) if !is_pdf_name(path) => settled.push(path.clone()),
                now_stat @ Stat::File(..) => match pending.checked {
                    Some((_, before)) if before == now_stat => {
                        ready.files.push(path.clone());
                        settled.push(path.clone());
                    }
                    // First look, or still growing: look again later.
                    _ => pending.checked = Some((now, now_stat)),
                },
            }
        }
        for path in settled {
            self.pending.remove(&path);
        }
        ready.files.sort();
        ready.gone.sort();
        ready.dirs.sort();
        ready
    }
}

/// The safety net's comparison: PDFs under `roots` that are new or whose
/// stamp differs from `known`, and known paths that are no longer files.
/// Stat-only; nothing is opened.
pub fn rescan_diff(
    roots: &[PathBuf],
    skip_dirs: &[PathBuf],
    known: &HashMap<PathBuf, (u64, u64)>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut seen = HashSet::new();
    let mut changed = Vec::new();
    for root in roots.iter().filter(|root| root_mounted(root)) {
        for path in collect_pdfs(root) {
            if is_ignored(&path, roots, skip_dirs) || !seen.insert(path.clone()) {
                continue;
            }
            let stamp = file_stamp(&path);
            if stamp.is_some() && known.get(&path).copied() != stamp {
                changed.push(path);
            }
        }
    }
    let mut gone: Vec<PathBuf> = known
        .keys()
        .filter(|path| !seen.contains(*path) && !path.is_file())
        .cloned()
        .collect();
    changed.sort();
    gone.sort();
    (changed, gone)
}

/// Whether a root can be watched now: an existing folder, not on a
/// `/Volumes` volume that is unmounted.
pub fn root_mounted(root: &Path) -> bool {
    root.is_dir()
        && !root
            .to_str()
            .is_some_and(super::papers::on_unmounted_volume)
}

/// The engine's side of the watcher.
pub trait WatchSink: Send + 'static {
    /// The library roots now; None once the engine is gone.
    fn roots(&self) -> Option<Vec<PathBuf>>;
    /// Folders never to report from: the app's own data.
    fn skip_dirs(&self) -> Vec<PathBuf>;
    /// A settled batch: PDFs to scan, paths that are gone.
    fn deliver(&self, files: Vec<PathBuf>, gone: Vec<PathBuf>);
    /// Every indexed path's stamp, for the safety net; None while a full
    /// scan is running, which covers the same ground.
    fn known_stamps(&self) -> Option<HashMap<PathBuf, (u64, u64)>>;
}

enum Msg {
    Event(notify::Result<notify::Event>),
    Rearm,
    Rescan,
    Stop,
}

/// Owner's handle on the watcher thread. Dropping it stops the thread.
pub struct WatchHandle {
    tx: Sender<Msg>,
}

impl WatchHandle {
    /// The roots changed: watch the new set.
    pub fn rearm(&self) {
        let _ = self.tx.send(Msg::Rearm);
    }

    /// Run the safety-net pass now rather than at its next tick.
    pub fn rescan(&self) {
        let _ = self.tx.send(Msg::Rescan);
    }
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arm {
    Watching,
    /// Not there now; armed when it comes back.
    Unmounted,
    /// The watcher refused it; the safety net covers it.
    Failed,
}

pub fn spawn(sink: impl WatchSink, timing: WatchTiming) -> WatchHandle {
    let (tx, rx) = mpsc::channel::<Msg>();
    let events = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("speedyf-library-watch".into())
        .spawn(move || run(sink, timing, events, rx));
    if let Err(error) = spawned {
        log::warn!("cannot start the library watcher: {error}");
    }
    WatchHandle { tx }
}

fn run(sink: impl WatchSink, timing: WatchTiming, events: Sender<Msg>, rx: mpsc::Receiver<Msg>) {
    use notify::Watcher;
    let mut watcher = match notify::recommended_watcher(move |event| {
        let _ = events.send(Msg::Event(event));
    }) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log::warn!("library watcher unavailable, rescanning every 10 minutes: {error}");
            None
        }
    };
    let mut armed: HashMap<PathBuf, Arm> = HashMap::new();
    let mut warned_events = false;
    let mut debouncer = Debouncer::new(timing.quiet, timing.recheck);
    let start = Instant::now();
    let mut next_rescan = start + timing.rescan_every;
    let mut next_arm_check = start + timing.arm_check_every;
    let mut rescan_now = false;

    // Arm the roots there are, and drop the ones that went.
    let arm = |watcher: &mut Option<notify::RecommendedWatcher>,
               armed: &mut HashMap<PathBuf, Arm>,
               roots: &[PathBuf]|
     -> bool {
        let mut returned = false;
        armed.retain(|root, state| {
            let keep = roots.contains(root);
            if !keep && *state == Arm::Watching {
                if let Some(watcher) = watcher.as_mut() {
                    let _ = watcher.unwatch(root);
                }
            }
            keep
        });
        for root in roots {
            let state = armed.get(root).copied();
            let mounted = root_mounted(root);
            match (state, mounted) {
                (Some(Arm::Watching), true) | (Some(Arm::Failed), _) => {}
                (Some(Arm::Watching), false) => {
                    if let Some(watcher) = watcher.as_mut() {
                        let _ = watcher.unwatch(root);
                    }
                    armed.insert(root.clone(), Arm::Unmounted);
                }
                (_, false) => {
                    armed.insert(root.clone(), Arm::Unmounted);
                }
                (state, true) => {
                    let result = match watcher.as_mut() {
                        Some(watcher) => watcher
                            .watch(root, notify::RecursiveMode::Recursive)
                            .map_err(|error| error.to_string()),
                        None => Err("no watcher".to_owned()),
                    };
                    match result {
                        Ok(()) => {
                            armed.insert(root.clone(), Arm::Watching);
                            returned |= state == Some(Arm::Unmounted);
                        }
                        Err(error) => {
                            if watcher.is_some() {
                                log::warn!(
                                    "cannot watch library folder {}, rescanning it every 10 minutes: {error}",
                                    root.display()
                                );
                            }
                            armed.insert(root.clone(), Arm::Failed);
                        }
                    }
                }
            }
        }
        returned
    };

    let Some(roots) = sink.roots() else { return };
    arm(&mut watcher, &mut armed, &roots);

    loop {
        let now = Instant::now();
        let mut due = next_rescan.min(next_arm_check);
        if let Some(next) = debouncer.next_due() {
            due = due.min(next);
        }
        let wait = due
            .saturating_duration_since(now)
            .max(Duration::from_millis(10));
        match rx.recv_timeout(wait) {
            Ok(Msg::Event(Ok(event))) => {
                if event.need_rescan() {
                    rescan_now = true;
                }
                if relevant(&event.kind) {
                    let roots = sink.roots().unwrap_or_default();
                    let skip = sink.skip_dirs();
                    let now = Instant::now();
                    for path in event.paths {
                        if !is_ignored(&path, &roots, &skip) {
                            debouncer.touch(path, now);
                        }
                    }
                }
            }
            Ok(Msg::Event(Err(error))) => {
                if !warned_events {
                    warned_events = true;
                    log::warn!("library watcher error, the 10-minute rescan covers it: {error}");
                }
                rescan_now = true;
            }
            Ok(Msg::Rearm) => {
                let Some(roots) = sink.roots() else { return };
                if arm(&mut watcher, &mut armed, &roots) {
                    rescan_now = true;
                }
            }
            Ok(Msg::Rescan) => rescan_now = true,
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        let now = Instant::now();
        if !debouncer.is_empty() {
            let ready = debouncer.poll(now, stat);
            if !ready.dirs.is_empty() {
                let roots = sink.roots().unwrap_or_default();
                let skip = sink.skip_dirs();
                for dir in &ready.dirs {
                    for path in collect_pdfs(dir) {
                        if !is_ignored(&path, &roots, &skip) {
                            debouncer.touch(path, now);
                        }
                    }
                }
            }
            if !ready.files.is_empty() || !ready.gone.is_empty() {
                sink.deliver(ready.files, ready.gone);
            }
        }
        if now >= next_arm_check {
            next_arm_check = now + timing.arm_check_every;
            let Some(roots) = sink.roots() else { return };
            if arm(&mut watcher, &mut armed, &roots) {
                rescan_now = true;
            }
        }
        if rescan_now || now >= next_rescan {
            rescan_now = false;
            match sink.known_stamps() {
                Some(known) => {
                    next_rescan = now + timing.rescan_every;
                    let roots = sink.roots().unwrap_or_default();
                    let skip = sink.skip_dirs();
                    let (changed, gone) = rescan_diff(&roots, &skip, &known);
                    for path in changed.into_iter().chain(gone) {
                        debouncer.touch(path, now);
                    }
                }
                None => next_rescan = now + RESCAN_RETRY.min(timing.rescan_every),
            }
        }
    }
}

/// Whether an event kind can mean a file appeared, changed or went.
/// Opening or reading a file is not one; closing it after a write is.
fn relevant(kind: &notify::EventKind) -> bool {
    use notify::event::{AccessKind, AccessMode, EventKind};
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn non_pdfs_hidden_files_trash_and_partial_downloads_are_ignored() {
        let root = PathBuf::from("/lib");
        let roots = vec![root.clone()];
        let skip = vec![PathBuf::from("/lib/appdata")];
        let ignored = |p: &str| is_ignored(Path::new(p), &roots, &skip);
        assert!(!ignored("/lib/a.pdf"));
        assert!(!ignored("/lib/sub/B.PDF"));
        // Non-PDF files pass the path filter (they may be folders) and are
        // dropped once they are seen to be files; see the debouncer test.
        assert!(!is_pdf_name(Path::new("/lib/notes.txt")));
        assert!(is_pdf_name(Path::new("/lib/x.Pdf")));
        assert!(ignored("/elsewhere/a.pdf"));
        assert!(ignored("/lib/.hidden.pdf"));
        assert!(ignored("/lib/.cache/a.pdf"));
        assert!(ignored("/lib/a.pdf.crdownload"));
        assert!(ignored("/lib/a.pdf.part"));
        assert!(ignored("/lib/a.pdf.download/a.pdf"));
        assert!(ignored("/lib/Unconfirmed 1.CRDOWNLOAD"));
        assert!(ignored("/lib/appdata/x.pdf"));
        // A root that holds the home folder: the Trash is hidden, and a
        // volume's .Trashes is ignored wherever it is.
        let home = vec![PathBuf::from("/Users/me")];
        assert!(is_ignored(Path::new("/Users/me/.Trash/a.pdf"), &home, &[]));
        assert!(is_ignored(
            Path::new("/Volumes/X/.Trashes/501/a.pdf"),
            &[PathBuf::from("/Volumes/X")],
            &[]
        ));
        assert!(is_ignored(
            Path::new("/home/me/.local/share/Trash/files/a.pdf"),
            &[PathBuf::from("/home/me")],
            &[]
        ));
        // A root inside a hidden folder is still a root.
        let hidden_root = vec![PathBuf::from("/Users/me/.papers")];
        assert!(!is_ignored(
            Path::new("/Users/me/.papers/a.pdf"),
            &hidden_root,
            &[]
        ));
    }

    #[test]
    fn a_path_is_released_after_quiet_and_two_matching_looks() {
        let t0 = Instant::now();
        let mut debouncer = Debouncer::new(ms(1500), ms(500));
        let path = PathBuf::from("/lib/a.pdf");
        let stamp = std::cell::Cell::new(Stat::File(1, 100));
        let look = |_: &Path| stamp.get();
        debouncer.touch(path.clone(), t0);
        assert_eq!(debouncer.next_due(), Some(t0 + ms(1500)));
        assert!(debouncer.poll(t0 + ms(1000), look).is_empty(), "not quiet");
        // Another event restarts the quiet period.
        debouncer.touch(path.clone(), t0 + ms(1000));
        assert!(debouncer.poll(t0 + ms(2000), look).is_empty());
        // First look once quiet: recorded, not released.
        assert!(debouncer.poll(t0 + ms(2500), look).is_empty());
        assert_eq!(debouncer.next_due(), Some(t0 + ms(3000)));
        // Still growing at the second look: look again.
        stamp.set(Stat::File(2, 200));
        assert!(debouncer.poll(t0 + ms(3000), look).is_empty());
        assert!(debouncer.poll(t0 + ms(3200), look).is_empty(), "too soon");
        let ready = debouncer.poll(t0 + ms(3500), look);
        assert_eq!(ready.files, vec![path]);
        assert!(debouncer.is_empty());
    }

    #[test]
    fn non_pdf_files_are_dropped_and_folders_handed_back() {
        let t0 = Instant::now();
        let mut debouncer = Debouncer::new(ms(100), ms(50));
        debouncer.touch("/lib/notes.txt".into(), t0);
        debouncer.touch("/lib/new folder".into(), t0);
        let ready = debouncer.poll(t0 + ms(100), |path| {
            if path.ends_with("new folder") {
                Stat::Dir
            } else {
                Stat::File(1, 1)
            }
        });
        assert_eq!(ready.dirs, vec![PathBuf::from("/lib/new folder")]);
        assert!(ready.files.is_empty() && ready.gone.is_empty());
        assert!(debouncer.is_empty(), "the text file is dropped");
    }

    #[test]
    fn both_halves_of_a_rename_settle_in_one_batch() {
        let t0 = Instant::now();
        let mut debouncer = Debouncer::new(ms(1500), ms(500));
        // inotify and Windows pair them; FSEvents reports each path alone,
        // a moment apart. Either way both are pending together.
        debouncer.touch("/lib/old.pdf".into(), t0);
        debouncer.touch("/lib/new.pdf".into(), t0 + ms(20));
        let look = |path: &Path| {
            if path.ends_with("old.pdf") {
                Stat::Gone
            } else {
                Stat::File(5, 500)
            }
        };
        let first = debouncer.poll(t0 + ms(1520), look);
        // The vanished path needs no second look; the arrival does.
        assert_eq!(first.gone, vec![PathBuf::from("/lib/old.pdf")]);
        assert!(first.files.is_empty());
        let second = debouncer.poll(t0 + ms(2020), look);
        assert_eq!(second.files, vec![PathBuf::from("/lib/new.pdf")]);
    }

    #[test]
    fn a_burst_of_files_comes_out_as_one_batch() {
        let t0 = Instant::now();
        let mut debouncer = Debouncer::new(ms(1500), ms(500));
        for n in 0..50 {
            debouncer.touch(PathBuf::from(format!("/lib/drop/{n:02}.pdf")), t0 + ms(n));
        }
        let mut batches = Vec::new();
        let mut now = t0;
        while !debouncer.is_empty() {
            now = debouncer.next_due().unwrap();
            let ready = debouncer.poll(now, |_| Stat::File(1, 1));
            if !ready.files.is_empty() {
                batches.push(ready.files.len());
            }
        }
        // Their looks fall within the burst's 50 ms; the second looks are
        // due at different instants, so polling at each due time is the
        // worst case for batching and still bounded.
        assert!(now <= t0 + ms(2100));
        assert_eq!(batches.iter().sum::<usize>(), 50);
        // Polled at a coarser tick (the thread's minimum wait), it is one.
        let mut debouncer = Debouncer::new(ms(1500), ms(500));
        for n in 0..50 {
            debouncer.touch(PathBuf::from(format!("/lib/drop/{n:02}.pdf")), t0 + ms(n));
        }
        assert!(debouncer
            .poll(t0 + ms(1550), |_| Stat::File(1, 1))
            .is_empty());
        let ready = debouncer.poll(t0 + ms(2050), |_| Stat::File(1, 1));
        assert_eq!(ready.files.len(), 50);
    }

    #[test]
    fn the_safety_net_finds_new_changed_and_gone_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("same.pdf"), b"%PDF-1.4 same").unwrap();
        std::fs::write(root.join("edited.pdf"), b"%PDF-1.4 edited").unwrap();
        std::fs::write(root.join("new.pdf"), b"%PDF-1.4 new").unwrap();
        std::fs::create_dir(root.join(".Trash")).unwrap();
        std::fs::write(root.join(".Trash/trashed.pdf"), b"%PDF-1.4 t").unwrap();
        let mut known = HashMap::new();
        known.insert(
            root.join("same.pdf"),
            file_stamp(&root.join("same.pdf")).unwrap(),
        );
        known.insert(root.join("edited.pdf"), (1, 1));
        known.insert(root.join("deleted.pdf"), (1, 1));
        let (changed, gone) = rescan_diff(std::slice::from_ref(&root), &[], &known);
        assert_eq!(changed, vec![root.join("edited.pdf"), root.join("new.pdf")]);
        assert_eq!(gone, vec![root.join("deleted.pdf")]);
        // A root that is not there is not walked, and its files are gone.
        let missing = root.join("not-mounted");
        assert!(!root_mounted(&missing));
        assert!(!root_mounted(Path::new(
            "/Volumes/speedyf-not-mounted-x/papers"
        )));
    }

    struct Collect {
        roots: Vec<PathBuf>,
        got: Arc<parking_lot::Mutex<(Vec<PathBuf>, Vec<PathBuf>)>>,
    }

    impl WatchSink for Collect {
        fn roots(&self) -> Option<Vec<PathBuf>> {
            Some(self.roots.clone())
        }
        fn skip_dirs(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn deliver(&self, files: Vec<PathBuf>, gone: Vec<PathBuf>) {
            let mut got = self.got.lock();
            got.0.extend(files);
            got.1.extend(gone);
        }
        fn known_stamps(&self) -> Option<HashMap<PathBuf, (u64, u64)>> {
            None
        }
    }

    fn wait_for(what: &str, check: impl Fn() -> bool, state: impl Fn() -> String) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !check() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {}",
                state()
            );
            std::thread::sleep(ms(50));
        }
    }

    /// Whether this environment delivers file events at all: a sandbox
    /// can keep FSEvents from a process. The watcher's fallback (the
    /// 10-minute pass) is what such a machine runs on.
    fn events_arrive(root: &Path) -> bool {
        use notify::Watcher;
        let (tx, rx) = mpsc::channel();
        let Ok(mut watcher) = notify::recommended_watcher(tx) else {
            return false;
        };
        if watcher
            .watch(root, notify::RecursiveMode::Recursive)
            .is_err()
        {
            return false;
        }
        std::thread::sleep(ms(300));
        std::fs::write(root.join("probe.txt"), b"probe").unwrap();
        let arrived = rx.recv_timeout(Duration::from_secs(20)).is_ok();
        std::fs::remove_file(root.join("probe.txt")).unwrap();
        arrived
    }

    /// The real watcher on a temporary folder. Generous timeouts: CI
    /// machines are slow and FSEvents batches.
    #[test]
    fn the_real_watcher_reports_a_new_file_its_rename_and_its_removal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        if !events_arrive(&root) {
            eprintln!("skipped: this environment delivers no file events");
            return;
        }
        let got = Arc::new(parking_lot::Mutex::new((Vec::new(), Vec::new())));
        let timing = WatchTiming {
            quiet: ms(300),
            recheck: ms(200),
            ..WatchTiming::default()
        };
        let handle = spawn(
            Collect {
                roots: vec![root.clone()],
                got: Arc::clone(&got),
            },
            timing,
        );
        // Let the watcher arm before the first write.
        std::thread::sleep(ms(500));
        let first = root.join("paper.pdf");
        std::fs::write(&first, b"%PDF-1.4 watched").unwrap();
        std::fs::write(root.join("notes.txt"), b"not a pdf").unwrap();
        let seen = || format!("{:?}", got.lock());
        wait_for("the new file", || got.lock().0.contains(&first), seen);
        let renamed = root.join("renamed.pdf");
        std::fs::rename(&first, &renamed).unwrap();
        wait_for(
            "the rename",
            || {
                let got = got.lock();
                got.0.contains(&renamed) && got.1.contains(&first)
            },
            seen,
        );
        std::fs::remove_file(&renamed).unwrap();
        wait_for("the removal", || got.lock().1.contains(&renamed), seen);
        assert!(!got.lock().0.iter().any(|p| p.ends_with("notes.txt")));
        drop(handle);
    }
}

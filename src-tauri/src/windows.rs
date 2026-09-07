//! Which windows exist, what each has open, and the cross-window moves the
//! frontend needs: focus the window that already holds a file, hand a tab to
//! another window, open a new one.
//!
//! This lives in Rust rather than in the webviews because every question it
//! answers spans windows, and a webview can only see itself. Two windows
//! sharing one `localStorage` cannot even keep a session between them — both
//! write the same key and the last one wins.
//!
//! The registry doubles as that session: the same per-window list of open
//! paths is what gets written to disk and read back at launch.

use crate::errors::{AppError, AppResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

pub const MAIN_LABEL: &str = "main";
const SESSION_FILE: &str = "session.json";
const SESSION_VERSION: u32 = 2;

/// Past this the windows are unmanageable anyway, and a restore loop that has
/// gone wrong stops being able to spawn windows forever.
const MAX_WINDOWS: usize = 12;

/// Same geometry the main window is configured with in `tauri.conf.json`.
/// A new window is not a different kind of window.
const WINDOW_SIZE: (f64, f64) = (1280.0, 840.0);
const MIN_WINDOW_SIZE: (f64, f64) = (720.0, 480.0);
/// How far a new window is offset from the one it was spawned from, so it
/// does not land exactly on top of it.
const CASCADE: f64 = 28.0;

/// One window's open documents. Ordered as the tab strip orders them.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowTabs {
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub active_path: Option<String>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    version: u32,
    #[serde(default)]
    windows: Vec<WindowTabs>,
}

/// What a window asks for once, on mount: the tabs it should reopen and any
/// paths waiting for it. Both are drained by the read.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowStartupDto {
    restore: WindowTabs,
    opens: Vec<String>,
}

/// A window as another window's "move tab to…" menu sees it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummaryDto {
    label: String,
    /// The active document's file name, or the first tab's — what the window
    /// is actually showing, which is the only way to tell two of them apart.
    title: String,
    tab_count: usize,
}

#[derive(Default)]
struct Registry {
    /// Live windows in creation order, which is the order menus list them in.
    live: Vec<String>,
    tabs: HashMap<String, WindowTabs>,
    /// Paths handed to a window that has not asked for them yet: a cold
    /// launch, or a window created a moment ago to receive a moved tab.
    pending: HashMap<String, Vec<String>>,
    /// Tabs a restored window should reopen, drained once at startup.
    restore: HashMap<String, WindowTabs>,
    next_id: u32,
    /// Set once the app is on its way out, so that windows tearing down do
    /// not erase the session they exist to leave behind. Without it, quitting
    /// with three windows open writes a session with none: each Destroyed
    /// event would prune its own window on the way past.
    exiting: bool,
}

impl Registry {
    /// The session as it stands: every live window, in creation order.
    fn snapshot(&self) -> SessionFile {
        SessionFile {
            version: SESSION_VERSION,
            windows: self
                .live
                .iter()
                .map(|label| self.tabs.get(label).cloned().unwrap_or_default())
                .collect(),
        }
    }
}

/// Stand-in for a window that has not published its tabs yet.
static EMPTY_TABS: WindowTabs = WindowTabs {
    tabs: Vec::new(),
    active_path: None,
};

pub struct WindowState {
    inner: Mutex<Registry>,
    /// `None` when the app data directory is unavailable — the session is
    /// then simply not persisted, which is not a reason to fail a launch.
    session_path: Option<PathBuf>,
}

impl WindowState {
    /// Reads the session from disk without acting on it. `setup` decides how
    /// many windows to raise; this only says what was open.
    pub fn new(app_data_dir: Option<PathBuf>) -> (Self, Vec<WindowTabs>) {
        let session_path = app_data_dir.map(|dir| dir.join(SESSION_FILE));
        let restored = session_path
            .as_deref()
            .and_then(read_session)
            .map(|s| s.windows)
            .unwrap_or_default();
        (
            Self {
                inner: Mutex::new(Registry::default()),
                session_path,
            },
            restored,
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Registry> {
        // A registry poisoned by a panic elsewhere is still perfectly usable
        // data; refusing to open windows over it helps nobody.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds `label` to the live set, optionally with tabs to restore and
    /// paths to open once its frontend is listening.
    fn register(&self, label: String, restore: Option<WindowTabs>, opens: Vec<String>) {
        let mut reg = self.lock();
        if !reg.live.contains(&label) {
            reg.live.push(label.clone());
        }
        if let Some(tabs) = restore {
            reg.restore.insert(label.clone(), tabs);
        }
        if !opens.is_empty() {
            reg.pending.entry(label).or_default().extend(opens);
        }
    }

    fn forget(&self, label: &str) {
        let mut reg = self.lock();
        if reg.exiting {
            return;
        }
        reg.live.retain(|l| l != label);
        reg.tabs.remove(label);
        reg.pending.remove(label);
        reg.restore.remove(label);
    }

    /// Stops any further pruning: from here on the session file describes the
    /// windows that were open when the user quit.
    pub fn begin_exit(&self) {
        self.lock().exiting = true;
    }

    /// Mints the next window label. Labels are never reused within a run, so
    /// a pending open cannot land in a window that replaced its addressee.
    fn next_label(&self) -> String {
        let mut reg = self.lock();
        reg.next_id += 1;
        format!("win-{}", reg.next_id)
    }

    fn window_count(&self) -> usize {
        self.lock().live.len()
    }

    fn persist(&self) {
        let Some(path) = self.session_path.as_deref() else {
            return;
        };
        let snapshot = self.lock().snapshot();
        // A session naming no windows at all is never a state worth keeping:
        // launching into nothing is what happens with no session file anyway,
        // so the only thing such a write can do is lose what was open. It is
        // the shape a teardown produces, not one the user ever asks for — a
        // window that is open with no tabs still reports itself, empty.
        if snapshot.windows.is_empty() {
            return;
        }
        // A session that fails to write costs the next launch its tabs, and
        // nothing else. It is never worth interrupting the user over.
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(&snapshot) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, bytes) {
                    log::warn!("could not write session: {e}");
                }
            }
            Err(e) => log::warn!("could not encode session: {e}"),
        }
    }
}

fn read_session(path: &Path) -> Option<SessionFile> {
    let bytes = std::fs::read(path).ok()?;
    let parsed: SessionFile = serde_json::from_slice(&bytes).ok()?;
    (parsed.version == SESSION_VERSION).then_some(parsed)
}

/// The path a window is named after: the document it is showing, or failing
/// that the first one it holds.
fn window_path(tabs: &WindowTabs) -> Option<&str> {
    tabs.active_path
        .as_deref()
        .or_else(|| tabs.tabs.first().map(String::as_str))
}

/// The name to show for a window in another window's menu.
fn window_title(tabs: &WindowTabs) -> String {
    match window_path(tabs) {
        Some(path) => Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        None => "Empty window".to_string(),
    }
}

/// Makes repeated titles tell each other apart.
///
/// A menu offering "Move to “main.pdf”" twice names no destination at all,
/// and papers arrive named that way in bulk — `1-s2.0-…-main.pdf` beside
/// `…-main.pdf`. The containing folder is what distinguishes those, so
/// duplicates take it; empty windows, which have no folder, are numbered.
fn disambiguate(entries: &mut [(String, Option<&str>)]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (title, _) in entries.iter() {
        *seen.entry(title.clone()).or_default() += 1;
    }
    let mut nth: HashMap<String, usize> = HashMap::new();
    for (title, path) in entries.iter_mut() {
        if seen.get(title).copied().unwrap_or(0) < 2 {
            continue;
        }
        let index = nth.entry(title.clone()).or_default();
        *index += 1;
        match path
            .and_then(|p| Path::new(p).parent())
            .and_then(|p| p.file_name())
        {
            Some(folder) => *title = format!("{title} — {}", folder.to_string_lossy()),
            None => *title = format!("{title} {}", *index + 1),
        }
    }
}

/// Builds a window and wires its teardown. `restore` and `opens` are stashed
/// before the window exists, so its frontend finds them waiting on mount
/// however fast it loads.
fn build_window(
    app: &AppHandle,
    restore: Option<WindowTabs>,
    opens: Vec<String>,
) -> AppResult<String> {
    let state = app.state::<WindowState>();
    if state.window_count() >= MAX_WINDOWS {
        return Err(AppError::Unsupported(format!(
            "too many windows open (limit {MAX_WINDOWS})"
        )));
    }
    let label = state.next_label();
    state.register(label.clone(), restore, opens);

    let (x, y) = cascade_position(app);
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("SpeedyF")
        .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1)
        .min_inner_size(MIN_WINDOW_SIZE.0, MIN_WINDOW_SIZE.1)
        .position(x, y)
        .build();

    match built {
        Ok(window) => {
            watch_window(app, &window);
            Ok(label)
        }
        Err(e) => {
            // The registry must not keep a window that does not exist, or the
            // move-tab menu offers a destination nothing can reach.
            state.forget(&label);
            Err(AppError::Internal(format!("could not open a window: {e}")))
        }
    }
}

/// Offset from whichever window is focused, so a new one lands beside its
/// parent rather than exactly on top of it. Falls back to the OS default
/// placement when there is nothing to cascade from.
fn cascade_position(app: &AppHandle) -> (f64, f64) {
    let taken = window_positions(app);
    let from = focused_window(app).or_else(|| app.get_webview_window(MAIN_LABEL));
    let Some((mut x, mut y)) = from.and_then(|w| logical_position(&w)) else {
        return (120.0, 120.0);
    };
    // Stepping off the *focused* window is what the user means by "next to
    // this one", but two windows spawned from the same parent would then land
    // on the same spot and read as one window. So keep stepping until the
    // place is free.
    for _ in 0..MAX_WINDOWS {
        x += CASCADE;
        y += CASCADE;
        if !taken
            .iter()
            .any(|(tx, ty)| (tx - x).abs() < 2.0 && (ty - y).abs() < 2.0)
        {
            break;
        }
    }
    (x, y)
}

fn logical_position(window: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let logical = window.outer_position().ok()?.to_logical::<f64>(scale);
    Some((logical.x, logical.y))
}

fn window_positions(app: &AppHandle) -> Vec<(f64, f64)> {
    app.webview_windows()
        .values()
        .filter_map(logical_position)
        .collect()
}

/// A window that goes away has to leave the registry, or it stays a
/// destination in every other window's menu and a phantom in the session.
pub fn watch_window<R: tauri::Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    let handle = app.clone();
    let label = window.label().to_string();
    window.on_window_event(move |event| {
        if !matches!(event, WindowEvent::Destroyed) {
            return;
        }
        if let Some(state) = handle.try_state::<WindowState>() {
            state.forget(&label);
            state.persist();
        }
    });
}

/// Reopens the windows that were open at last quit. The main window takes the
/// first entry — it already exists — and the rest get windows of their own.
pub fn restore_windows(app: &AppHandle, session: Vec<WindowTabs>) {
    let state = app.state::<WindowState>();
    let mut entries = session.into_iter();
    state.register(MAIN_LABEL.to_string(), entries.next(), Vec::new());

    for tabs in entries.take(MAX_WINDOWS - 1) {
        // An empty window is not worth restoring: reopening it would greet
        // the user with a home screen they never asked for.
        if tabs.tabs.is_empty() {
            continue;
        }
        if let Err(e) = build_window(app, Some(tabs), Vec::new()) {
            log::warn!("could not restore a window: {e}");
        }
    }
}

/// The window an OS-level open should land in: whichever the user is looking
/// at, else the main one.
pub fn target_window(app: &AppHandle) -> String {
    focused_window(app)
        .map(|w| w.label().to_string())
        .unwrap_or_else(|| MAIN_LABEL.to_string())
}

fn focused_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.webview_windows()
        .into_values()
        .find(|w| w.is_focused().unwrap_or(false))
}

/// Hand `paths` to one window: stashed for a frontend that is not listening
/// yet, and emitted for one that is. Both, because a cold launch reaches here
/// before the webview exists and a running window never sees the stash.
pub fn deliver_to_window(app: &AppHandle, label: &str, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    if let Some(state) = app.try_state::<WindowState>() {
        state
            .lock()
            .pending
            .entry(label.to_string())
            .or_default()
            .extend(paths.clone());
    }
    let _ = app.emit_to(label, "file-open-requested", &paths);
}

/// Bring a window forward. The OS hands us a file but does not raise us:
/// without this an already-running SpeedyF loads the PDF behind whatever the
/// user was looking at, and the open looks like it did nothing.
pub fn raise_window(app: &AppHandle, label: &str) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// --- Commands --------------------------------------------------------------

/// Everything a window needs at mount, drained in one call so a slow frontend
/// cannot miss half of it.
#[tauri::command]
pub fn window_startup(
    state: tauri::State<'_, WindowState>,
    label: String,
) -> AppResult<WindowStartupDto> {
    let mut reg = state.lock();
    if !reg.live.contains(&label) {
        reg.live.push(label.clone());
    }
    let restore = reg.restore.remove(&label).unwrap_or_default();
    let opens = reg.pending.remove(&label).unwrap_or_default();
    Ok(WindowStartupDto { restore, opens })
}

/// What this window has open, as of now. Every window republishes on any tab
/// change; the registry is what the other windows read.
#[tauri::command]
pub fn update_window_tabs(
    state: tauri::State<'_, WindowState>,
    label: String,
    tabs: Vec<String>,
    active_path: Option<String>,
) -> AppResult<()> {
    {
        let mut reg = state.lock();
        if !reg.live.contains(&label) {
            reg.live.push(label.clone());
        }
        reg.tabs.insert(label, WindowTabs { tabs, active_path });
    }
    state.persist();
    Ok(())
}

/// The other windows, for a "move tab to…" menu.
#[tauri::command]
pub fn window_roster(
    state: tauri::State<'_, WindowState>,
    exclude: Option<String>,
) -> AppResult<Vec<WindowSummaryDto>> {
    let reg = state.lock();
    let listed: Vec<(&String, &WindowTabs)> = reg
        .live
        .iter()
        .filter(|label| Some(label.as_str()) != exclude.as_deref())
        .map(|label| (label, reg.tabs.get(label).unwrap_or(&EMPTY_TABS)))
        .collect();

    let mut titles: Vec<(String, Option<&str>)> = listed
        .iter()
        .map(|(_, tabs)| (window_title(tabs), window_path(tabs)))
        .collect();
    disambiguate(&mut titles);

    Ok(listed
        .iter()
        .zip(titles)
        .map(|((label, tabs), (title, _))| WindowSummaryDto {
            label: (*label).clone(),
            title,
            tab_count: tabs.tabs.len(),
        })
        .collect())
}

/// Which other window already has `path` open, if any. One document open
/// twice means two PDFium handles, two dirty states, and whichever window
/// saves last silently overwriting the other — so nobody opens a second copy.
#[tauri::command]
pub fn find_window_for_path(
    state: tauri::State<'_, WindowState>,
    path: String,
    exclude: Option<String>,
) -> AppResult<Option<String>> {
    let reg = state.lock();
    Ok(reg
        .live
        .iter()
        .filter(|label| Some(label.as_str()) != exclude.as_deref())
        .find(|label| reg.tabs.get(*label).is_some_and(|t| t.tabs.contains(&path)))
        .cloned())
}

#[tauri::command]
pub fn focus_window(app: tauri::AppHandle, label: String) -> AppResult<()> {
    raise_window(&app, &label);
    Ok(())
}

/// Opens a window, optionally on some documents. Returns its label so the
/// caller can address it straight away.
#[tauri::command]
pub fn open_window(app: tauri::AppHandle, paths: Vec<String>) -> AppResult<String> {
    build_window(&app, None, paths)
}

/// Hands documents to an existing window and brings it forward — the
/// receiving half of moving a tab between windows.
#[tauri::command]
pub fn send_paths_to_window(
    app: tauri::AppHandle,
    label: String,
    paths: Vec<String>,
) -> AppResult<()> {
    if app.get_webview_window(&label).is_none() {
        return Err(AppError::Unsupported("that window is gone".into()));
    }
    deliver_to_window(&app, &label, paths);
    raise_window(&app, &label);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(paths: &[&str], active: Option<&str>) -> WindowTabs {
        WindowTabs {
            tabs: paths.iter().map(|p| p.to_string()).collect(),
            active_path: active.map(str::to_string),
        }
    }

    #[test]
    fn a_window_is_named_by_the_document_it_is_showing() {
        assert_eq!(
            window_title(&tabs(
                &["/papers/a.pdf", "/papers/b.pdf"],
                Some("/papers/b.pdf")
            )),
            "b.pdf"
        );
        // No active tab (the home screen) still names the window by something
        // the user recognises rather than going blank.
        assert_eq!(window_title(&tabs(&["/papers/a.pdf"], None)), "a.pdf");
        assert_eq!(window_title(&tabs(&[], None)), "Empty window");
    }

    #[test]
    fn windows_showing_the_same_file_name_are_told_apart_by_their_folder() {
        // Papers arrive named this way in bulk: a menu offering "Move to
        // main.pdf" twice names no destination at all.
        let mut entries = vec![
            ("main.pdf".to_string(), Some("/papers/elsevier/main.pdf")),
            ("main.pdf".to_string(), Some("/papers/springer/main.pdf")),
            ("solo.pdf".to_string(), Some("/papers/solo.pdf")),
        ];
        disambiguate(&mut entries);
        assert_eq!(entries[0].0, "main.pdf — elsevier");
        assert_eq!(entries[1].0, "main.pdf — springer");
        // A name nothing collides with is left alone.
        assert_eq!(entries[2].0, "solo.pdf");
    }

    #[test]
    fn empty_windows_are_numbered_since_they_have_no_folder_to_name() {
        let mut entries = vec![
            ("Empty window".to_string(), None),
            ("Empty window".to_string(), None),
        ];
        disambiguate(&mut entries);
        assert_eq!(entries[0].0, "Empty window 2");
        assert_eq!(entries[1].0, "Empty window 3");
    }

    #[test]
    fn the_session_snapshot_follows_window_order_not_hash_order() {
        let mut reg = Registry::default();
        for (label, path) in [("main", "/a.pdf"), ("win-1", "/b.pdf"), ("win-2", "/c.pdf")] {
            reg.live.push(label.to_string());
            reg.tabs
                .insert(label.to_string(), tabs(&[path], Some(path)));
        }
        let snapshot = reg.snapshot();
        assert_eq!(snapshot.version, SESSION_VERSION);
        assert_eq!(
            snapshot
                .windows
                .iter()
                .map(|w| w.tabs[0].clone())
                .collect::<Vec<_>>(),
            vec!["/a.pdf", "/b.pdf", "/c.pdf"]
        );
    }

    #[test]
    fn quitting_leaves_the_session_alone_as_windows_tear_down() {
        // Every window fires Destroyed on the way out. Pruning on each would
        // write an empty session and lose everything that was open.
        let (state, _) = WindowState::new(None);
        state.register("main".into(), None, Vec::new());
        state.register("win-1".into(), None, Vec::new());
        state.begin_exit();
        state.forget("main");
        state.forget("win-1");
        assert_eq!(state.lock().live.len(), 2, "the roster survives the exit");
    }

    #[test]
    fn closing_one_window_while_the_app_runs_does_prune_it() {
        let (state, _) = WindowState::new(None);
        state.register("main".into(), None, Vec::new());
        state.register("win-1".into(), None, Vec::new());
        state.forget("win-1");
        assert_eq!(state.lock().live, vec!["main".to_string()]);
    }

    #[test]
    fn a_session_from_a_different_version_is_ignored_rather_than_guessed_at() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join(SESSION_FILE);
        std::fs::write(&path, br#"{"version":1,"windows":[{"tabs":["/a.pdf"]}]}"#).expect("write");
        assert!(read_session(&path).is_none());

        std::fs::write(&path, br#"{"version":2,"windows":[{"tabs":["/a.pdf"]}]}"#).expect("write");
        let session = read_session(&path).expect("current version reads back");
        assert_eq!(session.windows[0].tabs, vec!["/a.pdf".to_string()]);
        // A window with no active tab is legitimate: the home screen.
        assert!(session.windows[0].active_path.is_none());
    }

    #[test]
    fn labels_are_never_reused_within_a_run() {
        // A pending open addressed to a window that has since closed must not
        // land in whatever window came after it.
        let (state, _) = WindowState::new(None);
        let first = state.next_label();
        state.register(first.clone(), None, vec!["/a.pdf".into()]);
        state.forget(&first);
        let second = state.next_label();
        assert_ne!(first, second);
    }
}

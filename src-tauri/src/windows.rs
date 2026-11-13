//! Window ownership and transactional tab handoff. PDF handles stay on the
//! engine thread; only lightweight editor state crosses between webviews.
//!
//! Besides the documents each window owns (by engine id), the registry keeps
//! the *paths* each window shows, in tab order. Those answer the questions
//! that span windows and that a webview cannot answer about itself: which
//! other windows a tab can be moved to, and whether another window already
//! has a file open - nobody opens a second copy of a document, since that is
//! two PDFium handles, two dirty states, and a silent overwrite on save.
use crate::commands::EngineState;
use crate::engine::types::{DocId, Priority};
use crate::engine::Work;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

struct Transfer {
    source: String,
    doc: DocId,
    snapshot: Value,
    ready: mpsc::Sender<()>,
}

#[derive(Default)]
pub struct Windows(Mutex<Registry>);

#[derive(Default)]
struct Registry {
    sequence: u64,
    documents: HashMap<String, HashSet<DocId>>,
    transfers: HashMap<String, Transfer>,
    pending_opens: HashMap<String, Vec<String>>,
    ready: HashSet<String>,
    focused: Option<String>,
    /// What each window shows, as it last published it.
    tabs: HashMap<String, WindowTabs>,
    /// Tabs a window restored from the previous launch should reopen, handed
    /// to it once when it asks.
    restores: HashMap<String, WindowTabs>,
}

/// One window's open documents, in tab-strip order.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowTabs {
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub active_path: Option<String>,
}

/// A window as another window's "move tab to..." menu sees it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummaryDto {
    label: String,
    /// The active document's file name, or the first tab's - what the window
    /// is actually showing, which is the only way to tell two of them apart.
    title: String,
    tab_count: usize,
}

/// Stand-in for a window that has not published its tabs yet.
static EMPTY_TABS: WindowTabs = WindowTabs {
    tabs: Vec::new(),
    active_path: None,
};

impl Windows {
    pub fn opened(&self, label: &str, doc: DocId) {
        self.0
            .lock()
            .unwrap()
            .documents
            .entry(label.into())
            .or_default()
            .insert(doc);
    }

    pub fn closed(&self, label: &str, doc: DocId) {
        if let Some(docs) = self.0.lock().unwrap().documents.get_mut(label) {
            docs.remove(&doc);
        }
    }
}

pub fn target_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    let state = app.state::<Windows>();
    let registry = state.0.lock().unwrap();
    registry
        .focused
        .as_deref()
        .and_then(|label| app.get_webview_window(label))
        .or_else(|| app.get_webview_window("main"))
        .or_else(|| {
            app.webview_windows()
                .into_values()
                .find(|w| !registry.transfers.contains_key(w.label()))
        })
}

pub fn deliver_opens(app: &tauri::AppHandle, paths: Vec<String>) {
    let Some(window) = target_window(app) else {
        return;
    };
    let state = app.state::<Windows>();
    let mut registry = state.0.lock().unwrap();
    if registry.ready.contains(window.label()) {
        let _ = app.emit_to(window.label(), "file-open-requested", paths);
    } else {
        registry
            .pending_opens
            .entry(window.label().into())
            .or_default()
            .extend(paths);
    }
    drop(registry);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

#[tauri::command]
pub fn take_pending_opens(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Windows>,
) -> Vec<String> {
    let mut registry = state.0.lock().unwrap();
    registry.ready.insert(window.label().into());
    registry
        .pending_opens
        .remove(window.label())
        .unwrap_or_default()
}

pub fn build_window(
    app: &tauri::AppHandle,
    label: &str,
    position: Option<(f64, f64)>,
    visible: bool,
) -> Result<tauri::WebviewWindow, String> {
    let mut config = app.config().app.windows[0].clone();
    config.label = label.into();
    config.visible = visible;
    config.focus = visible;
    // New windows start at Home, never by replaying the previous app session.
    config.url = tauri::WebviewUrl::App("index.html?window=new".into());
    let position = position
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .or_else(|| cascade_position(app));
    if let Some((x, y)) = position {
        config.x = Some(x);
        config.y = Some(y);
    }
    tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

/// How far a new window is offset from the one it was spawned from, so it
/// does not land exactly on top of it.
const CASCADE: f64 = 28.0;

/// Offset from whichever window is focused, so a new one lands beside its
/// parent rather than exactly on top of it. `None` leaves placement to the
/// window configuration when there is nothing to cascade from.
fn cascade_position(app: &tauri::AppHandle) -> Option<(f64, f64)> {
    let taken: Vec<(f64, f64)> = app
        .webview_windows()
        .values()
        .filter_map(logical_position)
        .collect();
    let (mut x, mut y) = target_window(app).and_then(|w| logical_position(&w))?;
    // Two windows spawned from the same parent would otherwise land on the
    // same spot and read as one window, so keep stepping until it is free.
    for _ in 0..16 {
        x += CASCADE;
        y += CASCADE;
        if !taken
            .iter()
            .any(|(tx, ty)| (tx - x).abs() < 2.0 && (ty - y).abs() < 2.0)
        {
            break;
        }
    }
    Some((x, y))
}

fn logical_position(window: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let logical = window.outer_position().ok()?.to_logical::<f64>(scale);
    Some((logical.x, logical.y))
}

fn next_label(state: &Windows) -> String {
    let mut registry = state.0.lock().unwrap();
    registry.sequence += 1;
    format!("speedyf-{}", registry.sequence)
}

/// Opens an empty window, or - at launch - one that reopens the tabs a window
/// had when the app last quit.
#[tauri::command]
pub async fn new_window(app: tauri::AppHandle, restore: Option<WindowTabs>) -> Result<(), String> {
    let state = app.state::<Windows>();
    let label = next_label(&state);
    if let Some(restore) = restore {
        state
            .0
            .lock()
            .unwrap()
            .restores
            .insert(label.clone(), restore);
    }
    if let Err(error) = build_window(&app, &label, None, true) {
        state.0.lock().unwrap().restores.remove(&label);
        return Err(error);
    }
    Ok(())
}

/// Opens `path` in a window of its own (quick open's Cmd+Enter, Cmd-click on
/// a file's Open). A path some window already shows, or one a window still
/// coming up was created for, raises that window instead: nobody opens a
/// second copy of a document. Answers whether a new window was made.
#[tauri::command]
pub async fn open_in_new_window(app: tauri::AppHandle, path: String) -> Result<bool, String> {
    let state = app.state::<Windows>();
    let label = next_label(&state);
    let existing = state
        .0
        .lock()
        .unwrap()
        .claim_new_window(&label, path.clone());
    if let Some(existing) = existing {
        let Some(other) = app.get_webview_window(&existing) else {
            return Ok(false);
        };
        let mut registry = state.0.lock().unwrap();
        if registry.ready.contains(&existing) {
            let _ = app.emit_to(existing.as_str(), "file-open-requested", vec![path]);
        } else {
            registry
                .pending_opens
                .entry(existing)
                .or_default()
                .push(path);
        }
        drop(registry);
        let _ = other.unminimize();
        let _ = other.show();
        let _ = other.set_focus();
        return Ok(false);
    }
    if let Err(error) = build_window(&app, &label, None, true) {
        state.0.lock().unwrap().restores.remove(&label);
        return Err(error);
    }
    Ok(true)
}

/// The tabs this window was created to restore, if any. Drained by the read.
#[tauri::command]
pub fn take_window_restore(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Windows>,
) -> Option<WindowTabs> {
    state.0.lock().unwrap().restores.remove(window.label())
}

#[tauri::command]
pub async fn detach_tab(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    doc_id: DocId,
    snapshot: Value,
    position: Option<(f64, f64)>,
) -> Result<(), String> {
    let state = app.state::<Windows>();
    let label = next_label(&state);
    let (ready, receiver) = mpsc::channel();
    {
        let mut registry = state.0.lock().unwrap();
        if !registry
            .documents
            .get(window.label())
            .is_some_and(|docs| docs.contains(&doc_id))
        {
            return Err("This window no longer owns the document".into());
        }
        registry.transfers.insert(
            label.clone(),
            Transfer {
                source: window.label().into(),
                doc: doc_id,
                snapshot,
                ready,
            },
        );
    }
    let target = match build_window(&app, &label, position, false) {
        Ok(target) => target,
        Err(error) => {
            state.0.lock().unwrap().transfers.remove(&label);
            return Err(error);
        }
    };
    // Bounded wait on a blocking worker, never the UI or PDF engine thread.
    let received = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(Duration::from_secs(20))
    })
    .await;
    let failed = {
        let mut registry = state.0.lock().unwrap();
        // A ready signal racing the timeout has already committed ownership.
        if received.is_ok_and(|result| result.is_ok()) {
            false
        } else {
            registry.transfers.remove(&label).is_some()
        }
    };
    if failed {
        let _ = target.destroy();
        return Err("The new window did not finish opening. Your tab is still here.".into());
    }
    let _ = target.show();
    let _ = target.set_focus();
    Ok(())
}

/// Hands a tab to a window that already exists - the "Move to ..." half of the
/// tab menu. The same transaction as `detach_tab`: the target adopts the
/// snapshot and acknowledges, and only then does ownership move. Unsaved edits
/// travel with the snapshot, so nothing has to be saved first.
#[tauri::command]
pub async fn send_tab_to_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    doc_id: DocId,
    snapshot: Value,
    target: String,
) -> Result<(), String> {
    let state = app.state::<Windows>();
    let (ready, receiver) = mpsc::channel();
    {
        let mut registry = state.0.lock().unwrap();
        if target == window.label() {
            return Err("The tab is already in that window".into());
        }
        if !registry
            .documents
            .get(window.label())
            .is_some_and(|docs| docs.contains(&doc_id))
        {
            return Err("This window no longer owns the document".into());
        }
        if app.get_webview_window(&target).is_none() || !registry.ready.contains(&target) {
            return Err("That window is gone".into());
        }
        // One handoff per receiving window at a time: the snapshot is keyed
        // by the receiver, which asks for "its" transfer.
        if registry.transfers.contains_key(&target) {
            return Err("That window is busy receiving another tab".into());
        }
        registry.transfers.insert(
            target.clone(),
            Transfer {
                source: window.label().into(),
                doc: doc_id,
                snapshot,
                ready,
            },
        );
    }
    if let Err(error) = app.emit_to(target.as_str(), "tab-transfer-offered", ()) {
        state.0.lock().unwrap().transfers.remove(&target);
        return Err(error.to_string());
    }
    let received = tauri::async_runtime::spawn_blocking(move || {
        receiver.recv_timeout(Duration::from_secs(20))
    })
    .await;
    let failed = {
        let mut registry = state.0.lock().unwrap();
        if received.is_ok_and(|result| result.is_ok()) {
            false
        } else {
            registry.transfers.remove(&target);
            true
        }
    };
    if failed {
        return Err("The other window did not take the tab. It is still here.".into());
    }
    if let Some(target) = app.get_webview_window(&target) {
        let _ = target.unminimize();
        let _ = target.show();
        let _ = target.set_focus();
    }
    Ok(())
}

/// The receiving window cannot take the offered tab (it is full, or adopting
/// it failed). Dropping the transfer's sender wakes the source at once rather
/// than leaving it waiting out the timeout.
#[tauri::command]
pub fn decline_window_tab(window: tauri::WebviewWindow, state: tauri::State<'_, Windows>) {
    state.0.lock().unwrap().transfers.remove(window.label());
}

/// What this window has open, as of now. Every window republishes on any tab
/// change; the registry is what the other windows read.
#[tauri::command]
pub fn update_window_tabs(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Windows>,
    tabs: Vec<String>,
    active_path: Option<String>,
) {
    state
        .0
        .lock()
        .unwrap()
        .tabs
        .insert(window.label().into(), WindowTabs { tabs, active_path });
}

/// The other windows, for a "move tab to..." menu, in creation order.
#[tauri::command]
pub fn window_roster(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Windows>,
) -> Vec<WindowSummaryDto> {
    state.0.lock().unwrap().roster(window.label())
}

/// If another window already has `path` open, hands it the path (it focuses
/// that tab) and raises it, and answers true. The caller then opens nothing.
#[tauri::command]
pub fn focus_window_for_path(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: String,
) -> bool {
    let state = app.state::<Windows>();
    let mut registry = state.0.lock().unwrap();
    let Some(label) = registry.window_with_path(&path, window.label()) else {
        return false;
    };
    let Some(other) = app.get_webview_window(&label) else {
        return false;
    };
    if registry.ready.contains(&label) {
        let _ = app.emit_to(label.as_str(), "file-open-requested", vec![path]);
    } else {
        registry.pending_opens.entry(label).or_default().push(path);
    }
    drop(registry);
    let _ = other.unminimize();
    let _ = other.show();
    let _ = other.set_focus();
    true
}

#[tauri::command]
pub fn window_tab_snapshot(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Windows>,
) -> Option<Value> {
    state
        .0
        .lock()
        .unwrap()
        .transfers
        .get(window.label())
        .map(|t| t.snapshot.clone())
}

#[tauri::command]
pub fn accept_window_tab(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Windows>,
) -> Result<(), String> {
    state.0.lock().unwrap().accept_transfer(window.label())
}

impl Registry {
    /// Which window other than `exclude` shows `path`, if any.
    fn window_with_path(&self, path: &str, exclude: &str) -> Option<String> {
        let mut labels: Vec<&String> = self
            .tabs
            .iter()
            .filter(|(label, tabs)| {
                label.as_str() != exclude && tabs.tabs.iter().any(|p| p == path)
            })
            .map(|(label, _)| label)
            .collect();
        labels.sort_by_key(|label| window_order(label));
        labels.first().map(|label| (*label).clone())
    }

    /// Either the window that already has `path` (open, or waiting to be
    /// restored with it), or - when none does - `path` queued as the opening
    /// tab of the new window `label`, which takes it at startup the way a
    /// restored window takes its session.
    fn claim_new_window(&mut self, label: &str, path: String) -> Option<String> {
        let mut found = self.window_with_path(&path, label);
        if found.is_none() {
            let mut waiting: Vec<&String> = self
                .restores
                .iter()
                .filter(|(_, tabs)| tabs.tabs.contains(&path))
                .map(|(label, _)| label)
                .collect();
            waiting.sort_by_key(|label| window_order(label));
            found = waiting.first().map(|label| (*label).clone());
        }
        if found.is_some() {
            return found;
        }
        self.restores.insert(
            label.into(),
            WindowTabs {
                tabs: vec![path.clone()],
                active_path: Some(path),
            },
        );
        None
    }

    fn roster(&self, exclude: &str) -> Vec<WindowSummaryDto> {
        let mut labels: Vec<&String> = self
            .ready
            .iter()
            .filter(|label| label.as_str() != exclude && !self.transfers.contains_key(*label))
            .collect();
        labels.sort_by_key(|label| window_order(label));
        let listed: Vec<(&String, &WindowTabs)> = labels
            .into_iter()
            .map(|label| (label, self.tabs.get(label).unwrap_or(&EMPTY_TABS)))
            .collect();
        let mut titles: Vec<(String, Option<&str>)> = listed
            .iter()
            .map(|(_, tabs)| (window_title(tabs), window_path(tabs)))
            .collect();
        disambiguate(&mut titles);
        listed
            .iter()
            .zip(titles)
            .map(|((label, tabs), (title, _))| WindowSummaryDto {
                label: (*label).clone(),
                title,
                tab_count: tabs.tabs.len(),
            })
            .collect()
    }

    fn accept_transfer(&mut self, label: &str) -> Result<(), String> {
        let transfer = self
            .transfers
            .remove(label)
            .ok_or("The tab transfer expired")?;
        if let Some(docs) = self.documents.get_mut(&transfer.source) {
            docs.remove(&transfer.doc);
        }
        self.documents
            .entry(label.into())
            .or_default()
            .insert(transfer.doc);
        let _ = transfer.ready.send(());
        Ok(())
    }
}

pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let app = window.app_handle();
    let Some(state) = app.try_state::<Windows>() else {
        return;
    };
    match event {
        tauri::WindowEvent::Focused(true) => {
            state.0.lock().unwrap().focused = Some(window.label().into())
        }
        tauri::WindowEvent::Destroyed => {
            let mut registry = state.0.lock().unwrap();
            registry.ready.remove(window.label());
            registry.pending_opens.remove(window.label());
            registry.tabs.remove(window.label());
            registry.restores.remove(window.label());
            let docs = registry
                .documents
                .remove(window.label())
                .unwrap_or_default();
            drop(registry);
            let engine = &app.state::<EngineState>().0;
            for doc in docs {
                engine.bump_generation(doc);
                engine.submit(Priority::VisiblePage, doc, Work::Close { doc });
            }
        }
        _ => {}
    }
}

/// Creation order: "main" first, then `speedyf-N` by N.
fn window_order(label: &str) -> u64 {
    label
        .strip_prefix("speedyf-")
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
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
/// A menu offering "Move to "main.pdf"" twice names no destination at all,
/// and papers arrive named that way in bulk - `1-s2.0-...-main.pdf` beside
/// `...-main.pdf`. The containing folder is what distinguishes those, so
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
            Some(folder) => *title = format!("{title} ({})", folder.to_string_lossy()),
            None => *title = format!("{title} {}", *index + 1),
        }
    }
}

pub fn install_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind, Submenu};
    let menu = Menu::default(app)?;
    let item = MenuItem::with_id(app, "new-window", "New Window", true, Some("CmdOrCtrl+N"))?;
    let file = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) if submenu.text().ok().as_deref() == Some("File") => {
            Some(submenu)
        }
        _ => None,
    });
    if let Some(file) = file {
        file.prepend(&item)?;
    } else {
        menu.prepend(&Submenu::with_items(app, "File", true, &[&item])?)?;
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().as_ref() == "new-window" {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = new_window(app, None).await {
                    log::error!("new window: {error}");
                }
            });
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> (Windows, mpsc::Receiver<()>) {
        let windows = Windows::default();
        windows.opened("main", 42);
        windows.opened("main", 43);
        let (ready, receiver) = mpsc::channel();
        windows.0.lock().unwrap().transfers.insert(
            "speedyf-1".into(),
            Transfer {
                source: "main".into(),
                doc: 42,
                snapshot: serde_json::json!({"dirty": true}),
                ready,
            },
        );
        (windows, receiver)
    }

    #[test]
    fn accepting_moves_only_the_transferred_document_and_acknowledges() {
        let (windows, receiver) = pending();
        let mut registry = windows.0.lock().unwrap();
        registry.accept_transfer("speedyf-1").unwrap();
        assert!(receiver.try_recv().is_ok());
        assert_eq!(registry.documents["main"], HashSet::from([43]));
        assert_eq!(registry.documents["speedyf-1"], HashSet::from([42]));
        // Destroying the source can release its own PDFs without touching 42.
        assert_eq!(
            registry.documents.remove("main").unwrap(),
            HashSet::from([43])
        );
        assert!(registry.accept_transfer("speedyf-1").is_err());
    }

    #[test]
    fn failed_transfer_retains_source_ownership() {
        let (windows, receiver) = pending();
        let mut registry = windows.0.lock().unwrap();
        registry.transfers.remove("speedyf-1");
        assert!(receiver.try_recv().is_err());
        assert!(registry.accept_transfer("speedyf-1").is_err());
        assert_eq!(registry.documents["main"], HashSet::from([42, 43]));
        assert!(!registry.documents.contains_key("speedyf-1"));
    }

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
        let mut entries = vec![
            ("main.pdf".to_string(), Some("/papers/elsevier/main.pdf")),
            ("main.pdf".to_string(), Some("/papers/springer/main.pdf")),
            ("solo.pdf".to_string(), Some("/papers/solo.pdf")),
        ];
        disambiguate(&mut entries);
        assert_eq!(entries[0].0, "main.pdf (elsevier)");
        assert_eq!(entries[1].0, "main.pdf (springer)");
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

    fn registry_with(windows: &[(&str, &[&str])]) -> Registry {
        let mut registry = Registry::default();
        for (label, paths) in windows {
            registry.ready.insert((*label).into());
            registry
                .tabs
                .insert((*label).into(), tabs(paths, paths.first().copied()));
        }
        registry
    }

    #[test]
    fn the_roster_lists_other_ready_windows_in_creation_order() {
        let mut registry = registry_with(&[
            ("speedyf-10", &["/c.pdf"]),
            ("main", &["/a.pdf"]),
            ("speedyf-2", &["/b.pdf", "/d.pdf"]),
        ]);
        // A window still being built to receive a dragged tab is no
        // destination yet.
        registry.ready.insert("speedyf-11".into());
        let (ready, _receiver) = mpsc::channel();
        registry.transfers.insert(
            "speedyf-11".into(),
            Transfer {
                source: "main".into(),
                doc: 1,
                snapshot: Value::Null,
                ready,
            },
        );
        let roster = registry.roster("speedyf-2");
        let labels: Vec<&str> = roster.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, vec!["main", "speedyf-10"]);
        assert_eq!(roster[0].title, "a.pdf");
        assert_eq!(roster[1].tab_count, 1);
    }

    #[test]
    fn a_path_open_elsewhere_is_found_but_never_in_the_asking_window() {
        let registry =
            registry_with(&[("main", &["/a.pdf"]), ("speedyf-1", &["/b.pdf", "/a.pdf"])]);
        assert_eq!(
            registry.window_with_path("/b.pdf", "main").as_deref(),
            Some("speedyf-1")
        );
        assert_eq!(
            registry.window_with_path("/a.pdf", "speedyf-1").as_deref(),
            Some("main")
        );
        assert_eq!(
            registry.window_with_path("/a.pdf", "main").as_deref(),
            Some("speedyf-1")
        );
        assert_eq!(registry.window_with_path("/b.pdf", "speedyf-1"), None);
        assert_eq!(registry.window_with_path("/z.pdf", "main"), None);
    }

    #[test]
    fn accepting_a_tab_into_an_existing_window_moves_ownership_there() {
        let windows = Windows::default();
        windows.opened("main", 7);
        windows.opened("speedyf-1", 8);
        let (ready, receiver) = mpsc::channel();
        let mut registry = windows.0.lock().unwrap();
        registry.transfers.insert(
            "speedyf-1".into(),
            Transfer {
                source: "main".into(),
                doc: 7,
                snapshot: Value::Null,
                ready,
            },
        );
        registry.accept_transfer("speedyf-1").unwrap();
        assert!(receiver.try_recv().is_ok());
        assert!(registry.documents["main"].is_empty());
        assert_eq!(registry.documents["speedyf-1"], HashSet::from([7, 8]));
    }

    #[test]
    fn declining_wakes_the_source_at_once() {
        let windows = Windows::default();
        let (ready, receiver) = mpsc::channel::<()>();
        windows.0.lock().unwrap().transfers.insert(
            "speedyf-1".into(),
            Transfer {
                source: "main".into(),
                doc: 1,
                snapshot: Value::Null,
                ready,
            },
        );
        windows.0.lock().unwrap().transfers.remove("speedyf-1");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn a_new_window_for_a_path_takes_it_as_its_opening_tab() {
        let mut registry = registry_with(&[("main", &["/a.pdf"])]);
        assert_eq!(
            registry.claim_new_window("speedyf-1", "/b.pdf".into()),
            None
        );
        let restore = registry.restores.remove("speedyf-1").unwrap();
        assert_eq!(restore.tabs, vec!["/b.pdf".to_string()]);
        assert_eq!(restore.active_path.as_deref(), Some("/b.pdf"));
    }

    #[test]
    fn a_path_already_open_or_on_its_way_raises_that_window_instead() {
        let mut registry = registry_with(&[("main", &["/a.pdf"])]);
        // Open in a window (the asking one included): that window, no new one.
        assert_eq!(
            registry.claim_new_window("speedyf-1", "/a.pdf".into()),
            Some("main".to_string())
        );
        assert!(registry.restores.is_empty());
        // A second Cmd+Enter before the first new window has published its
        // tabs still finds it.
        assert_eq!(
            registry.claim_new_window("speedyf-2", "/b.pdf".into()),
            None
        );
        assert_eq!(
            registry.claim_new_window("speedyf-3", "/b.pdf".into()),
            Some("speedyf-2".to_string())
        );
        assert_eq!(registry.restores.len(), 1);
    }

    #[test]
    fn closing_a_tab_does_not_release_other_windows_documents() {
        let windows = Windows::default();
        windows.opened("main", 1);
        windows.opened("speedyf-1", 2);
        windows.closed("main", 1);
        let registry = windows.0.lock().unwrap();
        assert!(registry.documents["main"].is_empty());
        assert_eq!(registry.documents["speedyf-1"], HashSet::from([2]));
    }
}

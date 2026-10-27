//! Window ownership and transactional tab handoff. PDF handles stay on the
//! engine thread; only lightweight editor state crosses between webviews.
use crate::commands::EngineState;
use crate::engine::types::{DocId, Priority};
use crate::engine::Work;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
}

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
    if let Some((x, y)) = position.filter(|(x, y)| x.is_finite() && y.is_finite()) {
        config.x = Some(x);
        config.y = Some(y);
    }
    tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

fn next_label(state: &Windows) -> String {
    let mut registry = state.0.lock().unwrap();
    registry.sequence += 1;
    format!("speedyf-{}", registry.sequence)
}

#[tauri::command]
pub async fn new_window(app: tauri::AppHandle) -> Result<(), String> {
    let label = next_label(&app.state::<Windows>());
    build_window(&app, &label, None, true)?;
    Ok(())
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
                if let Err(error) = new_window(app).await {
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

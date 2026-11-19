//! What makes SpeedyF behave like a desktop application rather than a web
//! page in a frame: the Settings window, the document state the title bar
//! shows on macOS (the edited dot in the close button and the proxy icon),
//! the system's list of recent documents (the Dock menu on macOS), and
//! showing a file in Finder or Explorer.
//!
//! The menus themselves are built by the frontend from its command registry
//! (src/app/appMenu.ts); Rust only installs a bare menu for the moment before
//! the first window has loaded.
use crate::windows::{reveal_eventually, SETTINGS_LABEL};
use std::path::Path;
use tauri::Manager;

/// Opens the Settings window, or raises it when it is already open: there is
/// only ever one.
#[tauri::command]
pub async fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        SETTINGS_LABEL,
        tauri::WebviewUrl::App("index.html?window=settings".into()),
    )
    .title("Settings")
    .inner_size(640.0, 720.0)
    .min_inner_size(520.0, 420.0)
    .minimizable(false)
    .maximizable(false)
    .center()
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    reveal_eventually(&app, SETTINGS_LABEL);
    Ok(())
}

/// What the title bar says about the window's active document: whether it
/// has unsaved changes, and which file it is. On macOS that is the dot in
/// the close button and the proxy icon beside the title (Cmd-click it for
/// the folder path). The other platforms have neither; their title carries
/// the state instead, which the frontend sets.
#[tauri::command]
pub fn set_document_state(
    window: tauri::WebviewWindow,
    title: String,
    edited: bool,
    path: Option<String>,
) {
    #[cfg(target_os = "macos")]
    macos::document_state(&window, title, edited, path);
    #[cfg(not(target_os = "macos"))]
    let _ = (window, title, edited, path);
}

/// Puts a document window's traffic lights back where tauri.conf.json asks.
/// AppKit lays the title bar out afresh when the title, the proxy icon or the
/// edited dot change, or the window resizes or leaves full screen, and that
/// returns the buttons to their stock place: higher and further left than the
/// tab strip they are centred on. Tauri only reapplies its position when its
/// own view redraws, which the webview covering it never lets happen.
#[cfg(target_os = "macos")]
pub fn place_traffic_lights(window: &tauri::Window) {
    if !crate::windows::is_document_window(window.label()) {
        return;
    }
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(pointer) = target.ns_window() else {
            return;
        };
        // SAFETY: on the main thread; `ns_window` is this window's live
        // NSWindow, or null.
        if let Some(ns_window) = unsafe { (pointer as *const objc2::runtime::AnyObject).as_ref() } {
            macos::place_traffic_lights(&target.app_handle().clone(), ns_window);
        }
    });
}

/// Adds `path` to the system's recent documents: the Dock menu's list on
/// macOS. Nothing elsewhere, where the in-app Recents list is the only one.
#[tauri::command]
pub fn note_recent_document(app: tauri::AppHandle, path: String) {
    #[cfg(target_os = "macos")]
    if Path::new(&path).is_absolute() {
        macos::note_recent(&app, path);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, path);
}

/// Empties the system's recent documents, with the in-app list.
#[tauri::command]
pub fn clear_recent_documents(app: tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    macos::clear_recents(&app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Shows `path` selected in Finder or Explorer (its folder, on Linux, where
/// there is no common way to select a file).
#[tauri::command]
pub async fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let path = Path::new(&path);
    if !path.is_absolute() || !path.exists() {
        return Err("That file is no longer there.".into());
    }
    reveal(path).map_err(|e| format!("Could not show the file: {e}"))
}

#[cfg(target_os = "macos")]
fn reveal(path: &Path) -> std::io::Result<()> {
    spawn(std::process::Command::new("open").arg("-R").arg(path))
}

#[cfg(target_os = "windows")]
fn reveal(path: &Path) -> std::io::Result<()> {
    // explorer takes "/select," and the path as one argument.
    let mut select = std::ffi::OsString::from("/select,");
    select.push(path);
    spawn(std::process::Command::new("explorer").arg(select))
}

#[cfg(target_os = "linux")]
fn reveal(path: &Path) -> std::io::Result<()> {
    let folder = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    spawn(std::process::Command::new("xdg-open").arg(folder))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn reveal(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "not supported on this platform",
    ))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn spawn(command: &mut std::process::Command) -> std::io::Result<()> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// The bare menu bar shown until the first window has built its own from the
/// command registry: the standard macOS menus, About filled in from the
/// bundle, plus New Window. Windows and Linux have no menu bar at all - the
/// toolbar and the command palette are the app's menus there.
#[cfg(target_os = "macos")]
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
                if let Err(error) = crate::windows::new_window(app, None).await {
                    log::error!("new window: {error}");
                }
            });
        }
    });
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSRect, NSString, NSURL};
    use tauri::Manager;

    pub fn document_state(
        window: &tauri::WebviewWindow,
        title: String,
        edited: bool,
        path: Option<String>,
    ) {
        let target = window.clone();
        let _ = window.run_on_main_thread(move || {
            let Ok(pointer) = target.ns_window() else {
                return;
            };
            // SAFETY: on the main thread; `ns_window` is this window's live
            // NSWindow, or null.
            let Some(ns_window) = (unsafe { (pointer as *const AnyObject).as_ref() }) else {
                return;
            };
            // An empty name clears the proxy icon (the Home screen).
            let name = NSString::from_str(path.as_deref().unwrap_or(""));
            let title = NSString::from_str(&title);
            // SAFETY: plain NSWindow setters taking an NSString or a BOOL.
            unsafe {
                let _: () = msg_send![ns_window, setTitle: &*title];
                let _: () = msg_send![ns_window, setDocumentEdited: edited];
                let _: () = msg_send![ns_window, setRepresentedFilename: &*name];
            }
            // Each of those lays the title bar out again.
            if crate::windows::is_document_window(target.label()) {
                place_traffic_lights(target.app_handle(), ns_window);
            }
        });
    }

    /// The move Tauri makes for `trafficLightPosition`: the title bar's
    /// container grows to the buttons' height plus `y`, so the buttons, which
    /// keep their offset from its bottom, come down with it; each button then
    /// starts `x` in, at the system's spacing.
    pub fn place_traffic_lights(app: &tauri::AppHandle, ns_window: &AnyObject) {
        let Some(position) = app
            .config()
            .app
            .windows
            .iter()
            .find(|w| w.label == "main")
            .and_then(|w| w.traffic_light_position.as_ref())
            .map(|p| (p.x, p.y))
        else {
            return;
        };
        let (x, y) = position;
        // SAFETY: on the main thread with a live NSWindow. The standard
        // buttons exist on every titled window (null otherwise, checked), and
        // their superview's superview is the title bar container AppKit
        // lays them out in.
        unsafe {
            let button = |kind: usize| -> Option<&AnyObject> {
                let b: *mut AnyObject = msg_send![ns_window, standardWindowButton: kind];
                b.as_ref()
            };
            let (Some(close), Some(mini), Some(zoom)) = (button(0), button(1), button(2)) else {
                return;
            };
            let parent: *mut AnyObject = msg_send![close, superview];
            let Some(parent) = parent.as_ref() else {
                return;
            };
            let container: *mut AnyObject = msg_send![parent, superview];
            let Some(container) = container.as_ref() else {
                return;
            };
            let close_rect: NSRect = msg_send![close, frame];
            let window_rect: NSRect = msg_send![ns_window, frame];
            let height = close_rect.size.height + y;
            let mut bar: NSRect = msg_send![container, frame];
            bar.size.height = height;
            bar.origin.y = window_rect.size.height - height;
            let _: () = msg_send![container, setFrame: bar];
            let mini_rect: NSRect = msg_send![mini, frame];
            let spacing = mini_rect.origin.x - close_rect.origin.x;
            for (i, b) in [close, mini, zoom].into_iter().enumerate() {
                let mut rect: NSRect = msg_send![b, frame];
                rect.origin.x = x + i as f64 * spacing;
                let _: () = msg_send![b, setFrameOrigin: rect.origin];
            }
        }
    }

    fn document_controller() -> Option<&'static AnyObject> {
        let class = AnyClass::get(c"NSDocumentController")?;
        // SAFETY: the shared controller is a process-wide singleton.
        let controller: *mut AnyObject = unsafe { msg_send![class, sharedDocumentController] };
        // SAFETY: AppKit returns the live singleton, never released.
        unsafe { controller.as_ref() }
    }

    pub fn note_recent(app: &tauri::AppHandle, path: String) {
        let _ = app.run_on_main_thread(move || {
            let Some(controller) = document_controller() else {
                return;
            };
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
            // SAFETY: noteNewRecentDocumentURL: takes one NSURL.
            unsafe {
                let _: () = msg_send![controller, noteNewRecentDocumentURL: &*url];
            }
        });
    }

    pub fn clear_recents(app: &tauri::AppHandle) {
        let _ = app.run_on_main_thread(|| {
            let Some(controller) = document_controller() else {
                return;
            };
            let nothing: *mut AnyObject = std::ptr::null_mut();
            // SAFETY: clearRecentDocuments: is an action taking a sender.
            unsafe {
                let _: () = msg_send![controller, clearRecentDocuments: nothing];
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revealing_a_missing_or_relative_path_is_refused_before_any_process_starts() {
        let refused = tauri::async_runtime::block_on(reveal_in_file_manager(
            "/definitely/not/here/paper.pdf".into(),
        ));
        assert!(refused.is_err());
        let relative = tauri::async_runtime::block_on(reveal_in_file_manager("paper.pdf".into()));
        assert!(relative.is_err());
    }
}

//! Window chrome. On macOS the title bar is folded into the tab strip
//! (`titleBarStyle: Overlay` in tauri.conf.json), so the webview receives
//! the clicks on its empty space and has to do what the system title bar
//! would have done with a double-click. Windows and Linux keep their native
//! title bars, which handle that themselves.

/// What a double-click on a title bar does.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum DoubleClickAction {
    Zoom,
    Minimize,
    Nothing,
}

/// The user's choice in System Settings > Desktop & Dock ("Double-click a
/// window's title bar to"), as the global defaults record it. Older systems
/// only kept a minimize flag; anything unrecognised (such as "Fill") zooms.
#[cfg(any(target_os = "macos", test))]
fn parse_action(action: Option<&str>, legacy_minimize: bool) -> DoubleClickAction {
    match action {
        Some("Minimize") => DoubleClickAction::Minimize,
        Some("None") => DoubleClickAction::Nothing,
        Some(_) => DoubleClickAction::Zoom,
        None if legacy_minimize => DoubleClickAction::Minimize,
        None => DoubleClickAction::Zoom,
    }
}

#[cfg(target_os = "macos")]
fn system_action() -> DoubleClickAction {
    use objc2_foundation::{NSString, NSUserDefaults};
    let defaults = NSUserDefaults::standardUserDefaults();
    let action = defaults
        .stringForKey(&NSString::from_str("AppleActionOnDoubleClick"))
        .map(|s| s.to_string());
    let legacy = defaults.boolForKey(&NSString::from_str("AppleMiniaturizeOnDoubleClick"));
    parse_action(action.as_deref(), legacy)
}

#[cfg(not(target_os = "macos"))]
fn system_action() -> DoubleClickAction {
    DoubleClickAction::Zoom
}

/// A double-click on empty title-bar space (the tab strip or toolbar).
#[tauri::command]
pub fn titlebar_double_click(window: tauri::WebviewWindow) -> Result<(), String> {
    let result = match system_action() {
        DoubleClickAction::Zoom => {
            if window.is_maximized().map_err(|e| e.to_string())? {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        DoubleClickAction::Minimize => window.minimize(),
        DoubleClickAction::Nothing => Ok(()),
    };
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_system_setting() {
        assert_eq!(
            parse_action(Some("Maximize"), false),
            DoubleClickAction::Zoom
        );
        assert_eq!(parse_action(Some("Fill"), false), DoubleClickAction::Zoom);
        assert_eq!(
            parse_action(Some("Minimize"), false),
            DoubleClickAction::Minimize
        );
        assert_eq!(parse_action(Some("None"), true), DoubleClickAction::Nothing);
    }

    #[test]
    fn falls_back_to_the_legacy_flag_then_zoom() {
        assert_eq!(parse_action(None, true), DoubleClickAction::Minimize);
        assert_eq!(parse_action(None, false), DoubleClickAction::Zoom);
    }
}

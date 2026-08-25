//! Extensions that ship inside the application, rather than being installed.
//!
//! A built-in is read from the app bundle and never copied into the user's
//! extensions directory. That is the whole design, and everything else here
//! follows from it: updates arrive with the app instead of needing a version
//! compare against a stale copy, the files cannot be edited out from under the
//! signature, and `installed.json` stays a record of what the *user* installed.
//!
//! The one thing a built-in needs to persist is whether it has been switched
//! off, which cannot live in the read-only bundle. That goes in a small file
//! beside the registry. Absence means enabled, so a built-in that is removed
//! from a later release leaves nothing behind but a harmless stale id.
//!
//! Search order mirrors `pdfium_init`, for the same reason: the bundle's
//! resource directory in production, the repository checkout under
//! `cargo tauri dev`, and executable-adjacent paths for anything unusual.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{is_valid_id, MANIFEST_FILE};
use crate::errors::{AppError, AppResult};

/// Directory holding built-ins, both at the repository root and inside the
/// bundle's resources. Deliberately the same name as the user's extensions
/// directory: the two are the same shape, and only their writability differs.
pub const BUILTIN_DIR: &str = "extensions";

/// Where the disabled list is kept, inside the *user's* extensions directory.
pub const STATE_FILE: &str = "builtins.json";

pub const STATE_VERSION: u32 = 1;

/// Directories that may hold built-ins, most specific first.
pub fn candidate_roots(resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(env_path) = std::env::var("SPEEDYF_BUILTIN_EXTENSIONS") {
        dirs.push(PathBuf::from(env_path));
    }
    if let Some(resources) = resource_dir {
        dirs.push(resources.join(BUILTIN_DIR));
    }
    // Development layout: this file is `src-tauri/src/extensions/`, and the
    // built-ins sit at the repository root next to `src-tauri`.
    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(BUILTIN_DIR),
    );
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join(BUILTIN_DIR));
            // macOS bundle: Contents/MacOS/<exe> → Contents/Resources/extensions
            dirs.push(dir.join("../Resources").join(BUILTIN_DIR));
        }
    }
    dirs
}

/// The first candidate that exists, or `None` when the app ships no built-ins.
///
/// `None` is an ordinary outcome, not a failure: a build with the resource
/// omitted still runs, with user-installed extensions only.
pub fn root(resource_dir: Option<&Path>) -> Option<PathBuf> {
    candidate_roots(resource_dir)
        .into_iter()
        .find(|d| d.is_dir())
}

/// Ids of the built-ins present in `root`, sorted.
///
/// A directory counts when its name is a usable id and it carries a manifest
/// claiming that same id. The directory name *is* the id everywhere else —
/// `resolve_asset` builds `<root>/<id>/…` and installs land in `<root>/<id>` —
/// so a folder named for the extension rather than for its id resolves to
/// nothing at all.
///
/// That mistake is loud on purpose. The first version of this shipped
/// `extensions/references/` for an extension whose id is
/// `dev.speedyf.references`, and because ids without a dot are rejected the
/// result was zero built-ins and not one word about it. A stray file or a
/// `.DS_Store` is still ignored silently — that is noise, not a mistake.
pub fn ids(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let dir = entry.path();
            let name = entry.file_name().to_str()?.to_owned();
            if !dir.join(MANIFEST_FILE).is_file() {
                return None;
            }
            if !is_valid_id(&name) {
                log::error!(
                    "built-in at {} is ignored: \"{name}\" is not a usable extension id — \
                     the directory must be named for the manifest's id",
                    dir.display()
                );
                return None;
            }
            if let Some(declared) = manifest_id(&dir) {
                if declared != name {
                    log::error!(
                        "built-in at {} is ignored: it declares id \"{declared}\" but sits in \
                         \"{name}\" — the two must match",
                        dir.display()
                    );
                    return None;
                }
            }
            Some(name)
        })
        .collect();
    out.sort();
    out
}

/// The id a directory's manifest claims, when it parses.
///
/// `None` covers both "no manifest" and "unreadable manifest". A manifest that
/// will not parse is still listed, so the panel can show the user why the
/// extension is not running rather than hiding it.
fn manifest_id(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    super::parse_manifest(&text).ok().map(|m| m.id)
}

/// Which built-ins the user has switched off.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            version: STATE_VERSION,
            disabled: Vec::new(),
        }
    }
}

impl State {
    /// Read the disabled list, falling back to empty.
    ///
    /// Same trade as the registry: an unreadable file means every built-in is
    /// enabled, which is the state the app ships in, rather than a startup
    /// failure over a file the user never asked to exist.
    pub fn load(user_root: &Path) -> State {
        let path = user_root.join(STATE_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return State::default();
        };
        match serde_json::from_str::<State>(&text) {
            Ok(state) if state.version == STATE_VERSION => state,
            Ok(state) => {
                log::warn!(
                    "built-in extension state is version {}, this build reads {} — ignoring it",
                    state.version,
                    STATE_VERSION
                );
                State::default()
            }
            Err(e) => {
                log::warn!("built-in extension state unreadable ({e}) — ignoring it");
                State::default()
            }
        }
    }

    pub fn save(&self, user_root: &Path) -> AppResult<()> {
        std::fs::create_dir_all(user_root)
            .map_err(|e| AppError::Io(format!("{}: {e}", user_root.display())))?;
        let path = user_root.join(STATE_FILE);
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Internal(format!("built-in state: {e}")))?;
        std::fs::write(&path, text).map_err(|e| AppError::Io(format!("{}: {e}", path.display())))
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        !self.disabled.iter().any(|d| d == id)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.disabled.retain(|d| d != id);
        } else if self.is_enabled(id) {
            self.disabled.push(id.to_string());
            self.disabled.sort();
        }
    }
}

/// Record that a built-in is on or off.
pub fn set_enabled(user_root: &Path, id: &str, enabled: bool) -> AppResult<()> {
    let mut state = State::load(user_root);
    state.set_enabled(id, enabled);
    state.save(user_root)
}

/// Delete user installs that a built-in has taken over.
///
/// Only reachable for someone who hand-installed an extension that a later
/// release began shipping — which is exactly how References arrived. Both
/// copies would otherwise be listed under one id, and the id is what commands,
/// storage and asset URLs are keyed by, so two rows claiming it is not a state
/// worth supporting. The built-in wins because it is at least as new, and
/// nothing is lost: settings are keyed by id, not by install.
///
/// Returns the ids it removed, for the caller to log.
pub fn prune_shadowed(user_root: &Path, builtin_ids: &[String]) -> Vec<String> {
    let registry = super::Registry::load(user_root);
    let shadowed: Vec<String> = registry
        .extensions
        .iter()
        .filter(|e| builtin_ids.contains(&e.id))
        .map(|e| e.id.clone())
        .collect();
    shadowed
        .into_iter()
        .filter(|id| super::uninstall(user_root, id).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plant(root: &Path, id: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                r#"{{"apiVersion":1,"id":"{id}","name":"Planted","version":"1.0.0","entry":"worker.js"}}"#
            ),
        )
        .unwrap();
        std::fs::write(dir.join("worker.js"), "// planted").unwrap();
    }

    #[test]
    fn ids_lists_only_manifested_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        plant(root, "speedyf.references");
        plant(root, "speedyf.notes");
        // A directory with no manifest, a file, and a name that is not a usable
        // id are all things a bundle can pick up; none of them is an extension.
        std::fs::create_dir_all(root.join("speedyf.empty")).unwrap();
        std::fs::write(root.join(".DS_Store"), "junk").unwrap();
        std::fs::create_dir_all(root.join("Not An Id")).unwrap();
        std::fs::write(root.join("Not An Id").join(MANIFEST_FILE), "{}").unwrap();

        assert_eq!(
            ids(root),
            vec![
                "speedyf.notes".to_string(),
                "speedyf.references".to_string()
            ]
        );
    }

    /// The bug this file shipped with for exactly one build.
    ///
    /// `extensions/references/` looked entirely reasonable and discovered
    /// nothing, because ids must contain a dot and the directory name is the
    /// id. Nothing failed; the app simply had no built-ins.
    #[test]
    fn a_directory_not_named_for_its_id_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        plant(root, "dev.speedyf.references");
        // Same extension, filed under a friendly name instead of its id.
        std::fs::create_dir_all(root.join("references")).unwrap();
        std::fs::copy(
            root.join("dev.speedyf.references").join(MANIFEST_FILE),
            root.join("references").join(MANIFEST_FILE),
        )
        .unwrap();

        assert_eq!(ids(root), vec!["dev.speedyf.references".to_string()]);
    }

    #[test]
    fn a_manifest_claiming_another_id_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        plant(root, "dev.speedyf.references");
        // The directory is a usable id, but not the one inside.
        std::fs::create_dir_all(root.join("dev.speedyf.imposter")).unwrap();
        std::fs::copy(
            root.join("dev.speedyf.references").join(MANIFEST_FILE),
            root.join("dev.speedyf.imposter").join(MANIFEST_FILE),
        )
        .unwrap();

        assert_eq!(ids(root), vec!["dev.speedyf.references".to_string()]);
    }

    /// A built-in whose manifest will not parse still has to be listed: the
    /// panel is the only place its error can be reported.
    #[test]
    fn an_unparseable_manifest_is_still_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("dev.speedyf.broken")).unwrap();
        std::fs::write(root.join("dev.speedyf.broken").join(MANIFEST_FILE), "{oops").unwrap();
        assert_eq!(ids(root), vec!["dev.speedyf.broken".to_string()]);
    }

    /// The real thing, as it ships. Guards the packaging, not the logic: a
    /// rename that forgets `tauri.conf.json` or the directory contract fails
    /// here instead of at launch.
    #[test]
    fn the_shipped_built_ins_are_discoverable() {
        let checked_out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(BUILTIN_DIR);
        assert_eq!(
            ids(&checked_out),
            vec!["dev.speedyf.references".to_string()],
            "the extensions/ folder does not contain what this build expects to ship"
        );
    }

    #[test]
    fn ids_of_a_missing_root_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(ids(&tmp.path().join("nothing-here")).is_empty());
    }

    #[test]
    fn root_prefers_the_resource_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let resources = tmp.path().join("Resources");
        std::fs::create_dir_all(resources.join(BUILTIN_DIR)).unwrap();
        assert_eq!(
            root(Some(&resources)),
            Some(resources.join(BUILTIN_DIR)),
            "the bundle's copy must win over the checkout's"
        );
    }

    #[test]
    fn a_built_in_is_enabled_until_it_is_switched_off() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Nothing has been written yet: absence means enabled, which is the
        // state the app ships in.
        assert!(State::load(root).is_enabled("speedyf.references"));

        set_enabled(root, "speedyf.references", false).unwrap();
        assert!(!State::load(root).is_enabled("speedyf.references"));
        assert!(State::load(root).is_enabled("speedyf.other"));

        set_enabled(root, "speedyf.references", true).unwrap();
        assert!(State::load(root).is_enabled("speedyf.references"));
    }

    #[test]
    fn disabling_twice_records_one_entry() {
        let mut state = State::default();
        state.set_enabled("speedyf.references", false);
        state.set_enabled("speedyf.references", false);
        assert_eq!(state.disabled, vec!["speedyf.references".to_string()]);
    }

    #[test]
    fn a_future_state_file_is_ignored_rather_than_obeyed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(STATE_FILE),
            r#"{"version":99,"disabled":["speedyf.references"]}"#,
        )
        .unwrap();
        // Falling back to "everything on" is the shipped state; refusing to
        // start over an unreadable preferences file would be the worse trade.
        assert!(State::load(tmp.path()).is_enabled("speedyf.references"));
    }

    #[test]
    fn an_unreadable_state_file_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(STATE_FILE), "{not json").unwrap();
        assert!(State::load(tmp.path()).is_enabled("speedyf.references"));
    }

    #[test]
    fn a_hand_installed_copy_of_a_built_in_is_removed() {
        let user_root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        plant(source.path(), "speedyf.references");
        plant(source.path(), "com.example.other");
        super::super::install_from_folder(
            user_root.path(),
            &source.path().join("speedyf.references"),
        )
        .unwrap();
        super::super::install_from_folder(
            user_root.path(),
            &source.path().join("com.example.other"),
        )
        .unwrap();

        let removed = prune_shadowed(user_root.path(), &["speedyf.references".to_string()]);

        assert_eq!(removed, vec!["speedyf.references".to_string()]);
        assert!(!user_root.path().join("speedyf.references").exists());
        let left = super::super::Registry::load(user_root.path());
        assert_eq!(left.extensions.len(), 1);
        assert_eq!(left.extensions[0].id, "com.example.other");
    }

    #[test]
    fn pruning_with_nothing_shadowed_changes_nothing() {
        let user_root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        plant(source.path(), "com.example.other");
        super::super::install_from_folder(
            user_root.path(),
            &source.path().join("com.example.other"),
        )
        .unwrap();

        assert!(prune_shadowed(user_root.path(), &["speedyf.references".to_string()]).is_empty());
        assert!(user_root.path().join("com.example.other").exists());
    }
}

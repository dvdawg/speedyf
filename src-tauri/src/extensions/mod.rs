//! Extension hosting: where installed extensions live on disk, and how their
//! files reach the webview.
//!
//! Extension code never runs in the app document. The app's CSP is
//! `script-src 'self'`, which exists so a hostile PDF cannot inject script,
//! and it stays that way. Extensions run in Web Workers instead — a separate
//! JavaScript environment with no DOM and no access to app state, whose only
//! channel out is `postMessage`. Isolation is therefore a property of where
//! the code runs rather than a rule anyone has to remember.
//!
//! A Worker still needs somewhere to load from, and the obvious route does not
//! work: `new Worker(url)` requires the URL to be same-origin with the creating
//! document, which is a platform rule no CSP setting relaxes. A custom scheme
//! is by definition a different origin, so serving worker code from
//! `speedyf-ext://` fails with an opaque error. This was measured, not assumed.
//!
//! So worker source is read here and handed to the frontend, which wraps it in
//! a blob URL — blob URLs inherit the creating document's origin, satisfying
//! the rule. Two consequences worth knowing:
//!
//! - An extension's entry must be self-contained. Relative `importScripts`
//!   cannot resolve from a blob, so extensions ship a bundle.
//! - The worker inherits the app's CSP for its own subresource loads, and
//!   `connect-src` is `ipc:` only. An extension therefore cannot reach the
//!   network at all in v1 — denied by the platform rather than by policy.
//!
//! `resolve_asset` and the `speedyf-ext://` scheme it backs are still here and
//! still used: sandboxed pane documents (v3) are iframes, and iframes are
//! allowed to be cross-origin. Only workers need the blob route.

use crate::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Directory under the app data dir holding every installed extension.
pub const EXTENSIONS_DIR: &str = "extensions";

/// Resolve a request path like `/com.example.thing/worker.js` to a file inside
/// the extensions directory.
///
/// Returns `None` for anything that is not a plain `<id>/<relative path>` under
/// the root. The rejection is structural rather than a string scan: the path is
/// walked component by component and only ordinary names are accepted, so `..`,
/// absolute segments, root prefixes and empty components are all refused before
/// a `PathBuf` is built. A URL-encoded `..` is decoded by the caller and lands
/// here as a `ParentDir` component, so it is refused on the same branch.
pub fn resolve_asset(root: &Path, request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let (id, rest) = trimmed.split_once('/')?;
    if !is_valid_id(id) || rest.is_empty() {
        return None;
    }

    let mut out = root.join(id);
    let mut depth = 0usize;
    for component in Path::new(rest).components() {
        match component {
            Component::Normal(part) => {
                // A component that is not valid UTF-8 cannot have come from a
                // URL path, so treating it as hostile costs nothing.
                let part = part.to_str()?;
                if part.is_empty() {
                    return None;
                }
                out.push(part);
                depth += 1;
            }
            _ => return None,
        }
    }
    (depth > 0).then_some(out)
}

/// Whether a string is usable as an extension id.
///
/// Reverse-DNS, and deliberately narrow: the id is a directory name, a URL path
/// segment, a command namespace and a storage namespace all at once, so the
/// character set is the intersection of what all of those handle without
/// escaping. Case is fixed to lower so two ids cannot collide on a
/// case-insensitive filesystem while looking distinct in the UI.
pub fn is_valid_id(id: &str) -> bool {
    if id.len() < 3 || id.len() > 128 {
        return false;
    }
    if id.starts_with('.') || id.ends_with('.') || id.contains("..") {
        return false;
    }
    let mut has_dot = false;
    for ch in id.chars() {
        match ch {
            '.' => has_dot = true,
            'a'..='z' | '0'..='9' | '-' | '_' => {}
            _ => return false,
        }
    }
    has_dot
}

/// Content type for an extension asset. Deliberately a short allowlist: these
/// are the only kinds of file a v1 extension has any use for, and serving an
/// unknown type as `application/octet-stream` is a better default than guessing.
pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("js") | Some("mjs") => "text/javascript",
        Some("json") => "application/json",
        Some("css") => "text/css",
        Some("html") => "text/html",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

// ---------- manifest ----------

/// The API generation an extension is written against. Bumped only when a
/// change would break extensions already in the wild; the host refuses
/// anything it does not recognise rather than guessing.
pub const CURRENT_API_VERSION: u32 = 1;

/// A capability an extension asks for in its manifest.
///
/// v1 gates almost nothing — there is nothing yet to gate, since commands are
/// the only contribution point. The enum exists anyway, and the gate is
/// enforced from the first release, because retrofitting permissions onto
/// extensions that already had unrestricted access is a breaking change for
/// every one of them.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permission {
    #[serde(rename = "document:read")]
    DocumentRead,
    #[serde(rename = "citations:read")]
    CitationsRead,
    #[serde(rename = "library:read")]
    LibraryRead,
    #[serde(rename = "clipboard:write")]
    ClipboardWrite,
    #[serde(rename = "storage")]
    Storage,
    #[serde(rename = "commands:run")]
    CommandsRun,
}

/// An action offered on something the user picked — a text selection, a
/// citation. It names one of the extension's own commands, which is invoked
/// with the picked thing as its value.
///
/// Deliberately needs no permission. The extension is handed exactly what the
/// user chose and nothing else, which is the whole point: a translate or
/// summarise extension should never need `document:read`, and the narrow path
/// only stays narrow if it is genuinely usable.
/// One setting an extension exposes.
///
/// Declared rather than drawn, like everything else: the extension says what it
/// wants to ask, and SpeedyF renders the control. That keeps a settings form
/// looking like the rest of the app, and means an extension cannot put
/// arbitrary UI in the Extensions panel.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingContribution {
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: SettingKind,
    #[serde(default)]
    pub default: Option<String>,
    /// Choices for a `select`; ignored otherwise.
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    Text,
    Checkbox,
    Select,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionContribution {
    /// One of this extension's declared command ids.
    pub command: String,
    pub title: String,
}

/// One command an extension contributes to the palette.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandContribution {
    /// Unique within the extension. Namespaced with the extension id before it
    /// reaches the palette, so two extensions can both contribute `export`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// A key an extension asks to bind to one of its own commands.
///
/// Bindings point at the extension's own commands, never at SpeedyF's. An
/// extension that wants to drive the app binds a key to its own command and
/// calls `commands.run` from there — which keeps every such call inside the
/// permission gate rather than letting a manifest entry bypass it.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeybindingContribution {
    /// One of this extension's declared command ids.
    pub command: String,
    /// A neutral chord spec: `j`, `mod+shift+p`, `alt+ArrowDown`.
    pub key: String,
}

/// A sidebar tab an extension contributes.
///
/// Declarative only: the extension sends a description of rows and SpeedyF
/// draws them. There is no `kind` field yet — sandboxed custom panes are v3,
/// and adding the discriminator before there is a second variant would be
/// guessing at its shape.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelContribution {
    /// Unique within the extension; namespaced before it reaches the sidebar.
    pub id: String,
    pub title: String,
}

/// Deliberately strict on both counts.
///
/// `camelCase` because that is what the rest of the manifest uses, and an
/// author should not have to learn which fields are snake_case.
///
/// `deny_unknown_fields` because the alternative is what this cost an
/// afternoon to find: a manifest that says `selectionActions` while serde
/// expects `selection_actions` parses *successfully*, defaults the field to
/// empty, and produces an extension that installs cleanly and silently does
/// nothing. A typo in a contribution point must be an install-time error, the
/// same as a keybinding naming a command that does not exist.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub keybindings: Vec<KeybindingContribution>,
    #[serde(default)]
    pub panels: Vec<PanelContribution>,
    #[serde(default)]
    pub selection_actions: Vec<ActionContribution>,
    #[serde(default)]
    pub citation_actions: Vec<ActionContribution>,
    #[serde(default)]
    pub settings: Vec<SettingContribution>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub api_version: u32,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub contributes: Contributions,
}

/// Why a manifest was rejected. Every variant names the offending field, so the
/// Extensions panel can say what to fix rather than "invalid extension".
#[derive(Debug, PartialEq, Eq)]
pub enum ManifestError {
    Syntax(String),
    BadId(String),
    UnsupportedApiVersion { found: u32, supported: u32 },
    MissingField(&'static str),
    BadEntry(String),
    DuplicateCommand(String),
    BadCommandId(String),
    UnknownBoundCommand(String),
    BadSetting(String),
    DuplicatePanel(String),
    BadPanelId(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Syntax(e) => write!(f, "speedyf.json is not valid JSON: {e}"),
            ManifestError::BadId(id) => write!(
                f,
                "\"{id}\" is not a valid extension id (expected lowercase reverse-DNS, e.g. com.example.thing)"
            ),
            ManifestError::UnsupportedApiVersion { found, supported } => write!(
                f,
                "extension targets API version {found}; this build supports {supported}"
            ),
            ManifestError::MissingField(field) => write!(f, "{field} is required and must not be empty"),
            ManifestError::BadEntry(entry) => {
                write!(f, "entry \"{entry}\" must be a plain relative path inside the extension")
            }
            ManifestError::DuplicateCommand(id) => write!(f, "two commands share the id \"{id}\""),
            ManifestError::BadCommandId(id) => write!(
                f,
                "command id \"{id}\" must be non-empty and free of \"/\", which separates it from the extension id"
            ),
            ManifestError::BadSetting(key) => write!(
                f,
                "setting \"{key}\" needs a non-empty key and title, and a select needs options"
            ),
            ManifestError::DuplicatePanel(id) => write!(f, "two panels share the id \"{id}\""),
            ManifestError::BadPanelId(id) => write!(
                f,
                "panel id \"{id}\" must be non-empty and free of \"/\", which separates it from the extension id"
            ),
            ManifestError::UnknownBoundCommand(id) => write!(
                f,
                "keybinding points at \"{id}\", which this extension does not declare as a command"
            ),
        }
    }
}

/// Parse and validate a manifest.
///
/// Validation is total: a manifest that returns `Ok` here can be installed and
/// run without further checking, so nothing downstream has to re-litigate
/// whether an id is safe as a directory name or a command id is unambiguous.
pub fn parse_manifest(source: &str) -> Result<Manifest, ManifestError> {
    let manifest: Manifest =
        serde_json::from_str(source).map_err(|e| ManifestError::Syntax(e.to_string()))?;

    if !is_valid_id(&manifest.id) {
        return Err(ManifestError::BadId(manifest.id));
    }
    if manifest.api_version != CURRENT_API_VERSION {
        return Err(ManifestError::UnsupportedApiVersion {
            found: manifest.api_version,
            supported: CURRENT_API_VERSION,
        });
    }
    if manifest.name.trim().is_empty() {
        return Err(ManifestError::MissingField("name"));
    }
    if manifest.version.trim().is_empty() {
        return Err(ManifestError::MissingField("version"));
    }
    // The entry is resolved against the extension's own directory, so it goes
    // through the same component walk that serves assets.
    if resolve_asset(
        Path::new("/root"),
        &format!("/{}/{}", manifest.id, manifest.entry),
    )
    .is_none()
    {
        return Err(ManifestError::BadEntry(manifest.entry));
    }

    let mut seen = std::collections::HashSet::new();
    for command in &manifest.contributes.commands {
        if command.id.trim().is_empty() || command.id.contains('/') {
            return Err(ManifestError::BadCommandId(command.id.clone()));
        }
        if !seen.insert(command.id.as_str()) {
            return Err(ManifestError::DuplicateCommand(command.id.clone()));
        }
    }
    // A binding naming a command that does not exist would simply never fire,
    // which is the kind of failure an author spends an afternoon on. Caught at
    // install instead, where the message can name the id.
    for binding in &manifest.contributes.keybindings {
        if !seen.contains(binding.command.as_str()) {
            return Err(ManifestError::UnknownBoundCommand(binding.command.clone()));
        }
    }
    // Same rule for actions: an action naming a command that does not exist
    // would appear in a menu and then do nothing.
    for action in manifest
        .contributes
        .selection_actions
        .iter()
        .chain(&manifest.contributes.citation_actions)
    {
        if !seen.contains(action.command.as_str()) {
            return Err(ManifestError::UnknownBoundCommand(action.command.clone()));
        }
        if action.title.trim().is_empty() {
            return Err(ManifestError::MissingField("action title"));
        }
    }
    let mut settings = std::collections::HashSet::new();
    for setting in &manifest.contributes.settings {
        let bad = setting.key.trim().is_empty()
            || setting.title.trim().is_empty()
            || (setting.kind == SettingKind::Select && setting.options.is_empty())
            || !settings.insert(setting.key.as_str());
        if bad {
            return Err(ManifestError::BadSetting(setting.key.clone()));
        }
    }

    let mut panels = std::collections::HashSet::new();
    for panel in &manifest.contributes.panels {
        if panel.id.trim().is_empty() || panel.id.contains('/') || panel.title.trim().is_empty() {
            return Err(ManifestError::BadPanelId(panel.id.clone()));
        }
        if !panels.insert(panel.id.as_str()) {
            return Err(ManifestError::DuplicatePanel(panel.id.clone()));
        }
    }
    Ok(manifest)
}

/// The palette-facing id for a contributed command: `<extension>/<command>`.
///
/// Namespacing exists from the first release on purpose. Retrofitting it later
/// renames every extension's commands and silently breaks every keybinding a
/// user has set against them.
pub fn qualified_command_id(extension: &str, command: &str) -> String {
    format!("{extension}/{command}")
}

// ---------- registry ----------

/// Schema version of `installed.json`.
///
/// Present from the first release deliberately. The library index shipped
/// without one and an unrecognised shape silently discarded the whole file;
/// both persisted library files carry versions now, and this one starts that
/// way rather than learning the same lesson twice.
pub const REGISTRY_VERSION: u32 = 1;

pub const REGISTRY_FILE: &str = "installed.json";
pub const MANIFEST_FILE: &str = "speedyf.json";

/// One installed extension as recorded on disk.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Where it came from, for the panel to show. v1 installs from a folder.
    pub source: String,
    /// Granted at install. Stored rather than re-read from the manifest so a
    /// self-update cannot quietly widen its own access: new permissions in a
    /// new manifest require asking again.
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub version: u32,
    #[serde(default)]
    pub extensions: Vec<InstalledExtension>,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            version: REGISTRY_VERSION,
            extensions: Vec::new(),
        }
    }
}

impl Registry {
    /// Read the registry, falling back to empty rather than failing.
    ///
    /// A corrupt or future-versioned file yields an empty registry: the
    /// extensions on disk are still there, and the user can reinstall. Refusing
    /// to start because a JSON file went bad would be a worse trade.
    pub fn load(root: &Path) -> Registry {
        let path = root.join(REGISTRY_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Registry::default();
        };
        match serde_json::from_str::<Registry>(&text) {
            Ok(registry) if registry.version == REGISTRY_VERSION => registry,
            Ok(registry) => {
                log::warn!(
                    "extension registry is version {}, this build reads {} — ignoring it",
                    registry.version,
                    REGISTRY_VERSION
                );
                Registry::default()
            }
            Err(e) => {
                log::warn!("extension registry unreadable ({e}) — ignoring it");
                Registry::default()
            }
        }
    }

    pub fn save(&self, root: &Path) -> AppResult<()> {
        std::fs::create_dir_all(root)
            .map_err(|e| AppError::Io(format!("{}: {e}", root.display())))?;
        let path = root.join(REGISTRY_FILE);
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Internal(format!("registry: {e}")))?;
        std::fs::write(&path, text).map_err(|e| AppError::Io(format!("{}: {e}", path.display())))
    }

    /// Insert or replace, keeping the list ordered by id so the panel and the
    /// file on disk are both stable across installs.
    pub fn upsert(&mut self, entry: InstalledExtension) {
        match self.extensions.iter_mut().find(|e| e.id == entry.id) {
            Some(slot) => *slot = entry,
            None => self.extensions.push(entry),
        }
        self.extensions.sort_by(|a, b| a.id.cmp(&b.id));
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.extensions.len();
        self.extensions.retain(|e| e.id != id);
        self.extensions.len() != before
    }
}

/// Copy an unpacked extension folder into the extensions directory.
///
/// The manifest is validated *before* anything is written, so a rejected
/// extension leaves no trace. An existing install of the same id is replaced;
/// its granted permissions are not carried over, because a new version may ask
/// for more than the user previously agreed to.
pub fn install_from_folder(root: &Path, source: &Path) -> AppResult<InstalledExtension> {
    let manifest_path = source.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| AppError::Io(format!("{}: {e}", manifest_path.display())))?;
    let manifest = parse_manifest(&text).map_err(|e| AppError::Malformed(e.to_string()))?;

    let entry_path = source.join(&manifest.entry);
    if !entry_path.is_file() {
        return Err(AppError::Malformed(format!(
            "entry \"{}\" does not exist in the extension folder",
            manifest.entry
        )));
    }

    let dest = root.join(&manifest.id);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| AppError::Io(format!("{}: {e}", dest.display())))?;
    }
    copy_tree(source, &dest)?;

    let entry = InstalledExtension {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        source: source.display().to_string(),
        permissions: manifest.permissions.clone(),
        enabled: true,
    };
    let mut registry = Registry::load(root);
    registry.upsert(entry.clone());
    registry.save(root)?;
    Ok(entry)
}

/// Remove an extension's files and its registry row.
///
/// The registry row goes first: a half-deleted directory that is no longer
/// listed is inert, where a listed extension whose files are gone would be a
/// broken row the user cannot clear.
pub fn uninstall(root: &Path, id: &str) -> AppResult<()> {
    if !is_valid_id(id) {
        return Err(AppError::Malformed(format!("bad extension id \"{id}\"")));
    }
    let mut registry = Registry::load(root);
    registry.remove(id);
    registry.save(root)?;

    let dir = root.join(id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| AppError::Io(format!("{}: {e}", dir.display())))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> AppResult<()> {
    std::fs::create_dir_all(to).map_err(|e| AppError::Io(format!("{}: {e}", to.display())))?;
    let entries =
        std::fs::read_dir(from).map_err(|e| AppError::Io(format!("{}: {e}", from.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| AppError::Io(e.to_string()))?;
        let name = entry.file_name();
        // Skip dotfiles: an extension developed in place carries .git, and
        // copying it in would be both large and pointless.
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let target = to.join(&name);
        let kind = entry
            .file_type()
            .map_err(|e| AppError::Io(format!("{}: {e}", entry.path().display())))?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| AppError::Io(format!("{}: {e}", target.display())))?;
        }
        // Symlinks are skipped rather than followed: an extension folder should
        // not be able to pull in files from outside itself.
    }
    Ok(())
}

// ---------- installing from a git URL ----------

/// Schemes a clone URL may use.
///
/// `file://` and bare local paths are excluded deliberately. Cloning a local
/// path is what `install_from_folder` already does, and allowing it here would
/// mean a URL could reach anywhere on disk. `ssh://` and `git@host:path` are
/// excluded because they can consult the user's keys and known_hosts, turning
/// "install this extension" into an action with their credentials.
const ALLOWED_SCHEMES: [&str; 2] = ["https://", "http://"];

/// Where a git-installed extension came from, and at what point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSource {
    pub url: String,
    /// Branch, tag or commit. `None` clones the default branch.
    pub reference: Option<String>,
}

/// Read `<url>` or `<url>#<ref>` into something safe to hand to git.
///
/// Validation is a whitelist rather than a blacklist, because the value ends up
/// as an argument to a subprocess. Even though the arguments are passed as a
/// vector — never through a shell, so there is no quoting to escape — a URL
/// beginning with `-` would still be read by git as an *option* rather than a
/// URL, which is the one injection this interface actually has.
pub fn parse_git_source(input: &str) -> Result<GitSource, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("no URL given".into());
    }
    let (url, reference) = match trimmed.split_once('#') {
        Some((url, reference)) => (url, Some(reference)),
        None => (trimmed, None),
    };
    if !ALLOWED_SCHEMES.iter().any(|s| url.starts_with(s)) {
        return Err(format!(
            "only {} URLs can be installed from",
            ALLOWED_SCHEMES.join(" and ")
        ));
    }
    // Belt and braces: a scheme prefix already rules this out, but the check is
    // cheap and the consequence of being wrong is git reading a URL as a flag.
    if url.starts_with('-') {
        return Err("a URL cannot start with \"-\"".into());
    }
    if url.contains(char::is_whitespace) || url.contains('\0') {
        return Err("a URL cannot contain whitespace".into());
    }
    if url.len() > 2048 {
        return Err("URL is too long".into());
    }

    let reference = match reference {
        None => None,
        Some(reference) => {
            let reference = reference.trim();
            if reference.is_empty() {
                return Err("the ref after \"#\" is empty".into());
            }
            // Same reasoning as the URL: a ref is an argument to git, and one
            // beginning with `-` is an option.
            if reference.starts_with('-') {
                return Err("a ref cannot start with \"-\"".into());
            }
            if !reference
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
            {
                return Err("a ref may only contain letters, digits, and . _ / -".into());
            }
            if reference.len() > 200 {
                return Err("ref is too long".into());
            }
            Some(reference.to_string())
        }
    };
    Ok(GitSource {
        url: url.to_string(),
        reference,
    })
}

/// One `git clone` attempt. Arguments are passed as a vector, never through a
/// shell, so there is nothing to quote or escape — and `--` stops git reading
/// anything after it as an option whatever `parse_git_source` may have missed.
fn run_clone(source: &GitSource, target: &Path, shallow: bool) -> AppResult<std::process::Output> {
    let mut args: Vec<String> = vec!["clone".into()];
    if shallow {
        args.push("--depth".into());
        args.push("1".into());
    }
    // A submodule is a second repository the user did not agree to fetch.
    args.push("--recurse-submodules=no".into());
    args.push("--quiet".into());
    if let Some(reference) = &source.reference {
        args.push("--branch".into());
        args.push(reference.clone());
    }
    args.push("--".into());
    args.push(source.url.clone());
    args.push(target.display().to_string());

    std::process::Command::new("git")
        .args(&args)
        // Never prompt: without this a private repository hangs the clone
        // forever on a credential prompt nobody can see.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::Unsupported(
                    "installing from a URL needs git, which was not found on this machine".into(),
                )
            } else {
                AppError::Io(format!("could not run git: {e}"))
            }
        })
}

/// Clone an extension and install it.
///
/// The clone lands in a temporary directory that is removed either way, so a
/// repository that turns out not to hold a valid extension leaves nothing
/// behind — the same promise `install_from_folder` already makes.
pub fn install_from_git(root: &Path, input: &str) -> AppResult<InstalledExtension> {
    let source = parse_git_source(input).map_err(AppError::Malformed)?;
    let temp = tempfile::tempdir().map_err(|e| AppError::Io(format!("temp dir: {e}")))?;
    let target = temp.path().join("clone");

    // Shallow first: extensions are source, not history, and a full clone of a
    // repository with a long past is wasted bandwidth. Not every remote can do
    // it though — the "dumb" HTTP transport cannot — so a shallow failure falls
    // back to a full clone rather than reporting an error the user cannot act
    // on. Every hosted forge supports shallow, so the fallback is rare.
    let output = match run_clone(&source, &target, true) {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if stderr.contains("shallow") {
                run_clone(&source, &target, false)?
            } else {
                output
            }
        }
        Err(e) => return Err(e),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().last().unwrap_or("git clone failed").trim();
        return Err(AppError::Io(format!("could not clone: {detail}")));
    }

    // A cloned repository is not trusted any differently from a folder: the
    // manifest is validated, and a bad one installs nothing.
    let mut installed = install_from_folder(root, &target)?;

    // Record where it came from rather than the temporary path, which will not
    // exist a moment from now.
    installed.source = match &source.reference {
        Some(reference) => format!("{}#{reference}", source.url),
        None => source.url.clone(),
    };
    let mut registry = Registry::load(root);
    registry.upsert(installed.clone());
    registry.save(root)?;
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/data/extensions")
    }

    #[test]
    fn resolves_a_plain_asset() {
        assert_eq!(
            resolve_asset(&root(), "/com.example.thing/worker.js"),
            Some(root().join("com.example.thing").join("worker.js"))
        );
    }

    #[test]
    fn resolves_a_nested_asset() {
        assert_eq!(
            resolve_asset(&root(), "/com.example.thing/lib/parse.js"),
            Some(
                root()
                    .join("com.example.thing")
                    .join("lib")
                    .join("parse.js")
            )
        );
    }

    #[test]
    fn refuses_traversal_out_of_the_extension() {
        for path in [
            "/com.example.thing/../other/worker.js",
            "/com.example.thing/lib/../../escape.js",
            "/com.example.thing/..",
            "/../etc/passwd",
        ] {
            assert_eq!(resolve_asset(&root(), path), None, "{path} was allowed");
        }
    }

    #[test]
    fn refuses_absolute_and_rooted_paths() {
        for path in [
            "/com.example.thing//etc/passwd",
            "/com.example.thing/./worker.js",
        ] {
            assert_eq!(resolve_asset(&root(), path), None, "{path} was allowed");
        }
    }

    #[test]
    fn refuses_a_bare_extension_directory() {
        assert_eq!(resolve_asset(&root(), "/com.example.thing"), None);
        assert_eq!(resolve_asset(&root(), "/com.example.thing/"), None);
    }

    #[test]
    fn refuses_an_empty_request() {
        assert_eq!(resolve_asset(&root(), ""), None);
        assert_eq!(resolve_asset(&root(), "/"), None);
    }

    #[test]
    fn refuses_an_invalid_id() {
        assert_eq!(resolve_asset(&root(), "/Not-Reverse-DNS/worker.js"), None);
        assert_eq!(resolve_asset(&root(), "/nodots/worker.js"), None);
    }

    #[test]
    fn ids_are_reverse_dns_and_lowercase() {
        assert!(is_valid_id("com.example.thing"));
        assert!(is_valid_id("dev.speedyf.vim-mode"));
        assert!(is_valid_id("a.b"));

        assert!(!is_valid_id("nodots"), "a namespace needs a dot");
        assert!(!is_valid_id("com.Example.Thing"), "case is fixed to lower");
        assert!(!is_valid_id(".com.example"), "no leading dot");
        assert!(!is_valid_id("com.example."), "no trailing dot");
        assert!(!is_valid_id("com..example"), "no empty segment");
        assert!(!is_valid_id("com.example/thing"), "no path separators");
        assert!(!is_valid_id("com example"), "no spaces");
        assert!(!is_valid_id("ab"), "too short");
    }

    #[test]
    fn worker_code_is_served_as_javascript() {
        assert_eq!(content_type(Path::new("worker.js")), "text/javascript");
        assert_eq!(content_type(Path::new("speedyf.json")), "application/json");
        assert_eq!(
            content_type(Path::new("notes.txt")),
            "application/octet-stream"
        );
    }

    // ---------- manifest ----------

    fn manifest_json(extra: &str) -> String {
        format!(
            r#"{{"id":"com.example.thing","apiVersion":1,"name":"Thing",
                 "version":"1.0.0","entry":"worker.js"{extra}}}"#
        )
    }

    #[test]
    fn accepts_a_minimal_manifest() {
        let m = parse_manifest(&manifest_json("")).expect("should parse");
        assert_eq!(m.id, "com.example.thing");
        assert!(m.permissions.is_empty());
        assert!(m.contributes.commands.is_empty());
    }

    #[test]
    fn reads_commands_and_permissions() {
        let m = parse_manifest(&manifest_json(
            r#","permissions":["citations:read"],
               "contributes":{"commands":[{"id":"export","title":"Export","keywords":["bib"]}]}"#,
        ))
        .expect("should parse");
        assert_eq!(m.permissions, vec![Permission::CitationsRead]);
        assert_eq!(m.contributes.commands[0].id, "export");
        assert_eq!(m.contributes.commands[0].keywords, vec!["bib"]);
    }

    #[test]
    fn rejects_an_unknown_permission() {
        let err = parse_manifest(&manifest_json(r#","permissions":["disk:format"]"#)).unwrap_err();
        assert!(matches!(err, ManifestError::Syntax(_)));
    }

    #[test]
    fn rejects_a_future_api_version() {
        let json = manifest_json("").replace("\"apiVersion\":1", "\"apiVersion\":2");
        assert_eq!(
            parse_manifest(&json).unwrap_err(),
            ManifestError::UnsupportedApiVersion {
                found: 2,
                supported: 1
            }
        );
    }

    #[test]
    fn rejects_a_bad_id() {
        let json = manifest_json("").replace("com.example.thing", "Not Valid");
        assert!(matches!(
            parse_manifest(&json).unwrap_err(),
            ManifestError::BadId(_)
        ));
    }

    #[test]
    fn rejects_an_entry_that_escapes_the_extension() {
        for bad in ["../../etc/passwd", "/etc/passwd", ".."] {
            let json = manifest_json("").replace("worker.js", bad);
            assert!(
                matches!(
                    parse_manifest(&json).unwrap_err(),
                    ManifestError::BadEntry(_)
                ),
                "{bad} was allowed as an entry"
            );
        }
    }

    #[test]
    fn rejects_duplicate_and_unqualified_command_ids() {
        let dup = parse_manifest(&manifest_json(
            r#","contributes":{"commands":[{"id":"go","title":"A"},{"id":"go","title":"B"}]}"#,
        ))
        .unwrap_err();
        assert_eq!(dup, ManifestError::DuplicateCommand("go".into()));

        let slash = parse_manifest(&manifest_json(
            r#","contributes":{"commands":[{"id":"a/b","title":"A"}]}"#,
        ))
        .unwrap_err();
        assert_eq!(slash, ManifestError::BadCommandId("a/b".into()));
    }

    #[test]
    fn reads_keybindings() {
        let m = parse_manifest(&manifest_json(
            r#","contributes":{"commands":[{"id":"down","title":"Down"}],
               "keybindings":[{"command":"down","key":"j"}]}"#,
        ))
        .expect("should parse");
        assert_eq!(m.contributes.keybindings[0].key, "j");
        assert_eq!(m.contributes.keybindings[0].command, "down");
    }

    /// A binding naming a command that does not exist would silently never
    /// fire; better to refuse the install and say which id is wrong.
    #[test]
    fn rejects_a_keybinding_for_an_undeclared_command() {
        let err = parse_manifest(&manifest_json(
            r#","contributes":{"commands":[{"id":"down","title":"Down"}],
               "keybindings":[{"command":"up","key":"k"}]}"#,
        ))
        .unwrap_err();
        assert_eq!(err, ManifestError::UnknownBoundCommand("up".into()));
        assert!(err.to_string().contains("up"));
    }

    #[test]
    fn reads_panels() {
        let m = parse_manifest(&manifest_json(
            r#","contributes":{"panels":[{"id":"refs","title":"References"}]}"#,
        ))
        .expect("should parse");
        assert_eq!(m.contributes.panels[0].id, "refs");
        assert_eq!(m.contributes.panels[0].title, "References");
    }

    #[test]
    fn rejects_bad_or_duplicate_panel_ids() {
        let dup = parse_manifest(&manifest_json(
            r#","contributes":{"panels":[{"id":"a","title":"A"},{"id":"a","title":"B"}]}"#,
        ))
        .unwrap_err();
        assert_eq!(dup, ManifestError::DuplicatePanel("a".into()));

        for bad in [
            r#"{"id":"a/b","title":"A"}"#,
            r#"{"id":"","title":"A"}"#,
            r#"{"id":"a","title":" "}"#,
        ] {
            let err = parse_manifest(&manifest_json(&format!(
                r#","contributes":{{"panels":[{bad}]}}"#
            )))
            .unwrap_err();
            assert!(
                matches!(err, ManifestError::BadPanelId(_)),
                "{bad} was allowed"
            );
        }
    }

    #[test]
    fn reads_selection_and_citation_actions() {
        let m = parse_manifest(&manifest_json(
            r#","contributes":{"commands":[{"id":"keep","title":"Keep"}],
               "selectionActions":[{"command":"keep","title":"Keep Selection"}],
               "citationActions":[{"command":"keep","title":"Keep Citation"}]}"#,
        ))
        .expect("should parse");
        assert_eq!(m.contributes.selection_actions[0].title, "Keep Selection");
        assert_eq!(m.contributes.citation_actions[0].command, "keep");
    }

    /// The bug this guards against cost an afternoon: a manifest saying
    /// `selectionActions` while serde expected `selection_actions` parsed
    /// *successfully*, defaulted the field to empty, and produced an extension
    /// that installed cleanly and silently did nothing.
    #[test]
    fn a_misspelled_contribution_point_is_an_error_not_a_silent_empty() {
        let err = parse_manifest(&manifest_json(
            r#","contributes":{"commands":[{"id":"keep","title":"Keep"}],
               "selection_actions":[{"command":"keep","title":"Keep"}]}"#,
        ))
        .unwrap_err();
        assert!(matches!(err, ManifestError::Syntax(_)), "got {err:?}");
        assert!(err.to_string().contains("selection_actions"), "{err}");
    }

    #[test]
    fn an_unknown_manifest_field_is_refused() {
        let err = parse_manifest(&manifest_json(r#","permisions":["storage"]"#)).unwrap_err();
        assert!(matches!(err, ManifestError::Syntax(_)));
    }

    #[test]
    fn rejects_an_action_for_an_undeclared_command() {
        let err = parse_manifest(&manifest_json(
            r#","contributes":{"selectionActions":[{"command":"nope","title":"X"}]}"#,
        ))
        .unwrap_err();
        assert_eq!(err, ManifestError::UnknownBoundCommand("nope".into()));
    }

    #[test]
    fn reads_the_commands_run_permission() {
        let m = parse_manifest(&manifest_json(r#","permissions":["commands:run"]"#)).unwrap();
        assert_eq!(m.permissions, vec![Permission::CommandsRun]);
    }

    #[test]
    fn rejects_empty_required_fields() {
        let json = manifest_json("").replace("\"name\":\"Thing\"", "\"name\":\"  \"");
        assert_eq!(
            parse_manifest(&json).unwrap_err(),
            ManifestError::MissingField("name")
        );
    }

    #[test]
    fn errors_name_the_field_to_fix() {
        let json = manifest_json("").replace("\"apiVersion\":1", "\"apiVersion\":9");
        let message = parse_manifest(&json).unwrap_err().to_string();
        assert!(message.contains('9') && message.contains('1'), "{message}");
    }

    #[test]
    fn commands_are_namespaced_by_extension() {
        assert_eq!(
            qualified_command_id("com.example.thing", "export"),
            "com.example.thing/export"
        );
    }

    // ---------- registry ----------

    fn write_extension(dir: &Path, manifest: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), manifest).unwrap();
        std::fs::write(dir.join("worker.js"), "self.onmessage=()=>{}").unwrap();
    }

    // ---------- git sources ----------

    #[test]
    fn accepts_an_https_url() {
        assert_eq!(
            parse_git_source("https://github.com/someone/ext.git").unwrap(),
            GitSource {
                url: "https://github.com/someone/ext.git".into(),
                reference: None
            }
        );
    }

    #[test]
    fn reads_a_ref_after_a_hash() {
        let source = parse_git_source("https://github.com/someone/ext#v1.2.0").unwrap();
        assert_eq!(source.reference.as_deref(), Some("v1.2.0"));
        assert_eq!(source.url, "https://github.com/someone/ext");
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        assert_eq!(
            parse_git_source("  https://example.com/e.git#main  ")
                .unwrap()
                .reference
                .as_deref(),
            Some("main")
        );
    }

    /// ssh and file URLs are refused deliberately: ssh would clone with the
    /// user's own credentials, and file:// would let a "URL" reach anywhere on
    /// disk.
    #[test]
    fn refuses_schemes_that_would_use_credentials_or_local_paths() {
        for bad in [
            "ssh://git@github.com/someone/ext.git",
            "git@github.com:someone/ext.git",
            "file:///etc",
            "/Users/someone/ext",
            "ftp://example.com/e.git",
        ] {
            assert!(parse_git_source(bad).is_err(), "{bad} was allowed");
        }
    }

    /// The one injection this interface has: git reads an argument beginning
    /// with `-` as an option, whatever it was meant to be.
    #[test]
    fn refuses_anything_git_would_read_as_a_flag() {
        assert!(parse_git_source("--upload-pack=touch /tmp/pwned").is_err());
        assert!(parse_git_source("-c core.sshCommand=evil").is_err());
        assert!(parse_git_source("https://example.com/e.git#--upload-pack=evil").is_err());
    }

    #[test]
    fn refuses_whitespace_and_control_characters() {
        assert!(parse_git_source("https://example.com/a b.git").is_err());
        assert!(parse_git_source("https://example.com/e.git\0").is_err());
    }

    #[test]
    fn refuses_a_ref_with_shell_or_path_tricks() {
        for bad in [
            "https://e.com/x#a;b",
            "https://e.com/x#$(id)",
            "https://e.com/x#a b",
        ] {
            assert!(parse_git_source(bad).is_err(), "{bad} was allowed");
        }
    }

    #[test]
    fn refuses_an_empty_url_or_ref() {
        assert!(parse_git_source("").is_err());
        assert!(parse_git_source("   ").is_err());
        assert!(parse_git_source("https://example.com/e.git#").is_err());
    }

    #[test]
    fn refuses_an_absurd_url() {
        let long = format!("https://example.com/{}", "a".repeat(4000));
        assert!(parse_git_source(&long).is_err());
    }

    /// A ref with a slash is ordinary (`release/2.0`), so it must survive.
    #[test]
    fn allows_ordinary_branch_names() {
        for good in ["main", "release/2.0", "v1.0.0", "feature_x", "a-b.c"] {
            let spec = format!("https://example.com/e.git#{good}");
            assert_eq!(
                parse_git_source(&spec).unwrap().reference.as_deref(),
                Some(good)
            );
        }
    }

    #[test]
    fn installs_and_lists_an_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        write_extension(&src, &manifest_json(""));

        let entry = install_from_folder(&root, &src).expect("install");
        assert_eq!(entry.id, "com.example.thing");
        assert!(entry.enabled);
        assert!(root.join("com.example.thing").join("worker.js").is_file());
        assert_eq!(Registry::load(&root).extensions.len(), 1);
    }

    #[test]
    fn a_rejected_manifest_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        write_extension(
            &src,
            &manifest_json("").replace("\"apiVersion\":1", "\"apiVersion\":7"),
        );

        assert!(install_from_folder(&root, &src).is_err());
        assert!(!root.join("com.example.thing").exists());
        assert!(Registry::load(&root).extensions.is_empty());
    }

    #[test]
    fn a_missing_entry_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join(MANIFEST_FILE), manifest_json("")).unwrap();

        assert!(install_from_folder(&root, &src).is_err());
        assert!(Registry::load(&root).extensions.is_empty());
    }

    #[test]
    fn reinstalling_replaces_rather_than_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        write_extension(&src, &manifest_json(""));
        install_from_folder(&root, &src).unwrap();

        write_extension(&src, &manifest_json("").replace("1.0.0", "2.0.0"));
        install_from_folder(&root, &src).unwrap();

        let registry = Registry::load(&root);
        assert_eq!(registry.extensions.len(), 1);
        assert_eq!(
            registry
                .extensions
                .iter()
                .find(|e| e.id == "com.example.thing")
                .unwrap()
                .version,
            "2.0.0"
        );
    }

    /// A new version asking for more than the user agreed to must not inherit
    /// the old grant.
    #[test]
    fn reinstalling_does_not_carry_permissions_forward_silently() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        write_extension(&src, &manifest_json(""));
        install_from_folder(&root, &src).unwrap();
        assert!(Registry::load(&root).extensions[0].permissions.is_empty());

        write_extension(&src, &manifest_json(r#","permissions":["library:read"]"#));
        let entry = install_from_folder(&root, &src).unwrap();
        assert_eq!(entry.permissions, vec![Permission::LibraryRead]);
    }

    #[test]
    fn uninstall_removes_files_and_the_row() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        write_extension(&src, &manifest_json(""));
        install_from_folder(&root, &src).unwrap();

        uninstall(&root, "com.example.thing").unwrap();
        assert!(!root.join("com.example.thing").exists());
        assert!(Registry::load(&root).extensions.is_empty());
    }

    #[test]
    fn uninstalling_something_absent_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ext");
        assert!(uninstall(&root, "com.example.gone").is_ok());
    }

    #[test]
    fn a_corrupt_registry_reads_as_empty_rather_than_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join(REGISTRY_FILE), "{not json").unwrap();
        assert!(Registry::load(&root).extensions.is_empty());
    }

    #[test]
    fn a_future_registry_version_is_ignored_rather_than_misread() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::write(
            root.join(REGISTRY_FILE),
            r#"{"version":99,"extensions":[{"id":"com.a.b","name":"x","version":"1",
                "source":"","permissions":[],"enabled":true}]}"#,
        )
        .unwrap();
        assert!(Registry::load(&root).extensions.is_empty());
    }

    #[test]
    fn dotfiles_are_not_copied_in() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, src) = (tmp.path().join("ext"), tmp.path().join("src"));
        write_extension(&src, &manifest_json(""));
        std::fs::create_dir_all(src.join(".git")).unwrap();
        std::fs::write(src.join(".git").join("HEAD"), "ref: x").unwrap();

        install_from_folder(&root, &src).unwrap();
        assert!(!root.join("com.example.thing").join(".git").exists());
    }
}

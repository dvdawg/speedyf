//! Markup (`dev.speedyf.markup`): comments on someone else's paper, stored
//! per document, exported as one annotated PDF.
//!
//! The comment JSON is the frontend's; it is stored here opaquely under a key
//! that follows the document, not its path: the PDF's permanent identifier
//! (trailer `/ID[0]`) with its page count when it has one, else a hash of the
//! canonical path. Files live at `<app data>/markup/<key>.json`.

use super::EngineState;
use crate::engine::markup_export::{self, MarkupExportRequest, MarkupExportResult};
use crate::engine::types::Priority;
use crate::engine::Work;
use crate::errors::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tauri::Manager;

const DIR: &str = "markup";
const MAX_JSON_BYTES: usize = 32 << 20;

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The storage key for a document. A long identifier is hashed so the key
/// stays a sane file name; the page count tells apart files from a writer
/// that stamps one identifier on everything it makes.
pub(crate) fn storage_key(id: Option<(String, usize)>, canonical: &Path) -> String {
    match id {
        Some((hex, pages)) if hex.len() <= 64 => format!("id-{hex}-{pages}"),
        Some((hex, pages)) => format!("id-{}-{pages}", sha256_hex(hex.as_bytes())),
        None => format!(
            "path-{}",
            sha256_hex(canonical.to_string_lossy().as_bytes())
        ),
    }
}

/// Keys already read, by canonical path, valid while the file's size and
/// modification time are unchanged.
/// (size, modified, key)
type KeyEntry = (u64, Option<SystemTime>, String);

#[derive(Default)]
pub struct MarkupKeys(Mutex<HashMap<PathBuf, KeyEntry>>);

async fn key_for(app: &tauri::AppHandle, path: &str) -> AppResult<String> {
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|e| AppError::Io(format!("cannot read {path}: {e}")))?;
    let meta = std::fs::metadata(&canonical)?;
    let stamp = (meta.len(), meta.modified().ok());
    let keys = app.state::<MarkupKeys>();
    if let Some((len, modified, key)) = keys
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&canonical)
    {
        if (*len, *modified) == stamp {
            return Ok(key.clone());
        }
    }
    let engine = &app.state::<EngineState>().0;
    let target = canonical.to_string_lossy().into_owned();
    let id = engine
        .call_async(Priority::AdjacentPage, 0, move |respond| Work::MarkupKey {
            path: target,
            respond,
        })
        .await?;
    let key = storage_key(id, &canonical);
    keys.0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(canonical, (stamp.0, stamp.1, key.clone()));
    Ok(key)
}

fn store_dir(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map(|d| d.join(DIR))
        .map_err(|e| AppError::Io(format!("no app data directory: {e}")))
}

pub(crate) fn read_stored(dir: &Path, key: &str) -> AppResult<Option<String>> {
    match std::fs::read_to_string(dir.join(format!("{key}.json"))) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::Io(format!("cannot read markup: {e}"))),
    }
}

/// Atomic: a temp sibling, synced, renamed over the old file.
pub(crate) fn write_stored(dir: &Path, key: &str, json: &str) -> AppResult<()> {
    if json.len() > MAX_JSON_BYTES {
        return Err(AppError::Unsupported("markup is too large to store".into()));
    }
    serde_json::from_str::<serde::de::IgnoredAny>(json)
        .map_err(|e| AppError::Unsupported(format!("markup is not JSON: {e}")))?;
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".markup-")
        .suffix(".json")
        .tempfile_in(dir)?;
    std::io::Write::write_all(&mut temp, json.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(dir.join(format!("{key}.json")))
        .map_err(|e| AppError::Io(format!("cannot save markup: {e}")))?;
    Ok(())
}

/// The markup stored for the PDF at `path`, or null when there is none.
#[tauri::command]
pub async fn markup_load(app: tauri::AppHandle, path: String) -> AppResult<Option<String>> {
    let key = key_for(&app, &path).await?;
    read_stored(&store_dir(&app)?, &key)
}

/// Store `json` as the markup for the PDF at `path`.
#[tauri::command]
pub async fn markup_save(app: tauri::AppHandle, path: String, json: String) -> AppResult<()> {
    let key = key_for(&app, &path).await?;
    write_stored(&store_dir(&app)?, &key, &json)
}

/// Write the annotated copy: sheets rendered by WebKit, then the PDF work on
/// the engine thread.
#[tauri::command]
pub async fn markup_export(
    app: tauri::AppHandle,
    request: MarkupExportRequest,
) -> AppResult<MarkupExportResult> {
    run_export(&app, request).await
}

pub(crate) async fn run_export(
    app: &tauri::AppHandle,
    request: MarkupExportRequest,
) -> AppResult<MarkupExportResult> {
    markup_export::validate(&request)?;
    let sheets = crate::markup_capture::capture_sheets(app, &request).await?;
    app.state::<EngineState>()
        .0
        .call_async(Priority::VisiblePage, 0, move |respond| {
            Work::MarkupExport {
                request: Box::new(request),
                sheets,
                respond,
            }
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_identifier_then_the_path() {
        let p = Path::new("/papers/a.pdf");
        assert_eq!(storage_key(Some(("00ff".into(), 12)), p), "id-00ff-12");
        let long = "ab".repeat(40);
        let k = storage_key(Some((long, 3)), p);
        assert!(k.starts_with("id-") && k.ends_with("-3") && k.len() == 3 + 64 + 2);
        let a = storage_key(None, p);
        assert!(a.starts_with("path-"));
        assert_eq!(a, storage_key(None, p));
        assert_ne!(a, storage_key(None, Path::new("/papers/b.pdf")));
    }

    #[test]
    fn stored_markup_round_trips_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("markup");
        assert_eq!(read_stored(&store, "id-1-1").unwrap(), None);
        write_stored(&store, "id-1-1", r#"{"version":1,"comments":[]}"#).unwrap();
        write_stored(&store, "id-1-1", r#"{"version":1,"summary":"x"}"#).unwrap();
        assert_eq!(
            read_stored(&store, "id-1-1").unwrap().as_deref(),
            Some(r#"{"version":1,"summary":"x"}"#)
        );
        assert!(write_stored(&store, "id-1-1", "not json").is_err());
        let names: Vec<_> = std::fs::read_dir(&store)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["id-1-1.json"]);
    }
}

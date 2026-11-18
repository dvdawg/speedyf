//! The user's saved visual signatures and initials, and the two pieces of
//! work the create dialog hands to Rust: setting a typed name in a bundled
//! font as outlines, and cleaning up an imported picture.
//!
//! Saved signatures are personal and stay on this machine: one JSON file at
//! `<app data>/signatures/signatures.json`, never in the library and never in
//! anything a sync or a project export touches. The JSON is the frontend's
//! (`features/sign/signStore.ts`); it is stored opaquely, only checked to be
//! JSON.

use crate::engine::signature::{self, PreparedImage, TypedOutline};
use crate::errors::{AppError, AppResult};
use std::path::{Path, PathBuf};
use tauri::Manager;

const DIR: &str = "signatures";
const FILE: &str = "signatures.json";
/// Room for a dozen imported pictures; anything bigger is a mistake.
const MAX_JSON_BYTES: usize = 24 << 20;

fn store_dir(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map(|d| d.join(DIR))
        .map_err(|e| AppError::Io(format!("no app data directory: {e}")))
}

pub(crate) fn read_store(dir: &Path) -> AppResult<Option<String>> {
    match std::fs::read_to_string(dir.join(FILE)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(AppError::Io(format!("cannot read saved signatures: {e}"))),
    }
}

/// Atomic: a temp sibling, synced, renamed over the old file.
pub(crate) fn write_store(dir: &Path, json: &str) -> AppResult<()> {
    if json.len() > MAX_JSON_BYTES {
        return Err(AppError::Unsupported(
            "saved signatures are too large to store".into(),
        ));
    }
    serde_json::from_str::<serde::de::IgnoredAny>(json)
        .map_err(|e| AppError::Unsupported(format!("signatures are not JSON: {e}")))?;
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".signatures-")
        .suffix(".json")
        .tempfile_in(dir)?;
    std::io::Write::write_all(&mut temp, json.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(dir.join(FILE))
        .map_err(|e| AppError::Io(format!("cannot save signatures: {e}")))?;
    Ok(())
}

/// The saved signatures, or null before any was made.
#[tauri::command]
pub fn signatures_load(app: tauri::AppHandle) -> AppResult<Option<String>> {
    read_store(&store_dir(&app)?)
}

#[tauri::command]
pub fn signatures_save(app: tauri::AppHandle, json: String) -> AppResult<()> {
    write_store(&store_dir(&app)?, &json)
}

/// `text` set in one of the typed styles, as outlines.
#[tauri::command]
pub fn signature_outline(text: String, style: String) -> AppResult<TypedOutline> {
    signature::typed_outline(&text, &style)
}

/// A picture of a signature with its paper made transparent and cropped.
#[tauri::command]
pub async fn signature_image(path: String) -> AppResult<PreparedImage> {
    tauri::async_runtime::spawn_blocking(move || signature::prepare_image_file(&path))
        .await
        .map_err(|error| AppError::Internal(format!("image task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_round_trips_and_refuses_non_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signatures");
        assert_eq!(read_store(&path).unwrap(), None);
        write_store(&path, r#"{"v":1,"items":[]}"#).unwrap();
        assert_eq!(
            read_store(&path).unwrap().as_deref(),
            Some(r#"{"v":1,"items":[]}"#)
        );
        assert!(write_store(&path, "not json").is_err());
        assert_eq!(
            read_store(&path).unwrap().as_deref(),
            Some(r#"{"v":1,"items":[]}"#)
        );
    }
}

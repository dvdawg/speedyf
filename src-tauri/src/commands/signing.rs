//! Checking the digital signatures in a document (`engine::signing`).
//!
//! Runs off the render worker: it reads the file itself and does no PDFium
//! work, so a large signed file never holds up a page render. Offline.

use crate::engine::signing::{self, SignatureReport};
use crate::errors::{AppError, AppResult};

/// Every signature in the PDF at `path`, checked. Empty for a document with
/// none, which is the common case and costs one scan of the bytes.
#[tauri::command]
pub async fn check_signatures(path: String) -> AppResult<Vec<SignatureReport>> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path)?;
        Ok(signing::check_document(&bytes))
    })
    .await
    .map_err(|e| AppError::Internal(format!("signature check failed: {e}")))?
}

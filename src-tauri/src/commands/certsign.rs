//! Certificate signing: the identities to sign with, and signing a document.
//!
//! Identities are personal and stay on this machine: in the Keychain on
//! macOS, otherwise in `<app data>/certificates/identities.json` with each
//! key encrypted under the user's passphrase (engine/signing/keys.rs).
//!
//! Signing is offline unless the request names a timestamp server, which the
//! dialog only sends when the user turned the online timestamp on.

use super::EngineState;
use crate::engine::signing::keys::{self, IdentityDto, StoreKind};
use crate::engine::signing::sign::{self, cms, tsa, SignRequest};
use crate::engine::types::{DocId, Priority};
use crate::engine::Work;
use crate::errors::{AppError, AppResult};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

const DIR: &str = "certificates";

fn store_dir(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map(|d| d.join(DIR))
        .map_err(|e| AppError::Io(format!("no app data directory: {e}")))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("signing task failed: {e}")))?
}

/// Every identity that can sign, newest store first.
#[tauri::command]
pub async fn cert_identities(app: tauri::AppHandle) -> AppResult<Vec<IdentityDto>> {
    let dir = store_dir(&app)?;
    blocking(move || keys::list(&dir)).await
}

/// Make a self-signed identity.
#[tauri::command]
pub async fn cert_create_identity(
    app: tauri::AppHandle,
    name: String,
    email: String,
    organization: String,
    store: String,
    passphrase: Option<String>,
) -> AppResult<IdentityDto> {
    let dir = store_dir(&app)?;
    let store = StoreKind::parse(&store)?;
    blocking(move || {
        keys::create_self_signed(
            &dir,
            store,
            &name,
            &email,
            &organization,
            passphrase.as_deref(),
        )
    })
    .await
}

/// Import a .p12 / .pfx file.
#[tauri::command]
pub async fn cert_import_identity(
    app: tauri::AppHandle,
    path: String,
    password: String,
    store: String,
    passphrase: Option<String>,
) -> AppResult<IdentityDto> {
    let dir = store_dir(&app)?;
    let store = StoreKind::parse(&store)?;
    blocking(move || {
        let meta = std::fs::metadata(&path)?;
        if meta.len() > 1 << 20 {
            return Err(AppError::Unsupported("that is not a .p12 file".into()));
        }
        let bytes = std::fs::read(&path)?;
        keys::import_p12(&dir, store, &bytes, &password, passphrase.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn cert_remove_identity(app: tauri::AppHandle, id: String) -> AppResult<()> {
    let dir = store_dir(&app)?;
    blocking(move || keys::remove(&dir, &id)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignResultDto {
    pub path: String,
    pub bytes: u64,
    pub timestamped: bool,
}

/// Sign `src_path` (the open document `doc_id`, saved) into `dest_path`.
///
/// The signature is an incremental update to the file as it is on disk, so
/// the caller saves pending edits first. `dest_path` may be the same file.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn cert_sign(
    app: tauri::AppHandle,
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src_path: String,
    dest_path: String,
    identity: String,
    passphrase: Option<String>,
    request: SignRequest,
) -> AppResult<SignResultDto> {
    let dir = store_dir(&app)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let tsa_url = request
        .tsa_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string);
    let req = request.clone();
    let (mut prepared, mut parts) = blocking(move || {
        let src = std::fs::read(&src_path)?;
        let signer = keys::signer(&dir, &identity, passphrase.as_deref())?;
        let chain_bytes: usize = signer.chain().iter().map(Vec::len).sum();
        let name = sign::signer_name(signer.as_ref());
        let prepared = sign::prepare(&src, &req, &name, chain_bytes, now)?;
        let digest = prepared.digest();
        let parts = cms::build(signer.as_ref(), &digest, req.sub_filter, now)?;
        Ok((prepared, parts))
    })
    .await?;

    let timestamped = match tsa_url {
        Some(url) => {
            let token = tsa::fetch(&url, parts.signature()).await?;
            parts.add_timestamp(&token)?;
            true
        }
        None => false,
    };

    let bytes = blocking(move || {
        let der = parts.to_der()?;
        sign::embed(&mut prepared, &der)?;
        sign::check(&prepared.bytes)?;
        Ok(prepared.bytes)
    })
    .await?;

    let dest = PathBuf::from(&dest_path);
    let written = state
        .0
        .call_async(Priority::VisiblePage, doc_id, move |respond| {
            Work::WriteSigned {
                doc: doc_id,
                bytes,
                dest,
                respond,
            }
        })
        .await?;
    Ok(SignResultDto {
        path: dest_path,
        bytes: written,
        timestamped,
    })
}

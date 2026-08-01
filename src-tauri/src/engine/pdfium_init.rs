//! Locates and binds the PDFium dynamic library.
//!
//! Search order: `SPEEDYF_PDFIUM_PATH` env (file or dir) → caller-provided
//! hints (Tauri resource dir in production) → dev-time `src-tauri/pdfium/`
//! (compile-time manifest path) → executable-adjacent dirs → system library.

use pdfium_render::prelude::*;
use std::path::PathBuf;

pub fn candidate_dirs(hints: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(env_path) = std::env::var("SPEEDYF_PDFIUM_PATH") {
        dirs.push(PathBuf::from(env_path));
    }
    dirs.extend(hints.iter().cloned());
    // Development layout (cargo run / tauri dev):
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pdfium"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("pdfium"));
            dirs.push(dir.to_path_buf());
            // macOS bundle: Contents/MacOS/<exe> → Contents/Resources/pdfium
            dirs.push(dir.join("../Resources/pdfium"));
            dirs.push(dir.join("../Resources"));
        }
    }
    dirs
}

pub fn init_bindings(hints: &[PathBuf]) -> Result<Box<dyn PdfiumLibraryBindings>, String> {
    let mut tried = Vec::new();
    for dir in candidate_dirs(hints) {
        let lib = if dir.is_file() {
            dir.clone()
        } else {
            Pdfium::pdfium_platform_library_name_at_path(&dir)
        };
        if lib.is_file() {
            match Pdfium::bind_to_library(&lib) {
                Ok(b) => {
                    log::info!("pdfium loaded from {}", lib.display());
                    return Ok(b);
                }
                Err(e) => tried.push(format!("{}: {e:?}", lib.display())),
            }
        } else {
            tried.push(format!("{} (not found)", lib.display()));
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(b) => {
            log::info!("pdfium loaded from system library path");
            Ok(b)
        }
        Err(e) => Err(format!(
            "could not locate PDFium. Run `pnpm fetch-pdfium` first. Tried: {tried:?}; system: {e:?}"
        )),
    }
}

#[cfg(test)]
pub fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

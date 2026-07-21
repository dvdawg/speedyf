//! Typed Tauri commands. All file access flows through these narrow entry
//! points — the webview has no filesystem capability of its own.
//! (JS invoke args arrive camelCase; Tauri maps them onto snake_case params.)

use crate::engine::types::*;
use crate::engine::{EngineHandle, Work};
use crate::errors::{AppError, AppResult};
use std::path::PathBuf;

pub struct EngineState(pub EngineHandle);

#[tauri::command]
pub fn open_document(
    state: tauri::State<'_, EngineState>,
    path: String,
    password: Option<String>,
) -> AppResult<DocMetaDto> {
    if !std::path::Path::new(&path).is_file() {
        return Err(AppError::Io(format!("not a readable file: {path}")));
    }
    state.0.call(Priority::VisiblePage, 0, |respond| Work::Open {
        path,
        password,
        respond,
    })
}

#[tauri::command]
pub fn close_document(state: tauri::State<'_, EngineState>, doc_id: DocId) {
    // bump first so every queued job for this doc is skipped, not executed
    state.0.bump_generation(doc_id);
    state
        .0
        .submit(Priority::VisiblePage, doc_id, Work::Close { doc: doc_id });
}

#[tauri::command]
pub fn request_page_sizes(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    from: u32,
    count: u32,
) -> AppResult<PageSizesDto> {
    state
        .0
        .call(Priority::AdjacentPage, doc_id, |respond| Work::Sizes {
            doc: doc_id,
            from,
            count: count.min(256),
            respond,
        })
}

#[tauri::command]
pub fn get_text_layout(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u16,
) -> AppResult<PageTextDto> {
    state
        .0
        .call(Priority::AdjacentPage, doc_id, |respond| Work::TextLayout {
            doc: doc_id,
            src,
            respond,
        })
}

#[tauri::command]
pub fn start_indexing(state: tauri::State<'_, EngineState>, doc_id: DocId) {
    state
        .0
        .submit(Priority::TextExtract, doc_id, Work::IndexNext { doc: doc_id });
}

#[tauri::command]
pub fn search_query(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    query: String,
    case_sensitive: bool,
) -> Vec<PageMatchesDto> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    // Runs directly against the shared index — no engine round-trip, so
    // results are instant over whatever subset is indexed so far.
    state.0 .0.search.lock().query(doc_id, &query, case_sensitive, 200)
}

#[tauri::command]
pub fn get_match_rects(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u16,
    start: u32,
    len: u32,
) -> AppResult<Vec<[f32; 4]>> {
    state
        .0
        .call(Priority::VisibleThumb, doc_id, |respond| Work::MatchRects {
            doc: doc_id,
            src,
            start,
            len,
            respond,
        })
}

#[tauri::command]
pub fn save_document(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    plan: EditPlan,
    dest_path: String,
) -> AppResult<SaveResultDto> {
    if plan.pages.is_empty() {
        return Err(AppError::Unsupported("cannot save an empty document".into()));
    }
    state.0.call(Priority::VisiblePage, doc_id, |respond| Work::Save {
        doc: doc_id,
        plan,
        dest: PathBuf::from(dest_path),
        respond,
    })
}

#[tauri::command]
pub fn get_form_fields(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<Vec<FormFieldDto>> {
    state
        .0
        .call(Priority::AdjacentPage, doc_id, |respond| Work::FormFields {
            doc: doc_id,
            respond,
        })
}

#[tauri::command]
pub fn image_size(state: tauri::State<'_, EngineState>, path: String) -> AppResult<[u32; 2]> {
    state
        .0
        .call(Priority::AdjacentPage, 0, |respond| Work::ImageSize { path, respond })
}

/// Small downscaled PNG preview of a local image, returned over binary IPC
/// (used by the annotation overlay; the full-size file goes into the PDF only
/// at save time, engine-side).
#[tauri::command]
pub fn image_preview(path: String) -> AppResult<tauri::ipc::Response> {
    let img = image::open(&path).map_err(|e| AppError::Io(format!("cannot open image: {e}")))?;
    let thumb = img.thumbnail(512, 512);
    let mut png = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| AppError::Internal(format!("png encode: {e}")))?;
    Ok(tauri::ipc::Response::new(png))
}

#[tauri::command]
pub fn engine_metrics(state: tauri::State<'_, EngineState>) -> EngineMetricsDto {
    state.0.metrics_snapshot()
}

#[tauri::command]
pub fn set_low_memory(state: tauri::State<'_, EngineState>, enabled: bool) {
    state.0.set_low_memory(enabled);
}

#[tauri::command]
pub fn doc_generation(state: tauri::State<'_, EngineState>, doc_id: DocId) -> u64 {
    state.0.current_generation(doc_id)
}

#[tauri::command]
pub fn bump_generation(state: tauri::State<'_, EngineState>, doc_id: DocId) -> u64 {
    state.0.bump_generation(doc_id)
}

//! Typed Tauri commands. All file access flows through these narrow entry
//! points — the webview has no filesystem capability of its own.
//! (JS invoke args arrive camelCase; Tauri maps them onto snake_case params.)

use crate::engine::types::*;
use crate::engine::{EngineHandle, Work};
use crate::errors::{AppError, AppResult};
use std::path::PathBuf;

pub struct EngineState(pub EngineHandle);

#[tauri::command]
pub async fn open_document(
    state: tauri::State<'_, EngineState>,
    path: String,
    password: Option<String>,
) -> AppResult<DocMetaDto> {
    if !std::path::Path::new(&path).is_file() {
        return Err(AppError::Io(format!("not a readable file: {path}")));
    }
    state
        .0
        .call_async(Priority::VisiblePage, 0, move |respond| Work::Open {
            path,
            password,
            respond,
        })
        .await
}

#[tauri::command]
pub async fn close_document(state: tauri::State<'_, EngineState>, doc_id: DocId) -> AppResult<()> {
    // bump first so every queued job for this doc is skipped, not executed
    state.0.bump_generation(doc_id);
    state
        .0
        .submit(Priority::VisiblePage, doc_id, Work::Close { doc: doc_id });
    Ok(())
}

#[tauri::command]
pub async fn request_page_sizes(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    from: u32,
    count: u32,
) -> AppResult<PageSizesDto> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| Work::Sizes {
            doc: doc_id,
            from,
            count: count.min(256),
            respond,
        })
        .await
}

#[tauri::command]
pub async fn get_text_layout(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
) -> AppResult<PageTextDto> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::TextLayout {
                doc: doc_id,
                src,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn get_page_links(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
) -> AppResult<PageLinksDto> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::PageLinks {
                doc: doc_id,
                src,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn get_preview_rect(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
    x: Option<f32>,
    y: Option<f32>,
) -> AppResult<PreviewSpecDto> {
    state
        .0
        .call_async(Priority::HoverPreview, doc_id, move |respond| {
            Work::PreviewRect {
                doc: doc_id,
                src,
                x,
                y,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn resolve_citation(
    state: tauri::State<'_, EngineState>,
    id: CitationIdDto,
) -> AppResult<Option<ResolvedCitationDto>> {
    let main_epoch = state.0.main_epoch();
    state
        .0
        .call_async(Priority::HoverPreview, 0, move |respond| {
            Work::ResolveCitation {
                id,
                main_epoch,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn set_library_root(
    state: tauri::State<'_, EngineState>,
    path: Option<String>,
) -> AppResult<()> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || engine.set_library_root(path))
        .await
        .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

#[tauri::command]
pub async fn library_status(state: tauri::State<'_, EngineState>) -> AppResult<LibraryStatusDto> {
    Ok(state.0.library_status())
}

#[tauri::command]
pub async fn start_indexing(state: tauri::State<'_, EngineState>, doc_id: DocId) -> AppResult<()> {
    state.0.submit(
        Priority::TextExtract,
        doc_id,
        Work::IndexNext { doc: doc_id },
    );
    Ok(())
}

#[tauri::command]
pub async fn search_query(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    query: String,
    case_sensitive: bool,
) -> AppResult<SearchQueryDto> {
    const GLOBAL_MATCH_LIMIT: usize = 500;
    if query.trim().is_empty() {
        return Ok(SearchQueryDto {
            pages: Vec::new(),
            truncated: false,
            limit: GLOBAL_MATCH_LIMIT as u32,
        });
    }
    // Runs directly against the shared index — no engine round-trip, so
    // results are instant over whatever subset is indexed so far.
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.search_indexed(doc_id, &query, case_sensitive, 200, GLOBAL_MATCH_LIMIT)
    })
    .await
    .map_err(|error| AppError::Internal(format!("search task failed: {error}")))
}

#[tauri::command]
pub async fn get_match_rects(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
    start: u32,
    len: u32,
) -> AppResult<Vec<[f32; 4]>> {
    state
        .0
        .call_async(Priority::VisibleThumb, doc_id, move |respond| {
            Work::MatchRects {
                doc: doc_id,
                src,
                start,
                len,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn save_document(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    plan: EditPlan,
    dest_path: String,
) -> AppResult<SaveResultDto> {
    if plan.pages.is_empty() {
        return Err(AppError::Unsupported(
            "cannot save an empty document".into(),
        ));
    }
    state
        .0
        .call_async(Priority::VisiblePage, doc_id, move |respond| Work::Save {
            doc: doc_id,
            plan,
            dest: PathBuf::from(dest_path),
            respond,
        })
        .await
}

#[tauri::command]
pub async fn get_form_fields(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<Vec<FormFieldDto>> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::FormFields {
                doc: doc_id,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn image_size(state: tauri::State<'_, EngineState>, path: String) -> AppResult<[u32; 2]> {
    state
        .0
        .call_async(Priority::AdjacentPage, 0, move |respond| Work::ImageSize {
            path,
            respond,
        })
        .await
}

/// Small downscaled PNG preview of a local image, returned over binary IPC
/// (used by the annotation overlay; the full-size file goes into the PDF only
/// at save time, engine-side).
#[tauri::command]
pub async fn image_preview(path: String) -> AppResult<tauri::ipc::Response> {
    tauri::async_runtime::spawn_blocking(move || {
        let img =
            image::open(&path).map_err(|e| AppError::Io(format!("cannot open image: {e}")))?;
        let thumb = img.thumbnail(512, 512);
        let mut png = Vec::new();
        thumb
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| AppError::Internal(format!("png encode: {e}")))?;
        Ok(tauri::ipc::Response::new(png))
    })
    .await
    .map_err(|error| AppError::Internal(format!("image preview task failed: {error}")))?
}

#[tauri::command]
pub async fn engine_metrics(state: tauri::State<'_, EngineState>) -> AppResult<EngineMetricsDto> {
    Ok(state.0.metrics_snapshot())
}

#[tauri::command]
pub async fn set_low_memory(state: tauri::State<'_, EngineState>, enabled: bool) -> AppResult<()> {
    state.0.set_low_memory(enabled);
    Ok(())
}

#[tauri::command]
pub async fn doc_generation(state: tauri::State<'_, EngineState>, doc_id: DocId) -> AppResult<u64> {
    Ok(state.0.current_generation(doc_id))
}

#[tauri::command]
pub async fn bump_generation(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<u64> {
    Ok(state.0.bump_generation(doc_id))
}

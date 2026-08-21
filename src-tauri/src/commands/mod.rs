//! Typed Tauri commands. All file access flows through these narrow entry
//! points — the webview has no filesystem capability of its own.
//! (JS invoke args arrive camelCase; Tauri maps them onto snake_case params.)

use crate::engine::types::*;
use crate::engine::{EngineHandle, Work};
use crate::errors::{AppError, AppResult};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct EngineState(pub EngineHandle);

/// File paths the OS asked us to open (double-click, "Open With", Dock drop)
/// before the frontend was ready to receive the live `file-open-requested`
/// event — drained once on startup so a cold launch-to-open never loses them.
#[derive(Default)]
pub struct PendingOpens(pub Mutex<Vec<String>>);

#[tauri::command]
pub fn take_pending_opens(state: tauri::State<'_, PendingOpens>) -> Vec<String> {
    std::mem::take(&mut *state.0.lock().unwrap())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetaDto {
    size_bytes: u64,
    modified_ms: i64,
}

/// Pure `std::fs` stat, no worker-thread involvement — used by the home
/// screen's recent-files list, not a PDFium operation.
#[tauri::command]
pub async fn file_metadata(path: String) -> AppResult<FileMetaDto> {
    let meta =
        std::fs::metadata(&path).map_err(|e| AppError::Io(format!("cannot stat {path}: {e}")))?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Ok(FileMetaDto {
        size_bytes: meta.len(),
        modified_ms,
    })
}

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
pub async fn set_active_document(
    state: tauri::State<'_, EngineState>,
    doc_id: Option<DocId>,
) -> AppResult<()> {
    state.0.submit(
        Priority::VisiblePage,
        doc_id.unwrap_or(0),
        Work::SetActiveDocument { doc: doc_id },
    );
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

/// Add a folder to the library, or remove one.
///
/// Runs off the main thread: adding a root walks the whole tree to find its
/// PDFs, which on a large or networked folder is not instant.
#[tauri::command]
pub async fn change_library_root(
    state: tauri::State<'_, EngineState>,
    path: String,
    add: bool,
) -> AppResult<()> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || engine.change_library_root(path, add))
        .await
        .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Search every indexed document in the library.
///
/// On a blocking thread rather than the engine thread — it reads text sidecars
/// and never touches PDFium, so it cannot stall the page being read.
#[tauri::command]
pub async fn library_search(
    state: tauri::State<'_, EngineState>,
    query: String,
    case_sensitive: bool,
) -> AppResult<LibrarySearchDto> {
    /// Enough hits from one document to show it is the right one; the document
    /// is opened for a full search past that.
    const PER_DOCUMENT_LIMIT: usize = 20;
    /// Enough documents to choose between. More than this is not a result
    /// list, it is a second search.
    const DOCUMENT_LIMIT: usize = 60;

    if query.trim().is_empty() {
        return Ok(LibrarySearchDto {
            documents: Vec::new(),
            truncated: false,
        });
    }
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.library_search(&query, case_sensitive, PER_DOCUMENT_LIMIT, DOCUMENT_LIMIT)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library search failed: {error}")))
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
        Work::StartIndexing { doc: doc_id },
    );
    Ok(())
}

/// Abandon queued render work for a document whose visible region moved on.
/// Deliberately not `bump_generation`: cached rasters and minted URLs stay
/// valid, so nothing already on screen has to be fetched or decoded again.
#[tauri::command]
pub async fn cancel_renders(state: tauri::State<'_, EngineState>, doc_id: DocId) -> AppResult<()> {
    state.0.cancel_renders(doc_id);
    Ok(())
}

/// Follow a link the document points at, out to the user's browser.
///
/// The frontend asks the user before calling this. The scheme check inside is
/// the second gate and the one the document cannot influence — see
/// `crate::external` for what is refused and why.
#[tauri::command]
pub async fn open_external_url(url: String) -> AppResult<()> {
    crate::external::open_external_url(&url)
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

/// Materialize the document — including every unsaved edit — into a temp file
/// for printing, and return where it went.
///
/// The caller opens that path as an ordinary document to preview it, then
/// prints it, then discards it. Nothing here touches the user's own file.
#[tauri::command]
pub async fn build_print_pdf(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    plan: EditPlan,
) -> AppResult<String> {
    let dest = crate::printing::temp_print_path();
    let path = dest.to_string_lossy().into_owned();
    state
        .0
        .call_async(Priority::VisiblePage, doc_id, move |respond| {
            Work::BuildPrintPdf {
                doc: doc_id,
                plan,
                dest,
                respond,
            }
        })
        .await?;
    Ok(path)
}

/// Delete a finished print job. Refuses any path we did not create.
#[tauri::command]
pub async fn discard_print_pdf(path: String) -> AppResult<()> {
    crate::printing::discard(&path)
}

/// Save a prepared print job where the user asked — "print to PDF".
#[tauri::command]
pub async fn export_print_pdf(path: String, dest_path: String) -> AppResult<()> {
    crate::printing::export_to(&path, &dest_path)
}

/// Printers CUPS knows about. An empty list is a valid answer.
#[tauri::command]
pub async fn list_printers() -> AppResult<Vec<crate::printing::PrinterDto>> {
    crate::printing::list_printers()
}

/// What one printer can vary — duplex, paper, quality — as it reports it, so
/// the dialog offers real capabilities instead of a guessed subset.
#[tauri::command]
pub async fn printer_options(printer: String) -> AppResult<Vec<crate::printing::PrintOptionDto>> {
    crate::printing::printer_options(&printer)
}

/// Hand a prepared print job to CUPS. Every field is re-validated here against
/// a grammar — the job description comes from the webview.
#[tauri::command]
pub async fn submit_print(job: crate::printing::PrintJobDto, path: String) -> AppResult<()> {
    crate::printing::submit(&job, std::path::Path::new(&path))
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
pub async fn get_outline(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<Vec<OutlineNodeDto>> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::Outline {
                doc: doc_id,
                respond,
            }
        })
        .await
}

/// Theorems, lemmas, definitions and captions recovered from a LaTeX-compiled
/// document. Empty for PDFs that were not built with hyperref.
/// Figures, tables and algorithms with a crop of each, for the figure panel.
#[tauri::command]
pub async fn get_figures(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<Vec<FigureDto>> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::Figures {
                doc: doc_id,
                respond,
            }
        })
        .await
}

#[tauri::command]
pub async fn get_formal_envs(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<Vec<FormalEntryDto>> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::FormalEnvs {
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

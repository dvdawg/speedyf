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

/// Every annotation already in the file, so they can be selected, moved and
/// deleted rather than being pixels the renderer happens to draw.
#[tauri::command]
pub async fn get_annotations(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<Vec<PageAnnotationsDto>> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::Annotations {
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

// ---------- extensions ----------

use crate::extensions::{self, InstalledExtension, Manifest};

fn extensions_root(app: &tauri::AppHandle) -> AppResult<std::path::PathBuf> {
    use tauri::Manager;
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Io(format!("no app data dir: {e}")))?
        .join(extensions::EXTENSIONS_DIR))
}

/// One installed extension, with the manifest re-read from disk.
///
/// The registry row and the manifest are reported separately on purpose: the
/// row records what the user granted at install, the manifest records what the
/// files currently ask for. When they disagree the panel can say so rather than
/// silently trusting either.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionDto {
    #[serde(flatten)]
    pub installed: InstalledExtension,
    pub manifest: Option<Manifest>,
    /// Contributed commands, ids already namespaced for the palette. Derived
    /// here rather than in the frontend so the id format has exactly one owner
    /// — it is what a user's keybindings are stored against.
    pub commands: Vec<PaletteCommandDto>,
    /// Contributed panels, ids namespaced the same way commands are.
    pub panels: Vec<PanelDto>,
    /// Actions on a selection or a citation, ids namespaced like commands.
    pub selection_actions: Vec<ActionDto>,
    pub citation_actions: Vec<ActionDto>,
    /// Settings the extension declared, for the panel to render.
    pub settings: Vec<crate::extensions::SettingContribution>,
    pub document_tasks: Vec<ActionDto>,
    /// Why this extension cannot run, if it cannot.
    pub error: Option<String>,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PaletteCommandDto {
    /// `<extension id>/<command id>`
    pub id: String,
    /// The bare id the extension knows itself by, sent back when invoked.
    pub local_id: String,
    pub title: String,
    pub keywords: Vec<String>,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PanelDto {
    /// `<extension id>/<panel id>`
    pub id: String,
    pub local_id: String,
    pub title: String,
    pub extension_id: String,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActionDto {
    /// `<extension id>/<command id>`
    pub id: String,
    pub local_id: String,
    pub title: String,
    pub extension_id: String,
}

fn actions(manifest: &Manifest, which: &[crate::extensions::ActionContribution]) -> Vec<ActionDto> {
    which
        .iter()
        .map(|a| ActionDto {
            id: extensions::qualified_command_id(&manifest.id, &a.command),
            local_id: a.command.clone(),
            title: a.title.clone(),
            extension_id: manifest.id.clone(),
        })
        .collect()
}

fn panels(manifest: &Manifest) -> Vec<PanelDto> {
    manifest
        .contributes
        .panels
        .iter()
        .map(|p| PanelDto {
            id: extensions::qualified_command_id(&manifest.id, &p.id),
            local_id: p.id.clone(),
            title: p.title.clone(),
            extension_id: manifest.id.clone(),
        })
        .collect()
}

fn palette_commands(manifest: &Manifest) -> Vec<PaletteCommandDto> {
    manifest
        .contributes
        .commands
        .iter()
        .map(|c| PaletteCommandDto {
            id: extensions::qualified_command_id(&manifest.id, &c.id),
            local_id: c.id.clone(),
            title: c.title.clone(),
            keywords: c.keywords.clone(),
        })
        .collect()
}

#[tauri::command]
pub async fn list_extensions(app: tauri::AppHandle) -> AppResult<Vec<ExtensionDto>> {
    let root = extensions_root(&app)?;
    let registry = extensions::Registry::load(&root);
    Ok(registry
        .extensions
        .into_iter()
        .map(|installed| {
            let manifest_path = root.join(&installed.id).join(extensions::MANIFEST_FILE);
            match std::fs::read_to_string(&manifest_path) {
                Ok(text) => match extensions::parse_manifest(&text) {
                    Ok(manifest) => ExtensionDto {
                        installed,
                        commands: palette_commands(&manifest),
                        panels: panels(&manifest),
                        selection_actions: actions(
                            &manifest,
                            &manifest.contributes.selection_actions,
                        ),
                        citation_actions: actions(
                            &manifest,
                            &manifest.contributes.citation_actions,
                        ),
                        settings: manifest.contributes.settings.clone(),
                        document_tasks: actions(&manifest, &manifest.contributes.document_tasks),
                        manifest: Some(manifest),
                        error: None,
                    },
                    Err(e) => ExtensionDto {
                        installed,
                        manifest: None,
                        commands: Vec::new(),
                        panels: Vec::new(),
                        selection_actions: Vec::new(),
                        citation_actions: Vec::new(),
                        settings: Vec::new(),
                        document_tasks: Vec::new(),
                        error: Some(e.to_string()),
                    },
                },
                Err(e) => ExtensionDto {
                    installed,
                    manifest: None,
                    commands: Vec::new(),
                    panels: Vec::new(),
                    selection_actions: Vec::new(),
                    citation_actions: Vec::new(),
                    settings: Vec::new(),
                    document_tasks: Vec::new(),
                    error: Some(format!("manifest unreadable: {e}")),
                },
            }
        })
        .collect())
}

#[tauri::command]
pub async fn install_extension(
    app: tauri::AppHandle,
    folder: String,
) -> AppResult<InstalledExtension> {
    let root = extensions_root(&app)?;
    extensions::install_from_folder(&root, std::path::Path::new(&folder))
}

/// Install from a git URL, optionally pinned with `#<branch|tag|commit>`.
#[tauri::command]
pub async fn install_extension_from_git(
    app: tauri::AppHandle,
    url: String,
) -> AppResult<InstalledExtension> {
    let root = extensions_root(&app)?;
    // Cloning is network-bound and can take seconds; off the main thread so the
    // UI is not blocked behind someone's slow connection.
    tauri::async_runtime::spawn_blocking(move || extensions::install_from_git(&root, &url))
        .await
        .map_err(|e| AppError::Internal(format!("install task failed: {e}")))?
}

#[tauri::command]
pub async fn uninstall_extension(app: tauri::AppHandle, id: String) -> AppResult<()> {
    extensions::uninstall(&extensions_root(&app)?, &id)
}

#[tauri::command]
pub async fn set_extension_enabled(
    app: tauri::AppHandle,
    id: String,
    enabled: bool,
) -> AppResult<()> {
    let root = extensions_root(&app)?;
    let mut registry = extensions::Registry::load(&root);
    let Some(entry) = registry.extensions.iter_mut().find(|e| e.id == id) else {
        return Err(AppError::Io(format!("no extension \"{id}\" installed")));
    };
    entry.enabled = enabled;
    registry.save(&root)
}

/// A page raster for an extension, as PNG bytes.
///
/// A command rather than a `pdfr://` URL because the CSP's `connect-src` is
/// `ipc:` alone — the webview cannot `fetch` the render scheme, and an
/// extension needs the bytes, not an `<img>`.
///
/// Submitted at `Priority::Idle`, so a batch job over three hundred pages is
/// starved behind the reader's own scrolling rather than competing with it.
#[tauri::command]
pub async fn extension_render_page(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
    scale_milli: u32,
) -> AppResult<tauri::ipc::Response> {
    // Bounded well below the protocol's own ceiling: this is for recognition,
    // not display, and 300dpi is past the point OCR gains anything.
    let scale = scale_milli.clamp(200, 4_000);
    let key = RenderKey {
        doc: doc_id,
        src,
        rot: 0,
        scale_milli: scale,
        kind: RenderKind::Page,
        tile: None,
        invert: false,
    };
    if let Some(hit) = state.0.cached_render(&key) {
        return Ok(tauri::ipc::Response::new(hit.to_vec()));
    }
    let bytes = state
        .0
        .call_async(Priority::Idle, doc_id, move |respond| Work::Render {
            key,
            respond,
        })
        .await?;
    Ok(tauri::ipc::Response::new(bytes.to_vec()))
}

/// Record text an extension produced for one page of a document.
#[tauri::command]
pub async fn extension_contribute_text(
    app: tauri::AppHandle,
    extension_id: String,
    document: String,
    page: u32,
    text: String,
) -> AppResult<()> {
    let root = extensions_root(&app)?;
    extensions::contribute_text(&root, &document, &extension_id, page, &text)
}

/// What an extension has contributed for a document, if anything.
#[tauri::command]
pub async fn extension_contributed_text(
    app: tauri::AppHandle,
    document: String,
) -> AppResult<Option<extensions::ContributedText>> {
    Ok(extensions::load_contributed(
        &extensions_root(&app)?,
        &document,
    ))
}

/// Discard everything extensions have contributed for a document.
#[tauri::command]
pub async fn forget_contributed_text(app: tauri::AppHandle, document: String) -> AppResult<()> {
    extensions::forget_contributed(&extensions_root(&app)?, &document)
}

/// Source text of an extension's entry file.
///
/// The host wraps this in a blob URL rather than pointing a Worker at a custom
/// scheme: workers must be same-origin with the document that creates them, and
/// a custom scheme never is. See `extensions` for the measurement behind that.
#[tauri::command]
pub async fn extension_source(app: tauri::AppHandle, id: String) -> AppResult<String> {
    let root = extensions_root(&app)?;
    let manifest_path = root.join(&id).join(extensions::MANIFEST_FILE);
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| AppError::Io(format!("{}: {e}", manifest_path.display())))?;
    let manifest = extensions::parse_manifest(&manifest_text)
        .map_err(|e| AppError::Malformed(e.to_string()))?;
    // Resolved through the same component walk that guards asset serving, so a
    // manifest cannot name an entry outside its own directory.
    let entry = extensions::resolve_asset(&root, &format!("/{id}/{}", manifest.entry))
        .ok_or_else(|| AppError::Malformed("entry escapes the extension".into()))?;
    std::fs::read_to_string(&entry).map_err(|e| AppError::Io(format!("{}: {e}", entry.display())))
}

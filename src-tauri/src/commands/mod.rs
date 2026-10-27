//! Typed Tauri commands. All file access flows through these narrow entry
//! points — the webview has no filesystem capability of its own.
//! (JS invoke args arrive camelCase; Tauri maps them onto snake_case params.)

use crate::engine::types::*;
use crate::engine::{EngineHandle, Work};
use crate::errors::{AppError, AppResult};
use std::path::PathBuf;

pub struct EngineState(pub EngineHandle);

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
    window: tauri::WebviewWindow,
    windows: tauri::State<'_, crate::windows::Windows>,
    state: tauri::State<'_, EngineState>,
    path: String,
    password: Option<String>,
) -> AppResult<DocMetaDto> {
    if !std::path::Path::new(&path).is_file() {
        return Err(AppError::Io(format!("not a readable file: {path}")));
    }
    let meta = state
        .0
        .call_async(Priority::VisiblePage, 0, move |respond| Work::Open {
            path,
            password,
            respond,
        })
        .await?;
    windows.opened(window.label(), meta.doc_id);
    Ok(meta)
}

#[tauri::command]
pub async fn close_document(
    window: tauri::WebviewWindow,
    windows: tauri::State<'_, crate::windows::Windows>,
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<()> {
    windows.closed(window.label(), doc_id);
    // bump first so every queued job for this doc is skipped, not executed
    state.0.bump_generation(doc_id);
    state
        .0
        .submit(Priority::VisiblePage, doc_id, Work::Close { doc: doc_id });
    Ok(())
}

#[tauri::command]
pub async fn set_active_document(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, EngineState>,
    doc_id: Option<DocId>,
) -> AppResult<()> {
    if !window.is_focused().unwrap_or(false) {
        return Ok(());
    }
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

/// The drawn bubbles/checkboxes on one page, for the shading tool.
#[tauri::command]
pub async fn get_page_bubbles(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
) -> AppResult<PageBubblesDto> {
    state
        .0
        .call_async(Priority::AdjacentPage, doc_id, move |respond| {
            Work::PageBubbles {
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

use crate::extensions::{self, builtin, InstalledExtension, Manifest};

fn extensions_root(app: &tauri::AppHandle) -> AppResult<std::path::PathBuf> {
    use tauri::Manager;
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Io(format!("no app data dir: {e}")))?
        .join(extensions::EXTENSIONS_DIR))
}

/// Where the built-ins that ship with this build live, if any do.
fn builtin_root(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    let resources = app.path().resource_dir().ok();
    builtin::root(resources.as_deref())
}

fn builtin_ids(app: &tauri::AppHandle) -> Vec<String> {
    builtin_root(app)
        .map(|r| builtin::ids(&r))
        .unwrap_or_default()
}

fn is_builtin(app: &tauri::AppHandle, id: &str) -> bool {
    builtin_ids(app).iter().any(|b| b == id)
}

/// Which directory holds an extension's files: the read-only bundle for a
/// built-in, the user's extensions directory for everything installed.
fn root_for(app: &tauri::AppHandle, id: &str) -> AppResult<std::path::PathBuf> {
    match builtin_root(app) {
        Some(root) if builtin::ids(&root).iter().any(|b| b == id) => Ok(root),
        _ => extensions_root(app),
    }
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
    /// `speedyf-ext://` URL for the extension's icon, when it ships one.
    pub icon_url: Option<String>,
    /// Why this extension cannot run, if it cannot.
    pub error: Option<String>,
    /// True when the extension ships inside the app. A built-in can be switched
    /// off but not removed, and the panel offers different controls for each.
    pub builtin: bool,
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

/// An icon that would bloat the IPC payload is treated as absent. Real 16px
/// glyphs are a few hundred bytes; this is generous enough that no honest icon
/// hits it and small enough that a mistake stays cheap.
const MAX_ICON_BYTES: u64 = 256 * 1024;

/// An extension's icon as a `data:` URI, or `None` if it has none worth serving.
///
/// Not a `speedyf-ext://` URL, though the scheme would serve the same bytes.
/// The icon is drawn as a CSS mask so it can take the sidebar's own colour, and
/// a mask is subject to origin restrictions an `<img>` is not: a cross-origin
/// one paints nothing whatsoever — no error, no broken-image mark, just an
/// empty square. Measured in the WebKit this ships against, where a `data:`
/// mask renders and a file or custom-scheme mask does not.
///
/// Also the reason there is no load probe on the frontend any more. A missing
/// or oversized file answers `None` here, which is a fact rather than a guess,
/// and the initials fallback keys off it directly.
fn icon_data_uri(root: &std::path::Path, id: &str, icon: &str) -> Option<String> {
    use base64::Engine as _;

    // The same component walk that guards asset serving: a manifest cannot
    // name an icon outside its own directory.
    let path = extensions::resolve_asset(root, &format!("/{id}/{icon}"))?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_ICON_BYTES {
        return None;
    }
    // An allowlist, not a guess: whatever the file is called, it is only
    // embedded if it is one of the image types an icon can actually be.
    let mime = extensions::content_type(&path);
    if !matches!(mime, "image/svg+xml" | "image/png" | "image/jpeg") {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

/// One row for the panel: the registry's record plus everything the manifest
/// currently declares.
///
/// A manifest that will not parse still produces a row. The extension cannot
/// run, but the user needs to see it in order to remove it — and for a built-in
/// they need to see *why*, since they cannot have caused it.
fn describe(root: &std::path::Path, installed: InstalledExtension, builtin: bool) -> ExtensionDto {
    let blank = ExtensionDto {
        installed: installed.clone(),
        manifest: None,
        commands: Vec::new(),
        panels: Vec::new(),
        selection_actions: Vec::new(),
        citation_actions: Vec::new(),
        settings: Vec::new(),
        document_tasks: Vec::new(),
        icon_url: None,
        error: None,
        builtin,
    };
    let manifest_path = root.join(&installed.id).join(extensions::MANIFEST_FILE);
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(e) => {
            return ExtensionDto {
                error: Some(format!("manifest unreadable: {e}")),
                ..blank
            }
        }
    };
    let manifest = match extensions::parse_manifest(&text) {
        Ok(manifest) => manifest,
        Err(e) => {
            return ExtensionDto {
                error: Some(e.to_string()),
                ..blank
            }
        }
    };
    ExtensionDto {
        commands: palette_commands(&manifest),
        panels: panels(&manifest),
        selection_actions: actions(&manifest, &manifest.contributes.selection_actions),
        citation_actions: actions(&manifest, &manifest.contributes.citation_actions),
        settings: manifest.contributes.settings.clone(),
        document_tasks: actions(&manifest, &manifest.contributes.document_tasks),
        icon_url: manifest
            .icon
            .as_ref()
            .and_then(|icon| icon_data_uri(root, &manifest.id, icon)),
        manifest: Some(manifest),
        ..blank
    }
}

/// A built-in as a registry row.
///
/// Built-ins are not in `installed.json` and never will be — the identity is
/// read from the manifest each time, so a release that renames or re-describes
/// one needs no migration. Only the enabled bit is persisted, and that lives in
/// its own file beside the registry.
fn builtin_row(root: &std::path::Path, id: &str, enabled: bool) -> InstalledExtension {
    let manifest = std::fs::read_to_string(root.join(id).join(extensions::MANIFEST_FILE))
        .ok()
        .and_then(|text| extensions::parse_manifest(&text).ok());
    InstalledExtension {
        id: id.to_string(),
        name: manifest
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| id.to_string()),
        version: manifest
            .as_ref()
            .map(|m| m.version.clone())
            .unwrap_or_default(),
        description: manifest.as_ref().and_then(|m| m.description.clone()),
        source: "built in".to_string(),
        // A built-in's permissions are whatever it ships asking for. There is no
        // install moment at which the user granted a narrower set, and the app
        // is trusted with them already by virtue of being the app.
        permissions: manifest.map(|m| m.permissions).unwrap_or_default(),
        enabled,
    }
}

#[tauri::command]
pub async fn list_extensions(app: tauri::AppHandle) -> AppResult<Vec<ExtensionDto>> {
    let root = extensions_root(&app)?;
    // Built-ins first: they are the ones a new user has, and a list that opens
    // with them reads as "here is what SpeedyF does" rather than as an
    // alphabetical accident.
    let mut out = Vec::new();
    if let Some(builtin_root) = builtin_root(&app) {
        let state = builtin::State::load(&root);
        for id in builtin::ids(&builtin_root) {
            let enabled = state.is_enabled(&id);
            let row = builtin_row(&builtin_root, &id, enabled);
            out.push(describe(&builtin_root, row, true));
        }
    }
    let installed = extensions::Registry::load(&root).extensions;
    out.extend(
        installed
            .into_iter()
            .map(|installed| describe(&root, installed, false)),
    );
    Ok(out)
}

/// Refuse an install that would collide with a built-in.
///
/// Checked after the fact because a git install only learns its own id once it
/// has cloned. The files are removed again, so a refused install leaves nothing
/// behind — the same guarantee `install_from_folder` makes for a bad manifest.
fn refuse_builtin_collision(
    app: &tauri::AppHandle,
    root: &std::path::Path,
    entry: InstalledExtension,
) -> AppResult<InstalledExtension> {
    if is_builtin(app, &entry.id) {
        let _ = extensions::uninstall(root, &entry.id);
        return Err(AppError::Malformed(format!(
            "\"{}\" ships with SpeedyF — disable the built-in rather than installing over it",
            entry.id
        )));
    }
    Ok(entry)
}

#[tauri::command]
pub async fn install_extension(
    app: tauri::AppHandle,
    folder: String,
) -> AppResult<InstalledExtension> {
    let root = extensions_root(&app)?;
    let entry = extensions::install_from_folder(&root, std::path::Path::new(&folder))?;
    refuse_builtin_collision(&app, &root, entry)
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
    let cloned = root.clone();
    let entry =
        tauri::async_runtime::spawn_blocking(move || extensions::install_from_git(&cloned, &url))
            .await
            .map_err(|e| AppError::Internal(format!("install task failed: {e}")))??;
    refuse_builtin_collision(&app, &root, entry)
}

#[tauri::command]
pub async fn uninstall_extension(app: tauri::AppHandle, id: String) -> AppResult<()> {
    // The files are inside the app bundle, which is read-only and signed. Even
    // if it were writable, removing them would only last until the next update.
    if is_builtin(&app, &id) {
        return Err(AppError::Malformed(format!(
            "\"{id}\" ships with SpeedyF and cannot be removed — disable it instead"
        )));
    }
    extensions::uninstall(&extensions_root(&app)?, &id)
}

#[tauri::command]
pub async fn set_extension_enabled(
    app: tauri::AppHandle,
    id: String,
    enabled: bool,
) -> AppResult<()> {
    let root = extensions_root(&app)?;
    // A built-in has no registry row to carry the bit, so it is kept in a file
    // of its own. See `extensions::builtin` for why it is not in the registry.
    if is_builtin(&app, &id) {
        return builtin::set_enabled(&root, &id, enabled);
    }
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
    let root = root_for(&app, &id)?;
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

#[cfg(test)]
mod icon_tests {
    use super::*;

    fn plant_icon(root: &std::path::Path, id: &str, name: &str, bytes: &[u8]) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    #[test]
    fn an_svg_icon_becomes_a_data_uri() {
        let tmp = tempfile::tempdir().unwrap();
        plant_icon(tmp.path(), "dev.speedyf.thing", "icon.svg", b"<svg/>");
        let uri = icon_data_uri(tmp.path(), "dev.speedyf.thing", "icon.svg").unwrap();
        // A `speedyf-ext://` URL would be silently blank as a CSS mask; the
        // scheme here is the whole point of the function.
        assert_eq!(uri, "data:image/svg+xml;base64,PHN2Zy8+");
    }

    #[test]
    fn a_png_icon_keeps_its_own_type() {
        let tmp = tempfile::tempdir().unwrap();
        plant_icon(tmp.path(), "dev.speedyf.thing", "icon.png", b"\x89PNG");
        let uri = icon_data_uri(tmp.path(), "dev.speedyf.thing", "icon.png").unwrap();
        assert!(uri.starts_with("data:image/png;base64,"), "{uri}");
    }

    #[test]
    fn a_missing_icon_is_none_rather_than_an_empty_uri() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("dev.speedyf.thing")).unwrap();
        // The manifest may name a file that was deleted afterwards. `None` is
        // what drives the initials fallback, so it has to be `None`.
        assert!(icon_data_uri(tmp.path(), "dev.speedyf.thing", "icon.svg").is_none());
    }

    #[test]
    fn a_non_image_is_refused_whatever_the_manifest_calls_it() {
        let tmp = tempfile::tempdir().unwrap();
        plant_icon(
            tmp.path(),
            "dev.speedyf.thing",
            "worker.js",
            b"self.close()",
        );
        assert!(icon_data_uri(tmp.path(), "dev.speedyf.thing", "worker.js").is_none());
    }

    #[test]
    fn an_icon_outside_the_extension_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        plant_icon(tmp.path(), "dev.speedyf.thing", "icon.svg", b"<svg/>");
        std::fs::write(tmp.path().join("secret.svg"), b"<svg/>").unwrap();
        // Guarded by the same component walk that guards asset serving.
        assert!(icon_data_uri(tmp.path(), "dev.speedyf.thing", "../secret.svg").is_none());
    }

    #[test]
    fn an_oversized_icon_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let huge = vec![b'a'; (MAX_ICON_BYTES + 1) as usize];
        plant_icon(tmp.path(), "dev.speedyf.thing", "icon.png", &huge);
        assert!(icon_data_uri(tmp.path(), "dev.speedyf.thing", "icon.png").is_none());
    }

    /// The icon as it actually ships, encoded the way the panel will receive it.
    #[test]
    fn the_built_in_icon_encodes() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(crate::extensions::builtin::BUILTIN_DIR);
        let uri = icon_data_uri(&root, "dev.speedyf.references", "icon.svg")
            .expect("the shipped References icon must encode");
        assert!(uri.starts_with("data:image/svg+xml;base64,"), "{uri}");
    }
}

//! Commands for papers and projects (projects.md sec. 9). Every one goes through
//! the engine's single paper store, and every one that writes saves on the
//! spot and then tells every window through `library:changed`.
//!
//! Store work runs on a blocking thread, like `change_library_root`: it is
//! file I/O, and the native IPC callback must never wait on it. The two
//! commands that need PDFium - a first-page preview and the scan of a newly
//! filed PDF - go through the engine thread like any other document work.
//! The online lookups await their requests here, on the async runtime, each
//! through the engine's per-host limiter (`library::ratelimit`), and
//! take the store lock only to apply what came back.

use super::EngineState;
use crate::engine::types::{DocId, Priority, ResolvedCitationDto};
use crate::engine::{EngineHandle, Work};
use crate::errors::{AppError, AppResult};
use crate::library::backup::LibraryBackupDto;
use crate::library::bibtex::BibtexOptions;
use crate::library::bundle::{self, BundleReportDto};
use crate::library::chain::{
    apply_title_candidates, drive_batch, without_declined, BatchTurns, Chain, ChainKind, EdgeJob,
    TitleOutcome, Turn, PARK_LIMIT,
};
use crate::library::filing;
use crate::library::lookup;
use crate::library::papers::normalize_doi;
use crate::library::papers::{LookupPlan, LookupTarget};
use crate::library::store::PaperStore;
use crate::library::types::{
    CitationLinksDto, DuplicateGroupDto, GraphDto, LibraryCountsDto, LibraryDataInfoDto,
    LookupBatchDto, LookupCandidateDto, LookupMode, LookupResultDto, MetaPatchDto, MissingWorkDto,
    PaperDto, PaperFilterDto, PaperListDto, PapernessDto, ProjectDto, ProjectSuggestionDto,
};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `f` against the store on a blocking thread.
async fn read_store<T: Send + 'static>(
    engine: EngineHandle,
    f: impl FnOnce(&PaperStore) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = engine.0.papers.lock();
        f(&store)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Run `f` against the store, save, then tell the windows. The store lock is
/// released before the event goes out, since a window answers it by reading
/// the store back.
async fn write_store<T: Send + 'static>(
    engine: EngineHandle,
    f: impl FnOnce(&mut PaperStore) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let result = {
            let mut store = engine.0.papers.lock();
            let result = f(&mut store)?;
            store.save()?;
            result
        };
        engine.0.notify_library_changed();
        Ok(result)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Run `f` against the store; save and tell the windows only when it says
/// something changed. For writes that are usually nothing of the kind:
/// copying BibTeX pins keys the first time and never again.
async fn write_store_if_changed<T: Send + 'static>(
    engine: EngineHandle,
    f: impl FnOnce(&mut PaperStore) -> AppResult<(T, bool)> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let (result, changed) = {
            let mut store = engine.0.papers.lock();
            let (result, changed) = f(&mut store)?;
            if changed {
                store.save()?;
            }
            (result, changed)
        };
        if changed {
            engine.0.notify_library_changed();
        }
        Ok(result)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

#[tauri::command]
pub async fn library_papers(
    state: tauri::State<'_, EngineState>,
    filter: PaperFilterDto,
) -> AppResult<PaperListDto> {
    read_store(state.0.clone(), move |store| Ok(store.paper_rows(&filter))).await
}

#[tauri::command]
pub async fn library_counts(state: tauri::State<'_, EngineState>) -> AppResult<LibraryCountsDto> {
    read_store(state.0.clone(), |store| Ok(store.counts())).await
}

#[tauri::command]
pub async fn library_paper(
    state: tauri::State<'_, EngineState>,
    id: String,
) -> AppResult<PaperDto> {
    read_store(state.0.clone(), move |store| store.paper_dto(&id)).await
}

/// The paper whose file is at `path`, or none when the path is not in the
/// library - which is how the reader's Paper panel decides between showing
/// the paper and offering to add it.
#[tauri::command]
pub async fn library_paper_for_path(
    state: tauri::State<'_, EngineState>,
    path: String,
) -> AppResult<Option<PaperDto>> {
    read_store(state.0.clone(), move |store| {
        let path = PathBuf::from(path);
        let canonical = path.canonicalize().unwrap_or(path);
        Ok(store.paper_for_path(&canonical))
    })
    .await
}

#[tauri::command]
pub async fn library_update_paper(
    state: tauri::State<'_, EngineState>,
    id: String,
    patch: MetaPatchDto,
) -> AppResult<PaperDto> {
    write_store(state.0.clone(), move |store| store.apply_patch(&id, patch)).await
}

#[tauri::command]
pub async fn library_projects(state: tauri::State<'_, EngineState>) -> AppResult<Vec<ProjectDto>> {
    read_store(state.0.clone(), |store| Ok(store.projects())).await
}

#[tauri::command]
pub async fn library_create_project(
    state: tauri::State<'_, EngineState>,
    name: String,
) -> AppResult<ProjectDto> {
    write_store(state.0.clone(), move |store| store.create_project(&name)).await
}

#[tauri::command]
pub async fn library_rename_project(
    state: tauri::State<'_, EngineState>,
    id: String,
    name: String,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| {
        store.rename_project(&id, &name)
    })
    .await
}

#[tauri::command]
pub async fn library_delete_project(
    state: tauri::State<'_, EngineState>,
    id: String,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| store.delete_project(&id)).await
}

#[tauri::command]
pub async fn library_reorder_projects(
    state: tauri::State<'_, EngineState>,
    ids: Vec<String>,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| store.reorder_projects(&ids)).await
}

#[tauri::command]
pub async fn library_set_membership(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
    project_id: String,
    member: bool,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| {
        store.set_membership(&paper_id, &project_id, member)
    })
    .await
}

/// The paper's first file that is on disk and inside a library folder,
/// opened as a library document. None when there is no such file.
#[tauri::command]
pub async fn library_preview(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
) -> AppResult<Option<ResolvedCitationDto>> {
    let engine = state.0.clone();
    let files = read_store(engine.clone(), move |store| store.present_files(&paper_id)).await?;
    let path = {
        let library = engine.0.library.lock();
        files.iter().find_map(|file| library.contains(file))
    };
    let Some(path) = path else {
        return Ok(None);
    };
    let main_epoch = engine.main_epoch();
    engine
        .call_async(Priority::HoverPreview, 0, move |respond| {
            Work::LibraryPreview {
                path,
                main_epoch,
                respond,
            }
        })
        .await
}

/// Copy a PDF from outside the library into `inbox_dir`, scan the copy and
/// return its paper. Never a move, never a delete.
#[tauri::command]
pub async fn library_file_into_inbox(
    state: tauri::State<'_, EngineState>,
    path: String,
    inbox_dir: String,
) -> AppResult<PaperDto> {
    let engine = state.0.clone();
    let roots = engine.0.library.lock().roots();
    let filed = tauri::async_runtime::spawn_blocking(move || {
        filing::copy_into_inbox(&PathBuf::from(path), &PathBuf::from(inbox_dir), &roots)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))??;
    // The user is waiting on this one, unlike the idle scan.
    engine
        .call_async(Priority::HoverPreview, 0, move |respond| {
            Work::LibraryScanFile {
                path: filed,
                respond,
            }
        })
        .await
}

/// Sync to a .bib...: record the path, and write the file now - the save
/// behind `write_store` is what writes every synced `.bib`. With no path
/// the project just stops syncing; the file it wrote stays as it is.
#[tauri::command]
pub async fn library_set_project_bib(
    state: tauri::State<'_, EngineState>,
    project_id: String,
    path: Option<String>,
) -> AppResult<()> {
    let engine = state.0.clone();
    let id = project_id.clone();
    write_store(engine.clone(), move |store| {
        store.set_project_bib(&project_id, path)
    })
    .await?;
    // The save wrote the file, or tried to. A path that cannot be written -
    // a folder that does not exist, an unmounted volume - is the one thing
    // the user needs to hear about now, not in the log.
    if let Some(error) = read_store(engine, move |store| Ok(store.bib_error_for(&id))).await? {
        return Err(AppError::Io(format!(
            "the .bib path was kept, but it cannot be written: {error}"
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn library_set_lookup(
    state: tauri::State<'_, EngineState>,
    enabled: bool,
    contact: String,
    s2_key: String,
) -> AppResult<()> {
    state.0.set_lookup_settings(enabled, contact, s2_key);
    Ok(())
}

#[tauri::command]
pub async fn library_data_dir(
    state: tauri::State<'_, EngineState>,
) -> AppResult<LibraryDataInfoDto> {
    read_store(state.0.clone(), |store| Ok(store.data_info())).await
}

/* ---- Backups of library.json (projects.md sec. 3) ---- */

/// The backups of `library.json`, newest first.
#[tauri::command]
pub async fn library_backups(
    state: tauri::State<'_, EngineState>,
) -> AppResult<Vec<LibraryBackupDto>> {
    read_store(state.0.clone(), |store| Ok(store.backups())).await
}

/// Put the library back as a backup holds it, after backing up the
/// current state (`PaperStore::restore_backup`).
#[tauri::command]
pub async fn library_restore_backup(
    state: tauri::State<'_, EngineState>,
    file: String,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| store.restore_backup(&file)).await
}

/// Switch the data folder. Adopts a `library.json` already there, otherwise
/// copies the current one; either way the windows reload afterwards.
#[tauri::command]
pub async fn library_move_data(
    state: tauri::State<'_, EngineState>,
    dir: String,
) -> AppResult<LibraryDataInfoDto> {
    let engine = state.0.clone();
    let info = tauri::async_runtime::spawn_blocking(move || {
        let result = engine.0.papers.lock().move_data(PathBuf::from(dir));
        engine.0.notify_library_changed();
        result
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))??;
    Ok(info)
}

/* ---- BibTeX (projects.md sec. 7) ---- */

/// BibTeX for these papers, in this order, for the clipboard. It freezes
/// their keys: the text is about to be pasted into a `.bib`, and a key is
/// frozen the first time it is written to any (projects.md sec. 2).
#[tauri::command]
pub async fn library_bibtex(
    state: tauri::State<'_, EngineState>,
    paper_ids: Vec<String>,
) -> AppResult<String> {
    write_store_if_changed(state.0.clone(), move |store| store.bibtex_for(&paper_ids)).await
}

/// How BibTeX is written (projects.md sec. 7).
#[tauri::command]
pub async fn library_bibtex_options(
    state: tauri::State<'_, EngineState>,
) -> AppResult<BibtexOptions> {
    read_store(state.0.clone(), |store| Ok(store.bibtex_options())).await
}

/// Change how BibTeX is written; the save rewrites the synced `.bib` files.
#[tauri::command]
pub async fn library_set_bibtex_options(
    state: tauri::State<'_, EngineState>,
    options: BibtexOptions,
) -> AppResult<BibtexOptions> {
    write_store(state.0.clone(), move |store| {
        store.set_bibtex_options(options)
    })
    .await
}

/// Pin these papers' keys without rendering anything: the keys were just
/// copied as a `\cite{}` to be pasted into a manuscript, where a key that
/// changes later breaks the citation (projects.md sec. 2). Ids that name no
/// paper are skipped; saves and notifies only when a key was not pinned.
#[tauri::command]
pub async fn library_pin_keys(
    state: tauri::State<'_, EngineState>,
    paper_ids: Vec<String>,
) -> AppResult<()> {
    write_store_if_changed(state.0.clone(), move |store| {
        Ok(((), store.freeze_keys(&paper_ids)))
    })
    .await
}

/// Export .bib...: the project's entries, written once to `path`.
#[tauri::command]
pub async fn library_export_bib(
    state: tauri::State<'_, EngineState>,
    project_id: String,
    path: String,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| {
        store.export_project_bib(&project_id, &PathBuf::from(path))
    })
    .await
}

/// Export bundle (.zip)...: the project's `.bib` and each item's main PDF,
/// written to `path` (projects.md sec. 5). The plan is read under the store
/// lock; the PDFs are copied without it, so a large bundle never holds up
/// the library; the keys are pinned once the zip is in place, as an
/// Export .bib pins them.
#[tauri::command]
pub async fn library_export_bundle(
    state: tauri::State<'_, EngineState>,
    project_id: String,
    path: String,
) -> AppResult<BundleReportDto> {
    let engine = state.0.clone();
    let plan = read_store(engine.clone(), move |store| {
        store.project_bundle_plan(&project_id)
    })
    .await?;
    let ids = plan.paper_ids.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        bundle::write_bundle(&plan, &PathBuf::from(path))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))??;
    write_store_if_changed(engine, move |store| Ok(((), store.freeze_keys(&ids)))).await?;
    Ok(report)
}

/* ---- Online lookup (projects.md sec. 4) ---- */

/// A chain over the engine's client, limiter, cache and live settings. One
/// per lookup or batch.
fn chain_for(engine: &EngineHandle) -> Chain {
    Chain::new(engine.chain_shared())
}

/// Run `f` against the store and save, telling nobody: for a change no
/// window shows, such as the stamp of when a paper was last looked up.
async fn save_store<T: Send + 'static>(
    engine: EngineHandle,
    f: impl FnOnce(&mut PaperStore) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = engine.0.papers.lock();
        let result = f(&mut store)?;
        store.save()?;
        Ok(result)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Run `f` against the store for one paper of a batch. The change is
/// marked and reaches the disk through the scan's throttled flush - at
/// most once a second, without the `.bib` sync - and the windows hear of
/// it then, so a long batch fills the list in as it goes without a save, a
/// bib comparison and a broadcast per paper. `flush_store` ends the batch.
async fn write_store_throttled<T: Send + 'static>(
    engine: EngineHandle,
    f: impl FnOnce(&mut PaperStore) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let (result, saved) = {
            let mut store = engine.0.papers.lock();
            let result = f(&mut store)?;
            store.mark_dirty();
            let saved = store.flush(false)?;
            (result, saved)
        };
        if saved {
            engine.0.notify_library_changed();
        }
        Ok(result)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// The batch's final save, `.bib` sync included.
async fn flush_store(engine: EngineHandle) -> AppResult<bool> {
    tauri::async_runtime::spawn_blocking(move || engine.0.papers.lock().flush(true))
        .await
        .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// The lookup phase is over: its unsaved answers are written now, and the
/// citation edges it asked for are fetched in the background - the
/// lookup has answered, and the status bar stays lit until these land in
/// the cache. Nothing waits on them.
fn fetch_edges_later(engine: &EngineHandle, chain: Chain, jobs: Vec<EdgeJob>) {
    chain.flush();
    if jobs.is_empty() {
        return;
    }
    let hold = engine.hold_lookup();
    tauri::async_runtime::spawn(async move {
        chain.fetch_edges(jobs).await;
        drop(hold);
    });
}

fn lookup_error(errors: &[String]) -> AppError {
    AppError::Io(errors.join("; "))
}

/// Look one paper up online. `Id`: the identifier chain (Crossref,
/// DataCite, Semantic Scholar, INSPIRE, OpenAlex, arXiv, Open Library, as
/// the paper's identifiers and subject warrant), every answer applied at
/// its source's rank. `Title`: every title search at once, merged, less
/// what the user declined before, for the user to pick from - and parked
/// on the paper meanwhile; applied by `library_apply_candidate`. Refuses
/// when lookup is off; a request that fails with nothing found is an
/// error; nothing found - or nothing to look up with - is `Nothing`. The
/// paper is stamped as looked up only when every service was asked and
/// answered.
#[tauri::command]
pub async fn library_lookup(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
    mode: LookupMode,
) -> AppResult<LookupResultDto> {
    let engine = state.0.clone();
    lookup::ensure_enabled(&engine.lookup_settings())?;
    let _activity = engine.0.lookup_activity.start();
    let target = {
        let id = paper_id.clone();
        read_store(engine.clone(), move |store| store.lookup_target(&id)).await?
    };
    let chain = chain_for(&engine);
    match mode {
        LookupMode::Id => {
            let outcome = chain.lookup_by_identifiers(&target).await;
            fetch_edges_later(&engine, chain, outcome.edges.into_iter().collect());
            let settled = outcome.errors.is_empty() && outcome.complete;
            if outcome.candidates.is_empty() {
                if !outcome.errors.is_empty() {
                    return Err(lookup_error(&outcome.errors));
                }
                if settled {
                    save_store(engine.clone(), move |store| store.mark_looked_up(&paper_id))
                        .await?;
                }
                return Ok(LookupResultDto::Nothing);
            }
            let candidates = outcome.candidates;
            let paper = write_store(engine.clone(), move |store| {
                let paper = store.apply_candidates(&paper_id, &candidates)?;
                if settled {
                    store.mark_looked_up(&paper_id)?;
                }
                Ok(paper)
            })
            .await?;
            Ok(LookupResultDto::Applied { paper })
        }
        LookupMode::Title => {
            if target.title.is_none() {
                return Err(AppError::Internal(
                    "This paper has no title to search for; type one in first".into(),
                ));
            }
            let search = chain.search_by_title_everywhere(&target).await;
            chain.flush();
            let settled = search.errors.is_empty() && search.complete;
            let mut candidates = without_declined(search.candidates, &target.declined);
            if candidates.is_empty() {
                if !search.errors.is_empty() {
                    return Err(lookup_error(&search.errors));
                }
                if settled {
                    save_store(engine.clone(), move |store| store.mark_looked_up(&paper_id))
                        .await?;
                }
                return Ok(LookupResultDto::Nothing);
            }
            candidates.truncate(PARK_LIMIT);
            let parked = candidates.clone();
            write_store(engine.clone(), move |store| {
                store.park_candidates(&paper_id, parked)?;
                if settled {
                    store.mark_looked_up(&paper_id)?;
                }
                Ok(())
            })
            .await?;
            Ok(LookupResultDto::Candidates { candidates })
        }
    }
}

/// The candidate the user picked from a title search, applied as a lookup
/// result is: field by field, at its source's rank. Applying one also
/// clears any candidates parked on the paper.
#[tauri::command]
pub async fn library_apply_candidate(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
    candidate: LookupCandidateDto,
) -> AppResult<PaperDto> {
    write_store(state.0.clone(), move |store| {
        store.apply_candidate(&paper_id, &candidate)
    })
    .await
}

/// None of the parked title-search candidates was it: drop them, and
/// remember not to park them again.
#[tauri::command]
pub async fn library_discard_candidates(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
) -> AppResult<PaperDto> {
    write_store(state.0.clone(), move |store| {
        store.discard_candidates(&paper_id)
    })
    .await
}

/// One paper of a batch: its chain, then the store through the throttled
/// write. A request that failed is logged and the turn counted as failed
/// only when nothing came of it; the paper is stamped as looked up only
/// when every service was asked and answered, so a failure or a host given
/// up on is retried by the next batch.
async fn batch_turn(
    engine: &EngineHandle,
    chain: &Chain,
    kind: ChainKind,
    target: LookupTarget,
    edges: &mut Vec<EdgeJob>,
) -> Turn {
    let paper_id = target.paper_id.clone();
    let written: AppResult<Turn> = match kind {
        ChainKind::Identifier => {
            let outcome = chain.lookup_by_identifiers(&target).await;
            edges.extend(outcome.edges);
            let settled = outcome.errors.is_empty() && outcome.complete;
            if outcome.candidates.is_empty() {
                if settled {
                    write_store_throttled(engine.clone(), move |store| {
                        store.mark_looked_up(&paper_id)
                    })
                    .await
                    .map(|()| Turn::Nothing)
                } else if outcome.errors.is_empty() {
                    Ok(Turn::Nothing)
                } else {
                    Ok(Turn::Failed)
                }
            } else {
                let candidates = outcome.candidates;
                write_store_throttled(engine.clone(), move |store| {
                    store.apply_candidates(&paper_id, &candidates)?;
                    if settled {
                        store.mark_looked_up(&paper_id)?;
                    }
                    Ok(Turn::Applied)
                })
                .await
            }
        }
        ChainKind::Title => {
            let Some(title) = target.title.clone() else {
                return Turn::Nothing;
            };
            let search = chain.search_by_title_everywhere(&target).await;
            let settled = search.errors.is_empty() && search.complete;
            match apply_title_candidates(&title, target.year, search.candidates, &target.declined) {
                TitleOutcome::Apply(candidate) => {
                    write_store_throttled(engine.clone(), move |store| {
                        store.apply_candidate(&paper_id, &candidate)?;
                        if settled {
                            store.mark_looked_up(&paper_id)?;
                        }
                        Ok(Turn::Applied)
                    })
                    .await
                }
                TitleOutcome::Park(candidates) => {
                    write_store_throttled(engine.clone(), move |store| {
                        store.park_candidates(&paper_id, candidates)?;
                        if settled {
                            store.mark_looked_up(&paper_id)?;
                        }
                        Ok(Turn::Parked)
                    })
                    .await
                }
                TitleOutcome::Nothing if settled => {
                    write_store_throttled(engine.clone(), move |store| {
                        store.mark_looked_up(&paper_id)
                    })
                    .await
                    .map(|()| Turn::Nothing)
                }
                TitleOutcome::Nothing if search.errors.is_empty() => Ok(Turn::Nothing),
                TitleOutcome::Nothing => Ok(Turn::Failed),
            }
        }
    };
    match written {
        Ok(turn) => turn,
        Err(error) => {
            log::warn!("lookup of paper {} failed: {error}", target.paper_id);
            Turn::Failed
        }
    }
}

/// The engine as a batch host: the turns run the chains and the throttled
/// writes, lookup is read live between turns, progress goes to the gauge.
struct EngineTurns<'a> {
    engine: EngineHandle,
    chain: &'a Chain,
    edges: Vec<EdgeJob>,
}

impl BatchTurns for EngineTurns<'_> {
    fn enabled(&self) -> bool {
        self.engine.lookup_settings().enabled
    }

    fn turn(&mut self, kind: ChainKind, target: LookupTarget) -> impl Future<Output = Turn> + Send {
        batch_turn(&self.engine, self.chain, kind, target, &mut self.edges)
    }

    fn progress(&self, done: u32, total: u32) {
        self.engine.0.lookup_activity.progress(done, total);
    }
}

/// Run a batch plan: one paper at a time (the limiter spaces the requests
/// per host), each result applied as it arrives and reaching the windows
/// through the throttled flush, progress reported step by step, then one
/// final save with the `.bib` sync and one `library:changed`. Turning
/// lookup off ends the batch at the next paper. The citation edges the
/// lookups asked for are fetched afterwards in the background, with the
/// indicator still lit.
async fn run_batch(engine: EngineHandle, plan: LookupPlan) -> LookupBatchDto {
    if plan.targets.is_empty() {
        // Nothing to ask about lights no indicator.
        return LookupBatchDto {
            skipped: plan.skipped,
            ..LookupBatchDto::default()
        };
    }
    let activity = engine.0.lookup_activity.start();
    let chain = chain_for(&engine);
    let mut turns = EngineTurns {
        engine: engine.clone(),
        chain: &chain,
        edges: Vec::new(),
    };
    let batch = drive_batch(plan, &mut turns).await;
    let edges = std::mem::take(&mut turns.edges);
    if let Err(error) = flush_store(engine.clone()).await {
        log::warn!("cannot save the paper library after the lookup batch: {error}");
    }
    engine.0.notify_library_changed();
    if !edges.is_empty() {
        // The edge phase: lit, without a count that would otherwise sit at
        // "n of n" until the last edge is in.
        engine.0.lookup_activity.progress(0, 0);
    }
    fetch_edges_later(&engine, chain, edges);
    drop(activity);
    batch
}

/// Look up every paper in the project that a batch should ask about
/// (`PaperRecord::batch_chain`): by identifier where one can add
/// something, by title otherwise, never a record that is not a paper.
#[tauri::command]
pub async fn library_lookup_project(
    state: tauri::State<'_, EngineState>,
    project_id: String,
) -> AppResult<LookupBatchDto> {
    let engine = state.0.clone();
    lookup::ensure_enabled(&engine.lookup_settings())?;
    let now = unix_now();
    let plan = read_store(engine.clone(), move |store| {
        store.plan_project_lookup(&project_id, now)
    })
    .await?;
    Ok(run_batch(engine, plan).await)
}

/// The same over the whole library.
#[tauri::command]
pub async fn library_lookup_all(state: tauri::State<'_, EngineState>) -> AppResult<LookupBatchDto> {
    let engine = state.0.clone();
    lookup::ensure_enabled(&engine.lookup_settings())?;
    let now = unix_now();
    let plan = read_store(engine.clone(), move |store| {
        Ok(store.plan_library_lookup(now))
    })
    .await?;
    Ok(run_batch(engine, plan).await)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/* ---- Citation graph (projects.md sec. 8) ---- */

/// Which library papers this one cites and which cite it, through the
/// identifiers the text pass found in each file and the edges the lookup
/// services supplied, plus how many cited identifiers match no paper.
/// Empty lists until the text pass has reached the files involved or a
/// lookup has run.
#[tauri::command]
pub async fn library_citation_links(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
) -> AppResult<CitationLinksDto> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // The index and the cache are snapshots the store resolves against.
        let (entries, refs, network) = citation_inputs(&engine);
        let store = engine.0.papers.lock();
        store.citation_links(&paper_id, &entries, &refs, &network)
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// What citation resolution reads besides the store: the index entries,
/// the active provider's reference lists, and the network's edges. The
/// locks in turn, never nested.
fn citation_inputs(
    engine: &EngineHandle,
) -> (
    Vec<crate::library::LibraryEntry>,
    crate::library::references::DocRefs,
    crate::library::lookup::NetworkEdges,
) {
    let (entries, app_data_dir) = {
        let library = engine.0.library.lock();
        (library.all_entries(), library.app_data_dir())
    };
    let provider = engine.0.refs_provider.lock().active().map(str::to_owned);
    let refs = engine
        .0
        .refs_cache
        .lock()
        .load(&app_data_dir, &entries, provider.as_deref());
    let network = engine.0.lookup_cache.lock().network_edges();
    (entries, refs, network)
}

/// The citation graph of one project's members, or with no project of
/// every item in the library (projects.md sec. 8): the nodes in list order,
/// the edges between them, and how many citations point outside the set
/// or are not known yet. With `focus`, the graph around that paper instead -
/// it and every item within `depth` (1 or 2) citation steps across the
/// whole library - and `project_id` is ignored. Read-only, against the
/// same snapshots of the index and the lookup cache that
/// `library_citation_links` resolves against.
#[tauri::command]
pub async fn library_graph(
    state: tauri::State<'_, EngineState>,
    project_id: Option<String>,
    focus: Option<String>,
    depth: Option<u32>,
) -> AppResult<GraphDto> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (entries, refs, network) = citation_inputs(&engine);
        let store = engine.0.papers.lock();
        match focus {
            Some(focus) => store.focus_graph(&focus, depth.unwrap_or(1), &entries, &refs, &network),
            None => store.citation_graph(project_id.as_deref(), &entries, &refs, &network),
        }
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Works the view's items cite that are not in the library, cited by at
/// least `min_cited_by` of them: the view is `focus`'s neighbourhood
/// (`depth` steps, as `library_graph`), else the project's members, else
/// every item (`PaperStore::missing_works`).
#[tauri::command]
pub async fn library_missing_works(
    state: tauri::State<'_, EngineState>,
    project_id: Option<String>,
    min_cited_by: u32,
    focus: Option<String>,
    depth: Option<u32>,
) -> AppResult<Vec<MissingWorkDto>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (entries, refs, network) = citation_inputs(&engine);
        let store = engine.0.papers.lock();
        store.missing_works(
            project_id.as_deref(),
            focus.as_deref(),
            depth.unwrap_or(1),
            min_cited_by,
            &entries,
            &refs,
            &network,
        )
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Projects the item is not in that the citation graph ties it to, at
/// most three, strongest first (`PaperStore::suggest_projects`). Empty for
/// an id that names no item. Read-only, against the same snapshots as
/// `library_citation_links`.
#[tauri::command]
pub async fn library_suggest_projects(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
) -> AppResult<Vec<ProjectSuggestionDto>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (entries, refs, network) = citation_inputs(&engine);
        let store = engine.0.papers.lock();
        Ok(store.suggest_projects(&paper_id, &entries, &refs, &network))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// The items most related to this one by the citation graph - works both
/// cite, items citing both - at most five, strongest first
/// (`PaperStore::related_items`). Empty for an id that names no item.
/// Read-only and offline, against the same snapshots as
/// `library_citation_links`.
#[tauri::command]
pub async fn library_related_items(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
) -> AppResult<Vec<crate::library::types::RelatedItemDto>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (entries, refs, network) = citation_inputs(&engine);
        let store = engine.0.papers.lock();
        Ok(store.related_items(&paper_id, &entries, &refs, &network))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/* ---- Duplicates and merging (projects.md sec. 2) ---- */

/// Groups of items that look like one work. The lookup cache supplies
/// what arXiv said each preprint was published as.
#[tauri::command]
pub async fn library_duplicates(
    state: tauri::State<'_, EngineState>,
) -> AppResult<Vec<DuplicateGroupDto>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let arxiv_ids: Vec<String> = {
            let store = engine.0.papers.lock();
            store.arxiv_ids()
        };
        let preprint_dois: HashMap<String, String> = {
            let cache = engine.0.lookup_cache.lock();
            arxiv_ids
                .into_iter()
                .filter_map(|arxiv| {
                    let doi = cache
                        .get(&lookup::LookupCache::arxiv_key(&arxiv))?
                        .doi
                        .as_deref()
                        .and_then(normalize_doi)?;
                    Some((arxiv, doi))
                })
                .collect()
        };
        let store = engine.0.papers.lock();
        Ok(store.duplicates(&preprint_dois))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Merge `merge_ids` into `keep_id` (`PaperStore::merge_papers`).
#[tauri::command]
pub async fn library_merge_papers(
    state: tauri::State<'_, EngineState>,
    keep_id: String,
    merge_ids: Vec<String>,
) -> AppResult<PaperDto> {
    write_store(state.0.clone(), move |store| {
        store.merge_papers(&keep_id, &merge_ids)
    })
    .await
}

/// Make one of an item's files its main one: opened, previewed and read
/// for fields (`PaperStore::set_primary_file`).
#[tauri::command]
pub async fn library_set_primary_file(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
    path: String,
) -> AppResult<PaperDto> {
    write_store(state.0.clone(), move |store| {
        store.set_primary_file(&paper_id, &path)
    })
    .await
}

/// Split a merged record back into the records merged into it. Returns
/// the restored records, then this one.
#[tauri::command]
pub async fn library_unmerge(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
) -> AppResult<Vec<PaperDto>> {
    write_store(state.0.clone(), move |store| store.unmerge(&paper_id)).await
}

/// The user says these records are not duplicates of one another.
#[tauri::command]
pub async fn library_not_duplicates(
    state: tauri::State<'_, EngineState>,
    paper_ids: Vec<String>,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| {
        store.mark_not_duplicates(&paper_ids)
    })
    .await
}

/* ---- Reference provider (EXTENSIONS.md, "referenceProvider") ---- */

/// Which extension reads reference lists now, as `<id>@<version>`, or none.
/// Set by the window whenever the extensions change. With none, no refs
/// sidecar is read and nothing is handed out; a new version makes every
/// document pending again.
#[tauri::command]
pub async fn library_refs_provider(
    state: tauri::State<'_, EngineState>,
    provider: Option<String>,
) -> AppResult<()> {
    let provider = provider
        .map(|provider| provider.trim().to_owned())
        .filter(|provider| !provider.is_empty() && provider.len() <= 300);
    state.0 .0.refs_provider.lock().set_active(provider);
    // Title edges count only for the active provider's sidecars, so the
    // graph and the panels change the moment it does. Without this a view
    // drawn at launch, before the window had registered the provider, kept
    // showing no reference-list citations until something else changed.
    state.0 .0.notify_library_changed();
    Ok(())
}

/// Up to `limit` (at most 16) library documents the active provider has not
/// read: an opaque token and the document's stored text each, never a path.
#[tauri::command]
pub async fn library_refs_pending(
    state: tauri::State<'_, EngineState>,
    limit: u32,
) -> AppResult<Vec<crate::library::references::PendingRefsDto>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (entries, app_data_dir) = {
            let library = engine.0.library.lock();
            (library.all_entries(), library.app_data_dir())
        };
        let mut provider = engine.0.refs_provider.lock();
        let mut cache = engine.0.refs_cache.lock();
        Ok(provider.pending(
            &app_data_dir,
            &entries,
            &mut cache,
            limit.clamp(1, 16) as usize,
            std::time::Instant::now(),
        ))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// The provider's entries for a token: written to the document's refs
/// sidecar with the engine's compact text, year and identifiers. `None`
/// means the provider failed on it, and it is not offered again this
/// session. True when a sidecar was written.
#[tauri::command]
pub async fn library_refs_store(
    state: tauri::State<'_, EngineState>,
    token: String,
    entries: Option<Vec<crate::library::references::ProvidedRef>>,
) -> AppResult<bool> {
    let engine = state.0.clone();
    let worker = engine.clone();
    let stored: bool = tauri::async_runtime::spawn_blocking(move || -> AppResult<bool> {
        let engine = worker;
        let app_data_dir = engine.0.library.lock().app_data_dir();
        let mut provider = engine.0.refs_provider.lock();
        let mut cache = engine.0.refs_cache.lock();
        let outcome = provider.store(
            &app_data_dir,
            &token,
            |path| engine.0.library.lock().entry_for_path(path),
            entries.as_deref(),
            &mut cache,
        )?;
        Ok(matches!(
            outcome,
            crate::library::references::StoreOutcome::Stored(_)
        ))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))??;
    // A new reference list can add edges; tell the windows, outside every
    // lock. The provider reads one document at a time at idle, so this is
    // at most a refresh every half second or so while a backlog drains.
    if stored {
        engine.0.notify_library_changed();
    }
    Ok(stored)
}

/// Whether the open document looks like a paper. Asked by the reader before
/// it offers to file a PDF that is not in the library.
#[tauri::command]
pub async fn library_classify_document(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
) -> AppResult<PapernessDto> {
    state
        .0
        .call_async(Priority::HoverPreview, doc_id, move |respond| {
            Work::ClassifyDocument {
                doc: doc_id,
                respond,
            }
        })
        .await
}

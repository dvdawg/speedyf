//! Engine facade: a single dedicated worker thread owns every PDFium call
//! (PDFium is not thread-safe). Callers talk to it through a priority queue
//! of `Work` items carrying generation stamps; results come back via boxed
//! responder callbacks (the pdfr:// protocol hands its responder straight
//! in, so no extra waiting threads exist anywhere).
//!
//! The message-based boundary is deliberate: a future crash-isolated helper
//! process pool can replace the thread without changing any caller.

pub mod citation;
pub mod figures;
pub mod formal;
pub mod links;
pub mod pdfium_init;
pub mod preview;
pub mod queue;
pub mod render;
pub mod save;
pub mod tagged;
pub mod text;
pub mod types;
pub mod worker;

use crate::cache::ByteLru;
use crate::errors::{AppError, AppResult};
use crate::library::{CitationResolver, LibraryState, LocalLibraryResolver};
use crate::search::gramindex::GramIndex;
use crate::search::{query_pages, SearchStore, TextLayoutCache};
use parking_lot::Mutex;
use queue::{GenerationMap, JobMeta, PrioQueue};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use types::*;

pub type RespondBytes = Box<dyn FnOnce(AppResult<Arc<Vec<u8>>>) + Send>;
pub type Respond<T> = Box<dyn FnOnce(AppResult<T>) + Send>;

pub enum Work {
    Open {
        path: String,
        password: Option<String>,
        respond: Respond<DocMetaDto>,
    },
    Close {
        doc: DocId,
    },
    /// Marks which document is foreground for citation-hover bookkeeping,
    /// decoupled from Open/Close so opening a background tab never tears
    /// down another tab's hover session.
    SetActiveDocument {
        doc: Option<DocId>,
    },
    Sizes {
        doc: DocId,
        from: u32,
        count: u32,
        respond: Respond<PageSizesDto>,
    },
    Render {
        key: RenderKey,
        respond: RespondBytes,
    },
    TextLayout {
        doc: DocId,
        src: u32,
        respond: Respond<PageTextDto>,
    },
    PageLinks {
        doc: DocId,
        src: u32,
        respond: Respond<PageLinksDto>,
    },
    PreviewRect {
        doc: DocId,
        src: u32,
        x: Option<f32>,
        y: Option<f32>,
        respond: Respond<PreviewSpecDto>,
    },
    ResolveCitation {
        id: CitationIdDto,
        main_epoch: u64,
        respond: Respond<Option<ResolvedCitationDto>>,
    },
    /// Register a document as wanting text indexing. The worker runs exactly
    /// one indexing chain at a time (foreground document first), so opening
    /// many tabs at once cannot stampede the engine thread.
    StartIndexing {
        doc: DocId,
    },
    /// Extract the next unindexed page, then requeue itself (yields between
    /// pages so visible work always wins). `chain` identifies the scheduling
    /// run that issued it; orphaned chains are dropped on arrival.
    IndexNext {
        doc: DocId,
        chain: u64,
    },
    LibraryScanNext,
    /// Extract one library document's full text to its sidecar. Runs at Idle,
    /// one document per job, so a background library index can never outrank
    /// a page the reader is looking at.
    LibraryTextNext,
    MatchRects {
        doc: DocId,
        src: u32,
        start: u32,
        len: u32,
        respond: Respond<Vec<[f32; 4]>>,
    },
    /// Build a print-ready PDF at a throwaway path. Same materialization as
    /// Save, but the destination is never the document's own file.
    BuildPrintPdf {
        doc: DocId,
        plan: EditPlan,
        dest: PathBuf,
        respond: Respond<()>,
    },
    Save {
        doc: DocId,
        plan: EditPlan,
        dest: PathBuf,
        respond: Respond<SaveResultDto>,
    },
    FormFields {
        doc: DocId,
        respond: Respond<Vec<FormFieldDto>>,
    },
    Outline {
        doc: DocId,
        respond: Respond<Vec<OutlineNodeDto>>,
    },
    FormalEnvs {
        doc: DocId,
        respond: Respond<Vec<FormalEntryDto>>,
    },
    Figures {
        doc: DocId,
        respond: Respond<Vec<FigureDto>>,
    },
    ImageSize {
        path: String,
        respond: Respond<[u32; 2]>,
    },
}

#[derive(Default)]
pub struct Metrics {
    pub rendered: AtomicU64,
    pub skipped_stale: AtomicU64,
    pub pages_indexed: AtomicU64,
}

pub type ProgressCallback = Box<dyn Fn(DocId, SearchStatusDto) + Send + Sync>;

pub struct CacheBudgets {
    pub pages: u64,
    pub thumbs: u64,
    pub previews: u64,
    pub search: u64,
    pub text_layouts: u64,
    pub links: u64,
}

impl CacheBudgets {
    pub fn normal() -> Self {
        CacheBudgets {
            pages: 256 * 1024 * 1024,
            thumbs: 32 * 1024 * 1024,
            previews: 24 * 1024 * 1024,
            search: 64 * 1024 * 1024,
            text_layouts: 32 * 1024 * 1024,
            links: 8 * 1024 * 1024,
        }
    }
    pub fn low_memory() -> Self {
        CacheBudgets {
            pages: 96 * 1024 * 1024,
            thumbs: 16 * 1024 * 1024,
            previews: 8 * 1024 * 1024,
            search: 32 * 1024 * 1024,
            text_layouts: 16 * 1024 * 1024,
            links: 4 * 1024 * 1024,
        }
    }
}

pub struct EngineShared {
    pub queue: PrioQueue<Work>,
    pub gens: GenerationMap,
    /// Per-document render-cancel stamps. Bumped when queued render work is
    /// known to be pointless (the page it was for scrolled away); unlike
    /// `gens` this never invalidates a URL or a cache entry.
    pub cancels: GenerationMap,
    pub page_cache: Mutex<ByteLru<RenderKey>>,
    pub thumb_cache: Mutex<ByteLru<RenderKey>>,
    pub preview_cache: Mutex<ByteLru<RenderKey>>,
    pub search: Mutex<SearchStore>,
    pub text_layouts: Mutex<TextLayoutCache>,
    pub link_cache: Mutex<links::LinkCache>,
    pub library: Arc<Mutex<LibraryState>>,
    pub citation_resolver: Mutex<Option<Box<dyn CitationResolver>>>,
    pub metrics: Metrics,
    /// Changes only when the user-facing main document changes. Citation
    /// resolves capture it so queued stale hovers cannot open library PDFs.
    pub main_epoch: AtomicU64,
    pub progress: Mutex<Option<ProgressCallback>>,
    pub last_metrics: Mutex<EngineMetricsDto>,
    /// The inverted gram index over the library, once it has been built for
    /// the current set of documents. Absent means "search by scanning" — a
    /// slower answer, never a wrong one.
    pub gram_index: Mutex<Option<Arc<GramIndex>>>,
    /// Whether a rebuild is already under way, so a burst of queries against a
    /// stale library starts one build rather than one each.
    pub gram_building: AtomicBool,
}

impl EngineShared {
    /// A render job is worthless if either its URL generation has been
    /// superseded or its work has been explicitly cancelled since submission.
    pub fn is_render_stale(&self, meta: &JobMeta) -> bool {
        self.gens.is_stale(meta) || self.cancels.is_stale_at(meta.doc, meta.epoch)
    }
}

#[derive(Clone)]
pub struct EngineHandle(pub Arc<EngineShared>);

impl EngineHandle {
    /// Spawn the engine thread. `hints` are extra directories to search for
    /// the PDFium dynamic library (e.g. the Tauri resource dir).
    pub fn start(hints: Vec<PathBuf>, low_memory: bool, app_data_dir: Option<PathBuf>) -> Self {
        let budgets = if low_memory {
            CacheBudgets::low_memory()
        } else {
            CacheBudgets::normal()
        };
        let app_data_dir = app_data_dir.unwrap_or_else(|| {
            std::env::temp_dir().join(format!("speedyf-library-{}", std::process::id()))
        });
        let library = Arc::new(Mutex::new(LibraryState::load(app_data_dir)));
        let resolver: Option<Box<dyn CitationResolver>> = (!library.lock().roots().is_empty())
            .then(|| Box::new(LocalLibraryResolver::new(Arc::clone(&library))) as Box<_>);
        let shared = Arc::new(EngineShared {
            queue: PrioQueue::new(),
            gens: GenerationMap::default(),
            cancels: GenerationMap::default(),
            page_cache: Mutex::new(ByteLru::new(budgets.pages)),
            thumb_cache: Mutex::new(ByteLru::new(budgets.thumbs)),
            preview_cache: Mutex::new(ByteLru::new(budgets.previews)),
            search: Mutex::new(SearchStore::new(budgets.search)),
            text_layouts: Mutex::new(TextLayoutCache::new(budgets.text_layouts)),
            link_cache: Mutex::new(links::LinkCache::new(budgets.links)),
            library,
            citation_resolver: Mutex::new(resolver),
            metrics: Metrics::default(),
            main_epoch: AtomicU64::new(0),
            progress: Mutex::new(None),
            last_metrics: Mutex::new(EngineMetricsDto::default()),
            gram_index: Mutex::new(None),
            gram_building: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("speedyf-engine".into())
            .spawn(move || worker::run(worker_shared, hints))
            .expect("failed to spawn engine thread");
        let handle = EngineHandle(shared);
        if handle.0.library.lock().has_scan_work() {
            handle.submit(Priority::Idle, 0, Work::LibraryScanNext);
        }
        // Text extraction resumes from wherever it stopped: the sidecars from
        // previous runs are still on disk, so this only picks up what is new.
        if handle.0.library.lock().has_text_work() {
            handle.submit(Priority::Idle, 0, Work::LibraryTextNext);
        }
        handle
    }

    pub fn submit(&self, prio: Priority, doc: DocId, work: Work) {
        let meta = JobMeta {
            doc,
            gen: self.0.gens.current(doc),
            epoch: self.0.cancels.current(doc),
        };
        self.0.queue.push(prio, meta, work);
    }

    /// Submit pinned to a caller-known generation (used by the protocol so a
    /// URL minted for gen N can never be satisfied by gen N+1 state).
    pub fn submit_at_gen(&self, prio: Priority, doc: DocId, gen: u64, work: Work) {
        let epoch = self.0.cancels.current(doc);
        self.0.queue.push(prio, JobMeta { doc, gen, epoch }, work);
    }

    /// Invalidate every outstanding URL for `doc` and flag its cached rasters
    /// as first-choice eviction victims. Reserved for changes that make the
    /// cached pixels themselves wrong — it forces the webview to re-fetch and
    /// re-decode every mounted image, so it is far from free. To merely
    /// abandon queued work, use `cancel_renders`.
    pub fn bump_generation(&self, doc: DocId) -> u64 {
        let g = self.0.gens.bump(doc);
        // Rasters at the superseded scale are now eviction candidates before
        // anything else. Only the page cache: thumbnails and hover previews
        // are rendered at their own fixed scales, so a viewer zoom never makes
        // them wrong, and sweeping them here just evicted work the sidebar was
        // about to ask for again.
        self.0.page_cache.lock().mark_stale(|k| k.doc == doc);
        g
    }

    /// Abandon queued and in-flight render work for `doc`. Cached rasters and
    /// already-minted URLs stay valid, so this costs nothing to recover from:
    /// anything still on screen is served straight from cache.
    pub fn cancel_renders(&self, doc: DocId) {
        self.0.cancels.bump(doc);
    }

    pub fn current_generation(&self, doc: DocId) -> u64 {
        self.0.gens.current(doc)
    }

    pub fn main_epoch(&self) -> u64 {
        self.0.main_epoch.load(Ordering::Acquire)
    }

    pub fn cached_render(&self, key: &RenderKey) -> Option<Arc<Vec<u8>>> {
        let cache = match key.kind {
            RenderKind::Thumb => &self.0.thumb_cache,
            RenderKind::Preview => &self.0.preview_cache,
            _ => &self.0.page_cache,
        };
        cache.lock().get(key)
    }

    pub fn set_progress_callback(&self, cb: ProgressCallback) {
        *self.0.progress.lock() = Some(cb);
    }

    pub fn set_low_memory(&self, low: bool) {
        let budgets = if low {
            CacheBudgets::low_memory()
        } else {
            CacheBudgets::normal()
        };
        self.0.page_cache.lock().set_budget(budgets.pages);
        self.0.thumb_cache.lock().set_budget(budgets.thumbs);
        self.0.preview_cache.lock().set_budget(budgets.previews);
        self.0.search.lock().set_budget(budgets.search);
        self.0.text_layouts.lock().set_budget(budgets.text_layouts);
        self.0.link_cache.lock().set_budget(budgets.links);
    }

    pub fn metrics_snapshot(&self) -> EngineMetricsDto {
        let mut snapshot = self.0.last_metrics.lock().clone();
        snapshot.rendered = self.0.metrics.rendered.load(Ordering::Relaxed);
        snapshot.skipped_stale = self.0.metrics.skipped_stale.load(Ordering::Relaxed);
        snapshot.pages_indexed = self.0.metrics.pages_indexed.load(Ordering::Relaxed);
        snapshot.queue_depth = self.0.queue.len() as u64;

        // Diagnostics must never make an interactive render/search wait. Keep
        // the last complete cache sample if any cache is currently busy.
        if let (
            Some(page_cache),
            Some(thumb_cache),
            Some(preview_cache),
            Some(search),
            Some(text_layouts),
        ) = (
            self.0.page_cache.try_lock(),
            self.0.thumb_cache.try_lock(),
            self.0.preview_cache.try_lock(),
            self.0.search.try_lock(),
            self.0.text_layouts.try_lock(),
        ) {
            snapshot.cache_hits = page_cache.hits + thumb_cache.hits + preview_cache.hits;
            snapshot.cache_lookups =
                page_cache.lookups + thumb_cache.lookups + preview_cache.lookups;
            snapshot.page_cache_bytes = page_cache.used();
            snapshot.page_cache_budget = page_cache.budget();
            snapshot.thumb_cache_bytes = thumb_cache.used();
            snapshot.thumb_cache_budget = thumb_cache.budget();
            snapshot.preview_cache_bytes = preview_cache.used();
            snapshot.preview_cache_budget = preview_cache.budget();
            snapshot.text_bytes = search.used() + text_layouts.used();
            snapshot.text_budget = search.budget() + text_layouts.budget();
            *self.0.last_metrics.lock() = snapshot.clone();
        }
        snapshot
    }

    /// Add a folder to the library, or remove one.
    pub fn change_library_root(&self, path: String, add: bool) -> AppResult<()> {
        let path = PathBuf::from(path);
        let should_scan = {
            let mut library = self.0.library.lock();
            if add {
                library.add_root(path)?
            } else {
                library.remove_root(&path)?
            }
        };
        let enabled = !self.0.library.lock().roots().is_empty();
        *self.0.citation_resolver.lock() = enabled.then(|| {
            Box::new(LocalLibraryResolver::new(Arc::clone(&self.0.library)))
                as Box<dyn CitationResolver>
        });
        if should_scan {
            self.submit(Priority::Idle, 0, Work::LibraryScanNext);
        }
        self.submit(Priority::Idle, 0, Work::LibraryTextNext);
        Ok(())
    }

    /// Search every indexed document in the library.
    ///
    /// Off the engine thread entirely — it reads sidecars, never PDFium — so a
    /// query cannot stall rendering however large the library is.
    pub fn library_search(
        &self,
        query: &str,
        case_sensitive: bool,
        per_document_limit: usize,
        document_limit: usize,
    ) -> LibrarySearchDto {
        let (app_data_dir, documents) = {
            let library = self.0.library.lock();
            (
                library.app_data_dir(),
                library
                    .all_entries()
                    .into_iter()
                    .filter(|entry| entry.text_indexed)
                    .map(|entry| crate::search::librarytext::LibraryDocument {
                        path: entry.path,
                        title: entry.title,
                        mtime_ms: entry.mtime_ms,
                        size: entry.size,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let index = self.gram_index_for(&app_data_dir, &documents);
        let (documents, truncated) = crate::search::librarytext::search_library(
            &app_data_dir,
            &documents,
            index.as_deref(),
            query,
            case_sensitive,
            per_document_limit,
            document_limit,
        );
        LibrarySearchDto {
            documents,
            truncated,
        }
    }

    /// The gram index for this exact set of documents, if one is ready.
    ///
    /// A stale or missing index is rebuilt in the background rather than
    /// waited for: the query in hand is answered by scanning — the same answer,
    /// just slower — and the one after it gets the index. Building blocks on
    /// reading the library's whole folded text, which is not something a
    /// keystroke should wait on.
    fn gram_index_for(
        &self,
        app_data_dir: &std::path::Path,
        documents: &[crate::search::librarytext::LibraryDocument],
    ) -> Option<Arc<GramIndex>> {
        use crate::search::librarytext;

        let signature = librarytext::library_signature(documents);
        if let Some(index) = self.0.gram_index.lock().as_ref() {
            if index.signature() == signature {
                return Some(Arc::clone(index));
            }
        }
        // Not in memory, but a previous run may have left one on disk that
        // still describes this library.
        if let Some(index) = librarytext::load_gram_index(app_data_dir, signature) {
            let index = Arc::new(index);
            *self.0.gram_index.lock() = Some(Arc::clone(&index));
            return Some(index);
        }

        // Genuinely stale. Not worth building yet if the background text pass
        // is still running: every document it finishes changes the document
        // set, so an index built now would be stale before it was written.
        let status = self.0.library.lock().status();
        if status.scanning || status.text_indexed < status.text_total {
            return None;
        }

        // Start one rebuild, not one per query in a burst.
        if self
            .0
            .gram_building
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let shared = Arc::clone(&self.0);
            let app_data_dir = app_data_dir.to_path_buf();
            let fingerprints: Vec<_> = documents
                .iter()
                .map(|document| librarytext::LibraryDocument {
                    path: document.path.clone(),
                    title: None,
                    mtime_ms: document.mtime_ms,
                    size: document.size,
                })
                .collect();
            std::thread::spawn(move || {
                let index = librarytext::build_gram_index(&app_data_dir, &fingerprints);
                librarytext::save_gram_index(&app_data_dir, &index);
                *shared.gram_index.lock() = Some(Arc::new(index));
                shared.gram_building.store(false, Ordering::Release);
            });
        }
        None
    }

    pub fn library_status(&self) -> LibraryStatusDto {
        self.0.library.lock().status()
    }

    /// Query the pages currently present in the incremental search index.
    /// This is intentionally non-blocking: callers decide how aggressively
    /// to schedule extraction and can surface partial results immediately.
    pub fn search_indexed(
        &self,
        doc: DocId,
        query: &str,
        case_sensitive: bool,
        per_page_limit: usize,
        global_limit: usize,
    ) -> SearchQueryDto {
        // Clone cheap Arc entries, release the store mutex, then scan. This
        // prevents a broad query from blocking the background indexer.
        let pages = self.0.search.lock().snapshot(doc);
        query_pages(&pages, query, case_sensitive, per_page_limit, global_limit)
    }

    /// Submit work and await its callback without occupying the IPC caller.
    ///
    /// Tauri's synchronous command wrapper runs inline on the native IPC
    /// callback. Blocking that callback on PDFium can deadlock WebKit's main
    /// event loop when several page resources arrive together. A bounded
    /// async channel keeps the single-owner engine design while yielding the
    /// caller until the worker invokes the response closure.
    pub async fn call_async<T: Send + 'static>(
        &self,
        prio: Priority,
        doc: DocId,
        make: impl FnOnce(Respond<T>) -> Work + Send + 'static,
    ) -> AppResult<T> {
        let (tx, mut rx) = tauri::async_runtime::channel::<AppResult<T>>(1);
        let work = make(Box::new(move |result| {
            // There is exactly one response and one channel slot, so this
            // never waits on the PDFium owner thread. Failure only means the
            // invoking webview has already abandoned the request.
            let _ = tx.try_send(result);
        }));
        self.submit(prio, doc, work);
        rx.recv().await.ok_or(AppError::EngineUnavailable)?
    }

    /// Blocking adapter for the standalone benchmark runner only. UI command
    /// handlers must use `call_async()` so the native event loop never waits
    /// for the engine thread.
    pub fn call_blocking<T: Send + 'static>(
        &self,
        prio: Priority,
        doc: DocId,
        make: impl FnOnce(Respond<T>) -> Work,
    ) -> AppResult<T> {
        let (tx, rx) = crossbeam_channel::bounded::<AppResult<T>>(1);
        let work = make(Box::new(move |r| {
            let _ = tx.send(r);
        }));
        self.submit(prio, doc, work);
        rx.recv().map_err(|_| AppError::EngineUnavailable)?
    }
}

//! Engine facade: a single dedicated worker thread owns every PDFium call
//! (PDFium is not thread-safe). Callers talk to it through a priority queue
//! of `Work` items carrying generation stamps; results come back via boxed
//! responder callbacks (the pdfr:// protocol hands its responder straight
//! in, so no extra waiting threads exist anywhere).
//!
//! The message-based boundary is deliberate: a future crash-isolated helper
//! process pool can replace the thread without changing any caller.

pub mod citation;
pub mod links;
pub mod pdfium_init;
pub mod preview;
pub mod queue;
pub mod render;
pub mod save;
pub mod text;
pub mod types;
pub mod worker;

use crate::cache::ByteLru;
use crate::errors::{AppError, AppResult};
use crate::library::{CitationResolver, LibraryState, LocalLibraryResolver};
use crate::search::{query_pages, SearchStore, TextLayoutCache};
use parking_lot::Mutex;
use queue::{GenerationMap, JobMeta, PrioQueue};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Extract the next unindexed page, then requeue itself (yields between
    /// pages so visible work always wins).
    IndexNext {
        doc: DocId,
    },
    LibraryScanNext,
    MatchRects {
        doc: DocId,
        src: u32,
        start: u32,
        len: u32,
        respond: Respond<Vec<[f32; 4]>>,
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
        let resolver: Option<Box<dyn CitationResolver>> = library
            .lock()
            .root()
            .is_some()
            .then(|| Box::new(LocalLibraryResolver::new(Arc::clone(&library))) as Box<_>);
        let shared = Arc::new(EngineShared {
            queue: PrioQueue::new(),
            gens: GenerationMap::default(),
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
        handle
    }

    pub fn submit(&self, prio: Priority, doc: DocId, work: Work) {
        let meta = JobMeta {
            doc,
            gen: self.0.gens.current(doc),
        };
        self.0.queue.push(prio, meta, work);
    }

    /// Submit pinned to a caller-known generation (used by the protocol so a
    /// URL minted for gen N can never be satisfied by gen N+1 state).
    pub fn submit_at_gen(&self, prio: Priority, doc: DocId, gen: u64, work: Work) {
        self.0.queue.push(prio, JobMeta { doc, gen }, work);
    }

    pub fn bump_generation(&self, doc: DocId) -> u64 {
        let g = self.0.gens.bump(doc);
        // wrong-generation renders are now eviction candidates before anything else
        self.0.page_cache.lock().mark_stale(|k| k.doc == doc);
        self.0.thumb_cache.lock().mark_stale(|k| k.doc == doc);
        self.0.preview_cache.lock().mark_stale(|k| k.doc == doc);
        g
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

    pub fn set_library_root(&self, path: Option<String>) -> AppResult<()> {
        let should_scan = self.0.library.lock().set_root(path.map(PathBuf::from))?;
        let enabled = self.0.library.lock().root().is_some();
        *self.0.citation_resolver.lock() = enabled.then(|| {
            Box::new(LocalLibraryResolver::new(Arc::clone(&self.0.library)))
                as Box<dyn CitationResolver>
        });
        if should_scan {
            self.submit(Priority::Idle, 0, Work::LibraryScanNext);
        }
        Ok(())
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

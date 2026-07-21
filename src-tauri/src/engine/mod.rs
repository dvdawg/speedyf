//! Engine facade: a single dedicated worker thread owns every PDFium call
//! (PDFium is not thread-safe). Callers talk to it through a priority queue
//! of `Work` items carrying generation stamps; results come back via boxed
//! responder callbacks (the pdfr:// protocol hands its responder straight
//! in, so no extra waiting threads exist anywhere).
//!
//! The message-based boundary is deliberate: a future crash-isolated helper
//! process pool can replace the thread without changing any caller.

pub mod pdfium_init;
pub mod queue;
pub mod render;
pub mod save;
pub mod text;
pub mod types;
pub mod worker;

use crate::cache::ByteLru;
use crate::errors::{AppError, AppResult};
use crate::search::SearchStore;
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
        src: u16,
        respond: Respond<PageTextDto>,
    },
    /// Extract the next unindexed page, then requeue itself (yields between
    /// pages so visible work always wins).
    IndexNext {
        doc: DocId,
    },
    MatchRects {
        doc: DocId,
        src: u16,
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
    pub text: u64,
}

impl CacheBudgets {
    pub fn normal() -> Self {
        CacheBudgets {
            pages: 256 * 1024 * 1024,
            thumbs: 32 * 1024 * 1024,
            text: 24 * 1024 * 1024,
        }
    }
    pub fn low_memory() -> Self {
        CacheBudgets {
            pages: 96 * 1024 * 1024,
            thumbs: 16 * 1024 * 1024,
            text: 12 * 1024 * 1024,
        }
    }
}

pub struct EngineShared {
    pub queue: PrioQueue<Work>,
    pub gens: GenerationMap,
    pub page_cache: Mutex<ByteLru<RenderKey>>,
    pub thumb_cache: Mutex<ByteLru<RenderKey>>,
    pub search: Mutex<SearchStore>,
    pub metrics: Metrics,
    pub progress: Mutex<Option<ProgressCallback>>,
}

#[derive(Clone)]
pub struct EngineHandle(pub Arc<EngineShared>);

impl EngineHandle {
    /// Spawn the engine thread. `hints` are extra directories to search for
    /// the PDFium dynamic library (e.g. the Tauri resource dir).
    pub fn start(hints: Vec<PathBuf>, low_memory: bool) -> Self {
        let budgets = if low_memory {
            CacheBudgets::low_memory()
        } else {
            CacheBudgets::normal()
        };
        let shared = Arc::new(EngineShared {
            queue: PrioQueue::new(),
            gens: GenerationMap::default(),
            page_cache: Mutex::new(ByteLru::new(budgets.pages)),
            thumb_cache: Mutex::new(ByteLru::new(budgets.thumbs)),
            search: Mutex::new(SearchStore::new(budgets.text)),
            metrics: Metrics::default(),
            progress: Mutex::new(None),
        });
        let worker_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("speedyf-engine".into())
            .spawn(move || worker::run(worker_shared, hints))
            .expect("failed to spawn engine thread");
        EngineHandle(shared)
    }

    pub fn submit(&self, prio: Priority, doc: DocId, work: Work) {
        let meta = JobMeta {
            doc,
            gen: self.0.gens.current(doc),
            prio,
        };
        self.0.queue.push(prio, meta, work);
    }

    /// Submit pinned to a caller-known generation (used by the protocol so a
    /// URL minted for gen N can never be satisfied by gen N+1 state).
    pub fn submit_at_gen(&self, prio: Priority, doc: DocId, gen: u64, work: Work) {
        self.0.queue.push(prio, JobMeta { doc, gen, prio }, work);
    }

    pub fn bump_generation(&self, doc: DocId) -> u64 {
        let g = self.0.gens.bump(doc);
        // wrong-generation renders are now eviction candidates before anything else
        self.0.page_cache.lock().mark_stale(|k| k.doc == doc);
        self.0.thumb_cache.lock().mark_stale(|k| k.doc == doc);
        g
    }

    pub fn current_generation(&self, doc: DocId) -> u64 {
        self.0.gens.current(doc)
    }

    pub fn cached_render(&self, key: &RenderKey) -> Option<Arc<Vec<u8>>> {
        let cache = match key.kind {
            RenderKind::Thumb => &self.0.thumb_cache,
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
        self.0.search.lock().set_budget(budgets.text);
    }

    pub fn metrics_snapshot(&self) -> EngineMetricsDto {
        let page_cache = self.0.page_cache.lock();
        let thumb_cache = self.0.thumb_cache.lock();
        let search = self.0.search.lock();
        EngineMetricsDto {
            rendered: self.0.metrics.rendered.load(Ordering::Relaxed),
            skipped_stale: self.0.metrics.skipped_stale.load(Ordering::Relaxed),
            cache_hits: page_cache.hits + thumb_cache.hits,
            cache_lookups: page_cache.lookups + thumb_cache.lookups,
            page_cache_bytes: page_cache.used(),
            page_cache_budget: page_cache.budget(),
            thumb_cache_bytes: thumb_cache.used(),
            thumb_cache_budget: thumb_cache.budget(),
            text_bytes: search.used(),
            text_budget: search.budget(),
            pages_indexed: self.0.metrics.pages_indexed.load(Ordering::Relaxed),
            queue_depth: self.0.queue.len() as u64,
        }
    }

    /// Synchronous helper for commands: submit and block on the reply.
    pub fn call<T: Send + 'static>(
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

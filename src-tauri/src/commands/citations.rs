//! The citation pass (`library::consensus`): metadata from citing papers
//! and each item's year floor, brought up to date after reference lists
//! change. Offline; it reads the refs sidecars and writes the store.

use crate::engine::EngineHandle;
use crate::library::consensus::current_year;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A pass is waiting to run; requests meanwhile ride along with it.
static PENDING: AtomicBool = AtomicBool::new(false);
/// How long requests are gathered before the pass runs. While a backlog
/// drains the provider stores a list every half second or so; one pass
/// per few seconds keeps up without redoing the work for each.
const COALESCE: Duration = Duration::from_secs(3);

/// Run the pass soon, once for every request made before it starts.
pub fn schedule(engine: &EngineHandle) {
    if PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let engine = engine.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(COALESCE).await;
        PENDING.store(false, Ordering::Release);
        let _ = tauri::async_runtime::spawn_blocking(move || run(&engine)).await;
    });
}

/// The pass itself, against the active provider's reference lists: saved
/// and announced only when something changed. The locks in turn, never
/// nested, as the citation commands take them.
pub fn run(engine: &EngineHandle) {
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
    let saved = {
        let mut store = engine.0.papers.lock();
        if !store.apply_citation_facts(&entries, &refs, current_year()) {
            return;
        }
        match store.flush(true) {
            Ok(saved) => saved,
            Err(error) => {
                log::warn!("cannot save the citation pass: {error}");
                false
            }
        }
    };
    if saved {
        engine.0.notify_library_changed();
    }
}

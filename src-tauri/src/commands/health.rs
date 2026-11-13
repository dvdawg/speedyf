//! Library health, new arrivals and the "Cited here" counts (projects.md
//! sec. 5). Reads go through the same snapshots as the citation graph; the
//! two writes record what the user decided in the store's `review` section.

use super::library::{citation_inputs, duplicate_groups, read_store, write_store};
use super::EngineState;
use crate::errors::{AppError, AppResult};
use crate::library::health::HealthGroupDto;
use crate::library::types::{PaperFilterDto, PaperRowDto};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What SpeedyF suspects, grouped; empty groups left out.
#[tauri::command]
pub async fn library_health(
    state: tauri::State<'_, EngineState>,
) -> AppResult<Vec<HealthGroupDto>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let duplicates = duplicate_groups(&engine);
        let entries = engine.0.library.lock().all_entries();
        let store = engine.0.papers.lock();
        Ok(store.health(&duplicates, &entries, now_secs()))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

/// Never show this finding about this item again.
#[tauri::command]
pub async fn library_health_ignore(
    state: tauri::State<'_, EngineState>,
    paper_id: String,
    kind: String,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| {
        store.ignore_finding(&paper_id, &kind)
    })
    .await
}

/// Items that arrived in the last week and are not triaged, newest first.
#[tauri::command]
pub async fn library_new_items(
    state: tauri::State<'_, EngineState>,
    query: Option<String>,
) -> AppResult<Vec<PaperRowDto>> {
    read_store(state.0.clone(), move |store| {
        Ok(store.new_items(query.as_deref().unwrap_or(""), now_secs()))
    })
    .await
}

/// The user triaged these new items.
#[tauri::command]
pub async fn library_mark_reviewed(
    state: tauri::State<'_, EngineState>,
    paper_ids: Vec<String>,
) -> AppResult<()> {
    write_store(state.0.clone(), move |store| {
        store.mark_reviewed(&paper_ids, now_secs())
    })
    .await
}

/// Item id -> how many items of the scope cite it.
#[tauri::command]
pub async fn library_cited_here(
    state: tauri::State<'_, EngineState>,
    filter: PaperFilterDto,
) -> AppResult<HashMap<String, u32>> {
    let engine = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (entries, refs, network) = citation_inputs(&engine);
        let store = engine.0.papers.lock();
        Ok(store.cited_here(&filter, &entries, &refs, &network))
    })
    .await
    .map_err(|error| AppError::Internal(format!("library task failed: {error}")))?
}

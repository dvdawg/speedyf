//! Hover previews from page text: cross-references, footnotes, and terms an
//! extension contributes (`engine::hovers`).

use super::EngineState;
use crate::engine::hovers::{terms::TermInput, PageHoversDto};
use crate::engine::types::{DocId, PreviewSpecDto, Priority};
use crate::engine::Work;
use crate::errors::AppResult;

/// A page's hover spans. Below visible rendering: the first answer for a
/// document waits for its index, which is read a few pages at a time.
#[tauri::command]
pub async fn get_page_hovers(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
) -> AppResult<PageHoversDto> {
    state
        .0
        .call_async(Priority::TextExtract, doc_id, move |respond| {
            Work::PageHovers {
                doc: doc_id,
                src,
                respond,
            }
        })
        .await
}

/// Replace the terms contributed for a document; an empty list clears them.
/// Answers how many were kept.
#[tauri::command]
pub async fn set_hover_terms(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    terms: Vec<TermInput>,
) -> AppResult<u32> {
    state
        .0
        .call_async(Priority::HoverPreview, doc_id, move |respond| {
            Work::SetHoverTerms {
                doc: doc_id,
                terms,
                respond,
            }
        })
        .await
}

/// The raster crop a hover span previews.
#[tauri::command]
pub async fn get_hover_preview(
    state: tauri::State<'_, EngineState>,
    doc_id: DocId,
    src: u32,
    kind: String,
    region: [f32; 4],
) -> AppResult<PreviewSpecDto> {
    state
        .0
        .call_async(Priority::HoverPreview, doc_id, move |respond| {
            Work::HoverPreview {
                doc: doc_id,
                src,
                kind,
                region,
                respond,
            }
        })
        .await
}

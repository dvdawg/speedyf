//! The engine thread. Sole owner of every PDFium handle in the process.
//! Raw FPDF_* handles never leave this thread; results cross back as owned
//! bytes/DTOs through responder callbacks.

use super::links;
use super::pdfium_init;
use super::preview::{self, CropInput, PREVIEW_SCALE_MILLI};
use super::queue::JobMeta;
use super::render;
use super::save;
use super::text;
use super::types::*;
use super::{EngineShared, Work};
use crate::errors::{AppError, AppResult};
use crate::library::{self, LibraryEntry};
use crate::search::TextLayoutData;
use pdfium_render::prelude::*;
use pdfium_render::prelude::{FPDF_DOCUMENT, FPDF_PAGE};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Open page handles kept per document to avoid re-parsing page objects on
/// every render/text request for the same page.
const PAGE_HANDLE_CAP: usize = 8;
const LIBRARY_DOC_CAP: usize = 3;

struct DocState {
    raw: FPDF_DOCUMENT,
    path: String,
    password: Option<String>,
    page_count: u32,
    /// display sizes (pt), lazily hydrated
    sizes: Vec<Option<[f32; 2]>>,
    /// tiny MRU of open page handles: (src_index, handle)
    pages: Vec<(u32, FPDF_PAGE)>,
    index_cursor: u32,
    form_fields: Option<Vec<FormFieldDto>>,
    /// Computed lazily; invalidated (left None) whenever the page set changes,
    /// since every entry carries a page index.
    formal_envs: Option<Vec<FormalEntryDto>>,
}

struct WorkerState<'a> {
    bindings: &'a dyn PdfiumLibraryBindings,
    pdfium: &'a Pdfium,
    docs: HashMap<DocId, DocState>,
    next_id: DocId,
    active_main: Option<DocId>,
    /// Least-recently-used first; these documents exist only for hover cards.
    library_docs: Vec<DocId>,
    /// Includes LRU-evicted library handles whose encoded preview can still be
    /// served from the bounded preview cache until the main document changes.
    library_session_docs: Vec<DocId>,
    /// Documents with text-index work left to do, oldest request first.
    index_pending: Vec<DocId>,
    /// The single indexing chain currently cycling, as (document, chain id).
    /// Only one runs at a time: N open tabs each spinning their own IndexNext
    /// loop would interleave N page extractions onto this one thread for no
    /// gain, since only the foreground document's index is ever queried.
    index_active: Option<(DocId, u64)>,
    index_chain_seq: u64,
}

impl<'a> WorkerState<'a> {
    fn doc(&mut self, id: DocId) -> AppResult<&mut DocState> {
        self.docs.get_mut(&id).ok_or(AppError::DocumentNotFound)
    }

    fn page_handle(&mut self, id: DocId, src: u32) -> AppResult<FPDF_PAGE> {
        let b = self.bindings;
        let doc = self.doc(id)?;
        if let Some(pos) = doc.pages.iter().position(|(s, _)| *s == src) {
            let entry = doc.pages.remove(pos);
            doc.pages.push(entry);
            return Ok(doc.pages.last().unwrap().1);
        }
        let handle = b.FPDF_LoadPage(doc.raw, src as i32);
        if handle.is_null() {
            return Err(AppError::Malformed(format!("cannot load page {src}")));
        }
        if doc.pages.len() >= PAGE_HANDLE_CAP {
            let (_, old) = doc.pages.remove(0);
            b.FPDF_ClosePage(old);
        }
        doc.pages.push((src, handle));
        Ok(handle)
    }

    fn display_size(&mut self, id: DocId, src: u32) -> AppResult<[f32; 2]> {
        let b = self.bindings;
        let doc = self.doc(id)?;
        if let Some(Some(s)) = doc.sizes.get(src as usize) {
            return Ok(*s);
        }
        let s = render::page_display_size(b, doc.raw, src as i32)
            .ok_or_else(|| AppError::Malformed(format!("no size for page {src}")))?;
        if let Some(slot) = doc.sizes.get_mut(src as usize) {
            *slot = Some(s);
        }
        Ok(s)
    }

    fn close_one(&mut self, id: DocId, shared: &EngineShared, remove_preview: bool) {
        if let Some(doc) = self.docs.remove(&id) {
            for (_, p) in doc.pages {
                self.bindings.FPDF_ClosePage(p);
            }
            self.bindings.FPDF_CloseDocument(doc.raw);
        }
        // Keep the generation tombstone. Document ids are never reused, and
        // queued render URLs minted before close must remain stale even after
        // the PDFium handles and caches are gone.
        shared.page_cache.lock().remove_matching(|k| k.doc == id);
        shared.thumb_cache.lock().remove_matching(|k| k.doc == id);
        if remove_preview {
            shared.preview_cache.lock().remove_matching(|k| k.doc == id);
        }
        shared.search.lock().remove_doc(id);
        shared.text_layouts.lock().remove_doc(id);
        shared.link_cache.lock().remove_doc(id);
        self.library_docs.retain(|candidate| *candidate != id);
    }

    fn close_library_docs(&mut self, shared: &EngineShared) {
        let docs = std::mem::take(&mut self.library_docs);
        for id in docs {
            self.close_one(id, shared, false);
        }
        let session_docs = std::mem::take(&mut self.library_session_docs);
        if !session_docs.is_empty() {
            shared
                .preview_cache
                .lock()
                .remove_matching(|key| session_docs.contains(&key.doc));
        }
    }

    fn close_doc(&mut self, id: DocId, shared: &EngineShared) {
        self.close_one(id, shared, true);
        self.library_session_docs
            .retain(|candidate| *candidate != id);
        if self.active_main == Some(id) {
            self.active_main = None;
        }
        // Hand the indexer to the next document rather than stalling on a
        // chain whose document no longer exists.
        self.end_indexing(id, true, shared);
    }

    /// Marks `doc` as the foreground document for citation-hover bookkeeping.
    /// A no-op if it's already active, so redundant calls (e.g. re-activating
    /// the same tab) never spuriously invalidate in-flight citation resolves.
    fn set_active_document(&mut self, doc: Option<DocId>, shared: &EngineShared) {
        if doc == self.active_main {
            return;
        }
        shared.main_epoch.fetch_add(1, Ordering::AcqRel);
        self.close_library_docs(shared);
        self.active_main = doc;
        // The document you are looking at is the one whose search index you
        // can actually use, so hand it the indexer immediately.
        if let Some(doc) = doc {
            if self.index_pending.contains(&doc) && self.index_active.map(|(d, _)| d) != Some(doc) {
                self.index_active = None;
                self.pump_indexing(shared);
            }
        }
    }

    /// Register `doc` as wanting text indexing and start it if the indexer is
    /// idle. Repeat requests for an already-pending document are no-ops.
    fn request_indexing(&mut self, doc: DocId, shared: &EngineShared) {
        if !self.index_pending.contains(&doc) {
            self.index_pending.push(doc);
        }
        self.pump_indexing(shared);
    }

    /// Start the next indexing chain if none is running. Prefers the
    /// foreground document, then falls back to oldest-request-first.
    fn pump_indexing(&mut self, shared: &EngineShared) {
        if self.index_active.is_some() {
            return;
        }
        let pick = self
            .active_main
            .filter(|d| self.index_pending.contains(d))
            .or_else(|| self.index_pending.first().copied());
        let Some(doc) = pick else {
            return;
        };
        self.index_chain_seq += 1;
        let chain = self.index_chain_seq;
        self.index_active = Some((doc, chain));
        shared.queue.push(
            Priority::TextExtract,
            JobMeta {
                doc,
                gen: shared.gens.current(doc),
                epoch: shared.cancels.current(doc),
            },
            Work::IndexNext { doc, chain },
        );
    }

    /// Retire the running chain. `done` means the document needs no further
    /// indexing (finished, truncated, or gone) and should leave the queue.
    fn end_indexing(&mut self, doc: DocId, done: bool, shared: &EngineShared) {
        if self.index_active.map(|(d, _)| d) == Some(doc) {
            self.index_active = None;
        }
        if done {
            self.index_pending.retain(|candidate| *candidate != doc);
        }
        self.pump_indexing(shared);
    }
}

const FIRST_SIZES: u32 = 64;

pub fn run(shared: Arc<EngineShared>, hints: Vec<PathBuf>) {
    let bindings = match pdfium_init::init_bindings(&hints) {
        Ok(b) => b,
        Err(e) => {
            log::error!("engine unavailable: {e}");
            // Drain jobs with errors so callers fail fast instead of hanging.
            while let Some((_, work)) = shared.queue.pop_blocking() {
                fail_work(work, || AppError::EngineUnavailable);
            }
            return;
        }
    };
    // The engine lives for the process lifetime; leaking gives us 'static
    // lifetimes for both the raw bindings and the high-level wrapper.
    let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(bindings)));
    let mut state = WorkerState {
        bindings: pdfium.bindings(),
        pdfium,
        docs: HashMap::new(),
        next_id: 1,
        active_main: None,
        library_docs: Vec::new(),
        library_session_docs: Vec::new(),
        index_pending: Vec::new(),
        index_active: None,
        index_chain_seq: 0,
    };

    while let Some((meta, work)) = shared.queue.pop_blocking() {
        // Generation/cancel stamps represent render state only. Never let a
        // zoom gesture or a page turn cancel indexing, text, save, or form work.
        let stale = shared.is_render_stale(&meta) && is_generation_cancellable(&work);
        if stale {
            shared.metrics.skipped_stale.fetch_add(1, Ordering::Relaxed);
            fail_work(work, || AppError::Stale);
            continue;
        }
        handle_work(&mut state, &shared, meta, work);
    }
}

fn is_generation_cancellable(work: &Work) -> bool {
    matches!(work, Work::Render { .. })
}

fn fail_work(work: Work, err: impl Fn() -> AppError) {
    match work {
        Work::Open { respond, .. } => respond(Err(err())),
        Work::Sizes { respond, .. } => respond(Err(err())),
        Work::Render { respond, .. } => respond(Err(err())),
        Work::TextLayout { respond, .. } => respond(Err(err())),
        Work::PageLinks { respond, .. } => respond(Err(err())),
        Work::PreviewRect { respond, .. } => respond(Err(err())),
        Work::ResolveCitation { respond, .. } => respond(Err(err())),
        Work::MatchRects { respond, .. } => respond(Err(err())),
        Work::Save { respond, .. } => respond(Err(err())),
        Work::BuildPrintPdf { respond, .. } => respond(Err(err())),
        Work::FormFields { respond, .. } => respond(Err(err())),
        Work::Outline { respond, .. } => respond(Err(err())),
        Work::FormalEnvs { respond, .. } => respond(Err(err())),
        Work::Figures { respond, .. } => respond(Err(err())),
        Work::ImageSize { respond, .. } => respond(Err(err())),
        Work::Close { .. }
        | Work::SetActiveDocument { .. }
        | Work::StartIndexing { .. }
        | Work::IndexNext { .. }
        | Work::LibraryScanNext
        | Work::LibraryTextNext => {}
    }
}

fn handle_work(state: &mut WorkerState, shared: &EngineShared, meta: JobMeta, work: Work) {
    match work {
        Work::Open {
            path,
            password,
            respond,
        } => respond(do_open(state, path, password)),
        Work::Close { doc } => state.close_doc(doc, shared),
        Work::SetActiveDocument { doc } => state.set_active_document(doc, shared),
        Work::Sizes {
            doc,
            from,
            count,
            respond,
        } => respond(do_sizes(state, doc, from, count)),
        Work::Render { key, respond } => {
            let started = Instant::now();
            let result = do_render(state, shared, meta, &key);
            match &result {
                Ok(_) => {
                    shared.metrics.rendered.fetch_add(1, Ordering::Relaxed);
                    log::debug!(
                        "render doc={} src={} kind={:?} scale={} in {:?}",
                        key.doc,
                        key.src,
                        key.kind,
                        key.scale_milli,
                        started.elapsed()
                    );
                }
                Err(AppError::Stale) => {
                    // Count progressive in-flight aborts in the same public
                    // metric as work rejected before dispatch.
                    shared.metrics.skipped_stale.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {}
            }
            respond(result);
        }
        Work::TextLayout { doc, src, respond } => respond(do_text_layout(state, shared, doc, src)),
        Work::PageLinks { doc, src, respond } => respond(do_page_links(state, shared, doc, src)),
        Work::PreviewRect {
            doc,
            src,
            x,
            y,
            respond,
        } => respond(do_preview_rect(state, shared, doc, src, x, y)),
        Work::ResolveCitation {
            id,
            main_epoch,
            respond,
        } => {
            if main_epoch == shared.main_epoch.load(Ordering::Acquire) {
                respond(do_resolve_citation(state, shared, &id));
            } else {
                respond(Ok(None));
            }
        }
        Work::StartIndexing { doc } => state.request_indexing(doc, shared),
        Work::IndexNext { doc, chain } => do_index_next(state, shared, meta, doc, chain),
        Work::LibraryScanNext => do_library_scan_next(state, shared),
        Work::LibraryTextNext => do_library_text_next(state, shared),
        Work::MatchRects {
            doc,
            src,
            start,
            len,
            respond,
        } => respond(do_match_rects(state, shared, doc, src, start, len)),
        Work::Save {
            doc,
            plan,
            dest,
            respond,
        } => respond(do_save(state, doc, plan, dest)),
        Work::BuildPrintPdf {
            doc,
            plan,
            dest,
            respond,
        } => respond(do_build_print_pdf(state, doc, plan, &dest)),
        Work::FormFields { doc, respond } => respond(do_form_fields(state, doc)),
        Work::Outline { doc, respond } => respond(do_outline(state, doc)),
        Work::FormalEnvs { doc, respond } => respond(do_formal_envs(state, doc)),
        Work::Figures { doc, respond } => {
            let result = do_figures(state, doc);
            respond(result)
        }
        Work::ImageSize { path, respond } => respond(
            image::image_dimensions(&path)
                .map(|(w, h)| [w, h])
                .map_err(|e| AppError::Io(format!("cannot read image: {e}"))),
        ),
    }
}

fn do_open(
    state: &mut WorkerState,
    path: String,
    password: Option<String>,
) -> AppResult<DocMetaDto> {
    let raw = render::open_document(state.bindings, &path, password.as_deref())?;
    let page_count = state.bindings.FPDF_GetPageCount(raw).max(0) as u32;
    if page_count == 0 {
        state.bindings.FPDF_CloseDocument(raw);
        return Err(AppError::Malformed("document has no pages".into()));
    }
    let id = state.next_id;
    state.next_id += 1;

    // Opening a document has no effect on the citation/library subsystem —
    // becoming the foreground ("active") document for hover bookkeeping is a
    // separate, explicit step (see `set_active_document`), so opening a
    // background tab never tears down another tab's library hover session.
    let mut sizes_vec: Vec<Option<[f32; 2]>> = vec![None; page_count as usize];
    let mut sizes = Vec::new();
    for i in 0..page_count.min(FIRST_SIZES) {
        if let Some(s) = render::page_display_size(state.bindings, raw, i as i32) {
            sizes_vec[i as usize] = Some(s);
            sizes.push([s[0], s[1], 0.0, 0.0, 0.0]);
        } else {
            sizes.push([612.0, 792.0, 0.0, 0.0, 0.0]);
        }
    }
    let mut widths: Vec<f32> = sizes.iter().map(|s| s[0]).collect();
    let mut heights: Vec<f32> = sizes.iter().map(|s| s[1]).collect();
    widths.sort_by(f32::total_cmp);
    heights.sort_by(f32::total_cmp);
    let estimated = [
        widths.get(widths.len() / 2).copied().unwrap_or(612.0),
        heights.get(heights.len() / 2).copied().unwrap_or(792.0),
    ];

    let name = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    state.docs.insert(
        id,
        DocState {
            raw,
            path: path.clone(),
            password,
            page_count,
            sizes: sizes_vec,
            pages: Vec::new(),
            index_cursor: 0,
            form_fields: None,
            formal_envs: None,
        },
    );
    Ok(DocMetaDto {
        doc_id: id,
        path,
        name,
        page_count,
        sizes,
        estimated_size: estimated,
    })
}

fn do_sizes(state: &mut WorkerState, doc: DocId, from: u32, count: u32) -> AppResult<PageSizesDto> {
    let b = state.bindings;
    let d = state.doc(doc)?;
    let end = (from + count).min(d.page_count);
    let mut sizes = Vec::new();
    for i in from..end {
        let s = match d.sizes.get(i as usize).copied().flatten() {
            Some(s) => s,
            None => {
                let s = render::page_display_size(b, d.raw, i as i32).unwrap_or([612.0, 792.0]);
                if let Some(slot) = d.sizes.get_mut(i as usize) {
                    *slot = Some(s);
                }
                s
            }
        };
        sizes.push([s[0], s[1], 0.0, 0.0, 0.0]);
    }
    Ok(PageSizesDto { from, sizes })
}

fn do_render(
    state: &mut WorkerState,
    shared: &EngineShared,
    meta: JobMeta,
    key: &RenderKey,
) -> AppResult<Arc<Vec<u8>>> {
    // Another request may have populated the cache while this job queued.
    if let Some(hit) = match key.kind {
        RenderKind::Thumb => shared.thumb_cache.lock().get(key),
        RenderKind::Preview => shared.preview_cache.lock().get(key),
        _ => shared.page_cache.lock().get(key),
    } {
        return Ok(hit);
    }
    let [w_pt, h_pt] = state.display_size(key.doc, key.src)?;
    let page = state.page_handle(key.doc, key.src)?;
    let out = render::render_page(
        state.bindings,
        page,
        [w_pt, h_pt],
        key.scale_milli as f32 / 1000.0,
        key.rot,
        key.tile,
        || shared.is_render_stale(&meta),
    )?;
    let bytes = Arc::new(out.png);
    // The Rust cache stores encoded PNG bytes. Browser-decoded memory is
    // bounded separately by page virtualization, not by overcharging every
    // cached PNG as if it remained mounted RGBA.
    let cost = bytes.len() as u64;
    match key.kind {
        RenderKind::Thumb => {
            shared
                .thumb_cache
                .lock()
                .insert(key.clone(), Arc::clone(&bytes), cost)
        }
        RenderKind::Preview => {
            shared
                .preview_cache
                .lock()
                .insert(key.clone(), Arc::clone(&bytes), cost)
        }
        _ => shared
            .page_cache
            .lock()
            .insert(key.clone(), Arc::clone(&bytes), cost),
    }
    Ok(bytes)
}

fn ensure_search_extracted(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<(u32, bool)> {
    if shared.search.lock().contains_page(doc, src) {
        return Ok((0, true));
    }
    let page = state.page_handle(doc, src)?;
    let extracted = text::extract_search_text(state.bindings, page);
    let stored = shared
        .search
        .lock()
        .store_page(doc, src, &extracted.raw, extracted.char_count);
    if stored {
        shared.metrics.pages_indexed.fetch_add(1, Ordering::Relaxed);
    }
    Ok((extracted.char_count, stored))
}

fn ensure_text_layout(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<TextLayoutData> {
    if let Some(layout) = shared.text_layouts.lock().get(doc, src) {
        return Ok(layout);
    }
    let [w_pt, h_pt] = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    let extracted = text::extract_page(state.bindings, page, w_pt, h_pt);

    // A foreground request can extend search coverage, but failure to retain
    // search text must never erase the freshly extracted selectable layout.
    if !shared.search.lock().contains_page(doc, src) {
        let stored =
            shared
                .search
                .lock()
                .store_page(doc, src, &extracted.raw, extracted.char_count);
        if stored {
            shared.metrics.pages_indexed.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(shared.text_layouts.lock().insert(
        doc,
        src,
        extracted.runs,
        extracted.boxes,
        extracted.char_count,
    ))
}

fn do_text_layout(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<PageTextDto> {
    let layout = ensure_text_layout(state, shared, doc, src)?;
    Ok(PageTextDto {
        src,
        runs: layout.runs.as_ref().clone(),
        char_count: layout.char_count,
    })
}

fn do_page_links(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<PageLinksDto> {
    if let Some(cached) = shared.link_cache.lock().get(doc, src) {
        return Ok(PageLinksDto {
            src,
            links: cached.as_ref().clone(),
        });
    }
    let (raw, page_count) = {
        let document = state.doc(doc)?;
        if src >= document.page_count {
            return Err(AppError::Malformed(format!("page {src} is out of range")));
        }
        (document.raw, document.page_count)
    };
    let layout = ensure_text_layout(state, shared, doc, src)?;
    let display_size = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    let text_page = state.bindings.FPDFText_LoadPage(page);
    let extracted = links::extract_links(
        state.bindings,
        raw,
        page,
        text_page,
        display_size,
        &layout.boxes,
        page_count,
    );
    if !text_page.is_null() {
        state.bindings.FPDFText_ClosePage(text_page);
    }
    let cached = shared.link_cache.lock().insert(doc, src, extracted);
    Ok(PageLinksDto {
        src,
        links: cached.as_ref().clone(),
    })
}

fn do_preview_rect(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
    x: Option<f32>,
    y: Option<f32>,
) -> AppResult<PreviewSpecDto> {
    let [page_w, page_h] = state.display_size(doc, src)?;
    let layout = ensure_text_layout(state, shared, doc, src)?;
    let rect = preview::crop_rect(&CropInput {
        page_w_pt: page_w,
        page_h_pt: page_h,
        dest_x: x,
        dest_y: y,
        runs: &layout.runs,
    });
    let tile = preview::rect_to_tile(rect, page_h, PREVIEW_SCALE_MILLI);
    let [crop_x, crop_y, crop_w, crop_h] = rect;
    let mut preview_text = layout
        .runs
        .iter()
        .filter(|run| {
            run.x + run.w > crop_x
                && run.x < crop_x + crop_w
                && run.y + run.h > crop_y
                && run.y < crop_y + crop_h
        })
        .map(|run| run.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    preview_text = preview_text.chars().take(400).collect();
    Ok(PreviewSpecDto {
        doc_id: doc,
        src,
        tile,
        scale_milli: PREVIEW_SCALE_MILLI,
        text: preview_text,
    })
}

fn do_index_next(
    state: &mut WorkerState,
    shared: &EngineShared,
    meta: JobMeta,
    doc: DocId,
    chain: u64,
) {
    // Drop chains the scheduler has since superseded (document closed, or a
    // tab switch handed the indexer to the newly foreground document).
    if state.index_active != Some((doc, chain)) {
        return;
    }
    let Ok(d) = state.doc(doc) else {
        state.end_indexing(doc, true, shared);
        return;
    };
    if shared.search.lock().is_truncated(doc) {
        state.end_indexing(doc, true, shared);
        return;
    }
    let total = d.page_count;
    // find next page missing from the index
    let mut next = d.index_cursor;
    {
        let search = shared.search.lock();
        while next < total && search.contains_page(doc, next) {
            next += 1;
        }
    }
    if next >= total {
        emit_progress(shared, doc, total, total, false);
        state.end_indexing(doc, true, shared);
        return;
    }
    let ok = match ensure_search_extracted(state, shared, doc, next) {
        Ok((_, stored)) => stored,
        Err(_) => true, // unreadable page: skip it, keep going
    };
    if let Ok(d) = state.doc(doc) {
        d.index_cursor = next + 1;
    }
    let indexed = shared.search.lock().indexed_count(doc);
    let truncated = shared.search.lock().is_truncated(doc);
    if truncated || next + 1 >= total || (next + 1) % 32 == 0 {
        emit_progress(shared, doc, indexed, total, truncated);
    }
    if !ok {
        log::warn!("search index budget exhausted for doc {doc} after {indexed}/{total} pages");
        state.end_indexing(doc, true, shared);
        return;
    }
    // Requeue at the same stamps: closing/reloading the doc cancels this.
    shared.queue.push(
        Priority::TextExtract,
        JobMeta {
            doc,
            gen: meta.gen,
            epoch: meta.epoch,
        },
        Work::IndexNext { doc, chain },
    );
}

/// Extract one library document's full text to its sidecar.
///
/// One document per job, then back to the queue — the same yielding shape as
/// `do_library_scan_next`, and the reason a library indexing pass does not
/// make the app feel slow: anything visible outranks `Idle`.
///
/// A document that cannot be read is marked done rather than retried. It will
/// not start being readable, and leaving it at the head of the queue would
/// stall every document behind it forever.
fn do_library_text_next(state: &mut WorkerState, shared: &EngineShared) {
    let (app_data_dir, candidate) = {
        let library = shared.library.lock();
        (library.app_data_dir(), library.next_text_candidate())
    };
    let Some(entry) = candidate else {
        return;
    };

    if let Some(pages) = extract_library_text(state, &entry.path) {
        if let Err(error) = crate::search::librarytext::write_sidecar(
            &app_data_dir,
            &entry.path,
            entry.mtime_ms,
            entry.size,
            &pages,
        ) {
            log::warn!("cannot index {} for search: {error}", entry.path.display());
        }
    }
    shared.library.lock().finish_text(&entry.path);

    if shared.library.lock().has_text_work() {
        shared.queue.push(
            Priority::Idle,
            JobMeta {
                doc: 0,
                gen: shared.gens.current(0),
                epoch: shared.cancels.current(0),
            },
            Work::LibraryTextNext,
        );
    }
}

/// Every page's text, using the cheap extraction path — no character boxes, no
/// font info, which is thousands of FFI calls a page the index does not need.
fn extract_library_text(state: &mut WorkerState, path: &Path) -> Option<Vec<(u32, String)>> {
    let path = path.to_str()?;
    let doc = render::open_document(state.bindings, path, None).ok()?;
    let count = state.bindings.FPDF_GetPageCount(doc).max(0);
    let mut pages = Vec::new();
    for index in 0..count {
        let page = state.bindings.FPDF_LoadPage(doc, index);
        if page.is_null() {
            continue;
        }
        let extracted = crate::engine::text::extract_search_text(state.bindings, page);
        state.bindings.FPDF_ClosePage(page);
        if !extracted.raw.trim().is_empty() {
            pages.push((index as u32, extracted.raw));
        }
    }
    state.bindings.FPDF_CloseDocument(doc);
    Some(pages)
}

fn scan_library_candidate(
    state: &mut WorkerState,
    candidate: &library::ScanCandidate,
) -> LibraryEntry {
    if let Some(entry) = library::entry_from_filename(candidate) {
        return entry;
    }
    let Some(path) = candidate.path.to_str() else {
        return library::entry_from_first_page(candidate, "");
    };
    let raw = match render::open_document(state.bindings, path, None) {
        Ok(raw) => raw,
        Err(error) => {
            log::warn!(
                "cannot scan library PDF {}: {error}",
                candidate.path.display()
            );
            return library::entry_from_first_page(candidate, "");
        }
    };
    let page = state.bindings.FPDF_LoadPage(raw, 0);
    let text = if page.is_null() {
        String::new()
    } else {
        let extracted = text::extract_search_text(state.bindings, page);
        state.bindings.FPDF_ClosePage(page);
        extracted.raw
    };
    state.bindings.FPDF_CloseDocument(raw);
    library::entry_from_first_page(candidate, &text)
}

fn do_library_scan_next(state: &mut WorkerState, shared: &EngineShared) {
    let candidate = shared.library.lock().next_candidate();
    let Some(candidate) = candidate else {
        return;
    };
    let entry = (!candidate.unchanged).then(|| scan_library_candidate(state, &candidate));
    shared.library.lock().finish_candidate(&candidate, entry);
    let (more_scan, more_text) = {
        let library = shared.library.lock();
        (library.has_scan_work(), library.has_text_work())
    };
    let requeue = |work| {
        shared.queue.push(
            Priority::Idle,
            JobMeta {
                doc: 0,
                gen: shared.gens.current(0),
                epoch: shared.cancels.current(0),
            },
            work,
        );
    };
    if more_scan {
        requeue(Work::LibraryScanNext);
    } else if more_text {
        // The metadata pass is what discovers files, so the text pass starts
        // once it has nothing left to find.
        requeue(Work::LibraryTextNext);
    }
}

fn open_library_document(
    state: &mut WorkerState,
    shared: &EngineShared,
    path: &Path,
) -> AppResult<DocId> {
    if let Some(position) = state.library_docs.iter().position(|id| {
        state
            .docs
            .get(id)
            .is_some_and(|document| std::path::Path::new(&document.path) == path)
    }) {
        let id = state.library_docs.remove(position);
        state.library_docs.push(id);
        return Ok(id);
    }

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let Some(path_string) = path.to_str() else {
        return Err(AppError::Io(format!("Could not open {file_name}")));
    };
    let raw = match render::open_document(state.bindings, path_string, None) {
        Ok(raw) => raw,
        Err(error) => {
            log::warn!(
                "could not open citation preview {}: {error}",
                path.display()
            );
            return Err(AppError::Io(format!("Could not open {file_name}")));
        }
    };
    let page_count = state.bindings.FPDF_GetPageCount(raw).max(0) as u32;
    if page_count == 0 {
        state.bindings.FPDF_CloseDocument(raw);
        log::warn!("citation preview has no pages: {}", path.display());
        return Err(AppError::Malformed(format!("Could not open {file_name}")));
    }
    if state.library_docs.len() >= LIBRARY_DOC_CAP {
        let victim = state.library_docs.remove(0);
        // Keep any already-rendered PNG alive in the separate bounded preview
        // cache; this preserves instant memoized re-hover without retaining a
        // fourth PDFium document handle.
        state.close_one(victim, shared, false);
    }
    let id = state.next_id;
    state.next_id += 1;
    state.docs.insert(
        id,
        DocState {
            raw,
            path: path_string.to_owned(),
            password: None,
            page_count,
            sizes: vec![None; page_count as usize],
            pages: Vec::new(),
            index_cursor: 0,
            form_fields: None,
            formal_envs: None,
        },
    );
    state.library_docs.push(id);
    state.library_session_docs.push(id);
    Ok(id)
}

fn do_resolve_citation(
    state: &mut WorkerState,
    shared: &EngineShared,
    id: &CitationIdDto,
) -> AppResult<Option<ResolvedCitationDto>> {
    let path = shared
        .citation_resolver
        .lock()
        .as_ref()
        .and_then(|resolver| resolver.resolve(id));
    let Some(path) = path else {
        return Ok(None);
    };
    let metadata = shared.library.lock().entry_for_path(&path);
    let doc_id = open_library_document(state, shared, &path)?;
    let page_count = state.doc(doc_id)?.page_count;
    let [page_w, page_h] = state.display_size(doc_id, 0)?;
    let tile = TileRect {
        x: 0,
        y: 0,
        w: (page_w * PREVIEW_SCALE_MILLI as f32 / 1_000.0)
            .round()
            .clamp(1.0, 4_096.0) as u32,
        h: (page_h * PREVIEW_SCALE_MILLI as f32 / 1_000.0)
            .round()
            .clamp(1.0, 4_096.0) as u32,
    };
    let (text, extracted_title) = ensure_text_layout(state, shared, doc_id, 0)
        .map(|layout| {
            let text = layout
                .runs
                .iter()
                .map(|run| run.text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(400)
                .collect();
            let title = layout
                .runs
                .iter()
                .map(|run| run.text.trim())
                .find(|line| line.chars().count() > 12)
                .map(|line| line.chars().take(300).collect());
            (text, title)
        })
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    Ok(Some(ResolvedCitationDto {
        doc_id,
        title: metadata.and_then(|entry| entry.title).or(extracted_title),
        page_count,
        file_name,
        path: path.to_string_lossy().into_owned(),
        preview: PreviewSpecDto {
            doc_id,
            src: 0,
            tile,
            scale_milli: PREVIEW_SCALE_MILLI,
            text,
        },
    }))
}

fn emit_progress(shared: &EngineShared, doc: DocId, indexed: u32, total: u32, truncated: bool) {
    if let Some(cb) = shared.progress.lock().as_ref() {
        let chars_indexed = shared.search.lock().chars_indexed(doc);
        cb(
            doc,
            SearchStatusDto {
                indexed,
                total,
                truncated,
                chars_indexed,
            },
        );
    }
}

fn do_match_rects(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
    start: u32,
    len: u32,
) -> AppResult<Vec<[f32; 4]>> {
    let layout = ensure_text_layout(state, shared, doc, src)?;
    Ok(text::merge_match_rects(&layout.boxes, start, len))
}

/// Materialize the document as it currently stands into a throwaway file, for
/// printing.
///
/// The same page assembly, rotation, annotation and form work `do_save`
/// performs — this is what puts unsaved edits on paper — but without the
/// atomic-replace machinery around it. That exists to protect a destination
/// worth protecting; here the destination is a temp file nobody has yet seen,
/// so the rename dance and the reopen-verify pass would be pure cost. And
/// because the destination is never the document's own path, the close-and-
/// reopen that `do_save` needs on Windows cannot apply: the open document is
/// left entirely alone.
fn do_build_print_pdf(
    state: &mut WorkerState,
    doc: DocId,
    plan: EditPlan,
    dest: &std::path::Path,
) -> AppResult<()> {
    if plan.pages.is_empty() {
        return Err(AppError::Unsupported("nothing to print".into()));
    }
    if plan.pages.len() > u16::MAX as usize {
        return Err(AppError::Unsupported(
            "printing is limited to 65,535 pages".into(),
        ));
    }
    let (src_path, password) = {
        let d = state.doc(doc)?;
        (d.path.clone(), d.password.clone())
    };
    save::build_output(state.pdfium, &src_path, password.as_deref(), &plan, dest)
}

fn do_save(
    state: &mut WorkerState,
    doc: DocId,
    plan: EditPlan,
    dest: PathBuf,
) -> AppResult<SaveResultDto> {
    if plan.pages.len() > u16::MAX as usize {
        return Err(AppError::Unsupported(
            "save is limited to 65,535 output pages".into(),
        ));
    }
    let started = Instant::now();
    let (src_path, password, same_file, page_count, sizes, index_cursor, form_fields) = {
        let d = state.doc(doc)?;
        (
            d.path.clone(),
            d.password.clone(),
            std::path::Path::new(&d.path) == dest.as_path(),
            d.page_count,
            d.sizes.clone(),
            d.index_cursor,
            d.form_fields.clone(),
        )
    };
    let pdfium = state.pdfium;
    let bindings = state.bindings;
    let expected_pages = plan.pages.len();
    let docs = &mut state.docs;
    let mut closed_for_replace = false;
    let save_result = save::verified_atomic_replace(
        &dest,
        |temp_path| save::build_output(pdfium, &src_path, password.as_deref(), &plan, temp_path),
        |temp_path| {
            let path = temp_path
                .to_str()
                .ok_or_else(|| AppError::Io("bad temp path".into()))?;
            let verify = render::open_document(bindings, path, None)?;
            let count = bindings.FPDF_GetPageCount(verify).max(0) as usize;
            bindings.FPDF_CloseDocument(verify);
            if count != expected_pages {
                return Err(AppError::Internal(format!(
                    "verification failed: wrote {count} pages, expected {expected_pages}"
                )));
            }
            Ok(())
        },
        || {
            // Windows cannot rename over an open mapping. The frontend
            // immediately reopens the persisted file after a successful save.
            if same_file {
                if let Some(mut d) = docs.remove(&doc) {
                    for (_, p) in d.pages.drain(..) {
                        bindings.FPDF_ClosePage(p);
                    }
                    bindings.FPDF_CloseDocument(d.raw);
                    closed_for_replace = true;
                }
            }
            Ok(())
        },
    );
    let bytes = match save_result {
        Ok(bytes) => bytes,
        Err(save_error) => {
            // A same-path save must close the source before Windows can
            // replace it. If replacement itself fails, restore the engine
            // session over the still-intact destination so the user's
            // in-memory EditPlan remains retryable.
            if same_file && closed_for_replace {
                match render::open_document(bindings, &src_path, password.as_deref()) {
                    Ok(raw) => {
                        docs.insert(
                            doc,
                            DocState {
                                raw,
                                path: src_path,
                                password,
                                page_count,
                                sizes,
                                pages: Vec::new(),
                                index_cursor,
                                form_fields,
                                // page indices moved; recompute on next ask
                                formal_envs: None,
                            },
                        );
                    }
                    Err(restore_error) => {
                        return Err(AppError::Internal(format!(
                            "{save_error}; could not restore document session: {restore_error}"
                        )));
                    }
                }
            }
            return Err(save_error);
        }
    };

    Ok(SaveResultDto {
        path: dest.to_string_lossy().into_owned(),
        bytes,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// A bookmark's target is either a direct destination, or (more commonly for
/// links authored in modern tools) a GoTo action wrapping one. `1` is
/// PDFium's `PDFACTION_GOTO` constant (not exposed as a Rust const by
/// pdfium-render, only documented on the raw binding).
const PDFACTION_GOTO: std::os::raw::c_ulong = 1;

/// The page a bookmark lands on, plus the y of its destination when the
/// destination carries one — which lets the table of contents land on the
/// heading itself rather than the top of its page.
fn resolve_bookmark_target(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    bookmark: FPDF_BOOKMARK,
) -> (Option<u32>, Option<f32>) {
    let mut dest = bindings.FPDFBookmark_GetDest(document, bookmark);
    if dest.is_null() {
        let action = bindings.FPDFBookmark_GetAction(bookmark);
        if !action.is_null() && bindings.FPDFAction_GetType(action) == PDFACTION_GOTO {
            dest = bindings.FPDFAction_GetDest(document, action);
        }
    }
    if dest.is_null() {
        return (None, None);
    }
    let index = bindings.FPDFDest_GetDestPageIndex(document, dest);
    if index < 0 {
        return (None, None);
    }
    let (mut has_x, mut has_y, mut has_zoom) = (0, 0, 0);
    let (mut x, mut y, mut zoom) = (0.0f32, 0.0f32, 0.0f32);
    bindings.FPDFDest_GetLocationInPage(
        dest,
        &mut has_x,
        &mut has_y,
        &mut has_zoom,
        &mut x,
        &mut y,
        &mut zoom,
    );
    // A /Fit destination has no coordinates; the page alone is the answer.
    (Some(index as u32), (has_y != 0).then_some(y))
}

/// Walks the bookmark tree depth-first. `visited` guards against the
/// circular sibling/child references PDFium's own docs warn malformed PDFs
/// can produce; `depth` is a second, cheaper backstop.
fn walk_bookmarks(
    bindings: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    parent: FPDF_BOOKMARK,
    depth: u32,
    visited: &mut std::collections::HashSet<usize>,
) -> Vec<OutlineNodeDto> {
    if depth > 64 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut current = bindings.FPDFBookmark_GetFirstChild(document, parent);
    while !current.is_null() {
        if !visited.insert(current as usize) {
            break;
        }
        let title = read_pdfium_utf16(|buffer, len| {
            bindings.FPDFBookmark_GetTitle(current, buffer.cast(), len)
        });
        let (page, y) = resolve_bookmark_target(bindings, document, current);
        let children = walk_bookmarks(bindings, document, current, depth + 1, visited);
        out.push(OutlineNodeDto {
            title,
            page,
            y,
            children,
        });
        current = bindings.FPDFBookmark_GetNextSibling(document, current);
    }
    out
}

/// Headings recovered from hyperref's named destinations, for documents that
/// ship no bookmark tree.
///
/// Plenty of arXiv papers are in that position: `/Outlines` is simply absent,
/// while `section.4`, `subsection.4.1` and `appendix.A` are all still there in
/// `/Dests`. The destination gives the position and the nesting depth; the
/// title has to be read off the page, because the name encodes only a counter.
///
/// Returned flat, in document order, with the depth `heading_depth` assigns.
fn headings_from_destinations(state: &mut WorkerState, doc: DocId) -> Vec<(u8, FormalEntryDto)> {
    let (Ok(raw), Ok(page_count)) = (
        state.doc(doc).map(|d| d.raw),
        state.doc(doc).map(|d| d.page_count),
    ) else {
        return Vec::new();
    };
    let mut anchors = crate::engine::formal::enumerate_anchors(state.bindings, raw, page_count);
    anchors.sort_by(|a, b| a.page.cmp(&b.page).then(b.y.total_cmp(&a.y)));

    let mut out: Vec<(u8, u8, FormalEntryDto)> = Vec::new();
    let mut current: Option<(
        u32,
        crate::engine::text::ExtractedPage,
        Vec<crate::engine::tagged::MathSpan>,
    )> = None;
    for anchor in anchors {
        let Some(depth) = crate::engine::formal::heading_depth(&anchor.name) else {
            continue;
        };
        if crate::engine::formal::is_structural_destination(&anchor.name) {
            continue;
        }
        let (extracted, spans) = match &current {
            Some((p, e, s)) if *p == anchor.page => (e, s),
            _ => {
                let Some((e, s)) = extract_with_tags(state, doc, anchor.page) else {
                    continue;
                };
                current = Some((anchor.page, e, s));
                let entry = current.as_ref().unwrap();
                (&entry.1, &entry.2)
            }
        };
        // The counter in the destination's own name is what makes the title
        // checkable; without one there is nothing to verify against, so the
        // entry is skipped rather than guessed at.
        let Some(counter) = crate::engine::formal::destination_number(&anchor.name) else {
            continue;
        };
        let meta = crate::engine::formal::GlyphMeta::new(&extracted.sizes, &extracted.math)
            .with_spans(spans);
        let Some(hit) = crate::engine::formal::heading_line(
            &extracted.raw,
            &extracted.boxes,
            meta,
            anchor.x,
            anchor.y,
            counter,
        )
        .or_else(|| {
            // The column filter keeps the arXiv margin stamp out, but it also
            // hides a heading whose anchor x landed in the other column.
            crate::engine::formal::heading_line(
                &extracted.raw,
                &extracted.boxes,
                meta,
                None,
                anchor.y,
                counter,
            )
        }) else {
            continue;
        };
        // Document order comes from where the heading *is*. On a two-column
        // page that is column-major: a heading low in the left column still
        // precedes one high in the right, which sorting on y alone reverses.
        let page_w = state
            .display_size(doc, anchor.page)
            .map(|size| size[0])
            .unwrap_or(612.0);
        let column = u8::from(hit.x > page_w / 2.0);
        out.push((
            depth,
            column,
            FormalEntryDto {
                heading: true,
                depth: depth.min(1),
                label: hit.title,
                page: anchor.page,
                y: hit.y,
                char_index: 0,
                x: Some(hit.x),
            },
        ));
    }
    out.sort_by(|a, b| {
        a.2.page
            .cmp(&b.2.page)
            .then(a.1.cmp(&b.1))
            .then(b.2.y.total_cmp(&a.2.y))
    });
    out.into_iter()
        .map(|(depth, _, entry)| (depth, entry))
        .collect()
}

/// Nest a flat depth-tagged heading list into the tree the outline renders.
fn nest_headings(flat: Vec<(u8, FormalEntryDto)>) -> Vec<OutlineNodeDto> {
    let mut roots: Vec<OutlineNodeDto> = Vec::new();
    for (depth, entry) in flat {
        let node = OutlineNodeDto {
            title: entry.label,
            page: Some(entry.page),
            y: Some(entry.y),
            children: Vec::new(),
        };
        // Only two levels exist here (heading_depth yields 0 or 1), so a
        // subsection attaches to the section standing above it, and one with
        // no section above it stays at the top rather than being dropped.
        match (depth, roots.last_mut()) {
            (0, _) | (_, None) => roots.push(node),
            (_, Some(parent)) => parent.children.push(node),
        }
    }
    roots
}

fn do_outline(state: &mut WorkerState, doc: DocId) -> AppResult<Vec<OutlineNodeDto>> {
    let raw = state.doc(doc)?.raw;
    let mut visited = std::collections::HashSet::new();
    let bookmarks = walk_bookmarks(state.bindings, raw, std::ptr::null_mut(), 0, &mut visited);
    if !bookmarks.is_empty() {
        return Ok(bookmarks);
    }
    Ok(nest_headings(headings_from_destinations(state, doc)))
}

/// Formal environments (theorems, figures, …) recovered from hyperref's named
/// destinations, cross-checked against the text printed at each anchor. See
/// engine::formal for why both halves are needed.
fn do_formal_envs(state: &mut WorkerState, doc: DocId) -> AppResult<Vec<FormalEntryDto>> {
    if let Some(cached) = state.doc(doc)?.formal_envs.clone() {
        return Ok(cached);
    }
    let raw = state.doc(doc)?.raw;
    let page_count = state.doc(doc)?.page_count;
    let b = state.bindings;

    let mut anchors = crate::engine::formal::enumerate_anchors(b, raw, page_count);

    // Document order: page, then down the page (display space is y-up).
    anchors.sort_by(|a, b| a.page.cmp(&b.page).then(b.y.total_cmp(&a.y)));

    // One text layout per page, shared by every anchor on it. Sections and
    // environments interleave in this single ordered pass, so the section a
    // given environment falls under is simply the last one seen before it.
    // Sections and environments interleave in one ordered pass, so a section
    // heading is simply emitted before the rows that follow it. A heading is
    // held back until something actually lands under it — a section with no
    // environments is noise in a list that exists to find environments.
    // Headings come from the bookmark tree, not from the page text. hyperref
    // writes the same titles there, already numbered and already nested, and
    // reading them avoids a problem the page cannot solve: a run-in heading
    // shares its line with the paragraph it introduces, so
    // "1.2 Preliminaries We use kvk2 to denote..." has no boundary to find.
    let mut headings: Vec<FormalEntryDto> = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let outline = walk_bookmarks(b, raw, std::ptr::null_mut(), 0, &mut visited);
    fn collect(nodes: &[OutlineNodeDto], depth: u8, out: &mut Vec<FormalEntryDto>) {
        for node in nodes {
            if let Some(page) = node.page {
                let label = node.title.trim();
                if !label.is_empty() {
                    out.push(FormalEntryDto {
                        heading: true,
                        // Only two heading levels are shown; deeper bookmarks
                        // still group, they just stop indenting further.
                        depth: depth.min(1),
                        label: label.to_string(),
                        page,
                        // no coordinates means the bookmark points at the page
                        // itself, which sorts as the very top of it
                        y: node.y.unwrap_or(f32::MAX),
                        char_index: 0,
                        x: None,
                    });
                }
            }
            collect(&node.children, depth + 1, out);
        }
    }
    collect(&outline, 0, &mut headings);
    headings.sort_by(|a, c| a.page.cmp(&c.page).then(c.y.total_cmp(&a.y)));
    // No bookmark tree: recover the same headings from the section
    // destinations, so the panel still groups by section instead of showing a
    // flat list of theorems. Already in document order, and deliberately not
    // re-sorted here — that sort is y-only, which reverses two-column pages.
    if headings.is_empty() {
        headings = headings_from_destinations(state, doc)
            .into_iter()
            .map(|(_, entry)| entry)
            .collect();
    }

    let mut environments: Vec<FormalEntryDto> = Vec::new();
    let mut current: Option<(u32, crate::engine::text::ExtractedPage)> = None;
    for anchor in anchors {
        let (name, page, x, y) = (anchor.name, anchor.page, anchor.x, anchor.y);
        if crate::engine::formal::heading_depth(&name).is_some() {
            continue; // the bookmark tree already has this one
        }
        let extracted = match &current {
            Some((p, e)) if *p == page => e,
            _ => {
                let (Ok(handle), Ok(size)) =
                    (state.page_handle(doc, page), state.display_size(doc, page))
                else {
                    continue;
                };
                let e = crate::engine::text::extract_page(b, handle, size[0], size[1]);
                current = Some((page, e));
                &current.as_ref().unwrap().1
            }
        };
        // Read from the anchor first; it is right whenever the destination
        // actually sits on the statement.
        let anchored = crate::engine::formal::anchor_text(&extracted.raw, &extracted.boxes, x, y)
            .and_then(|text| crate::engine::formal::reconcile(&name, &text))
            .map(|heading| (heading, y));
        // Otherwise search the page for the statement the counter names — the
        // anchor may have landed in a formula, or on the page number.
        let found = anchored.or_else(|| {
            let counter = crate::engine::formal::destination_number(&name)?;
            crate::engine::formal::environment_line(
                &extracted.raw,
                &extracted.boxes,
                crate::engine::formal::GlyphMeta::new(&extracted.sizes, &extracted.math),
                y,
                counter,
            )
        });
        if let Some((heading, found_y)) = found {
            environments.push(FormalEntryDto {
                heading: false,
                depth: 0, // set below, once its heading context is known
                label: format!("{} {}", heading.kind, heading.number),
                page,
                y: found_y,
                char_index: crate::engine::formal::anchor_start(&extracted.boxes, x, found_y)
                    .unwrap_or(0) as u32,
                x,
            });
        }
    }

    let out = crate::engine::formal::merge_structure(headings, environments);

    state.doc(doc)?.formal_envs = Some(out.clone());
    Ok(out)
}

/// Extract a page's text along with any mathematics its structure tree tags.
///
/// Tagging is rare — most papers carry no structure tree — so the extra text
/// page is only opened when one exists, and the span list is empty otherwise.
fn extract_with_tags(
    state: &mut WorkerState,
    doc: DocId,
    page: u32,
) -> Option<(
    crate::engine::text::ExtractedPage,
    Vec<crate::engine::tagged::MathSpan>,
)> {
    let (Ok(handle), Ok(size)) = (state.page_handle(doc, page), state.display_size(doc, page))
    else {
        return None;
    };
    let extracted = crate::engine::text::extract_page(state.bindings, handle, size[0], size[1]);
    let mut spans = Vec::new();
    let text_page = state.bindings.FPDFText_LoadPage(handle);
    if !text_page.is_null() {
        spans = crate::engine::tagged::page_math(state.bindings, handle, text_page, &extracted.raw);
        state.bindings.FPDFText_ClosePage(text_page);
    }
    Some((extracted, spans))
}

/// Figure rows: every caption in the document, each paired with a crop of the
/// artwork sitting above it.
///
/// Captions come from page text rather than hyperref anchors, so this works on
/// papers whose figures carry no `\label` — which is most of them. The cost is
/// one text extraction per page, paid when the panel is opened.
fn do_figures(state: &mut WorkerState, doc: DocId) -> AppResult<Vec<FigureDto>> {
    use crate::engine::figures::{
        figure_crop_rect, find_captions, FigureCropInput, FIGURE_THUMB_SCALE_MILLI,
    };

    let page_count = state.doc(doc)?.page_count;
    let mut out = Vec::new();
    for page in 0..page_count {
        let Ok([page_w, page_h]) = state.display_size(doc, page) else {
            continue;
        };
        // The character stream, not the cached run layout: runs drop the
        // inter-word spaces a caption needs, and carry no per-glyph geometry,
        // so neither the caption text nor its scripts survive them.
        let Some((extracted, spans)) = extract_with_tags(state, doc, page) else {
            continue;
        };
        let lines = crate::engine::formal::page_lines(
            &extracted.raw,
            &extracted.boxes,
            crate::engine::formal::GlyphMeta::new(&extracted.sizes, &extracted.math)
                .with_spans(&spans),
        );
        for caption in find_captions(&lines) {
            let rect = figure_crop_rect(&FigureCropInput {
                page_w_pt: page_w,
                page_h_pt: page_h,
                caption_y: caption.y,
                caption_x: Some(caption.x),
                runs: &extracted.runs,
            });
            out.push(FigureDto {
                label: caption.label,
                title: caption.title,
                page,
                y: caption.y,
                tile: preview::rect_to_tile(rect, page_h, FIGURE_THUMB_SCALE_MILLI),
                scale_milli: FIGURE_THUMB_SCALE_MILLI,
            });
        }
    }
    Ok(out)
}

fn do_form_fields(state: &mut WorkerState, doc: DocId) -> AppResult<Vec<FormFieldDto>> {
    let (raw, page_count, cached) = {
        let d = state.doc(doc)?;
        (d.raw, d.page_count, d.form_fields.clone())
    };
    if let Some(fields) = cached {
        return Ok(fields);
    }

    let bindings = state.bindings;
    if bindings.FPDF_GetFormType(raw) == 0 {
        state.doc(doc)?.form_fields = Some(Vec::new());
        return Ok(Vec::new());
    }

    // PDFium retains this pointer until ExitFormFillEnvironment(), so the
    // callback table must stay pinned for the complete scan.
    let mut form_fill_info = Box::pin(FPDF_FORMFILLINFO {
        version: 2,
        Release: None,
        FFI_Invalidate: None,
        FFI_OutputSelectedRect: None,
        FFI_SetCursor: None,
        FFI_SetTimer: None,
        FFI_KillTimer: None,
        FFI_GetLocalTime: None,
        FFI_OnChange: None,
        FFI_GetPage: None,
        FFI_GetCurrentPage: None,
        FFI_GetRotation: None,
        FFI_ExecuteNamedAction: None,
        FFI_SetTextFieldFocus: None,
        FFI_DoURIAction: None,
        FFI_DoGoToAction: None,
        m_pJsPlatform: std::ptr::null_mut(),
        xfa_disabled: 0,
        FFI_DisplayCaret: None,
        FFI_GetCurrentPageIndex: None,
        FFI_SetCurrentPage: None,
        FFI_GotoURL: None,
        FFI_GetPageViewRect: None,
        FFI_PageEvent: None,
        FFI_PopupMenu: None,
        FFI_OpenFile: None,
        FFI_EmailTo: None,
        FFI_UploadTo: None,
        FFI_GetPlatform: None,
        FFI_GetLanguage: None,
        FFI_DownloadFromURL: None,
        FFI_PostRequestURL: None,
        FFI_PutRequestURL: None,
        FFI_OnFocusChange: None,
        FFI_DoURIActionWithKeyboardModifier: None,
    });
    let form = bindings.FPDFDOC_InitFormFillEnvironment(raw, form_fill_info.as_mut().get_mut());
    if form.is_null() {
        state.doc(doc)?.form_fields = Some(Vec::new());
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for pi in 0..page_count {
        let page = bindings.FPDF_LoadPage(raw, pi as i32);
        if page.is_null() {
            continue;
        }
        bindings.FORM_OnAfterLoadPage(page, form);
        let annotation_count = bindings.FPDFPage_GetAnnotCount(page).max(0);
        for ai in 0..annotation_count {
            let annotation = bindings.FPDFPage_GetAnnot(page, ai);
            if annotation.is_null() {
                continue;
            }
            let field_type = bindings.FPDFAnnot_GetFormFieldType(form, annotation);
            if field_type < 0 {
                bindings.FPDFPage_CloseAnnot(annotation);
                continue;
            };
            let name = read_pdfium_utf16(|buffer, len| {
                bindings.FPDFAnnot_GetFormFieldName(form, annotation, buffer, len)
            });
            let value = read_pdfium_utf16(|buffer, len| {
                bindings.FPDFAnnot_GetFormFieldValue(form, annotation, buffer, len)
            });
            let flags = bindings.FPDFAnnot_GetFormFieldFlags(form, annotation);
            out.push(FormFieldDto {
                name,
                kind: form_field_kind(field_type).into(),
                value,
                page: pi,
                read_only: flags & 1 != 0,
            });
            bindings.FPDFPage_CloseAnnot(annotation);
        }
        bindings.FORM_OnBeforeClosePage(page, form);
        bindings.FPDF_ClosePage(page);
    }

    bindings.FPDFDOC_ExitFormFillEnvironment(form);
    state.doc(doc)?.form_fields = Some(out.clone());
    Ok(out)
}

fn form_field_kind(field_type: i32) -> &'static str {
    match field_type {
        1 => "button",
        2 => "checkbox",
        3 => "radio",
        4 => "combo",
        5 => "list",
        6 => "text",
        7 => "signature",
        _ => "other",
    }
}

const MAX_FORM_STRING_BYTES: usize = 16 * 1024 * 1024;

fn read_pdfium_utf16(
    mut read: impl FnMut(*mut FPDF_WCHAR, std::os::raw::c_ulong) -> std::os::raw::c_ulong,
) -> String {
    let bytes = read(std::ptr::null_mut(), 0) as usize;
    if bytes == 0 || bytes > MAX_FORM_STRING_BYTES {
        return String::new();
    }
    let mut units = vec![0_u16; bytes.div_ceil(2)];
    let written = read(
        units.as_mut_ptr(),
        (units.len() * std::mem::size_of::<u16>()) as std::os::raw::c_ulong,
    ) as usize;
    if written == 0 {
        return String::new();
    }
    units.truncate(written.div_ceil(2).min(units.len()));
    decode_pdfium_utf16(&units)
}

fn decode_pdfium_utf16(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_cancellation_only_applies_to_render_work() {
        let render = Work::Render {
            key: RenderKey {
                doc: 1,
                src: 0,
                rot: 0,
                scale_milli: 1_000,
                kind: RenderKind::Page,
                tile: None,
            },
            respond: Box::new(|_| {}),
        };
        assert!(is_generation_cancellable(&render));
        assert!(!is_generation_cancellable(&Work::PreviewRect {
            doc: 1,
            src: 0,
            x: None,
            y: None,
            respond: Box::new(|_| {}),
        }));
        assert!(!is_generation_cancellable(&Work::PageLinks {
            doc: 1,
            src: 0,
            respond: Box::new(|_| {}),
        }));
        assert!(!is_generation_cancellable(&Work::ResolveCitation {
            id: CitationIdDto::Doi("10.1000/test".into()),
            main_epoch: 0,
            respond: Box::new(|_| {}),
        }));
        assert!(!is_generation_cancellable(&Work::IndexNext {
            doc: 1,
            chain: 1
        }));
        assert!(!is_generation_cancellable(&Work::LibraryScanNext));
        assert!(!is_generation_cancellable(&Work::Close { doc: 1 }));
    }

    #[test]
    fn form_field_types_have_stable_ui_kinds() {
        assert_eq!(form_field_kind(1), "button");
        assert_eq!(form_field_kind(2), "checkbox");
        assert_eq!(form_field_kind(3), "radio");
        assert_eq!(form_field_kind(4), "combo");
        assert_eq!(form_field_kind(5), "list");
        assert_eq!(form_field_kind(6), "text");
        assert_eq!(form_field_kind(7), "signature");
        assert_eq!(form_field_kind(-1), "other");
    }

    #[test]
    fn pdfium_utf16_decoder_removes_the_terminal_nul() {
        assert_eq!(
            decode_pdfium_utf16(&['S' as u16, 'p' as u16, 'e' as u16, 'e' as u16, 'd' as u16, 0]),
            "Speed"
        );
    }
}

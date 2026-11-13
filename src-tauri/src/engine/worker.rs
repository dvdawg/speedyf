//! The engine thread. Sole owner of every PDFium handle in the process.
//! Raw FPDF_* handles never leave this thread; results cross back as owned
//! bytes/DTOs through responder callbacks.

use super::annots;
use super::bubbles;
use super::columns;
use super::glyphs;
use super::layout_text;
use super::links;
use super::ocr::{self, OcrLine};
use super::ocr_thread::{OcrDone, OcrRequest, OcrService, Refused};
use super::pdfium_init;
use super::preview::{self, CropInput, PREVIEW_SCALE_MILLI};
use super::queue::JobMeta;
use super::render;
use super::save;
use super::text;
use super::types::*;
use super::vision;
use super::{EngineShared, Work};
use crate::errors::{AppError, AppResult};
use crate::library::ocrtext::{self, OcrCache, PageProbe};
use crate::library::types::{PaperDto, PapernessDto};
use crate::library::{self, LibraryEntry};
use crate::search::TextLayoutData;
use pdfium_render::prelude::*;
use pdfium_render::prelude::{FPDF_DOCUMENT, FPDF_PAGE};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
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
    /// The library's OCR cache for this file, looked up the first time one
    /// of its pages needs OCR (`cached_page_ocr`); `Some(None)`: it has none.
    ocr_cache: Option<Option<OcrCache>>,
    /// Display equations found per page (`glyphs::equation_blocks`), keyed
    /// by page in the file, so hovering a page twice costs nothing.
    equation_blocks: HashMap<u32, Vec<glyphs::EquationBlockDto>>,
    /// Pages rendered for layout analysis (`vision`), a few at a time.
    rasters: HashMap<u32, std::sync::Arc<vision::PageRaster>>,
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
    /// Where pages go to be read (`ocr_thread`): Vision runs there, never
    /// on this thread.
    ocr: OcrService,
    /// Whether pages are probed for OCR at all: Vision exists here
    /// (`ocr::available`).
    ocr_enabled: bool,
    /// Documents with a page at the OCR thread. One at a time each.
    ocr_in_flight: HashSet<PathBuf>,
    /// The library scan's current file, waiting for a page's OCR: scanned
    /// again when it is back (`do_ocr_done`).
    scan_waiting: Option<library::ScanCandidate>,
    /// The library text pass, waiting for a page's OCR.
    text_waiting: Option<TextWaiting>,
}

impl<'a> WorkerState<'a> {
    fn new(
        bindings: &'a dyn PdfiumLibraryBindings,
        pdfium: &'a Pdfium,
        ocr: OcrService,
        ocr_enabled: bool,
    ) -> Self {
        WorkerState {
            bindings,
            pdfium,
            docs: HashMap::new(),
            next_id: 1,
            active_main: None,
            library_docs: Vec::new(),
            library_session_docs: Vec::new(),
            index_pending: Vec::new(),
            index_active: None,
            index_chain_seq: 0,
            ocr,
            ocr_enabled,
            ocr_in_flight: HashSet::new(),
            scan_waiting: None,
            text_waiting: None,
        }
    }

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
    let ocr = OcrService::new(
        Arc::new(|png: &[u8]| ocr::recognize_png(png, &[])),
        ocr_done_to_queue(Arc::downgrade(&shared)),
    );
    let mut state = WorkerState::new(pdfium.bindings(), pdfium, ocr, ocr::available());

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

/// What the OCR thread does with a reading once it is cached: queue it
/// for this thread, at idle, to resume whatever waited for it.
fn ocr_done_to_queue(shared: Weak<EngineShared>) -> super::ocr_thread::OnDone {
    Arc::new(move |done: OcrDone| {
        if let Some(shared) = shared.upgrade() {
            shared.queue.push(
                Priority::Idle,
                JobMeta {
                    doc: 0,
                    gen: shared.gens.current(0),
                    epoch: shared.cancels.current(0),
                },
                Work::OcrDone { done },
            );
        }
    })
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
        Work::PageBubbles { respond, .. } => respond(Err(err())),
        Work::PageGlyphs { respond, .. } => respond(Err(err())),
        Work::MathTags { respond, .. } => respond(Err(err())),
        Work::EquationBlocks { respond, .. } => respond(Err(err())),
        Work::ColumnLayout { respond, .. } => respond(Err(err())),
        Work::PreviewRect { respond, .. } => respond(Err(err())),
        Work::ResolveCitation { respond, .. } => respond(Err(err())),
        Work::LibraryPreview { respond, .. } => respond(Err(err())),
        Work::ClassifyDocument { respond, .. } => respond(Err(err())),
        Work::LibraryScanFile { respond, .. } => respond(Err(err())),
        Work::MatchRects { respond, .. } => respond(Err(err())),
        Work::Save { respond, .. } => respond(Err(err())),
        Work::BuildPrintPdf { respond, .. } => respond(Err(err())),
        Work::FormFields { respond, .. } => respond(Err(err())),
        Work::Outline { respond, .. } => respond(Err(err())),
        Work::FormalEnvs { respond, .. } => respond(Err(err())),
        Work::Figures { respond, .. } => respond(Err(err())),
        Work::Annotations { respond, .. } => respond(Err(err())),
        Work::ImageSize { respond, .. } => respond(Err(err())),
        Work::Close { .. }
        | Work::SetActiveDocument { .. }
        | Work::StartIndexing { .. }
        | Work::IndexNext { .. }
        | Work::LibraryScanNext
        | Work::LibraryWatchNext
        | Work::LibraryTextNext
        | Work::LibraryCitesNext
        | Work::OcrDone { .. } => {}
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
        Work::PageBubbles { doc, src, respond } => {
            respond(do_page_bubbles(state, shared, doc, src))
        }
        Work::PageGlyphs {
            doc,
            src,
            rects,
            grow,
            respond,
        } => respond(do_page_glyphs(state, doc, src, &rects, grow)),
        Work::MathTags { doc, src, respond } => respond(do_math_tags(state, doc, src)),
        Work::EquationBlocks { doc, src, respond } => respond(do_equation_blocks(state, doc, src)),
        Work::ColumnLayout { doc, src, respond } => {
            respond(do_column_layout(state, shared, doc, src))
        }
        Work::PreviewRect {
            doc,
            src,
            x,
            y,
            respond,
        } => respond(do_preview_rect(state, shared, doc, src, x, y)),
        Work::ClassifyDocument { doc, respond } => respond(do_classify_document(state, doc)),
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
        Work::LibraryPreview {
            path,
            main_epoch,
            respond,
        } => {
            // Same guard as a citation hover: a preview requested before the
            // foreground document changed must not open library PDFs now.
            if main_epoch == shared.main_epoch.load(Ordering::Acquire) {
                respond(do_library_preview(state, shared, &path));
            } else {
                respond(Ok(None));
            }
        }
        Work::LibraryScanFile { path, respond } => {
            respond(do_library_scan_file(state, shared, &path))
        }
        Work::StartIndexing { doc } => state.request_indexing(doc, shared),
        Work::IndexNext { doc, chain } => do_index_next(state, shared, meta, doc, chain),
        Work::LibraryScanNext => do_library_scan_next(state, shared),
        Work::LibraryWatchNext => do_library_watch_next(state, shared),
        Work::LibraryTextNext => do_library_text_next(state, shared),
        Work::LibraryCitesNext => do_library_cites_next(shared),
        Work::OcrDone { done } => do_ocr_done(state, shared, done),
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
        Work::Annotations { doc, respond } => respond(do_annotations(state, shared, doc)),
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

    // Opening a document has no effect on the citation/library subsystem -
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
            ocr_cache: None,
            equation_blocks: HashMap::new(),
            rasters: HashMap::new(),
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
        key.invert,
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

/// Put one page of an open document in its search index, as the library's
/// text pass reads a page: word gaps that are only geometry inserted as
/// spaces (`library::word_breaks`), so a LaTeX paper's run-together words
/// are found as words, with offsets still PDFium character indices; and a
/// page whose text layer is unusable indexed from the library's OCR of it,
/// when there is one (`cached_page_ocr`).
fn ensure_search_extracted(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<(u32, bool)> {
    if shared.search.lock().contains_page(doc, src) {
        return Ok((0, true));
    }
    let [w, h] = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    let glyphs = layout_text::extract_spacing_glyphs(state.bindings, page, w, h);
    let char_count = glyphs.raw.chars().count() as u32;
    let breaks = library::word_breaks(&glyphs.raw, &glyphs.boxes, &glyphs.upright);
    let ocr_lines = if state.ocr_enabled {
        cached_page_ocr(state, shared, doc, src, page, &glyphs.raw, &breaks, w * h)
    } else {
        None
    };
    let stored = match ocr_lines {
        Some(lines) => {
            let (text, geometry) = crate::search::ocr_page(&lines, w, h);
            shared
                .search
                .lock()
                .store_ocr_page(doc, src, &text, geometry)
        }
        None => shared
            .search
            .lock()
            .store_page_spaced(doc, src, &glyphs.raw, char_count, &breaks),
    };
    if stored {
        shared.metrics.pages_indexed.fetch_add(1, Ordering::Relaxed);
    }
    Ok((char_count, stored))
}

/// The library's OCR of an open document's page, when the page's text
/// layer needs it (`ocr::needs_ocr`, judged on the same text the text pass
/// judges) and the OCR cache has read it. Never reads a page itself: OCR
/// belongs to the idle library jobs, and an open document only uses what
/// they left.
#[allow(clippy::too_many_arguments)]
fn cached_page_ocr(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
    page: FPDF_PAGE,
    raw: &str,
    breaks: &[u32],
    area_pts: f32,
) -> Option<Vec<OcrLine>> {
    let text = crate::search::librarytext::normalized_spaced_text(raw, breaks);
    let (has_images, _) = render::image_coverage(state.bindings, page);
    if !ocr::needs_ocr(&text, area_pts, has_images) {
        return None;
    }
    let d = state.docs.get_mut(&doc)?;
    if d.ocr_cache.is_none() {
        d.ocr_cache = Some(open_document_ocr_cache(shared, Path::new(&d.path)));
    }
    let lines = d.ocr_cache.as_ref()?.as_ref()?.get(src)??.to_vec();
    // Vision finding nothing leaves the text layer, as in the sidecar.
    (!ocr::page_text(&lines).trim().is_empty()).then_some(lines)
}

/// The OCR cache the library keeps for the file at `path`, if it has one.
/// Keyed by the content fingerprint: the library's, or computed when the
/// file is not in the library.
fn open_document_ocr_cache(shared: &EngineShared, path: &Path) -> Option<OcrCache> {
    let (app_data_dir, entry) = {
        let library = shared.library.lock();
        (library.app_data_dir(), library.entry_for_path(path))
    };
    if !ocrtext::cache_path(&app_data_dir, path).exists() {
        return None;
    }
    let fingerprint = entry
        .and_then(|entry| entry.fingerprint)
        .or_else(|| library::content_fingerprint(path));
    Some(OcrCache::load(&app_data_dir, path, fingerprint.as_deref()))
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
    // The index wants the spacing glyphs, not these boxes (ink, not advance:
    // they open false gaps), so it reads the page once more.
    if !shared.search.lock().contains_page(doc, src) {
        let _ = ensure_search_extracted(state, shared, doc, src);
    }
    // Bubbles come off the same character stream, so they are found here
    // rather than on demand: by the time anyone asks, the page has already
    // been read and re-reading it would learn nothing new.
    let bubbles = bubbles::detect(&extracted.raw, &extracted.boxes, &extracted.sizes);
    Ok(shared.text_layouts.lock().insert(
        doc,
        src,
        extracted.runs,
        extracted.boxes,
        bubbles,
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

/// Every drawn bubble/checkbox on one page.
///
/// Reads straight out of the cached text layout, because that is where
/// detection already happened - the first request for a page pays for the
/// extraction it would have paid for on selection or search anyway.
fn do_page_bubbles(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<PageBubblesDto> {
    let layout = ensure_text_layout(state, shared, doc, src)?;
    Ok(PageBubblesDto {
        src,
        bubbles: layout.bubbles.as_ref().clone(),
    })
}

/// Every glyph and rule on a page, in display points from the top-left.
fn all_page_glyphs(
    state: &mut WorkerState,
    doc: DocId,
    src: u32,
) -> AppResult<(Vec<glyphs::GlyphDto>, [f32; 2])> {
    if src >= state.doc(doc)?.page_count {
        return Err(AppError::Malformed(format!("page {src} is out of range")));
    }
    let size = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    Ok((
        glyphs::extract_page_glyphs(state.bindings, page, size[0], size[1]),
        size,
    ))
}

/// Pages whose rasters a document keeps for layout analysis.
const RASTER_CACHE_PAGES: usize = 4;

/// The page rendered and binarized for layout analysis, cached.
fn page_raster(
    state: &mut WorkerState,
    doc: DocId,
    src: u32,
) -> AppResult<Option<std::sync::Arc<vision::PageRaster>>> {
    if let Some(r) = state.doc(doc)?.rasters.get(&src) {
        return Ok(Some(r.clone()));
    }
    let [w, h] = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    let Some(raster) = vision::render_raster(state.bindings, page, w, h) else {
        return Ok(None);
    };
    let raster = std::sync::Arc::new(raster);
    let cache = &mut state.doc(doc)?.rasters;
    if cache.len() >= RASTER_CACHE_PAGES {
        cache.clear();
    }
    cache.insert(src, raster.clone());
    Ok(Some(raster))
}

fn do_page_glyphs(
    state: &mut WorkerState,
    doc: DocId,
    src: u32,
    rects: &[[f32; 4]],
    grow: bool,
) -> AppResult<glyphs::GlyphLayoutDto> {
    let (all, _) = all_page_glyphs(state, doc, src)?;
    let raster = if rects.is_empty() {
        None
    } else {
        page_raster(state, doc, src)?
    };
    Ok(glyphs::glyph_layout(&all, raster.as_deref(), rects, grow))
}

fn do_math_tags(
    state: &mut WorkerState,
    doc: DocId,
    src: u32,
) -> AppResult<Vec<glyphs::MathTagDto>> {
    if src >= state.doc(doc)?.page_count {
        return Err(AppError::Malformed(format!("page {src} is out of range")));
    }
    let size = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    Ok(glyphs::page_math_tags(
        state.bindings,
        page,
        size[0],
        size[1],
    ))
}

/// Pages whose equations a document keeps; past this the cache starts over.
const EQUATION_CACHE_PAGES: usize = 256;

fn do_equation_blocks(
    state: &mut WorkerState,
    doc: DocId,
    src: u32,
) -> AppResult<Vec<glyphs::EquationBlockDto>> {
    if let Some(cached) = state.doc(doc)?.equation_blocks.get(&src) {
        return Ok(cached.clone());
    }
    let (all, [w, h]) = all_page_glyphs(state, doc, src)?;
    let raster = page_raster(state, doc, src)?;
    let blocks = glyphs::equation_blocks_with_raster(&all, raster.as_deref(), w, h);
    let cache = &mut state.doc(doc)?.equation_blocks;
    if cache.len() >= EQUATION_CACHE_PAGES {
        cache.clear();
    }
    cache.insert(src, blocks.clone());
    Ok(blocks)
}

/// A page's reading segments for column zoom: from the cached text layout
/// and the page's graphics, or from its pixels when it has no text layer.
/// Cheap enough to recompute; the viewer keeps the answer per page.
fn do_column_layout(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u32,
) -> AppResult<columns::ColumnLayoutDto> {
    if src >= state.doc(doc)?.page_count {
        return Err(AppError::Malformed(format!("page {src} is out of range")));
    }
    let layout = ensure_text_layout(state, shared, doc, src)?;
    let [w, h] = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    let content = columns::PageContent {
        page_w: w,
        page_h: h,
        text: layout
            .runs
            .iter()
            .map(|r| columns::Bx::from_up([r.x, r.y, r.w, r.h], h))
            .collect(),
        graphics: columns::page_graphics(state.bindings, page, w, h),
    };
    let (analysis, source) =
        columns::analyze_page(content, || page_raster(state, doc, src).ok().flatten());
    Ok(analysis.into_dto(src, h, source))
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
/// One document per job, then back to the queue - the same yielding shape as
/// `do_library_scan_next`, and the reason a library indexing pass does not
/// make the app feel slow: anything visible outranks `Idle`.
///
/// A document that cannot be read is marked done rather than retried. It will
/// not start being readable, and leaving it at the head of the queue would
/// stall every document behind it forever.
fn do_library_text_next(state: &mut WorkerState, shared: &EngineShared) {
    if state.text_waiting.is_some() {
        // A document waits for a page's OCR; its reading resumes the pass
        // (`do_ocr_done`), so a second chain would only collide with it.
        return;
    }
    let (app_data_dir, candidate) = {
        let library = shared.library.lock();
        (library.app_data_dir(), library.next_text_candidate())
    };
    let Some(entry) = candidate else {
        requeue_library_text(shared);
        return;
    };
    if entry.ocr_deferred && state.ocr_enabled {
        // Scanned on demand, when OCR was not allowed: read its first
        // pages again with OCR now, at idle, before its text.
        if rescan_with_ocr(state, shared, &entry) {
            state.text_waiting = Some(TextWaiting::Rescan);
            return;
        }
        requeue_library_text(shared);
        return;
    }
    let read = extract_library_text(state, &entry, &app_data_dir);
    finish_library_text(state, shared, &entry, &app_data_dir, read);
}

/// The text pass's step after reading a document: its sidecar and cites
/// written, or the pass left waiting for the OCR thread, or the document
/// left for the next job when its OCR budget is spent.
fn finish_library_text(
    state: &mut WorkerState,
    shared: &EngineShared,
    entry: &LibraryEntry,
    app_data_dir: &Path,
    read: LibraryText,
) {
    let (pages, ocr_pages) = match read {
        LibraryText::AwaitingOcr(job) => {
            // No requeue: the reading's `OcrDone` resumes this job.
            state.text_waiting = Some(TextWaiting::Extract(job));
            return;
        }
        LibraryText::Pending => {
            // Its OCR budget is spent; the next job resumes it, the
            // pages read so far cached.
            requeue_library_text(shared);
            return;
        }
        LibraryText::Unreadable => (None, Vec::new()),
        LibraryText::Read { pages, ocr_pages } => (Some(pages), ocr_pages),
    };
    // A job that waited for OCR may have outlived the bytes it read: the
    // file changed or left the library meanwhile, and its new entry is the
    // text pass's to read from scratch.
    let current = shared.library.lock().entry_for_path(&entry.path);
    if current
        .is_none_or(|current| (current.mtime_ms, current.size) != (entry.mtime_ms, entry.size))
    {
        requeue_library_text(shared);
        return;
    }
    if let Some(pages) = &pages {
        if let Err(error) = crate::search::librarytext::write_spaced_sidecar(
            app_data_dir,
            &entry.path,
            entry.mtime_ms,
            entry.size,
            pages,
            &ocr_pages,
        ) {
            log::warn!("cannot index {} for search: {error}", entry.path.display());
        }
    }
    // A reference list read from the old text describes text that is
    // gone: drop it, so the provider is handed the new text. Its stamp
    // alone would not say so - a re-extraction of the same bytes (after
    // an upgrade that changes the text) keeps the stamp.
    crate::search::librarytext::remove_refs(app_data_dir, &entry.path);
    // The cited identifiers come from the same text while it is in
    // hand, in the form the sidecar stores it so the backfill would read
    // the same ones. The reference list is the reference provider's to
    // read, from the sidecar, at idle. A document that yielded no text
    // cites nothing, finally - as for the sidecar, it will not start
    // being readable.
    let cites = match pages {
        Some(pages) => {
            let text = pages
                .iter()
                .map(|(_, text, breaks)| {
                    crate::search::librarytext::normalized_spaced_text(text, breaks)
                })
                .collect::<Vec<_>>()
                .join("\n");
            library::record_citations(entry, &text)
        }
        None => Vec::new(),
    };
    shared.refs_cache.lock().forget(&entry.path);
    shared.library.lock().finish_text(&entry.path, cites);
    requeue_library_text(shared);
}

/// A page the OCR thread has read, back on this thread. It is cached
/// already (the OCR thread writes through); here the library jobs that
/// waited for it go on: the scan's file is scanned again, the text pass's
/// document picks up where it stopped - its pages extracted, its budget
/// as spent - or a deferred rescan is tried again.
fn do_ocr_done(state: &mut WorkerState, shared: &EngineShared, done: OcrDone) {
    state.ocr_in_flight.remove(&done.path);
    if let Some(candidate) = state.scan_waiting.take() {
        scan_step(state, shared, Some(candidate));
    }
    match state.text_waiting.take() {
        Some(TextWaiting::Extract(mut job)) => {
            if job.entry.path == done.path {
                job.cache.insert(done.page, done.lines);
            }
            let (entry, app_data_dir) = (job.entry.clone(), job.app_data_dir.clone());
            let read = continue_text_job(state, job);
            finish_library_text(state, shared, &entry, &app_data_dir, read);
        }
        Some(TextWaiting::Rescan) => requeue_library_text(shared),
        None => {}
    }
}

/// Read one already-indexed document's cited identifiers back out of its
/// sidecar: the backfill for entries indexed before cites were extracted
/// with the text. No PDFium, so it is cheap; still one entry per job, so a
/// thousand of them never hold the thread against a render.
fn do_library_cites_next(shared: &EngineShared) {
    let (app_data_dir, candidate) = {
        let library = shared.library.lock();
        (library.app_data_dir(), library.next_cites_candidate())
    };
    if let Some(entry) = candidate {
        let cites = library::backfill_cites(&app_data_dir, &entry);
        shared.refs_cache.lock().forget(&entry.path);
        shared.library.lock().finish_cites(&entry.path, cites);
    }
    requeue_library_text(shared);
}

/// The idle job after a library text step: more text if any document still
/// lacks a sidecar, else the cites backfill if any sidecar is still unread
/// for them, else nothing. Text first: the backfill reads what it wrote.
fn requeue_library_text(shared: &EngineShared) {
    let (more_text, more_cites) = {
        let library = shared.library.lock();
        (library.has_text_work(), library.has_cites_work())
    };
    let work = if more_text {
        Work::LibraryTextNext
    } else if more_cites {
        Work::LibraryCitesNext
    } else {
        return;
    };
    shared.queue.push(
        Priority::Idle,
        JobMeta {
            doc: 0,
            gen: shared.gens.current(0),
            epoch: shared.cancels.current(0),
        },
        work,
    );
}

/// What the text pass got from one document.
enum LibraryText {
    /// PDFium cannot open it.
    Unreadable,
    /// Pages still wait for OCR: this job's budget ran out.
    Pending,
    /// A page is at the OCR thread; the job resumes when it is back.
    AwaitingOcr(Box<TextJob>),
    /// Every page's text, and which of them came from OCR.
    Read {
        pages: Vec<(u32, String, Vec<u32>)>,
        ocr_pages: Vec<u32>,
    },
}

/// The text pass, waiting for the OCR thread.
enum TextWaiting {
    /// A file scanned on demand, being scanned again with OCR.
    Rescan,
    /// A document's text, part of its OCR still to come.
    Extract(Box<TextJob>),
}

/// One document's text job between OCR readings: everything extracted,
/// the document still open, so a reading resumes it where it stopped
/// rather than reading every page again.
struct TextJob {
    entry: LibraryEntry,
    app_data_dir: PathBuf,
    doc: FPDF_DOCUMENT,
    pages: Vec<(u32, String, Vec<u32>)>,
    probes: Vec<PageProbe>,
    cache: OcrCache,
    /// Pages this job may still send to be read (`TEXT_OCR_BUDGET`).
    budget: usize,
}

/// Every page's raw character stream and the word breaks its geometry
/// implies (`library::word_breaks`). The stream stays raw because a hit's
/// offsets are PDFium character indices, which is how it is highlighted
/// once the document is open; the breaks become whitespace in the sidecar's
/// normalized text, so a LaTeX paper's words are searchable - and handed to
/// the cites pass and the reference provider - as words. Four FFI calls a
/// character (`layout_text::extract_spacing_glyphs`) where the raw stream
/// alone costs one, at idle priority, once per document; no font queries.
///
/// A page whose text layer is unusable (`ocr::needs_ocr`) gets Vision's
/// reading of it instead (`ocrtext`), at most `TEXT_OCR_BUDGET` pages read
/// per job and every reading cached by fingerprint, so a long scan takes
/// a few jobs and a rescan reads nothing twice. The reading happens on the
/// OCR thread (`continue_text_job`). A document whose first pages all have
/// good text and that is not a scan is never probed past them, nor
/// rendered.
fn extract_library_text(
    state: &mut WorkerState,
    entry: &LibraryEntry,
    app_data_dir: &Path,
) -> LibraryText {
    let Some(path) = entry.path.to_str() else {
        return LibraryText::Unreadable;
    };
    let Ok(doc) = render::open_document(state.bindings, path, None) else {
        return LibraryText::Unreadable;
    };
    let b = state.bindings;
    let count = b.FPDF_GetPageCount(doc).max(0);
    let mut pages = Vec::new();
    // (page, area, has images, image coverage) for every page, to probe
    // with once it is known whether this document is worth OCR.
    let mut layout = Vec::new();
    for index in 0..count {
        let page = b.FPDF_LoadPage(doc, index);
        if page.is_null() {
            continue;
        }
        let [w, h] = render::page_display_size(b, doc, index).unwrap_or([612.0, 792.0]);
        let glyphs = layout_text::extract_spacing_glyphs(b, page, w, h);
        let (has_images, coverage) = if state.ocr_enabled {
            render::image_coverage(b, page)
        } else {
            (false, 0.0)
        };
        b.FPDF_ClosePage(page);
        layout.push((index as u32, w * h, has_images, coverage));
        if !glyphs.raw.trim().is_empty() {
            let breaks = library::word_breaks(&glyphs.raw, &glyphs.boxes, &glyphs.upright);
            pages.push((index as u32, glyphs.raw, breaks));
        }
    }
    let probe = |&(index, area_pts, has_images, image_coverage): &(u32, f32, bool, f32)| {
        let text = pages
            .iter()
            .find(|(page, _, _)| *page == index)
            .map(|(_, raw, breaks)| crate::search::librarytext::normalized_spaced_text(raw, breaks))
            .unwrap_or_default();
        PageProbe {
            index,
            text,
            area_pts,
            has_images,
            image_coverage,
        }
    };
    let first: Vec<PageProbe> = layout
        .iter()
        .take(ocrtext::PROBE_PAGES as usize)
        .map(probe)
        .collect();
    if state.ocr_enabled && ocrtext::wants_ocr(&first) {
        let probes: Vec<PageProbe> = first
            .into_iter()
            .chain(layout.iter().skip(ocrtext::PROBE_PAGES as usize).map(probe))
            .collect();
        let cache = OcrCache::load(app_data_dir, &entry.path, entry.fingerprint.as_deref());
        let job = TextJob {
            entry: entry.clone(),
            app_data_dir: app_data_dir.to_path_buf(),
            doc,
            pages,
            probes,
            cache,
            budget: ocrtext::TEXT_OCR_BUDGET,
        };
        return continue_text_job(state, Box::new(job));
    }
    b.FPDF_CloseDocument(doc);
    LibraryText::Read {
        pages,
        ocr_pages: Vec::new(),
    }
}

/// The OCR half of a text job, from wherever it stopped: the first page
/// that needs OCR and is not cached goes to the OCR thread, and the job
/// waits for it - one page in flight per document, the budget spent per
/// page sent. With every page it needs cached, the OCR text takes the text
/// layer's place (`ocrtext::ocr_pages`, `ocrtext::substitute`) exactly as
/// it did when the job read them itself.
fn continue_text_job(state: &mut WorkerState, mut job: Box<TextJob>) -> LibraryText {
    let b = state.bindings;
    let mut pending = false;
    let wanted: Vec<u32> = job
        .probes
        .iter()
        .filter(|probe| probe.needs_ocr())
        .map(|probe| probe.index)
        .collect();
    for index in wanted {
        if job.cache.get(index).is_some() {
            continue;
        }
        if job.budget == 0 {
            pending = true;
            break;
        }
        let mut ask = OcrAsk {
            service: &mut state.ocr,
            in_flight: &mut state.ocr_in_flight,
            path: &job.entry.path,
            fingerprint: job.entry.fingerprint.as_deref(),
            app_data_dir: &job.app_data_dir,
        };
        match ask.page(b, job.doc, index) {
            Requested::Sent => {
                job.budget -= 1;
                return LibraryText::AwaitingOcr(job);
            }
            Requested::Wait => return LibraryText::AwaitingOcr(job),
            Requested::Failed => {
                job.budget -= 1;
                job.cache.insert(index, None);
                job.cache.save(&job.app_data_dir, &job.entry.path);
            }
        }
    }
    job.cache.save(&job.app_data_dir, &job.entry.path);
    b.FPDF_CloseDocument(job.doc);
    if pending {
        return LibraryText::Pending;
    }
    // Every page that needs OCR is cached: nothing is read here.
    let pass = ocrtext::ocr_pages(&job.probes, &mut job.cache, 0, &mut |_| None);
    let TextJob { mut pages, .. } = *job;
    let ocr_pages = ocrtext::substitute(&mut pages, &pass);
    LibraryText::Read { pages, ocr_pages }
}

/// What became of a request to read a page.
enum Requested {
    /// At the OCR thread now.
    Sent,
    /// Not sent: this document already has a page there, or the channel is
    /// full. Either way a reading is coming back to wake the caller.
    Wait,
    /// It cannot be read: the page does not render, or there is no OCR
    /// thread. Cached as a failed reading, as Vision failing on it is.
    Failed,
}

/// Sending one document's pages to the OCR thread.
struct OcrAsk<'a> {
    service: &'a mut OcrService,
    in_flight: &'a mut HashSet<PathBuf>,
    path: &'a Path,
    fingerprint: Option<&'a str>,
    app_data_dir: &'a Path,
}

impl OcrAsk<'_> {
    /// Render page `index` here - PDFium stays on this thread - and hand
    /// the image to the OCR thread, unless this document already has a page
    /// there.
    fn page(&mut self, b: &dyn PdfiumLibraryBindings, doc: FPDF_DOCUMENT, index: u32) -> Requested {
        if self.in_flight.contains(self.path) {
            return Requested::Wait;
        }
        let Some(png) = render_for_ocr(b, doc, index) else {
            return Requested::Failed;
        };
        let request = OcrRequest {
            path: self.path.to_path_buf(),
            fingerprint: self.fingerprint.map(str::to_owned),
            app_data_dir: self.app_data_dir.to_path_buf(),
            page: index,
            png,
        };
        match self.service.submit(request) {
            Ok(()) => {
                self.in_flight.insert(self.path.to_path_buf());
                Requested::Sent
            }
            Err(Refused::Busy) => Requested::Wait,
            Err(Refused::Unavailable) => Requested::Failed,
        }
    }
}

/// One page rendered at about 2x for Vision, as PNG.
fn render_for_ocr(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    index: u32,
) -> Option<Vec<u8>> {
    /// 2x, unless that puts the long side past this many pixels.
    const MAX_SIDE_PX: f32 = 3000.0;
    let size = render::page_display_size(b, doc, index as i32)?;
    let scale = 2f32.min(MAX_SIDE_PX / size[0].max(size[1]).max(1.0));
    let page = b.FPDF_LoadPage(doc, index as i32);
    if page.is_null() {
        return None;
    }
    let rendered = render::render_page(b, page, size, scale, 0, None, false, || false);
    b.FPDF_ClosePage(page);
    Some(rendered.ok()?.png)
}

/// Scan a file again with OCR allowed, from the idle text pass: it was
/// scanned on demand, when OCR was not (`LibraryEntry::ocr_deferred`).
/// True when a page went to the OCR thread instead: the rescan is tried
/// again once it is back.
fn rescan_with_ocr(state: &mut WorkerState, shared: &EngineShared, entry: &LibraryEntry) -> bool {
    let Some((mtime_ms, size)) = library::file_stamp(&entry.path) else {
        // Gone or unreadable: nothing to read it again from.
        let mut cleared = entry.clone();
        cleared.ocr_deferred = false;
        shared.library.lock().upsert_entry(cleared);
        return false;
    };
    let candidate = library::ScanCandidate {
        path: entry.path.clone(),
        mtime_ms,
        size,
        epoch: 0,
        unchanged: false,
    };
    let app_data_dir = shared.library.lock().app_data_dir();
    let (scanned, awaiting) = scan_library_candidate(state, &candidate, &app_data_dir, true);
    if awaiting {
        return true;
    }
    shared.library.lock().upsert_entry(scanned.clone());
    shared.reconcile_library_entry(&scanned);
    false
}

/// The Info dictionary fields a paper record uses, and the ones the
/// paperness classifier reads. Read through the raw bindings in the
/// two-call pattern (length, then UTF-16LE text) so the document handle
/// opened for the first page serves both.
struct InfoDictionary {
    title: Option<String>,
    /// The Title as written, kept when the filter rejects it as a title:
    /// an old publisher scan puts the article's PII there and nowhere else.
    raw_title: Option<String>,
    author: Option<String>,
    year: Option<i32>,
    subject: Option<String>,
    producer: Option<String>,
    creator: Option<String>,
}

impl InfoDictionary {
    fn classifier_fields(&self) -> library::paperness::InfoFields<'_> {
        library::paperness::InfoFields {
            subject: self.subject.as_deref(),
            producer: self.producer.as_deref(),
            creator: self.creator.as_deref(),
        }
    }
}

fn read_meta_text(b: &dyn PdfiumLibraryBindings, doc: FPDF_DOCUMENT, tag: &str) -> Option<String> {
    let text =
        read_pdfium_utf16(|buffer, len| b.FPDF_GetMetaText(doc, tag, buffer.cast::<c_void>(), len));
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// What the open document's first pages and Info dictionary say about it
/// being a library item. Read from the document itself rather than the
/// library: the question only arises for a file the library does not have.
fn do_classify_document(state: &mut WorkerState, doc: DocId) -> AppResult<PapernessDto> {
    let raw = state.doc(doc)?.raw;
    let b = state.bindings;
    let info = read_info_dictionary(b, raw);
    // A page that cannot be read classifies as unknown, not as an error: the
    // panel's answer to "unknown" is the quiet one.
    let (text, _) = classification_text(b, raw, &info.classifier_fields(), false, None);
    Ok(library::paperness::classify(
        &text,
        &info.classifier_fields(),
    ))
}

/// How many pages after the first the classifier may read when the first
/// says too little: a thesis's title page is behind its cover, a book's
/// copyright page behind its half-title.
const EXTRA_CLASSIFY_PAGES: u32 = 3;

/// One page as the scan reads it: its repaired text, its printed title
/// and authors when asked for, and what OCR would need to know about it.
struct PageRead {
    text: String,
    first_page: Option<library::firstpage::FirstPage>,
    probe: PageProbe,
    size: [f32; 2],
}

/// One page's text with its word spaces repaired, uncapped; empty for a
/// page that cannot be loaded. Character geometry, not the raw character
/// stream alone: many PDFs - LaTeX output above all - encode word gaps as
/// position rather than as space characters, so the raw stream yields
/// titles like "PrivateandpolynomialtimealgorithmsforlearningGaussians".
/// `library::text_from_page` puts the spaces back from each glyph's loose
/// box, leaving rotated glyphs (the arXiv stamp) whole on their own line.
/// Same extraction as `page_text_and_title`, which keeps only enough for a
/// hover; a publisher's footer is at the bottom of the page.
///
/// With `layout`, the page's typeset lines are also read, while it is
/// open, for the title and authors printed on it (`library::firstpage`).
fn repaired_page_text(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    index: u32,
    layout: bool,
    probe_images: bool,
) -> PageRead {
    let page = b.FPDF_LoadPage(raw, index as i32);
    if page.is_null() {
        return PageRead {
            text: String::new(),
            first_page: None,
            probe: PageProbe {
                index,
                ..PageProbe::default()
            },
            size: [612.0, 792.0],
        };
    }
    let [w, h] = render::page_display_size(b, raw, index as i32).unwrap_or([612.0, 792.0]);
    let glyphs = layout_text::extract_spacing_glyphs(b, page, w, h);
    let first_page = layout.then(|| {
        let lines = layout_text::extract_lines(b, page, w, h);
        library::firstpage::extract(&lines, w, h)
    });
    let (has_images, image_coverage) = if probe_images {
        render::image_coverage(b, page)
    } else {
        (false, 0.0)
    };
    b.FPDF_ClosePage(page);
    let text = library::text_from_page(&glyphs.raw, &glyphs.boxes, &glyphs.upright);
    PageRead {
        probe: PageProbe {
            index,
            text: text.clone(),
            area_pts: w * h,
            has_images,
            image_coverage,
        },
        text,
        first_page,
        size: [w, h],
    }
}

/// OCR as the library scan uses it: for the first pages it reads, from the
/// cache or from the OCR thread. The OCR thread only with `ask` - the idle
/// scan, which then waits for the reading and scans the file again; a scan
/// on demand reads the cache alone and records what it went without.
struct ScanOcr<'a> {
    cache: OcrCache,
    /// Where a page not cached goes to be read; `None` on demand.
    ask: Option<OcrAsk<'a>>,
    /// The file is worth OCR (`ocrtext::wants_ocr`, over the pages read).
    wanted: bool,
    /// OCR was wanted and not allowed.
    deferred: bool,
    /// A page went to the OCR thread (or waits to): this reading of the
    /// file is incomplete, and is done again when the page is back.
    awaiting: bool,
}

impl ScanOcr<'_> {
    fn lines(
        &mut self,
        b: &dyn PdfiumLibraryBindings,
        raw: FPDF_DOCUMENT,
        index: u32,
    ) -> Option<Vec<OcrLine>> {
        if let Some(cached) = self.cache.get(index) {
            return cached.map(<[OcrLine]>::to_vec);
        }
        let Some(ask) = self.ask.as_mut() else {
            self.deferred = true;
            return None;
        };
        match ask.page(b, raw, index) {
            Requested::Sent | Requested::Wait => self.awaiting = true,
            Requested::Failed => {
                // Saved now, before a later page's reading is written
                // through by the OCR thread.
                self.cache.insert(index, None);
                if let Some(ask) = &self.ask {
                    self.cache.save(ask.app_data_dir, ask.path);
                }
            }
        }
        None
    }

    /// Put OCR in for what the text layer could not give: the text of a
    /// page whose layer is unusable, and the printed title and authors of a
    /// scanned first page. A scan's capture layer often has everything but
    /// the title, which was set too large or too decorated for the capture
    /// tool and stays a picture - the JCSS 1988 scan's layer reads "hh"
    /// where the first title line is, and its layout reader then takes the
    /// second line for the whole title, confidently.
    fn apply(&mut self, b: &dyn PdfiumLibraryBindings, raw: FPDF_DOCUMENT, page: &mut PageRead) {
        let needs = page.probe.needs_ocr();
        let scan = page.probe.index == 0 && page.probe.is_scan();
        self.wanted |= needs || scan;
        let for_title = page.first_page.is_some() && scan;
        if !needs && !for_title {
            return;
        }
        let Some(lines) = self.lines(b, raw, page.probe.index) else {
            return;
        };
        if needs {
            let text = ocr::page_text(&lines);
            if !text.trim().is_empty() {
                page.text = text;
            }
        }
        if let Some(current) = &page.first_page {
            let [w, h] = page.size;
            let read = library::firstpage::extract(&ocrtext::text_lines(&lines, w, h), w, h);
            if ocr_reading_is_better(current, &read, needs) {
                page.first_page = Some(read);
            }
        }
    }
}

/// Whether OCR's reading of a first page should replace the text layer's:
/// it found a title, and the layer's is unusable, absent or unsure - or
/// both are sure and OCR's title holds the layer's whole and more (the
/// part the capture tool left a picture).
fn ocr_reading_is_better(
    layer: &library::firstpage::FirstPage,
    ocr: &library::firstpage::FirstPage,
    layer_unusable: bool,
) -> bool {
    use library::firstpage::Confidence;
    let Some(ocr_title) = &ocr.title else {
        return false;
    };
    let sure = ocr.title_confidence == Confidence::High;
    let Some(layer_title) = layer.title.as_deref() else {
        return true;
    };
    if layer_unusable || layer.title_confidence != Confidence::High {
        return sure;
    }
    let key = |title: &str| {
        title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let (layer_key, ocr_key) = (key(layer_title), key(ocr_title));
    sure && !layer_key.is_empty() && ocr_key.len() > layer_key.len() && ocr_key.contains(&layer_key)
}

/// The text the classifier reads: the first page and, when that says
/// little - a sparse cover, or nothing the classifier recognizes - the
/// next `EXTRA_CLASSIFY_PAGES` pages after it, joined in order. The scan
/// reads its identifiers and ISBN from the same text, so a book's ISBN is
/// found on its copyright page. This costs the character boxes for one
/// page per document, and four for the few that need them.
///
/// With `layout`, also the first page's printed title and authors. With
/// `ocr`, pages whose text layer is unusable are read by OCR instead
/// (`ScanOcr::apply`).
fn classification_text(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    info: &library::paperness::InfoFields,
    layout: bool,
    mut ocr: Option<&mut ScanOcr>,
) -> (String, Option<library::firstpage::FirstPage>) {
    let probe_images = ocr.is_some();
    let mut first = repaired_page_text(b, raw, 0, layout, probe_images);
    if let Some(ocr) = ocr.as_deref_mut() {
        ocr.apply(b, raw, &mut first);
    }
    let mut text = first.text;
    if library::paperness::needs_more_pages(&text, info) {
        let count = b.FPDF_GetPageCount(raw).max(0) as u32;
        for index in 1..count.min(1 + EXTRA_CLASSIFY_PAGES) {
            let mut page = repaired_page_text(b, raw, index, false, probe_images);
            if let Some(ocr) = ocr.as_deref_mut() {
                ocr.apply(b, raw, &mut page);
            }
            text.push('\n');
            text.push_str(&page.text);
        }
    }
    (text, first.first_page)
}

fn read_info_dictionary(b: &dyn PdfiumLibraryBindings, doc: FPDF_DOCUMENT) -> InfoDictionary {
    let raw_title = read_meta_text(b, doc, "Title");
    let title = raw_title
        .clone()
        .filter(|title| library::plausible_info_title(title));
    // CreationDate is the paper's year only in a PDF the publisher made,
    // and a publisher's PDF carries a real Title. A scan of a 1988 paper has
    // no usable Title and a CreationDate of whenever it was scanned, which
    // would otherwise outrank the first page and turn up in the cite key.
    let year = title
        .is_some()
        .then(|| read_meta_text(b, doc, "CreationDate"))
        .flatten()
        .and_then(|date| library::year_from_pdf_date(&date));
    InfoDictionary {
        title,
        raw_title,
        author: read_meta_text(b, doc, "Author"),
        year,
        subject: read_meta_text(b, doc, "Subject"),
        producer: read_meta_text(b, doc, "Producer"),
        creator: read_meta_text(b, doc, "Creator"),
    }
}

/// Everything the scan learns about one file: identifiers from the file
/// name and the first page (or the first few, when the first says little),
/// a DOI derived from an Elsevier PII when there is no other, the arXiv
/// stamp's category, the first-page title, the Info dictionary, the XMP
/// packet, an ISBN, whether it looks like a paper, a thesis or a book at
/// all, and the content fingerprint. The file name used to short-circuit the whole
/// read; now every file is opened once, because a paper record wants a
/// title and an author for `arXiv-2401.12345.pdf` as much as for any other.
///
/// A file PDFium cannot open is still fingerprinted and still searched for
/// an XMP packet (plain bytes, no parser), and classified from nothing:
/// `Unknown`, which puts it among the other PDFs until the user says
/// otherwise.
///
/// With `ocr_allowed` (the idle scan), a page whose text layer is unusable
/// and a scanned first page's title are read by OCR (`ScanOcr`); without
/// it (a scan on demand) only from the OCR cache, and the entry is marked
/// for the idle text pass to scan again (`ocr_deferred`).
///
/// The second value says a page went to the OCR thread: the entry is not
/// final, and the caller scans the file again once the page is back.
fn scan_library_candidate(
    state: &mut WorkerState,
    candidate: &library::ScanCandidate,
    app_data_dir: &Path,
    ocr_allowed: bool,
) -> (LibraryEntry, bool) {
    if library::is_dataless(&candidate.path) {
        // A cloud placeholder: any read downloads it. Its name and stamp
        // only; without a fingerprint the next scan reads it properly once
        // it has been downloaded.
        let entry = library::entry_from_filename(candidate)
            .unwrap_or_else(|| library::entry_from_first_page(candidate, ""));
        return (entry, false);
    }
    let fingerprint = library::content_fingerprint(&candidate.path);
    let named = library::entry_from_filename(candidate);
    let xmp = library::xmp::read_xmp(&candidate.path);
    let finish = |mut entry: LibraryEntry, info: Option<InfoDictionary>, text: &str| {
        // The file name is the more deliberate statement of identity; the
        // first page fills in what it does not say. The source goes with
        // the value, so the record knows the name said it.
        if let Some(named) = &named {
            if named.doi.is_some() {
                entry.doi = named.doi.clone();
                entry.doi_source = named.doi_source;
            }
            if named.arxiv.is_some() {
                entry.arxiv = named.arxiv.clone();
                entry.arxiv_source = named.arxiv_source;
            }
        }
        entry.fingerprint = fingerprint.clone();
        let paperness = library::paperness::classify(
            text,
            &info
                .as_ref()
                .map(InfoDictionary::classifier_fields)
                .unwrap_or_default(),
        );
        entry.paper_verdict = Some(paperness.verdict);
        entry.paper_reason = Some(paperness.reason);
        entry.paper_kind = paperness.kind;
        entry.xmp = xmp.clone();
        let raw_title = info.as_ref().and_then(|info| info.raw_title.clone());
        if let Some(info) = info {
            entry.info_title = info.title;
            entry.info_author = info.author;
            entry.info_year = info.year;
        }
        library::derive_from_identifiers(&mut entry, text, raw_title.as_deref());
        entry
    };
    let Some(path) = candidate.path.to_str() else {
        let entry = finish(library::entry_from_first_page(candidate, ""), None, "");
        return (entry, false);
    };
    let b = state.bindings;
    let raw = match render::open_document(b, path, None) {
        Ok(raw) => raw,
        Err(error) => {
            log::warn!(
                "cannot scan library PDF {}: {error}",
                candidate.path.display()
            );
            let entry = finish(library::entry_from_first_page(candidate, ""), None, "");
            return (entry, false);
        }
    };
    let info = read_info_dictionary(b, raw);
    let mut scan_ocr = state.ocr_enabled.then(|| ScanOcr {
        cache: OcrCache::load(app_data_dir, &candidate.path, fingerprint.as_deref()),
        ask: ocr_allowed.then(|| OcrAsk {
            service: &mut state.ocr,
            in_flight: &mut state.ocr_in_flight,
            path: &candidate.path,
            fingerprint: fingerprint.as_deref(),
            app_data_dir,
        }),
        wanted: false,
        deferred: false,
        awaiting: false,
    });
    let (text, first_page) =
        classification_text(b, raw, &info.classifier_fields(), true, scan_ocr.as_mut());
    b.FPDF_CloseDocument(raw);
    let mut entry = finish(
        library::entry_from_page(candidate, &text, first_page.as_ref()),
        Some(info),
        &text,
    );
    let mut awaiting = false;
    if let Some(mut scan_ocr) = scan_ocr {
        scan_ocr.cache.save(app_data_dir, &candidate.path);
        entry.ocr_wanted = scan_ocr.wanted;
        entry.ocr_deferred = scan_ocr.deferred;
        awaiting = scan_ocr.awaiting;
    }
    (entry, awaiting)
}

fn do_library_scan_next(state: &mut WorkerState, shared: &EngineShared) {
    if state.scan_waiting.is_some() {
        // A file waits for a page's OCR; its reading resumes the scan.
        return;
    }
    let candidate = shared.library.lock().next_candidate();
    scan_step(state, shared, candidate);
}

/// Scan one candidate (if any) and queue the next - or, when a page of it
/// went to the OCR thread, park it until the reading is back
/// (`do_ocr_done`), which scans it again with the page cached.
fn scan_step(
    state: &mut WorkerState,
    shared: &EngineShared,
    candidate: Option<library::ScanCandidate>,
) {
    if let Some(candidate) = candidate {
        let app_data_dir = shared.library.lock().app_data_dir();
        let scanned = if candidate.unchanged {
            None
        } else {
            let (entry, awaiting) = scan_library_candidate(state, &candidate, &app_data_dir, true);
            if awaiting {
                state.scan_waiting = Some(candidate);
                return;
            }
            Some(entry)
        };
        // An unchanged file still gets reconciled, from its stored entry:
        // a library.json that starts empty would otherwise never get papers
        // for the files indexed before it existed.
        let entry = scanned
            .clone()
            .or_else(|| shared.library.lock().entry_for_path(&candidate.path));
        if shared.library.lock().finish_candidate(&candidate, scanned) {
            if let Some(entry) = entry {
                shared.reconcile_library_entry(&entry);
            }
        }
    }
    if shared.library.lock().has_scan_work() {
        shared.queue.push(
            Priority::Idle,
            JobMeta {
                doc: 0,
                gen: shared.gens.current(0),
                epoch: shared.cancels.current(0),
            },
            Work::LibraryScanNext,
        );
        return;
    }
    // The index is complete: settle the paper records against it.
    shared.finish_library_scan();
    // The metadata pass is what discovers files, so the text pass (and the
    // cites backfill behind it) starts once it has nothing left to find.
    requeue_library_text(shared);
}

/// One step of the watcher's queue (`library::watch`): paths that went are
/// dropped from the index and their paper files marked missing now; then
/// one settled file is scanned - at idle, without OCR, copying nothing -
/// and reconciled, which reattaches a renamed file by its fingerprint.
/// Saves (and so `library:changed`) come at most once a second while a
/// burst drains, and once more when it is done.
fn do_library_watch_next(state: &mut WorkerState, shared: &EngineShared) {
    let gone = shared.library.lock().take_gone();
    if !gone.is_empty() {
        let flushed = {
            let mut papers = shared.papers.lock();
            papers.mark_paths_missing(&gone);
            papers.flush(false)
        };
        shared.after_flush(flushed);
    }
    let candidate = shared.library.lock().next_watched();
    if let Some(candidate) = candidate.filter(|candidate| !candidate.unchanged) {
        let app_data_dir = shared.library.lock().app_data_dir();
        let (entry, _) = scan_library_candidate(state, &candidate, &app_data_dir, false);
        shared.library.lock().record_watched(entry.clone());
        shared.reconcile_library_entry(&entry);
    }
    if shared.library.lock().finish_watch_step() {
        shared.queue.push(
            Priority::Idle,
            JobMeta {
                doc: 0,
                gen: shared.gens.current(0),
                epoch: shared.cancels.current(0),
            },
            Work::LibraryWatchNext,
        );
        return;
    }
    let flushed = shared.papers.lock().flush(true);
    shared.after_flush(flushed);
    // New files' text, cites and OCR follow through the usual idle chain.
    requeue_library_text(shared);
}

/// Scan one file outside the idle scan - just copied into the inbox - put
/// it in the index and the paper records, and answer with its paper.
fn do_library_scan_file(
    state: &mut WorkerState,
    shared: &EngineShared,
    path: &Path,
) -> AppResult<PaperDto> {
    let (mtime_ms, size) = library::file_stamp(path)
        .ok_or_else(|| AppError::Io(format!("cannot read {}", path.display())))?;
    let candidate = library::ScanCandidate {
        path: path.to_path_buf(),
        mtime_ms,
        size,
        epoch: 0,
        unchanged: false,
    };
    // On demand - the user is waiting for this paper - so no OCR: the
    // idle text pass reads it again with OCR if it wants it.
    let app_data_dir = shared.library.lock().app_data_dir();
    let (entry, _) = scan_library_candidate(state, &candidate, &app_data_dir, false);
    shared.library.lock().upsert_entry(entry.clone());
    let paper = {
        let mut papers = shared.papers.lock();
        papers.reconcile_file(&entry);
        papers.flush(true)?;
        papers.paper_for_path(path).ok_or_else(|| {
            AppError::Internal(format!(
                "no paper record for {} after scanning it",
                path.display()
            ))
        })?
    };
    shared.notify_library_changed();
    requeue_library_text(shared);
    Ok(paper)
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
            ocr_cache: None,
            equation_blocks: HashMap::new(),
            rasters: HashMap::new(),
        },
    );
    state.library_docs.push(id);
    state.library_session_docs.push(id);
    Ok(id)
}

/// First-page text of a document, spacing repaired, with its first plausible
/// line as a title.
///
/// Extracted directly rather than through `ensure_text_layout`, because that
/// caches runs and boxes but not the character stream they index, and the two
/// are only meaningful together - which is the same coupling that produced
/// run-derived titles with no spaces in them.
fn page_text_and_title(state: &mut WorkerState, doc: DocId) -> AppResult<(String, Option<String>)> {
    let [w_pt, h_pt] = state.display_size(doc, 0)?;
    let page = state.page_handle(doc, 0)?;
    let glyphs = layout_text::extract_spacing_glyphs(state.bindings, page, w_pt, h_pt);
    let repaired = library::text_from_page(&glyphs.raw, &glyphs.boxes, &glyphs.upright);

    let text = repaired
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect();
    // The first line long enough to be a title rather than a running header,
    // a page number or a journal stamp. Same threshold the previous version
    // used, applied to lines that now have their spaces.
    let title = repaired
        .lines()
        .map(str::trim)
        .find(|line| line.chars().count() > 12)
        .map(|line| line.chars().take(300).collect());
    Ok((text, title))
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
    let doc_id = open_library_document(state, shared, &path)?;
    resolved_library_document(state, shared, doc_id, &path).map(Some)
}

/// A library file's first page for the Projects screen. The same shape a
/// citation hover gets, through the same document pool.
fn do_library_preview(
    state: &mut WorkerState,
    shared: &EngineShared,
    path: &Path,
) -> AppResult<Option<ResolvedCitationDto>> {
    // Re-checked here, not only in the command: the roots can change while
    // the job waits in the queue.
    let Some(path) = shared.library.lock().contains(path) else {
        return Ok(None);
    };
    let doc_id = open_library_document(state, shared, &path)?;
    resolved_library_document(state, shared, doc_id, &path).map(Some)
}

/// The preview spec and first-page text of an open library document, as a
/// citation resolve answers it.
fn resolved_library_document(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc_id: DocId,
    path: &Path,
) -> AppResult<ResolvedCitationDto> {
    let metadata = shared.library.lock().entry_for_path(path);
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
    // Read through the same repair the library indexer uses, not off the raw
    // runs. PDFium's runs carry no spaces between words in a great many
    // documents, so a title taken from them arrives as
    // "ATheoryofSaddleEscapeinDeepNonlinear"; `text_from_page` puts the spaces
    // back from character geometry. This fallback is only reached when the
    // library has no title for the file, and it was reached often enough to be
    // what the References panel actually displayed.
    let (text, extracted_title) = page_text_and_title(state, doc_id).unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let cite_key = shared.papers.lock().cite_key_for_path(path);
    Ok(ResolvedCitationDto {
        doc_id,
        title: metadata.and_then(|entry| entry.title).or(extracted_title),
        page_count,
        file_name,
        path: path.to_string_lossy().into_owned(),
        cite_key,
        preview: PreviewSpecDto {
            doc_id,
            src: 0,
            tile,
            scale_milli: PREVIEW_SCALE_MILLI,
            text,
        },
    })
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
    // A page indexed from OCR: the hit's offsets index the OCR text, and
    // its place on the page is the OCR lines' boxes.
    let ocr = shared.search.lock().ocr_geometry(doc, src);
    if let Some(geometry) = ocr {
        return Ok(geometry.match_rects(start, len));
    }
    let layout = ensure_text_layout(state, shared, doc, src)?;
    Ok(text::merge_match_rects(&layout.boxes, start, len))
}

/// Materialize the document as it currently stands into a throwaway file, for
/// printing.
///
/// The same page assembly, rotation, annotation and form work `do_save`
/// performs - this is what puts unsaved edits on paper - but without the
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
                                ocr_cache: None,
                                equation_blocks: HashMap::new(),
                                rasters: HashMap::new(),
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
/// destination carries one - which lets the table of contents land on the
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

/// Every annotation SpeedyF owns, read out of the file on disk - and hidden in
/// the render document on the way out.
///
/// The two halves are one job on purpose. Renders include annotations
/// (`FPDF_ANNOT`), so once the editor holds an editable copy the page image
/// would draw it a second time underneath. Hiding exactly the annotations that
/// were just handed over keeps those two facts from ever disagreeing.
///
/// The hidden flag is set only in the in-memory render document. Saving reopens
/// the file from disk, so nothing here can reach what is written.
///
/// Reads through a separate document handle rather than the render one, exactly
/// as saving does: the flags set below must not be visible to the reader.
fn do_annotations(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
) -> AppResult<Vec<PageAnnotationsDto>> {
    let (src_path, password) = {
        let d = state.doc(doc)?;
        (d.path.clone(), d.password.clone())
    };
    let pages = annots::read_annotations(state.pdfium, &src_path, password.as_deref())?;
    if !pages.is_empty() {
        let raw = state.doc(doc)?.raw;
        for page in &pages {
            let indices: Vec<u32> = page.annots.iter().map(|a| a.index).collect();
            hide_annotations(state, raw, page.src, &indices);
        }
        // Visibility changed the page pixels. Generation bumps only mark
        // rasters as eviction candidates; RenderKey deliberately excludes the
        // generation, so otherwise a new URL can still return an old bitmap
        // with the saved text painted beneath the editor's copy.
        shared
            .page_cache
            .lock()
            .remove_matching(|key| key.doc == doc);
        shared
            .thumb_cache
            .lock()
            .remove_matching(|key| key.doc == doc);
        shared
            .preview_cache
            .lock()
            .remove_matching(|key| key.doc == doc);
    }
    Ok(pages)
}

/// PDF annotation flag bit 2: do not display.
const ANNOT_FLAG_HIDDEN: i32 = 2;

/// Hide the named annotations on one page of the render document.
///
/// Best effort throughout: an annotation that cannot be hidden is drawn twice,
/// which looks wrong but loses nothing, and is far better than refusing to give
/// the editor anything.
fn hide_annotations(state: &mut WorkerState, raw: FPDF_DOCUMENT, src: u32, indices: &[u32]) {
    let bindings = state.bindings;
    let page = bindings.FPDF_LoadPage(raw, src as i32);
    if page.is_null() {
        return;
    }
    for index in indices {
        let annot = bindings.FPDFPage_GetAnnot(page, *index as i32);
        if annot.is_null() {
            continue;
        }
        let flags = bindings.FPDFAnnot_GetFlags(annot);
        bindings.FPDFAnnot_SetFlags(annot, flags | ANNOT_FLAG_HIDDEN);
        bindings.FPDFPage_CloseAnnot(annot);
    }
    bindings.FPDF_ClosePage(page);
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

/// Formal environments (theorems, figures, ...) recovered from hyperref's named
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
    // held back until something actually lands under it - a section with no
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
    // re-sorted here - that sort is y-only, which reverses two-column pages.
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
        // Otherwise search the page for the statement the counter names - the
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
/// Tagging is rare - most papers carry no structure tree - so the extra text
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
/// papers whose figures carry no `\label` - which is most of them. The cost is
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
        // Widget rects come out of PDFium in user space; the frontend places
        // everything in display space, so they are mapped exactly as link
        // rects and character boxes are.
        let mapper = render::page_display_size(bindings, raw, pi as i32)
            .map(|[w, h]| text::DispMapper::new(bindings, page, w, h));
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
            let export_value = read_pdfium_utf16(|buffer, len| {
                bindings.FPDFAnnot_GetFormFieldExportValue(form, annotation, buffer, len)
            });
            let flags = bindings.FPDFAnnot_GetFormFieldFlags(form, annotation);
            let rect = mapper
                .as_ref()
                .and_then(|m| widget_rect(bindings, annotation, m))
                .unwrap_or(RectDto {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                });
            out.push(FormFieldDto {
                name,
                kind: form_field_kind(field_type).into(),
                value,
                page: pi,
                read_only: flags & 1 != 0,
                index: ai as u32,
                rect,
                export_value,
                checked: bindings.FPDFAnnot_IsChecked(form, annotation) != 0,
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

/// One widget's rectangle, in display space, or `None` when PDFium reports
/// none or reports a degenerate one - a control with no area cannot be aimed
/// at, so offering it to the UI would only produce an invisible hit target.
fn widget_rect(
    b: &dyn PdfiumLibraryBindings,
    annotation: FPDF_ANNOTATION,
    mapper: &text::DispMapper,
) -> Option<RectDto> {
    let mut rect = FS_RECTF {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    if b.FPDFAnnot_GetRect(annotation, &mut rect) == 0 {
        return None;
    }
    let (x, y, w, h) = mapper.map_box(
        rect.left as f64,
        rect.right as f64,
        rect.bottom as f64,
        rect.top as f64,
    );
    (w > 0.0 && h > 0.0).then_some(RectDto { x, y, w, h })
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

    fn test_shared(dir: &Path) -> EngineShared {
        use crate::cache::ByteLru;
        use crate::engine::queue::{GenerationMap, PrioQueue};
        use crate::engine::{LookupGauge, LookupSettings, Metrics};
        use crate::library::lookup::{self, LookupCache};
        use crate::library::ratelimit::HostLimiter;
        use crate::library::store::PaperStore;
        use crate::library::LibraryState;
        use crate::search::{SearchStore, TextLayoutCache};
        use parking_lot::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        EngineShared {
            queue: PrioQueue::new(),
            gens: GenerationMap::default(),
            cancels: GenerationMap::default(),
            page_cache: Mutex::new(ByteLru::new(1_000_000)),
            thumb_cache: Mutex::new(ByteLru::new(1_000_000)),
            preview_cache: Mutex::new(ByteLru::new(1_000_000)),
            search: Mutex::new(SearchStore::new(1_000_000)),
            text_layouts: Mutex::new(TextLayoutCache::new(1_000_000)),
            link_cache: Mutex::new(links::LinkCache::new(1_000_000)),
            library: Arc::new(Mutex::new(LibraryState::load(dir.to_path_buf()))),
            papers: Arc::new(Mutex::new(PaperStore::load(dir.to_path_buf()))),
            library_changed: Mutex::new(None),
            lookup_settings: Arc::new(Mutex::new(LookupSettings::default())),
            http: lookup::client(),
            limiter: Arc::new(HostLimiter::new()),
            lookup_cache: Arc::new(Mutex::new(LookupCache::load(dir))),
            edges_in_flight: Arc::new(Mutex::new(std::collections::HashSet::new())),
            refs_cache: Mutex::new(Default::default()),
            refs_provider: Mutex::new(Default::default()),
            app_data_dir: dir.to_path_buf(),
            lookup_activity: LookupGauge::default(),
            citation_resolver: Mutex::new(None),
            metrics: Metrics::default(),
            main_epoch: AtomicU64::new(0),
            progress: Mutex::new(None),
            last_metrics: Mutex::new(EngineMetricsDto::default()),
            gram_index: Mutex::new(None),
            gram_building: AtomicBool::new(false),
            library_watch: Mutex::new(None),
        }
    }

    /// A PDF whose `pages` pages are each one picture and no text: a scan
    /// without a text layer, every page of which needs OCR.
    fn scanned_pdf(pdfium: &Pdfium, path: &Path, pages: usize) {
        let mut document = pdfium.create_new_pdf().unwrap();
        let picture = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            30,
            40,
            image::Rgb([200, 200, 200]),
        ));
        for _ in 0..pages {
            let object = PdfPageImageObject::new_with_size(
                &document,
                &picture,
                PdfPoints::new(300.0),
                PdfPoints::new(400.0),
            )
            .unwrap();
            let mut page = document
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::Custom(
                    PdfPoints::new(300.0),
                    PdfPoints::new(400.0),
                ))
                .unwrap();
            page.objects_mut().add_image_object(object).unwrap();
        }
        document.save_to_file(path).unwrap();
    }

    fn idle() -> JobMeta {
        JobMeta {
            doc: 0,
            gen: 0,
            epoch: 0,
        }
    }

    /// The next job on the queue, waiting up to ten seconds for one.
    fn next_job(shared: &EngineShared) -> (JobMeta, Work) {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while shared.queue.is_empty() {
            assert!(Instant::now() < deadline, "no job arrived");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        shared.queue.pop_blocking().unwrap()
    }

    /// A stand-in for Vision that reads nothing until the test lets it,
    /// one page per permit, and numbers its readings.
    struct SlowOcr {
        permits: parking_lot::Mutex<std::sync::mpsc::Receiver<()>>,
        reading: std::sync::atomic::AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl SlowOcr {
        fn recognizer(self: &Arc<Self>) -> super::super::ocr_thread::Recognizer {
            let ocr = Arc::clone(self);
            Arc::new(move |_: &[u8]| {
                ocr.reading.store(true, Ordering::SeqCst);
                let _ = ocr.permits.lock().recv();
                let n = ocr.calls.fetch_add(1, Ordering::SeqCst);
                ocr.reading.store(false, Ordering::SeqCst);
                Some(vec![OcrLine {
                    text: format!("Recovered polynomial hierarchy reading {n}"),
                    confidence: 1.0,
                    bbox: [0.1, 0.1, 0.8, 0.05],
                }])
            })
        }
    }

    #[test]
    fn the_text_pass_waits_for_ocr_off_the_render_thread_and_resumes_within_budget() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("scan.pdf");
        scanned_pdf(&pdfium, &pdf, 8);
        let shared = Arc::new(test_shared(dir.path()));
        let (mtime_ms, size) = library::file_stamp(&pdf).unwrap();
        let candidate = library::ScanCandidate {
            path: pdf.clone(),
            mtime_ms,
            size,
            epoch: 0,
            unchanged: false,
        };
        let mut entry = library::entry_from_first_page(&candidate, "");
        entry.fingerprint = library::content_fingerprint(&pdf);
        shared.library.lock().upsert_entry(entry);

        let (permit, permits) = std::sync::mpsc::channel();
        let slow = Arc::new(SlowOcr {
            permits: parking_lot::Mutex::new(permits),
            reading: Default::default(),
            calls: Default::default(),
        });
        let service = OcrService::new(
            slow.recognizer(),
            ocr_done_to_queue(Arc::downgrade(&shared)),
        );
        let mut state = WorkerState::new(pdfium.bindings(), &pdfium, service, true);
        let budget_left = |state: &WorkerState| match &state.text_waiting {
            Some(TextWaiting::Extract(job)) => Some(job.budget),
            _ => None,
        };

        handle_work(&mut state, &shared, idle(), Work::LibraryTextNext);
        assert_eq!(budget_left(&state), Some(ocrtext::TEXT_OCR_BUDGET - 1));
        assert!(shared.queue.is_empty(), "the pass waits for the reading");
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while !slow.reading.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "OCR never started");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        // A page renders while Vision is busy with the first.
        let doc = do_open(&mut state, pdf.to_str().unwrap().into(), None)
            .unwrap()
            .doc_id;
        let key = RenderKey {
            doc,
            src: 3,
            rot: 0,
            scale_milli: 1000,
            kind: RenderKind::Page,
            tile: None,
            invert: false,
        };
        let meta = JobMeta {
            doc,
            gen: 0,
            epoch: 0,
        };
        assert!(!do_render(&mut state, &shared, meta, &key)
            .unwrap()
            .is_empty());
        assert!(
            slow.reading.load(Ordering::SeqCst),
            "the OCR is still in flight"
        );
        // A second chain of the pass is absorbed, not run against the first.
        handle_work(&mut state, &shared, idle(), Work::LibraryTextNext);
        assert!(shared.queue.is_empty());

        // Six pages a job: each reading resumes the job, which sends the next.
        for read in 1..=ocrtext::TEXT_OCR_BUDGET {
            permit.send(()).unwrap();
            let (meta, work) = next_job(&shared);
            assert!(matches!(work, Work::OcrDone { .. }));
            handle_work(&mut state, &shared, meta, work);
            if read < ocrtext::TEXT_OCR_BUDGET {
                assert_eq!(
                    budget_left(&state),
                    Some(ocrtext::TEXT_OCR_BUDGET - 1 - read)
                );
            }
        }
        // Budget spent with pages left: the document waits for the next job.
        assert!(state.text_waiting.is_none());
        assert_eq!(slow.calls.load(Ordering::SeqCst), ocrtext::TEXT_OCR_BUDGET);
        assert!(
            !shared
                .library
                .lock()
                .entry_for_path(&pdf)
                .unwrap()
                .text_indexed
        );
        let (meta, work) = next_job(&shared);
        assert!(matches!(work, Work::LibraryTextNext));
        handle_work(&mut state, &shared, meta, work);
        for _ in 0..2 {
            permit.send(()).unwrap();
            let (meta, work) = next_job(&shared);
            handle_work(&mut state, &shared, meta, work);
        }
        assert_eq!(slow.calls.load(Ordering::SeqCst), 8, "each page read once");
        assert!(state.text_waiting.is_none() && state.ocr_in_flight.is_empty());
        assert!(
            shared
                .library
                .lock()
                .entry_for_path(&pdf)
                .unwrap()
                .text_indexed
        );
        let text = crate::search::librarytext::read_sidecar_text(dir.path(), &pdf, mtime_ms, size)
            .unwrap();
        for n in 0..8 {
            assert!(text.contains(&format!("hierarchy reading {n}")), "{text}");
        }
    }

    #[test]
    fn the_idle_scan_parks_a_file_on_its_ocr_and_scans_it_again_when_read() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let papers = dir.path().join("papers");
        std::fs::create_dir_all(&papers).unwrap();
        scanned_pdf(&pdfium, &papers.join("scan.pdf"), 1);
        let shared = Arc::new(test_shared(dir.path()));
        let (permit, permits) = std::sync::mpsc::channel();
        let slow = Arc::new(SlowOcr {
            permits: parking_lot::Mutex::new(permits),
            reading: Default::default(),
            calls: Default::default(),
        });
        let service = OcrService::new(
            slow.recognizer(),
            ocr_done_to_queue(Arc::downgrade(&shared)),
        );
        let mut state = WorkerState::new(pdfium.bindings(), &pdfium, service, true);
        assert!(shared.library.lock().add_root(papers).unwrap());

        handle_work(&mut state, &shared, idle(), Work::LibraryScanNext);
        assert!(state.scan_waiting.is_some(), "parked on its first page");
        assert!(shared.library.lock().all_entries().is_empty());
        // Another scan job finds it parked and leaves it be.
        handle_work(&mut state, &shared, idle(), Work::LibraryScanNext);
        assert!(state.scan_waiting.is_some() && shared.queue.is_empty());

        permit.send(()).unwrap();
        let (meta, work) = next_job(&shared);
        assert!(matches!(work, Work::OcrDone { .. }));
        handle_work(&mut state, &shared, meta, work);
        assert!(state.scan_waiting.is_none() && state.ocr_in_flight.is_empty());
        let entries = shared.library.lock().all_entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].ocr_wanted && !entries[0].ocr_deferred);
        // The scan is done: the text pass is next.
        assert!(matches!(next_job(&shared).1, Work::LibraryTextNext));
        assert_eq!(slow.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_open_scan_is_searched_from_the_ocr_cache_and_highlighted_from_line_boxes() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("scan.pdf");
        scanned_pdf(&pdfium, &pdf, 2);
        let shared = test_shared(dir.path());
        // The library has read page 1 of 2.
        let fingerprint = library::content_fingerprint(&pdf);
        let mut cache = OcrCache::load(dir.path(), &pdf, fingerprint.as_deref());
        cache.insert(
            0,
            Some(vec![OcrLine {
                text: "Arthur-Merlin Games".into(),
                confidence: 0.9,
                bbox: [0.1, 0.2, 0.5, 0.05],
            }]),
        );
        cache.save(dir.path(), &pdf);
        let never = OcrService::new(
            Arc::new(|_: &[u8]| panic!("an open document never runs OCR")),
            Arc::new(|_| {}),
        );
        let mut state = WorkerState::new(pdfium.bindings(), &pdfium, never, true);
        let doc = do_open(&mut state, pdf.to_str().unwrap().into(), None)
            .unwrap()
            .doc_id;
        for src in 0..2 {
            ensure_search_extracted(&mut state, &shared, doc, src).unwrap();
        }
        assert!(state.ocr_in_flight.is_empty());
        assert!(shared.search.lock().ocr_geometry(doc, 1).is_none());

        let found = shared.search.lock().query(doc, "merlin games", false, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].src, 0);
        let hit = &found[0].matches[0];
        assert_eq!((hit.start, hit.len), (7, 12));
        let rects = do_match_rects(&mut state, &shared, doc, 0, hit.start, hit.len).unwrap();
        // The page is 300 x 400 pt; the line's box spans x 30..180, and
        // "Merlin Games" is its last 12 of 19 characters.
        assert_eq!(rects.len(), 1);
        let [x, y, w, h] = rects[0];
        assert!((x - (30.0 + 150.0 * 7.0 / 19.0)).abs() < 0.01, "{x}");
        assert!((w - 150.0 * 12.0 / 19.0).abs() < 0.01, "{w}");
        assert!((y - (1.0 - 0.2 - 0.05) * 400.0).abs() < 0.01, "{y}");
        assert!((h - 0.05 * 400.0).abs() < 0.01, "{h}");
    }

    /// A one-page PDF whose only text is `label`, so every fixture has
    /// its own bytes (and fingerprint).
    fn labelled_pdf(pdfium: &Pdfium, path: &Path, label: &str) {
        let mut document = pdfium.create_new_pdf().unwrap();
        let font = document.fonts_mut().helvetica();
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(300.0),
                PdfPoints::new(400.0),
            ))
            .unwrap();
        let mut text =
            PdfPageTextObject::new(&document, label, font, PdfPoints::new(14.0)).unwrap();
        text.translate(PdfPoints::new(20.0), PdfPoints::new(300.0))
            .unwrap();
        page.objects_mut().add_text_object(text).unwrap();
        drop(page);
        document.save_to_file(path).unwrap();
    }

    /// Every library item, papers and others alike.
    fn items(shared: &EngineShared) -> u32 {
        let counts = shared.papers.lock().counts();
        counts.all + counts.others
    }

    fn quiet_state(pdfium: &Pdfium) -> WorkerState<'_> {
        WorkerState::new(
            pdfium.bindings(),
            pdfium,
            OcrService::new(Arc::new(|_: &[u8]| None), Arc::new(|_| {})),
            false,
        )
    }

    /// Hand the watcher's batch over as `EngineWatchSink::deliver` does
    /// and run its jobs until the chain ends.
    fn drain_watch(
        state: &mut WorkerState,
        shared: &EngineShared,
        files: Vec<PathBuf>,
        gone: Vec<PathBuf>,
    ) {
        if shared.library.lock().queue_watched(files, gone) {
            handle_work(state, shared, idle(), Work::LibraryWatchNext);
        }
        while let Some((meta, work)) = (!shared.queue.is_empty())
            .then(|| shared.queue.pop_blocking())
            .flatten()
        {
            if matches!(work, Work::LibraryWatchNext) {
                handle_work(state, shared, meta, work);
            }
        }
    }

    #[test]
    fn a_watched_file_is_indexed_followed_through_a_rename_and_marked_missing_when_deleted() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("papers");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let shared = test_shared(dir.path());
        shared.library.lock().add_root(root.clone()).unwrap();
        let mut state = quiet_state(&pdfium);

        // Created: scanned, indexed and given a paper; text work follows.
        let first = root.join("downloaded.pdf");
        labelled_pdf(
            &pdfium,
            &first,
            "Interactive proofs and the polynomial hierarchy",
        );
        drain_watch(&mut state, &shared, vec![first.clone()], Vec::new());
        assert!(shared.library.lock().entry_for_path(&first).is_some());
        let paper = shared.papers.lock().paper_for_path(&first).unwrap();
        assert_eq!(paper.files.len(), 1);

        // Renamed: the same paper, now at the new path, not missing.
        let renamed = root.join("renamed.pdf");
        std::fs::rename(&first, &renamed).unwrap();
        drain_watch(
            &mut state,
            &shared,
            vec![renamed.clone()],
            vec![first.clone()],
        );
        assert!(shared.library.lock().entry_for_path(&first).is_none());
        let moved = shared.papers.lock().paper_for_path(&renamed).unwrap();
        assert_eq!(moved.id, paper.id, "reattached by fingerprint");
        assert_eq!(moved.files.len(), 1);
        assert!(!moved.files[0].missing);
        assert_eq!(items(&shared), 1);

        // Moved to the Trash under the root (a root that holds the home
        // folder): the Trash path is never the library's, so the file is
        // missing, not reattached there.
        let trash = root.join(".Trash");
        std::fs::create_dir_all(&trash).unwrap();
        let trashed = trash.join("renamed.pdf");
        std::fs::rename(&renamed, &trashed).unwrap();
        let roots = vec![root.clone()];
        assert!(library::watch::is_ignored(&trashed, &roots, &[]));
        drain_watch(&mut state, &shared, Vec::new(), vec![renamed.clone()]);
        let gone = shared.papers.lock().paper_for_path(&renamed).unwrap();
        assert!(gone.files[0].missing);
        assert!(gone.files[0].missing_since.is_some());
        assert!(shared.library.lock().entry_for_path(&renamed).is_none());
        // Even a full scan does not find it there.
        shared.library.lock().add_root(root.clone()).unwrap();
        assert!(!shared.library.lock().has_scan_work());
        assert!(shared.papers.lock().paper_for_path(&trashed).is_none());
    }

    #[test]
    fn a_burst_of_watched_files_saves_and_notifies_a_bounded_number_of_times() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("papers");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let shared = test_shared(dir.path());
        shared.library.lock().add_root(root.clone()).unwrap();
        let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&notified);
        *shared.library_changed.lock() = Some(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let mut state = quiet_state(&pdfium);
        let files: Vec<PathBuf> = (0..30)
            .map(|n| {
                let path = root.join(format!("paper-{n:02}.pdf"));
                labelled_pdf(&pdfium, &path, &format!("A distinct study number {n}"));
                path
            })
            .collect();
        let started = Instant::now();
        drain_watch(&mut state, &shared, files.clone(), Vec::new());
        let elapsed = started.elapsed().as_secs() as usize;
        assert_eq!(items(&shared), 30);
        assert_eq!(shared.library.lock().all_entries().len(), 30);
        let count = notified.load(Ordering::SeqCst);
        // At most one a second while it drains, and one at the end.
        assert!(count >= 1, "the end of the burst is announced");
        assert!(count <= elapsed + 2, "{count} notifications in {elapsed} s");
        // The same batch again is unchanged and costs nothing.
        notified.store(0, Ordering::SeqCst);
        drain_watch(&mut state, &shared, files, Vec::new());
        assert_eq!(notified.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn adopting_saved_text_removes_it_from_cached_rasters() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("saved-text.pdf");
        let mut document = pdfium.create_new_pdf().unwrap();
        let font = document.fonts_mut().helvetica();
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(200.0),
                PdfPoints::new(100.0),
            ))
            .unwrap();
        let mut annot = page.annotations_mut().create_stamp_annotation().unwrap();
        annot
            .set_bounds(PdfRect::new_from_values(10.0, 10.0, 90.0, 190.0))
            .unwrap();
        annot.set_creator(annots::TEXT_STAMP_CREATOR).unwrap();
        annot.set_contents("Saved text").unwrap();
        let mut text =
            PdfPageTextObject::new(&document, "Saved text", font, PdfPoints::new(20.0)).unwrap();
        text.translate(PdfPoints::new(20.0), PdfPoints::new(40.0))
            .unwrap();
        annot.objects_mut().add_text_object(text).unwrap();
        drop(page);
        document.save_to_file(&source).unwrap();
        drop(document);

        let shared = test_shared(dir.path());
        let mut state = WorkerState::new(
            pdfium.bindings(),
            &pdfium,
            OcrService::new(Arc::new(|_: &[u8]| None), Arc::new(|_| {})),
            false,
        );
        let doc = do_open(&mut state, source.to_str().unwrap().into(), None)
            .unwrap()
            .doc_id;
        let meta = JobMeta {
            doc,
            gen: 0,
            epoch: 0,
        };
        let keys: Vec<_> = [RenderKind::Page, RenderKind::Thumb, RenderKind::Preview]
            .into_iter()
            .map(|kind| RenderKey {
                doc,
                src: 0,
                rot: 0,
                scale_milli: 1000,
                kind,
                tile: None,
                invert: false,
            })
            .collect();
        let has_ink = |png: &[u8]| {
            image::load_from_memory(png)
                .unwrap()
                .to_rgb8()
                .pixels()
                .any(|p| p.0.iter().any(|c| *c < 128))
        };
        for key in &keys {
            assert!(has_ink(&do_render(&mut state, &shared, meta, key).unwrap()));
        }
        handle_work(
            &mut state,
            &shared,
            meta,
            Work::Annotations {
                doc,
                respond: Box::new(|result| {
                    assert_eq!(result.unwrap()[0].annots[0].kind, "textbox");
                }),
            },
        );
        let after: Vec<_> = keys
            .iter()
            .map(|key| do_render(&mut state, &shared, meta, key).unwrap())
            .collect();
        state.close_one(doc, &shared, true);
        for png in after {
            assert!(
                !has_ink(&png),
                "a cached appearance would draw a second copy under the editable text"
            );
        }
    }

    #[test]
    fn ocr_reading_replaces_a_scan_title_only_when_it_adds_to_it() {
        use library::firstpage::{Confidence, FirstPage};
        let reading = |title: Option<&str>, confidence| FirstPage {
            title: title.map(str::to_owned),
            authors: Vec::new(),
            title_confidence: confidence,
            authors_confidence: Confidence::Low,
        };
        let ocr = reading(
            Some("Arthur Merlin Games: A Randomized Proof System, and a Hierarchy of Complexity Classes"),
            Confidence::High,
        );
        // The capture layer lost the first line and read the second, sure.
        let layer = reading(
            Some("and a Hierarchy of Complexity Classes"),
            Confidence::High,
        );
        assert!(ocr_reading_is_better(&layer, &ocr, false));
        // A sure, different title from the layer stands.
        let other = reading(Some("Interactive Proofs"), Confidence::High);
        assert!(!ocr_reading_is_better(&other, &ocr, false));
        // No title, or an unsure one, gives way to a sure OCR reading.
        assert!(ocr_reading_is_better(
            &reading(None, Confidence::Low),
            &ocr,
            false
        ));
        assert!(ocr_reading_is_better(
            &reading(Some("hh"), Confidence::Low),
            &ocr,
            false
        ));
        assert!(
            ocr_reading_is_better(&other, &ocr, true),
            "the layer is garbage"
        );
        // OCR with no title never replaces anything.
        assert!(!ocr_reading_is_better(
            &layer,
            &reading(None, Confidence::Low),
            true
        ));
        let unsure = reading(Some("Arthur"), Confidence::Low);
        assert!(!ocr_reading_is_better(
            &reading(Some("x"), Confidence::Low),
            &unsure,
            false
        ));
    }

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
                invert: false,
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

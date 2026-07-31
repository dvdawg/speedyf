//! The engine thread. Sole owner of every PDFium handle in the process.
//! Raw FPDF_* handles never leave this thread; results cross back as owned
//! bytes/DTOs through responder callbacks.

use super::pdfium_init;
use super::queue::JobMeta;
use super::render;
use super::save;
use super::text;
use super::types::*;
use super::{EngineShared, Work};
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::*;
use pdfium_render::prelude::{FPDF_DOCUMENT, FPDF_PAGE};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Open page handles kept per document to avoid re-parsing page objects on
/// every render/text request for the same page.
const PAGE_HANDLE_CAP: usize = 8;

struct DocState {
    raw: FPDF_DOCUMENT,
    path: String,
    password: Option<String>,
    page_count: u32,
    /// display sizes (pt), lazily hydrated
    sizes: Vec<Option<[f32; 2]>>,
    /// tiny MRU of open page handles: (src_index, handle)
    pages: Vec<(u16, FPDF_PAGE)>,
    index_cursor: u32,
    index_stopped: bool,
}

struct WorkerState<'a> {
    bindings: &'a dyn PdfiumLibraryBindings,
    pdfium: &'a Pdfium,
    docs: HashMap<DocId, DocState>,
    next_id: DocId,
}

impl<'a> WorkerState<'a> {
    fn doc(&mut self, id: DocId) -> AppResult<&mut DocState> {
        self.docs.get_mut(&id).ok_or(AppError::DocumentNotFound)
    }

    fn page_handle(&mut self, id: DocId, src: u16) -> AppResult<FPDF_PAGE> {
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

    fn display_size(&mut self, id: DocId, src: u16) -> AppResult<[f32; 2]> {
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

    fn close_doc(&mut self, id: DocId, shared: &EngineShared) {
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
        shared.search.lock().remove_doc(id);
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
    };

    while let Some((meta, work)) = shared.queue.pop_blocking() {
        // A generation represents render state (principally the scale bucket).
        // Never let a zoom gesture cancel indexing, text, save, or form work.
        let stale = shared.gens.is_stale(&meta) && is_generation_cancellable(&work);
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
        Work::MatchRects { respond, .. } => respond(Err(err())),
        Work::Save { respond, .. } => respond(Err(err())),
        Work::FormFields { respond, .. } => respond(Err(err())),
        Work::ImageSize { respond, .. } => respond(Err(err())),
        Work::Close { .. } | Work::IndexNext { .. } => {}
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
        Work::Sizes {
            doc,
            from,
            count,
            respond,
        } => respond(do_sizes(state, doc, from, count)),
        Work::Render { key, respond } => {
            let started = Instant::now();
            let result = do_render(state, shared, &key);
            if result.is_ok() {
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
            respond(result);
        }
        Work::TextLayout { doc, src, respond } => respond(do_text_layout(state, shared, doc, src)),
        Work::IndexNext { doc } => do_index_next(state, shared, meta, doc),
        Work::MatchRects {
            doc,
            src,
            start,
            len,
            respond,
        } => respond(do_match_rects(state, doc, src, start, len)),
        Work::Save {
            doc,
            plan,
            dest,
            respond,
        } => respond(do_save(state, doc, plan, dest)),
        Work::FormFields { doc, respond } => respond(do_form_fields(state, doc)),
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
            index_stopped: false,
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
    key: &RenderKey,
) -> AppResult<Arc<Vec<u8>>> {
    // Another request may have populated the cache while this job queued.
    if let Some(hit) = match key.kind {
        RenderKind::Thumb => shared.thumb_cache.lock().get(key),
        _ => shared.page_cache.lock().get(key),
    } {
        return Ok(hit);
    }
    let [w_pt, h_pt] = state.display_size(key.doc, key.src)?;
    let page = state.page_handle(key.doc, key.src)?;
    let out = render::render_page(
        state.bindings,
        page,
        w_pt,
        h_pt,
        key.scale_milli as f32 / 1000.0,
        key.rot,
        key.tile,
    )?;
    let bytes = Arc::new(out.png);
    let cost = crate::cache::bitmap_cost(out.width, out.height);
    match key.kind {
        RenderKind::Thumb => {
            shared
                .thumb_cache
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

fn ensure_extracted(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u16,
) -> AppResult<(u32, bool)> {
    if shared.search.lock().page(doc, src).is_some() {
        return Ok((0, true));
    }
    let [w_pt, h_pt] = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    let extracted = text::extract_page(state.bindings, page, w_pt, h_pt);
    let stored = shared.search.lock().store_page(
        doc,
        src,
        &extracted.raw,
        extracted.runs,
        extracted.char_count,
    );
    if stored {
        shared.metrics.pages_indexed.fetch_add(1, Ordering::Relaxed);
    }
    Ok((extracted.char_count, stored))
}

fn do_text_layout(
    state: &mut WorkerState,
    shared: &EngineShared,
    doc: DocId,
    src: u16,
) -> AppResult<PageTextDto> {
    ensure_extracted(state, shared, doc, src)?;
    let search = shared.search.lock();
    let entry = search.page(doc, src);
    Ok(PageTextDto {
        src: src as u32,
        runs: entry.map(|e| e.runs.clone()).unwrap_or_default(),
        char_count: entry.map(|e| e.char_count).unwrap_or(0),
    })
}

fn do_index_next(state: &mut WorkerState, shared: &EngineShared, meta: JobMeta, doc: DocId) {
    let Ok(d) = state.doc(doc) else { return };
    if d.index_stopped {
        return;
    }
    let total = d.page_count;
    // find next page missing from the index
    let mut next = d.index_cursor;
    {
        let search = shared.search.lock();
        while next < total && search.page(doc, next as u16).is_some() {
            next += 1;
        }
    }
    if next >= total {
        emit_progress(shared, doc, total, total, false);
        return;
    }
    let ok = match ensure_extracted(state, shared, doc, next as u16) {
        Ok((_, stored)) => stored,
        Err(_) => true, // unreadable page: skip it, keep going
    };
    if let Ok(d) = state.doc(doc) {
        d.index_cursor = next + 1;
        if !ok {
            d.index_stopped = true;
        }
    }
    let indexed = shared.search.lock().indexed_count(doc);
    let truncated = shared.search.lock().is_truncated(doc);
    emit_progress(shared, doc, indexed, total, truncated);
    if !ok {
        log::warn!("search index budget exhausted for doc {doc}; indexing stopped");
        return;
    }
    // Requeue at the same generation: closing/reloading the doc cancels this.
    shared.queue.push(
        Priority::TextExtract,
        JobMeta { doc, gen: meta.gen },
        Work::IndexNext { doc },
    );
}

fn emit_progress(shared: &EngineShared, doc: DocId, indexed: u32, total: u32, truncated: bool) {
    if let Some(cb) = shared.progress.lock().as_ref() {
        cb(
            doc,
            SearchStatusDto {
                indexed,
                total,
                truncated,
                chars_indexed: shared.search.lock().used(),
            },
        );
    }
}

fn do_match_rects(
    state: &mut WorkerState,
    doc: DocId,
    src: u16,
    start: u32,
    len: u32,
) -> AppResult<Vec<[f32; 4]>> {
    let [w_pt, h_pt] = state.display_size(doc, src)?;
    let page = state.page_handle(doc, src)?;
    Ok(text::match_rects(
        state.bindings,
        page,
        w_pt,
        h_pt,
        start,
        len,
    ))
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
    let (src_path, password, same_file, page_count, sizes, index_cursor, index_stopped) = {
        let d = state.doc(doc)?;
        (
            d.path.clone(),
            d.password.clone(),
            std::path::Path::new(&d.path) == dest.as_path(),
            d.page_count,
            d.sizes.clone(),
            d.index_cursor,
            d.index_stopped,
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
                                index_stopped,
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

fn do_form_fields(state: &mut WorkerState, doc: DocId) -> AppResult<Vec<FormFieldDto>> {
    let (path, password) = {
        let d = state.doc(doc)?;
        (d.path.clone(), d.password.clone())
    };
    let document = state
        .pdfium
        .load_pdf_from_file(&path, password.as_deref())
        .map_err(|e| AppError::Malformed(format!("cannot reopen for form read: {e:?}")))?;
    if document.form().is_none() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let page_count = document.pages().len();
    for pi in 0..page_count {
        let Ok(page) = document.pages().get(pi) else {
            continue;
        };
        let annotation_count = page.annotations().len();
        for ai in 0..annotation_count {
            let Ok(annotation) = page.annotations().get(ai) else {
                continue;
            };
            let Some(field) = annotation.as_form_field() else {
                continue;
            };
            let kind = if field.as_text_field().is_some() {
                "text"
            } else {
                "other"
            };
            let value = field
                .as_text_field()
                .and_then(|t| t.value())
                .unwrap_or_default();
            out.push(FormFieldDto {
                name: field.name().unwrap_or_default(),
                kind: kind.into(),
                value,
                page: pi as u32,
                read_only: false,
            });
        }
    }
    Ok(out)
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
        assert!(!is_generation_cancellable(&Work::IndexNext { doc: 1 }));
        assert!(!is_generation_cancellable(&Work::Close { doc: 1 }));
    }
}

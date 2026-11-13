//! Incremental, byte-budgeted search index and foreground text-layout cache.
//!
//! Search text and selectable-page geometry have deliberately separate
//! lifetimes:
//!
//! - The search index stores compact UTF-8 normalized text plus a sparse map
//!   back to PDFium character indices. It is filled in page order so a large
//!   document gets the broadest possible coverage for the configured budget.
//! - The text-layout cache is an LRU of recent foreground pages. Selection and
//!   highlighting therefore keep working after the search budget is full, and
//!   revisiting nearby pages does not re-run PDFium extraction.

use crate::engine::ocr::{self, OcrLine};
use crate::engine::types::{BubbleDto, DocId, MatchDto, PageMatchesDto, SearchQueryDto, TextRun};
use std::collections::HashMap;
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

/// Piecewise-constant `raw_index - normalized_index` offsets.
///
/// Most text is identity-mapped. Whitespace collapsing and NFKC expansion add
/// only a handful of breakpoints per line/page instead of one `u32` per
/// normalized character.
#[derive(Default)]
struct OffsetMap {
    breaks: Vec<(u32, i32)>,
}

impl OffsetMap {
    fn push(&mut self, norm_index: u32, raw_index: u32) {
        let delta = raw_index as i64 - norm_index as i64;
        let delta = delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        if self
            .breaks
            .last()
            .is_none_or(|(_, previous)| *previous != delta)
        {
            self.breaks.push((norm_index, delta));
        }
    }

    fn raw_index(&self, norm_index: u32) -> u32 {
        let position = self
            .breaks
            .partition_point(|(at, _)| *at <= norm_index)
            .saturating_sub(1);
        let delta = self
            .breaks
            .get(position)
            .map(|(_, delta)| *delta)
            .unwrap_or(0);
        (norm_index as i64 + delta as i64).max(0) as u32
    }

    fn cost(&self) -> u64 {
        (self.breaks.len() * std::mem::size_of::<(u32, i32)>()) as u64
    }

    /// The breakpoints, for persisting a page to the library's text index.
    pub(crate) fn breakpoints(&self) -> &[(u32, i32)] {
        &self.breaks
    }

    pub(crate) fn from_breakpoints(breaks: Vec<(u32, i32)>) -> Self {
        OffsetMap { breaks }
    }
}

struct NormalizedText {
    text: String,
    map: OffsetMap,
}

/// Normalize one raw character stream while retaining a compact mapping back
/// to PDFium's character indices.
fn normalize(raw: &str) -> NormalizedText {
    normalize_spaced(raw, &[])
}

/// `normalize`, with whitespace inserted before each raw index in `breaks`
/// (ascending): word gaps a file encodes only as position
/// (`library::word_breaks`). An inserted space maps back to the index it
/// precedes, so a match's raw offsets stay PDFium character indices.
fn normalize_spaced(raw: &str, breaks: &[u32]) -> NormalizedText {
    let mut text = String::with_capacity(raw.len());
    let mut map = OffsetMap::default();
    let mut norm_index = 0u32;
    let mut in_ws = false;

    let push = |text: &mut String,
                map: &mut OffsetMap,
                norm_index: &mut u32,
                raw_index: u32,
                value: char| {
        map.push(*norm_index, raw_index);
        text.push(value);
        *norm_index += 1;
    };

    let mut breaks = breaks.iter().copied().peekable();
    for (raw_index, value) in raw.chars().enumerate() {
        let raw_index = raw_index as u32;
        let mut gap = false;
        while let Some(at) = breaks.next_if(|at| *at <= raw_index) {
            gap |= at == raw_index;
        }
        if gap {
            if !in_ws && !text.is_empty() {
                push(&mut text, &mut map, &mut norm_index, raw_index, ' ');
            }
            in_ws = true;
        }
        if value.is_whitespace() {
            if !in_ws && !text.is_empty() {
                push(&mut text, &mut map, &mut norm_index, raw_index, ' ');
            }
            in_ws = true;
            continue;
        }
        in_ws = false;

        let straightened = match value {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => '"',
            _ => value,
        };
        if straightened != value {
            push(
                &mut text,
                &mut map,
                &mut norm_index,
                raw_index,
                straightened,
            );
            continue;
        }
        for normalized in value.nfkc() {
            push(&mut text, &mut map, &mut norm_index, raw_index, normalized);
        }
    }

    NormalizedText { text, map }
}

/// Case-insensitive one-character fold. Keeping the character count stable
/// means the sparse raw-offset map is shared by case-sensitive and folded
/// searches.
fn fold(value: char) -> char {
    value.to_lowercase().next().unwrap_or(value)
}

fn fold_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        // `fold` goes through `char::to_lowercase`, which constructs an
        // iterator per character. That is invisible for one page and dominates
        // when it runs over a whole library, so ASCII — nearly everything, even
        // in a maths paper — takes the byte-wise path instead.
        if ch.is_ascii() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(fold(ch));
        }
    }
    out
}

pub struct PageEntry {
    normalized: String,
    folded: String,
    map: OffsetMap,
    pub char_count: u32,
    pub cost: u64,
    /// Set when the page's searchable text is OCR's rather than PDFium's
    /// (`ocr_page`): its offsets index the OCR text, not PDFium characters,
    /// so a hit is highlighted from the OCR line boxes instead.
    pub ocr: Option<Arc<OcrGeometry>>,
}

/// One OCR line's place in an OCR page's text and on the page.
#[derive(Clone, Debug, PartialEq)]
struct OcrSpan {
    /// Character range `[start, end)` of the line in the page text.
    start: u32,
    end: u32,
    /// Display space, `[x, y_bottom, w, h]`, y-up: the form
    /// `text::merge_match_rects` returns.
    rect: [f32; 4],
}

/// Where each line of an OCR'd page's text sits on the page. Vision gives
/// a box per line, not per character, so a hit's rectangle is the share of
/// its line's box that its characters take, spaced evenly — close for the
/// proportional type of a journal page, and on the right line always.
#[derive(Debug, Default)]
pub struct OcrGeometry {
    spans: Vec<OcrSpan>,
}

impl OcrGeometry {
    /// Rectangles for the OCR text's characters `[start, start + len)`:
    /// one per line the range touches.
    pub fn match_rects(&self, start: u32, len: u32) -> Vec<[f32; 4]> {
        let end = start.saturating_add(len);
        self.spans
            .iter()
            .filter(|span| span.start < end && start < span.end)
            .map(|span| {
                let chars = (span.end - span.start).max(1) as f32;
                let from = start.max(span.start) - span.start;
                let to = end.min(span.end) - span.start;
                let [x, y, w, h] = span.rect;
                let left = x + w * from as f32 / chars;
                let right = x + w * to as f32 / chars;
                [left, y, (right - left).max(0.0), h]
            })
            .collect()
    }

    fn cost(&self) -> u64 {
        (self.spans.len() * std::mem::size_of::<OcrSpan>()) as u64
    }
}

/// An OCR'd page as the search index takes it: its text exactly as
/// `ocr::page_text` joins it (lines under `MIN_LINE_CONFIDENCE` and blank
/// ones dropped, `\n` between), and each kept line's character range and
/// box. The boxes are Vision's, normalized to the rendered page with a
/// top-left origin; `page_w`/`page_h` are the page's display size, which is
/// what it was rendered at.
pub fn ocr_page(lines: &[OcrLine], page_w: f32, page_h: f32) -> (String, OcrGeometry) {
    let mut text = String::new();
    let mut count = 0u32;
    let mut spans = Vec::new();
    for line in lines {
        if line.confidence < ocr::MIN_LINE_CONFIDENCE || line.text.trim().is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
            count += 1;
        }
        let kept = line.text.trim_end();
        let start = count;
        count += kept.chars().count() as u32;
        text.push_str(kept);
        let [x, y, w, h] = line.bbox;
        spans.push(OcrSpan {
            start,
            end: count,
            rect: [x * page_w, (1.0 - y - h) * page_h, w * page_w, h * page_h],
        });
    }
    (text, OcrGeometry { spans })
}

impl PageEntry {
    /// The normalized text and its offset map, for writing to a sidecar.
    ///
    /// The library index stores this form rather than the raw stream so a
    /// query does not have to re-run NFKC over the whole library on every
    /// keystroke — only the cheap case fold, in `from_indexed`.
    pub(crate) fn indexed_parts(&self) -> (&str, &str, &[(u32, i32)], u32) {
        (
            &self.normalized,
            &self.folded,
            self.map.breakpoints(),
            self.char_count,
        )
    }

    /// Rebuild a page from a sidecar. The expensive half of `new` — Unicode
    /// normalization — was done once, when the document was indexed.
    pub(crate) fn from_indexed(
        normalized: String,
        folded: String,
        breaks: Vec<(u32, i32)>,
        char_count: u32,
    ) -> Self {
        let map = OffsetMap::from_breakpoints(breaks);
        let cost = 64 + normalized.len() as u64 + folded.len() as u64 + map.cost();
        PageEntry {
            normalized,
            folded,
            map,
            char_count,
            cost,
            ocr: None,
        }
    }

    /// A page whose word gaps are partly geometry: whitespace is inserted
    /// before each raw index in `breaks` (`normalize_spaced`).
    pub(crate) fn new_spaced(raw: &str, char_count: u32, breaks: &[u32]) -> Self {
        PageEntry::build(normalize_spaced(raw, breaks), char_count)
    }

    fn new(raw: &str, char_count: u32) -> Self {
        PageEntry::build(normalize(raw), char_count)
    }

    /// An OCR'd page (`ocr_page`): its offsets index `text`.
    fn new_ocr(text: &str, geometry: OcrGeometry) -> Self {
        let mut entry = PageEntry::new(text, text.chars().count() as u32);
        entry.cost += geometry.cost();
        entry.ocr = Some(Arc::new(geometry));
        entry
    }

    fn build(normalized: NormalizedText, char_count: u32) -> Self {
        let NormalizedText {
            text: normalized,
            map,
        } = normalized;
        let folded = fold_text(&normalized);
        // Count actual retained bytes, not a decoded/worst-case multiplier.
        let cost = 64 + normalized.len() as u64 + folded.len() as u64 + map.cost();
        PageEntry {
            normalized,
            folded,
            map,
            char_count,
            cost,
            ocr: None,
        }
    }
}

#[derive(Default)]
pub struct DocIndex {
    pub pages: HashMap<u32, Arc<PageEntry>>,
    pub truncated: bool,
    chars_indexed: u64,
    /// Retained bytes for this document alone, so the shared budget can be
    /// rebalanced between documents without summing every page.
    used: u64,
}

pub struct SearchStore {
    docs: HashMap<DocId, DocIndex>,
    budget: u64,
    used: u64,
}

fn char_boundaries(value: &str) -> Vec<usize> {
    let mut boundaries: Vec<usize> = value.char_indices().map(|(index, _)| index).collect();
    boundaries.push(value.len());
    boundaries
}

fn snippet(normalized: &str, boundaries: &[usize], match_start: usize, match_len: usize) -> String {
    let start = match_start.saturating_sub(30);
    let end = (match_start + match_len + 30).min(boundaries.len().saturating_sub(1));
    normalized[boundaries[start]..boundaries[end]].to_owned()
}

pub(crate) fn find_prepared_matches(
    entry: &PageEntry,
    prepared_query: &str,
    case_sensitive: bool,
    limit: usize,
) -> Vec<(u32, u32, String)> {
    if prepared_query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let target = if case_sensitive {
        &entry.normalized
    } else {
        &entry.folded
    };
    if target.len() < prepared_query.len() {
        return Vec::new();
    }

    let target_boundaries = char_boundaries(target);
    let normalized_boundaries = if case_sensitive {
        None
    } else {
        Some(char_boundaries(&entry.normalized))
    };
    let snippet_boundaries = normalized_boundaries
        .as_deref()
        .unwrap_or(target_boundaries.as_slice());
    let query_chars = prepared_query.chars().count();
    let mut output = Vec::new();

    for (byte_index, _) in target.match_indices(prepared_query).take(limit) {
        let Ok(norm_start) = target_boundaries.binary_search(&byte_index) else {
            continue;
        };
        let norm_end = norm_start + query_chars;
        if norm_end == 0 || norm_end >= target_boundaries.len() {
            continue;
        }
        let raw_start = entry.map.raw_index(norm_start as u32);
        let raw_end = entry.map.raw_index((norm_end - 1) as u32);
        output.push((
            raw_start,
            raw_end.saturating_sub(raw_start) + 1,
            snippet(
                &entry.normalized,
                snippet_boundaries,
                norm_start,
                query_chars,
            ),
        ));
    }
    output
}

pub(crate) fn prepare_query(query: &str, case_sensitive: bool) -> String {
    let normalized = normalize(query).text;
    if case_sensitive {
        normalized
    } else {
        fold_text(&normalized)
    }
}

pub fn query_pages(
    pages: &[(u32, Arc<PageEntry>)],
    query: &str,
    case_sensitive: bool,
    per_page_limit: usize,
    global_limit: usize,
) -> SearchQueryDto {
    let prepared = prepare_query(query, case_sensitive);
    if prepared.is_empty() || global_limit == 0 {
        return SearchQueryDto {
            pages: Vec::new(),
            truncated: false,
            limit: global_limit as u32,
        };
    }

    let mut output = Vec::new();
    let mut returned = 0usize;
    let mut truncated = false;
    for (src, entry) in pages {
        // One extra result is enough to prove truncation without generating a
        // large intermediate vector.
        let remaining = global_limit.saturating_sub(returned);
        let requested = per_page_limit.min(remaining).saturating_add(1);
        let mut found = find_prepared_matches(entry, &prepared, case_sensitive, requested.max(1));
        if returned >= global_limit {
            if !found.is_empty() {
                truncated = true;
                break;
            }
            continue;
        }
        if found.len() > per_page_limit || found.len() > remaining {
            truncated = true;
            found.truncate(per_page_limit.min(remaining));
        }
        if found.is_empty() {
            continue;
        }
        returned += found.len();
        output.push(PageMatchesDto {
            src: *src,
            matches: found
                .into_iter()
                .map(|(start, len, snippet)| MatchDto {
                    start,
                    len,
                    snippet,
                })
                .collect(),
        });
    }

    SearchQueryDto {
        pages: output,
        truncated,
        limit: global_limit as u32,
    }
}

impl SearchStore {
    pub fn new(budget: u64) -> Self {
        SearchStore {
            docs: HashMap::new(),
            budget,
            used: 0,
        }
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        // Preserve the earliest pages for deterministic prefix coverage.
        while self.used > self.budget {
            let victim = self
                .docs
                .iter()
                .flat_map(|(doc, index)| index.pages.keys().map(move |src| (*src, *doc)))
                .max();
            let Some((src, doc)) = victim else {
                self.used = 0;
                break;
            };
            if let Some(index) = self.docs.get_mut(&doc) {
                if let Some(entry) = index.pages.remove(&src) {
                    self.used = self.used.saturating_sub(entry.cost);
                    index.used = index.used.saturating_sub(entry.cost);
                    index.chars_indexed =
                        index.chars_indexed.saturating_sub(entry.char_count as u64);
                }
                index.truncated = true;
            }
        }
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Store compact search text for one page. Foreground text runs are kept
    /// by `TextLayoutCache`, not here.
    pub fn store_page(&mut self, doc: DocId, src: u32, raw: &str, char_count: u32) -> bool {
        self.store_entry(doc, src, PageEntry::new(raw, char_count))
    }

    /// `store_page` with the word gaps the page's geometry implies
    /// (`library::word_breaks`) inserted as spaces, as the library's text
    /// pass indexes it: run-together words are found as words, and a hit's
    /// offsets are still PDFium character indices.
    pub fn store_page_spaced(
        &mut self,
        doc: DocId,
        src: u32,
        raw: &str,
        char_count: u32,
        breaks: &[u32],
    ) -> bool {
        self.store_entry(doc, src, PageEntry::new_spaced(raw, char_count, breaks))
    }

    /// Store an OCR'd page's text in place of its text layer's
    /// (`ocr_page`). Its hits are highlighted through `ocr_geometry`.
    pub fn store_ocr_page(
        &mut self,
        doc: DocId,
        src: u32,
        text: &str,
        geometry: OcrGeometry,
    ) -> bool {
        self.store_entry(doc, src, PageEntry::new_ocr(text, geometry))
    }

    /// The line boxes of a page whose index holds OCR text, if it does.
    pub fn ocr_geometry(&self, doc: DocId, src: u32) -> Option<Arc<OcrGeometry>> {
        self.docs
            .get(&doc)?
            .pages
            .get(&src)
            .and_then(|entry| entry.ocr.clone())
    }

    fn store_entry(&mut self, doc: DocId, src: u32, entry: PageEntry) -> bool {
        let entry = Arc::new(entry);
        let old_cost = self
            .docs
            .get(&doc)
            .and_then(|index| index.pages.get(&src))
            .map(|old| old.cost)
            .unwrap_or(0);
        let projected = self.used.saturating_sub(old_cost) + entry.cost;
        if projected > self.budget {
            // The budget is shared by every open document. Reclaim from
            // whichever document is hogging it rather than letting whoever
            // filled it first permanently starve the others.
            self.reclaim_for(doc, projected - self.budget);
        }
        let index = self.docs.entry(doc).or_default();
        let old_cost = index.pages.get(&src).map(|old| old.cost).unwrap_or(0);
        if self.used.saturating_sub(old_cost) + entry.cost > self.budget {
            index.truncated = true;
            return false;
        }
        if let Some(old) = index.pages.insert(src, Arc::clone(&entry)) {
            self.used = self.used.saturating_sub(old.cost);
            index.used = index.used.saturating_sub(old.cost);
            index.chars_indexed = index.chars_indexed.saturating_sub(old.char_count as u64);
        }
        self.used += entry.cost;
        index.used += entry.cost;
        index.chars_indexed += entry.char_count as u64;
        true
    }

    /// Free at least `need` bytes on behalf of `doc` by trimming whichever
    /// document currently holds the most, dropping its highest page indices
    /// first (search degrades to prefix coverage, matching `set_budget`).
    /// Stops once `doc` itself is the largest holder — a document never robs
    /// others to grow past them, so the steady state is an even split.
    fn reclaim_for(&mut self, doc: DocId, need: u64) {
        let mut freed = 0u64;
        while freed < need {
            let Some((victim_doc, _)) = self
                .docs
                .iter()
                .filter(|(candidate, index)| **candidate != doc && !index.pages.is_empty())
                .map(|(candidate, index)| (*candidate, index.used))
                .max_by_key(|(_, used)| *used)
            else {
                return;
            };
            let own_used = self.docs.get(&doc).map(|index| index.used).unwrap_or(0);
            let victim_used = self
                .docs
                .get(&victim_doc)
                .map(|index| index.used)
                .unwrap_or(0);
            if victim_used <= own_used {
                return;
            }
            let Some(index) = self.docs.get_mut(&victim_doc) else {
                return;
            };
            let Some(src) = index.pages.keys().max().copied() else {
                return;
            };
            if let Some(entry) = index.pages.remove(&src) {
                self.used = self.used.saturating_sub(entry.cost);
                index.used = index.used.saturating_sub(entry.cost);
                index.chars_indexed = index.chars_indexed.saturating_sub(entry.char_count as u64);
                freed += entry.cost;
            }
            // The victim no longer holds a complete index; its own chain will
            // observe this and stop rather than fight for the space back.
            index.truncated = true;
        }
    }

    pub fn contains_page(&self, doc: DocId, src: u32) -> bool {
        self.docs
            .get(&doc)
            .is_some_and(|index| index.pages.contains_key(&src))
    }

    pub fn indexed_count(&self, doc: DocId) -> u32 {
        self.docs
            .get(&doc)
            .map(|index| index.pages.len() as u32)
            .unwrap_or(0)
    }

    pub fn chars_indexed(&self, doc: DocId) -> u64 {
        self.docs
            .get(&doc)
            .map(|index| index.chars_indexed)
            .unwrap_or(0)
    }

    pub fn is_truncated(&self, doc: DocId) -> bool {
        self.docs.get(&doc).is_some_and(|index| index.truncated)
    }

    pub fn snapshot(&self, doc: DocId) -> Vec<(u32, Arc<PageEntry>)> {
        let Some(index) = self.docs.get(&doc) else {
            return Vec::new();
        };
        let mut pages: Vec<(u32, Arc<PageEntry>)> = index
            .pages
            .iter()
            .map(|(src, entry)| (*src, Arc::clone(entry)))
            .collect();
        pages.sort_by_key(|(src, _)| *src);
        pages
    }

    pub fn query(
        &self,
        doc: DocId,
        query: &str,
        case_sensitive: bool,
        per_page_limit: usize,
    ) -> Vec<PageMatchesDto> {
        query_pages(
            &self.snapshot(doc),
            query,
            case_sensitive,
            per_page_limit,
            usize::MAX,
        )
        .pages
    }

    pub fn query_limited(
        &self,
        doc: DocId,
        query: &str,
        case_sensitive: bool,
        per_page_limit: usize,
        global_limit: usize,
    ) -> SearchQueryDto {
        query_pages(
            &self.snapshot(doc),
            query,
            case_sensitive,
            per_page_limit,
            global_limit,
        )
    }

    pub fn remove_doc(&mut self, doc: DocId) {
        if let Some(index) = self.docs.remove(&doc) {
            for entry in index.pages.into_values() {
                self.used = self.used.saturating_sub(entry.cost);
            }
        }
    }
}

#[derive(Clone)]
pub struct TextLayoutData {
    pub runs: Arc<Vec<TextRun>>,
    pub boxes: Arc<Vec<[f32; 4]>>,
    /// Fillable-looking glyphs on the page (see `engine::bubbles`). Cached
    /// here rather than on their own because they are derived from exactly
    /// the character stream this entry already paid to extract — finding them
    /// separately would mean reading the page twice to learn the same thing.
    pub bubbles: Arc<Vec<BubbleDto>>,
    pub char_count: u32,
}

struct LayoutEntry {
    data: TextLayoutData,
    cost: u64,
    last_used: u64,
}

/// Recent foreground page geometry. This is independent of search coverage so
/// selection/highlighting never become an empty successful response merely
/// because the document-wide index reached its memory budget.
pub struct TextLayoutCache {
    pages: HashMap<(DocId, u32), LayoutEntry>,
    budget: u64,
    used: u64,
    tick: u64,
}

impl TextLayoutCache {
    pub fn new(budget: u64) -> Self {
        TextLayoutCache {
            pages: HashMap::new(),
            budget,
            used: 0,
            tick: 0,
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.evict_to(budget);
    }

    pub fn get(&mut self, doc: DocId, src: u32) -> Option<TextLayoutData> {
        self.tick += 1;
        let entry = self.pages.get_mut(&(doc, src))?;
        entry.last_used = self.tick;
        Some(entry.data.clone())
    }

    pub fn insert(
        &mut self,
        doc: DocId,
        src: u32,
        runs: Vec<TextRun>,
        boxes: Vec<[f32; 4]>,
        bubbles: Vec<BubbleDto>,
        char_count: u32,
    ) -> TextLayoutData {
        let runs = Arc::new(runs);
        let boxes = Arc::new(boxes);
        let bubbles = Arc::new(bubbles);
        let cost = 64
            + (boxes.len() * std::mem::size_of::<[f32; 4]>()) as u64
            + (bubbles.len() * std::mem::size_of::<BubbleDto>()) as u64
            + runs
                .iter()
                .map(|run| std::mem::size_of::<TextRun>() as u64 + run.text.len() as u64)
                .sum::<u64>();
        let data = TextLayoutData {
            runs,
            boxes,
            bubbles,
            char_count,
        };
        if cost > self.budget {
            return data;
        }
        if let Some(old) = self.pages.remove(&(doc, src)) {
            self.used = self.used.saturating_sub(old.cost);
        }
        self.evict_to(self.budget.saturating_sub(cost));
        self.tick += 1;
        self.pages.insert(
            (doc, src),
            LayoutEntry {
                data: data.clone(),
                cost,
                last_used: self.tick,
            },
        );
        self.used += cost;
        data
    }

    pub fn remove_doc(&mut self, doc: DocId) {
        let keys: Vec<(DocId, u32)> = self
            .pages
            .keys()
            .filter(|(candidate, _)| *candidate == doc)
            .copied()
            .collect();
        for key in keys {
            if let Some(entry) = self.pages.remove(&key) {
                self.used = self.used.saturating_sub(entry.cost);
            }
        }
    }

    fn evict_to(&mut self, target: u64) {
        while self.used > target && !self.pages.is_empty() {
            let victim = self
                .pages
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);
            let Some(key) = victim else {
                break;
            };
            if let Some(entry) = self.pages.remove(&key) {
                self.used = self.used.saturating_sub(entry.cost);
            }
        }
    }
}

pub mod gramindex;
pub mod librarytext;
pub mod trigram;

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &str, query: &str, case_sensitive: bool) -> Vec<(u32, u32, String)> {
        let entry = PageEntry::new(text, text.chars().count() as u32);
        let prepared = prepare_query(query, case_sensitive);
        find_prepared_matches(&entry, &prepared, case_sensitive, 100)
    }

    #[test]
    fn normalize_folds_ligatures_via_nfkc() {
        let normalized = normalize("ﬁle");
        assert_eq!(normalized.text, "file");
        assert_eq!(normalized.map.raw_index(0), 0);
        assert_eq!(normalized.map.raw_index(1), 0);
        assert_eq!(normalized.map.raw_index(2), 1);
    }

    #[test]
    fn geometric_word_gaps_become_spaces_that_map_to_raw_indices() {
        let raw = "ATheoryof Saddle";
        // Breaks before "T", "o", and the "S" a real space already precedes.
        let entry = PageEntry::new_spaced(raw, raw.chars().count() as u32, &[1, 7, 10]);
        assert_eq!(entry.indexed_parts().0, "A Theory of Saddle");
        let found =
            find_prepared_matches(&entry, &prepare_query("theory of saddle", false), false, 10);
        assert_eq!(found.len(), 1);
        // Raw indices 1..16: "Theoryof Saddle" in the stream as PDFium has it.
        assert_eq!((found[0].0, found[0].1), (1, 15));
        let found = find_prepared_matches(&entry, &prepare_query("A", true), true, 10);
        assert_eq!((found[0].0, found[0].1), (0, 1));
    }

    #[test]
    fn a_run_together_line_is_found_in_an_open_document_at_raw_offsets() {
        let mut store = SearchStore::new(1_000_000);
        // PDFium's stream for a LaTeX line whose spaces are only geometry.
        let raw = "Wepresentanewmethod for saddle escape";
        // Breaks before "present", "a", "new", "method".
        assert!(store.store_page_spaced(4, 0, raw, raw.chars().count() as u32, &[2, 9, 10, 13]));
        let found = store.query(4, "a new method", false, 10);
        assert_eq!(found.len(), 1);
        let hit = &found[0].matches[0];
        // "anewmethod": raw indices 9..19, which is what gets highlighted.
        assert_eq!((hit.start, hit.len), (9, 10));
        assert_eq!(&raw[9..19], "anewmethod");
        assert!(store.ocr_geometry(4, 0).is_none());
    }

    fn ocr_line(text: &str, confidence: f32, bbox: [f32; 4]) -> OcrLine {
        OcrLine {
            text: text.into(),
            confidence,
            bbox,
        }
    }

    #[test]
    fn an_ocr_page_is_searchable_and_highlighted_from_its_line_boxes() {
        let lines = [
            ocr_line("Arthur-Merlin Games:", 0.9, [0.1, 0.1, 0.5, 0.02]),
            ocr_line("~", 0.1, [0.9, 0.9, 0.01, 0.01]),
            ocr_line("a Randomized Proof System", 0.8, [0.2, 0.2, 0.6, 0.025]),
        ];
        let (w, h) = (600.0, 800.0);
        let (text, geometry) = ocr_page(&lines, w, h);
        assert_eq!(
            text,
            ocr::page_text(&lines),
            "the same text the library indexes"
        );

        let mut store = SearchStore::new(1_000_000);
        assert!(store.store_ocr_page(9, 3, &text, geometry));
        let found = store.query(9, "games: a randomized", false, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].src, 3);
        let hit = &found[0].matches[0];
        // Offsets index the OCR text, across its line break.
        assert_eq!(hit.start, text.find("Games").unwrap() as u32);

        let rects = store
            .ocr_geometry(9, 3)
            .expect("an OCR page")
            .match_rects(hit.start, hit.len);
        assert_eq!(rects.len(), 2, "one rect per line the hit spans");
        // "Games:" is the last 6 of 20 characters of the first line's box,
        // which sits 0.1 down a page of height 800, in display space (y-up).
        let [x, y, rw, rh] = rects[0];
        assert!((x - (60.0 + 300.0 * 14.0 / 20.0)).abs() < 0.01, "{x}");
        assert!((rw - 300.0 * 6.0 / 20.0).abs() < 0.01, "{rw}");
        assert!((y - (1.0 - 0.1 - 0.02) * h).abs() < 0.01, "{y}");
        assert!((rh - 0.02 * h).abs() < 0.01);
        // "a Randomized" starts the second line's box.
        let [x2, y2, w2, _] = rects[1];
        assert!((x2 - 120.0).abs() < 0.01);
        assert!((w2 - 360.0 * 12.0 / 25.0).abs() < 0.01, "{w2}");
        assert!(y2 < y, "the second line is lower on the page");
    }

    #[test]
    fn normalize_collapses_whitespace_runs() {
        assert_eq!(normalize("hello \t\n  world").text, "hello world");
    }

    #[test]
    fn normalize_straightens_curly_quotes() {
        assert_eq!(normalize("it’s “fine”").text, "it's \"fine\"");
    }

    #[test]
    fn finds_case_insensitive_matches_with_pdfium_ranges() {
        let found = matches("The Quick brown the END", "the", false);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, 0);
        assert_eq!(found[0].1, 3);
        assert_eq!(found[1].0, 16);
    }

    #[test]
    fn case_sensitive_toggle_respects_case() {
        let found = matches("The Quick brown the END", "the", true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 16);
    }

    #[test]
    fn query_spanning_collapsed_whitespace_matches() {
        let found = matches("hello   \n world", "hello world", false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 0);
        assert_eq!(found[0].1, 15);
    }

    #[test]
    fn ligature_text_matches_plain_query() {
        let found = matches("my ﬁle here", "file", false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 3);
        assert_eq!(found[0].1, 3);
    }

    #[test]
    fn snippets_surround_the_match() {
        let found = matches("aaaa needle bbbb", "needle", false);
        assert!(found[0].2.contains("needle"));
    }

    #[test]
    fn store_respects_budget_and_reports_truncation() {
        let mut store = SearchStore::new(200);
        assert!(store.store_page(1, 0, "small", 5));
        let big = "x".repeat(10_000);
        assert!(!store.store_page(1, 1, &big, 10_000));
        assert!(store.is_truncated(1));
        assert_eq!(store.indexed_count(1), 1);
        assert_eq!(store.chars_indexed(1), 5);
        store.remove_doc(1);
        assert_eq!(store.used(), 0);
        assert_eq!(store.chars_indexed(1), 0);
    }

    #[test]
    fn a_second_document_reclaims_budget_from_the_one_that_filled_it() {
        // The budget is shared by every open tab. Whoever indexes first must
        // not be able to lock every later document out of search entirely.
        let mut store = SearchStore::new(4_000);
        let page = "lorem ipsum dolor sit amet ".repeat(8);
        let mut src = 0;
        while store.store_page(1, src, &page, page.len() as u32) {
            src += 1;
        }
        assert!(store.is_truncated(1), "doc 1 should have filled the budget");
        let doc_one_pages = store.indexed_count(1);
        assert!(doc_one_pages > 2, "need room for a meaningful split");

        assert!(
            store.store_page(2, 0, &page, page.len() as u32),
            "doc 2 must be able to index despite a full budget"
        );
        assert!(store.store_page(2, 1, &page, page.len() as u32));

        assert!(store.used() <= store.budget(), "hard bound still holds");
        assert!(
            store.indexed_count(1) < doc_one_pages,
            "space came out of the largest holder"
        );
        // Reclaim drops the highest page indices, so prefix coverage survives.
        assert!(store.contains_page(1, 0));
    }

    #[test]
    fn reclaim_stops_once_the_requesting_document_is_the_largest_holder() {
        // Otherwise two documents would trade pages back and forth forever
        // instead of settling on a share each.
        let mut store = SearchStore::new(4_000);
        let page = "lorem ipsum dolor sit amet ".repeat(8);
        let mut src = 0;
        while store.store_page(1, src, &page, page.len() as u32) {
            src += 1;
        }
        let mut src = 0;
        while store.store_page(2, src, &page, page.len() as u32) {
            src += 1;
        }
        let one = store.indexed_count(1);
        let two = store.indexed_count(2);
        assert!(one > 0 && two > 0, "both documents keep a share");
        assert!(
            one.abs_diff(two) <= 1,
            "shares should settle roughly even, got {one} vs {two}"
        );
        assert!(store.used() <= store.budget());
    }

    #[test]
    fn shrinking_budget_evicts_high_pages_to_restore_the_hard_bound() {
        let mut store = SearchStore::new(10_000);
        assert!(store.store_page(1, 0, "first page", 10));
        let one_page_budget = store.used();
        assert!(store.store_page(1, 1, "second page", 11));
        assert!(store.used() > one_page_budget);

        store.set_budget(one_page_budget);

        assert!(store.used() <= one_page_budget);
        assert!(store.contains_page(1, 0));
        assert!(!store.contains_page(1, 1));
        assert!(store.is_truncated(1));
    }

    #[test]
    fn query_returns_page_ordered_results_from_indexed_subset() {
        let mut store = SearchStore::new(1_000_000);
        store.store_page(1, 2, "beta beta", 9);
        store.store_page(1, 0, "alpha beta", 10);
        let result = store.query(1, "beta", false, 10);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].src, 0);
        assert_eq!(result[1].src, 2);
        assert_eq!(result[1].matches.len(), 2);
    }

    #[test]
    fn compact_index_covers_the_evaluators_thousand_page_text_shape() {
        let mut store = SearchStore::new(16 * 1024 * 1024);
        let body = format!(
            "SpeedyF contract page NEEDLE\n{}\n",
            "ordinary agreement text ".repeat(195)
        );
        assert!(body.len() >= 4_500);

        for src in 0..1_000 {
            assert!(
                store.store_page(7, src, &body, body.chars().count() as u32),
                "page {src} should fit in the compact index"
            );
        }

        assert_eq!(store.indexed_count(7), 1_000);
        assert!(!store.is_truncated(7));
        assert!(store.used() <= store.budget());
    }

    #[test]
    fn query_enforces_a_document_wide_result_limit() {
        let mut store = SearchStore::new(1_000_000);
        for src in 0..10 {
            assert!(store.store_page(3, src, "needle needle", 13));
        }

        let result = store.query_limited(3, "needle", false, 200, 3);

        assert_eq!(
            result
                .pages
                .iter()
                .map(|page| page.matches.len())
                .sum::<usize>(),
            3
        );
        assert!(result.truncated);
    }

    #[test]
    fn foreground_text_layout_cache_is_lru_and_independent_of_search_coverage() {
        let mut cache = TextLayoutCache::new(500);
        cache.insert(1, 10, vec![], vec![[0.0, 0.0, 10.0, 10.0]; 10], vec![], 10);
        cache.insert(1, 11, vec![], vec![[0.0, 0.0, 10.0, 10.0]; 10], vec![], 10);
        assert!(cache.get(1, 10).is_some());

        cache.insert(1, 12, vec![], vec![[0.0, 0.0, 10.0, 10.0]; 10], vec![], 10);

        assert!(cache.get(1, 10).is_some(), "recent layout survives");
        assert!(cache.get(1, 11).is_none(), "least-recent layout is evicted");
        assert!(cache.get(1, 12).is_some());
    }
}

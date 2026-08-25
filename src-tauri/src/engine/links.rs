//! Structured PDF link extraction. Annotation links take precedence over
//! PDFium's text-derived web links and explicit DOI/arXiv text detections.

use super::citation::{find_explicit_citations, parse_citation_id};
use super::render;
use super::text;
use super::types::{DocId, LinkDto, LinkTarget};
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_DEST, FPDF_DOCUMENT, FPDF_LINK, FPDF_PAGE, FPDF_TEXTPAGE, FS_RECTF,
};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;

const ACTION_GOTO: std::os::raw::c_ulong = 1;
const ACTION_URI: std::os::raw::c_ulong = 3;
const MAX_LINK_STRING: usize = 1024 * 1024;
const RUNAWAY_URI_MIN_RECTS: usize = 12;
const RUNAWAY_URI_MIN_PAGE_SPAN: f32 = 0.45;

/// One of PDFium's text-derived web links: where it sits in the page's
/// character stream, and the URL it read there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebLinkSpan {
    pub start: u32,
    pub len: u32,
    pub uri: String,
}

/// Characters that can end a sentence but not a URL.
const TRAILING_PUNCTUATION: &[char] = &['.', ',', ';', ':', ')', ']', '}', '"', '\'', '>'];

/// Join web links that PDFium split in the middle of one URL.
///
/// PDFium's detector stops at characters it does not treat as part of a URL, so
/// a single address containing, say, a parenthesis or a comma comes back as two
/// or three separate links — each hoverable, each opening a fragment. Spans
/// that are adjacent in the character stream with no whitespace between them
/// were never two URLs: nothing in real prose puts two addresses flush against
/// each other.
///
/// The merged link's URL is rebuilt from the page text across the whole span
/// rather than concatenating what PDFium reported, because the pieces it
/// reports are already normalised and may not join back cleanly.
pub fn merge_adjacent_web_links(mut spans: Vec<WebLinkSpan>, raw: &[char]) -> Vec<WebLinkSpan> {
    spans.sort_by_key(|span| span.start);
    let mut out: Vec<WebLinkSpan> = Vec::with_capacity(spans.len());

    for span in spans {
        let Some(previous) = out.last_mut() else {
            out.push(span);
            continue;
        };
        let previous_end = previous.start + previous.len;
        let between = raw
            .get(previous_end as usize..span.start as usize)
            .unwrap_or_default();
        let contiguous = span.start >= previous.start
            && span.start <= previous_end
            && !between.iter().any(|ch| ch.is_whitespace());
        // A URL printed in a bibliography wraps across lines, and PDFium reads
        // each line as its own link — "https://arxiv.org/" then "abs/1312.6120".
        // The break between them is whitespace, so the contiguous rule alone
        // leaves both halves, and neither resolves to anything. Joining across
        // a break is only allowed when the first half is visibly unfinished and
        // the second is not a URL in its own right.
        let wrapped = span.start >= previous_end
            && between.iter().all(|ch| ch.is_whitespace())
            && ends_mid_url(&previous.uri)
            && !starts_a_url(raw, span.start, span.len);
        let joins = contiguous || wrapped;

        if !joins {
            out.push(span);
            continue;
        }
        let end = previous_end.max(span.start + span.len);
        previous.len = end - previous.start;
        if let Some(text) = text_of(raw, previous.start, previous.len) {
            previous.uri = text;
        }
    }
    out
}

/// Whether a URL looks cut off rather than finished.
///
/// A wrap lands on a separator far more often than not, because that is where
/// typesetting prefers to break a long address — and an address ending in one
/// is not an address anybody meant to publish.
fn ends_mid_url(uri: &str) -> bool {
    let Some(rest) = uri
        .strip_prefix("https://")
        .or_else(|| uri.strip_prefix("http://"))
    else {
        return false;
    };
    match rest.split_once('/') {
        // Nothing after the host: "https://arxiv.org/" or "https://arxiv.org".
        None => true,
        Some((_, path)) => {
            path.is_empty() || path.ends_with(['/', '.', '-', '_', ':', '=', '?', '&'])
        }
    }
}

/// Whether a span begins a URL of its own, which a continuation never does.
fn starts_a_url(raw: &[char], start: u32, len: u32) -> bool {
    let text: String = raw
        .get(start as usize..(start + len) as usize)
        .unwrap_or_default()
        .iter()
        .collect();
    let lower = text.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")
}

/// The page text under a span, with trailing sentence punctuation removed.
///
/// A closing bracket is only punctuation when nothing opened it. URLs
/// legitimately end in one — a Wikipedia article with a disambiguator is the
/// standard example — so stripping every trailing `)` would break exactly the
/// addresses this merge exists to reassemble.
fn text_of(raw: &[char], start: u32, len: u32) -> Option<String> {
    let slice = raw.get(start as usize..(start + len) as usize)?;
    // Whitespace inside a span only ever comes from a line wrap, and a URL
    // never contains any, so it is removed rather than preserved.
    let mut text: String = slice.iter().filter(|ch| !ch.is_whitespace()).collect();
    while let Some(last) = text.chars().last() {
        if !TRAILING_PUNCTUATION.contains(&last) {
            break;
        }
        if let Some(opener) = matching_opener(last) {
            let body = &text[..text.len() - last.len_utf8()];
            if body.matches(opener).count() > body.matches(last).count() {
                break; // this bracket closes something
            }
        }
        text.truncate(text.len() - last.len_utf8());
    }
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn matching_opener(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

fn read_uri(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    action: pdfium_render::prelude::FPDF_ACTION,
) -> Option<String> {
    let needed = b.FPDFAction_GetURIPath(doc, action, std::ptr::null_mut(), 0) as usize;
    if needed <= 1 || needed > MAX_LINK_STRING {
        return None;
    }
    let mut bytes = vec![0_u8; needed];
    let written = b.FPDFAction_GetURIPath(
        doc,
        action,
        bytes.as_mut_ptr().cast::<c_void>(),
        bytes.len() as _,
    ) as usize;
    if written == 0 {
        return None;
    }
    bytes.truncate(written.min(bytes.len()));
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8(bytes).ok().filter(|uri| !uri.is_empty())
}

fn read_web_url(
    b: &dyn PdfiumLibraryBindings,
    links: pdfium_render::prelude::FPDF_PAGELINK,
    index: i32,
) -> Option<String> {
    let needed = b.FPDFLink_GetURL(links, index, std::ptr::null_mut(), 0);
    if needed <= 1 || needed as usize > MAX_LINK_STRING / 2 {
        return None;
    }
    let mut units = vec![0_u16; needed as usize];
    let written = b.FPDFLink_GetURL(links, index, units.as_mut_ptr(), units.len() as i32);
    if written <= 0 {
        return None;
    }
    units.truncate((written as usize).min(units.len()));
    if units.last() == Some(&0) {
        units.pop();
    }
    let value = String::from_utf16_lossy(&units);
    (!value.is_empty()).then_some(value)
}

fn resolve_dest(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    dest: FPDF_DEST,
    page_count: u32,
) -> Option<LinkTarget> {
    if dest.is_null() {
        return None;
    }
    let page = b.FPDFDest_GetDestPageIndex(doc, dest);
    if page < 0 || page as u32 >= page_count {
        return None;
    }
    let (mut has_x, mut has_y, mut has_zoom) = (0, 0, 0);
    let (mut x, mut y, mut zoom) = (0.0_f32, 0.0_f32, 0.0_f32);
    b.FPDFDest_GetLocationInPage(
        dest,
        &mut has_x,
        &mut has_y,
        &mut has_zoom,
        &mut x,
        &mut y,
        &mut zoom,
    );
    Some(LinkTarget::Internal {
        page: page as u32,
        x: (has_x != 0).then_some(x),
        y: (has_y != 0).then_some(y),
    })
}

fn annotation_target(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    link: FPDF_LINK,
    page_count: u32,
) -> LinkTarget {
    let action = b.FPDFLink_GetAction(link);
    if !action.is_null() {
        match b.FPDFAction_GetType(action) {
            ACTION_GOTO => {
                return resolve_dest(b, doc, b.FPDFAction_GetDest(doc, action), page_count)
                    .unwrap_or(LinkTarget::Unknown);
            }
            ACTION_URI => {
                return read_uri(b, doc, action)
                    .map(|uri| LinkTarget::Uri {
                        citation: parse_citation_id(&uri),
                        uri,
                    })
                    .unwrap_or(LinkTarget::Unknown);
            }
            _ => return LinkTarget::Unknown,
        }
    }
    resolve_dest(b, doc, b.FPDFLink_GetDest(doc, link), page_count).unwrap_or(LinkTarget::Unknown)
}

fn raw_text(b: &dyn PdfiumLibraryBindings, text_page: FPDF_TEXTPAGE) -> String {
    let count = b.FPDFText_CountChars(text_page).max(0);
    let mut raw = String::with_capacity(count as usize);
    for index in 0..count {
        raw.push(char::from_u32(b.FPDFText_GetUnicode(text_page, index)).unwrap_or(' '));
    }
    raw
}

fn normalized_link_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn text_overlapping_rect(raw: &str, boxes: &[[f32; 4]], rect: [f32; 4]) -> String {
    let [left, bottom, width, height] = rect;
    let right = left + width;
    let top = bottom + height;
    raw.chars()
        .zip(boxes.iter())
        .filter_map(|(ch, [x, y, w, h])| {
            (*w > 0.0 && *h > 0.0 && *x + *w > left && *x < right && *y + *h > bottom && *y < top)
                .then_some(ch)
        })
        .collect()
}

/// Some PDF producers emit a single URI annotation as one rectangle for
/// every intervening text line when a link scope is accidentally left open.
/// PDFium faithfully enumerates those rectangles, which would otherwise turn
/// most of a page into hover targets. Only intervene for an extreme same-URI
/// group spanning nearly half a page; then retain rectangles whose visible
/// text is actually part of the URI. If the link uses an arbitrary label,
/// preserve its narrowest text-bearing endpoint as a conservative fallback.
fn sanitize_runaway_uri_annotations(
    links: Vec<LinkDto>,
    raw: &str,
    boxes: &[[f32; 4]],
    page_h: f32,
) -> Vec<LinkDto> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, link) in links.iter().enumerate() {
        if let LinkTarget::Uri { uri, .. } = &link.target {
            groups.entry(uri.clone()).or_default().push(index);
        }
    }

    let mut keep = vec![true; links.len()];
    for (uri, indices) in groups {
        if indices.len() < RUNAWAY_URI_MIN_RECTS {
            continue;
        }
        let bottom = indices
            .iter()
            .map(|index| links[*index].rect[1])
            .min_by(f32::total_cmp)
            .unwrap_or(0.0);
        let top = indices
            .iter()
            .map(|index| links[*index].rect[1] + links[*index].rect[3])
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);
        if top - bottom < page_h.max(1.0) * RUNAWAY_URI_MIN_PAGE_SPAN {
            continue;
        }

        let uri_text = normalized_link_text(&uri);
        let text_candidates: Vec<(usize, String)> = indices
            .iter()
            .filter_map(|index| {
                let text =
                    normalized_link_text(&text_overlapping_rect(raw, boxes, links[*index].rect));
                (!text.is_empty()).then_some((*index, text))
            })
            .collect();
        if text_candidates.is_empty() {
            continue;
        }

        let matching: Vec<usize> = text_candidates
            .iter()
            .filter(|(_, text)| {
                text.len() >= 5 && (uri_text.contains(text) || text.contains(&uri_text))
            })
            .map(|(index, _)| *index)
            .collect();
        let retained = if matching.is_empty() {
            text_candidates
                .iter()
                .min_by(|(left_index, left_text), (right_index, right_text)| {
                    links[*left_index].rect[2]
                        .total_cmp(&links[*right_index].rect[2])
                        .then_with(|| left_text.len().cmp(&right_text.len()))
                })
                .map(|(index, _)| vec![*index])
                .unwrap_or_default()
        } else {
            matching
        };

        for index in indices {
            keep[index] = retained.contains(&index);
        }
    }

    links
        .into_iter()
        .zip(keep)
        .filter_map(|(link, keep)| keep.then_some(link))
        .collect()
}

fn intersection_area(left: [f32; 4], right: [f32; 4]) -> f32 {
    let x0 = left[0].max(right[0]);
    let y0 = left[1].max(right[1]);
    let x1 = (left[0] + left[2]).min(right[0] + right[2]);
    let y1 = (left[1] + left[3]).min(right[1] + right[3]);
    (x1 - x0).max(0.0) * (y1 - y0).max(0.0)
}

fn intersection_over_union(left: [f32; 4], right: [f32; 4]) -> f32 {
    let intersection = intersection_area(left, right);
    if intersection <= 0.0 {
        return 0.0;
    }
    let union = left[2] * left[3] + right[2] * right[3] - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// How much of a new link may sit inside an existing one before it is treated
/// as part of it rather than as a link of its own.
const CONTAINED_IN_EXISTING: f32 = 0.60;

fn push_deduplicated(links: &mut Vec<LinkDto>, link: LinkDto) {
    if links.iter().any(|existing| {
        // Same link, seen twice.
        intersection_over_union(existing.rect, link.rect) > 0.60
            // Or: this one sits inside a link already found. Annotation links
            // are pushed first and carry the URL the author encoded; PDFium's
            // text scanner then reads the *printed* URL and, where it wraps or
            // hits punctuation, stops early — yielding "https://arxiv.org/"
            // over the first part of an address already covered in full.
            // Those fragments overlap the real link far too little for an
            // intersection-over-union test to catch, because they are a small
            // piece of it rather than a near-duplicate of it.
            || contained_fraction(link.rect, existing.rect) > CONTAINED_IN_EXISTING
    }) {
        return;
    }
    links.push(link);
}

/// What share of `inner` lies within `outer`.
fn contained_fraction(inner: [f32; 4], outer: [f32; 4]) -> f32 {
    let area = inner[2] * inner[3];
    if area <= 0.0 {
        return 0.0;
    }
    intersection_area(inner, outer) / area
}

fn normalize_destination(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    target: LinkTarget,
    mappers: &mut HashMap<u32, Option<text::DispMapper>>,
) -> LinkTarget {
    let LinkTarget::Internal { page, x, y } = target else {
        return target;
    };
    let (Some(raw_x), Some(raw_y)) = (x, y) else {
        return LinkTarget::Internal { page, x, y };
    };
    let mapper = if let Some(mapper) = mappers.get(&page) {
        *mapper
    } else {
        let mapper = render::page_display_size(b, doc, page as i32).and_then(|[width, height]| {
            let target_page = b.FPDF_LoadPage(doc, page as i32);
            if target_page.is_null() {
                return None;
            }
            let mapper = text::DispMapper::new(b, target_page, width, height);
            b.FPDF_ClosePage(target_page);
            Some(mapper)
        });
        mappers.insert(page, mapper);
        mapper
    };
    let Some(mapper) = mapper else {
        return LinkTarget::Internal { page, x, y };
    };
    let (display_x, display_y) = mapper.map(raw_x as f64, raw_y as f64);
    LinkTarget::Internal {
        page,
        x: Some(display_x),
        y: Some(display_y),
    }
}

pub fn extract_links(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    page: FPDF_PAGE,
    text_page: FPDF_TEXTPAGE,
    display_size_pt: [f32; 2],
    char_boxes: &[[f32; 4]],
    page_count: u32,
) -> Vec<LinkDto> {
    let [display_w, display_h] = display_size_pt;
    let mapper = text::DispMapper::new(b, page, display_w, display_h);
    let mut destination_mappers = HashMap::new();
    let mut out = Vec::new();
    let page_raw = (!text_page.is_null()).then(|| raw_text(b, text_page));

    // Annotation-derived entries go first so overlap de-duplication always
    // retains their more authoritative target.
    let mut position = 0;
    loop {
        let mut link: FPDF_LINK = std::ptr::null_mut();
        if b.FPDFLink_Enumerate(page, &mut position, &mut link) == 0 {
            break;
        }
        if link.is_null() {
            continue;
        }
        let mut rect = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        if b.FPDFLink_GetAnnotRect(link, &mut rect) == 0 {
            continue;
        }
        let (x, y, w, h) = mapper.map_box(
            rect.left as f64,
            rect.right as f64,
            rect.bottom as f64,
            rect.top as f64,
        );
        let display_rect = [x, y, w, h];
        if display_rect[2] <= 0.0 || display_rect[3] <= 0.0 {
            continue;
        }
        out.push(LinkDto {
            rect: display_rect,
            target: normalize_destination(
                b,
                doc,
                annotation_target(b, doc, link, page_count),
                &mut destination_mappers,
            ),
        });
    }
    out = sanitize_runaway_uri_annotations(
        out,
        page_raw.as_deref().unwrap_or_default(),
        char_boxes,
        display_h,
    );

    let web_links = (!text_page.is_null()).then(|| b.FPDFLink_LoadWebLinks(text_page));
    if let Some(web_links) = web_links.filter(|links| !links.is_null()) {
        let count = b.FPDFLink_CountWebLinks(web_links).max(0);
        // Collected before use so links PDFium split mid-URL can be rejoined;
        // see `merge_adjacent_web_links`.
        let mut spans = Vec::new();
        for index in 0..count {
            let Some(uri) = read_web_url(b, web_links, index) else {
                continue;
            };
            let (mut start, mut len) = (0, 0);
            if b.FPDFLink_GetTextRange(web_links, index, &mut start, &mut len) == 0
                || start < 0
                || len <= 0
            {
                continue;
            }
            spans.push(WebLinkSpan {
                start: start as u32,
                len: len as u32,
                uri,
            });
        }
        let raw_chars: Vec<char> = page_raw.as_deref().unwrap_or_default().chars().collect();
        for span in merge_adjacent_web_links(spans, &raw_chars) {
            let WebLinkSpan { start, len, uri } = span;
            let target = LinkTarget::Uri {
                citation: parse_citation_id(&uri),
                uri,
            };
            for rect in text::merge_match_rects(char_boxes, start, len) {
                push_deduplicated(
                    &mut out,
                    LinkDto {
                        rect,
                        target: target.clone(),
                    },
                );
            }
        }
        b.FPDFLink_CloseWebLinks(web_links);
    }

    let Some(page_raw) = page_raw else {
        return out;
    };
    for occurrence in find_explicit_citations(&page_raw) {
        for rect in text::merge_match_rects(char_boxes, occurrence.start, occurrence.len) {
            let value = match &occurrence.id {
                super::types::CitationIdDto::Doi(value) => format!("doi:{value}"),
                super::types::CitationIdDto::ArXiv(value) => format!("arXiv:{value}"),
            };
            push_deduplicated(
                &mut out,
                LinkDto {
                    rect,
                    target: LinkTarget::Uri {
                        uri: value,
                        citation: Some(occurrence.id.clone()),
                    },
                },
            );
        }
    }
    out
}

struct CacheEntry {
    links: Arc<Vec<LinkDto>>,
    cost: u64,
    last_used: u64,
}

pub struct LinkCache {
    entries: HashMap<(DocId, u32), CacheEntry>,
    budget: u64,
    used: u64,
    tick: u64,
}

impl LinkCache {
    pub fn new(budget: u64) -> Self {
        Self {
            entries: HashMap::new(),
            budget,
            used: 0,
            tick: 0,
        }
    }

    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.evict_to(budget);
    }

    pub fn get(&mut self, doc: DocId, src: u32) -> Option<Arc<Vec<LinkDto>>> {
        self.tick += 1;
        let entry = self.entries.get_mut(&(doc, src))?;
        entry.last_used = self.tick;
        Some(Arc::clone(&entry.links))
    }

    pub fn insert(&mut self, doc: DocId, src: u32, links: Vec<LinkDto>) -> Arc<Vec<LinkDto>> {
        let links = Arc::new(links);
        let cost = 64
            + links.len() as u64 * 64
            + links
                .iter()
                .map(|link| match &link.target {
                    LinkTarget::Uri { uri, .. } => uri.len() as u64,
                    _ => 0,
                })
                .sum::<u64>();
        if cost > self.budget {
            return links;
        }
        if let Some(old) = self.entries.remove(&(doc, src)) {
            self.used = self.used.saturating_sub(old.cost);
        }
        self.evict_to(self.budget.saturating_sub(cost));
        self.tick += 1;
        self.entries.insert(
            (doc, src),
            CacheEntry {
                links: Arc::clone(&links),
                cost,
                last_used: self.tick,
            },
        );
        self.used += cost;
        links
    }

    pub fn remove_doc(&mut self, doc: DocId) {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|(candidate, _)| *candidate == doc)
            .copied()
            .collect();
        for key in keys {
            if let Some(entry) = self.entries.remove(&key) {
                self.used = self.used.saturating_sub(entry.cost);
            }
        }
    }

    fn evict_to(&mut self, target: u64) {
        while self.used > target && !self.entries.is_empty() {
            let victim = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);
            let Some(key) = victim else {
                break;
            };
            if let Some(entry) = self.entries.remove(&key) {
                self.used = self.used.saturating_sub(entry.cost);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{pdfium_init, render};
    use pdfium_render::prelude::Pdfium;
    use std::io::Write;

    fn fixture_pdf() -> Vec<u8> {
        let content = "BT /F1 12 Tf 50 700 Td (doi:10.1145/1234567) Tj ET";
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 8 0 R >> >> /Contents 7 0 R /Annots [5 0 R 6 0 R] >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 9 0 R >>".to_string(),
            "<< /Type /Annot /Subtype /Link /Rect [10 20 100 40] /Dest [4 0 R /XYZ 30 700 null] >>".to_string(),
            "<< /Type /Annot /Subtype /Link /Rect [120 20 260 40] /A << /S /URI /URI (https://doi.org/10.1145/1234567) >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
            "<< /Length 0 >>\nstream\n\nendstream".to_string(),
        ];
        let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(&mut bytes, "{} 0 obj\n{}\nendobj", index + 1, object).unwrap();
        }
        let xref = bytes.len();
        writeln!(&mut bytes, "xref\n0 {}", objects.len() + 1).unwrap();
        writeln!(&mut bytes, "0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(&mut bytes, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            &mut bytes,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .unwrap();
        bytes
    }

    fn uri_link(uri: &str, rect: [f32; 4]) -> LinkDto {
        LinkDto {
            rect,
            target: LinkTarget::Uri {
                uri: uri.into(),
                citation: None,
            },
        }
    }

    fn append_boxed_line(raw: &mut String, boxes: &mut Vec<[f32; 4]>, text: &str, y: f32) {
        for (column, ch) in text.chars().enumerate() {
            raw.push(ch);
            boxes.push([20.0 + column as f32 * 5.0, y, 5.0, 10.0]);
        }
        raw.push('\n');
        boxes.push([0.0; 4]);
    }

    #[test]
    fn runaway_same_uri_annotations_keep_only_the_visible_uri_fragment() {
        let uri = "https://github.com/alphanso-org/alphanso";
        let mut raw = String::new();
        let mut boxes = Vec::new();
        let mut links = Vec::new();
        for line in 0..12 {
            let y = 700.0 - line as f32 * 50.0;
            let text = if line == 11 {
                "org/alphanso"
            } else {
                "ordinary body text that must not become a link"
            };
            append_boxed_line(&mut raw, &mut boxes, text, y);
            links.push(uri_link(
                uri,
                [20.0, y, if line == 11 { 60.0 } else { 420.0 }, 10.0],
            ));
        }

        let sanitized = sanitize_runaway_uri_annotations(links, &raw, &boxes, 792.0);

        assert_eq!(sanitized.len(), 1);
        assert_eq!(sanitized[0].rect, [20.0, 150.0, 60.0, 10.0]);
    }

    #[test]
    fn ordinary_wrapped_uri_annotations_are_not_filtered() {
        let links = vec![
            uri_link("https://example.test/long/path", [20.0, 80.0, 200.0, 10.0]),
            uri_link("https://example.test/long/path", [20.0, 60.0, 80.0, 10.0]),
        ];
        assert_eq!(
            sanitize_runaway_uri_annotations(links.clone(), "", &[], 792.0),
            links
        );
    }

    #[test]
    fn repeated_internal_references_are_never_treated_as_runaway_uris() {
        let links: Vec<_> = (0..20)
            .map(|line| LinkDto {
                rect: [20.0, 700.0 - line as f32 * 30.0, 80.0, 10.0],
                target: LinkTarget::Internal {
                    page: 4,
                    x: None,
                    y: None,
                },
            })
            .collect();
        assert_eq!(
            sanitize_runaway_uri_annotations(links.clone(), "", &[], 792.0),
            links
        );
    }

    #[test]
    fn overlap_deduplication_keeps_first_annotation_entry() {
        let first = LinkDto {
            rect: [10.0, 10.0, 100.0, 20.0],
            target: LinkTarget::Internal {
                page: 1,
                x: None,
                y: None,
            },
        };
        let second = LinkDto {
            rect: [11.0, 10.0, 99.0, 20.0],
            target: LinkTarget::Uri {
                uri: "https://example.test".into(),
                citation: None,
            },
        };
        let mut links = vec![first.clone()];
        push_deduplicated(&mut links, second);
        assert_eq!(links, vec![first]);
    }

    #[test]
    fn link_cache_is_budgeted_and_document_scoped() {
        let mut cache = LinkCache::new(256);
        let make = |uri: &str| LinkDto {
            rect: [0.0, 0.0, 10.0, 10.0],
            target: LinkTarget::Uri {
                uri: uri.into(),
                citation: None,
            },
        };
        cache.insert(1, 0, vec![make("a")]);
        cache.insert(2, 0, vec![make("b")]);
        cache.remove_doc(1);
        assert!(cache.get(1, 0).is_none());
        assert!(cache.get(2, 0).is_some());
    }

    #[test]
    fn extracts_goto_and_uri_annotations_from_a_pdf_fixture() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"));
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("links.pdf");
        std::fs::write(&path, fixture_pdf()).expect("write PDF fixture");
        let bindings = pdfium.bindings();
        let document = render::open_document(bindings, path.to_str().unwrap(), None)
            .expect("open PDF fixture");
        let page = bindings.FPDF_LoadPage(document, 0);
        assert!(!page.is_null());
        let size = render::page_display_size(bindings, document, 0).expect("page size");
        let extracted = text::extract_page(bindings, page, size[0], size[1]);
        let text_page = bindings.FPDFText_LoadPage(page);
        let links = extract_links(
            bindings,
            document,
            page,
            text_page,
            size,
            &extracted.boxes,
            2,
        );
        bindings.FPDFText_ClosePage(text_page);
        bindings.FPDF_ClosePage(page);
        bindings.FPDF_CloseDocument(document);

        assert!(links.iter().any(|link| matches!(
            link.target,
            LinkTarget::Internal {
                page: 1,
                x: Some(x),
                y: Some(y),
            } if (x - 30.0).abs() < 0.1 && (y - 700.0).abs() < 0.1
        )));
        assert!(links.iter().any(|link| matches!(
            &link.target,
            LinkTarget::Uri {
                citation: Some(super::super::types::CitationIdDto::Doi(value)),
                ..
            } if value == "10.1145/1234567"
        )));
        let goto = links
            .iter()
            .find(|link| matches!(link.target, LinkTarget::Internal { .. }))
            .expect("GoTo annotation");
        assert!((goto.rect[0] - 10.0).abs() < 0.2);
        assert!((goto.rect[1] - 20.0).abs() < 0.2);
    }
}

#[cfg(test)]
mod weblink_tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn span(start: u32, len: u32, uri: &str) -> WebLinkSpan {
        WebLinkSpan {
            start,
            len,
            uri: uri.into(),
        }
    }

    /// The bug: PDFium stops at a character it does not treat as part of a URL,
    /// so one address comes back as two hoverable links opening fragments.
    #[test]
    fn joins_a_url_split_at_punctuation() {
        let raw = chars("see https://example.com/a(b)c now");
        let merged = merge_adjacent_web_links(
            vec![span(4, 22, "https://example.com/a("), span(26, 3, "b)c")],
            &raw,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].uri, "https://example.com/a(b)c");
        assert_eq!((merged[0].start, merged[0].len), (4, 25));
    }

    /// Two genuinely separate links have whitespace between them.
    #[test]
    fn leaves_separate_urls_alone() {
        let raw = chars("https://a.example https://b.example");
        let merged = merge_adjacent_web_links(
            vec![
                span(0, 17, "https://a.example"),
                span(18, 17, "https://b.example"),
            ],
            &raw,
        );
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn joins_three_pieces_of_one_url() {
        let raw = chars("https://e.com/a(b)c,d");
        let merged = merge_adjacent_web_links(
            vec![
                span(0, 15, "https://e.com/a"),
                span(15, 3, "(b)"),
                span(18, 3, "c,d"),
            ],
            &raw,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].uri, "https://e.com/a(b)c,d");
    }

    /// A URL ending a sentence must not swallow the full stop.
    /// A URL ending a sentence must not swallow the full stop — but must keep
    /// a bracket that closes one it opened.
    #[test]
    fn drops_trailing_sentence_punctuation_but_keeps_balanced_brackets() {
        let raw = chars("at https://e.com/a(b).");
        let merged = merge_adjacent_web_links(
            vec![span(3, 16, "https://e.com/a("), span(19, 3, "b).")],
            &raw,
        );
        assert_eq!(merged[0].uri, "https://e.com/a(b)");
    }

    #[test]
    fn drops_an_unmatched_closing_bracket() {
        let raw = chars("(see https://e.com/a)");
        let merged =
            merge_adjacent_web_links(vec![span(5, 15, "https://e.com/a"), span(20, 1, ")")], &raw);
        assert_eq!(merged[0].uri, "https://e.com/a");
    }

    #[test]
    fn a_single_link_is_untouched() {
        let raw = chars("https://example.com");
        let merged = merge_adjacent_web_links(vec![span(0, 19, "https://example.com")], &raw);
        assert_eq!(merged, vec![span(0, 19, "https://example.com")]);
    }

    #[test]
    fn handles_spans_arriving_out_of_order() {
        let raw = chars("https://e.com/ab");
        let merged =
            merge_adjacent_web_links(vec![span(14, 2, "ab"), span(0, 14, "https://e.com/")], &raw);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].uri, "https://e.com/ab");
    }

    /// Geometry that does not line up with the text must not panic or invent
    /// a URL out of nothing.
    #[test]
    fn tolerates_spans_past_the_end_of_the_text() {
        let raw = chars("short");
        let merged = merge_adjacent_web_links(vec![span(0, 3, "abc"), span(3, 99, "def")], &raw);
        assert_eq!(merged.len(), 1);
        // Rebuild failed, so the first piece's URL stands rather than nothing.
        assert_eq!(merged[0].uri, "abc");
    }

    /// The real case: a bibliography prints the URL and it wraps, so PDFium
    /// reads each line as its own link and neither half resolves to anything.
    #[test]
    fn joins_a_url_wrapped_across_a_line() {
        let raw = chars("https://arxiv.org/\nabs/1312.6120");
        let merged = merge_adjacent_web_links(
            vec![
                span(0, 18, "https://arxiv.org/"),
                span(19, 13, "abs/1312.6120"),
            ],
            &raw,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].uri, "https://arxiv.org/abs/1312.6120");
    }

    #[test]
    fn joins_a_doi_wrapped_after_the_prefix() {
        let raw = chars("http://dx.doi.org/10.\n1073/pnas.1806579115");
        let merged = merge_adjacent_web_links(
            vec![
                span(0, 21, "http://dx.doi.org/10."),
                span(22, 20, "1073/pnas.1806579115"),
            ],
            &raw,
        );
        assert_eq!(merged[0].uri, "http://dx.doi.org/10.1073/pnas.1806579115");
    }

    /// Two complete URLs on consecutive lines are two URLs.
    #[test]
    fn does_not_join_two_finished_urls_across_a_break() {
        let raw = chars("https://a.example/x\nhttps://b.example/y");
        let merged = merge_adjacent_web_links(
            vec![
                span(0, 19, "https://a.example/x"),
                span(20, 19, "https://b.example/y"),
            ],
            &raw,
        );
        assert_eq!(merged.len(), 2);
    }

    /// A finished URL followed by ordinary prose must not swallow the prose.
    #[test]
    fn does_not_join_when_the_first_url_is_complete() {
        let raw = chars("https://e.com/paper\nand then some words");
        let merged = merge_adjacent_web_links(
            vec![span(0, 19, "https://e.com/paper"), span(20, 3, "and")],
            &raw,
        );
        assert_eq!(merged.len(), 2);
    }

    /// A wrap where the next line is itself a URL is two links, however
    /// unfinished the first looks.
    #[test]
    fn does_not_join_an_unfinished_url_to_another_url() {
        let raw = chars("https://a.example/\nhttps://b.example/y");
        let merged = merge_adjacent_web_links(
            vec![
                span(0, 18, "https://a.example/"),
                span(19, 19, "https://b.example/y"),
            ],
            &raw,
        );
        assert_eq!(merged.len(), 2);
    }

    fn uri_link(rect: [f32; 4], uri: &str) -> LinkDto {
        LinkDto {
            rect,
            target: LinkTarget::Uri {
                uri: uri.into(),
                citation: None,
            },
        }
    }

    /// The real failure: PDFium reads the *printed* URL in a bibliography and
    /// stops where it wraps, producing "https://arxiv.org/" over the first part
    /// of an address the annotation already covers in full. The fragment is a
    /// small piece of the real link, so intersection-over-union is nowhere near
    /// a match and it survived as a second, broken link.
    #[test]
    fn drops_a_fragment_sitting_inside_a_link_already_found() {
        let mut links = vec![uri_link(
            [72.0, 700.0, 160.0, 10.0],
            "https://arxiv.org/abs/1312.6120",
        )];
        push_deduplicated(
            &mut links,
            uri_link([72.0, 700.0, 40.0, 10.0], "https://arxiv.org/"),
        );
        assert_eq!(links.len(), 1);
        assert!(
            matches!(&links[0].target, LinkTarget::Uri { uri, .. } if uri.ends_with("1312.6120"))
        );
    }

    #[test]
    fn still_drops_a_near_duplicate_of_the_same_link() {
        let mut links = vec![uri_link([72.0, 700.0, 100.0, 10.0], "https://e.com/a")];
        push_deduplicated(
            &mut links,
            uri_link([73.0, 700.0, 99.0, 10.0], "https://e.com/a"),
        );
        assert_eq!(links.len(), 1);
    }

    /// Two links on the same line must both survive: a reference list puts
    /// several addresses side by side.
    #[test]
    fn keeps_a_separate_link_beside_an_existing_one() {
        let mut links = vec![uri_link([72.0, 700.0, 60.0, 10.0], "https://a.example")];
        push_deduplicated(
            &mut links,
            uri_link([140.0, 700.0, 60.0, 10.0], "https://b.example"),
        );
        assert_eq!(links.len(), 2);
    }

    /// Barely touching is not containment.
    #[test]
    fn keeps_a_link_that_only_grazes_another() {
        let mut links = vec![uri_link([72.0, 700.0, 60.0, 10.0], "https://a.example")];
        push_deduplicated(
            &mut links,
            uri_link([128.0, 700.0, 60.0, 10.0], "https://b.example"),
        );
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn a_zero_area_link_is_not_treated_as_contained() {
        assert_eq!(
            contained_fraction([72.0, 700.0, 0.0, 0.0], [0.0, 0.0, 500.0, 500.0]),
            0.0
        );
    }

    #[test]
    fn no_links_is_empty() {
        assert!(merge_adjacent_web_links(Vec::new(), &chars("text")).is_empty());
    }
}

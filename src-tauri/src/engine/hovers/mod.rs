//! Text-based hover previews: cross-references ("Eq. (12)", "Fig. 3",
//! "Theorem 2.1"), footnote marks, and terms an extension contributes
//! (Notation's acronyms).
//!
//! The page text is read once per page into a `page::Page`, in document
//! order, by an idle chain (`IndexBuilder`): cross-references need every
//! definition in the document before any mention can be resolved, and a
//! bibliography or a part boundary seen on one page applies to the pages
//! after it. A 1000-page book is read a few pages per job at prefetch
//! priority, so rendering never waits behind it. The finished index is
//! kept per open document and, by content fingerprint, for a few closed
//! ones - a reopened file, or a second tab on it, costs nothing.
//!
//! Spans are what the frontend draws: one hover layer per page, each span
//! a list of rectangles (a mention broken across lines has several), a
//! kind, an accessible label, and a target. Where the document's own link
//! annotations already cover a span, the span is dropped - the link has a
//! preview of its own, and there must never be two.
//!
//! Everything but `extract` is PDFium-free.

pub mod extract;
pub mod footnotes;
pub mod page;
pub mod regions;
pub mod terms;
pub mod xref;

use page::Page;
use regions::{classify, RegionState};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use terms::TermInput;
use xref::{Definition, Kind, Mention};

/// `[x, y, w, h]`, y-up page space.
pub type Rect = [f32; 4];

/// Pages indexed for cross-references. A longer document gets footnotes and
/// terms on every page, cross-references within these.
pub const MAX_INDEX_PAGES: u32 = 2_000;

/// One definition a term occurrence points at.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TermEntryDto {
    pub title: String,
    pub body: String,
    /// The page in the file where it is defined.
    pub page: u32,
    pub rect: Option<Rect>,
}

/// Where a span points.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoverTargetDto {
    /// The page in the file.
    pub page: u32,
    pub x: Option<f32>,
    /// The top of the target, where a click lands.
    pub y: f32,
    /// What the preview shows, rendered from the page; absent for a term,
    /// whose preview is its text.
    pub region: Option<Rect>,
    /// A footnote that runs on onto the next page.
    pub continues: bool,
    /// A term's definitions.
    pub terms: Vec<TermEntryDto>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoverSpanDto {
    /// "equation", "figure", "table", "algorithm", "section", "theorem",
    /// "footnote" or "term".
    pub kind: &'static str,
    pub rects: Vec<Rect>,
    /// For screen readers: "Equation 12", "Footnote 3", "FDT".
    pub label: String,
    pub target: HoverTargetDto,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageHoversDto {
    pub src: u32,
    pub spans: Vec<HoverSpanDto>,
}

/// A finished index: every page's cross-reference and footnote spans, and
/// the region state each page starts in (whether it opens inside a
/// bibliography), for reading terms later.
#[derive(Debug, Default)]
pub struct IndexResult {
    pub spans: HashMap<u32, Vec<HoverSpanDto>>,
    pub starts: Vec<RegionState>,
    pub indexed: u32,
}

impl IndexResult {
    pub fn region_start(&self, src: u32) -> RegionState {
        self.starts.get(src as usize).cloned().unwrap_or_default()
    }
}

/// Reads a document page by page, in order.
pub struct IndexBuilder {
    total: u32,
    next: u32,
    state: RegionState,
    starts: Vec<RegionState>,
    definitions: Vec<Definition>,
    mentions: Vec<Mention>,
    footnotes: HashMap<u32, Vec<HoverSpanDto>>,
    /// The page before, kept until the next one says whether its last
    /// footnote runs on.
    previous: Option<(u32, Page)>,
}

impl IndexBuilder {
    pub fn new(total: u32) -> IndexBuilder {
        IndexBuilder {
            total: total.min(MAX_INDEX_PAGES),
            next: 0,
            state: RegionState::default(),
            starts: Vec::new(),
            definitions: Vec::new(),
            mentions: Vec::new(),
            footnotes: HashMap::new(),
            previous: None,
        }
    }

    /// The next page to read, or None when every page has been read.
    pub fn next_page(&self) -> Option<u32> {
        (self.next < self.total).then_some(self.next)
    }

    /// Add the next page; `None` for a page that could not be read, which
    /// contributes nothing and breaks no state.
    pub fn add_page(&mut self, page: Option<Page>) {
        let src = self.next;
        self.next += 1;
        self.starts.push(self.state.clone());
        let Some(mut page) = page else {
            self.flush_previous(None);
            return;
        };
        let regions = classify(&mut page, &mut self.state);
        let definitions = xref::find_definitions(&page, &regions, src);
        let mentions = xref::find_mentions(&page, &regions, src);
        self.mentions.extend(
            mentions
                .into_iter()
                .filter(|m| !xref::is_definition_site(m, &definitions)),
        );
        self.definitions.extend(definitions);
        self.flush_previous(Some(&page));
        self.previous = Some((src, page));
    }

    fn flush_previous(&mut self, next: Option<&Page>) {
        if let Some((src, page)) = self.previous.take() {
            let spans = footnote_spans(&page, next, src);
            if !spans.is_empty() {
                self.footnotes.insert(src, spans);
            }
        }
    }

    pub fn finish(mut self) -> IndexResult {
        self.flush_previous(None);
        let mut by_label: HashMap<(Kind, &str), Vec<Definition>> = HashMap::new();
        for d in &self.definitions {
            by_label
                .entry((d.kind, d.label.as_str()))
                .or_default()
                .push(d.clone());
        }
        let mut spans: HashMap<u32, Vec<HoverSpanDto>> = HashMap::new();
        for m in &self.mentions {
            let Some(candidates) = by_label.get(&(m.kind, m.label.as_str())) else {
                continue;
            };
            let Some(d) = xref::resolve(m, candidates) else {
                continue;
            };
            if m.rects.is_empty() {
                continue;
            }
            spans.entry(m.page).or_default().push(HoverSpanDto {
                kind: m.kind.wire(),
                rects: m.rects.clone(),
                label: xref::describe(m.kind, &m.label),
                target: HoverTargetDto {
                    page: d.page,
                    x: Some(d.jump.0),
                    y: d.jump.1,
                    region: Some(d.region),
                    continues: false,
                    terms: Vec::new(),
                },
            });
        }
        for (src, notes) in self.footnotes {
            let page_spans = spans.entry(src).or_default();
            for note in notes {
                if !page_spans
                    .iter()
                    .any(|s| any_overlap(&s.rects, &note.rects))
                {
                    page_spans.push(note);
                }
            }
        }
        IndexResult {
            spans,
            starts: self.starts,
            indexed: self.next,
        }
    }
}

fn footnote_spans(page: &Page, next: Option<&Page>, src: u32) -> Vec<HoverSpanDto> {
    footnotes::find_footnotes(page, next)
        .into_iter()
        .filter(|f| !f.rects.is_empty())
        .map(|f| HoverSpanDto {
            kind: "footnote",
            rects: f.rects,
            label: format!("Footnote {}", f.mark),
            target: HoverTargetDto {
                page: src,
                x: Some(f.jump.0),
                y: f.jump.1,
                region: Some(f.region),
                continues: f.continues,
                terms: Vec::new(),
            },
        })
        .collect()
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if w <= 0.0 || h <= 0.0 {
        0.0
    } else {
        w * h
    }
}

fn any_overlap(a: &[Rect], b: &[Rect]) -> bool {
    a.iter()
        .any(|ra| b.iter().any(|rb| overlap_area(*ra, *rb) > 0.0))
}

/// Whether a link annotation covers a span: a third of the span's area, or
/// of the link's, is under the other.
fn covered_by_link(span: &HoverSpanDto, links: &[Rect]) -> bool {
    span.rects.iter().any(|r| {
        links.iter().any(|l| {
            let shared = overlap_area(*r, *l);
            let smaller = (r[2] * r[3]).min(l[2] * l[3]).max(0.01);
            shared >= 0.33 * smaller
        })
    })
}

/// A page's spans for the frontend: the indexed ones, footnotes for a page
/// past the index, and term occurrences - minus anything a link annotation
/// already covers.
pub fn page_spans(
    src: u32,
    indexed: Option<&IndexResult>,
    page: Option<(&Page, Option<&Page>)>,
    terms: &[TermInput],
    links: &[Rect],
) -> Vec<HoverSpanDto> {
    let mut spans: Vec<HoverSpanDto> = indexed
        .and_then(|r| r.spans.get(&src))
        .cloned()
        .unwrap_or_default();
    let beyond = indexed.is_none_or(|r| src >= r.indexed);
    if let Some((page, next)) = page {
        if beyond {
            spans.extend(footnote_spans(page, next, src));
        }
        for hit in terms::find_terms(page, src, terms) {
            if spans.iter().any(|s| any_overlap(&s.rects, &hit.rects)) {
                continue;
            }
            let entries: Vec<TermEntryDto> = hit
                .definitions
                .iter()
                .map(|&i| {
                    let t = &terms[i];
                    TermEntryDto {
                        title: t.title.clone(),
                        body: t.body.clone(),
                        page: t.target_page,
                        rect: t.target_rect,
                    }
                })
                .collect();
            let Some(first) = entries.first() else {
                continue;
            };
            let (x, y) = first
                .rect
                .map(|r| (Some(r[0]), r[1] + r[3]))
                .unwrap_or((None, f32::MAX));
            spans.push(HoverSpanDto {
                kind: "term",
                rects: hit.rects,
                label: terms[hit.definitions[0]].text.clone(),
                target: HoverTargetDto {
                    page: first.page,
                    x,
                    y: if y.is_finite() && y < f32::MAX {
                        y
                    } else {
                        0.0
                    },
                    region: None,
                    continues: false,
                    terms: entries,
                },
            });
        }
    }
    spans.retain(|s| !covered_by_link(s, links));
    spans
}

/// Read a page for terms or footnotes on request: the page, classified from
/// the region state the index recorded for it.
pub fn prepare_page(mut page: Page, start: RegionState) -> Page {
    let mut state = start;
    classify(&mut page, &mut state);
    page
}

/// Pages a document keeps read for terms, so moving between a few pages
/// does not read them again.
const MODEL_CACHE_PAGES: usize = 6;

/// One open document's hover state, kept by the engine thread.
#[derive(Default)]
pub struct DocHovers {
    /// The index being read, while it is.
    pub builder: Option<IndexBuilder>,
    pub result: Option<Arc<IndexResult>>,
    /// Page requests that arrived before the index was finished.
    pub waiting: Vec<(u32, crate::engine::Respond<PageHoversDto>)>,
    pub terms: Vec<TermInput>,
    /// The content fingerprint the index will be remembered under.
    pub fingerprint: Option<String>,
    models: Vec<(u32, Arc<Page>)>,
}

impl DocHovers {
    pub fn model(&mut self, src: u32) -> Option<Arc<Page>> {
        let at = self.models.iter().position(|(s, _)| *s == src)?;
        let entry = self.models.remove(at);
        let page = entry.1.clone();
        self.models.push(entry);
        Some(page)
    }

    pub fn keep_model(&mut self, src: u32, page: Arc<Page>) {
        self.models.retain(|(s, _)| *s != src);
        self.models.push((src, page));
        while self.models.len() > MODEL_CACHE_PAGES {
            self.models.remove(0);
        }
    }

    /// The document is gone: answer everyone still waiting.
    pub fn fail_waiting(self) {
        for (_, respond) in self.waiting {
            respond(Err(crate::errors::AppError::DocumentNotFound));
        }
    }
}

/// Finished indexes of recently open files, by content fingerprint.
static RECENT: std::sync::Mutex<Vec<(String, Arc<IndexResult>)>> =
    std::sync::Mutex::new(Vec::new());
const RECENT_CAP: usize = 6;

pub fn recall(fingerprint: &str) -> Option<Arc<IndexResult>> {
    let mut recent = RECENT.lock().ok()?;
    let at = recent.iter().position(|(f, _)| f == fingerprint)?;
    let entry = recent.remove(at);
    let result = entry.1.clone();
    recent.push(entry);
    Some(result)
}

pub fn remember(fingerprint: String, result: Arc<IndexResult>) {
    let Ok(mut recent) = RECENT.lock() else {
        return;
    };
    recent.retain(|(f, _)| *f != fingerprint);
    recent.push((fingerprint, result));
    while recent.len() > RECENT_CAP {
        recent.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::page::build::*;
    use super::*;

    fn paper() -> Vec<Page> {
        // Page 0: a mention of Fig. 1 and Eq. (2), and a footnote.
        let mut b = Builder::new();
        for i in 0..6 {
            b.line(
                700.0 - 12.0 * i as f32,
                BODY,
                "Running text of the paper goes on here.",
            );
        }
        b.line(620.0, BODY, "As Fig. 1 shows, and from Eq. (2) we get it.");
        let end = b.text(72.0, 608.0, BODY, "A remark with a note");
        b.newline();
        let after = b.sup(end, 608.0, "1");
        b.newline();
        b.text(after + 2.0, 608.0, BODY, " attached to it here.");
        b.newline();
        let after = b.sup(72.0, 90.0, "1");
        b.text(after, 90.0, SMALL, "The note.");
        b.newline();
        let p0 = b.page();
        // Page 1: the figure's caption and the equation.
        let mut b = Builder::new();
        b.line(700.0, BODY, "More text before the figure caption.");
        let x = b.text(72.0, 500.0, BOLD, "Figure 1:");
        b.text(x, 500.0, BODY, " A caption.");
        b.newline();
        b.line(480.0, BODY, "The relation is");
        b.text(200.0, 460.0, MATH, "a=b+c");
        b.text(500.0, 460.0, BODY, "(2)");
        b.newline();
        b.line(440.0, BODY, "which ends the page.");
        vec![p0, b.page()]
    }

    #[test]
    fn a_document_resolves_mentions_and_footnotes() {
        let mut builder = IndexBuilder::new(2);
        for page in paper() {
            builder.add_page(Some(page));
        }
        assert_eq!(builder.next_page(), None);
        let result = builder.finish();
        let spans = &result.spans[&0];
        let kinds: Vec<&str> = spans.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec!["figure", "equation", "footnote"]);
        assert_eq!(spans[0].target.page, 1);
        assert_eq!(spans[0].label, "Figure 1");
        assert_eq!(spans[1].target.page, 1);
        assert_eq!(spans[2].target.page, 0);
        // the caption's own label is not a span
        assert!(!result.spans.contains_key(&1));
    }

    #[test]
    fn pathology_d_spans_under_a_link_annotation_defer_to_it() {
        let mut builder = IndexBuilder::new(2);
        for page in paper() {
            builder.add_page(Some(page));
        }
        let result = builder.finish();
        let figure = result.spans[&0][0].rects[0];
        let spans = page_spans(0, Some(&result), None, &[], &[figure]);
        assert!(spans.iter().all(|s| s.kind != "figure"));
        assert_eq!(spans.len(), 2);
        // A footnote mark that is itself a link (hyperref's footnotes).
        let mark = result.spans[&0][2].rects[0];
        let spans = page_spans(0, Some(&result), None, &[], &[mark]);
        assert!(spans.iter().all(|s| s.kind != "footnote"));
    }

    #[test]
    fn pathology_h_an_ocr_text_layer_guesses_nothing() {
        // One font for everything, as an OCR layer has: no theorem label or
        // heading can stand apart, so none is read.
        let mut b = Builder::new();
        b.line(700.0, BODY, "Some recognised text of the page.");
        b.line(680.0, BODY, "Theorem 2. Every bounded sequence converges.");
        b.line(660.0, BODY, "3 Results");
        b.line(640.0, BODY, "By Theorem 2 and Section 3 we are done.");
        let mut builder = IndexBuilder::new(1);
        builder.add_page(Some(b.page()));
        let result = builder.finish();
        assert!(result.spans.is_empty());
    }

    #[test]
    fn pathology_j_a_page_that_failed_to_read_defines_nothing() {
        let mut builder = IndexBuilder::new(2);
        let mut pages = paper();
        pages.truncate(1);
        builder.add_page(pages.pop());
        builder.add_page(None);
        let result = builder.finish();
        assert!(result.spans[&0].iter().all(|s| s.kind == "footnote"));
    }

    #[test]
    fn pathology_i_huge_documents_index_a_bounded_prefix() {
        let builder = IndexBuilder::new(70_000);
        assert_eq!(builder.total, MAX_INDEX_PAGES);
    }

    #[test]
    fn indexes_are_recalled_by_fingerprint() {
        let result = Arc::new(IndexResult::default());
        remember("abc-1".into(), result.clone());
        assert!(recall("abc-1").is_some());
        assert!(recall("abc-2").is_none());
        for i in 0..RECENT_CAP + 1 {
            remember(format!("other-{i}"), Arc::new(IndexResult::default()));
        }
        assert!(recall("abc-1").is_none());
    }

    #[test]
    fn terms_become_spans_with_their_entries() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "The FDT holds, see Fig. 9 too.");
        let page = b.page();
        let terms = vec![TermInput {
            text: "FDT".into(),
            case_sensitive: true,
            kind: None,
            title: "FDT".into(),
            body: "fluctuation-dissipation theorem".into(),
            target_page: 3,
            target_rect: Some([72.0, 500.0, 100.0, 10.0]),
        }];
        let spans = page_spans(0, None, Some((&page, None)), &terms, &[]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, "term");
        assert_eq!(spans[0].target.page, 3);
        assert_eq!(spans[0].target.y, 510.0);
        assert_eq!(
            spans[0].target.terms[0].body,
            "fluctuation-dissipation theorem"
        );
    }
}
#[cfg(test)]
mod eval;

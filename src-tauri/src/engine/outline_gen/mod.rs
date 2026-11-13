//! An outline for PDFs that ship none: generated from the document's own
//! headings (see `detect` for the rules), at idle priority, a slice of
//! pages per request so rendering always comes first.
//!
//! The reader asks only when the real outline is missing or trivial (one
//! entry, or every entry on the first page). Each request reads pages for
//! a few milliseconds and answers with how far it got; the caller asks
//! again until `done`. Large documents also get a provisional outline from
//! the pages read so far. The finished outline is cached by the file's
//! content fingerprint under the app data directory, so a document is read
//! once per version of its bytes: saving writes new bytes and so a new
//! outline. Nothing generated is ever written into the PDF.

pub mod detect;
pub mod extract;
pub mod normalize;
pub mod numbering;
pub mod toc;

#[cfg(test)]
mod eval;

use crate::engine::types::OutlineNodeDto;
use detect::{Doc, FontInfo, Heading};
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_DOCUMENT};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Bumped whenever the rules change what they produce, so a cached
/// outline from older rules is recomputed.
const CACHE_VERSION: u32 = 1;
/// Page reading per request.
const SLICE: Duration = Duration::from_millis(25);
/// Documents this long get a provisional outline while they are read, every
/// `PROVISIONAL_EVERY` pages.
const PROVISIONAL_FROM: u32 = 150;
const PROVISIONAL_EVERY: u32 = 100;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedOutlineDto {
    /// Whether every page has been read; false means ask again.
    pub done: bool,
    /// The outline: final when `done` (empty when no confident outline was
    /// found); otherwise a provisional one when it has just changed, and
    /// empty in between, so the caller redraws only when there is news.
    pub outline: Vec<OutlineNodeDto>,
    pub pages_done: u32,
    pub page_count: u32,
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    fingerprint: String,
    headings: Vec<Heading>,
}

fn cache_path(app_data_dir: &Path, fingerprint: &str) -> PathBuf {
    app_data_dir
        .join("outlines")
        .join(format!("{fingerprint}.json"))
}

fn load_cache(app_data_dir: &Path, fingerprint: &str) -> Option<Vec<Heading>> {
    let bytes = std::fs::read(cache_path(app_data_dir, fingerprint)).ok()?;
    let file: CacheFile = serde_json::from_slice(&bytes).ok()?;
    (file.version == CACHE_VERSION && file.fingerprint == fingerprint).then_some(file.headings)
}

/// Whole or not at all; a cache that cannot be written costs a recompute.
fn save_cache(app_data_dir: &Path, fingerprint: &str, headings: &[Heading]) {
    let target = cache_path(app_data_dir, fingerprint);
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = CacheFile {
        version: CACHE_VERSION,
        fingerprint: fingerprint.to_owned(),
        headings: headings.to_vec(),
    };
    let Ok(bytes) = serde_json::to_vec(&file) else {
        return;
    };
    let partial = target.with_extension("json.part");
    if std::fs::write(&partial, bytes).is_ok() {
        let _ = std::fs::rename(&partial, &target);
    }
}

/// Nest a flat, ordered list of levelled headings into the tree the
/// outline panel draws. A heading deeper than the one before it by more
/// than a level hangs directly under it.
pub fn nest(headings: &[Heading]) -> Vec<OutlineNodeDto> {
    fn node(h: &Heading) -> OutlineNodeDto {
        OutlineNodeDto {
            title: h.title.clone(),
            page: Some(h.page),
            y: h.y,
            children: Vec::new(),
        }
    }
    fn insert(nodes: &mut Vec<OutlineNodeDto>, levels: &mut Vec<u8>, h: &Heading) {
        // `levels` holds the level of the open node at each depth.
        while levels.last().is_some_and(|l| *l >= h.level) {
            levels.pop();
        }
        let mut list = nodes;
        for _ in 0..levels.len() {
            list = &mut list.last_mut().expect("open node").children;
        }
        list.push(node(h));
        levels.push(h.level);
    }
    let mut roots = Vec::new();
    let mut levels: Vec<u8> = Vec::new();
    for h in headings {
        insert(&mut roots, &mut levels, h);
    }
    roots
}

/// One document's generation, held by the worker between requests.
pub struct OutlineGen {
    doc: Doc,
    fonts: extract::FontTable,
    next: u32,
    fingerprint: Option<String>,
    result: Option<Vec<Heading>>,
    /// A provisional outline computed since the last answer, not yet sent.
    provisional: Option<Vec<Heading>>,
}

impl OutlineGen {
    pub fn new(
        b: &dyn PdfiumLibraryBindings,
        raw: FPDF_DOCUMENT,
        page_count: u32,
        path: &Path,
        app_data_dir: &Path,
    ) -> Self {
        let fingerprint = crate::library::content_fingerprint(path);
        let result = fingerprint
            .as_deref()
            .and_then(|f| load_cache(app_data_dir, f));
        OutlineGen {
            doc: Doc {
                pages: Vec::new(),
                fonts: Vec::new(),
                tagged: extract::is_tagged(b, raw),
                page_count,
            },
            fonts: extract::FontTable::default(),
            next: 0,
            fingerprint,
            result,
            provisional: None,
        }
    }

    /// Read pages for one slice; finish when they are all read.
    pub fn step(
        &mut self,
        b: &dyn PdfiumLibraryBindings,
        raw: FPDF_DOCUMENT,
        app_data_dir: &Path,
    ) -> GeneratedOutlineDto {
        let page_count = self.doc.page_count;
        if let Some(result) = &self.result {
            return GeneratedOutlineDto {
                done: true,
                outline: nest(result),
                pages_done: page_count,
                page_count,
            };
        }
        let started = Instant::now();
        while self.next < page_count {
            let page = extract::read_page(b, raw, self.next, &mut self.fonts, self.doc.tagged);
            self.doc.pages.push(page);
            self.next += 1;
            if self.next % PROVISIONAL_EVERY == 0 && page_count >= PROVISIONAL_FROM {
                self.sync_fonts();
                self.provisional = Some(detect::generate(&self.doc));
            }
            if started.elapsed() >= SLICE {
                break;
            }
        }
        if self.next < page_count {
            return GeneratedOutlineDto {
                done: false,
                outline: self.provisional.take().map_or_else(Vec::new, |h| nest(&h)),
                pages_done: self.next,
                page_count,
            };
        }
        self.sync_fonts();
        let headings = detect::generate(&self.doc);
        if let Some(fingerprint) = &self.fingerprint {
            save_cache(app_data_dir, fingerprint, &headings);
        }
        // The page text is no longer needed; only the answer is kept.
        self.doc.pages = Vec::new();
        let outline = nest(&headings);
        self.result = Some(headings);
        GeneratedOutlineDto {
            done: true,
            outline,
            pages_done: page_count,
            page_count,
        }
    }

    fn sync_fonts(&mut self) {
        self.doc.fonts = self
            .fonts
            .facts
            .iter()
            .map(|f| FontInfo {
                glyphless: f.glyphless,
            })
            .collect();
    }
}

/// Generate an outline for the file at `path` in one go, for evaluation:
/// the same reading and rules the reader uses, no cache.
pub fn generate_for_file(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    page_count: u32,
) -> Vec<Heading> {
    let mut fonts = extract::FontTable::default();
    let tagged = extract::is_tagged(b, raw);
    let pages = (0..page_count)
        .map(|i| extract::read_page(b, raw, i, &mut fonts, tagged))
        .collect();
    let doc = Doc {
        pages,
        fonts: fonts
            .facts
            .iter()
            .map(|f| FontInfo {
                glyphless: f.glyphless,
            })
            .collect(),
        tagged,
        page_count,
    };
    detect::generate(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(level: u8, title: &str) -> Heading {
        Heading {
            level,
            title: title.into(),
            page: 0,
            y: None,
        }
    }

    fn shape(nodes: &[OutlineNodeDto]) -> String {
        nodes
            .iter()
            .map(|n| {
                if n.children.is_empty() {
                    n.title.clone()
                } else {
                    format!("{}({})", n.title, shape(&n.children))
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    #[test]
    fn nests_by_level() {
        let tree = nest(&[
            h(0, "1"),
            h(1, "1.1"),
            h(2, "1.1.1"),
            h(1, "1.2"),
            h(0, "2"),
        ]);
        assert_eq!(shape(&tree), "1(1.1(1.1.1),1.2),2");
    }

    #[test]
    fn a_skipped_level_hangs_under_the_last_heading() {
        let tree = nest(&[h(1, "orphan"), h(0, "1"), h(2, "deep")]);
        assert_eq!(shape(&tree), "orphan,1(deep)");
    }

    #[test]
    fn cache_round_trips_and_checks_version_and_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let headings = vec![h(0, "Intro"), h(1, "Detail")];
        save_cache(dir.path(), "abc-123", &headings);
        assert_eq!(load_cache(dir.path(), "abc-123"), Some(headings));
        assert_eq!(load_cache(dir.path(), "other"), None);
        let path = cache_path(dir.path(), "abc-123");
        let stale = String::from_utf8(std::fs::read(&path).unwrap())
            .unwrap()
            .replace("\"version\":1", "\"version\":0");
        std::fs::write(&path, stale).unwrap();
        assert_eq!(load_cache(dir.path(), "abc-123"), None);
    }

    use crate::engine::pdfium_init;
    use pdfium_render::prelude::*;

    const BODY: &str = "We study the problem in a setting where observations arrive one at a time";

    /// A three-page article: Helvetica body text, Helvetica-Bold headings
    /// with space above them, a large title on the first page.
    fn article_pdf(pdfium: &Pdfium, path: &Path, headings: bool) {
        let mut document = pdfium.create_new_pdf().unwrap();
        let regular = document.fonts_mut().helvetica();
        let bold = document.fonts_mut().helvetica_bold();
        let pages: [&[&str]; 3] = [
            &["1 Introduction"],
            &["2 Methods", "2.1 Data"],
            &["3 Results", "References"],
        ];
        for (index, names) in pages.iter().enumerate() {
            // (text, bold, size, baseline) for every line of the page
            let mut lines: Vec<(&str, bool, f32, f32)> = Vec::new();
            let mut y = 780.0;
            if index == 0 {
                lines.push(("A Study of Things", true, 18.0, y));
                y -= 40.0;
            }
            for name in names.iter() {
                for _ in 0..12 {
                    lines.push((BODY, false, 10.0, y));
                    y -= 13.0;
                }
                y -= 14.0;
                if headings {
                    lines.push((name, true, 12.0, y));
                } else {
                    lines.push((BODY, false, 10.0, y));
                }
                y -= 22.0;
            }
            for _ in 0..8 {
                lines.push((BODY, false, 10.0, y));
                y -= 13.0;
            }
            let mut page = document
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::a4())
                .unwrap();
            for (text, is_bold, size, y) in lines {
                let font = if is_bold { bold } else { regular };
                let mut object =
                    PdfPageTextObject::new(&document, text, font, PdfPoints::new(size)).unwrap();
                object
                    .translate(PdfPoints::new(60.0), PdfPoints::new(y))
                    .unwrap();
                page.objects_mut().add_text_object(object).unwrap();
            }
        }
        document.save_to_file(path).unwrap();
    }

    fn run(
        gen: &mut OutlineGen,
        b: &dyn PdfiumLibraryBindings,
        raw: FPDF_DOCUMENT,
        dir: &Path,
    ) -> GeneratedOutlineDto {
        for _ in 0..1000 {
            let answer = gen.step(b, raw, dir);
            if answer.done {
                return answer;
            }
        }
        panic!("generation never finished");
    }

    fn titles(nodes: &[OutlineNodeDto]) -> Vec<String> {
        let mut out = Vec::new();
        for node in nodes {
            out.push(node.title.clone());
            out.extend(titles(&node.children));
        }
        out
    }

    /// The whole path on a real file: extraction, detection, nesting, a
    /// jump target at the heading, and the cache the second open reads.
    #[test]
    fn generates_and_caches_an_outline_for_a_real_pdf() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let b = pdfium.bindings();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("article.pdf");
        article_pdf(&pdfium, &file, true);
        let raw = b.FPDF_LoadDocument(file.to_str().unwrap(), None);
        assert!(!raw.is_null());
        let pages = b.FPDF_GetPageCount(raw) as u32;

        let mut gen = OutlineGen::new(b, raw, pages, &file, dir.path());
        let answer = run(&mut gen, b, raw, dir.path());
        assert_eq!(
            titles(&answer.outline),
            [
                "1 Introduction",
                "2 Methods",
                "2.1 Data",
                "3 Results",
                "References"
            ]
        );
        assert_eq!(answer.outline[1].children.len(), 1);
        let methods = &answer.outline[1];
        assert_eq!(methods.page, Some(1));
        let y = methods.y.expect("a jump target at the heading");
        assert!(y > 300.0 && y < 842.0, "{y}");

        // A second open of the same bytes reads the cache, not the pages.
        let mut again = OutlineGen::new(b, raw, pages, &file, dir.path());
        assert!(again.result.is_some());
        assert_eq!(
            titles(&run(&mut again, b, raw, dir.path()).outline),
            titles(&answer.outline)
        );
        b.FPDF_CloseDocument(raw);
    }

    #[test]
    fn a_pdf_without_headings_gets_no_outline() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let b = pdfium.bindings();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.pdf");
        article_pdf(&pdfium, &file, false);
        let raw = b.FPDF_LoadDocument(file.to_str().unwrap(), None);
        let pages = b.FPDF_GetPageCount(raw) as u32;
        let mut gen = OutlineGen::new(b, raw, pages, &file, dir.path());
        assert!(run(&mut gen, b, raw, dir.path()).outline.is_empty());
        b.FPDF_CloseDocument(raw);
    }
}

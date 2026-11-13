//! OCR for the library's scanned papers: which pages get it, the cache that
//! keeps a page from being read twice, and the shape its lines take for the
//! first-page reader.
//!
//! The engine does the PDFium side (probing pages, rendering one at 2x) and
//! Vision does the reading (`engine::ocr`); everything here is pure, so the
//! rules are tested with the recognizer stubbed. OCR only ever runs from the
//! idle-priority library jobs: the first Vision call of a session can take
//! tens of seconds while the model loads, and the worker thread it runs on
//! is the one renders wait for.

use crate::engine::layout_text::TextLine;
use crate::engine::ocr::{self, OcrLine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Bumped when the cache's shape or what a cached page means changes.
const CACHE_VERSION: u32 = 1;

/// A page whose images together cover at least this share of it is a scan:
/// the page *is* the picture, and any text on it is a capture tool's
/// invisible layer, which is why the amount of text does not tell a scan
/// from a born-digital page — Acrobat Capture lays a full one over each.
pub const SCAN_COVERAGE: f32 = 0.6;

/// The first pages probed before deciding whether a document needs OCR at
/// all. A born-digital paper's first three pages settle it, and the text
/// pass skips the rest of the probing — and every render — for it.
pub const PROBE_PAGES: u32 = 3;

/// Pages OCR'd per idle text job. A page costs ~0.4 s on Apple silicon, so
/// a job holds the worker thread for a couple of seconds at most; a longer
/// scan resumes on the next job, its finished pages cached.
pub const TEXT_OCR_BUDGET: usize = 6;

/// What the engine learned about one page without reading it.
#[derive(Clone, Debug, Default)]
pub struct PageProbe {
    pub index: u32,
    /// The page's text layer as the library would index it (spaces
    /// repaired), for `ocr::needs_ocr` to judge.
    pub text: String,
    /// Width × height in points.
    pub area_pts: f32,
    pub has_images: bool,
    /// The page's images' combined share of its area, 0..1.
    pub image_coverage: f32,
}

impl PageProbe {
    pub fn is_scan(&self) -> bool {
        self.image_coverage >= SCAN_COVERAGE
    }

    pub fn needs_ocr(&self) -> bool {
        ocr::needs_ocr(&self.text, self.area_pts, self.has_images)
    }
}

/// Whether a document is worth OCR at all, from its first `PROBE_PAGES`:
/// one of them has an unusable text layer, or the first is a scan. The
/// scan records this on the entry (`ocr_wanted`); the text pass asks it
/// before probing past page 2.
pub fn wants_ocr(first_pages: &[PageProbe]) -> bool {
    first_pages.first().is_some_and(PageProbe::is_scan)
        || first_pages
            .iter()
            .filter(|probe| probe.index < PROBE_PAGES)
            .any(PageProbe::needs_ocr)
}

/// Every page of one document Vision has read, keyed by the content
/// fingerprint it was read from. `None` for a page Vision failed on, so it
/// is not retried on every pass either.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct OcrCache {
    version: u32,
    fingerprint: String,
    pages: BTreeMap<u32, Option<Vec<OcrLine>>>,
    #[serde(skip)]
    dirty: bool,
}

/// Where a document's OCR cache lives: beside its text sidecar.
pub fn cache_path(app_data_dir: &Path, document: &Path) -> PathBuf {
    crate::search::librarytext::sidecar_path(app_data_dir, document).with_extension("ocr.json")
}

impl OcrCache {
    /// The cache for `document` as it is now. One written for other bytes
    /// — a different fingerprint — is as good as none.
    pub fn load(app_data_dir: &Path, document: &Path, fingerprint: Option<&str>) -> Self {
        let fingerprint = fingerprint.unwrap_or_default();
        std::fs::read(cache_path(app_data_dir, document))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<OcrCache>(&bytes).ok())
            .filter(|cache| cache.version == CACHE_VERSION && cache.fingerprint == fingerprint)
            .unwrap_or_else(|| OcrCache {
                version: CACHE_VERSION,
                fingerprint: fingerprint.to_owned(),
                pages: BTreeMap::new(),
                dirty: false,
            })
    }

    /// Write the cache if anything was added since it was loaded. A cache
    /// that cannot be written costs a repeat OCR later, nothing else.
    pub fn save(&mut self, app_data_dir: &Path, document: &Path) {
        if !self.dirty {
            return;
        }
        let target = cache_path(app_data_dir, document);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec(self) {
            Ok(bytes) => match std::fs::write(&target, bytes) {
                Ok(()) => self.dirty = false,
                Err(error) => log::warn!("cannot write OCR cache {}: {error}", target.display()),
            },
            Err(error) => log::warn!("cannot serialize OCR cache: {error}"),
        }
    }

    /// `Some(result)` when the page has been read (successfully or not).
    pub fn get(&self, page: u32) -> Option<Option<&[OcrLine]>> {
        self.pages.get(&page).map(Option::as_deref)
    }

    pub fn insert(&mut self, page: u32, lines: Option<Vec<OcrLine>>) {
        self.pages.insert(page, lines);
        self.dirty = true;
    }

    /// The page's lines, from the cache or — when `budget` allows — from
    /// `recognize`, which is then cached. `None` when the page could not be
    /// read or the budget is spent; `pending` says which.
    pub fn lines_or_read(
        &mut self,
        page: u32,
        budget: &mut usize,
        pending: &mut bool,
        recognize: &mut dyn FnMut(u32) -> Option<Vec<OcrLine>>,
    ) -> Option<Vec<OcrLine>> {
        if let Some(cached) = self.get(page) {
            return cached.map(<[OcrLine]>::to_vec);
        }
        if *budget == 0 {
            *pending = true;
            return None;
        }
        *budget -= 1;
        let lines = recognize(page);
        self.insert(page, lines.clone());
        lines
    }
}

/// One text pass's worth of OCR over a document.
#[derive(Debug, Default)]
pub struct OcrPass {
    /// The OCR text for each page whose text layer it replaces.
    pub texts: BTreeMap<u32, String>,
    /// Pages still to read: the budget ran out. The document is not done.
    pub pending: bool,
}

/// OCR every page in `probes` whose text layer needs it, reading at most
/// `budget` pages that are not cached yet. Callers check `wants_ocr` first.
pub fn ocr_pages(
    probes: &[PageProbe],
    cache: &mut OcrCache,
    mut budget: usize,
    recognize: &mut dyn FnMut(u32) -> Option<Vec<OcrLine>>,
) -> OcrPass {
    let mut pass = OcrPass::default();
    for probe in probes.iter().filter(|probe| probe.needs_ocr()) {
        let lines = cache.lines_or_read(probe.index, &mut budget, &mut pass.pending, recognize);
        let text = lines
            .map(|lines| ocr::page_text(&lines))
            .unwrap_or_default();
        // Vision finding nothing leaves whatever the text layer had.
        if !text.trim().is_empty() {
            pass.texts.insert(probe.index, text);
        }
    }
    pass
}

/// Put the OCR text in place of the text layer's, for pages as
/// `extract_library_text` gives them (`(page, raw, word breaks)`). OCR text
/// has its spaces already, so it carries no breaks; a page the text layer
/// had nothing for is added in order. Returns the pages now holding OCR.
pub fn substitute(pages: &mut Vec<(u32, String, Vec<u32>)>, pass: &OcrPass) -> Vec<u32> {
    for (index, text) in &pass.texts {
        match pages.iter_mut().find(|(page, _, _)| page == index) {
            Some(page) => *page = (*index, text.clone(), Vec::new()),
            None => pages.push((*index, text.clone(), Vec::new())),
        }
    }
    pages.sort_by_key(|(page, _, _)| *page);
    pass.texts.keys().copied().collect()
}

/// Sizes within this ratio of each other are one size of type: Vision's
/// boxes follow the ink, so the two lines of one title differ by their
/// descenders, and `firstpage` would otherwise read them as two tiers.
const SIZE_SNAP: f32 = 1.25;

/// OCR lines as the typeset lines `firstpage::extract` reads. Positions in
/// points from the page's top-left, from the normalized boxes; the size is
/// the box height in points (the ink's height, close enough to the type
/// size for a reader that only compares sizes), snapped so lines whose
/// heights differ by their descenders read as one size. Weight unknown,
/// never bold: Vision does not say.
pub fn text_lines(lines: &[OcrLine], page_w: f32, page_h: f32) -> Vec<TextLine> {
    let mut out: Vec<TextLine> = lines
        .iter()
        .filter(|line| line.confidence >= ocr::MIN_LINE_CONFIDENCE && !line.text.trim().is_empty())
        .map(|line| {
            let [x, y, w, h] = line.bbox;
            TextLine {
                text: line.text.trim().to_owned(),
                size: h * page_h,
                weight: 0,
                bold: false,
                top: y * page_h,
                bottom: (y + h) * page_h,
                left: x * page_w,
                right: (x + w) * page_w,
            }
        })
        .collect();
    unify_display_blocks(&mut out);
    snap_sizes(&mut out);
    out
}

/// Display type at least this much larger than the body's is a title's
/// (or a heading's), for `unify_display_blocks`.
const DISPLAY_TYPE: f32 = 1.15;
/// Stacked display lines this close in height are one block's.
const BLOCK_SIZE_RATIO: f32 = 1.4;
/// …when the gap between them is under this share of the smaller's height.
const BLOCK_GAP: f32 = 0.6;

/// One size for every line of a stacked block of display type: a title
/// set over two lines. Vision's boxes follow each line's ink, so "Arthur-
/// Merlin Games: A Randomized Proof System," and "and a Hierarchy of
/// Complexity Classes" come out 15.9 and 13.0 pt — two tiers to the
/// first-page reader, which then takes the first line for the whole title.
/// Lines that are both display type, close in height and stacked tight are
/// given the larger's size. Body lines are left alone.
fn unify_display_blocks(lines: &mut [TextLine]) {
    let body = body_height(lines);
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| lines[a].top.total_cmp(&lines[b].top));
    for pair in order.windows(2) {
        let (above, below) = (&lines[pair[0]], &lines[pair[1]]);
        let (large, small) = if above.size >= below.size {
            (above.size, below.size)
        } else {
            (below.size, above.size)
        };
        let gap = below.top - above.bottom;
        let overlaps = above.left < below.right && below.left < above.right;
        if small >= DISPLAY_TYPE * body
            && large <= BLOCK_SIZE_RATIO * small
            && gap < BLOCK_GAP * small
            && overlaps
        {
            lines[pair[0]].size = large;
            lines[pair[1]].size = large;
        }
    }
}

/// The height most of the page's characters are set at: the median line
/// height, weighted by characters.
fn body_height(lines: &[TextLine]) -> f32 {
    let mut sized: Vec<(f32, usize)> = lines
        .iter()
        .map(|line| (line.size, line.text.chars().count()))
        .collect();
    sized.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: usize = sized.iter().map(|(_, n)| n).sum();
    let mut seen = 0;
    for (size, n) in sized {
        seen += n;
        if seen * 2 >= total {
            return size;
        }
    }
    0.0
}

/// Group sizes into runs, smallest first, where each is within `SIZE_SNAP`
/// of the run's smallest, and give every line its run's median.
fn snap_sizes(lines: &mut [TextLine]) {
    let mut sizes: Vec<f32> = lines.iter().map(|line| line.size).collect();
    sizes.sort_by(f32::total_cmp);
    let mut groups: Vec<Vec<f32>> = Vec::new();
    for size in sizes {
        match groups.last_mut() {
            Some(group) if size <= group[0] * SIZE_SNAP => group.push(size),
            _ => groups.push(vec![size]),
        }
    }
    for line in lines.iter_mut() {
        if let Some(group) = groups
            .iter()
            .find(|group| line.size >= group[0] && line.size <= *group.last().unwrap())
        {
            line.size = group[group.len() / 2];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, bbox: [f32; 4]) -> OcrLine {
        OcrLine {
            text: text.into(),
            confidence: 1.0,
            bbox,
        }
    }

    fn probe(index: u32, text: &str, coverage: f32) -> PageProbe {
        PageProbe {
            index,
            text: text.into(),
            area_pts: 612.0 * 792.0,
            has_images: coverage > 0.0,
            image_coverage: coverage,
        }
    }

    const PROSE: &str = "One can view NP as the complexity class that captures the notion \
        of efficient provability by classical (formal) proofs. We consider broader classes.";
    const CONFETTI: &str = "~ ,;l1 Il ~ ' .. ,; 1~ I ~ ,;l1 Il ~ ' .. ,; 1~ I";

    #[test]
    fn clean_documents_are_skipped() {
        let probes = [
            probe(0, PROSE, 0.0),
            probe(1, PROSE, 0.2),
            probe(2, PROSE, 0.0),
        ];
        assert!(!wants_ocr(&probes));
        // A scanned first page wants it even with a usable text layer.
        let scan = [probe(0, PROSE, 0.95), probe(1, PROSE, 0.95)];
        assert!(wants_ocr(&scan));
        // So does an image-only page among the first three.
        let blank = [probe(0, PROSE, 0.0), probe(1, "", 0.9)];
        assert!(wants_ocr(&blank));
        // A page past the probe window does not decide it.
        assert!(!wants_ocr(&[probe(0, PROSE, 0.0), probe(5, "", 0.9)]));
    }

    #[test]
    fn needs_ocr_pages_are_replaced_in_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("scan.pdf");
        let mut pages = vec![
            (0, PROSE.to_owned(), Vec::new()),
            (1, CONFETTI.to_owned(), Vec::new()),
        ];
        // Page 2 has an image and no text layer at all.
        let probes = [
            probe(0, PROSE, 0.0),
            probe(1, CONFETTI, 0.9),
            probe(2, "", 0.9),
        ];
        let mut cache = OcrCache::load(dir.path(), &doc, Some("fp"));
        let mut calls = Vec::new();
        let pass = ocr_pages(&probes, &mut cache, TEXT_OCR_BUDGET, &mut |page| {
            calls.push(page);
            Some(vec![line(
                &format!("Recovered polynomial hierarchy page {page}"),
                [0.1, 0.1, 0.8, 0.02],
            )])
        });
        assert_eq!(calls, [1, 2], "only the pages that need it");
        assert!(!pass.pending);
        let replaced = substitute(&mut pages, &pass);
        assert_eq!(replaced, [1, 2]);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].1, PROSE);
        assert_eq!(pages[1].1, "Recovered polynomial hierarchy page 1");
        assert_eq!(pages[2].0, 2);

        crate::search::librarytext::write_spaced_sidecar(dir.path(), &doc, 1, 2, &pages, &replaced)
            .unwrap();
        let text = crate::search::librarytext::read_sidecar_text(dir.path(), &doc, 1, 2).unwrap();
        assert!(
            text.contains("Recovered polynomial hierarchy page 2"),
            "{text}"
        );
        assert!(!text.contains(";l1"), "{text}");
    }

    #[test]
    fn cached_pages_are_not_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("scan.pdf");
        let probes = [probe(0, "", 0.9)];
        let mut cache = OcrCache::load(dir.path(), &doc, Some("fp"));
        let first = ocr_pages(&probes, &mut cache, 6, &mut |_| {
            Some(vec![line("Arthur-Merlin Games", [0.1, 0.1, 0.5, 0.03])])
        });
        cache.save(dir.path(), &doc);
        assert_eq!(first.texts[&0], "Arthur-Merlin Games");

        let mut cache = OcrCache::load(dir.path(), &doc, Some("fp"));
        let again = ocr_pages(&probes, &mut cache, 6, &mut |_| {
            panic!("cache hit expected")
        });
        assert_eq!(again.texts[&0], "Arthur-Merlin Games");

        // A failed read is cached too, and not retried.
        cache.insert(1, None);
        let failed = ocr_pages(&[probe(1, "", 0.9)], &mut cache, 6, &mut |_| {
            panic!("cached")
        });
        assert!(failed.texts.is_empty() && !failed.pending);

        // Different bytes: the cache is empty.
        let other = OcrCache::load(dir.path(), &doc, Some("other"));
        assert!(other.get(0).is_none());
    }

    #[test]
    fn budget_spent_leaves_the_rest_pending_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("long-scan.pdf");
        let probes: Vec<PageProbe> = (0..10).map(|i| probe(i, "", 1.0)).collect();
        let mut read = 0;
        let mut recognize = |page: u32| {
            read += 1;
            Some(vec![line(
                &format!("page {page} text"),
                [0.1, 0.1, 0.8, 0.02],
            )])
        };
        let mut cache = OcrCache::load(dir.path(), &doc, Some("fp"));
        let first = ocr_pages(&probes, &mut cache, 6, &mut recognize);
        assert!(first.pending);
        assert_eq!(first.texts.len(), 6);
        cache.save(dir.path(), &doc);

        let mut cache = OcrCache::load(dir.path(), &doc, Some("fp"));
        let second = ocr_pages(&probes, &mut cache, 6, &mut recognize);
        assert!(!second.pending);
        assert_eq!(second.texts.len(), 10);
        assert_eq!(read, 10, "each page read once across the two jobs");
    }

    #[test]
    fn ocr_lines_become_text_lines_for_the_first_page_reader() {
        let (w, h) = (453.0, 669.0);
        let lines = [
            line(
                "JOURNAL OF COMPUTER AND SYSTEM SCIENCES 36, 254-276 (1988)",
                [0.126, 0.057, 0.491, 0.0135],
            ),
            line(
                "Arthur Merlin Games: A Randomized Proof System,",
                [0.194, 0.136, 0.652, 0.0237],
            ),
            line(
                "and a Hierarchy of Complexity Classes",
                [0.278, 0.163, 0.485, 0.0194],
            ),
            line("LASZLO BABAI", [0.452, 0.199, 0.137, 0.0135]),
            line(
                "One can view NP as the complexity class that captures the notion of efficient",
                [0.183, 0.42, 0.692, 0.0135],
            ),
            OcrLine {
                text: "~".into(),
                confidence: 0.1,
                bbox: [0.1, 0.5, 0.01, 0.01],
            },
        ];
        let out = text_lines(&lines, w, h);
        assert_eq!(out.len(), 5, "the low-confidence speck is dropped");
        let title = &out[1];
        assert!((title.top - 0.136 * h).abs() < 0.01);
        assert!((title.left - 0.194 * w).abs() < 0.01);
        assert!((title.right - (0.194 + 0.652) * w).abs() < 0.01);
        assert!(title.bottom > title.top && !title.bold);
        // Both title lines read as one size, larger than the byline's.
        assert_eq!(out[1].size, out[2].size);
        assert!(out[1].size > out[3].size);
        assert_eq!(out[0].size, out[3].size);
    }
}

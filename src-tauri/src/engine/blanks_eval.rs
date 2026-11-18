//! Blank detection measured against real forms.
//!
//! Ignored by default: it needs PDFs that are not in the repository. The
//! labelled source is fillable forms made flat: the same PDF with its widget
//! annotations and AcroForm removed (pypdf), so the page shows exactly what
//! a printed copy shows, while the original's widget rects say where the
//! blanks are. Point the eval at both folders (same file names):
//!
//! ```text
//! SPEEDYF_BLANKS_FORMS=/path/to/fillable SPEEDYF_BLANKS_FLAT=/path/to/flat \
//!   cargo test --lib blanks_eval -- --ignored --nocapture
//! ```
//!
//! A ground-truth blank is found when a detection overlaps it with IoU of at
//! least 0.5, or when one of the two lies (70%) inside the other and they are
//! within a factor of six in area. Precision counts detections that match
//! some widget the same way. `SPEEDYF_BLANKS_NEGATIVE` may name a folder of
//! ordinary documents (papers), reported as blanks per page: there should be
//! few. `SPEEDYF_BLANKS_DEBUG=1` lists misses and false alarms.

use super::blanks;
use super::forms::read_form;
use super::pdfium_init;
use super::render;
use super::types::{BlankDto, RectDto};
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn area(r: &RectDto) -> f32 {
    r.w.max(0.0) * r.h.max(0.0)
}

fn inter(a: &RectDto, b: &RectDto) -> f32 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    if w <= 0.0 || h <= 0.0 {
        0.0
    } else {
        w * h
    }
}

pub(super) fn matches(truth: &RectDto, found: &RectDto) -> bool {
    let i = inter(truth, found);
    if i <= 0.0 {
        return false;
    }
    let (at, af) = (area(truth), area(found));
    let iou = i / (at + af - i);
    let ratio = at.max(af) / at.min(af).max(1e-3);
    iou >= 0.5 || ((i >= 0.7 * at || i >= 0.7 * af) && ratio <= 6.0)
}

pub(super) fn pdfs(dir: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Detect on one page of an open document, with the time it took.
fn detect_page(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    index: i32,
) -> (Vec<BlankDto>, f64) {
    let Some([w, h]) = render::page_display_size(b, doc, index) else {
        return (Vec::new(), 0.0);
    };
    let page = b.FPDF_LoadPage(doc, index);
    if page.is_null() {
        return (Vec::new(), 0.0);
    }
    let started = Instant::now();
    let input = blanks::collect(b, page, w, h);
    if let Ok(region) = std::env::var("SPEEDYF_BLANKS_DUMP") {
        // "page:x0,y0,x1,y1": print the raw input inside that box.
        let (p, r) = region.split_once(':').unwrap_or(("", ""));
        let r: Vec<f32> = r.split(',').filter_map(|v| v.parse().ok()).collect();
        if p.parse::<i32>().ok() == Some(index) && r.len() == 4 {
            let inside = |x: f32, y: f32| x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3];
            for l in input
                .hlines
                .iter()
                .filter(|l| inside(l.x0, l.y) || inside(l.x1, l.y))
            {
                eprintln!("  H {:.1}..{:.1} @ {:.1} t{:.1}", l.x0, l.x1, l.y, l.t);
            }
            for l in input
                .vlines
                .iter()
                .filter(|l| inside(l.x, l.y0) || inside(l.x, l.y1))
            {
                eprintln!("  V {:.1} @ {:.1}..{:.1} t{:.1}", l.x, l.y0, l.y1, l.t);
            }
            for g in input.glyphs.iter().filter(|g| inside(g.b.x0, g.b.y0)) {
                eprintln!(
                    "  G {:?} {:.1},{:.1} {:.1}x{:.1} s{:.1}",
                    g.ch,
                    g.b.x0,
                    g.b.y0,
                    g.b.x1 - g.b.x0,
                    g.b.y1 - g.b.y0,
                    g.size
                );
            }
            for i in input.ink.iter().filter(|i| inside(i.x0, i.y0)) {
                eprintln!("  I {i:?}");
            }
        }
    }
    let found = blanks::detect(&input);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    b.FPDF_ClosePage(page);
    (found, ms)
}

#[derive(Default)]
struct Row {
    name: String,
    pages: usize,
    truth: usize,
    found_truth: usize,
    detections: usize,
    true_detections: usize,
    ms: f64,
}

#[test]
#[ignore]
fn blanks_eval() {
    let (Ok(forms), Ok(flat)) = (
        std::env::var("SPEEDYF_BLANKS_FORMS"),
        std::env::var("SPEEDYF_BLANKS_FLAT"),
    ) else {
        return;
    };
    let debug = std::env::var("SPEEDYF_BLANKS_DEBUG").is_ok();
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();

    let mut rows: Vec<Row> = Vec::new();
    for original in pdfs(&forms) {
        let name = original.file_name().unwrap().to_string_lossy().into_owned();
        if std::env::var("SPEEDYF_BLANKS_ONLY").is_ok_and(|only| !name.contains(&only)) {
            continue;
        }
        let flat_path = Path::new(&flat).join(&name);
        if !flat_path.exists() {
            continue;
        }
        let path = original.to_string_lossy().into_owned();
        let Ok(raw) = render::open_document(b, &path, None) else {
            continue;
        };
        let page_count = b.FPDF_GetPageCount(raw).max(0) as u32;
        let form = read_form(b, raw, page_count, &path, None);
        b.FPDF_CloseDocument(raw);
        let fields: Vec<_> = form
            .fields
            .iter()
            .filter(|f| {
                matches!(
                    f.kind.as_str(),
                    "text" | "checkbox" | "radio" | "combo" | "list" | "signature"
                ) && f.rect.w > 1.0
                    && f.rect.h > 1.0
            })
            .collect();
        if fields.is_empty() {
            continue;
        }
        let flat_str = flat_path.to_string_lossy().into_owned();
        let doc = render::open_document(b, &flat_str, None).expect("flat copy opens");
        let mut row = Row {
            name: name.clone(),
            ..Row::default()
        };
        for page in 0..page_count {
            let truth: Vec<RectDto> = fields
                .iter()
                .filter(|f| f.page == page)
                .map(|f| f.rect)
                .collect();
            if truth.is_empty() {
                continue;
            }
            let (found, ms) = detect_page(b, doc, page as i32);
            row.pages += 1;
            row.ms += ms;
            row.truth += truth.len();
            row.detections += found.len();
            for t in &truth {
                if found.iter().any(|f| matches(t, &f.rect)) {
                    row.found_truth += 1;
                } else if debug {
                    eprintln!("  MISS {name} p{page} {t:?}");
                }
            }
            for f in &found {
                if truth.iter().any(|t| matches(t, &f.rect)) {
                    row.true_detections += 1;
                } else if debug {
                    eprintln!("  FALSE {name} p{page} {} {:?}", f.kind, f.rect);
                }
            }
        }
        b.FPDF_CloseDocument(doc);
        rows.push(row);
    }

    println!();
    println!(
        "{:<14} {:>5} {:>6} {:>6} {:>7} {:>7} {:>9}",
        "form", "pages", "truth", "found", "recall", "prec", "ms/page"
    );
    let mut total = Row::default();
    for r in &rows {
        println!(
            "{:<14} {:>5} {:>6} {:>6} {:>6.1}% {:>6.1}% {:>9.2}",
            r.name,
            r.pages,
            r.truth,
            r.detections,
            100.0 * r.found_truth as f64 / r.truth.max(1) as f64,
            100.0 * r.true_detections as f64 / r.detections.max(1) as f64,
            r.ms / r.pages.max(1) as f64
        );
        total.pages += r.pages;
        total.truth += r.truth;
        total.found_truth += r.found_truth;
        total.detections += r.detections;
        total.true_detections += r.true_detections;
        total.ms += r.ms;
    }
    println!(
        "{:<14} {:>5} {:>6} {:>6} {:>6.1}% {:>6.1}% {:>9.2}",
        "ALL",
        total.pages,
        total.truth,
        total.detections,
        100.0 * total.found_truth as f64 / total.truth.max(1) as f64,
        100.0 * total.true_detections as f64 / total.detections.max(1) as f64,
        total.ms / total.pages.max(1) as f64
    );

    if let Ok(neg) = std::env::var("SPEEDYF_BLANKS_NEGATIVE") {
        let (mut pages, mut found, mut ms) = (0usize, 0usize, 0f64);
        for path in pdfs(&neg) {
            let path = path.to_string_lossy().into_owned();
            let Ok(doc) = render::open_document(b, &path, None) else {
                continue;
            };
            let n = b.FPDF_GetPageCount(doc).clamp(0, 12);
            for page in 0..n {
                let (blanks, t) = detect_page(b, doc, page);
                if debug {
                    for f in &blanks {
                        eprintln!("  NEG {path} p{page} {} {:?}", f.kind, f.rect);
                    }
                }
                pages += 1;
                found += blanks.len();
                ms += t;
            }
            b.FPDF_CloseDocument(doc);
        }
        println!(
            "negatives: {pages} pages, {found} blanks ({:.2}/page), {:.2} ms/page",
            found as f64 / pages.max(1) as f64,
            ms / pages.max(1) as f64
        );
    }
}

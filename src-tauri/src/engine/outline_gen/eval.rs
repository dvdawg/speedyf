//! Evaluation against real PDFs, read-only. Not part of the test run:
//!
//!   OUTLINE_EVAL_LIST=/path/to/list.txt cargo test --lib outline_eval \
//!     -- --ignored --nocapture
//!
//! `list.txt` names one PDF per line. A document with a real outline has
//! it hidden (generation never reads bookmarks) and compared: a generated
//! heading counts as correct when an outline entry within the first three
//! levels names the same page and the same title. A document without one
//! has its generated outline printed for checking by hand.

use super::detect::Heading;
use super::normalize;
use crate::engine::pdfium_init;
use crate::engine::tagged::utf16_string;
use pdfium_render::prelude::*;

struct Real {
    depth: usize,
    title: String,
    page: Option<u32>,
}

fn walk(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    parent: FPDF_BOOKMARK,
    depth: usize,
    out: &mut Vec<Real>,
) {
    if depth > 16 || out.len() > 5000 {
        return;
    }
    let mut current = b.FPDFBookmark_GetFirstChild(doc, parent);
    while !current.is_null() && out.len() <= 5000 {
        let title = utf16_string(|buffer, len| b.FPDFBookmark_GetTitle(current, buffer, len));
        let mut dest = b.FPDFBookmark_GetDest(doc, current);
        if dest.is_null() {
            let action = b.FPDFBookmark_GetAction(current);
            if !action.is_null() {
                dest = b.FPDFAction_GetDest(doc, action);
            }
        }
        let page = (!dest.is_null())
            .then(|| b.FPDFDest_GetDestPageIndex(doc, dest))
            .filter(|p| *p >= 0)
            .map(|p| p as u32);
        out.push(Real { depth, title, page });
        walk(b, doc, current, depth + 1, out);
        current = b.FPDFBookmark_GetNextSibling(doc, current);
    }
}

fn key(text: &str) -> String {
    let title = super::numbering::parse(text).map_or_else(|| text.to_owned(), |n| n.title);
    let mut letters = normalize::letters(&normalize::clean(&title));
    if letters.is_empty() {
        letters = normalize::letters(text);
    }
    letters
}

fn same_title(a: &str, b: &str) -> bool {
    let (a, b) = (key(a), key(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    // A wrapped or abbreviated title: one is the start of the other.
    a.len().min(b.len()) >= 8 && (a.starts_with(&b) || b.starts_with(&a))
}

fn matches(g: &Heading, r: &Real) -> bool {
    r.page.is_some_and(|p| p.abs_diff(g.page) <= 1) && same_title(&g.title, &r.title)
}

#[test]
#[ignore]
fn outline_eval() {
    let Ok(list) = std::env::var("OUTLINE_EVAL_LIST") else {
        return;
    };
    let paths: Vec<String> = std::fs::read_to_string(list)
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let b = pdfium.bindings();
    let (mut tp, mut gen_total, mut real_total, mut real_hit) = (0usize, 0usize, 0usize, 0usize);
    let mut unverifiable = 0usize;
    let mut docs_with = 0;
    for path in paths {
        let doc = b.FPDF_LoadDocument(&path, None);
        if doc.is_null() {
            println!("SKIP (cannot open) {path}");
            continue;
        }
        let pages = b.FPDF_GetPageCount(doc).max(0) as u32;
        let started = std::time::Instant::now();
        let generated = super::generate_for_file(b, doc, pages);
        let elapsed = started.elapsed();
        let mut real = Vec::new();
        walk(b, doc, std::ptr::null_mut(), 0, &mut real);
        b.FPDF_CloseDocument(doc);
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let real: Vec<Real> = real.into_iter().filter(|r| r.depth < 3).collect();
        let trivial = real.len() <= 1 || real.iter().all(|r| r.page.unwrap_or(0) == 0);
        if !trivial {
            docs_with += 1;
            let hits = generated
                .iter()
                .filter(|g| real.iter().any(|r| matches(g, r)))
                .count();
            // Headings hyperref leaves out of the bookmarks (\section*):
            // right or wrong, the real outline cannot say.
            unverifiable += generated
                .iter()
                .filter(|g| {
                    !real.iter().any(|r| matches(g, r)) && normalize::is_top_level_name(&g.title)
                })
                .count();
            let found = real
                .iter()
                .filter(|r| generated.iter().any(|g| matches(g, r)))
                .count();
            tp += hits;
            gen_total += generated.len();
            real_total += real.len();
            real_hit += found;
            println!(
                "WITH {name}: pages={pages} real={} gen={} correct={} found={} ({:?})",
                real.len(),
                generated.len(),
                hits,
                found,
                elapsed
            );
            for g in &generated {
                let ok = real.iter().any(|r| matches(g, r));
                println!(
                    "   {} {}{} p{}",
                    if ok { "+" } else { "-" },
                    "  ".repeat(g.level as usize),
                    g.title,
                    g.page
                );
            }
            if std::env::var("OUTLINE_EVAL_REAL").is_ok_and(|v| !v.is_empty()) {
                for r in &real {
                    println!("   R {}{} p{:?}", "  ".repeat(r.depth), r.title, r.page);
                }
            }
        } else {
            println!(
                "NONE {name}: pages={pages} gen={} ({:?})",
                generated.len(),
                elapsed
            );
            for g in &generated {
                println!(
                    "     {}{} p{}",
                    "  ".repeat(g.level as usize),
                    g.title,
                    g.page
                );
            }
        }
    }
    let precision = tp as f32 / gen_total.max(1) as f32;
    let checkable = gen_total - unverifiable;
    println!(
        "UNVERIFIABLE known names absent from the real outline: {unverifiable}; precision over the rest {:.3}",
        tp as f32 / checkable.max(1) as f32
    );
    let recall = real_hit as f32 / real_total.max(1) as f32;
    println!(
        "SUMMARY docs_with_outline={docs_with} generated={gen_total} correct={tp} precision={precision:.3} real={real_total} found={real_hit} recall={recall:.3}"
    );
}

/// Print one page's lines with their style facts:
///   OUTLINE_DUMP=/path/file.pdf:3 cargo test --lib outline_dump -- --ignored --nocapture
#[test]
#[ignore]
fn outline_dump() {
    let Ok(spec) = std::env::var("OUTLINE_DUMP") else {
        return;
    };
    let (path, page) = spec.rsplit_once(':').unwrap();
    let page: u32 = page.parse().unwrap();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(path, None);
    assert!(!doc.is_null());
    let mut fonts = super::extract::FontTable::default();
    let p = super::extract::read_page(b, doc, page, &mut fonts, true);
    println!(
        "page {} {}x{} label={:?} chars={}",
        p.index, p.width, p.height, p.label, p.chars
    );
    for l in &p.lines {
        println!(
            "{:6.1}-{:6.1} x{:5.1}-{:5.1} s{:4.1} b{} i{} sc{} m{:.1} f{} lead={:?} | {}",
            l.top,
            l.bottom,
            l.left,
            l.right,
            l.size,
            u8::from(l.bold),
            u8::from(l.italic),
            u8::from(l.smallcaps),
            l.math,
            l.font,
            l.lead.as_ref().map(|x| x.text.as_str()),
            l.text
        );
    }
    for t in &p.tagged {
        println!("TAG H{} {:.1} {}", t.level, t.top, t.text);
    }
    for (i, name) in fonts.names.iter().enumerate() {
        println!("font f{i} {name} {:?}", fonts.facts[i]);
    }
    b.FPDF_CloseDocument(doc);
}

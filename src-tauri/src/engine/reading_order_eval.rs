//! Measure reading order on real PDFs: for each page, how often the stream
//! order jumps between columns and how often the reading order does, and
//! (with SPEEDYF_ORDER_OUT) a JSON dump of each two-column page's runs in
//! both orders for `src/lib/text/readingOrderEval.test.ts`. Read only. Run
//! with:
//!
//!   SPEEDYF_ORDER_PDFS="a.pdf;b.pdf" SPEEDYF_ORDER_OUT=/tmp/order \
//!     cargo test --manifest-path src-tauri/Cargo.toml reading_order_eval -- --ignored --nocapture

use crate::engine::preview::{nearest_column, page_columns, rows_with_members, spans_gutter};
use crate::engine::types::TextRun;
use crate::engine::{pdfium_init, reading_order, render, text};
use pdfium_render::prelude::*;
use serde::Serialize;

#[derive(Serialize)]
struct PageDump {
    path: String,
    page: u32,
    size: [f32; 2],
    columns: Vec<[f32; 2]>,
    /// the runs in the stream's order
    runs: Vec<TextRun>,
    /// indices of `runs` in reading order
    order: Vec<usize>,
}

/// Share of consecutive runs, in `order`, that move from one column to
/// another (rows across a gutter count as no column).
fn jumps(runs: &[TextRun], order: &[usize], page_w: f32) -> (usize, usize) {
    let columns = page_columns(runs, page_w);
    let (rows, member) = rows_with_members(runs, &columns, true);
    let column_of = |i: usize| {
        let row = &rows[member[i]?];
        let c = nearest_column(&columns, row.x + row.w.min(20.0) / 2.0);
        (!spans_gutter(&columns, c, row)).then_some(c)
    };
    let mut pairs = 0;
    let mut moved = 0;
    for pair in order.windows(2) {
        if let (Some(a), Some(b)) = (column_of(pair[0]), column_of(pair[1])) {
            pairs += 1;
            moved += usize::from(a != b);
        }
    }
    (moved, pairs)
}

/// Read one dump again (SPEEDYF_ORDER_DUMP), for tracing a page.
#[test]
#[ignore]
fn reading_order_dump() {
    let Ok(path) = std::env::var("SPEEDYF_ORDER_DUMP") else {
        return;
    };
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("dump")).expect("json");
    let f = |r: &serde_json::Value, k: &str| r[k].as_f64().unwrap_or(0.0) as f32;
    let runs: Vec<TextRun> = v["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .map(|r| TextRun {
            text: r["text"].as_str().unwrap_or("").to_string(),
            start: r["start"].as_u64().unwrap_or(0) as u32,
            x: f(r, "x"),
            y: f(r, "y"),
            w: f(r, "w"),
            h: f(r, "h"),
            edges: Vec::new(),
            math: Vec::new(),
        })
        .collect();
    let size = |k: usize| v["size"][k].as_f64().unwrap_or(0.0) as f32;
    for i in reading_order::reading_order(&runs, size(0), size(1)) {
        let r = &runs[i];
        println!("{i} {:.0},{:.0} {}", r.x, r.y, r.text);
    }
}

#[test]
#[ignore]
fn reading_order_eval() {
    let Ok(mut list) = std::env::var("SPEEDYF_ORDER_PDFS") else {
        return;
    };
    // Or a directory of them.
    if let Ok(entries) = std::fs::read_dir(&list) {
        let mut paths: Vec<String> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        paths.sort();
        list = paths.join(";");
    }
    let out = std::env::var("SPEEDYF_ORDER_OUT").ok();
    if let Some(out) = &out {
        std::fs::create_dir_all(out).expect("output directory");
    }
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    for path in list.split(';').filter(|s| !s.trim().is_empty()) {
        let Ok(raw) = render::open_document(b, path, None) else {
            println!("{path}: cannot open");
            continue;
        };
        let count = b.FPDF_GetPageCount(raw);
        let (mut before, mut after, mut pairs, mut two) = (0, 0, 0, 0);
        let (mut one, mut one_moved) = (0, 0);
        let mut slowest = (0.0f64, 0usize);
        let mut worst: Vec<(u32, f32)> = Vec::new();
        for page in 0..count.min(40) as u32 {
            let Some([w, h]) = render::page_display_size(b, raw, page as i32) else {
                continue;
            };
            let handle = b.FPDF_LoadPage(raw, page as i32);
            if handle.is_null() {
                continue;
            }
            let runs = text::extract_page(b, handle, w, h).runs;
            b.FPDF_ClosePage(handle);
            let started = std::time::Instant::now();
            let _ = reading_order::reading_order(&runs, w, h);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if ms > slowest.0 {
                slowest = (ms, runs.len());
            }
            let columns = page_columns(&runs, w);
            if columns.len() < 2 {
                // A single column: how much of the stream's order moved.
                let order = reading_order::reading_order(&runs, w, h);
                one += 1;
                let moved = order.iter().enumerate().filter(|(k, &i)| *k != i).count();
                one_moved += usize::from(moved > 0);
                if moved > 0 && std::env::var("SPEEDYF_ORDER_SINGLE").is_ok() {
                    let first = order.iter().enumerate().find(|(k, &i)| *k != i);
                    if let Some((k, &i)) = first {
                        println!(
                            "  p{page}: {moved}/{} moved, first at {k}: {:?} for {:?}",
                            runs.len(),
                            runs[i].text,
                            runs[k].text
                        );
                    }
                }
                continue;
            }
            two += 1;
            let stream: Vec<usize> = (0..runs.len()).collect();
            let order = reading_order::reading_order(&runs, w, h);
            let (m0, p) = jumps(&runs, &stream, w);
            let (m1, _) = jumps(&runs, &order, w);
            before += m0;
            after += m1;
            pairs += p;
            if p > 0 {
                worst.push((page, m0 as f32 / p as f32));
            }
            if let Some(out) = &out {
                let name = std::path::Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let dump = PageDump {
                    path: path.to_string(),
                    page,
                    size: [w, h],
                    columns: columns.iter().map(|c| [c.start, c.end]).collect(),
                    runs,
                    order,
                };
                let file = std::path::Path::new(out).join(format!("{name}-p{page}.json"));
                std::fs::write(file, serde_json::to_string(&dump).expect("json")).expect("write");
            }
        }
        b.FPDF_CloseDocument(raw);
        worst.sort_by(|a, b| b.1.total_cmp(&a.1));
        println!(
            "{}: {two} two-column pages, column jumps {before}/{pairs} in stream order, {after}/{pairs} in reading order; worst pages {:?}; {one_moved}/{one} one-column pages reordered; slowest page {:.1} ms ({} runs)",
            path.rsplit('/').next().unwrap_or(path),
            worst.iter().take(3).collect::<Vec<_>>(),
            slowest.0,
            slowest.1
        );
    }
}

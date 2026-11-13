//! Running the hover detectors over real PDFs, read-only, for hand-checking.
//!
//! Ignored by default. Point it at a file listing one PDF path per line:
//!
//! ```text
//! SPEEDYF_HOVER_EVAL=list.txt SPEEDYF_HOVER_OUT=report.txt \
//!   cargo test --lib hovers::eval -- --ignored --nocapture
//! ```
//!
//! Every span is printed with the text around it and the text at its
//! target. Where the document's own link annotation covers a mention
//! (hyperref), the link's destination page is a free second opinion: the
//! report counts how often the two agree.

use super::page::Page;
use super::{extract, page_spans, IndexBuilder, IndexResult};
use crate::engine::{links, pdfium_init, render, text, types::LinkTarget};
use pdfium_render::prelude::*;
use std::fmt::Write as _;
use std::time::Instant;

fn context(page: &Page, start: usize, end: usize) -> String {
    let from = start.saturating_sub(50);
    let to = (end + 50).min(page.text.len());
    let before: String = page.text[from..start].iter().collect();
    let inside: String = page.text[start..end].iter().collect();
    let after: String = page.text[end..to].iter().collect();
    format!("...{before}[[{inside}]]{after}...")
}

fn region_text(page: &Page, region: [f32; 4]) -> String {
    let mut out = String::new();
    for (i, l) in page.lines.iter().enumerate() {
        let inside = l.bottom >= region[1] - 1.0
            && l.top <= region[1] + region[3] + 1.0
            && l.x1 >= region[0]
            && l.x0 <= region[0] + region[2];
        if inside {
            out.push_str(&page.line_text(i));
            out.push_str(" / ");
        }
        if out.chars().count() > 160 {
            break;
        }
    }
    out.chars().take(160).collect()
}

/// Where in `page.text` a span's first rect sits: the characters whose
/// glyphs fall inside it.
fn locate(page: &Page, rect: [f32; 4]) -> Option<(usize, usize)> {
    let inside: Vec<usize> = (0..page.text.len())
        .filter(|&i| {
            page.glyph_at(i).is_some_and(|g| {
                let cx = g.rect[0] + g.rect[2] / 2.0;
                let cy = g.rect[1] + g.rect[3] / 2.0;
                g.has_box()
                    && cx >= rect[0]
                    && cx <= rect[0] + rect[2]
                    && cy >= rect[1]
                    && cy <= rect[1] + rect[3]
            })
        })
        .collect();
    Some((*inside.first()?, *inside.last()? + 1))
}

#[test]
#[ignore]
fn evaluate_real_documents() {
    let Ok(list) = std::env::var("SPEEDYF_HOVER_EVAL") else {
        return;
    };
    let out_path = std::env::var("SPEEDYF_HOVER_OUT").unwrap_or_else(|_| "hover-eval.txt".into());
    let paths: Vec<String> = std::fs::read_to_string(list)
        .expect("list")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let mut report = String::new();
    let mut totals: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let (mut agree, mut disagree, mut linked) = (0usize, 0usize, 0usize);
    let mut worst_page_ms = 0f64;
    for (doc_index, path) in paths.into_iter().enumerate() {
        let Ok(raw) = render::open_document(b, &path, None) else {
            continue;
        };
        let count = b.FPDF_GetPageCount(raw).max(0) as u32;
        let started = Instant::now();
        let mut builder = IndexBuilder::new(count);
        let mut pages: Vec<Option<Page>> = Vec::new();
        let mut page_links: Vec<Vec<([f32; 4], Option<u32>)>> = Vec::new();
        let mut page_runs: Vec<Vec<crate::engine::types::TextRun>> = Vec::new();
        while let Some(src) = builder.next_page() {
            let page_started = Instant::now();
            let size = render::page_display_size(b, raw, src as i32);
            let handle = b.FPDF_LoadPage(raw, src as i32);
            let model = match (size, handle.is_null()) {
                (Some([w, h]), false) => {
                    let input = extract::extract(b, handle, w, h);
                    // links, for the cross-check
                    let extracted = text::extract_page(b, handle, w, h);
                    page_runs.push(extracted.runs.clone());
                    let tp = b.FPDFText_LoadPage(handle);
                    let found =
                        links::extract_links(b, raw, handle, tp, [w, h], &extracted.boxes, count);
                    if !tp.is_null() {
                        b.FPDFText_ClosePage(tp);
                    }
                    page_links.push(
                        found
                            .into_iter()
                            .map(|l| {
                                let dest = match l.target {
                                    LinkTarget::Internal { page, .. } => Some(page),
                                    _ => None,
                                };
                                (l.rect, dest)
                            })
                            .collect(),
                    );
                    Some(Page::new(input))
                }
                _ => {
                    page_links.push(Vec::new());
                    page_runs.push(Vec::new());
                    None
                }
            };
            if !handle.is_null() {
                b.FPDF_ClosePage(handle);
            }
            builder.add_page(model.clone());
            pages.push(model.map(|p| super::prepare_page(p, Default::default())));
            worst_page_ms = worst_page_ms.max(page_started.elapsed().as_secs_f64() * 1000.0);
        }
        let result: IndexResult = builder.finish();
        let elapsed = started.elapsed().as_secs_f64();
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = writeln!(report, "\n=== {name} ({count} pages, {:.2}s) ===", elapsed);
        evaluate_terms(&mut report, doc_index, &name, &result, &pages, &page_runs);
        let mut keys: Vec<&u32> = result.spans.keys().collect();
        keys.sort();
        for src in keys {
            let Some(Some(page)) = pages.get(*src as usize) else {
                continue;
            };
            let links_here: Vec<[f32; 4]> =
                page_links[*src as usize].iter().map(|(r, _)| *r).collect();
            let shown = page_spans(*src, Some(&result), None, &[], &links_here);
            for span in &result.spans[src] {
                *totals.entry(span.kind).or_default() += 1;
                let covered = !shown.contains(span);
                let link_dest = page_links[*src as usize].iter().find_map(|(r, dest)| {
                    let hit = span.rects.iter().any(|s| {
                        s[0] < r[0] + r[2]
                            && r[0] < s[0] + s[2]
                            && s[1] < r[1] + r[3]
                            && r[1] < s[1] + s[3]
                    });
                    hit.then_some(*dest).flatten()
                });
                if let Some(dest) = link_dest {
                    linked += 1;
                    if dest == span.target.page {
                        agree += 1;
                    } else {
                        disagree += 1;
                    }
                }
                let ctx = span
                    .rects
                    .first()
                    .and_then(|r| locate(page, *r))
                    .map(|(s, e)| context(page, s, e))
                    .unwrap_or_default();
                let target = span
                    .target
                    .region
                    .and_then(|region| {
                        pages
                            .get(span.target.page as usize)
                            .and_then(|p| p.as_ref())
                            .map(|p| region_text(p, region))
                    })
                    .unwrap_or_default();
                let _ = writeln!(
                    report,
                    "p{} {} {:<10} -> p{}{}{} | {}\n      TARGET: {}",
                    src + 1,
                    span.label,
                    span.kind,
                    span.target.page + 1,
                    if covered { " [linked]" } else { "" },
                    match link_dest {
                        Some(d) if d != span.target.page => format!(" [LINK SAYS p{}]", d + 1),
                        _ => String::new(),
                    },
                    ctx.replace('\n', " "),
                    target
                );
            }
        }
        b.FPDF_CloseDocument(raw);
    }
    let _ = writeln!(
        report,
        "\nTOTALS {totals:?}\nlinked {linked}: agree {agree}, disagree {disagree}\nworst page {worst_page_ms:.1} ms"
    );
    std::fs::write(&out_path, &report).expect("write report");
    eprintln!("wrote {out_path}");
}

/// Page texts out (as `document.pageText` gives them, with each run's box
/// as `document.pageRuns` does) for Notation to read; its terms back in,
/// matched on the same pages the app would match them on.
fn evaluate_terms(
    report: &mut String,
    doc_index: usize,
    name: &str,
    result: &IndexResult,
    pages: &[Option<Page>],
    runs: &[Vec<crate::engine::types::TextRun>],
) {
    if let Ok(dir) = std::env::var("SPEEDYF_HOVER_DUMP") {
        let dump = serde_json::json!({
            "name": name,
            "pages": runs.iter().map(|page| {
                let mut text = String::new();
                for run in page {
                    if !text.is_empty() && !text.ends_with(char::is_whitespace)
                        && !run.text.starts_with(char::is_whitespace) {
                        text.push(' ');
                    }
                    text.push_str(&run.text);
                }
                serde_json::json!({
                    "text": text,
                    "runs": page.iter().filter(|r| !r.text.trim().is_empty()).map(|r| serde_json::json!({
                        "text": r.text,
                        "box": { "x": r.x, "y": r.y + r.h, "w": r.w, "h": r.h },
                    })).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        });
        let _ = std::fs::write(format!("{dir}/{doc_index}.json"), dump.to_string());
    }
    let Ok(dir) = std::env::var("SPEEDYF_HOVER_TERMS") else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(format!("{dir}/{doc_index}.terms.json")) else {
        return;
    };
    let Ok(raw): Result<Vec<serde_json::Value>, _> = serde_json::from_str(&text) else {
        return;
    };
    let terms: Vec<super::terms::TermInput> = super::terms::sanitize(
        raw.iter()
            .filter_map(|t| {
                let rect = t.get("box").and_then(|b| {
                    let x = b.get("x")?.as_f64()? as f32;
                    let y = b.get("y")?.as_f64()? as f32;
                    let w = b.get("w")?.as_f64()? as f32;
                    let h = b.get("h")?.as_f64()? as f32;
                    Some([x, y - h, w, h])
                });
                Some(super::terms::TermInput {
                    text: t.get("text")?.as_str()?.to_string(),
                    case_sensitive: true,
                    kind: Some("acronym".into()),
                    title: t.get("text")?.as_str()?.to_string(),
                    body: t.get("body")?.as_str()?.to_string(),
                    target_page: t.get("page")?.as_u64()? as u32,
                    target_rect: rect,
                })
            })
            .collect(),
    );
    let _ = writeln!(report, "TERMS {name}: {}", terms.len());
    for (src, page) in pages.iter().enumerate() {
        let Some(page) = page else { continue };
        let page = super::prepare_page(page.clone(), result.region_start(src as u32));
        for span in page_spans(src as u32, None, Some((&page, None)), &terms, &[]) {
            if span.kind != "term" {
                continue;
            }
            let ctx = span
                .rects
                .first()
                .and_then(|r| locate(&page, *r))
                .map(|(s, e)| context(&page, s, e))
                .unwrap_or_default();
            let bodies: Vec<String> = span
                .target
                .terms
                .iter()
                .map(|t| format!("{} (p{})", t.body, t.page + 1))
                .collect();
            let _ = writeln!(
                report,
                "TERM p{} {} => {} | {}",
                src + 1,
                span.label,
                bodies.join(" || "),
                ctx.replace('\n', " ")
            );
        }
    }
}

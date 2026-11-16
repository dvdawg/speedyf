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

/// Where a line sits on a two-column page.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Left,
    Right,
    Span,
}

/// The page's gutter, read independently of the preview code: the x in the
/// middle of the page crossed by the fewest wide lines, accepted when both
/// sides of it carry a column's worth of lines.
fn truth_gutter(page: &Page) -> Option<f32> {
    let lines: Vec<&super::page::Line> = page
        .lines
        .iter()
        .filter(|l| !l.flags.furniture && l.x1 - l.x0 >= 40.0)
        .collect();
    let w = page.width;
    let mut best: Option<(usize, usize)> = None;
    let mut band: Vec<f32> = Vec::new();
    let mut g = 0.35 * w;
    while g <= 0.65 * w {
        let crossing = lines
            .iter()
            .filter(|l| l.x0 < g - 1.0 && l.x1 > g + 1.0)
            .count();
        let left = lines.iter().filter(|l| l.x1 <= g).count();
        let right = lines.iter().filter(|l| l.x0 >= g).count();
        let balance = left.min(right);
        match best {
            Some((c, b)) if crossing > c || (crossing == c && balance < b) => {}
            Some((c, b)) if crossing == c && balance == b => band.push(g),
            _ => {
                best = Some((crossing, balance));
                band = vec![g];
            }
        }
        g += 1.0;
    }
    let (crossing, balance) = best?;
    let total = lines.len();
    if balance < 8 || crossing as f32 > 0.25 * total as f32 {
        return None;
    }
    // the middle of the first contiguous band
    let mut end = band[0];
    for &x in &band[1..] {
        if x - end > 1.5 {
            break;
        }
        end = x;
    }
    Some((band[0] + end) / 2.0)
}

fn side_of(x0: f32, x1: f32, gutter: f32) -> Side {
    if x1 <= gutter + 1.0 {
        Side::Left
    } else if x0 >= gutter - 1.0 {
        Side::Right
    } else {
        Side::Span
    }
}

#[derive(Default)]
struct Tally {
    checked: usize,
    bleed: usize,
    /// Only mathematics from across the gutter: mostly the pieces of a
    /// full-width display, which a full-width crop rightly shows.
    bleed_math: usize,
    miss: usize,
    clip: usize,
    no_x: usize,
    full_width: usize,
}

impl Tally {
    fn line(&self, name: &str) -> String {
        format!(
            "{name:<10} checked {:>5}  bleed {:>4}  miss {:>4}  clip {:>4}  (bleed-math {}, no-x {}, full-width targets {})",
            self.checked, self.bleed, self.miss, self.clip, self.bleed_math, self.no_x, self.full_width
        )
    }

    fn add(&mut self, other: &Tally) {
        self.checked += other.checked;
        self.bleed += other.bleed;
        self.bleed_math += other.bleed_math;
        self.miss += other.miss;
        self.clip += other.clip;
        self.no_x += other.no_x;
        self.full_width += other.full_width;
    }
}

/// Judge one crop against the page's ground truth: `target` is the box the
/// preview must show, `side` the column it sits in.
fn judge(
    page: &Page,
    gutter: f32,
    crop: [f32; 4],
    target: [f32; 4],
    side: Side,
    tally: &mut Tally,
) -> Vec<String> {
    let mut problems = Vec::new();
    tally.checked += 1;
    if side == Side::Span {
        tally.full_width += 1;
    }
    let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]);
    let v_overlap = |l: &super::page::Line| (l.top.min(cy1) - l.bottom.max(cy0)) > 0.5 * l.height();
    if side != Side::Span {
        let other = if side == Side::Left {
            Side::Right
        } else {
            Side::Left
        };
        let offenders: Vec<usize> = (0..page.lines.len())
            .filter(|&i| {
                let l = &page.lines[i];
                !l.flags.furniture
                    && side_of(l.x0, l.x1, gutter) == other
                    && l.x1.min(cx1) - l.x0.max(cx0) > 3.0
                    && v_overlap(l)
            })
            .collect();
        let prose = offenders
            .iter()
            .copied()
            .find(|&i| !page.lines[i].is_display());
        if prose.is_none() && !offenders.is_empty() {
            tally.bleed_math += 1;
        }
        if let Some(i) = prose {
            tally.bleed += 1;
            let l = &page.lines[i];
            problems.push(format!(
                "BLEED other column: {:?} [{:.0},{:.0}] crop [{cx0:.0},{cy0:.0},{cx1:.0},{cy1:.0}]",
                page.line_text(i),
                l.x0,
                l.x1
            ));
        }
        if let Some(i) = (0..page.lines.len()).find(|&i| {
            let l = &page.lines[i];
            !l.flags.furniture
                && side_of(l.x0, l.x1, gutter) == Side::Span
                && v_overlap(l)
                && (l.x0 < cx0 - 6.0 || l.x1 > cx1 + 6.0)
        }) {
            tally.clip += 1;
            problems.push(format!(
                "CLIPS full-width line: {:?} crop [{cx0:.0},{cy0:.0},{cx1:.0},{cy1:.0}]",
                page.line_text(i)
            ));
        }
    }
    let (tx0, ty0, tx1, ty1) = (
        target[0],
        target[1],
        target[0] + target[2],
        target[1] + target[3],
    );
    let inside = tx0 >= cx0 - 1.5 && tx1 <= cx1 + 1.5 && ty0 >= cy0 - 1.5 && ty1 <= cy1 + 1.5;
    if !inside {
        tally.miss += 1;
        problems.push(format!(
            "MISSES target [{tx0:.0},{ty0:.0},{tx1:.0},{ty1:.0}] crop [{cx0:.0},{cy0:.0},{cx1:.0},{cy1:.0}]"
        ));
    }
    problems
}

/// The line a link destination lands on: the first line at or below the
/// anchor, in the anchor's column when it names an x.
fn link_target_line(page: &Page, gutter: f32, x: Option<f32>, y: f32) -> Option<(usize, Side)> {
    let want = x.map(|x| if x < gutter { Side::Left } else { Side::Right });
    page.lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            let side = side_of(l.x0, l.x1, gutter);
            !l.flags.furniture
                && l.top <= y + 3.0
                && l.top >= y - 60.0
                && l.x1 - l.x0 > 4.0
                && match want {
                    Some(w) => side == w || side == Side::Span,
                    None => true,
                }
        })
        .max_by(|a, b| a.1.top.total_cmp(&b.1.top))
        .map(|(i, l)| (i, side_of(l.x0, l.x1, gutter)))
}

fn line_box(l: &super::page::Line) -> [f32; 4] {
    [l.x0, l.bottom, l.x1 - l.x0, l.top - l.bottom]
}

type PageLinks = Vec<(Option<f32>, Option<f32>, u32)>;

/// Every preview a two-column page can show - link destinations and hover
/// targets - checked against the page's columns: the crop stays in the
/// target's column, holds the target, and does not cut through a
/// full-width element.
///
/// ```text
/// SPEEDYF_HOVER_EVAL=list.txt SPEEDYF_HOVER_OUT=report.txt \
///   cargo test --lib hovers::eval::evaluate_two_column -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn evaluate_two_column_previews() {
    let Ok(list) = std::env::var("SPEEDYF_HOVER_EVAL") else {
        return;
    };
    let out_path = std::env::var("SPEEDYF_HOVER_OUT").unwrap_or_else(|_| "preview-eval.txt".into());
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
    let mut detail = String::new();
    let mut totals: std::collections::BTreeMap<String, Tally> = Default::default();
    let mut docs_two = 0usize;
    let mut columns_seen: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for path in paths {
        let Ok(raw) = render::open_document(b, &path, None) else {
            continue;
        };
        let count = (b.FPDF_GetPageCount(raw).max(0) as u32).min(60);
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut builder = IndexBuilder::new(count);
        let mut pages: Vec<Option<Page>> = Vec::new();
        let mut runs: Vec<Vec<crate::engine::types::TextRun>> = Vec::new();
        let mut sizes: Vec<[f32; 2]> = Vec::new();
        let mut links_of: Vec<PageLinks> = Vec::new();
        while let Some(src) = builder.next_page() {
            let size = render::page_display_size(b, raw, src as i32);
            let handle = b.FPDF_LoadPage(raw, src as i32);
            let model = match (size, handle.is_null()) {
                (Some([w, h]), false) => {
                    let input = extract::extract(b, handle, w, h);
                    let extracted = text::extract_page(b, handle, w, h);
                    let tp = b.FPDFText_LoadPage(handle);
                    let found =
                        links::extract_links(b, raw, handle, tp, [w, h], &extracted.boxes, count);
                    if !tp.is_null() {
                        b.FPDFText_ClosePage(tp);
                    }
                    links_of.push(
                        found
                            .into_iter()
                            .filter_map(|l| match l.target {
                                LinkTarget::Internal { page, x, y } => Some((x, y, page)),
                                _ => None,
                            })
                            .collect(),
                    );
                    runs.push(extracted.runs);
                    sizes.push([w, h]);
                    Some(Page::new(input))
                }
                _ => {
                    links_of.push(Vec::new());
                    runs.push(Vec::new());
                    sizes.push([0.0, 0.0]);
                    None
                }
            };
            if !handle.is_null() {
                b.FPDF_ClosePage(handle);
            }
            builder.add_page(model.clone());
            pages.push(model.map(|p| super::prepare_page(p, Default::default())));
        }
        let result: IndexResult = builder.finish();
        let gutters: Vec<Option<f32>> = pages
            .iter()
            .map(|p| p.as_ref().and_then(truth_gutter))
            .collect();
        let with_text = pages
            .iter()
            .filter(|p| p.as_ref().is_some_and(|p| p.lines.len() > 20))
            .count();
        let two = gutters.iter().filter(|g| g.is_some()).count();
        // Whether the preview code reads each page's columns as the ground
        // truth does: two where there is a gutter, one where there is not.
        for (src, page) in pages.iter().enumerate() {
            let Some(page) = page.as_ref().filter(|p| p.lines.len() > 20) else {
                continue;
            };
            let found = crate::engine::preview::page_columns(&runs[src], sizes[src][0]).len();
            let expected = if gutters[src].is_some() { 2 } else { 1 };
            let key = if found == expected {
                "agree"
            } else if expected == 1 {
                "false-split"
            } else {
                "missed-gutter"
            };
            *columns_seen.entry(key).or_default() += 1;
            if found != expected {
                let _ = writeln!(
                    detail,
                    "{name} p{} COLUMNS found {found}, truth {expected} ({} lines)",
                    src + 1,
                    page.lines.len()
                );
            }
        }
        if with_text == 0 || (two as f32) < 0.3 * with_text as f32 {
            let _ = writeln!(report, "skip {name}: {two}/{with_text} two-column pages");
            b.FPDF_CloseDocument(raw);
            continue;
        }
        docs_two += 1;
        let _ = writeln!(
            report,
            "\n=== {name}: {two}/{with_text} two-column pages ==="
        );
        let mut doc_tallies: std::collections::BTreeMap<String, Tally> = Default::default();
        // link destinations
        let mut seen = std::collections::HashSet::new();
        for (src, links_here) in links_of.iter().enumerate() {
            for &(x, y, to) in links_here {
                let Some(y) = y else { continue };
                if !seen.insert((to, x.map(f32::to_bits), y.to_bits())) {
                    continue;
                }
                let (Some(Some(page)), Some(Some(gutter))) =
                    (pages.get(to as usize), gutters.get(to as usize))
                else {
                    continue;
                };
                let Some((line, side)) = link_target_line(page, *gutter, x, y) else {
                    continue;
                };
                let [w, h] = sizes[to as usize];
                let crop = crate::engine::preview::crop_rect(&crate::engine::preview::CropInput {
                    page_w_pt: w,
                    page_h_pt: h,
                    dest_x: x,
                    dest_y: Some(y),
                    runs: &runs[to as usize],
                });
                let tally = doc_tallies.entry("link".into()).or_default();
                if x.is_none() {
                    tally.no_x += 1;
                }
                let problems = judge(
                    page,
                    *gutter,
                    crop,
                    line_box(&page.lines[line]),
                    side,
                    tally,
                );
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{name} link p{}->p{} x={x:?} y={y:.0} {side:?} {:?}\n    {}",
                        src + 1,
                        to + 1,
                        page.line_text(line),
                        problems.join("\n    ")
                    );
                }
            }
        }
        // hover targets
        let mut seen = std::collections::HashSet::new();
        let mut keys: Vec<&u32> = result.spans.keys().collect();
        keys.sort();
        for src in keys {
            for span in &result.spans[src] {
                let Some(region) = span.target.region else {
                    continue;
                };
                let to = span.target.page as usize;
                if !seen.insert((span.kind, to, region.map(f32::to_bits))) {
                    continue;
                }
                let (Some(Some(page)), Some(Some(gutter))) = (pages.get(to), gutters.get(to))
                else {
                    continue;
                };
                // The thing the preview must show.
                let target: Option<[f32; 4]> = if span.kind == "equation" {
                    let number = span.label.rsplit(' ').next().unwrap_or("");
                    result
                        .statements
                        .iter()
                        .find(|s| {
                            s.kind == "equation" && s.page as usize == to && s.number == number
                        })
                        .and_then(|s| {
                            s.label_rects.iter().copied().reduce(|a, b| {
                                let x0 = a[0].min(b[0]);
                                let y0 = a[1].min(b[1]);
                                let x1 = (a[0] + a[2]).max(b[0] + b[2]);
                                let y1 = (a[1] + a[3]).max(b[1] + b[3]);
                                [x0, y0, x1 - x0, y1 - y0]
                            })
                        })
                } else {
                    let (jx, jy) = (span.target.x.unwrap_or(0.0), span.target.y);
                    page.lines
                        .iter()
                        .filter(|l| (l.top - jy).abs() < 2.0 && (l.x0 - jx).abs() < 2.0)
                        .map(line_box)
                        .next()
                };
                let Some(target) = target else { continue };
                let side = side_of(target[0], target[0] + target[2], *gutter);
                let tally = doc_tallies.entry(span.kind.to_string()).or_default();
                let problems = judge(page, *gutter, region, target, side, tally);
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{name} {} p{}->p{} {side:?} {}\n    {}",
                        span.label,
                        src + 1,
                        to + 1,
                        region_text(page, region),
                        problems.join("\n    ")
                    );
                }
            }
        }
        for (kind, tally) in doc_tallies {
            let _ = writeln!(report, "  {}", tally.line(&kind));
            totals.entry(kind).or_default().add(&tally);
        }
        b.FPDF_CloseDocument(raw);
    }
    let _ = writeln!(report, "\nCOLUMN DETECTION (pages) {columns_seen:?}");
    let _ = writeln!(report, "\nTOTALS over {docs_two} two-column documents");
    for (kind, tally) in &totals {
        let _ = writeln!(report, "  {}", tally.line(kind));
    }
    let _ = writeln!(report, "\nPROBLEMS\n{detail}");
    std::fs::write(&out_path, &report).expect("write report");
    eprintln!("wrote {out_path}");
}

/// Pages' runs and the columns the preview code finds on them, for
/// debugging a crop: `SPEEDYF_PREVIEW_PAGE=a.pdf:3;b.pdf:1` (1-based),
/// with `SPEEDYF_PREVIEW_RUNS=1` to list the runs and
/// `SPEEDYF_PREVIEW_DEST=x,y` to print the crop for a destination.
#[test]
#[ignore]
fn dump_page_columns() {
    let Ok(spec) = std::env::var("SPEEDYF_PREVIEW_PAGE") else {
        return;
    };
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    for one in spec.split(';') {
        let (path, page) = one.rsplit_once(':').expect("path:page");
        let src: i32 = page.parse::<i32>().expect("page") - 1;
        let raw = render::open_document(b, path, None).expect("open");
        let [w, h] = render::page_display_size(b, raw, src).expect("size");
        let handle = b.FPDF_LoadPage(raw, src);
        let extracted = text::extract_page(b, handle, w, h);
        eprintln!("== {one}: page {w}x{h}");
        if std::env::var("SPEEDYF_PREVIEW_RUNS").is_ok() {
            for run in &extracted.runs {
                eprintln!(
                    "run x {:7.1}..{:7.1} y {:7.1}..{:7.1} {:?}",
                    run.x,
                    run.x + run.w,
                    run.y,
                    run.y + run.h,
                    run.text.chars().take(50).collect::<String>()
                );
            }
        }
        eprintln!(
            "columns {:?}",
            crate::engine::preview::page_columns(&extracted.runs, w)
        );
        if let Ok(dest) = std::env::var("SPEEDYF_PREVIEW_DEST") {
            let (x, y) = dest.split_once(',').expect("x,y");
            let crop = crate::engine::preview::crop_rect(&crate::engine::preview::CropInput {
                page_w_pt: w,
                page_h_pt: h,
                dest_x: x.parse().ok(),
                dest_y: y.parse().ok(),
                runs: &extracted.runs,
            });
            eprintln!("crop {crop:?}");
            for block in crate::engine::floats::caption_blocks(&extracted.runs) {
                eprintln!("caption {block:?}");
            }
        }
        b.FPDF_ClosePage(handle);
        b.FPDF_CloseDocument(raw);
    }
}

/// What a link's text names: ("figure" or "table", its number), read from
/// the link and the words just before it ("Fig.~[11]", "Table [3]").
fn float_reference(before: &str, inside: &str) -> Option<(&'static str, String)> {
    let number: String = inside
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .rfind(|t| t.chars().any(|c| c.is_ascii_digit()))?
        .trim_matches('.')
        .to_string();
    let words = format!("{before} {inside}").to_lowercase();
    let head = words
        .trim_end()
        .trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == ']');
    let head = head.trim_end().trim_end_matches(['~', '(', '[']).trim_end();
    let kind = if ["fig.", "figure", "figs.", "figures", "fig"]
        .iter()
        .any(|w| head.ends_with(w))
    {
        "figure"
    } else if ["table", "tables", "tab."]
        .iter()
        .any(|w| head.ends_with(w))
    {
        "table"
    } else {
        return None;
    };
    Some((kind, number))
}

/// A caption on a page: its lines, head first.
fn caption_lines(page: &Page, head: usize) -> Vec<usize> {
    let mut out = vec![head];
    let first = &page.lines[head];
    let em = first.em();
    while out.len() < 20 {
        let previous = &page.lines[*out.last().unwrap_or(&head)];
        let next = (0..page.lines.len())
            .filter(|&i| {
                let l = &page.lines[i];
                !out.contains(&i)
                    && l.top <= previous.bottom + 0.6 * em
                    && l.top > previous.bottom - 0.6 * em
                    && l.top < previous.top
                    && super::page::same_column(l, first)
                    && (l.size - first.size).abs() <= 0.15 * first.size.max(1.0)
                    && crate::engine::figures::parse_caption(&page.line_text(i)).is_none()
            })
            .max_by(|&a, &b| page.lines[a].top.total_cmp(&page.lines[b].top));
        match next {
            Some(i) => out.push(i),
            None => break,
        }
    }
    out
}

/// Words, mostly: a sentence, not a table row of numbers and short heads.
fn reads_as_prose(text: &str) -> bool {
    let words = text
        .split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 3)
        .count();
    let digits = text.chars().filter(char::is_ascii_digit).count();
    words >= 6 && digits * 5 < text.chars().count()
}

#[derive(Default)]
struct FloatTally {
    checked: usize,
    caption_missing: usize,
    other_caption: usize,
    prose: usize,
    cut: usize,
    no_artwork: usize,
}

impl FloatTally {
    fn add(&mut self, t: &FloatTally) {
        self.checked += t.checked;
        self.caption_missing += t.caption_missing;
        self.other_caption += t.other_caption;
        self.prose += t.prose;
        self.cut += t.cut;
        self.no_artwork += t.no_artwork;
    }

    fn line(&self, name: &str) -> String {
        format!(
            "{name:<40} checked {:>4}  caption-cut {:>3}  other-caption {:>3}  prose {:>3}  cut-line {:>3}  no-artwork {:>3}",
            self.checked, self.caption_missing, self.other_caption, self.prose, self.cut, self.no_artwork
        )
    }
}

/// Judge a float preview: it holds the whole matching caption and some
/// artwork, and no other caption, prose line or half a line.
fn judge_float(
    page: &Page,
    crop: [f32; 4],
    caption: &[usize],
    check_prose: bool,
    tally: &mut FloatTally,
) -> Vec<String> {
    tally.checked += 1;
    let mut problems = Vec::new();
    let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]);
    let inside = |l: &super::page::Line| {
        l.x0 >= cx0 - 2.0 && l.x1 <= cx1 + 2.0 && l.bottom >= cy0 - 1.5 && l.top <= cy1 + 1.5
    };
    if let Some(&i) = caption.iter().find(|&&i| !inside(&page.lines[i])) {
        tally.caption_missing += 1;
        problems.push(format!("CAPTION cut: {:?}", page.line_text(i)));
    }
    let mut other = None;
    let mut prose = None;
    let mut cut = None;
    for (i, l) in page.lines.iter().enumerate() {
        if l.flags.furniture || caption.contains(&i) || l.x1.min(cx1) - l.x0.max(cx0) <= 3.0 {
            continue;
        }
        let share = ((l.top.min(cy1) - l.bottom.max(cy0)) / l.height()).max(0.0);
        if share > 0.5 {
            if crate::engine::figures::parse_caption(&page.line_text(i)).is_some() {
                other.get_or_insert(i);
            } else if check_prose
                && !l.is_display()
                && l.font == page.body_font
                && (l.size - page.body_size).abs() < 0.3
                && l.x1 - l.x0 >= 150.0
                && l.x0 >= cx0 - 2.0
                && l.x1 <= cx1 + 2.0
                && reads_as_prose(&page.line_text(i))
            {
                prose.get_or_insert(i);
            }
        }
        // A lone script or a rotated label's letter is no line to cut.
        if share > 0.15 && share < 0.85 && page.line_text(i).trim().chars().count() >= 3 {
            cut.get_or_insert(i);
        }
    }
    if let Some(i) = other {
        tally.other_caption += 1;
        problems.push(format!("OTHER caption: {:?}", page.line_text(i)));
    }
    if let Some(i) = prose {
        tally.prose += 1;
        problems.push(format!("PROSE: {:?}", page.line_text(i)));
    }
    if let Some(i) = cut {
        tally.cut += 1;
        problems.push(format!("CUTS line: {:?}", page.line_text(i)));
    }
    let top = caption
        .iter()
        .map(|&i| page.lines[i].top)
        .fold(f32::MIN, f32::max);
    let bottom = caption
        .iter()
        .map(|&i| page.lines[i].bottom)
        .fold(f32::MAX, f32::min);
    if crop[3] < top - bottom + 25.0 {
        tally.no_artwork += 1;
        problems.push("NO artwork".into());
    }
    problems
}

type FloatLinks = Vec<([f32; 4], Option<f32>, Option<f32>, u32)>;

/// Every link to a figure or table, by the number its text names, checked
/// against the float the preview crop shows. Same list format as above.
#[test]
#[ignore]
fn evaluate_float_previews() {
    let Ok(list) = std::env::var("SPEEDYF_HOVER_EVAL") else {
        return;
    };
    let out_path = std::env::var("SPEEDYF_HOVER_OUT").unwrap_or_else(|_| "float-eval.txt".into());
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
    let mut detail = String::new();
    let mut total = FloatTally::default();
    for path in paths {
        let Ok(raw) = render::open_document(b, &path, None) else {
            continue;
        };
        let count = (b.FPDF_GetPageCount(raw).max(0) as u32).min(60);
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut pages: Vec<Option<Page>> = Vec::new();
        let mut runs: Vec<Vec<crate::engine::types::TextRun>> = Vec::new();
        let mut sizes: Vec<[f32; 2]> = Vec::new();
        let mut links_of: Vec<FloatLinks> = Vec::new();
        for src in 0..count {
            let size = render::page_display_size(b, raw, src as i32);
            let handle = b.FPDF_LoadPage(raw, src as i32);
            match (size, handle.is_null()) {
                (Some([w, h]), false) => {
                    let input = extract::extract(b, handle, w, h);
                    let extracted = text::extract_page(b, handle, w, h);
                    let tp = b.FPDFText_LoadPage(handle);
                    let found =
                        links::extract_links(b, raw, handle, tp, [w, h], &extracted.boxes, count);
                    if !tp.is_null() {
                        b.FPDFText_ClosePage(tp);
                    }
                    links_of.push(
                        found
                            .into_iter()
                            .filter_map(|l| match l.target {
                                LinkTarget::Internal { page, x, y } => Some((l.rect, x, y, page)),
                                _ => None,
                            })
                            .collect(),
                    );
                    runs.push(extracted.runs);
                    sizes.push([w, h]);
                    pages.push(Some(super::prepare_page(
                        Page::new(input),
                        Default::default(),
                    )));
                }
                _ => {
                    links_of.push(Vec::new());
                    runs.push(Vec::new());
                    sizes.push([0.0, 0.0]);
                    pages.push(None);
                }
            }
            if !handle.is_null() {
                b.FPDF_ClosePage(handle);
            }
        }
        let mut tally = FloatTally::default();
        let mut seen = std::collections::HashSet::new();
        for (src, links_here) in links_of.iter().enumerate() {
            let Some(source) = pages[src].as_ref() else {
                continue;
            };
            for &(rect, x, y, to) in links_here {
                let Some((start, end)) = locate(source, rect) else {
                    continue;
                };
                let before: String = source.text[start.saturating_sub(14)..start]
                    .iter()
                    .collect();
                let inside: String = source.text[start..end].iter().collect();
                let Some((kind, number)) = float_reference(&before, &inside) else {
                    continue;
                };
                let Some(Some(page)) = pages.get(to as usize) else {
                    continue;
                };
                let head = (0..page.lines.len()).find(|&i| {
                    crate::engine::figures::parse_caption(&page.line_text(i)).is_some_and(
                        |(label, _)| {
                            let lower = label.to_lowercase();
                            lower.starts_with(&kind[..3])
                                && lower.rsplit(' ').next() == Some(number.as_str())
                        },
                    )
                });
                let Some(head) = head else { continue };
                if !seen.insert((to, head, x.map(f32::to_bits), y.map(f32::to_bits))) {
                    continue;
                }
                let [w, h] = sizes[to as usize];
                let crop = crate::engine::preview::crop_rect(&crate::engine::preview::CropInput {
                    page_w_pt: w,
                    page_h_pt: h,
                    dest_x: x,
                    dest_y: y,
                    runs: &runs[to as usize],
                });
                let caption = caption_lines(page, head);
                // A table's rows can read as prose to this check: only a
                // figure's crop is held to it.
                let problems = judge_float(page, crop, &caption, kind == "figure", &mut tally);
                if !problems.is_empty() {
                    let _ = writeln!(
                        detail,
                        "{name} {kind} {number} p{}->p{} x={x:?} y={y:?} crop [{:.0},{:.0},{:.0},{:.0}]\n    {}",
                        src + 1,
                        to + 1,
                        crop[0],
                        crop[1],
                        crop[0] + crop[2],
                        crop[1] + crop[3],
                        problems.join("\n    ")
                    );
                }
            }
        }
        if tally.checked > 0 {
            let _ = writeln!(report, "{}", tally.line(&name));
        }
        total.add(&tally);
        b.FPDF_CloseDocument(raw);
    }
    let _ = writeln!(report, "\n{}", total.line("TOTAL"));
    let _ = writeln!(report, "\nPROBLEMS\n{detail}");
    std::fs::write(&out_path, &report).expect("write report");
    eprintln!("wrote {out_path}");
}

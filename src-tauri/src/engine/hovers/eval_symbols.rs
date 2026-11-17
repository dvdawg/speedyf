//! Symbol definitions over real PDFs, read-only. Ignored by default:
//!
//! ```text
//! SPEEDYF_SYMBOL_EVAL=list.txt SPEEDYF_SYMBOL_OUT=report.txt \
//!   SPEEDYF_SYMBOL_SAMPLE=sample.txt \
//!   cargo test --release --lib hovers::eval_symbols -- --ignored --nocapture
//! ```
//!
//! For every document: how many symbols are seen, how many are offered a
//! preview, how many of those rest on a definition cue (their own or their
//! base's) and how many on a first occurrence; how long reading them takes
//! next to the rest of the index. The sample file lists symbols drawn
//! evenly from all documents with the sentence chosen as their definition,
//! for judging by hand.

use super::page::Page;
use super::symbols::{Collector, Definition, Key, Via};
use super::{extract, IndexBuilder};
use crate::engine::{pdfium_init, render};
use pdfium_render::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::time::Instant;

/// The text of the lines whose middle is inside `region`.
fn region_text(page: &Page, region: [f32; 4]) -> String {
    let mut out = String::new();
    for (i, l) in page.lines.iter().enumerate() {
        let (cx, cy) = ((l.x0 + l.x1) / 2.0, (l.top + l.bottom) / 2.0);
        if cx >= region[0] - 1.0
            && cx <= region[0] + region[2] + 1.0
            && cy >= region[1] - 1.0
            && cy <= region[1] + region[3] + 1.0
            && !l.flags.furniture
        {
            if !out.is_empty() {
                out.push_str(" / ");
            }
            out.push_str(&page.line_text(i));
        }
    }
    out
}

/// The text of the line an occurrence is on.
fn line_at(page: &Page, rect: [f32; 4]) -> String {
    let cy = rect[1] + rect[3] / 2.0;
    let cx = rect[0] + rect[2] / 2.0;
    page.lines
        .iter()
        .position(|l| cy >= l.bottom && cy <= l.top && cx >= l.x0 - 1.0 && cx <= l.x1 + 1.0)
        .map(|i| page.line_text(i))
        .unwrap_or_default()
}

/// A small deterministic hash, for an even sample.
fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

struct Sampled {
    doc: String,
    label: String,
    how: String,
    page: u32,
    definition: String,
    use_page: u32,
    use_line: String,
    weight: u64,
}

#[test]
#[ignore]
fn evaluate_symbol_definitions() {
    let Ok(list) = std::env::var("SPEEDYF_SYMBOL_EVAL") else {
        return;
    };
    let out_path = std::env::var("SPEEDYF_SYMBOL_OUT").unwrap_or_else(|_| "symbols.txt".into());
    let sample_path = std::env::var("SPEEDYF_SYMBOL_SAMPLE").ok();
    let sample_size: usize = std::env::var("SPEEDYF_SYMBOL_SAMPLE_N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(30);
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
    let mut totals: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut cues: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sampled: Vec<Sampled> = Vec::new();
    let (mut symbols_ms, mut index_ms, mut finish_ms, mut worst_page_ms) = (0f64, 0f64, 0f64, 0f64);
    let mut total_pages = 0u32;
    for path in paths {
        let Ok(raw) = render::open_document(b, &path, None) else {
            continue;
        };
        let count = b.FPDF_GetPageCount(raw).max(0) as u32;
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut builder = IndexBuilder::new(count);
        let mut collector = Collector::new();
        let mut state = super::regions::RegionState::default();
        let mut pages: Vec<Option<Page>> = Vec::new();
        let mut doc_symbols_ms = 0f64;
        let started = Instant::now();
        while let Some(src) = builder.next_page() {
            let size = render::page_display_size(b, raw, src as i32);
            let handle = b.FPDF_LoadPage(raw, src as i32);
            let model = match (size, handle.is_null()) {
                (Some([w, h]), false) => {
                    let mut page = Page::new(extract::extract(b, handle, w, h));
                    let displays = extract::index_displays(b, handle, &page);
                    page.set_displays(displays);
                    Some(page)
                }
                _ => None,
            };
            if !handle.is_null() {
                b.FPDF_ClosePage(handle);
            }
            // The symbols alone, timed apart from the rest of the index.
            let classified = model.clone().map(|mut p| {
                super::regions::classify(&mut p, &mut state);
                p
            });
            if let Some(p) = &classified {
                let t = Instant::now();
                collector.add_page(p, src);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                doc_symbols_ms += ms;
                worst_page_ms = worst_page_ms.max(ms);
            }
            builder.add_page(model);
            pages.push(classified);
        }
        let t = Instant::now();
        let resolved = collector.resolve();
        let spans = collector.spans();
        let doc_finish_ms = t.elapsed().as_secs_f64() * 1000.0;
        let _ = builder.finish();
        let doc_index_ms = started.elapsed().as_secs_f64() * 1000.0;
        total_pages += count.min(super::MAX_INDEX_PAGES);
        symbols_ms += doc_symbols_ms;
        finish_ms += doc_finish_ms;
        index_ms += doc_index_ms;

        let occurrences = collector.occurrences();
        let mut seen: HashMap<&Key, usize> = HashMap::new();
        for o in occurrences {
            *seen.entry(&o.key).or_default() += 1;
        }
        let span_count: usize = spans.values().map(Vec::len).sum();
        // Each (symbol, definition) pair some span points at, with its
        // first use.
        let mut pairs: HashMap<(usize, &Key), (Definition, usize)> = HashMap::new();
        for (i, o) in occurrences.iter().enumerate() {
            let Some((c, d)) = &resolved[i] else { continue };
            let inside = o.page == d.page && super::symbols::centre_in(o.rect, d.region);
            if *c == i || inside {
                continue;
            }
            // Keyed by definition and symbol: theta_1 and theta_2 sharing
            // theta's definition are two decisions.
            pairs.entry((*c, &o.key)).or_insert((d.clone(), i));
        }
        let mut pairs: Vec<(&Key, Definition, usize)> = {
            let mut v: Vec<_> = pairs
                .into_iter()
                .map(|((c, k), (d, i))| (c, k, d, i))
                .collect();
            v.sort_by_key(|p| (p.0, p.3));
            v.into_iter().map(|(_, k, d, i)| (k, d, i)).collect()
        };
        pairs.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        let mut offered_keys: HashMap<&Key, ()> = HashMap::new();
        let mut by_via: BTreeMap<&'static str, usize> = BTreeMap::new();
        for (key, d, use_at) in &pairs {
            offered_keys.insert(key, ());
            *by_via.entry(d.via.name()).or_default() += 1;
            if let Some(c) = d.cue {
                *cues.entry(c.name()).or_default() += 1;
            }
            *totals.entry(d.via.name()).or_default() += 1;
            let how = match (d.cue, d.via) {
                (Some(c), Via::Own) => c.name().to_string(),
                (Some(c), via) => format!("{} (via {})", c.name(), via.name()),
                (None, _) => "first occurrence".to_string(),
            };
            let o = &occurrences[*use_at];
            let definition = pages
                .get(d.page as usize)
                .and_then(|p| p.as_ref())
                .map(|p| region_text(p, d.region))
                .unwrap_or_default();
            let use_line = pages
                .get(o.page as usize)
                .and_then(|p| p.as_ref())
                .map(|p| line_at(p, o.rect))
                .unwrap_or_default();
            let label = key.label();
            let mut h = 0u64;
            for c in format!("{name}/{label}/{}/{}", key.family, d.page).bytes() {
                h = mix(h ^ u64::from(c));
            }
            sampled.push(Sampled {
                doc: name.clone(),
                label: format!("{label} [{}]", key.family),
                how,
                page: d.page,
                definition,
                use_page: o.page,
                use_line,
                weight: h,
            });
        }
        let _ = writeln!(
            report,
            "{name}: {count} pages; {} symbols seen, {} offered; {} symbol-definition pairs {by_via:?}; {} spans; symbols {:.1} ms + {:.1} ms finish; index {:.0} ms",
            seen.len(),
            offered_keys.len(),
            pairs.len(),
            span_count,
            doc_symbols_ms,
            doc_finish_ms,
            doc_index_ms
        );
        *totals.entry("docs").or_default() += 1;
        *totals.entry("seen").or_default() += seen.len();
        *totals.entry("offered").or_default() += offered_keys.len();
        *totals.entry("pairs").or_default() += pairs.len();
        *totals.entry("spans").or_default() += span_count;
        b.FPDF_CloseDocument(raw);
    }
    let _ = writeln!(
        report,
        "\nTOTALS {totals:?}\ncues {cues:?}\npages {total_pages}: symbols {symbols_ms:.0} ms ({:.2} ms/page, worst page {worst_page_ms:.1} ms), finish {finish_ms:.0} ms, whole index {index_ms:.0} ms",
        symbols_ms / f64::from(total_pages.max(1))
    );
    std::fs::write(&out_path, &report).expect("write report");
    eprintln!("{report}");
    if let Some(sample_path) = sample_path {
        sampled.sort_by_key(|s| s.weight);
        let mut out = String::new();
        for (i, s) in sampled.iter().take(sample_size).enumerate() {
            let _ = writeln!(
                out,
                "#{} {} | {} | {} | p{}\n  DEF: {}\n  USE p{}: {}\n",
                i + 1,
                s.doc,
                s.label,
                s.how,
                s.page + 1,
                s.definition,
                s.use_page + 1,
                s.use_line
            );
        }
        std::fs::write(&sample_path, out).expect("write sample");
    }
}

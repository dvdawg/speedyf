//! Auto-detect measured against real forms.
//!
//! Ignored by default: it needs PDFs that are not in the repository - the
//! same pairs the blank detector is measured on (`blanks_eval`): fillable
//! forms, and copies of them made flat (widgets and AcroForm removed). The
//! flat copy is what auto-detect sees; the original's widgets are the
//! answer. A proposal matches a widget by the blank eval's position rule,
//! and by kind when both are text-like (text, combo, list, signature) or
//! both are buttons (checkbox, radio).
//!
//! ```text
//! SPEEDYF_BLANKS_FORMS=/path/to/fillable SPEEDYF_BLANKS_FLAT=/path/to/flat \
//!   cargo test --release --lib prepare_eval -- --ignored --nocapture
//! ```
//!
//! Names are scored too: the share of matched proposals whose name shares
//! a word (of three letters or more) with the original field's tooltip.

use super::{propose, read_form};
use crate::engine::blanks;
use crate::engine::blanks_eval::{matches, pdfs};
use crate::engine::pdfium_init;
use crate::engine::render;
use crate::engine::types::ProposedFieldDto;
use pdfium_render::prelude::*;
use std::collections::HashSet;
use std::path::Path;

fn class(kind: &str) -> &'static str {
    match kind {
        "checkbox" | "radio" => "button",
        _ => "text",
    }
}

fn words(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_lowercase)
        .collect()
}

#[derive(Default)]
struct Row {
    truth: usize,
    found: usize,
    found_kind: usize,
    proposed: usize,
    true_proposed: usize,
    true_kind: usize,
    named: usize,
    name_hits: usize,
}

#[test]
#[ignore]
fn prepare_eval() {
    let (Ok(forms), Ok(flat)) = (
        std::env::var("SPEEDYF_BLANKS_FORMS"),
        std::env::var("SPEEDYF_BLANKS_FLAT"),
    ) else {
        return;
    };
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();
    let mut total = Row::default();
    println!();
    println!(
        "{:<12} {:>6} {:>6} {:>8} {:>8} {:>8} {:>8} {:>7}",
        "form", "truth", "prop", "recall", "+kind", "prec", "+kind", "names"
    );
    for original in pdfs(&forms) {
        let name = original.file_name().unwrap().to_string_lossy().into_owned();
        let flat_path = Path::new(&flat).join(&name);
        if !flat_path.exists() {
            continue;
        }
        let path = original.to_string_lossy().into_owned();
        let Ok(raw) = render::open_document(b, &path, None) else {
            continue;
        };
        let pages = b.FPDF_GetPageCount(raw).max(0) as u32;
        let form = read_form(b, raw, pages, &path, None);
        b.FPDF_CloseDocument(raw);
        let truth: Vec<_> = form
            .fields
            .iter()
            .filter(|f| f.kind != "button" && f.kind != "other" && f.rect.w > 1.0 && f.rect.h > 1.0)
            .collect();
        if truth.is_empty() {
            continue;
        }
        let flat_str = flat_path.to_string_lossy().into_owned();
        let doc = render::open_document(b, &flat_str, None).expect("flat copy opens");
        let mut taken = HashSet::new();
        let mut proposals: Vec<ProposedFieldDto> = Vec::new();
        for page in 0..pages {
            let Some([w, h]) = render::page_display_size(b, doc, page as i32) else {
                continue;
            };
            let handle = b.FPDF_LoadPage(doc, page as i32);
            if handle.is_null() {
                continue;
            }
            let input = blanks::collect(b, handle, w, h);
            let found = blanks::detect(&input);
            proposals.extend(propose(page, &input, &found, &mut taken));
            b.FPDF_ClosePage(handle);
        }
        b.FPDF_CloseDocument(doc);

        let mut row = Row {
            truth: truth.len(),
            proposed: proposals.len(),
            ..Row::default()
        };
        for t in &truth {
            let hits: Vec<_> = proposals
                .iter()
                .filter(|p| p.page == t.page && matches(&t.rect, &p.rect))
                .collect();
            if !hits.is_empty() {
                row.found += 1;
            }
            if hits.iter().any(|p| class(&p.kind) == class(&t.kind)) {
                row.found_kind += 1;
            }
        }
        for p in &proposals {
            let hits: Vec<_> = truth
                .iter()
                .filter(|t| t.page == p.page && matches(&t.rect, &p.rect))
                .collect();
            if hits.is_empty() {
                continue;
            }
            row.true_proposed += 1;
            if hits.iter().any(|t| class(&t.kind) == class(&p.kind)) {
                row.true_kind += 1;
            }
            if let Some(t) = hits.iter().find(|t| !t.tooltip.trim().is_empty()) {
                row.named += 1;
                if !words(&p.name).is_disjoint(&words(&t.tooltip)) {
                    row.name_hits += 1;
                }
            }
        }
        print_row(&name, &row);
        total.truth += row.truth;
        total.found += row.found;
        total.found_kind += row.found_kind;
        total.proposed += row.proposed;
        total.true_proposed += row.true_proposed;
        total.true_kind += row.true_kind;
        total.named += row.named;
        total.name_hits += row.name_hits;
    }
    print_row("ALL", &total);
}

fn print_row(name: &str, r: &Row) {
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    println!(
        "{:<12} {:>6} {:>6} {:>7.1}% {:>7.1}% {:>7.1}% {:>7.1}% {:>6.1}%",
        name,
        r.truth,
        r.proposed,
        pct(r.found, r.truth),
        pct(r.found_kind, r.truth),
        pct(r.true_proposed, r.proposed),
        pct(r.true_kind, r.proposed),
        pct(r.name_hits, r.named),
    );
}

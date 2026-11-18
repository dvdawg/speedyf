//! Field-script evaluation over real forms, in two halves around the
//! frontend's helpers.
//!
//! Ignored by default: it needs PDFs that are not in the repository.
//!
//! ```text
//! export SPEEDYF_FORMS_EVAL_DIR=/path/to/forms SPEEDYF_FORMS_EVAL_OUT=/path/to/out
//! cargo test --lib forms::behaviour_eval::dump -- --ignored --nocapture
//! node scripts/forms-behaviour-eval.mjs "$SPEEDYF_FORMS_EVAL_OUT"   # fills, calculates, formats
//! cargo test --lib forms::behaviour_eval::save -- --ignored --nocapture
//! ```
//!
//! `dump` reads each form (and the invoice fixture, written beside them)
//! and prints how many script actions it has and how many were recognised,
//! by kind, writing `<form>.json` for the frontend half. That half - the
//! same formScripts.ts the app runs - picks a valid value for every field
//! with a recognised format, recalculates, formats, and writes
//! `<form>.plan.json`. `save` saves each plan through the app's own
//! `build_output`, reopens the result and checks every /V is the raw value
//! and every appearance stream draws the formatted one.

use super::behaviour_tests::{
    appearance_text, field_value, widgets_by_name, write_invoice_fixture,
};
use super::read_form;
use crate::engine::pdfium_init;
use crate::engine::types::{EditPlan, FormDto, PlanPage};
use pdfium_render::prelude::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn dirs() -> Option<(PathBuf, PathBuf)> {
    let input = PathBuf::from(std::env::var("SPEEDYF_FORMS_EVAL_DIR").ok()?);
    let out = PathBuf::from(std::env::var("SPEEDYF_FORMS_EVAL_OUT").ok()?);
    std::fs::create_dir_all(&out).ok()?;
    Some((input, out))
}

fn forms(input: &Path, out: &Path) -> Vec<PathBuf> {
    let invoice = out.join("invoice.pdf");
    write_invoice_fixture(&invoice);
    let mut list: Vec<PathBuf> = std::fs::read_dir(input)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    list.sort();
    list.push(invoice);
    list
}

fn read(pdfium: &Pdfium, path: &Path) -> (FormDto, Vec<(f32, f32)>) {
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(path.to_str().unwrap(), None);
    assert!(!doc.is_null(), "cannot open {}", path.display());
    let count = b.FPDF_GetPageCount(doc).max(0) as u32;
    let sizes = (0..count)
        .map(|i| {
            let mut size = FS_SIZEF {
                width: 0.0,
                height: 0.0,
            };
            b.FPDF_GetPageSizeByIndexF(doc, i as i32, &mut size);
            (size.width, size.height)
        })
        .collect();
    let form = read_form(b, doc, count, path.to_str().unwrap(), None);
    b.FPDF_CloseDocument(doc);
    (form, sizes)
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

#[test]
#[ignore = "needs SPEEDYF_FORMS_EVAL_DIR and SPEEDYF_FORMS_EVAL_OUT"]
fn dump() {
    let Some((input, out)) = dirs() else {
        eprintln!("SPEEDYF_FORMS_EVAL_DIR / _OUT not set");
        return;
    };
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    println!("| form | script actions | recognised | not run (field) | document scripts | keystroke | format | validate | calculate | /CO |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for path in forms(&input, &out) {
        let (form, _) = read(&pdfium, &path);
        let count = |pick: &dyn Fn(&super::scripts::FieldBehaviour) -> bool| {
            form.behaviours.values().filter(|b| pick(b)).count()
        };
        let s = &form.scripts;
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            stem(&path),
            s.total,
            s.recognised,
            s.total - s.recognised,
            s.document,
            count(&|b| b.keystroke.is_some()),
            count(&|b| b.format.is_some()),
            count(&|b| b.validate.is_some()),
            count(&|b| b.calculate.is_some()),
            form.calc_order.len(),
        );
        let json = serde_json::to_string_pretty(&form).unwrap();
        std::fs::write(out.join(format!("{}.json", stem(&path))), json).unwrap();
    }
}

#[derive(serde::Deserialize)]
struct Plan {
    texts: Vec<(String, String)>,
    display: Vec<(String, String, Option<bool>)>,
    /// Calculated fields: name, value the helpers computed, value computed
    /// by hand (None where the eval has no hand value).
    calcs: Vec<(String, String, Option<String>)>,
}

#[test]
#[ignore = "needs SPEEDYF_FORMS_EVAL_DIR and SPEEDYF_FORMS_EVAL_OUT"]
fn save() {
    let Some((input, out)) = dirs() else {
        eprintln!("SPEEDYF_FORMS_EVAL_DIR / _OUT not set");
        return;
    };
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    println!("| form | filled | calculated | calc = hand | formatted | AP text correct | /V raw | incremental | lopdf |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let mut all_ok = true;
    for path in forms(&input, &out) {
        let name = stem(&path);
        let Ok(plan_json) = std::fs::read_to_string(out.join(format!("{name}.plan.json"))) else {
            println!("| {name} | no plan (run the node half) | | | | | | | |");
            continue;
        };
        let plan_in: Plan = serde_json::from_str(&plan_json).unwrap();
        let (_, sizes) = read(&pdfium, &path);
        let plan = EditPlan {
            pages: sizes
                .iter()
                .enumerate()
                .map(|(i, (w, h))| PlanPage {
                    src_index: Some(i as u32),
                    width_pt: *w,
                    height_pt: *h,
                    rotation: 0,
                    annots: Vec::new(),
                    texts: Vec::new(),
                    images: Vec::new(),
                    drop_src_annots: Vec::new(),
                })
                .collect(),
            form: plan_in.texts.clone(),
            form_buttons: Vec::new(),
            form_choices: Vec::new(),
            form_display: plan_in.display.clone(),
            form_design: None,
            scrub: Default::default(),
        };
        let output = out.join(format!("formatted-{name}.pdf"));
        let mut problems = Vec::new();
        if let Err(e) =
            crate::engine::save::build_output(&pdfium, path.to_str().unwrap(), None, &plan, &output)
        {
            println!("| {name} | save failed: {e:?} | | | | | | | |");
            all_ok = false;
            continue;
        }
        let before = std::fs::read(&path).unwrap();
        let after = std::fs::read(&output).unwrap();
        let incremental = after.starts_with(&before);
        let doc = lopdf::Document::load(&output);
        let lopdf_ok = doc.is_ok();
        let (mut ap_ok, mut v_ok) = (0, 0);
        if let Ok(doc) = &doc {
            let widgets = widgets_by_name(doc);
            let display: BTreeMap<&str, &str> = plan_in
                .display
                .iter()
                .map(|(n, t, _)| (n.as_str(), t.as_str()))
                .collect();
            for (field, raw) in &plan_in.texts {
                let ids: Vec<_> = widgets.iter().filter(|(n, _)| n == field).collect();
                let Some((_, first)) = ids.first() else {
                    problems.push(format!("{field}: no widget"));
                    continue;
                };
                match field_value(doc, *first) {
                    Some(v) if v == *raw => v_ok += 1,
                    other => problems.push(format!("{field}: /V {other:?} != {raw:?}")),
                }
                if let Some(shown) = display.get(field.as_str()) {
                    let all_drawn = ids.iter().all(|(_, id)| {
                        let w = doc.get_dictionary(*id).unwrap();
                        appearance_text(doc, w).as_deref() == Some(*shown)
                    });
                    if all_drawn {
                        ap_ok += 1;
                    } else {
                        let got = doc
                            .get_dictionary(*first)
                            .ok()
                            .and_then(|w| appearance_text(doc, w));
                        problems.push(format!("{field}: AP {got:?} != {shown:?}"));
                    }
                }
            }
        }
        let hand: Vec<_> = plan_in.calcs.iter().filter(|c| c.2.is_some()).collect();
        let hand_ok = hand.iter().filter(|c| Some(&c.1) == c.2.as_ref()).count();
        for c in &hand {
            if Some(&c.1) != c.2.as_ref() {
                problems.push(format!("{}: calculated {} != hand {:?}", c.0, c.1, c.2));
            }
        }
        println!(
            "| {name} | {} | {} | {}/{} | {} | {ap_ok}/{} | {v_ok}/{} | {} | {} |",
            plan_in.texts.len(),
            plan_in.calcs.len(),
            hand_ok,
            hand.len(),
            plan_in.display.len(),
            plan_in.display.len(),
            plan_in.texts.len(),
            if incremental { "yes" } else { "NO" },
            if lopdf_ok { "ok" } else { "FAILED" },
        );
        for p in problems.iter().take(10) {
            println!("    {p}");
        }
        all_ok &= problems.is_empty() && incremental && lopdf_ok;
    }
    assert!(all_ok, "some formatted values did not survive a save");
}

//! Fill-everything evaluation over real forms.
//!
//! Ignored by default: it needs PDFs that are not in the repository. Point
//! `SPEEDYF_FORMS_EVAL_DIR` at a folder of fillable forms (the IRS W-9, W-4
//! and 1040 and a state form were used) and run
//!
//! ```text
//! SPEEDYF_FORMS_EVAL_DIR=/path/to/forms cargo test --lib forms::eval -- --ignored --nocapture
//! ```
//!
//! Per form it gives every fillable field a valid value, saves through the
//! same `build_output` the app uses, reopens the result and checks that every
//! value came back. Outputs land in `SPEEDYF_FORMS_EVAL_OUT` when set (else a
//! temp dir) and are run through `qpdf --check` when qpdf is installed, and
//! through lopdf's parser either way.

use super::read_form;
use crate::engine::pdfium_init;
use crate::engine::types::{EditPlan, FormDto, FormFieldDto, PlanFormButton, PlanFormChoice};
use pdfium_render::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// What one form was asked to hold.
#[derive(Default)]
struct Expected {
    texts: BTreeMap<String, String>,
    /// (page, index) of the widget that should be on.
    buttons: Vec<(u32, u32, String)>,
    choices: BTreeMap<String, Vec<u32>>,
}

fn is_fillable(f: &FormFieldDto) -> bool {
    matches!(
        f.kind.as_str(),
        "text" | "combo" | "list" | "checkbox" | "radio"
    ) && !f.read_only
        && f.rect.w > 0.0
        && f.rect.h > 0.0
}

fn text_value(f: &FormFieldDto, n: usize) -> String {
    let value = if f.comb && f.max_len > 0 {
        "1234567890".repeat(f.max_len as usize / 10 + 1)
    } else if f.multiline {
        format!("Line {n}\nSecond line")
    } else {
        format!("Value {n}")
    };
    if f.max_len > 0 {
        value.chars().take(f.max_len as usize).collect()
    } else {
        value
    }
}

fn plan_for(form: &FormDto, page_sizes: &[(f32, f32)]) -> (EditPlan, Expected) {
    let mut expected = Expected::default();
    let mut plan = EditPlan {
        pages: page_sizes
            .iter()
            .enumerate()
            .map(|(i, (w, h))| crate::engine::types::PlanPage {
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
        form: Vec::new(),
        form_buttons: Vec::new(),
        form_choices: Vec::new(),
        form_display: Vec::new(),
        form_design: None,
    };
    let mut done: HashSet<&str> = HashSet::new();
    let fillable: Vec<&FormFieldDto> = form.fields.iter().filter(|f| is_fillable(f)).collect();
    for (n, f) in fillable.iter().enumerate() {
        if !done.insert(f.name.as_str()) {
            continue;
        }
        match f.kind.as_str() {
            "text" => {
                let value = text_value(f, n);
                plan.form.push((f.name.clone(), value.clone()));
                expected.texts.insert(f.name.clone(), value);
            }
            "checkbox" | "radio" => {
                // A checkbox turns on its first widget; a radio group its
                // last, so the default-first button is never the answer.
                let group: Vec<&&FormFieldDto> =
                    fillable.iter().filter(|g| g.name == f.name).collect();
                let pick = if f.kind == "radio" {
                    group.last()
                } else {
                    group.first()
                };
                if let Some(pick) = pick {
                    plan.form_buttons.push(PlanFormButton {
                        page: pick.page,
                        index: pick.index,
                        checked: true,
                    });
                    expected
                        .buttons
                        .push((pick.page, pick.index, f.name.clone()));
                }
            }
            "combo" | "list" => {
                let count = f.options.len() as u32;
                if count == 0 {
                    continue;
                }
                let selected = if f.multi_select && count > 1 {
                    vec![0, count - 1]
                } else {
                    vec![count - 1]
                };
                plan.form_choices.push(PlanFormChoice {
                    field: f.name.clone(),
                    selected: selected.clone(),
                    text: None,
                });
                expected.choices.insert(f.name.clone(), selected);
            }
            _ => {}
        }
    }
    (plan, expected)
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

/// Values that did not come back, by field name.
fn survivors(after: &FormDto, expected: &Expected) -> (usize, Vec<String>) {
    let normalize = |s: &str| s.replace("\r\n", "\n").replace('\r', "\n");
    let mut ok = 0;
    let mut lost = Vec::new();
    for (name, value) in &expected.texts {
        match after.fields.iter().find(|f| &f.name == name) {
            Some(f) if normalize(&f.value) == *value => ok += 1,
            Some(f) => lost.push(format!("{name}: {:?} != {value:?}", f.value)),
            None => lost.push(format!("{name}: missing")),
        }
    }
    for (page, index, name) in &expected.buttons {
        match after
            .fields
            .iter()
            .find(|f| f.page == *page && f.index == *index)
        {
            Some(f) if f.checked => ok += 1,
            _ => lost.push(format!("{name}: not on")),
        }
    }
    for (name, selected) in &expected.choices {
        match after.fields.iter().find(|f| &f.name == name) {
            Some(f) if f.selected == *selected => ok += 1,
            Some(f) => lost.push(format!("{name}: {:?} != {selected:?}", f.selected)),
            None => lost.push(format!("{name}: missing")),
        }
    }
    (ok, lost)
}

fn qpdf_check(path: &Path) -> String {
    match std::process::Command::new("qpdf")
        .arg("--check")
        .arg(path)
        .output()
    {
        Ok(out) if out.status.success() => "ok".into(),
        Ok(out) => format!(
            "FAILED: {}",
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .chain(String::from_utf8_lossy(&out.stderr).lines())
                .filter(|l| l.contains("WARNING") || l.contains("ERROR") || l.contains("error"))
                .take(3)
                .collect::<Vec<_>>()
                .join("; ")
        ),
        Err(_) => "n/a".into(),
    }
}

#[test]
#[ignore = "needs SPEEDYF_FORMS_EVAL_DIR with real forms"]
fn fill_every_field_of_real_forms() {
    let Ok(dir) = std::env::var("SPEEDYF_FORMS_EVAL_DIR") else {
        eprintln!("SPEEDYF_FORMS_EVAL_DIR is not set; nothing to evaluate");
        return;
    };
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let temp = tempfile::tempdir().unwrap();
    let out_dir = std::env::var("SPEEDYF_FORMS_EVAL_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| temp.path().to_path_buf());
    std::fs::create_dir_all(&out_dir).unwrap();

    let mut forms: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    forms.sort();

    println!(
        "| form | widgets | fields | choice fields | filled | survived save | qpdf | lopdf | scripts | xfa before/after |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut all_survived = true;
    for path in forms {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (before, sizes) = read(&pdfium, &path);
        let names: HashSet<&str> = before.fields.iter().map(|f| f.name.as_str()).collect();
        let (plan, expected) = plan_for(&before, &sizes);
        let filled = expected.texts.len() + expected.buttons.len() + expected.choices.len();
        let output = out_dir.join(format!("filled-{name}"));
        let saved = crate::engine::save::build_output(
            &pdfium,
            path.to_str().unwrap(),
            None,
            &plan,
            &output,
        );
        let (survived, mut lost, qpdf, lopdf_ok, xfa_after) = match saved {
            Ok(()) => {
                let (after, _) = read(&pdfium, &output);
                let (ok, lost) = survivors(&after, &expected);
                let lopdf_ok = lopdf::Document::load(&output).is_ok();
                (ok, lost, qpdf_check(&output), lopdf_ok, after.xfa)
            }
            Err(error) => (
                0,
                vec![format!("save failed: {error:?}")],
                "-".into(),
                false,
                before.xfa,
            ),
        };
        if xfa_after {
            lost.push("the stale XFA copy is still there".into());
        }
        let choice_fields: HashSet<&str> = before
            .fields
            .iter()
            .filter(|f| f.kind == "combo" || f.kind == "list")
            .map(|f| f.name.as_str())
            .collect();
        let scripts: u32 = before.fields.iter().map(|f| f.script_count).sum();
        println!(
            "| {name} | {} | {} | {} | {filled} | {survived}/{filled} | {qpdf} | {} | {scripts} | {}/{} |",
            before.fields.len(),
            names.len(),
            choice_fields.len(),
            if lopdf_ok { "ok" } else { "FAILED" },
            before.xfa,
            xfa_after,
        );
        for line in lost.iter().take(10) {
            println!("    lost: {line}");
        }
        all_survived &= lost.is_empty() && lopdf_ok && !qpdf.starts_with("FAILED");
    }
    assert!(all_survived, "some values did not survive a save");
}

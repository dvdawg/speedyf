//! Field scripts end to end: recognised on read, formatted on save.
//!
//! The fixture is an invoice-like form whose scripts are all the standard
//! Acrobat helpers but two: a custom calculation and a blur alert, which
//! must be counted and left alone. The expected display strings and
//! calculation results below are the ones formScripts.ts produces for the
//! same behaviours (see `invoice` in formScripts.test.ts).

use super::read_form;
use super::scripts::{FieldCalc, FieldFormat, FieldRange};
use super::tests::write_objects;
use crate::engine::pdfium_init;
use crate::engine::types::{EditPlan, PlanPage};
use pdfium_render::prelude::*;
use std::path::Path;

/// Escape a PDF literal string body.
fn lit(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

pub(super) struct Spec {
    name: &'static str,
    read_only: bool,
    actions: &'static [(&'static str, &'static str)],
}

const MONEY_K: &str = "AFNumber_Keystroke(2, 0, 0, 0, \"$\", true);";
const MONEY_F: &str = "AFNumber_Format(2, 0, 0, 0, \"$\", true);";

pub(super) const INVOICE: &[Spec] = &[
    Spec {
        name: "l1",
        read_only: false,
        actions: &[("K", MONEY_K), ("F", MONEY_F)],
    },
    Spec {
        name: "l2",
        read_only: false,
        actions: &[("K", MONEY_K), ("F", MONEY_F)],
    },
    Spec {
        name: "l3",
        read_only: false,
        actions: &[("K", MONEY_K), ("F", MONEY_F)],
    },
    Spec {
        name: "rate",
        read_only: false,
        actions: &[
            ("K", "AFPercent_Keystroke(1, 0);"),
            ("F", "AFPercent_Format(1, 0);"),
            ("V", "AFRange_Validate(true, 0, true, 1);"),
        ],
    },
    Spec {
        name: "subtotal",
        read_only: true,
        actions: &[
            ("F", MONEY_F),
            (
                "C",
                "AFSimple_Calculate(\"SUM\", new Array (\"l1\", \"l2\", \"l3\"));",
            ),
        ],
    },
    Spec {
        name: "tax",
        read_only: true,
        actions: &[
            ("F", "AFNumber_Format(2, 0, 2, 0, \"\", true);"),
            ("C", "AFSimple_Calculate(\"PRD\", \"subtotal, rate\");"),
        ],
    },
    Spec {
        name: "total",
        read_only: true,
        actions: &[
            ("F", MONEY_F),
            ("C", "AFSimple_Calculate('SUM', ['subtotal', 'tax'])"),
        ],
    },
    Spec {
        name: "avg",
        read_only: true,
        actions: &[
            ("F", "AFNumber_Format(1, 2, 0, 0, \" EUR\", false);"),
            ("C", "AFSimple_Calculate(\"AVG\", \"l1, l2, l3\");"),
        ],
    },
    Spec {
        name: "maxv",
        read_only: true,
        actions: &[
            ("F", "AFNumber_Format(0, 1, 0, 0, \"\", true);"),
            ("C", "AFSimple_Calculate(\"MAX\", \"l1, l2, l3\");"),
        ],
    },
    Spec {
        name: "net",
        read_only: false,
        actions: &[
            ("K", "AFNumber_Keystroke(2, 0, 1, 0, \"\", true);"),
            ("F", "AFNumber_Format(2, 0, 1, 0, \"\", true);"),
        ],
    },
    Spec {
        name: "when",
        read_only: false,
        actions: &[
            ("K", "AFDate_KeystrokeEx(\"mmm d, yyyy\");"),
            ("F", "AFDate_FormatEx(\"mmm d, yyyy\");"),
        ],
    },
    Spec {
        name: "ssn",
        read_only: false,
        actions: &[
            ("K", "AFSpecial_Keystroke(3);"),
            ("F", "AFSpecial_Format(3);"),
        ],
    },
    Spec {
        name: "phone",
        read_only: false,
        actions: &[
            ("K", "AFSpecial_Keystroke(2);"),
            ("F", "AFSpecial_Format(2);"),
        ],
    },
    Spec {
        name: "custom",
        read_only: true,
        actions: &[("F", MONEY_F), ("C", "MyCalc();")],
    },
    Spec {
        name: "note",
        read_only: false,
        actions: &[("Bl", "app.alert('Thanks');")],
    },
];

/// The order the fixture's /CO lists.
pub(super) const INVOICE_CO: &[&str] = &["subtotal", "tax", "total", "avg", "maxv", "custom"];

/// A one-page form with the fields of `INVOICE`, one per row.
pub(super) fn write_invoice_fixture(path: &Path) {
    const FIRST: u32 = 10;
    let ids: Vec<u32> = (0..INVOICE.len() as u32).map(|i| FIRST + i).collect();
    let refs = |names: &[&str]| -> String {
        names
            .iter()
            .map(|n| {
                let at = INVOICE.iter().position(|s| s.name == *n).unwrap();
                format!("{} 0 R", ids[at])
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let all: Vec<String> = ids.iter().map(|id| format!("{id} 0 R")).collect();
    let all = all.join(" ");
    let mut objects: Vec<(u32, Vec<u8>)> = vec![
        (
            1,
            b"<< /Type /Catalog /Pages 2 0 R /AcroForm 3 0 R /Names << /JavaScript 4 0 R >> >>"
                .to_vec(),
        ),
        (2, b"<< /Type /Pages /Kids [7 0 R] /Count 1 >>".to_vec()),
        (
            3,
            format!(
                "<< /Fields [{all}] /CO [{}] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >>",
                refs(INVOICE_CO)
            )
            .into_bytes(),
        ),
        (4, b"<< /Names [(init) 6 0 R] >>".to_vec()),
        (
            5,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_vec(),
        ),
        (
            6,
            b"<< /S /JavaScript /JS (function MyCalc\\(\\) { event.value = 1; }) >>".to_vec(),
        ),
        (
            7,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 700] /Annots [{all}] >>")
                .into_bytes(),
        ),
    ];
    for (i, spec) in INVOICE.iter().enumerate() {
        let top = 660.0 - 40.0 * i as f32;
        let aa: String = spec
            .actions
            .iter()
            .map(|(event, js)| format!("/{event} << /S /JavaScript /JS ({}) >>", lit(js)))
            .collect();
        objects.push((
            ids[i],
            format!(
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T ({}) /F 4 /P 7 0 R /Rect [100 {} 300 {}] /DA (/Helv 10 Tf 0 g) /Q 2 /Ff {} /AA << {aa} >> >>",
                spec.name,
                top - 20.0,
                top,
                u8::from(spec.read_only)
            )
            .into_bytes(),
        ));
    }
    write_objects(path, objects);
}

/// What the user types, and what the calculations put in the read-only
/// fields, as raw values.
pub(super) const INVOICE_VALUES: &[(&str, &str)] = &[
    ("l1", "100"),
    ("l2", "250.7"),
    ("l3", "1000"),
    ("rate", "0.08"),
    ("net", "-42.5"),
    ("when", "3/5/2026"),
    ("ssn", "123456789"),
    ("phone", "5551234567"),
    ("note", "Paid"),
    // Hand-computed: 100 + 250.7 + 1000; 1350.7 x 0.08; 1350.7 + 108.056;
    // 1350.7 / 3; max(100, 250.7, 1000).
    ("subtotal", "1350.7"),
    ("tax", "108.056"),
    ("total", "1458.756"),
    ("avg", "450.233333333333"),
    ("maxv", "1000"),
];

/// What each formatted field shows: text, and colour for a red style.
pub(super) const INVOICE_DISPLAY: &[(&str, &str, Option<bool>)] = &[
    ("l1", "$100.00", None),
    ("l2", "$250.70", None),
    ("l3", "$1,000.00", None),
    ("rate", "8.0%", None),
    ("net", "42.50", Some(true)),
    ("when", "Mar 5, 2026", None),
    ("ssn", "123-45-6789", None),
    ("phone", "(555) 123-4567", None),
    ("subtotal", "$1,350.70", None),
    ("tax", "108.06", None),
    ("total", "$1,458.76", None),
    ("avg", "450,2 EUR", None),
];

fn read(pdfium: &Pdfium, path: &Path) -> crate::engine::types::FormDto {
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(path.to_str().unwrap(), None);
    assert!(!doc.is_null());
    let pages = b.FPDF_GetPageCount(doc) as u32;
    let form = read_form(b, doc, pages, path.to_str().unwrap(), None);
    b.FPDF_CloseDocument(doc);
    form
}

/// The text a widget's normal appearance stream draws: the strings shown
/// by its Tj and TJ operators, concatenated per line.
pub(super) fn appearance_text(doc: &lopdf::Document, widget: &lopdf::Dictionary) -> Option<String> {
    let ap = doc
        .dereference(widget.get(b"AP").ok()?)
        .ok()?
        .1
        .as_dict()
        .ok()?;
    let normal = ap.get(b"N").ok()?.as_reference().ok()?;
    let stream = doc.get_object(normal).ok()?.as_stream().ok()?;
    let content = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    let ops = lopdf::content::Content::decode(&content).ok()?;
    let mut text = String::new();
    for op in ops.operations {
        match op.operator.as_str() {
            "Tj" | "'" | "\"" => {
                if let Some(lopdf::Object::String(bytes, _)) = op.operands.last() {
                    text.push_str(&decode_shown(bytes));
                }
            }
            "TJ" => {
                if let Some(lopdf::Object::Array(items)) = op.operands.first() {
                    for item in items {
                        if let lopdf::Object::String(bytes, _) = item {
                            text.push_str(&decode_shown(bytes));
                        }
                    }
                }
            }
            "Td" | "TD" | "T*" if !text.is_empty() && !text.ends_with('\n') => text.push('\n'),
            _ => {}
        }
    }
    Some(text.trim_end_matches('\n').to_string())
}

/// Shown bytes in a simple font as text (WinAnsi and ASCII coincide for
/// everything the fixtures draw).
fn decode_shown(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

/// Widgets by field name in the newest revision of `path`.
pub(super) fn widgets_by_name(doc: &lopdf::Document) -> Vec<(String, lopdf::ObjectId)> {
    let mut out = Vec::new();
    for (_, page) in doc.get_pages() {
        let Ok(page) = doc.get_dictionary(page) else {
            continue;
        };
        let Some(annots) = page
            .get(b"Annots")
            .ok()
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
        else {
            continue;
        };
        for entry in annots {
            let Ok(id) = entry.as_reference() else {
                continue;
            };
            let Ok(widget) = doc.get_dictionary(id) else {
                continue;
            };
            let mut parts = Vec::new();
            let mut current = Some(widget);
            let mut depth = 0;
            while let Some(d) = current {
                if let Ok(t) = d.get(b"T") {
                    parts.push(lopdf::decode_text_string(t).unwrap_or_default());
                }
                current = d
                    .get(b"Parent")
                    .ok()
                    .and_then(|p| p.as_reference().ok())
                    .and_then(|p| doc.get_dictionary(p).ok());
                depth += 1;
                if depth > 32 {
                    break;
                }
            }
            parts.reverse();
            out.push((parts.join("."), id));
        }
    }
    out
}

/// The /V a field holds, following /Parent for a widget that has none.
pub(super) fn field_value(doc: &lopdf::Document, widget: lopdf::ObjectId) -> Option<String> {
    let mut current = doc.get_dictionary(widget).ok();
    let mut depth = 0;
    while let Some(d) = current {
        if let Ok(v) = d.get(b"V") {
            return lopdf::decode_text_string(v).ok();
        }
        current = d
            .get(b"Parent")
            .ok()
            .and_then(|p| p.as_reference().ok())
            .and_then(|p| doc.get_dictionary(p).ok());
        depth += 1;
        if depth > 32 {
            break;
        }
    }
    None
}

fn one_page_plan() -> EditPlan {
    EditPlan {
        pages: vec![PlanPage {
            src_index: Some(0),
            width_pt: 400.0,
            height_pt: 700.0,
            rotation: 0,
            annots: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }],
        form: Vec::new(),
        form_buttons: Vec::new(),
        form_choices: Vec::new(),
        form_display: Vec::new(),
        scrub: Default::default(),
    }
}

#[test]
fn reads_standard_helpers_counts_the_rest_and_keeps_calc_order() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invoice.pdf");
    write_invoice_fixture(&path);
    let form = read(&pdfium, &path);

    assert_eq!(form.calc_order, INVOICE_CO);
    assert_eq!(form.scripts.total, 30);
    assert_eq!(form.scripts.recognised, 28);
    assert_eq!(form.scripts.document, 1);

    let b = &form.behaviours;
    assert_eq!(
        b["subtotal"].calculate,
        Some(FieldCalc {
            op: "SUM".into(),
            fields: vec!["l1".into(), "l2".into(), "l3".into()]
        })
    );
    assert_eq!(
        b["rate"].validate,
        Some(FieldRange {
            min: Some(0.0),
            max: Some(1.0)
        })
    );
    assert_eq!(
        b["when"].format,
        Some(FieldFormat::Date {
            format: "mmm d, yyyy".into()
        })
    );
    // The custom calculation is not taken; its field keeps its format.
    assert!(b["custom"].calculate.is_none());
    assert!(b["custom"].format.is_some());
    assert!(!b.contains_key("note"));

    // The shape the frontend reads.
    let json = serde_json::to_value(&form).unwrap();
    assert_eq!(
        json["behaviours"]["l1"]["format"],
        serde_json::json!({
            "kind": "number", "decimals": 2, "sepStyle": 0, "negStyle": 0,
            "currency": "$", "prepend": true
        })
    );
    assert_eq!(
        json["behaviours"]["rate"]["validate"],
        serde_json::json!({ "min": 0.0, "max": 1.0 })
    );
    assert_eq!(
        json["scripts"],
        serde_json::json!({ "total": 30, "recognised": 28, "document": 1 })
    );
}

#[test]
fn saves_formatted_appearances_over_raw_values() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("invoice.pdf");
    let output = dir.path().join("out.pdf");
    write_invoice_fixture(&source);

    let mut plan = one_page_plan();
    plan.form = INVOICE_VALUES
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    plan.form_display = INVOICE_DISPLAY
        .iter()
        .map(|(n, t, red)| (n.to_string(), t.to_string(), *red))
        .collect();
    crate::engine::save::build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output)
        .expect("save");

    // Incremental: the source bytes are the start of the output.
    let before = std::fs::read(&source).unwrap();
    let after = std::fs::read(&output).unwrap();
    assert!(after.starts_with(&before), "the save rewrote the file");

    let doc = lopdf::Document::load(&output).expect("lopdf reads the output");
    let widgets = widgets_by_name(&doc);
    for (name, raw) in INVOICE_VALUES {
        let (_, id) = widgets.iter().find(|(n, _)| n == name).unwrap();
        assert_eq!(field_value(&doc, *id).as_deref(), Some(*raw), "{name} /V");
        let shown = INVOICE_DISPLAY
            .iter()
            .find(|(n, _, _)| n == name)
            .map_or(*raw, |(_, t, _)| *t);
        let widget = doc.get_dictionary(*id).unwrap();
        assert_eq!(
            appearance_text(&doc, widget).as_deref(),
            Some(shown),
            "{name} appearance"
        );
    }
    // The red negative style turned the field's ink red, as Acrobat does.
    let (_, net) = widgets.iter().find(|(n, _)| n == "net").unwrap();
    let da = doc.get_dictionary(*net).unwrap().get(b"DA").unwrap();
    assert_eq!(da.as_str().unwrap(), b"/Helv 10 Tf 1 0 0 rg");
    // Read-only calculated fields stay read-only.
    let (_, total) = widgets.iter().find(|(n, _)| n == "total").unwrap();
    let ff = doc.get_dictionary(*total).unwrap().get(b"Ff").unwrap();
    assert_eq!(ff.as_i64().unwrap() & 1, 1);

    // Reopened, the fields report the raw values.
    let form = read(&pdfium, &output);
    for (name, raw) in INVOICE_VALUES {
        let f = form.fields.iter().find(|f| f.name == *name).unwrap();
        assert_eq!(f.value, *raw, "{name} reopened");
    }
}

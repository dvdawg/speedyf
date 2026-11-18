//! Prepare form mode round trips: fields written by `design` and
//! `flatten`, read back through the same path the overlay uses.

use super::scripts::FieldFormat;
use super::tests::{
    field, one_page_plan, read_fields, write_form_fixture, write_kinds_fixture, write_objects,
};
use crate::engine::pdfium_init;
use crate::engine::types::{
    EditPlan, FormOptionDto, PlanDesignField, PlanFieldCalc, PlanFieldFormat, PlanFormButton,
    PlanFormDesign, PlanTabEntry, PlanTabOrder, PlanWidgetRef, RectDto,
};
use lopdf::dictionary;
use pdfium_render::prelude::*;
use std::path::Path;

fn rect(x: f32, y: f32, w: f32, h: f32) -> RectDto {
    RectDto { x, y, w, h }
}

fn new_field(key: &str, kind: &str, name: &str, r: RectDto) -> PlanDesignField {
    PlanDesignField {
        key: key.into(),
        origin: None,
        page: 0,
        rect: r,
        kind: kind.into(),
        name: name.into(),
        tooltip: String::new(),
        required: false,
        read_only: false,
        default_value: String::new(),
        font_size: 0.0,
        align: 0,
        max_len: 0,
        comb: false,
        multiline: false,
        options: Vec::new(),
        export_value: String::new(),
        border: false,
        format: None,
        calc: None,
        format_set: true,
        calc_set: true,
        restyle: true,
    }
}

/// An empty one-page document: what a printed form is to the form layer.
fn write_blank(path: &Path, rotate: u16) {
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Rotate {rotate} /Resources << >> /Contents 4 0 R >>"
    );
    let content = b"0 0 1 RG 10 10 m 100 10 l S";
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    write_objects(
        path,
        vec![
            (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
            (3, page.into_bytes()),
            (4, stream),
        ],
    );
}

fn plan_with(design: PlanFormDesign) -> EditPlan {
    let mut plan = one_page_plan();
    plan.form_design = Some(design);
    plan
}

fn save(pdfium: &Pdfium, source: &Path, output: &Path, plan: &EditPlan) {
    crate::engine::save::build_output(pdfium, source.to_str().unwrap(), None, plan, output)
        .expect("build output");
    // For checking the files with other readers by hand.
    if let Ok(dir) = std::env::var("SPEEDYF_PREPARE_DUMP") {
        let name = format!(
            "{}-{}",
            std::thread::current()
                .name()
                .unwrap_or("t")
                .replace("::", "_"),
            output.file_name().unwrap().to_string_lossy()
        );
        let _ = std::fs::copy(output, Path::new(&dir).join(name));
    }
}

fn close(a: &RectDto, b: &RectDto) -> bool {
    (a.x - b.x).abs() < 0.6
        && (a.y - b.y).abs() < 0.6
        && (a.w - b.w).abs() < 0.6
        && (a.h - b.h).abs() < 0.6
}

fn every_kind() -> PlanFormDesign {
    let mut name = new_field("n", "text", "Name", rect(20.0, 350.0, 180.0, 18.0));
    name.tooltip = "Your full name".into();
    name.required = true;
    name.font_size = 9.0;
    name.align = 1;
    name.max_len = 30;
    name.default_value = "Ada".into();

    let mut zip = new_field("z", "text", "Zip", rect(20.0, 320.0, 100.0, 16.0));
    zip.comb = true;
    zip.max_len = 5;
    zip.format = Some(PlanFieldFormat::Special { style: 0 });

    let mut notes = new_field("m", "text", "Notes", rect(150.0, 250.0, 130.0, 60.0));
    notes.multiline = true;

    let mut agree = new_field("c", "checkbox", "Agree", rect(20.0, 290.0, 12.0, 12.0));
    agree.export_value = "Yes".into();
    agree.default_value = "Yes".into();
    agree.border = true;

    let mut a = new_field("ra", "radio", "Pick", rect(20.0, 260.0, 12.0, 12.0));
    a.export_value = "A".into();
    a.default_value = "B".into();
    let mut b = new_field("rb", "radio", "Pick", rect(40.0, 260.0, 12.0, 12.0));
    b.export_value = "B".into();
    b.default_value = "B".into();

    let options = vec![
        FormOptionDto {
            label: "California".into(),
            export: "CA".into(),
        },
        FormOptionDto {
            label: "Texas".into(),
            export: "TX".into(),
        },
    ];
    let mut state = new_field("s", "combo", "State", rect(20.0, 220.0, 100.0, 18.0));
    state.options = options.clone();
    state.default_value = "TX".into();
    let mut list = new_field("l", "list", "Regions", rect(20.0, 150.0, 100.0, 60.0));
    list.options = options;

    let sig = new_field(
        "g",
        "signature",
        "Signature",
        rect(150.0, 150.0, 130.0, 30.0),
    );

    let mut date = new_field("d", "text", "Date", rect(150.0, 120.0, 80.0, 16.0));
    date.format = Some(PlanFieldFormat::Date {
        format: "mm/dd/yyyy".into(),
    });
    let mut amount = new_field("a1", "text", "Amount", rect(20.0, 100.0, 80.0, 16.0));
    amount.format = Some(PlanFieldFormat::Number {
        decimals: 2,
        sep_style: 0,
        neg_style: 0,
        currency: "$".into(),
        prepend: true,
    });
    let mut total = new_field("t", "text", "Total", rect(20.0, 70.0, 80.0, 16.0));
    total.read_only = true;
    total.calc = Some(PlanFieldCalc {
        op: "SUM".into(),
        fields: vec!["Amount".into()],
    });

    PlanFormDesign {
        fields: vec![
            name, zip, notes, agree, a, b, state, list, sig, date, amount, total,
        ],
        ..PlanFormDesign::default()
    }
}

#[test]
fn every_field_kind_is_created_and_reads_back() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("blank.pdf");
    let output = dir.path().join("out.pdf");
    write_blank(&source, 0);
    let design = every_kind();
    save(&pdfium, &source, &output, &plan_with(design.clone()));

    let before = std::fs::read(&source).unwrap();
    let after = std::fs::read(&output).unwrap();
    assert!(
        after.starts_with(&before),
        "written as an incremental update"
    );

    let form = read_fields(&pdfium, &output);
    assert_eq!(
        form.fields.len(),
        12,
        "{:?}",
        form.fields.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    for f in &design.fields {
        let got = form
            .fields
            .iter()
            .find(|g| g.name == f.name && close(&g.rect, &f.rect))
            .unwrap_or_else(|| panic!("no widget for {} at {:?}", f.name, f.rect));
        assert_eq!(got.kind, f.kind, "{}", f.name);
    }

    let name = field(&form, "Name");
    assert_eq!(name.tooltip, "Your full name");
    assert!(name.required);
    assert_eq!(name.font_size, 9.0);
    assert_eq!(name.align, 1);
    assert_eq!(name.max_len, 30);
    assert_eq!(name.value, "Ada");
    assert_eq!(name.default_value, "Ada");

    let zip = field(&form, "Zip");
    assert!(zip.comb);
    assert_eq!(zip.max_len, 5);
    assert!(field(&form, "Notes").multiline);

    let agree = field(&form, "Agree");
    assert!(agree.checked);
    assert_eq!(agree.export_value, "Yes");
    assert!(agree.default_checked);

    let picks: Vec<_> = form.fields.iter().filter(|f| f.name == "Pick").collect();
    assert_eq!(picks.len(), 2);
    let on: Vec<_> = picks
        .iter()
        .filter(|f| f.checked)
        .map(|f| f.export_value.as_str())
        .collect();
    assert_eq!(on, ["B"]);

    let state = field(&form, "State");
    let exports: Vec<_> = state.options.iter().map(|o| o.export.as_str()).collect();
    assert_eq!(exports, ["CA", "TX"]);
    assert_eq!(state.selected, [1]);
    assert_eq!(state.default_selected, [1]);
    assert_eq!(field(&form, "Regions").options.len(), 2);
    assert!(field(&form, "Total").read_only);

    // The helpers come back as Phase 2 behaviours.
    let b = |n: &str| form.behaviours.get(n).cloned().unwrap_or_default();
    assert_eq!(b("Zip").format, Some(FieldFormat::Special { style: 0 }));
    assert_eq!(
        b("Date").format,
        Some(FieldFormat::Date {
            format: "mm/dd/yyyy".into()
        })
    );
    assert!(matches!(
        b("Amount").format,
        Some(FieldFormat::Number { decimals: 2, .. })
    ));
    assert_eq!(
        b("Total").calculate.unwrap().fields,
        vec!["Amount".to_string()]
    );
    assert_eq!(form.calc_order, vec!["Total".to_string()]);
    assert_eq!(
        form.scripts.total, form.scripts.recognised,
        "every script written is a helper"
    );

    // Each widget has an appearance PDFium can draw: a checked box shows.
    let doc = pdfium.load_pdf_from_file(&output, None).unwrap();
    let page = doc.pages().get(0).unwrap();
    assert!(page.annotations().len() >= 12);
}

#[test]
fn created_fields_fill_and_save_like_any_other() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("blank.pdf");
    let made = dir.path().join("made.pdf");
    let filled = dir.path().join("filled.pdf");
    write_blank(&source, 0);
    save(&pdfium, &source, &made, &plan_with(every_kind()));

    let form = read_fields(&pdfium, &made);
    let agree = field(&form, "Agree");
    let pick_a = form
        .fields
        .iter()
        .find(|f| f.name == "Pick" && f.export_value == "A")
        .unwrap();
    let mut plan = one_page_plan();
    plan.form = vec![
        ("Name".into(), "Grace".into()),
        ("Amount".into(), "12.5".into()),
    ];
    plan.form_buttons = vec![
        PlanFormButton {
            page: 0,
            index: agree.index,
            checked: false,
        },
        PlanFormButton {
            page: 0,
            index: pick_a.index,
            checked: true,
        },
    ];
    plan.form_choices = vec![crate::engine::types::PlanFormChoice {
        field: "State".into(),
        selected: vec![0],
        text: None,
    }];
    save(&pdfium, &made, &filled, &plan);
    let form = read_fields(&pdfium, &filled);
    assert_eq!(field(&form, "Name").value, "Grace");
    assert_eq!(field(&form, "Amount").value, "12.5");
    assert!(!field(&form, "Agree").checked);
    let on: Vec<_> = form
        .fields
        .iter()
        .filter(|f| f.name == "Pick" && f.checked)
        .map(|f| f.export_value.as_str())
        .collect();
    assert_eq!(on, ["A"]);
    assert_eq!(field(&form, "State").selected, [0]);
}

fn origin_of(form: &crate::engine::types::FormDto, name: &str) -> PlanWidgetRef {
    let f = field(form, name);
    PlanWidgetRef {
        page: 0,
        name: name.into(),
        rect: f.rect,
    }
}

#[test]
fn existing_fields_take_new_properties() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("kinds.pdf");
    let output = dir.path().join("out.pdf");
    write_kinds_fixture(&source);
    let form = read_fields(&pdfium, &source);

    let mut name = new_field("n", "text", "Full name", rect(30.0, 340.0, 170.0, 22.0));
    name.origin = Some(origin_of(&form, "name"));
    name.tooltip = "Legal name".into();
    name.required = false;
    name.max_len = 0;
    name.font_size = 11.0;
    name.align = 2;
    name.default_value = "Default".into();
    // The name field carries an unrecognised format script: left alone.
    name.format_set = false;

    let mut size = new_field("s", "combo", "size", field(&form, "size").rect);
    size.origin = Some(origin_of(&form, "size"));
    size.options = vec![
        FormOptionDto {
            label: "Large".into(),
            export: "L".into(),
        },
        FormOptionDto {
            label: "Small".into(),
            export: "S".into(),
        },
    ];
    size.restyle = false;

    let design = PlanFormDesign {
        fields: vec![name, size],
        ..PlanFormDesign::default()
    };
    save(&pdfium, &source, &output, &plan_with(design));
    let after = read_fields(&pdfium, &output);
    assert!(after.fields.iter().all(|f| f.name != "name"));
    let renamed = field(&after, "Full name");
    assert_eq!(renamed.tooltip, "Legal name");
    assert!(!renamed.required);
    assert_eq!(renamed.max_len, 0);
    assert_eq!(renamed.font_size, 11.0);
    assert_eq!(renamed.align, 2);
    assert_eq!(
        renamed.value, "Before",
        "the value is the user's, not the form's"
    );
    assert_eq!(renamed.color, "#0000ff", "the field's own colour is kept");
    assert!(close(&renamed.rect, &rect(30.0, 340.0, 170.0, 22.0)));
    assert_eq!(renamed.script_count, 2, "unrecognised scripts survive");
    let size = field(&after, "size");
    let labels: Vec<_> = size.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["Large", "Small"]);
    assert_eq!(after.fields.len(), form.fields.len());
}

#[test]
fn deleted_widgets_leave_the_page_and_the_tree() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("form.pdf");
    let output = dir.path().join("out.pdf");
    write_form_fixture(&source);
    let form = read_fields(&pdfium, &source);
    let first_radio = form.fields.iter().find(|f| f.name == "choice").unwrap();
    let design = PlanFormDesign {
        deleted: vec![
            origin_of(&form, "name"),
            PlanWidgetRef {
                page: 0,
                name: "choice".into(),
                rect: first_radio.rect,
            },
        ],
        ..PlanFormDesign::default()
    };
    save(&pdfium, &source, &output, &plan_with(design));
    let after = read_fields(&pdfium, &output);
    let names: Vec<_> = after.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["agree", "choice"]);

    // Deleting the last radio removes its group from /Fields as well.
    let again = dir.path().join("again.pdf");
    let left = after.fields.iter().find(|f| f.name == "choice").unwrap();
    let design = PlanFormDesign {
        deleted: vec![PlanWidgetRef {
            page: 0,
            name: "choice".into(),
            rect: left.rect,
        }],
        ..PlanFormDesign::default()
    };
    save(&pdfium, &output, &again, &plan_with(design));
    let bytes = std::fs::read(&again).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let catalog = doc.catalog().unwrap();
    let acro = doc
        .dereference(catalog.get(b"AcroForm").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let fields = acro.get(b"Fields").unwrap().as_array().unwrap();
    assert_eq!(fields.len(), 1, "only the checkbox is left: {fields:?}");
}

#[test]
fn tab_order_is_written_as_widget_order() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("blank.pdf");
    let output = dir.path().join("out.pdf");
    write_blank(&source, 0);
    let a = new_field("a", "text", "A", rect(20.0, 300.0, 60.0, 16.0));
    let b = new_field("b", "text", "B", rect(100.0, 300.0, 60.0, 16.0));
    let c = new_field("c", "text", "C", rect(20.0, 200.0, 60.0, 16.0));
    let key = |k: &str| PlanTabEntry {
        key: Some(k.into()),
        widget: None,
    };
    let design = PlanFormDesign {
        fields: vec![a, b, c],
        tab_orders: vec![PlanTabOrder {
            page: 0,
            mode: "S".into(),
            order: vec![key("c"), key("a"), key("b")],
        }],
        ..PlanFormDesign::default()
    };
    save(&pdfium, &source, &output, &plan_with(design));
    let form = read_fields(&pdfium, &output);
    let mut order: Vec<_> = form.fields.iter().collect();
    order.sort_by_key(|f| f.tab_order);
    let names: Vec<_> = order.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["C", "A", "B"]);

    // Reordering existing widgets by reference, and switching to rows.
    let again = dir.path().join("again.pdf");
    let r = |n: &str| PlanTabEntry {
        key: None,
        widget: Some(origin_of(&form, n)),
    };
    let design = PlanFormDesign {
        tab_orders: vec![PlanTabOrder {
            page: 0,
            mode: "R".into(),
            order: vec![r("A"), r("B"), r("C")],
        }],
        ..PlanFormDesign::default()
    };
    save(&pdfium, &output, &again, &plan_with(design));
    let form = read_fields(&pdfium, &again);
    let mut order: Vec<_> = form.fields.iter().collect();
    order.sort_by_key(|f| f.index);
    let names: Vec<_> = order.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["A", "B", "C"], "widgets in the order given");
}

#[test]
fn fields_on_a_turned_page_land_where_they_were_drawn() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("turned.pdf");
    let output = dir.path().join("out.pdf");
    write_blank(&source, 90);
    // Display space on a page turned 90: 400 wide, 300 tall.
    let mut f = new_field("t", "text", "Turned", rect(40.0, 200.0, 150.0, 20.0));
    f.default_value = "Upright".into();
    let mut plan = plan_with(PlanFormDesign {
        fields: vec![f.clone()],
        ..PlanFormDesign::default()
    });
    plan.pages[0].width_pt = 400.0;
    plan.pages[0].height_pt = 300.0;
    save(&pdfium, &source, &output, &plan);
    let form = read_fields(&pdfium, &output);
    let got = field(&form, "Turned");
    assert!(close(&got.rect, &f.rect), "{:?} vs {:?}", got.rect, f.rect);
}

/// Text on page 0 of `path`, through PDFium's text extraction.
fn page_text(pdfium: &Pdfium, path: &Path) -> String {
    let doc = pdfium.load_pdf_from_file(path, None).unwrap();
    let page = doc.pages().get(0).unwrap();
    let text = page.text().unwrap().all();
    text
}

#[test]
fn flatten_merges_widgets_and_keeps_other_annotations() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("form.pdf");
    let output = dir.path().join("out.pdf");
    // The text-and-buttons fixture, plus a highlight that must survive.
    write_form_fixture(&source);
    let mut bytes = std::fs::read(&source).unwrap();
    let mut inc: lopdf::IncrementalDocument = bytes.as_slice().try_into().unwrap();
    let page = inc.get_prev_documents().get_pages()[&1];
    inc.opt_clone_object_to_new_document(page).unwrap();
    inc.new_document.max_id = 40;
    let note = inc.new_document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Square",
        "Rect" => vec![10.into(), 10.into(), 50.into(), 50.into()],
    });
    let pd = inc
        .new_document
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    let mut annots = pd.get(b"Annots").unwrap().as_array().unwrap().clone();
    annots.push(note.into());
    pd.set("Annots", annots);
    bytes.clear();
    inc.save_to(&mut bytes).unwrap();
    std::fs::write(&source, &bytes).unwrap();

    let mut plan = plan_with(PlanFormDesign {
        flatten: true,
        ..PlanFormDesign::default()
    });
    plan.form = vec![("name".into(), "Flattened value".into())];
    save(&pdfium, &source, &output, &plan);

    let form = read_fields(&pdfium, &output);
    assert!(form.fields.is_empty(), "no form left");
    let doc = pdfium.load_pdf_from_file(&output, None).unwrap();
    let page = doc.pages().get(0).unwrap();
    let kinds: Vec<_> = page
        .annotations()
        .iter()
        .map(|a| a.annotation_type())
        .collect();
    assert_eq!(kinds, [PdfPageAnnotationType::Square], "the comment stays");
    drop(doc);
    assert!(
        page_text(&pdfium, &output).contains("Flattened value"),
        "the value is now page text"
    );
}

#[test]
fn flatten_alone_saves_a_form_of_untouched_fields() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("kinds.pdf");
    let output = dir.path().join("out.pdf");
    write_kinds_fixture(&source);
    save(
        &pdfium,
        &source,
        &output,
        &plan_with(PlanFormDesign {
            flatten: true,
            ..PlanFormDesign::default()
        }),
    );
    assert!(read_fields(&pdfium, &output).fields.is_empty());
}

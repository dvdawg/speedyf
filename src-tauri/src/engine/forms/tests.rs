use super::*;
use crate::engine::pdfium_init;
use crate::engine::types::{EditPlan, PlanFormButton, PlanFormChoice, PlanPage};
use std::path::Path;

/// A small AcroForm: a checkbox, a two-button radio group, and a text
/// field. Written by hand because PDFium can read form fields but cannot
/// create them, so there is no way to build this through the library the
/// rest of the engine uses.
fn write_form_fixture(path: &Path) {
    let mut objects: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut add = |n: u32, body: &[u8]| objects.push((n, body.to_vec()));

    let on = b"q 0 0 0 rg 2 2 10 10 re f Q";
    let off = b"q Q";
    for (n, content) in [(20u32, &on[..]), (21, &off[..])] {
        let mut body = format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 14 14] /Length {} >>\nstream\n",
            content.len()
        )
        .into_bytes();
        body.extend_from_slice(content);
        body.extend_from_slice(b"\nendstream");
        add(n, &body);
    }

    add(
        1,
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [10 0 R 11 0 R 14 0 R]               /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 30 0 R >> >> >> >>",
    );
    add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    add(
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400]               /Resources << /Font << /Helv 30 0 R >> >> /Contents 4 0 R               /Annots [10 0 R 12 0 R 13 0 R 14 0 R] >>",
    );
    add(4, b"<< /Length 0 >>\nstream\n\nendstream");
    add(
        30,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    add(
        10,
        b"<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Off /AS /Off /F 4               /Rect [100 345 114 359] /AP << /N << /Yes 20 0 R /Off 21 0 R >> >> >>",
    );
    add(
        11,
        b"<< /FT /Btn /Ff 49152 /T (choice) /V /Off /Kids [12 0 R 13 0 R] >>",
    );
    add(
        12,
        b"<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /F 4               /Rect [100 295 114 309] /AP << /N << /a 20 0 R /Off 21 0 R >> >> >>",
    );
    add(
        13,
        b"<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /F 4               /Rect [140 295 154 309] /AP << /N << /b 20 0 R /Off 21 0 R >> >> >>",
    );
    add(
        14,
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Before) /F 4               /Rect [100 240 250 258] /DA (/Helv 10 Tf 0 g) /AP << /N 22 0 R >> >>",
    );
    let content = b"BT /Helv 10 Tf 0 g 2 5 Td (Before) Tj ET";
    let mut appearance = format!("<< /Type /XObject /Subtype /Form /BBox [0 0 150 18] /Resources << /Font << /Helv 30 0 R >> >> /Length {} >>\nstream\n", content.len()).into_bytes();
    appearance.extend_from_slice(content);
    appearance.extend_from_slice(b"\nendstream");
    add(22, &appearance);

    write_objects(path, objects);
}

pub(super) fn write_objects(path: &Path, mut objects: Vec<(u32, Vec<u8>)>) {
    objects.sort_by_key(|(n, _)| *n);
    let mut out: Vec<u8> = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<(u32, usize)> = Vec::new();
    for (n, body) in &objects {
        offsets.push((*n, out.len()));
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let size = objects.iter().map(|(n, _)| *n).max().unwrap() + 1;
    let start_xref = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for n in 1..size {
        match offsets.iter().find(|(o, _)| *o == n) {
            Some((_, at)) => out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes()),
            None => out.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{start_xref}\n%%EOF\n")
            .as_bytes(),
    );
    std::fs::write(path, out).expect("write form fixture");
}

fn write_shared_text_fixture(path: &Path) {
    let objects: Vec<(u32, &[u8])> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 8 0 R >> >> >> >>"),
        (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Annots [6 0 R] >>"),
        (4, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Annots [7 0 R] >>"),
        (5, b"<< /FT /Tx /T (shared) /V (Before) /Ff 4096 /MaxLen 40 /Kids [6 0 R 7 0 R] >>"),
        (6, b"<< /Type /Annot /Subtype /Widget /Parent 5 0 R /F 4 /Rect [40 200 260 280] /P 3 0 R >>"),
        (7, b"<< /Type /Annot /Subtype /Widget /Parent 5 0 R /F 4 /Rect [40 200 260 280] /P 4 0 R >>"),
        (8, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
    ];
    write_objects(
        path,
        objects
            .into_iter()
            .map(|(id, value)| (id, value.to_vec()))
            .collect(),
    );
}

/// Saving a form used to hand back a page of dead boxes: `FPDF_ImportPages`
/// copies widget annotations but not the `/AcroForm` that gives them
/// meaning, so the output stopped being a form at all - and the text
/// values the user had typed went with it, silently, while the panel said
/// they had been written.
#[test]
fn a_form_survives_a_save_with_its_values() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().expect("temp dir");
    let source = dir.path().join("form.pdf");
    let output = dir.path().join("out.pdf");
    write_form_fixture(&source);

    let before = pdfium.load_pdf_from_file(&source, None).expect("fixture");
    assert!(before.form().is_some(), "fixture is not a form");
    drop(before);

    let plan = EditPlan {
        pages: vec![PlanPage {
            src_index: Some(0),
            width_pt: 300.0,
            height_pt: 400.0,
            rotation: 0,
            annots: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }],
        form: vec![("name".to_string(), "Ada".to_string())],
        // Tick the checkbox, and select the SECOND radio so the assertion
        // below cannot pass by everything simply defaulting on.
        form_buttons: vec![
            PlanFormButton {
                page: 0,
                index: 0,
                checked: true,
            },
            PlanFormButton {
                page: 0,
                index: 2,
                checked: true,
            },
        ],
        form_choices: Vec::new(),
        form_display: Vec::new(),
    };
    crate::engine::save::build_output(
        &pdfium,
        source.to_str().expect("source path"),
        None,
        &plan,
        &output,
    )
    .expect("build output");

    let after = pdfium.load_pdf_from_file(&output, None).expect("reopen");
    assert!(
        after.form().is_some(),
        "the output is no longer a form — the /AcroForm was dropped"
    );
    assert_eq!(
        after
            .form()
            .unwrap()
            .field_values(after.pages())
            .get("name"),
        Some(&Some("Ada".into()))
    );

    // Read the states back the way the engine itself does, through a
    // form-fill environment, rather than through the typed wrappers.
    let states = button_states(&pdfium, &output);
    assert_eq!(
        states,
        vec![true, false, true],
        "expected the checkbox on, the first radio off and the second on"
    );
}

#[test]
fn text_field_updates_its_saved_appearance_without_button_edits() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().expect("temp dir");
    let source = dir.path().join("form.pdf");
    let output = dir.path().join("out.pdf");
    write_form_fixture(&source);
    let plan = EditPlan {
        pages: vec![PlanPage {
            src_index: Some(0),
            width_pt: 300.0,
            height_pt: 400.0,
            rotation: 0,
            annots: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }],
        form: vec![("name".into(), "Ada".into())],
        form_buttons: Vec::new(),
        form_choices: Vec::new(),
        form_display: Vec::new(),
    };
    crate::engine::save::build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output)
        .expect("save text field");
    // Read /AP before loading a form page view or rendering: either can
    // synthesize an appearance in memory and hide a broken file on disk.
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(output.to_str().unwrap(), None);
    assert!(!doc.is_null());
    let page = b.FPDF_LoadPage(doc, 0);
    let annot = b.FPDFPage_GetAnnot(page, 3);
    let len = b.FPDFAnnot_GetAP(annot, 0, std::ptr::null_mut(), 0);
    let mut buffer = vec![0u16; len as usize / 2];
    b.FPDFAnnot_GetAP(annot, 0, buffer.as_mut_ptr(), len);
    let appearance = String::from_utf16_lossy(&buffer);
    b.FPDFPage_CloseAnnot(annot);
    b.FPDF_ClosePage(page);
    b.FPDF_CloseDocument(doc);
    assert!(
        appearance.contains("Ada"),
        "saved appearance is stale: {appearance}"
    );
    assert!(!appearance.contains("Before"));
    let reopened = pdfium
        .load_pdf_from_file(&output, None)
        .expect("reopen form");
    assert_eq!(
        reopened
            .form()
            .unwrap()
            .field_values(reopened.pages())
            .get("name"),
        Some(&Some("Ada".into()))
    );
}

#[test]
fn shared_text_fields_save_multiline_values_and_can_be_cleared() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    let filled = dir.path().join("filled.pdf");
    let cleared = dir.path().join("cleared.pdf");
    write_shared_text_fixture(&source);
    for (input, output, value) in [
        (&source, &filled, "Ada (Lovelace)\nSecond line"),
        (&filled, &cleared, ""),
    ] {
        let plan = EditPlan {
            pages: (0..2)
                .map(|index| PlanPage {
                    src_index: Some(index),
                    width_pt: 300.0,
                    height_pt: 400.0,
                    rotation: 0,
                    annots: Vec::new(),
                    texts: Vec::new(),
                    images: Vec::new(),
                    drop_src_annots: Vec::new(),
                })
                .collect(),
            form: vec![("shared".into(), value.into())],
            form_buttons: Vec::new(),
            form_choices: Vec::new(),
            form_display: Vec::new(),
        };
        crate::engine::save::build_output(&pdfium, input.to_str().unwrap(), None, &plan, output)
            .unwrap();
        let b = pdfium.bindings();
        let doc = b.FPDF_LoadDocument(output.to_str().unwrap(), None);
        assert!(!doc.is_null());
        let mut info = Box::pin(form_fill_info());
        let form = b.FPDFDOC_InitFormFillEnvironment(doc, info.as_mut().get_mut());
        let mut states = Vec::new();
        for index in 0..2 {
            let page = b.FPDF_LoadPage(doc, index);
            let annot = b.FPDFPage_GetAnnot(page, 0);
            // Deliberately no OnAfterLoadPage: inspect only what was saved.
            let field_value =
                read_utf16(|buffer, len| b.FPDFAnnot_GetFormFieldValue(form, annot, buffer, len));
            let appearance = read_utf16(|buffer, len| b.FPDFAnnot_GetAP(annot, 0, buffer, len));
            states.push((field_value, appearance));
            b.FPDFPage_CloseAnnot(annot);
            b.FPDF_ClosePage(page);
        }
        b.FPDFDOC_ExitFormFillEnvironment(form);
        b.FPDF_CloseDocument(doc);
        for (field_value, appearance) in states {
            assert_eq!(field_value.replace("\r\n", "\n").replace('\r', "\n"), value);
            assert!(!appearance.is_empty(), "missing saved appearance");
            assert!(!appearance.contains("Before"));
            assert_eq!(
                appearance.contains("Ada"),
                !value.is_empty(),
                "stale appearance: {appearance}"
            );
        }
    }
}

#[test]
fn a_rejected_text_value_preserves_the_destination() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.pdf");
    let destination = dir.path().join("destination.pdf");
    write_shared_text_fixture(&source);
    std::fs::write(&destination, b"original bytes").unwrap();
    let result = crate::engine::save::verified_atomic_replace(
        &destination,
        |temp| {
            std::fs::copy(&source, temp)?;
            apply_edits(
                pdfium.bindings(),
                temp,
                FormEdits {
                    texts: &[("shared".into(), "x".repeat(100))],
                    buttons: &[],
                    choices: &[],
                    display: &[],
                },
            )
        },
        |_| Ok(()),
        || Ok(()),
    );
    assert!(
        result.is_err(),
        "a value exceeding /MaxLen must not silently truncate"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"original bytes");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "temporary file must be removed"
    );
}

/// `(checkbox, radio a, radio b)` as PDFium reports them.
fn button_states(pdfium: &Pdfium, path: &Path) -> Vec<bool> {
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(path.to_str().expect("path"), None);
    assert!(!doc.is_null(), "reopen for verification");
    let mut info = Box::pin(form_fill_info());
    let form = b.FPDFDOC_InitFormFillEnvironment(doc, info.as_mut().get_mut());
    assert!(!form.is_null(), "no form-fill environment");
    let page = b.FPDF_LoadPage(doc, 0);
    b.FORM_OnAfterLoadPage(page, form);
    let mut out = Vec::new();
    for index in 0..b.FPDFPage_GetAnnotCount(page) {
        let annotation = b.FPDFPage_GetAnnot(page, index);
        let kind = b.FPDFAnnot_GetFormFieldType(form, annotation);
        if kind == 2 || kind == 3 {
            out.push(b.FPDFAnnot_IsChecked(form, annotation) != 0);
        }
        b.FPDFPage_CloseAnnot(annotation);
    }
    b.FORM_OnBeforeClosePage(page, form);
    b.FPDFDOC_ExitFormFillEnvironment(form);
    b.FPDF_ClosePage(page);
    b.FPDF_CloseDocument(doc);
    out
}

#[test]
fn form_field_types_have_stable_ui_kinds() {
    assert_eq!(super::read::form_field_kind(1), "button");
    assert_eq!(super::read::form_field_kind(2), "checkbox");
    assert_eq!(super::read::form_field_kind(3), "radio");
    assert_eq!(super::read::form_field_kind(4), "combo");
    assert_eq!(super::read::form_field_kind(5), "list");
    assert_eq!(super::read::form_field_kind(6), "text");
    assert_eq!(super::read::form_field_kind(7), "signature");
    assert_eq!(super::read::form_field_kind(-1), "other");
}

/// Every field kind the page overlay fills, with the attributes it reads:
/// inherited /MaxLen and comb flag, quadding, tooltips, /DV, /Opt pairs,
/// scripts, and a page whose /Tabs asks for row order while its /Annots
/// list the widgets scrambled.
fn write_kinds_fixture(path: &Path) {
    write_kinds_fixture_with(path, b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [10 0 R 11 0 R 12 0 R 13 0 R 14 0 R 16 0 R 17 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 30 0 R >> >> >> >>");
}

/// The same form with an XFA stream beside the AcroForm, as IRS forms carry.
fn write_xfa_kinds_fixture(path: &Path) {
    write_kinds_fixture_with(path, b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /XFA 31 0 R /Fields [10 0 R 11 0 R 12 0 R 13 0 R 14 0 R 16 0 R 17 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 30 0 R >> >> >> >>");
}

fn write_kinds_fixture_with(path: &Path, catalog: &[u8]) {
    let objects: Vec<(u32, &[u8])> = vec![
        (1, catalog),
        (31, b"<< /Length 8 >>\nstream\n<xdp:x/>\nendstream"),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Tabs /R /Annots [16 0 R 13 0 R 12 0 R 15 0 R 14 0 R 10 0 R 17 0 R] >>"),
        (10, b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /TU (Your name) /Ff 2 /MaxLen 20 /Q 1 /DA (/Helv 0 Tf 0 0 1 rg) /DV (Default) /V (Before) /F 4 /Rect [20 350 200 370] /P 3 0 R /AA << /K << /S /JavaScript /JS (AFNumber_Keystroke\\(2\\)) >> /F << /S /JavaScript /JS (x) >> >> >>"),
        (11, b"<< /FT /Tx /T (acct) /Ff 16777216 /MaxLen 9 /Kids [15 0 R] >>"),
        (15, b"<< /Type /Annot /Subtype /Widget /Parent 11 0 R /F 4 /Rect [20 300 200 320] /DA (/Helv 12 Tf 0 g) /P 3 0 R >>"),
        (12, b"<< /Type /Annot /Subtype /Widget /FT /Ch /T (state) /Ff 393216 /Opt [[(CA) (California)] [(NY) (New York)] [(TX) (Texas)]] /V (NY) /DV (CA) /F 4 /Rect [20 250 200 270] /DA (/Helv 10 Tf 0 g) /P 3 0 R >>"),
        (13, b"<< /Type /Annot /Subtype /Widget /FT /Ch /T (colors) /Ff 2097152 /Opt [(Red) (Green) (Blue)] /V [(Red)] /DV [(Blue) (Green)] /F 4 /Rect [20 150 200 230] /DA (/Helv 10 Tf 0 g) /P 3 0 R >>"),
        (14, b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (notes) /Ff 4096 /F 4 /Rect [220 150 290 370] /DA (/Helv 9 Tf 0 g) /P 3 0 R >>"),
        (16, b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (pin) /Ff 8192 /F 4 /Rect [220 100 290 120] /DA (/Helv 10 Tf 0 g) /P 3 0 R >>"),
        (17, b"<< /Type /Annot /Subtype /Widget /FT /Ch /T (size) /Ff 131072 /Opt [(Small) (Large)] /V (Small) /F 4 /Rect [20 100 200 120] /DA (/Helv 10 Tf 0 g) /P 3 0 R >>"),
        (30, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
    ];
    write_objects(
        path,
        objects
            .into_iter()
            .map(|(id, value)| (id, value.to_vec()))
            .collect(),
    );
}

fn read_fields(pdfium: &Pdfium, path: &Path) -> crate::engine::types::FormDto {
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(path.to_str().unwrap(), None);
    assert!(!doc.is_null(), "open {}", path.display());
    let pages = b.FPDF_GetPageCount(doc) as u32;
    let form = read_form(b, doc, pages, path.to_str().unwrap(), None);
    b.FPDF_CloseDocument(doc);
    form
}

fn field<'f>(
    form: &'f crate::engine::types::FormDto,
    name: &str,
) -> &'f crate::engine::types::FormFieldDto {
    form.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn one_page_plan() -> EditPlan {
    EditPlan {
        pages: vec![PlanPage {
            src_index: Some(0),
            width_pt: 300.0,
            height_pt: 400.0,
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
    }
}

#[test]
fn reading_a_form_reports_what_the_overlay_needs() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("kinds.pdf");
    write_kinds_fixture(&source);
    let form = read_fields(&pdfium, &source);
    assert!(!form.xfa);

    let name = field(&form, "name");
    assert_eq!(name.kind, "text");
    assert_eq!(name.tooltip, "Your name");
    assert!(name.required);
    assert_eq!(name.max_len, 20);
    assert_eq!(name.align, 1);
    assert_eq!(
        name.font_size, 0.0,
        "a 0 Tf default appearance is auto-size"
    );
    assert_eq!(name.color, "#0000ff");
    assert_eq!(name.default_value, "Default");
    assert_eq!(name.script_count, 2);

    let acct = field(&form, "acct");
    assert!(acct.comb, "comb flag inherited from the parent field");
    assert_eq!(acct.max_len, 9, "/MaxLen inherited from the parent field");
    assert_eq!(acct.font_size, 12.0);
    assert!(!acct.required);

    let state = field(&form, "state");
    assert_eq!(state.kind, "combo");
    assert!(state.editable);
    let labels: Vec<_> = state.options.iter().map(|o| o.label.as_str()).collect();
    let exports: Vec<_> = state.options.iter().map(|o| o.export.as_str()).collect();
    assert_eq!(labels, ["California", "New York", "Texas"]);
    assert_eq!(exports, ["CA", "NY", "TX"]);
    assert_eq!(state.selected, [1]);
    assert_eq!(state.default_selected, [0]);

    let colors = field(&form, "colors");
    assert_eq!(colors.kind, "list");
    assert!(colors.multi_select);
    assert_eq!(colors.selected, [0]);
    assert_eq!(colors.default_selected, [1, 2]);

    assert!(field(&form, "notes").multiline);
    assert!(field(&form, "pin").password);
    assert!(!field(&form, "size").editable);

    // /Tabs /R: rows top to bottom, each left to right, whatever /Annots says.
    let mut by_order: Vec<_> = form.fields.iter().collect();
    by_order.sort_by_key(|f| f.tab_order);
    let names: Vec<_> = by_order.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["name", "notes", "acct", "state", "colors", "size", "pin"]
    );
}

#[test]
fn choice_fields_save_selections_and_typed_text() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("kinds.pdf");
    let first = dir.path().join("first.pdf");
    let second = dir.path().join("second.pdf");
    write_kinds_fixture(&source);

    let mut plan = one_page_plan();
    plan.form_choices = vec![
        PlanFormChoice {
            field: "state".into(),
            selected: Vec::new(),
            text: Some("Oregon".into()),
        },
        PlanFormChoice {
            field: "colors".into(),
            selected: vec![1, 2],
            text: None,
        },
        PlanFormChoice {
            field: "size".into(),
            selected: vec![1],
            text: None,
        },
    ];
    crate::engine::save::build_output(&pdfium, source.to_str().unwrap(), None, &plan, &first)
        .expect("save choices");
    let form = read_fields(&pdfium, &first);
    assert_eq!(field(&form, "state").value, "Oregon");
    assert_eq!(field(&form, "colors").selected, [1, 2]);
    assert_eq!(field(&form, "size").selected, [1]);
    assert_eq!(field(&form, "size").value, "Large");
    assert!(
        saved_appearance(&pdfium, &first, 2).contains("Oregon"),
        "typed combo text missing from its saved appearance"
    );

    // Back to an option, and down to one colour.
    let mut plan = one_page_plan();
    plan.form_choices = vec![
        PlanFormChoice {
            field: "state".into(),
            selected: vec![2],
            text: None,
        },
        PlanFormChoice {
            field: "colors".into(),
            selected: vec![0],
            text: None,
        },
    ];
    crate::engine::save::build_output(&pdfium, first.to_str().unwrap(), None, &plan, &second)
        .expect("save choices again");
    let form = read_fields(&pdfium, &second);
    assert_eq!(field(&form, "state").selected, [2]);
    assert_eq!(field(&form, "state").value, "TX");
    assert_eq!(field(&form, "colors").selected, [0]);
    let appearance = saved_appearance(&pdfium, &second, 2);
    assert!(appearance.contains("Texas"), "stale combo: {appearance}");
    assert!(!appearance.contains("Oregon"));
}

/// The saved /AP /N of annotation `index` on page 0, read before any page
/// view could regenerate it.
fn saved_appearance(pdfium: &Pdfium, path: &Path, index: i32) -> String {
    let b = pdfium.bindings();
    let doc = b.FPDF_LoadDocument(path.to_str().unwrap(), None);
    let page = b.FPDF_LoadPage(doc, 0);
    let annot = b.FPDFPage_GetAnnot(page, index);
    let appearance = read_utf16(|buffer, len| b.FPDFAnnot_GetAP(annot, 0, buffer, len));
    b.FPDFPage_CloseAnnot(annot);
    b.FPDF_ClosePage(page);
    b.FPDF_CloseDocument(doc);
    appearance
}

#[test]
fn form_edits_are_appended_not_rewritten() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("kinds.pdf");
    let output = dir.path().join("out.pdf");
    write_kinds_fixture(&source);
    let mut plan = one_page_plan();
    plan.form = vec![("name".into(), "Ada".into())];
    plan.form_choices = vec![PlanFormChoice {
        field: "colors".into(),
        selected: vec![2],
        text: None,
    }];
    crate::engine::save::build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output)
        .unwrap();
    let before = std::fs::read(&source).unwrap();
    let after = std::fs::read(&output).unwrap();
    assert!(
        after.len() > before.len() && after.starts_with(&before),
        "the source bytes (and any signature over them) must survive a form save"
    );
    let form = read_fields(&pdfium, &output);
    assert_eq!(field(&form, "name").value, "Ada");
    assert_eq!(field(&form, "colors").selected, [2]);
}

#[test]
fn row_and_column_tab_orders_group_by_band() {
    use super::read::{geometric_order, TabsOrder};
    use crate::engine::types::RectDto;
    let r = |x: f32, y: f32| RectDto {
        x,
        y,
        w: 50.0,
        h: 14.0,
    };
    // Two rows; the second widget of the top row sits 3pt lower than the
    // first, as real forms often do.
    let rects = [
        r(200.0, 300.0),
        r(10.0, 250.0),
        r(10.0, 303.0),
        r(200.0, 250.0),
    ];
    assert_eq!(geometric_order(&rects, TabsOrder::Row), [2, 0, 1, 3]);
    assert_eq!(geometric_order(&rects, TabsOrder::Column), [2, 1, 0, 3]);
    assert_eq!(geometric_order(&rects, TabsOrder::Annots), [0, 1, 2, 3]);
}

#[test]
fn saving_a_form_with_xfa_drops_the_stale_xfa_copy() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("xfa.pdf");
    let output = dir.path().join("out.pdf");
    write_xfa_kinds_fixture(&source);
    assert!(read_fields(&pdfium, &source).xfa);
    let mut plan = one_page_plan();
    plan.form = vec![("name".into(), "Ada".into())];
    crate::engine::save::build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output)
        .unwrap();
    let form = read_fields(&pdfium, &output);
    assert!(
        !form.xfa,
        "Acrobat would show the XFA copy's values over ours"
    );
    assert_eq!(field(&form, "name").value, "Ada");
    let before = std::fs::read(&source).unwrap();
    assert!(std::fs::read(&output).unwrap().starts_with(&before));
}

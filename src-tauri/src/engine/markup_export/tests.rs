use super::*;
use crate::engine::pdfium_init;
use lopdf::{dictionary, Object, Stream};
use pdfium_render::prelude::*;
use std::path::PathBuf;

fn test_pdfium() -> Pdfium {
    Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
}

/// Four pages built by hand, covering what export must not disturb:
///   0: plain, with a GoTo link annotation to page 1 and some text;
///   1: /Rotate 90 with a CropBox inside a larger MediaBox;
///   2: /Rotate 180;
///   3: /Rotate 270, MediaBox inherited from the page tree.
/// Plus a two-entry outline and an Info title.
fn source_fixture(path: &Path) {
    let mut doc = lopdf::Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources = dictionary! { "Font" => dictionary! { "F1" => font_id } };
    let content = |label: &str| {
        Stream::new(
            dictionary! {},
            format!("BT /F1 18 Tf 40 200 Td ({label}) Tj ET").into_bytes(),
        )
    };
    type Spec = (&'static str, i64, Option<[i64; 4]>, Option<[i64; 4]>);
    let specs: [Spec; 4] = [
        ("PAGE-A", 0, Some([0, 0, 300, 400]), None),
        (
            "PAGE-B",
            90,
            Some([0, 0, 400, 300]),
            Some([10, 20, 390, 290]),
        ),
        ("PAGE-C", 180, Some([0, 0, 320, 420]), None),
        ("PAGE-D", 270, None, None),
    ];
    let mut kids = Vec::new();
    for (label, rotate, media, crop) in specs {
        let content_id = doc.add_object(content(label));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources.clone(),
            "Rotate" => rotate,
        };
        if let Some(m) = media {
            page.set(
                "MediaBox",
                m.iter().map(|&v| v.into()).collect::<Vec<Object>>(),
            );
        }
        if let Some(c) = crop {
            page.set(
                "CropBox",
                c.iter().map(|&v| v.into()).collect::<Vec<Object>>(),
            );
        }
        kids.push(doc.add_object(page));
    }
    let link_id = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![40.into(), 190.into(), 140.into(), 220.into()],
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "Dest" => vec![kids[1].into(), "XYZ".into(), 0.into(), 300.into(), Object::Null],
    });
    doc.get_object_mut(kids[0])
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![link_id.into()]);
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids.iter().map(|&k| k.into()).collect::<Vec<Object>>(),
            "Count" => kids.len() as i64,
            "MediaBox" => vec![0.into(), 0.into(), 350.into(), 450.into()],
        }),
    );
    let outlines_id = doc.new_object_id();
    let first_id = doc.new_object_id();
    let second_id = doc.new_object_id();
    doc.objects.insert(
        first_id,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Introduction"),
            "Parent" => outlines_id,
            "Next" => second_id,
            "Dest" => vec![kids[0].into(), "Fit".into()],
        }),
    );
    doc.objects.insert(
        second_id,
        Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("Results"),
            "Parent" => outlines_id,
            "Prev" => first_id,
            "Dest" => vec![kids[2].into(), "Fit".into()],
        }),
    );
    doc.objects.insert(
        outlines_id,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => first_id,
            "Last" => second_id,
            "Count" => 2,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "Outlines" => outlines_id,
    });
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal("Fixture Paper"),
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal(b"0123456789abcdef".to_vec()),
            Object::string_literal(b"0123456789abcdef".to_vec()),
        ],
    );
    doc.save(path).expect("save fixture");
}

/// A one-page PDF standing in for a captured sheet.
fn sheet_pdf(pdfium: &Pdfium, w: f32, h: f32, label: &str) -> Vec<u8> {
    let mut document = pdfium.create_new_pdf().expect("sheet doc");
    let font = document.fonts_mut().helvetica();
    let mut page = document
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::Custom(
            PdfPoints::new(w),
            PdfPoints::new(h),
        ))
        .expect("sheet page");
    let mut text =
        PdfPageTextObject::new(&document, label, font, PdfPoints::new(10.0)).expect("text");
    text.translate(PdfPoints::new(8.0), PdfPoints::new(h - 20.0))
        .expect("place text");
    page.objects_mut().add_text_object(text).expect("add text");
    document.save_to_bytes().expect("sheet bytes")
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn request(src: &Path, dest: &Path) -> MarkupExportRequest {
    MarkupExportRequest {
        source: src.to_string_lossy().into_owned(),
        dest: dest.to_string_lossy().into_owned(),
        margin_pt: 180.0,
        marks: Vec::new(),
        sheets: Vec::new(),
        css: None,
    }
}

fn sheet(kind: SheetKind, page: Option<u32>, w: f32, h: f32) -> MarkupSheet {
    MarkupSheet {
        kind,
        page,
        width_pt: w,
        height_pt: h,
        html: "<p>x</p>".into(),
        links: Vec::new(),
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> RectDto {
    RectDto { x, y, w, h }
}

#[test]
fn widening_targets_the_displayed_right_edge() {
    let b = [10.0, 20.0, 390.0, 290.0];
    assert_eq!(widen_display_right(b, 0, 100.0), [10.0, 20.0, 490.0, 290.0]);
    assert_eq!(
        widen_display_right(b, 90, 100.0),
        [10.0, 20.0, 390.0, 390.0]
    );
    assert_eq!(
        widen_display_right(b, 180, 100.0),
        [-90.0, 20.0, 390.0, 290.0]
    );
    assert_eq!(
        widen_display_right(b, 270, 100.0),
        [10.0, -80.0, 390.0, 290.0]
    );
}

#[test]
fn widening_never_moves_existing_content_on_screen() {
    let crop = [10.0, 20.0, 390.0, 290.0];
    for rot in [0u16, 90, 180, 270] {
        let before = PageSpace::from_box(crop, rot);
        let after = PageSpace::from_box(widen_display_right(crop, rot, 75.0), rot);
        let (w, h) = display_size(crop, rot);
        let (w2, h2) = display_size(widen_display_right(crop, rot, 75.0), rot);
        assert_eq!((w2, h2), (w + 75.0, h), "rot {rot}");
        for (dx, dy) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w * 0.3, h * 0.7)] {
            assert_eq!(before.disp_to_user(dx, dy), after.disp_to_user(dx, dy));
        }
    }
}

#[test]
fn rotation_is_normalized() {
    assert_eq!(normalize_rotation(-90), 270);
    assert_eq!(normalize_rotation(450), 90);
    assert_eq!(normalize_rotation(0), 0);
}

#[test]
fn sheet_matrix_puts_the_sheet_upright_in_the_strip() {
    for rot in [0u16, 90, 180, 270] {
        let crop = widen_display_right([0.0, 0.0, 300.0, 400.0], rot, 180.0);
        let (w0, h) = display_size([0.0, 0.0, 300.0, 400.0], rot);
        let geo = PageGeo {
            crop,
            rot,
            content_w: w0,
            h,
        };
        let m = sheet_matrix(&geo, [0.0, 0.0, 180.0, h]);
        let space = PageSpace::from_box(crop, rot);
        // Sheet top-left lands at the strip's top-left, bottom-right at the
        // page's bottom-right, in display space.
        let tl = m.apply(0.0, h);
        let br = m.apply(180.0, 0.0);
        let close =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(close(space.user_to_disp(tl.0, tl.1), (w0, h)), "rot {rot}");
        assert!(
            close(space.user_to_disp(br.0, br.1), (w0 + 180.0, 0.0)),
            "rot {rot}"
        );
    }
}

#[test]
fn colors_parse_strictly() {
    assert_eq!(parse_hex_color("#ff8000"), Some([255, 128, 0]));
    assert_eq!(parse_hex_color("ff8000"), None);
    assert_eq!(parse_hex_color("#ff80"), None);
    assert_eq!(parse_hex_color("#gg0000"), None);
}

#[test]
fn validation_refuses_bad_requests() {
    let dir = scratch();
    let src = dir.path().join("in.pdf");
    std::fs::write(&src, b"%PDF-1.4").unwrap();
    let dest = dir.path().join("out.pdf");
    assert!(validate(&request(&src, &dest)).is_ok());

    // Same file, directly and through a symlink.
    assert!(validate(&request(&src, &src)).is_err());
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.pdf");
        std::os::unix::fs::symlink(&src, &alias).unwrap();
        assert!(validate(&request(&src, &alias)).is_err());
    }
    let mut r = request(&src, &dest);
    r.margin_pt = f32::NAN;
    assert!(validate(&r).is_err());
    r.margin_pt = 0.0;
    assert!(validate(&r).is_err());

    let mut r = request(&src, &dest);
    r.sheets.push(sheet(SheetKind::Margin, None, 180.0, 400.0));
    assert!(validate(&r).is_err(), "margin sheet without a page");

    let mut r = request(&src, &dest);
    r.sheets
        .push(sheet(SheetKind::Margin, Some(0), 180.0, 400.0));
    r.sheets
        .push(sheet(SheetKind::Margin, Some(0), 180.0, 400.0));
    assert!(validate(&r).is_err(), "two margin sheets for one page");

    let mut r = request(&src, &dest);
    let mut s = sheet(SheetKind::Feedback, None, 480.0, 400.0);
    s.links.push(SheetLink {
        rect: rect(0.0, 0.0, 10.0, 10.0),
        target: LinkTarget::Sheet { sheet: 3, y: 0.0 },
    });
    r.sheets.push(s);
    assert!(validate(&r).is_err(), "link to a missing sheet");

    let mut r = request(&src, &dest);
    r.marks.push(MarkupMark {
        page: 0,
        kind: MarkKind::Highlight,
        quads: None,
        rect: None,
        color: "#ffcc00".into(),
        contents: String::new(),
    });
    assert!(validate(&r).is_err(), "mark with no geometry");

    let mut r = request(&src, "relative.pdf".as_ref());
    r.dest = "relative.pdf".into();
    assert!(validate(&r).is_err());
}

#[test]
fn request_parses_the_contract_shape() {
    let json = r##"{
        "source": "/a.pdf", "dest": "/b.pdf", "marginPt": 180,
        "marks": [{ "page": 0, "kind": "strikeout",
                    "quads": [{"x1":1,"y1":2,"x2":3,"y2":2,"x3":1,"y3":0,"x4":3,"y4":0}],
                    "rect": null, "color": "#cc0000", "contents": "1. typo" }],
        "sheets": [{ "kind": "margin", "page": 0, "widthPt": 180, "heightPt": 792,
                     "html": "<div></div>",
                     "links": [{ "rect": {"x":0,"y":0,"w":10,"h":10},
                                 "target": {"kind":"sheet","sheet":1,"y":12} },
                               { "rect": {"x":0,"y":20,"w":10,"h":10},
                                 "target": {"kind":"page","page":0,"y":100} }] },
                   { "kind": "feedback", "widthPt": 792, "heightPt": 792, "html": "" }],
        "css": ".x{}"
    }"##;
    let r: MarkupExportRequest = serde_json::from_str(json).expect("parse");
    assert_eq!(r.marks[0].kind, MarkKind::Strikeout);
    assert_eq!(r.sheets[1].kind, SheetKind::Feedback);
    assert_eq!(
        r.sheets[0].links[0].target,
        LinkTarget::Sheet { sheet: 1, y: 12.0 }
    );
    assert_eq!(
        r.sheets[0].links[1].target,
        LinkTarget::Page { page: 0, y: 100.0 }
    );
}

struct Exported {
    _dir: tempfile::TempDir,
    src: PathBuf,
    dest: PathBuf,
    result: MarkupExportResult,
}

fn export_fixture(pdfium: &Pdfium) -> Exported {
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("paper-markup.pdf");
    source_fixture(&src);

    let mut req = request(&src, &dest);
    let mut margin0 = sheet(SheetKind::Margin, Some(0), 180.0, 400.0);
    margin0.links.push(SheetLink {
        rect: rect(10.0, 10.0, 60.0, 14.0),
        target: LinkTarget::Sheet { sheet: 2, y: 40.0 },
    });
    margin0.links.push(SheetLink {
        rect: rect(10.0, 40.0, 60.0, 14.0),
        target: LinkTarget::Page { page: 2, y: 100.0 },
    });
    let margin1 = sheet(SheetKind::Margin, Some(1), 180.0, 380.0);
    let mut feedback = sheet(SheetKind::Feedback, None, 480.0, 400.0);
    feedback.links.push(SheetLink {
        rect: rect(20.0, 30.0, 100.0, 12.0),
        target: LinkTarget::Sheet { sheet: 0, y: 10.0 },
    });
    req.sheets = vec![margin0, margin1, feedback];
    req.marks = vec![
        MarkupMark {
            page: 0,
            kind: MarkKind::Highlight,
            quads: Some(vec![MarkupQuad {
                x1: 40.0,
                y1: 220.0,
                x2: 120.0,
                y2: 220.0,
                x3: 40.0,
                y3: 198.0,
                x4: 120.0,
                y4: 198.0,
            }]),
            rect: None,
            color: "#ffd400".into(),
            contents: "1. Justify this step".into(),
        },
        MarkupMark {
            page: 1,
            kind: MarkKind::Strikeout,
            quads: None,
            rect: Some(rect(30.0, 100.0, 80.0, 14.0)),
            color: "#cc0000".into(),
            contents: "2. teh -> the".into(),
        },
        MarkupMark {
            page: 2,
            kind: MarkKind::Caret,
            quads: None,
            rect: Some(rect(50.0, 60.0, 30.0, 12.0)),
            color: "#0055cc".into(),
            contents: "3. Insert: \u{00e9}tude".into(),
        },
        MarkupMark {
            page: 3,
            kind: MarkKind::Square,
            quads: None,
            rect: Some(rect(20.0, 20.0, 100.0, 60.0)),
            color: "#008800".into(),
            contents: "4. Figure unclear".into(),
        },
    ];
    let sheets = vec![
        sheet_pdf(pdfium, 180.0, 400.0, "MARGIN-ONE"),
        sheet_pdf(pdfium, 180.0, 380.0, "MARGIN-TWO"),
        sheet_pdf(pdfium, 480.0, 400.0, "FEEDBACK"),
    ];
    let result = export(pdfium, &req, &sheets).expect("export");
    Exported {
        _dir: dir,
        src,
        dest,
        result,
    }
}

fn subtypes(page: &PdfPage<'_>) -> Vec<PdfPageAnnotationType> {
    page.annotations()
        .iter()
        .map(|a| a.annotation_type())
        .collect()
}

#[test]
fn export_widens_annotates_appends_and_keeps_the_original() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let out = export_fixture(&pdfium);
    assert_eq!(out.result.pages, 5);
    assert_eq!(out.result.links, 3);

    // Incremental: the original file is a byte-for-byte prefix.
    let original = std::fs::read(&out.src).unwrap();
    let written = std::fs::read(&out.dest).unwrap();
    assert!(written.len() > original.len());
    assert_eq!(&written[..original.len()], &original[..]);
    // And the source itself is untouched.
    assert_eq!(std::fs::read(&out.src).unwrap(), original);

    let doc = pdfium
        .load_pdf_from_file(&out.dest, None)
        .expect("reopen export");
    let pages = doc.pages();
    assert_eq!(pages.len(), 5);

    // Display sizes: widened by the margin on the right, height unchanged.
    let size = |i: u16| {
        let p = pages.get(i).unwrap();
        (p.width().value.round(), p.height().value.round())
    };
    assert_eq!(size(0), (480.0, 400.0));
    assert_eq!(size(1), (450.0, 380.0)); // rot 90, crop 380 x 270
    assert_eq!(size(2), (500.0, 420.0)); // rot 180
    assert_eq!(size(3), (630.0, 350.0)); // rot 270, inherited 350 x 450
    assert_eq!(size(4), (480.0, 400.0)); // feedback sheet

    // Metadata and outline survive.
    assert_eq!(
        doc.metadata()
            .get(PdfDocumentMetadataTagType::Title)
            .map(|t| t.value().to_string()),
        Some("Fixture Paper".into())
    );
    let titles: Vec<String> = doc.bookmarks().iter().filter_map(|b| b.title()).collect();
    assert_eq!(titles, vec!["Introduction", "Results"]);

    // Marks became real annotations.
    let p0 = pages.get(0).unwrap();
    assert!(subtypes(&p0).contains(&PdfPageAnnotationType::Highlight));
    let highlight = p0
        .annotations()
        .iter()
        .find(|a| a.annotation_type() == PdfPageAnnotationType::Highlight)
        .unwrap();
    assert_eq!(
        highlight.contents().as_deref(),
        Some("1. Justify this step")
    );
    assert!(subtypes(&pages.get(1).unwrap()).contains(&PdfPageAnnotationType::Strikeout));
    let p2 = pages.get(2).unwrap();
    let caret = p2
        .annotations()
        .iter()
        .find(|a| a.annotation_type() == PdfPageAnnotationType::Caret)
        .expect("caret");
    assert_eq!(caret.contents().as_deref(), Some("3. Insert: \u{00e9}tude"));
    assert!(subtypes(&pages.get(3).unwrap()).contains(&PdfPageAnnotationType::Square));
}

/// Destination page index of the link at a user-space point, if any.
fn link_dest_at(pdfium: &Pdfium, path: &Path, page: usize, x: f64, y: f64) -> Option<i32> {
    let b = pdfium.bindings();
    let doc = render::open_document(b, path.to_str().unwrap(), None).unwrap();
    let p = b.FPDF_LoadPage(doc, page as c_int);
    let link = b.FPDFLink_GetLinkAtPoint(p, x, y);
    let out = if link.is_null() {
        None
    } else {
        let dest = b.FPDFLink_GetDest(doc, link);
        (!dest.is_null()).then(|| b.FPDFDest_GetDestPageIndex(doc, dest))
    };
    b.FPDF_ClosePage(p);
    b.FPDF_CloseDocument(doc);
    out
}

#[test]
fn export_keeps_old_links_and_adds_internal_ones() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let out = export_fixture(&pdfium);

    // The source's own link, untouched, still goes to page 1.
    assert_eq!(link_dest_at(&pdfium, &out.dest, 0, 60.0, 200.0), Some(1));
    // Margin sheet of page 0 (strip starts at x = 300, sheet top at y = 400):
    // first link -> the feedback page, second -> source page 2.
    assert_eq!(link_dest_at(&pdfium, &out.dest, 0, 330.0, 383.0), Some(4));
    assert_eq!(link_dest_at(&pdfium, &out.dest, 0, 330.0, 353.0), Some(2));
    // Feedback page link back to page 0's margin.
    assert_eq!(
        link_dest_at(&pdfium, &out.dest, 4, 50.0, 400.0 - 36.0),
        Some(0)
    );
}

#[test]
fn export_places_sheets_in_the_strip_without_moving_content() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let out = export_fixture(&pdfium);
    let b = pdfium.bindings();
    let before = render::open_document(b, out.src.to_str().unwrap(), None).unwrap();
    let after = render::open_document(b, out.dest.to_str().unwrap(), None).unwrap();
    for index in 0..4 {
        let p0 = b.FPDF_LoadPage(before, index);
        let p1 = b.FPDF_LoadPage(after, index);
        let bounds = |page: FPDF_PAGE, i: c_int| {
            let obj = b.FPDFPage_GetObject(page, i);
            let (mut l, mut bo, mut r, mut t) = (0.0, 0.0, 0.0, 0.0);
            b.FPDFPageObj_GetBounds(obj, &mut l, &mut bo, &mut r, &mut t);
            [l, bo, r, t]
        };
        // The original text object, first on the page, has not moved.
        let a = bounds(p0, 0);
        let z = bounds(p1, 0);
        for k in 0..4 {
            assert!((a[k] - z[k]).abs() < 0.01, "page {index}: {a:?} vs {z:?}");
        }
        // Pages 0 and 1 carry a sheet: a form object, entirely in the new
        // strip (outside the original display box, inside the new one).
        if index < 2 {
            let count = b.FPDFPage_CountObjects(p1);
            let form = (0..count)
                .map(|i| b.FPDFPage_GetObject(p1, i))
                .find(|&o| b.FPDFPageObj_GetType(o) == 5)
                .expect("form object");
            let (mut l, mut bo, mut r, mut t) = (0.0, 0.0, 0.0, 0.0);
            b.FPDFPageObj_GetBounds(form, &mut l, &mut bo, &mut r, &mut t);
            let old = PageGeo {
                crop: display_box(b, p0).unwrap(),
                rot: page_rotation(b, p0),
                content_w: 0.0,
                h: 0.0,
            };
            let space = old.space();
            let (w0, _) = display_size(old.crop, old.rot);
            let (dx1, _) = space.user_to_disp(l, bo);
            let (dx2, _) = space.user_to_disp(r, t);
            assert!(
                dx1.min(dx2) >= w0 - 0.5,
                "page {index}: sheet starts at display x {} < {w0}",
                dx1.min(dx2)
            );
        }
        b.FPDF_ClosePage(p0);
        b.FPDF_ClosePage(p1);
    }
    b.FPDF_CloseDocument(before);
    b.FPDF_CloseDocument(after);
}

#[test]
fn export_refuses_to_overwrite_the_source() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    source_fixture(&src);
    let before = std::fs::read(&src).unwrap();
    let req = request(&src, &src);
    assert!(export(&pdfium, &req, &[]).is_err());
    assert_eq!(std::fs::read(&src).unwrap(), before);
}

#[test]
fn export_rejects_out_of_range_pages_and_leaves_no_file() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("out.pdf");
    source_fixture(&src);
    let mut req = request(&src, &dest);
    req.sheets = vec![sheet(SheetKind::Margin, Some(9), 180.0, 400.0)];
    let sheets = vec![sheet_pdf(&pdfium, 180.0, 400.0, "X")];
    assert!(export(&pdfium, &req, &sheets).is_err());
    assert!(!dest.exists());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".speedyf-save-")
        })
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn export_without_sheets_or_marks_still_widens() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("out.pdf");
    source_fixture(&src);
    let mut req = request(&src, &dest);
    req.margin_pt = 120.0;
    let result = export(&pdfium, &req, &[]).unwrap();
    assert_eq!(result.pages, 4);
    assert_eq!(result.links, 0);
    let doc = pdfium.load_pdf_from_file(&dest, None).unwrap();
    assert_eq!(doc.pages().get(0).unwrap().width().value.round(), 420.0);
}

#[test]
fn permanent_id_reads_the_trailer() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    source_fixture(&src);
    let (id, pages) = permanent_id(pdfium.bindings(), src.to_str().unwrap()).expect("id");
    assert_eq!(id, "30313233343536373839616263646566");
    assert_eq!(pages, 4);
}

/// Manual end-to-end check on a real paper. Copies the PDF into a temp dir
/// (the original is never opened for writing), exports it with synthetic
/// sheets, and checks the result with PDFium (plus `qpdf --check` when it
/// is installed). Run with:
///
///   SPEEDYF_MARKUP_E2E=~/Downloads/alphanso.pdf \
///     cargo test --manifest-path src-tauri/Cargo.toml markup_export_real_paper -- --ignored --nocapture
#[test]
#[ignore]
fn markup_export_real_paper() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let original = std::env::var("SPEEDYF_MARKUP_E2E").expect("set SPEEDYF_MARKUP_E2E to a PDF");
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("paper-markup.pdf");
    std::fs::copy(&original, &src).expect("copy paper");

    // What the source has, for comparison afterwards.
    let (src_pages, src_links, src_bookmarks, src_title) = {
        let doc = pdfium.load_pdf_from_file(&src, None).unwrap();
        let links: usize = doc.pages().iter().map(|p| p.links().len()).sum();
        let title = doc
            .metadata()
            .get(PdfDocumentMetadataTagType::Title)
            .map(|t| t.value().to_string());
        (
            doc.pages().len(),
            links,
            doc.bookmarks().iter().count(),
            title,
        )
    };
    let sizes: Vec<(f32, f32)> = {
        let doc = pdfium.load_pdf_from_file(&src, None).unwrap();
        doc.pages()
            .iter()
            .map(|p| (p.width().value, p.height().value))
            .collect()
    };
    let m = 180.0;
    let mut req = request(&src, &dest);
    req.margin_pt = m;
    let mut sheets = Vec::new();
    let margins = sizes.len().min(3);
    for (i, (_, h)) in sizes.iter().enumerate().take(margins) {
        let mut s = sheet(SheetKind::Margin, Some(i as u32), m, *h);
        s.links.push(SheetLink {
            rect: rect(8.0, 8.0, 120.0, 14.0),
            target: LinkTarget::Sheet {
                sheet: margins,
                y: 0.0,
            },
        });
        req.sheets.push(s);
        sheets.push(sheet_pdf(
            &pdfium,
            m,
            *h,
            &format!("COMMENT ON PAGE {}", i + 1),
        ));
        req.marks.push(MarkupMark {
            page: i as u32,
            kind: [MarkKind::Highlight, MarkKind::Strikeout, MarkKind::Caret][i % 3],
            quads: None,
            rect: Some(rect(72.0, h - 120.0, 200.0, 12.0)),
            color: "#ffb000".into(),
            contents: format!("{}. A comment", i + 1),
        });
    }
    let (w0, h0) = sizes[0];
    let mut fb = sheet(SheetKind::Feedback, None, w0 + m, h0);
    fb.links.push(SheetLink {
        rect: rect(20.0, 20.0, 200.0, 14.0),
        target: LinkTarget::Page {
            page: (sizes.len() - 1).min(1) as u32,
            y: 72.0,
        },
    });
    req.sheets.push(fb);
    sheets.push(sheet_pdf(&pdfium, w0 + m, h0, "FEEDBACK SUMMARY"));

    let started = std::time::Instant::now();
    let result = export(&pdfium, &req, &sheets).expect("export");
    eprintln!(
        "exported {} pages, {} links in {:?} -> {}",
        result.pages,
        result.links,
        started.elapsed(),
        dest.display()
    );
    assert_eq!(result.pages as u16, src_pages + 1);

    let doc = pdfium.load_pdf_from_file(&dest, None).unwrap();
    let links: usize = doc.pages().iter().map(|p| p.links().len()).sum();
    eprintln!(
        "pages {src_pages}, links: source {src_links}, export {links}; bookmarks {src_bookmarks}"
    );
    assert!(links >= src_links + result.links as usize);
    assert_eq!(doc.bookmarks().iter().count(), src_bookmarks);
    assert_eq!(
        doc.metadata()
            .get(PdfDocumentMetadataTagType::Title)
            .map(|t| t.value().to_string()),
        src_title
    );
    for (i, page) in doc.pages().iter().enumerate().take(sizes.len()) {
        let (w, h) = sizes[i];
        assert!((page.width().value - (w + m)).abs() < 0.5, "page {i} width");
        assert!((page.height().value - h).abs() < 0.5, "page {i} height");
    }
    drop(doc);
    let original_bytes = std::fs::read(&original).unwrap();
    let written = std::fs::read(&dest).unwrap();
    assert_eq!(&written[..original_bytes.len()], &original_bytes[..]);

    if let Ok(out) = std::process::Command::new("qpdf")
        .arg("--check")
        .arg(&dest)
        .output()
    {
        eprintln!("qpdf: {}", String::from_utf8_lossy(&out.stdout));
        assert!(out.status.success(), "qpdf --check failed");
    }
    if let Ok(keep) = std::env::var("SPEEDYF_MARKUP_E2E_KEEP") {
        std::fs::copy(&dest, &keep).expect("keep output");
        eprintln!("kept a copy at {keep}");
    }
}

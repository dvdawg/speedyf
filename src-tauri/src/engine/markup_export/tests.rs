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
        target: None,
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
            target: None,
        },
        MarkupMark {
            page: 1,
            kind: MarkKind::Strikeout,
            quads: None,
            rect: Some(rect(30.0, 100.0, 80.0, 14.0)),
            color: "#cc0000".into(),
            contents: "2. teh -> the".into(),
            target: None,
        },
        MarkupMark {
            page: 2,
            kind: MarkKind::Caret,
            quads: None,
            rect: Some(rect(50.0, 60.0, 30.0, 12.0)),
            color: "#0055cc".into(),
            contents: "3. Insert: \u{00e9}tude".into(),
            target: None,
        },
        MarkupMark {
            page: 3,
            kind: MarkKind::Square,
            quads: None,
            rect: Some(rect(20.0, 20.0, 100.0, 60.0)),
            color: "#008800".into(),
            contents: "4. Figure unclear".into(),
            target: None,
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

/// The object number at the byte offset the last `startxref` points to.
fn last_xref_object(bytes: &[u8]) -> Option<u32> {
    let at = bytes.windows(9).rposition(|w| w == b"startxref")?;
    let offset: usize = std::str::from_utf8(&bytes[at + 9..])
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let head = std::str::from_utf8(&bytes[offset..(offset + 24).min(bytes.len())]).ok()?;
    head.split_whitespace().next()?.parse().ok()
}

#[test]
fn links_on_a_stream_xref_file_do_not_reuse_the_xref_stream_number() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = scratch();
    let classic = dir.path().join("classic.pdf");
    source_fixture(&classic);
    let src = dir.path().join("modern.pdf");
    let mut modern = lopdf::Document::load(&classic).unwrap();
    let mut file = std::fs::File::create(&src).unwrap();
    modern.save_modern(&mut file).unwrap();
    drop(file);

    let mut req = request(&src, &dir.path().join("unused.pdf"));
    let mut margin = sheet(SheetKind::Margin, Some(0), 180.0, 400.0);
    margin.links.push(SheetLink {
        rect: rect(10.0, 10.0, 60.0, 14.0),
        target: LinkTarget::Page { page: 2, y: 0.0 },
    });
    req.sheets = vec![margin];
    let sheets = vec![sheet_pdf(&pdfium, 180.0, 400.0, "M")];
    let pass_path = dir.path().join("pass.pdf");
    let pass = pdfium_pass(&pdfium, &req, &sheets, false, &pass_path).unwrap();
    let pass_bytes = std::fs::read(&pass_path).unwrap();
    let xref_obj = last_xref_object(&pass_bytes).expect("PDFium wrote an xref stream");
    let out = lopdf_pass(&pass_bytes, &pass.links, &pass.carets).unwrap();

    let doc = lopdf::Document::load_mem(&out).unwrap();
    let page = doc.get_pages()[&1];
    let annots = doc
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let refs: Vec<u32> = annots
        .iter()
        .filter_map(|a| a.as_reference().ok())
        .map(|(n, _)| n)
        .collect();
    assert!(!refs.contains(&xref_obj), "{refs:?} reuses {xref_obj}");
    let declared = doc.trailer.get(b"Size").unwrap().as_i64().unwrap() as u32;
    assert!(refs.iter().all(|&n| n < declared));
}

#[test]
fn only_outward_sheet_links_are_kept() {
    assert!(keeps_sheet_uri("https://doi.org/10.1000/x"));
    assert!(keeps_sheet_uri("HTTP://arxiv.org/abs/1234"));
    assert!(keeps_sheet_uri("mailto:someone@example.org"));
    assert!(!keeps_sheet_uri("http://localhost:1420/markup-print.html#"));
    assert!(!keeps_sheet_uri(
        "http://tauri.localhost/markup-print.html#c3"
    ));
    assert!(!keeps_sheet_uri("tauri://localhost/markup-print.html"));
    assert!(!keeps_sheet_uri("http://127.0.0.1/x"));
    assert!(!keeps_sheet_uri("#c3"));
    assert!(!keeps_sheet_uri(""));
}

#[test]
fn webkit_links_back_into_the_print_page_are_dropped() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("out.pdf");
    source_fixture(&src);

    // A feedback sheet carrying what WebKit makes of two anchors.
    let sheet_bytes = {
        let mut document = pdfium.create_new_pdf().unwrap();
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(480.0),
                PdfPoints::new(400.0),
            ))
            .unwrap();
        for (i, uri) in [
            "http://localhost:1420/markup-print.html#",
            "https://doi.org/10.1000/x",
        ]
        .iter()
        .enumerate()
        {
            let mut link = page.annotations_mut().create_link_annotation(uri).unwrap();
            link.set_bounds(PdfRect::new_from_values(
                300.0 - 40.0 * i as f32,
                20.0,
                320.0 - 40.0 * i as f32,
                200.0,
            ))
            .unwrap();
        }
        drop(page);
        document.save_to_bytes().unwrap()
    };
    let mut req = request(&src, &dest);
    req.sheets = vec![sheet(SheetKind::Feedback, None, 480.0, 400.0)];
    export(&pdfium, &req, &[sheet_bytes]).unwrap();

    let doc = pdfium.load_pdf_from_file(&dest, None).unwrap();
    let page = doc.pages().get(4).unwrap();
    let uris: Vec<String> = page
        .links()
        .iter()
        .filter_map(|l| l.action())
        .filter_map(|a| a.as_uri_action().and_then(|u| u.uri().ok()))
        .collect();
    assert_eq!(uris, vec!["https://doi.org/10.1000/x".to_string()]);
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
            target: None,
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

/// Checks the output of the app's markup self-test (`markup_selftest.rs`):
/// the WebKit-captured sheets sit where they belong, upright, carry their
/// text, and are not blank. Pixel statistics only; no image is written.
///
///   SPEEDYF_MARKUP_INSPECT=/path/out.pdf cargo test ... markup_selftest_output -- --ignored --nocapture
#[cfg(debug_assertions)]
#[test]
#[ignore]
fn markup_selftest_output() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let path = std::env::var("SPEEDYF_MARKUP_INSPECT").expect("set SPEEDYF_MARKUP_INSPECT");
    let doc = pdfium.load_pdf_from_file(&path, None).unwrap();
    let n = doc.pages().len();
    let margin = 180.0f32;
    // (page, x where the sheet starts, its marker)
    let checks = [
        (0u16, None, crate::markup_selftest::MARKERS[0]),
        (1, None, crate::markup_selftest::MARKERS[1]),
        (n - 2, Some(0.0f32), crate::markup_selftest::MARKERS[2]),
        (n - 1, Some(0.0), crate::markup_selftest::MARKERS[3]),
    ];
    for (index, x0, marker) in checks {
        let page = doc.pages().get(index).unwrap();
        let (w, h) = (page.width().value, page.height().value);
        let x0 = x0.unwrap_or(w - margin);
        let sheet_w = w - x0;
        // Text: the marker is inside the sheet, near its top-left.
        let text = page.text().unwrap();
        let in_sheet = text.inside_rect(PdfRect::new_from_values(0.0, x0, h, w));
        let top_band = text.inside_rect(PdfRect::new_from_values(h - 80.0, x0, h, x0 + sheet_w));
        eprintln!(
            "page {index}: {w}x{h}, sheet from x={x0}; marker in sheet: {}, in top band: {}, math text: {}",
            in_sheet.contains(marker),
            top_band.contains(marker),
            in_sheet.contains("Inline math")
        );
        assert!(in_sheet.contains(marker), "page {index}: marker missing");
        assert!(
            top_band.contains(marker),
            "page {index}: sheet not top-aligned"
        );
        assert!(
            in_sheet.contains("long comment"),
            "page {index}: wrapped text missing"
        );

        // Pixels at 1 px per pt: ink in the sheet, and the red corner mark
        // the sheet draws at its own bottom-left lands at the sheet's
        // bottom-left (not flipped, not offset).
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(w.round() as i32)
                    .set_target_height(h.round() as i32),
            )
            .unwrap();
        let (bw, bh) = (bitmap.width() as usize, bitmap.height() as usize);
        let rgba = bitmap.as_rgba_bytes();
        let px = |x: usize, y: usize| {
            let i = (y.min(bh - 1) * bw + x.min(bw - 1)) * 4;
            (rgba[i], rgba[i + 1], rgba[i + 2])
        };
        let xs = x0.round() as usize..bw;
        let mut ink = 0usize;
        for y in 0..bh {
            for x in xs.clone() {
                let (r, g, b) = px(x, y);
                if (r as u32 + g as u32 + b as u32) < 600 {
                    ink += 1;
                }
            }
        }
        let frac = ink as f32 / (xs.len() * bh) as f32;
        let red = |x: usize, y: usize| {
            let (r, g, b) = px(x, y);
            r > 150 && g < 80 && b < 80
        };
        let corner = red(x0.round() as usize + 10, bh - 10);
        let other_corners = [
            red(bw - 10, bh - 10),
            red(x0.round() as usize + 10, 10),
            red(bw - 10, 10),
        ];
        eprintln!("   ink fraction in sheet {frac:.3}; red at bottom-left {corner}, elsewhere {other_corners:?}");
        assert!(frac > 0.005, "page {index}: sheet looks blank");
        assert!(
            corner,
            "page {index}: corner mark not at the sheet's bottom-left"
        );
        assert!(
            !other_corners.iter().any(|&c| c),
            "page {index}: sheet flipped"
        );
    }
}

/// Checks the frontend half of the app's markup self-test: the review laid
/// out by `buildExportRequest` and written by `markup_export`.
///
///   SPEEDYF_MARKUP_INSPECT_FRONTEND=/path/out-frontend.pdf cargo test ... markup_frontend_output -- --ignored --nocapture
///
/// Reads the report beside it (`out-frontend.json`) for where each comment
/// was anchored.
#[test]
#[ignore]
fn markup_frontend_output() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let path = std::env::var("SPEEDYF_MARKUP_INSPECT_FRONTEND").expect("set the variable");
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&path).with_extension("json")).expect("report"),
    )
    .unwrap();
    let doc = pdfium.load_pdf_from_file(&path, None).unwrap();
    let n = doc.pages().len();
    // Words each margin note starts with, by comment number.
    let first_words = [
        "",
        "The induction",
        "This paragraph",
        "teh",
        "Cite the",
        "This figure",
        "recieve",
    ];
    let mut worst = 0.0f32;
    for anchor in report["detail"]["anchors"].as_array().unwrap() {
        let number = anchor["number"].as_u64().unwrap() as usize;
        let page_index = anchor["page"].as_u64().unwrap() as u16;
        let top = anchor["top"].as_f64().unwrap() as f32;
        let page = doc.pages().get(page_index).unwrap();
        let (w, h) = (page.width().value, page.height().value);
        let strip = w - 180.0;
        let text = page.text().unwrap();
        // The note's first words, found by scanning down the strip in bands.
        let mut found = None;
        let mut y = 0.0;
        while y < h {
            let band = text.inside_rect(PdfRect::new_from_values(h - y - 14.0, strip, h - y, w));
            if band.contains(first_words[number]) {
                found = Some(y);
                break;
            }
            y += 2.0;
        }
        let at =
            found.unwrap_or_else(|| panic!("note {number} not in the strip of page {page_index}"));
        worst = worst.max((at - top).abs());
        eprintln!("note {number}: anchored {top} from top, set at about {at}");
        assert!(
            (at - top).abs() < 30.0,
            "note {number} is {at}, anchor {top}"
        );
    }
    eprintln!("worst offset {worst}");

    // Nothing but the paper: no letter, no pointers to pages that are not
    // there.
    for i in 0..n {
        let all = doc.pages().get(i).unwrap().text().unwrap().all();
        assert!(!all.contains("Comments on"), "page {i} has a letter");
        assert!(!all.contains("see 1 on"), "page {i} points elsewhere");
    }
    // Each phrase carries its number, just after it, in the page's own area.
    for anchor in report["detail"]["anchors"].as_array().unwrap() {
        let number = anchor["number"].as_u64().unwrap().to_string();
        let page = doc
            .pages()
            .get(anchor["page"].as_u64().unwrap() as u16)
            .unwrap();
        let top = anchor["top"].as_f64().unwrap() as f32;
        let (w, h) = (page.width().value, page.height().value);
        let band = page.text().unwrap().inside_rect(PdfRect::new_from_values(
            h - top - 24.0,
            0.0,
            h - top + 12.0,
            w - 180.0,
        ));
        assert!(band.contains(&number), "no marker {number} near its phrase");
    }
}

fn label_mark(page: u32, r: RectDto, text: &str) -> MarkupMark {
    MarkupMark {
        page,
        kind: MarkKind::Label,
        quads: None,
        rect: Some(r),
        color: "#694fe3".into(),
        contents: text.into(),
        target: None,
    }
}

/// The text of a text object, read through its page's text page.
fn text_of(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE, obj: FPDF_PAGEOBJECT) -> String {
    let text_page = b.FPDFText_LoadPage(page);
    let mut buf = vec![0u16; 64];
    let n = b.FPDFTextObj_GetText(obj, text_page, buf.as_mut_ptr(), (buf.len() * 2) as c_ulong);
    b.FPDFText_ClosePage(text_page);
    let chars = (n as usize / 2).saturating_sub(1);
    String::from_utf16_lossy(&buf[..chars.min(buf.len())])
}

#[test]
fn labels_sit_at_their_phrase_upright_small_and_on_top() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let b = pdfium.bindings();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("out.pdf");
    source_fixture(&src);
    let mut req = request(&src, &dest);
    // Page 0 is upright, page 1 is /Rotate 90: each label must land where
    // asked in display space, either way.
    let cases = [
        (0u32, rect(120.0, 214.0, 8.0, 6.5), "3"),
        (1, rect(60.0, 90.0, 11.0, 6.5), "12"),
    ];
    req.marks = cases
        .iter()
        .map(|(page, r, text)| label_mark(*page, *r, text))
        .collect();
    let result = export(&pdfium, &req, &[]).expect("export");
    assert_eq!(result.pages, 4, "a label never adds a page");

    let before = render::open_document(b, src.to_str().unwrap(), None).unwrap();
    let after = render::open_document(b, dest.to_str().unwrap(), None).unwrap();
    for (page, r, text) in cases {
        let p0 = b.FPDF_LoadPage(before, page as c_int);
        let p1 = b.FPDF_LoadPage(after, page as c_int);
        let bounds = |p: FPDF_PAGE, obj: FPDF_PAGEOBJECT| {
            let (mut l, mut bo, mut ri, mut t) = (0.0, 0.0, 0.0, 0.0);
            b.FPDFPageObj_GetBounds(obj, &mut l, &mut bo, &mut ri, &mut t);
            let _ = p;
            [l, bo, ri, t]
        };
        // The original content is where it was.
        let a = bounds(p0, b.FPDFPage_GetObject(p0, 0));
        let z = bounds(p1, b.FPDFPage_GetObject(p1, 0));
        for k in 0..4 {
            assert!((a[k] - z[k]).abs() < 0.01, "page {page}: {a:?} vs {z:?}");
        }
        let geo = page_geo(b, p1).unwrap();
        let space = geo.space();
        let display = |u: [f32; 4]| {
            let (x1, y1) = space.user_to_disp(u[0], u[1]);
            let (x2, y2) = space.user_to_disp(u[2], u[3]);
            [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)]
        };
        let count = b.FPDFPage_CountObjects(p1);
        let objects: Vec<FPDF_PAGEOBJECT> =
            (0..count).map(|i| b.FPDFPage_GetObject(p1, i)).collect();
        // The number: a text object reading it, centred in its pill.
        let number = objects
            .iter()
            .copied()
            .find(|&o| b.FPDFPageObj_GetType(o) == 1 && text_of(b, p1, o) == text)
            .unwrap_or_else(|| panic!("page {page}: no label text {text}"));
        let d = display(bounds(p1, number));
        let (cx, cy) = ((d[0] + d[2]) / 2.0, (d[1] + d[3]) / 2.0);
        assert!(
            (cx - (r.x + r.w / 2.0)).abs() < 1.0 && (cy - (r.y + r.h / 2.0)).abs() < 1.2,
            "page {page}: number centred at ({cx}, {cy}), pill {r:?}"
        );
        // Upright on screen: wider than tall for "12", and inside the pill.
        assert!(
            d[0] >= r.x - 0.5 && d[2] <= r.x + r.w + 0.5,
            "page {page}: {d:?} vs {r:?}"
        );
        // The pill: a filled path exactly over the rect, drawn before (under)
        // the number and after (over) the page's own content.
        let at = objects.iter().position(|&o| o == number).unwrap();
        let pill = objects[..at]
            .iter()
            .copied()
            .rev()
            .find(|&o| b.FPDFPageObj_GetType(o) == 2)
            .expect("pill path");
        let pd = display(bounds(p1, pill));
        for (got, want) in pd.iter().zip([r.x, r.y, r.x + r.w, r.y + r.h]) {
            assert!(
                (got - want).abs() < 0.6,
                "page {page}: pill {pd:?} vs {r:?}"
            );
        }
        // Nothing opaque was laid over the page's own area apart from the
        // small pills (and the white strip and rule, which sit right of it).
        let original_w = display_size(display_box(b, p0).unwrap(), page_rotation(b, p0)).0;
        for &o in &objects[1..] {
            let ob = display(bounds(p1, o));
            if ob[0] < original_w - 0.5 {
                let area = (ob[2] - ob[0]) * (ob[3] - ob[1]);
                assert!(
                    area <= r.w * r.h + 1.0,
                    "page {page}: large object {ob:?} over the content"
                );
            }
        }
        b.FPDF_ClosePage(p0);
        b.FPDF_ClosePage(p1);
    }
    b.FPDF_CloseDocument(before);
    b.FPDF_CloseDocument(after);
}

#[test]
fn labels_need_a_rect_and_a_short_ascii_number() {
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    source_fixture(&src);
    let dest = dir.path().join("out.pdf");
    let mut req = request(&src, &dest);
    req.marks = vec![label_mark(0, rect(1.0, 1.0, 8.0, 6.0), "1234567")];
    assert!(validate(&req).is_err());
    req.marks = vec![label_mark(0, rect(1.0, 1.0, 8.0, 6.0), "")];
    assert!(validate(&req).is_err());
    req.marks = vec![label_mark(0, rect(1.0, 1.0, 8.0, 6.0), "42")];
    assert!(validate(&req).is_ok());
}

/// The display rect of each annotation of `subtype` on an exported page.
fn annot_display_rects(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    subtype: c_int,
) -> Vec<[f32; 4]> {
    let space = page_geo(b, page).unwrap().space();
    let mut out = Vec::new();
    for i in 0..b.FPDFPage_GetAnnotCount(page) {
        let annot = b.FPDFPage_GetAnnot(page, i);
        if b.FPDFAnnot_GetSubtype(annot) == subtype {
            let mut r = FS_RECTF {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            };
            b.FPDFAnnot_GetRect(annot, &mut r);
            let (x1, y1) = space.user_to_disp(r.left, r.bottom);
            let (x2, y2) = space.user_to_disp(r.right, r.top);
            out.push([x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)]);
        }
        b.FPDFPage_CloseAnnot(annot);
    }
    out
}

/// The link at a display point, as its destination page and the display
/// distance of its /XYZ top below that page's top.
fn link_at_display(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    page: FPDF_PAGE,
    x: f32,
    y: f32,
) -> Option<(i32, f32)> {
    let space = page_geo(b, page).unwrap().space();
    let (ux, uy) = space.disp_to_user(x, y);
    let link = b.FPDFLink_GetLinkAtPoint(page, ux as f64, uy as f64);
    if link.is_null() {
        return None;
    }
    let dest = b.FPDFLink_GetDest(doc, link);
    if dest.is_null() {
        return None;
    }
    let index = b.FPDFDest_GetDestPageIndex(doc, dest);
    let (mut has_x, mut has_y, mut has_z) = (0, 0, 0);
    let (mut lx, mut ly, mut lz) = (0.0f32, 0.0f32, 0.0f32);
    b.FPDFDest_GetLocationInPage(
        dest, &mut has_x, &mut has_y, &mut has_z, &mut lx, &mut ly, &mut lz,
    );
    let target = load_page(b, doc, index as usize).unwrap();
    let geo = page_geo(b, target.page).unwrap();
    let (_, top) = geo.space().user_to_disp(lx, ly);
    Some((index, geo.h - top))
}

/// Text, typo, region and equation comments on every rotation: their marks
/// and their numbers land where the display-space request put them, and a
/// click on the number goes to its note in the strip.
#[test]
fn marks_labels_and_their_links_follow_every_rotation() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let b = pdfium.bindings();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("out.pdf");
    source_fixture(&src);
    let mut req = request(&src, &dest);
    // A phrase at display (40..120, 150..162) and a region (60..160,
    // 40..100) on each of the four pages (rotations 0, 90, 180, 270).
    let quad = MarkupQuad {
        x1: 40.0,
        y1: 162.0,
        x2: 120.0,
        y2: 162.0,
        x3: 40.0,
        y3: 150.0,
        x4: 120.0,
        y4: 150.0,
    };
    let mark = |page: u32, kind: MarkKind, quads: Option<Vec<MarkupQuad>>, r: Option<RectDto>| {
        MarkupMark {
            page,
            kind,
            quads,
            rect: r,
            color: "#694fe3".into(),
            contents: "1. A comment".into(),
            target: None,
        }
    };
    for page in 0..4u32 {
        req.marks
            .push(mark(page, MarkKind::Highlight, Some(vec![quad]), None));
        req.marks
            .push(mark(page, MarkKind::Strikeout, Some(vec![quad]), None));
        req.marks.push(mark(
            page,
            MarkKind::Square,
            None,
            Some(rect(60.0, 40.0, 100.0, 60.0)),
        ));
        let mut label = label_mark(page, rect(121.0, 158.0, 8.0, 6.2), "1");
        label.target = Some(LinkTarget::Page { page, y: 30.0 });
        req.marks.push(label);
    }
    let result = export(&pdfium, &req, &[]).expect("export");
    assert_eq!(result.links, 4);

    let doc = render::open_document(b, dest.to_str().unwrap(), None).unwrap();
    let close = |a: [f32; 4], want: [f32; 4]| a.iter().zip(want).all(|(x, y)| (x - y).abs() <= 0.6);
    for page in 0..4usize {
        let p = load_page(b, doc, page).unwrap();
        let rot = page_rotation(b, p.page);
        let highlights = annot_display_rects(b, p.page, ANNOT_HIGHLIGHT);
        let strikes = annot_display_rects(b, p.page, ANNOT_STRIKEOUT);
        let squares = annot_display_rects(b, p.page, ANNOT_SQUARE);
        assert!(
            close(highlights[0], [40.0, 150.0, 120.0, 162.0]),
            "rot {rot}: highlight at {:?}",
            highlights[0]
        );
        assert!(
            close(strikes[0], [40.0, 150.0, 120.0, 162.0]),
            "rot {rot}: strikeout at {:?}",
            strikes[0]
        );
        assert!(
            close(squares[0], [60.0, 40.0, 160.0, 100.0]),
            "rot {rot}: square at {:?}",
            squares[0]
        );
        // The number's link: over its pill, to its own page, 30 pt down.
        let (to, top) = link_at_display(b, doc, p.page, 125.0, 161.0)
            .unwrap_or_else(|| panic!("rot {rot}: no link on the label"));
        assert_eq!(to as usize, page, "rot {rot}");
        assert!(
            (top - 30.0).abs() < 0.6,
            "rot {rot}: lands {top} from the top"
        );
        // Nothing links off the pill.
        assert!(link_at_display(b, doc, p.page, 80.0, 156.0).is_none());
    }
    b.FPDF_CloseDocument(doc);
}

/// A margin sheet as WebKit captures one: a page with link annotations, one
/// out to the web, one to mail and one back into the print page.
fn sheet_with_links(pdfium: &Pdfium, w: f32, h: f32) -> Vec<u8> {
    let mut document = pdfium.create_new_pdf().unwrap();
    let mut page = document
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::Custom(
            PdfPoints::new(w),
            PdfPoints::new(h),
        ))
        .unwrap();
    // (uri, [left, bottom, right, top] in the sheet's own space, y-up)
    let links = [
        (
            "https://arxiv.org/abs/1234.5678",
            [10.0, h - 40.0, 90.0, h - 30.0],
        ),
        ("mailto:author@example.org", [10.0, h - 60.0, 70.0, h - 50.0]),
        (
            "http://localhost:1420/markup-print.html#",
            [10.0, h - 80.0, 50.0, h - 70.0],
        ),
    ];
    for (uri, [l, bo, r, t]) in links {
        let mut link = page.annotations_mut().create_link_annotation(uri).unwrap();
        link.set_bounds(PdfRect::new_from_values(bo, l, t, r))
            .unwrap();
    }
    drop(page);
    document.save_to_bytes().unwrap()
}

#[test]
fn web_links_in_a_margin_note_survive_on_every_rotation() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let b = pdfium.bindings();
    let dir = scratch();
    let src = dir.path().join("paper.pdf");
    let dest = dir.path().join("out.pdf");
    source_fixture(&src);
    let mut req = request(&src, &dest);
    let mut sheets = Vec::new();
    // Display heights of the four fixture pages.
    let heights = [400.0f32, 380.0, 420.0, 350.0];
    for (page, &h) in heights.iter().enumerate() {
        req.sheets
            .push(sheet(SheetKind::Margin, Some(page as u32), 180.0, h));
        sheets.push(sheet_with_links(&pdfium, 180.0, h));
    }
    export(&pdfium, &req, &sheets).expect("export");

    let document = pdfium.load_pdf_from_file(&dest, None).unwrap();
    let doc = render::open_document(b, dest.to_str().unwrap(), None).unwrap();
    for (index, &h) in heights.iter().enumerate() {
        let page = document.pages().get(index as u16).unwrap();
        let mut uris: Vec<String> = page
            .links()
            .iter()
            .filter_map(|l| l.action())
            .filter_map(|a| a.as_uri_action().and_then(|u| u.uri().ok()))
            .collect();
        uris.sort();
        assert_eq!(
            uris,
            vec![
                "https://arxiv.org/abs/1234.5678".to_string(),
                "mailto:author@example.org".to_string(),
            ],
            "page {index}"
        );
        // The web link sits where its words were drawn: 30..40 pt below the
        // strip's top, 10..90 pt into it, in display space.
        let p = load_page(b, doc, index).unwrap();
        let geo = page_geo(b, p.page).unwrap();
        assert!((geo.h - h).abs() < 0.5, "page {index}: height {}", geo.h);
        let strip = geo.content_w - 180.0;
        let links = annot_display_rects(b, p.page, ANNOT_LINK);
        let want = [strip + 10.0, h - 40.0, strip + 90.0, h - 30.0];
        assert!(
            links
                .iter()
                .any(|r| r.iter().zip(want).all(|(a, w)| (a - w).abs() < 0.6)),
            "page {index}: links at {links:?}, want {want:?}"
        );
    }
    b.FPDF_CloseDocument(doc);
}

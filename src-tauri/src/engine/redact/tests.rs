//! Redaction on synthetic fixtures, verified by re-reading the written file:
//! PDFium text extraction, the decompressed bytes of every stream, decoded
//! image samples and rendered pixels.

use super::*;
use crate::engine::pdfium_init;
use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};
use pdfium_render::prelude::*;

fn pdfium() -> Pdfium {
    Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
}

/// A Type1 Helvetica with every code 32..126 600 units wide (our own
/// metrics: PDFium positions glyphs by /Widths, so the test and the reader
/// agree exactly).
fn mono_font(doc: &mut Document) -> ObjectId {
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
        "FirstChar" => 32,
        "LastChar" => 126,
        "Widths" => Object::Array(vec![Object::Integer(600); 95]),
    })
}

struct Fixture {
    doc: Document,
    pages_id: ObjectId,
    pages: Vec<ObjectId>,
}

impl Fixture {
    fn new() -> Fixture {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        Fixture {
            doc,
            pages_id,
            pages: Vec::new(),
        }
    }

    fn page(&mut self, content: &str, resources: lopdf::Dictionary) -> ObjectId {
        let content = self
            .doc
            .add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
        let page = self.doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(self.pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => Object::Reference(content),
            "Resources" => resources,
        });
        self.pages.push(page);
        page
    }

    fn redact(&mut self, page: ObjectId, rect: [f32; 4]) -> ObjectId {
        let annot = self.doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Redact",
            "Rect" => rect.iter().map(|&v| Object::Real(v)).collect::<Vec<_>>(),
        });
        self.add_annot(page, annot);
        annot
    }

    fn add_annot(&mut self, page: ObjectId, annot: ObjectId) {
        let d = self.doc.get_dictionary_mut(page).unwrap();
        let mut list = d
            .get(b"Annots")
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default();
        list.push(Object::Reference(annot));
        d.set("Annots", Object::Array(list));
    }

    fn finish(mut self) -> Document {
        let kids: Vec<Object> = self.pages.iter().map(|p| Object::Reference(*p)).collect();
        let count = kids.len() as i64;
        self.doc.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => count,
            }),
        );
        let catalog = self.doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => Object::Reference(self.pages_id),
        });
        self.doc.trailer.set("Root", Object::Reference(catalog));
        self.doc
    }
}

fn save(doc: &mut Document) -> Vec<u8> {
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// Apply every mark and write the clean rewrite.
fn applied(mut doc: Document) -> (Vec<u8>, Report) {
    let report = apply_document(&mut doc).expect("apply");
    let bytes = clean_rewrite(&mut doc).expect("rewrite");
    (bytes, report)
}

fn text_of_pages(pdfium: &Pdfium, bytes: &[u8]) -> Vec<String> {
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .expect("PDFium opens it");
    doc.pages()
        .iter()
        .map(|p| p.text().map(|t| t.all()).unwrap_or_default())
        .collect()
}

/// Left edge of the first occurrence of `needle` on page 0, by PDFium.
fn x_of(pdfium: &Pdfium, bytes: &[u8], needle: &str) -> Option<f32> {
    let doc = pdfium.load_pdf_from_byte_slice(bytes, None).unwrap();
    let page = doc.pages().get(0).unwrap();
    let text = page.text().unwrap();
    let all = text.chars();
    let chars: Vec<_> = all.iter().collect();
    let s: Vec<char> = chars
        .iter()
        .map(|c| c.unicode_char().unwrap_or(' '))
        .collect();
    let n: Vec<char> = needle.chars().collect();
    let at = (0..s.len().saturating_sub(n.len() - 1)).find(|&i| s[i..i + n.len()] == n[..])?;
    chars[at].loose_bounds().ok().map(|b| b.left().value)
}

/// Every stream of the file, decompressed, concatenated with the rest of
/// the bytes: what a `grep` of `qpdf --qdf` output would see.
fn all_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut doc = Document::load_mem(bytes).expect("lopdf reads it");
    doc.decompress();
    save(&mut doc)
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn font_resources(font: ObjectId) -> lopdf::Dictionary {
    dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } }
}

#[test]
fn text_across_mark_boundary_is_cut_and_the_rest_stays_put() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let font = mono_font(&mut fx.doc);
    // 10 pt, 6 pt per glyph: "Name: " is 72..108, "JOHNSMITH" 108..162.
    let page = fx.page(
        "BT /F1 10 Tf 72 700 Td (Name: JOHNSMITH end) Tj ET",
        font_resources(font),
    );
    // Covers JOHNSMITH and only slivers of its neighbours.
    fx.redact(page, [107.5, 696.0, 162.5, 710.0]);
    let mut doc = fx.finish();
    let before = save(&mut doc);
    let end_before = x_of(&pdfium, &before, "end").unwrap();

    let (after, report) = applied(doc);
    let text = &text_of_pages(&pdfium, &after)[0];
    assert!(
        !text.contains("JOHN") && !text.contains("SMITH"),
        "{text:?}"
    );
    assert!(text.contains("Name:") && text.contains("end"), "{text:?}");
    assert_eq!(report.glyphs_removed, 9);
    let end_after = x_of(&pdfium, &after, "end").unwrap();
    assert!(
        (end_before - end_after).abs() < 0.01,
        "{end_before} vs {end_after}"
    );
    let raw = all_bytes(&after);
    assert!(!contains(&raw, "JOHN") && !contains(&raw, "SMITH"));
    // A full rewrite: one revision, no /Prev chain, no /Redact left.
    assert_eq!(crate::engine::sanitize::revision_count(&after), 0);
    assert!(!contains(&raw, "/Prev") && !contains(&raw, "/Redact"));
}

#[test]
fn kerned_tj_keeps_positions() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let font = mono_font(&mut fx.doc);
    let page = fx.page(
        "BT /F1 10 Tf 72 700 Td [(Pub) -200 (lic ) 300 (SEC) 50 (RET) -100 ( tail)] TJ ET",
        font_resources(font),
    );
    // Pub 18 -> 90; +2 -> 92; "lic " 24 -> 116; -3 -> 113; SEC 18 -> 131;
    // -0.5 -> 130.5; RET 18 -> 148.5.
    fx.redact(page, [112.5, 696.0, 148.5, 710.0]);
    let mut doc = fx.finish();
    let before = save(&mut doc);
    let tail_before = x_of(&pdfium, &before, "tail").unwrap();
    let (after, _) = applied(doc);
    let text = &text_of_pages(&pdfium, &after)[0];
    assert!(!text.contains("SEC") && !text.contains("RET"), "{text:?}");
    assert!(text.contains("Pub") && text.contains("tail"), "{text:?}");
    let tail_after = x_of(&pdfium, &after, "tail").unwrap();
    assert!(
        (tail_before - tail_after).abs() < 0.01,
        "{tail_before} vs {tail_after}"
    );
}

fn to_unicode(doc: &mut Document, pairs: &[(u32, char)], two_byte: bool) -> ObjectId {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/CMapName /T def\n1 begincodespacerange\n",
    );
    cmap.push_str(if two_byte {
        "<0000> <FFFF>\n"
    } else {
        "<00> <FF>\n"
    });
    cmap.push_str(&format!("endcodespacerange\n{} beginbfchar\n", pairs.len()));
    for (code, ch) in pairs {
        if two_byte {
            cmap.push_str(&format!("<{code:04X}> <{:04X}>\n", *ch as u32));
        } else {
            cmap.push_str(&format!("<{code:02X}> <{:04X}>\n", *ch as u32));
        }
    }
    cmap.push_str("endbfchar\nendcmap CMapName currentdict /CMap defineresource pop end end\n");
    doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()))
}

#[test]
fn cid_identity_font_glyphs_are_removed() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let pairs: Vec<(u32, char)> = (1..=26)
        .map(|i| (i, (b'A' + i as u8 - 1) as char))
        .collect();
    let tu = to_unicode(&mut fx.doc, &pairs, true);
    let descriptor = fx.doc.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "Arial", "Flags" => 32,
        "FontBBox" => vec![0.into(), (-200).into(), 1000.into(), 900.into()],
        "ItalicAngle" => 0, "Ascent" => 900, "Descent" => -200, "CapHeight" => 700, "StemV" => 80,
    });
    let cid = fx.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Arial",
        "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
        "DW" => 500, "FontDescriptor" => Object::Reference(descriptor), "CIDToGIDMap" => "Identity",
    });
    let font = fx.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Arial", "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(cid)], "ToUnicode" => Object::Reference(tu),
    });
    // 12 pt, 6 pt per glyph from x=72: A 72, B 78, C 84, D 90, E 96.
    let page = fx.page(
        "BT /F1 12 Tf 72 600 Td <0001000200030004000500060007> Tj ET",
        font_resources(font),
    );
    fx.redact(page, [85.0, 596.0, 95.0, 612.0]);
    let (after, report) = applied(fx.finish());
    let text = &text_of_pages(&pdfium, &after)[0];
    assert!(text.contains("AB") && text.contains("EFG"), "{text:?}");
    assert!(!text.contains('C') && !text.contains('D'), "{text:?}");
    assert_eq!(report.glyphs_removed, 2);
    let raw = all_bytes(&after);
    assert!(!raw.windows(4).any(|w| w == [0, 3, 0, 4]));
}

#[test]
fn type3_glyphs_are_removed() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let glyph = |doc: &mut Document| {
        doc.add_object(Stream::new(
            dictionary! {},
            b"500 0 0 0 500 700 d1 0 0 500 700 re f".to_vec(),
        ))
    };
    let (ga, gb) = (glyph(&mut fx.doc), glyph(&mut fx.doc));
    let tu = to_unicode(&mut fx.doc, &[(0x61, 'a'), (0x62, 'b')], false);
    let font = fx.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type3",
        "FontBBox" => vec![0.into(), 0.into(), 500.into(), 700.into()],
        "FontMatrix" => vec![Object::Real(0.001), 0.into(), 0.into(), Object::Real(0.001), 0.into(), 0.into()],
        "CharProcs" => dictionary! { "a" => Object::Reference(ga), "b" => Object::Reference(gb) },
        "Encoding" => dictionary! { "Type" => "Encoding", "Differences" => vec![97.into(), "a".into(), "b".into()] },
        "FirstChar" => 97, "LastChar" => 98,
        "Widths" => vec![500.into(), 500.into()],
        "Resources" => dictionary! {},
        "ToUnicode" => Object::Reference(tu),
    });
    // 10 pt: a 72..77, b 77..82, a 82..87, b 87..92
    let page = fx.page("BT /F1 10 Tf 72 600 Td (abab) Tj ET", font_resources(font));
    fx.redact(page, [77.5, 598.0, 81.5, 610.0]);
    let (after, report) = applied(fx.finish());
    assert_eq!(report.glyphs_removed, 1);
    let text = &text_of_pages(&pdfium, &after)[0];
    assert_eq!(text.chars().filter(|c| *c == 'b').count(), 1, "{text:?}");
    assert_eq!(text.chars().filter(|c| *c == 'a').count(), 2, "{text:?}");
}

#[test]
fn image_pixels_under_mark_are_blanked() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let red: Vec<u8> = (0..20 * 20).flat_map(|_| [255u8, 0, 0]).collect();
    let mut image = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 20, "Height" => 20,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        red,
    );
    image.compress().unwrap();
    let image = fx.doc.add_object(image);
    let page = fx.page(
        "q 100 0 0 100 100 100 cm /Im1 Do Q",
        dictionary! { "XObject" => dictionary! { "Im1" => Object::Reference(image) } },
    );
    // The top half of the image.
    fx.redact(page, [100.0, 150.0, 200.0, 200.0]);
    let (after, report) = applied(fx.finish());
    assert_eq!(report.images_blanked, 1);

    // The image data itself: rows 0-9 (the top) black, rows 10-19 red.
    let doc = Document::load_mem(&after).unwrap();
    let images: Vec<&Stream> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| {
            s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image".as_slice())
        })
        .collect();
    assert_eq!(images.len(), 1, "the original image must be gone");
    let data = images[0].decompressed_content().unwrap();
    assert_eq!(&data[0..3], &[0, 0, 0]);
    assert_eq!(&data[9 * 60 + 57..9 * 60 + 60], &[0, 0, 0]);
    assert_eq!(&data[10 * 60..10 * 60 + 3], &[255, 0, 0]);

    // Rendered: the marked area is the fill colour, the rest still red.
    let pdf = pdfium.load_pdf_from_byte_slice(&after, None).unwrap();
    let first = pdf.pages().get(0).unwrap();
    let bitmap = first
        .render_with_config(&PdfRenderConfig::new().set_target_width(612))
        .unwrap();
    let img = bitmap.as_image().to_rgb8();
    let top = img.get_pixel(150, 792 - 175);
    let bottom = img.get_pixel(150, 792 - 125);
    assert!(top[0] < 30 && top[1] < 30 && top[2] < 30, "{top:?}");
    assert!(bottom[0] > 200 && bottom[1] < 60, "{bottom:?}");
}

#[test]
fn shared_form_xobject_is_copied_on_write() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let font = mono_font(&mut fx.doc);
    let form = fx.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 300.into(), 50.into()],
            "Resources" => font_resources(font),
        },
        b"BT /F1 10 Tf 0 10 Td (SHAREDSECRET) Tj ET".to_vec(),
    ));
    let res = || dictionary! { "XObject" => dictionary! { "Fm1" => Object::Reference(form) } };
    let p1 = fx.page("q 1 0 0 1 72 690 cm /Fm1 Do Q", res());
    let _p2 = fx.page("q 1 0 0 1 72 690 cm /Fm1 Do Q", res());
    fx.redact(p1, [70.0, 695.0, 200.0, 715.0]);
    let (after, report) = applied(fx.finish());
    assert_eq!(report.forms_rewritten, 1);
    let texts = text_of_pages(&pdfium, &after);
    assert!(!texts[0].contains("SECRET"), "{:?}", texts[0]);
    assert!(texts[1].contains("SHAREDSECRET"), "{:?}", texts[1]);
}

#[test]
fn rotated_page_mark_from_display_space() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let font = mono_font(&mut fx.doc);
    let page = fx.page(
        "BT /F1 10 Tf 72 500 Td (ROTATEDSECRET visible) Tj ET",
        font_resources(font),
    );
    fx.doc.get_dictionary_mut(page).unwrap().set("Rotate", 90);
    let mut doc = fx.finish();
    // User box x 72..150, y 496..510 on a page turned 90 degrees: display
    // x = uy, display y = 612 - ux.
    let annot: crate::engine::types::PlanAnnot = serde_json::from_value(serde_json::json!({
        "kind": "redact", "color": "#000000", "opacity": 1.0,
        "rect": { "x": 496.0, "y": 612.0 - 150.0, "w": 14.0, "h": 78.0 },
    }))
    .unwrap();
    marks::add_marks(
        &mut doc,
        &[marks::PendingMark {
            page: 0,
            rotation: 0,
            annot: &annot,
        }],
    )
    .unwrap();
    let (after, _) = applied(doc);
    let text = &text_of_pages(&pdfium, &after)[0];
    assert!(
        !text.contains("ROTATED") && !text.contains("SECRET"),
        "{text:?}"
    );
    assert!(text.contains("visible"), "{text:?}");
}

#[test]
fn paths_inside_are_removed_and_crossing_ones_clipped() {
    let mut fx = Fixture::new();
    let page = fx.page(
        "0 0 1 rg 200 200 20 20 re f 1 0 0 RG 100 300 m 400 300 l S 50 50 10 10 re f",
        dictionary! {},
    );
    fx.redact(page, [190.0, 190.0, 230.0, 230.0]);
    fx.redact(page, [200.0, 290.0, 250.0, 310.0]);
    let (after, report) = applied(fx.finish());
    assert_eq!(report.paths_removed, 1);
    assert_eq!(report.paths_clipped, 1);
    let raw = String::from_utf8_lossy(&all_bytes(&after)).to_string();
    assert!(!raw.contains("200 200 20 20 re"), "{raw}");
    assert!(raw.contains("50 50 10 10 re"));
    assert!(raw.contains("W*"));
}

#[test]
fn overlapping_annotations_links_and_fields_go() {
    let mut fx = Fixture::new();
    let page = fx.page("", dictionary! {});
    let link_in = fx.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![100.into(), 100.into(), 150.into(), 120.into()],
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com/secret") },
    });
    let link_out = fx.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link",
        "Rect" => vec![400.into(), 400.into(), 450.into(), 420.into()],
    });
    let widget = fx.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
        "T" => Object::string_literal("ssn"), "V" => Object::string_literal("123-45-6789"),
        "Rect" => vec![100.into(), 200.into(), 200.into(), 220.into()],
    });
    for a in [link_in, link_out, widget] {
        fx.add_annot(page, a);
    }
    fx.redact(page, [90.0, 90.0, 210.0, 230.0]);
    let mut doc = fx.finish();
    let form = doc.add_object(dictionary! { "Fields" => vec![Object::Reference(widget)] });
    doc.catalog_mut()
        .unwrap()
        .set("AcroForm", Object::Reference(form));
    let (after, report) = applied(doc);
    assert_eq!(report.annots_removed, 2);
    assert_eq!(report.fields_removed, 1);
    let raw = all_bytes(&after);
    assert!(!contains(&raw, "123-45-6789") && !contains(&raw, "example.com/secret"));
    let doc = Document::load_mem(&after).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    assert_eq!(doc.get_page_annotations(page).unwrap().len(), 1);
}

#[test]
fn unknown_metrics_remove_the_whole_show() {
    let pdfium = pdfium();
    let mut fx = Fixture::new();
    let font = fx.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let page = fx.page(
        "BT /F1 10 Tf 72 700 Td (TOPSECRET) Tj 0 -20 Td (Other line) Tj ET",
        font_resources(font),
    );
    fx.redact(page, [80.0, 696.0, 100.0, 710.0]);
    let (after, report) = applied(fx.finish());
    assert_eq!(report.text_ops_removed, 1);
    let text = &text_of_pages(&pdfium, &after)[0];
    assert!(
        !text.contains("SECRET") && text.contains("Other line"),
        "{text:?}"
    );
}

#[test]
fn actual_text_of_cut_sequences_is_dropped() {
    let mut fx = Fixture::new();
    let font = mono_font(&mut fx.doc);
    let page = fx.page(
        "/Span << /ActualText (HIDDENNAME) >> BDC BT /F1 10 Tf 72 700 Td (XXXXXXXX) Tj ET EMC",
        font_resources(font),
    );
    fx.redact(page, [70.0, 696.0, 130.0, 710.0]);
    let (after, _) = applied(fx.finish());
    assert!(!contains(&all_bytes(&after), "HIDDENNAME"));
}

#[test]
fn overlay_text_and_fill_colour_are_drawn() {
    let mut fx = Fixture::new();
    let page = fx.page("", dictionary! {});
    let annot = fx.redact(page, [100.0, 100.0, 200.0, 120.0]);
    let d = fx.doc.get_dictionary_mut(annot).unwrap();
    d.set(
        "IC",
        vec![Object::Real(0.0), Object::Real(0.0), Object::Real(1.0)],
    );
    d.set(
        "OverlayText",
        Object::String(b"Redacted".to_vec(), StringFormat::Literal),
    );
    let (after, _) = applied(fx.finish());
    let raw = String::from_utf8_lossy(&all_bytes(&after)).to_string();
    assert!(raw.contains("0.000 0.000 1.000 rg"), "{raw}");
    assert!(raw.contains("(Redacted) Tj"), "{raw}");
}

#[test]
fn signatures_are_detected_and_removed() {
    let mut fx = Fixture::new();
    let page = fx.page("", dictionary! {});
    fx.redact(page, [10.0, 10.0, 20.0, 20.0]);
    let mut doc = fx.finish();
    let sig_value = doc.add_object(dictionary! { "Type" => "Sig", "Filter" => "Adobe.PPKLite" });
    let field = doc.add_object(dictionary! {
        "FT" => "Sig", "T" => Object::string_literal("Signature1"), "V" => Object::Reference(sig_value),
        "Subtype" => "Widget", "Rect" => vec![300.into(), 300.into(), 400.into(), 350.into()],
    });
    let form =
        doc.add_object(dictionary! { "Fields" => vec![Object::Reference(field)], "SigFlags" => 3 });
    doc.catalog_mut()
        .unwrap()
        .set("AcroForm", Object::Reference(form));
    assert!(has_signatures(&doc));
    let (after, report) = applied(doc);
    assert_eq!(report.signatures_removed, 1);
    assert!(!has_signatures(&Document::load_mem(&after).unwrap()));
}

/// The save path end to end: an unapplied mark is written as a /Redact
/// annotation and read back as one; applied, the text is gone.
#[test]
fn save_path_keeps_or_applies_marks() {
    let pdfium = pdfium();
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.pdf");
    let mut fx = Fixture::new();
    let font = mono_font(&mut fx.doc);
    fx.page(
        "BT /F1 10 Tf 72 700 Td (Name: JOHNSMITH end) Tj ET",
        font_resources(font),
    );
    std::fs::write(&src, save(&mut fx.finish())).unwrap();

    let plan = |apply: bool| -> crate::engine::types::EditPlan {
        serde_json::from_value(serde_json::json!({
            "pages": [{
                "srcIndex": 0, "widthPt": 612.0, "heightPt": 792.0, "rotation": 0,
                "annots": [{
                    "kind": "redact", "color": "#336699", "opacity": 1.0, "text": "Withheld",
                    "rect": { "x": 107.5, "y": 696.0, "w": 55.0, "h": 14.0 },
                    "quads": [{
                        "p1": { "x": 107.5, "y": 696.0 }, "p2": { "x": 162.5, "y": 696.0 },
                        "p3": { "x": 162.5, "y": 710.0 }, "p4": { "x": 107.5, "y": 710.0 }
                    }]
                }],
                "texts": [], "images": []
            }],
            "form": [],
            "scrub": { "applyRedactions": apply }
        }))
        .unwrap()
    };

    let kept = dir.path().join("kept.pdf");
    crate::engine::save::build_output(&pdfium, src.to_str().unwrap(), None, &plan(false), &kept)
        .unwrap();
    let pages =
        crate::engine::annots::read_annotations(&pdfium, kept.to_str().unwrap(), None).unwrap();
    let marks: Vec<_> = pages[0]
        .annots
        .iter()
        .filter(|a| a.kind == "redact")
        .collect();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].color, "#336699");
    assert_eq!(marks[0].text.as_deref(), Some("Withheld"));
    assert!(text_of_pages(&pdfium, &std::fs::read(&kept).unwrap())[0].contains("JOHNSMITH"));

    let done = dir.path().join("done.pdf");
    crate::engine::save::build_output(&pdfium, src.to_str().unwrap(), None, &plan(true), &done)
        .unwrap();
    let bytes = std::fs::read(&done).unwrap();
    let text = &text_of_pages(&pdfium, &bytes)[0];
    assert!(!text.contains("JOHN") && text.contains("end"), "{text:?}");
    assert!(!contains(&all_bytes(&bytes), "JOHNSMITH"));
    assert_eq!(crate::engine::sanitize::revision_count(&bytes), 0);
}

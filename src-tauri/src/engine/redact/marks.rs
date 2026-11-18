//! Writing redaction marks as /Redact annotations.
//!
//! PDFium cannot create /Redact annotations, so they are written with lopdf
//! after the rest of the save: as an incremental update when the marks are
//! only being kept for later, or straight into the document about to be
//! rewritten when they are being applied.
//!
//! Each mark carries /QuadPoints (text marks) or just /Rect (area marks),
//! /IC for the fill colour, /OverlayText, and a normal appearance that
//! outlines it so viewers without redaction support still show it. The fill
//! colour is also kept as `/SpeedyFFill`, because PDFium refuses to read /IC
//! from an annotation that has an appearance stream.

use super::geom::Rect;
use super::inherited;
use crate::engine::save::PageSpace;
use crate::engine::types::{EditPlan, PlanAnnot};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

pub const FILL_KEY: &str = "SpeedyFFill";

/// One mark from the plan, still in display space.
pub struct PendingMark<'a> {
    pub page: usize,
    /// The user's extra rotation of this page, already applied to the
    /// output's /Rotate when the mark is written.
    pub rotation: u16,
    pub annot: &'a PlanAnnot,
}

/// The plan's redaction marks, by output page.
pub fn plan_marks(plan: &EditPlan) -> Vec<PendingMark<'_>> {
    plan.pages
        .iter()
        .enumerate()
        .flat_map(|(page, p)| {
            p.annots
                .iter()
                .filter(|a| a.kind == "redact")
                .map(move |annot| PendingMark {
                    page,
                    rotation: p.rotation % 360,
                    annot,
                })
        })
        .collect()
}

fn parse_hex(hex: &str) -> [f32; 3] {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    if h.len() == 6 {
        [
            ((v >> 16) & 0xff) as f32 / 255.0,
            ((v >> 8) & 0xff) as f32 / 255.0,
            (v & 0xff) as f32 / 255.0,
        ]
    } else {
        [0.0; 3]
    }
}

fn reals(v: &[f32]) -> Object {
    Object::Array(v.iter().map(|&x| Object::Real(x)).collect())
}

/// A PDF text string: literal for ASCII, UTF-16BE with a BOM otherwise.
pub(crate) fn text_string(s: &str) -> Object {
    if s.is_ascii() {
        return Object::string_literal(s.as_bytes().to_vec());
    }
    let mut bytes = vec![0xfe, 0xff];
    for unit in s.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, StringFormat::Hexadecimal)
}

/// The page space a mark's display coordinates were taken in: the output
/// page's crop box and its rotation before the user's extra turn.
fn space_of(doc: &Document, page: ObjectId, extra: u16) -> PageSpace {
    let rect = |key: &[u8]| -> Option<[f32; 4]> {
        let a = inherited(doc, page, key)?.as_array().ok()?;
        let v: Vec<f32> = a.iter().filter_map(super::geom::number).collect();
        (v.len() == 4).then(|| {
            [
                v[0].min(v[2]),
                v[1].min(v[3]),
                v[0].max(v[2]),
                v[1].max(v[3]),
            ]
        })
    };
    let crop = rect(b"CropBox")
        .or_else(|| rect(b"MediaBox"))
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let rotate = inherited(doc, page, b"Rotate")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360) as u16;
    let original = (rotate + 360 - extra % 360) % 360;
    PageSpace::from_box(crop, original - original % 90)
}

/// The mark's areas in user space: one per quad, or its rectangle.
fn user_rects(mark: &PendingMark<'_>, space: &PageSpace) -> Vec<Rect> {
    let to_user = |x: f32, y: f32| space.disp_to_user(x, y);
    let mut out = Vec::new();
    if let Some(quads) = &mark.annot.quads {
        for q in quads {
            let pts = [
                to_user(q.p1.x, q.p1.y),
                to_user(q.p2.x, q.p2.y),
                to_user(q.p3.x, q.p3.y),
                to_user(q.p4.x, q.p4.y),
            ];
            let r = Rect::bounding(&pts);
            if r.is_valid() && r.area() > 0.0 {
                out.push(r);
            }
        }
    }
    if out.is_empty() {
        let r = &mark.annot.rect;
        let a = to_user(r.x, r.y);
        let b = to_user(r.x + r.w, r.y + r.h);
        let r = Rect::new(a.0, a.1, b.0, b.1);
        if r.area() > 0.0 {
            out.push(r);
        }
    }
    out
}

/// The annotation dictionary and its appearance stream for one mark.
fn mark_objects(mark: &PendingMark<'_>, page: ObjectId, rects: &[Rect]) -> (Dictionary, Stream) {
    let bounds = rects[1..].iter().fold(rects[0], |acc, r| acc.union(r));
    let fill = parse_hex(&mark.annot.color);
    let mut quads = Vec::new();
    let mut outline = String::from("q 0.78 0.16 0.16 RG 1 w\n");
    for r in rects {
        // Acrobat's order: top-left, top-right, bottom-left, bottom-right.
        quads.extend([r.x0, r.y1, r.x1, r.y1, r.x0, r.y0, r.x1, r.y0]);
        outline.push_str(&format!(
            "{:.3} {:.3} {:.3} {:.3} re S\n",
            r.x0 + 0.5,
            r.y0 + 0.5,
            (r.width() - 1.0).max(0.0),
            (r.height() - 1.0).max(0.0)
        ));
    }
    outline.push_str("Q\n");
    let appearance = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => reals(&[bounds.x0, bounds.y0, bounds.x1, bounds.y1]),
            "Resources" => Dictionary::new(),
        },
        outline.into_bytes(),
    );
    let mut dict = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Redact",
        "Rect" => reals(&[bounds.x0, bounds.y0, bounds.x1, bounds.y1]),
        "IC" => reals(&fill),
        "C" => reals(&[0.78, 0.16, 0.16]),
        "F" => 4,
        "P" => Object::Reference(page),
        FILL_KEY => Object::string_literal(mark.annot.color.clone().into_bytes()),
    };
    if mark.annot.quads.as_ref().is_some_and(|q| !q.is_empty()) {
        dict.set("QuadPoints", reals(&quads));
    }
    if let Some(text) = mark.annot.text.as_deref().filter(|t| !t.trim().is_empty()) {
        dict.set("OverlayText", text_string(text));
    }
    (dict, appearance)
}

/// Add the marks to a document that is about to be rewritten.
pub fn add_marks(doc: &mut Document, marks: &[PendingMark<'_>]) -> Result<(), String> {
    if marks.is_empty() {
        return Ok(());
    }
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for mark in marks {
        let page = *pages
            .get(mark.page)
            .ok_or_else(|| format!("page {} not found for a redaction mark", mark.page + 1))?;
        let space = space_of(doc, page, mark.rotation);
        let rects = user_rects(mark, &space);
        if rects.is_empty() {
            continue;
        }
        let (mut dict, ap) = mark_objects(mark, page, &rects);
        let ap = doc.add_object(ap);
        dict.set("AP", dictionary! { "N" => Object::Reference(ap) });
        let id = doc.add_object(dict);
        let existing = doc
            .get_dictionary(page)
            .map_err(|e| e.to_string())?
            .get(b"Annots")
            .ok()
            .cloned();
        match existing {
            Some(Object::Reference(array)) => {
                doc.get_object_mut(array)
                    .and_then(Object::as_array_mut)
                    .map_err(|e| e.to_string())?
                    .push(Object::Reference(id));
            }
            Some(Object::Array(mut a)) => {
                a.push(Object::Reference(id));
                doc.get_dictionary_mut(page)
                    .map_err(|e| e.to_string())?
                    .set("Annots", Object::Array(a));
            }
            _ => {
                doc.get_dictionary_mut(page)
                    .map_err(|e| e.to_string())?
                    .set("Annots", Object::Array(vec![Object::Reference(id)]));
            }
        }
    }
    Ok(())
}

/// Append the marks to `bytes` as an incremental update.
pub fn append_marks_incremental(
    bytes: Vec<u8>,
    marks: &[PendingMark<'_>],
) -> Result<Vec<u8>, String> {
    let mut inc: lopdf::IncrementalDocument = bytes
        .as_slice()
        .try_into()
        .map_err(|e: lopdf::Error| e.to_string())?;
    if inc.get_prev_documents().trailer.has(b"Encrypt") {
        return Err(
            "the document is encrypted; remove its password to keep redaction marks".into(),
        );
    }
    // New objects are numbered past the previous /Size - see markup_export.
    let declared = inc
        .get_prev_documents()
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    let pages: Vec<ObjectId> = inc
        .get_prev_documents()
        .get_pages()
        .values()
        .copied()
        .collect();
    let mut added: std::collections::BTreeMap<usize, Vec<ObjectId>> = Default::default();
    for mark in marks {
        let page = *pages
            .get(mark.page)
            .ok_or_else(|| format!("page {} not found for a redaction mark", mark.page + 1))?;
        let space = space_of(inc.get_prev_documents(), page, mark.rotation);
        let rects = user_rects(mark, &space);
        if rects.is_empty() {
            continue;
        }
        let (mut dict, ap) = mark_objects(mark, page, &rects);
        let ap = inc.new_document.add_object(ap);
        dict.set("AP", dictionary! { "N" => Object::Reference(ap) });
        let id = inc.new_document.add_object(dict);
        added.entry(mark.page).or_default().push(id);
    }
    for (page, ids) in added {
        let pid = pages[page];
        inc.opt_clone_object_to_new_document(pid)
            .map_err(|e| e.to_string())?;
        let existing = inc
            .new_document
            .get_object(pid)
            .and_then(Object::as_dict)
            .map_err(|e| e.to_string())?
            .get(b"Annots")
            .ok()
            .cloned();
        let refs = ids.into_iter().map(Object::Reference);
        let array = match existing {
            Some(Object::Reference(array_id)) => {
                inc.opt_clone_object_to_new_document(array_id)
                    .map_err(|e| e.to_string())?;
                inc.new_document
                    .get_object_mut(array_id)
                    .and_then(Object::as_array_mut)
                    .map_err(|e| e.to_string())?
                    .extend(refs);
                continue;
            }
            Some(Object::Array(mut a)) => {
                a.extend(refs);
                a
            }
            _ => refs.collect(),
        };
        inc.new_document
            .get_object_mut(pid)
            .and_then(Object::as_dict_mut)
            .map_err(|e| e.to_string())?
            .set("Annots", Object::Array(array));
    }
    let mut out = Vec::with_capacity(bytes.len() + 4096);
    inc.save_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn string_value(
    bindings: &dyn pdfium_render::prelude::PdfiumLibraryBindings,
    annot: pdfium_render::prelude::FPDF_ANNOTATION,
    key: &str,
) -> Option<String> {
    if !bindings.is_true(bindings.FPDFAnnot_HasKey(annot, key)) {
        return None;
    }
    let len = bindings.FPDFAnnot_GetStringValue(annot, key, std::ptr::null_mut(), 0);
    if len <= 2 || len > 1 << 20 {
        return None;
    }
    let mut buffer = vec![0u16; (len as usize).div_ceil(2)];
    bindings.FPDFAnnot_GetStringValue(annot, key, buffer.as_mut_ptr(), len);
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    Some(String::from_utf16_lossy(&buffer))
}

/// The fill colour (`#rrggbb`) and overlay text of the /Redact annotation at
/// `index`. The colour comes from SpeedyF's own key, else from /IC when PDFium
/// will read it (it will not once an appearance exists), else black.
pub(crate) fn read_stored(
    bindings: &dyn pdfium_render::prelude::PdfiumLibraryBindings,
    page: pdfium_render::prelude::FPDF_PAGE,
    index: u32,
) -> (String, Option<String>) {
    let annot = bindings.FPDFPage_GetAnnot(page, index as i32);
    if annot.is_null() {
        return ("#000000".into(), None);
    }
    let mut color = string_value(bindings, annot, FILL_KEY).filter(|c| {
        c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit())
    });
    if color.is_none() {
        let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
        // FPDFANNOT_COLORTYPE_InteriorColor = 1
        let ok = bindings.FPDFAnnot_GetColor(annot, 1, &mut r, &mut g, &mut b, &mut a);
        if bindings.is_true(ok) {
            color = Some(format!(
                "#{:02x}{:02x}{:02x}",
                r.min(255),
                g.min(255),
                b.min(255)
            ));
        }
    }
    let overlay = string_value(bindings, annot, "OverlayText").filter(|t| !t.trim().is_empty());
    bindings.FPDFPage_CloseAnnot(annot);
    (color.unwrap_or_else(|| "#000000".into()), overlay)
}

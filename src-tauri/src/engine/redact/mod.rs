//! True redaction: removing what lies under a mark from the file, not
//! covering it.
//!
//! Marks are /Redact annotations (ISO 32000-2 12.5.6.23), which is also how
//! they are saved while not yet applied, so Acrobat and other tools see them
//! as marks too. Applying (`apply_document`) consumes every /Redact
//! annotation in the document:
//!
//! 1. the page content is filtered (`content.rs`): covered glyphs cut out of
//!    their strings, covered image pixels blanked and re-encoded, vector
//!    paths dropped or clipped, form XObjects descended into and copied on
//!    write;
//! 2. annotations, links and form widgets overlapping a mark are removed, the
//!    widgets' fields with them;
//! 3. the fill boxes (and overlay text) are drawn on top;
//! 4. digital signatures are removed - the document they signed no longer
//!    exists.
//!
//! The caller then writes the document as a full rewrite with no incremental
//! history and no unreferenced objects (`clean_rewrite`), so nothing removed
//! survives in an earlier revision.

pub mod content;
pub mod find;
pub mod fonts;
pub mod geom;
pub mod image;
pub mod marks;
#[cfg(test)]
mod tests;

use content::{decode_ops, Filter, MarkArea};
use geom::{number, Matrix, Rect};
use lopdf::content::Content;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// What applying redactions did, for the log and the tests.
#[derive(Default, Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub pages: u32,
    pub marks: u32,
    pub glyphs_removed: u64,
    pub text_ops_removed: u64,
    pub images_blanked: u32,
    pub images_removed: u32,
    pub inline_images_removed: u32,
    pub paths_removed: u32,
    pub paths_clipped: u32,
    pub forms_rewritten: u32,
    pub forms_removed: u32,
    pub annots_removed: u32,
    pub fields_removed: u32,
    pub signatures_removed: u32,
    pub warnings: Vec<String>,
}

/// One /Redact annotation: its areas in user space, fill and overlay text.
#[derive(Clone, Debug)]
pub struct Mark {
    pub rects: Vec<Rect>,
    pub fill: [f32; 3],
    pub overlay: Option<String>,
}

/// How a save finishes protection work, from the edit plan.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScrubPlan {
    /// Apply every redaction mark: remove what lies under it.
    #[serde(default)]
    pub apply_redactions: bool,
    /// Remove hidden information (`engine::sanitize`).
    #[serde(default)]
    pub remove_hidden: Option<crate::engine::sanitize::SanitizeOptions>,
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

/// A page attribute, inherited through /Parent.
pub(crate) fn inherited<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut id = page;
    for _ in 0..64 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(v) = dict.get(key) {
            return Some(deref(doc, v));
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// A colour array (/IC, /C) as RGB.
pub(crate) fn rgb_of(o: Option<&Object>) -> Option<[f32; 3]> {
    let a = o?.as_array().ok()?;
    let v: Vec<f32> = a.iter().filter_map(number).collect();
    match v.len() {
        1 => Some([v[0]; 3]),
        3 => Some([v[0], v[1], v[2]]),
        4 => Some([
            (1.0 - v[0]) * (1.0 - v[3]),
            (1.0 - v[1]) * (1.0 - v[3]),
            (1.0 - v[2]) * (1.0 - v[3]),
        ]),
        _ => None,
    }
}

/// A PDF text string (UTF-16BE with BOM, or PDFDocEncoding read as Latin-1).
pub(crate) fn text_of(o: &Object) -> Option<String> {
    let bytes = o.as_str().ok()?;
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Some(String::from_utf16_lossy(&units))
    } else {
        Some(bytes.iter().map(|&b| b as char).collect())
    }
}

fn annot_ids(doc: &Document, page: ObjectId) -> Vec<Object> {
    let Ok(dict) = doc.get_dictionary(page) else {
        return Vec::new();
    };
    match dict.get(b"Annots").ok().map(|o| deref(doc, o)) {
        Some(Object::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

fn subtype_of(dict: &Dictionary) -> &[u8] {
    dict.get(b"Subtype")
        .and_then(Object::as_name)
        .unwrap_or(b"")
}

/// The /Redact annotations of a page, read as marks.
pub fn page_marks(doc: &Document, page: ObjectId) -> Vec<Mark> {
    let mut out = Vec::new();
    for entry in annot_ids(doc, page) {
        let Object::Dictionary(a) = deref(doc, &entry) else {
            continue;
        };
        if subtype_of(a) != b"Redact" {
            continue;
        }
        let mut rects = Vec::new();
        if let Ok(Object::Array(q)) = a.get(b"QuadPoints").map(|o| deref(doc, o)) {
            let v: Vec<f32> = q.iter().filter_map(number).collect();
            for quad in v.chunks_exact(8) {
                let pts: Vec<(f32, f32)> = quad.chunks_exact(2).map(|p| (p[0], p[1])).collect();
                let r = Rect::bounding(&pts);
                if r.is_valid() && r.area() > 0.0 {
                    rects.push(r);
                }
            }
        }
        if rects.is_empty() {
            if let Ok(Object::Array(r)) = a.get(b"Rect").map(|o| deref(doc, o)) {
                let v: Vec<f32> = r.iter().filter_map(number).collect();
                if v.len() == 4 {
                    let r = Rect::new(v[0], v[1], v[2], v[3]);
                    if r.area() > 0.0 {
                        rects.push(r);
                    }
                }
            }
        }
        if rects.is_empty() {
            continue;
        }
        out.push(Mark {
            rects,
            fill: rgb_of(a.get(b"IC").ok().map(|o| deref(doc, o))).unwrap_or([0.0; 3]),
            overlay: a
                .get(b"OverlayText")
                .ok()
                .map(|o| deref(doc, o))
                .and_then(text_of)
                .filter(|s| !s.trim().is_empty()),
        });
    }
    out
}

/// The page's resources, inherited if need be, as an owned dictionary.
fn page_resources(doc: &Document, page: ObjectId) -> Dictionary {
    match inherited(doc, page, b"Resources") {
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    }
}

/// The page's content streams, decoded and concatenated.
fn page_content(doc: &Document, page: ObjectId) -> Result<Vec<u8>, String> {
    let dict = doc.get_dictionary(page).map_err(|e| e.to_string())?;
    let refs: Vec<Object> = match dict.get(b"Contents").ok() {
        None => Vec::new(),
        Some(o) => match deref(doc, o) {
            Object::Array(a) => a.clone(),
            _ => vec![o.clone()],
        },
    };
    let mut bytes = Vec::new();
    for r in refs {
        match deref(doc, &r) {
            Object::Stream(s) => {
                let data = s
                    .decompressed_content()
                    .map_err(|e| format!("cannot decode the page content: {e}"))?;
                bytes.extend_from_slice(&data);
                bytes.push(b'\n');
            }
            Object::Null => {}
            _ => return Err("the page content is not a stream".into()),
        }
    }
    Ok(bytes)
}

/// Text that only uses characters Helvetica's WinAnsi encoding has; others
/// become '?'.
fn win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c {
            ' '..='~' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            _ => b'?',
        })
        .collect()
}

const OVERLAY_FONT: &[u8] = b"SpRdHelv";

/// Fill boxes and overlay text, in default user space.
fn fill_ops(doc: &mut Document, marks: &[Mark], resources: &mut Dictionary) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let mut font_needed = false;
    for m in marks {
        let [r, g, b] = m.fill;
        for rect in &m.rects {
            let _ = writeln!(
                s,
                "q {r:.3} {g:.3} {b:.3} rg {:.3} {:.3} {:.3} {:.3} re f Q",
                rect.x0,
                rect.y0,
                rect.width(),
                rect.height()
            );
        }
        if let Some(text) = &m.overlay {
            font_needed = true;
            let ink = if 0.299 * r + 0.587 * g + 0.114 * b < 0.5 {
                1.0
            } else {
                0.0
            };
            let mut literal = Vec::new();
            let _ = lopdf::Writer::write_object(
                &mut literal,
                &Object::String(win_ansi(text), lopdf::StringFormat::Literal),
            );
            let literal = String::from_utf8_lossy(&literal).to_string();
            for rect in &m.rects {
                let size = (rect.height() * 0.7).clamp(4.0, 12.0);
                let base = rect.y0 + (rect.height() - size * 0.7) / 2.0;
                let _ = writeln!(
                    s,
                    "q {:.3} {:.3} {:.3} {:.3} re W n BT /{} {size:.2} Tf {ink} g {:.3} {base:.3} Td {literal} Tj ET Q",
                    rect.x0,
                    rect.y0,
                    rect.width(),
                    rect.height(),
                    String::from_utf8_lossy(OVERLAY_FONT),
                    rect.x0 + 1.0,
                );
            }
        }
    }
    if font_needed {
        let font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let mut fonts = match resources.get(b"Font").ok().map(|o| deref(doc, o)) {
            Some(Object::Dictionary(d)) => d.clone(),
            _ => Dictionary::new(),
        };
        fonts.set(OVERLAY_FONT.to_vec(), Object::Reference(font));
        resources.set("Font", Object::Dictionary(fonts));
    }
    s
}

/// Apply every /Redact annotation in `doc`.
pub fn apply_document(doc: &mut Document) -> Result<Report, String> {
    let mut report = Report::default();
    let pages = doc.get_pages();
    for (number, page) in pages {
        let marks = page_marks(doc, page);
        if marks.is_empty() {
            // Unusable marks (no area) still go.
            remove_redact_annots(doc, page);
            continue;
        }
        report.pages += 1;
        report.marks += marks.len() as u32;
        redact_page(doc, page, &marks, &mut report).map_err(|e| format!("page {number}: {e}"))?;
    }
    if report.pages > 0 {
        report.signatures_removed = remove_signatures(doc);
    }
    Ok(report)
}

fn remove_redact_annots(doc: &mut Document, page: ObjectId) {
    let kept: Vec<Object> = annot_ids(doc, page)
        .into_iter()
        .filter(|e| match deref(doc, e) {
            Object::Dictionary(a) => subtype_of(a) != b"Redact",
            _ => true,
        })
        .collect();
    if let Ok(dict) = doc.get_dictionary_mut(page) {
        if dict.has(b"Annots") {
            dict.set("Annots", Object::Array(kept));
        }
    }
}

fn redact_page(
    doc: &mut Document,
    page: ObjectId,
    marks: &[Mark],
    report: &mut Report,
) -> Result<(), String> {
    let areas: Vec<MarkArea> = marks
        .iter()
        .flat_map(|m| {
            m.rects.iter().map(|r| MarkArea {
                rect: *r,
                fill: m.fill,
            })
        })
        .collect();
    let mut resources = page_resources(doc, page);
    let bytes = page_content(doc, page)?;
    let ops = decode_ops(&bytes, report)?;
    let (ops, _) =
        Filter::new(doc, &areas, report).filter(ops, &mut resources, Matrix::IDENTITY, 0)?;
    let mut content = b"q\n".to_vec();
    content.extend(
        Content { operations: ops }
            .encode()
            .map_err(|e| format!("cannot encode the page content: {e}"))?,
    );
    content.extend_from_slice(b"\nQ\n");
    content.extend(fill_ops(doc, marks, &mut resources).into_bytes());
    let mut stream = Stream::new(Dictionary::new(), content);
    let _ = stream.compress();
    let contents = doc.add_object(stream);

    let rects: Vec<Rect> = areas.iter().map(|a| a.rect).collect();
    remove_annotations(doc, page, &rects, report);

    let dict = doc.get_dictionary_mut(page).map_err(|e| e.to_string())?;
    dict.set("Contents", Object::Reference(contents));
    dict.set("Resources", Object::Dictionary(resources));
    // A thumbnail is a picture of the page as it was.
    dict.remove(b"Thumb");
    Ok(())
}

/// Remove the page's /Redact annotations and every annotation overlapping a
/// mark, with the popups of removed ones and the fields of removed widgets.
fn remove_annotations(doc: &mut Document, page: ObjectId, rects: &[Rect], report: &mut Report) {
    let entries = annot_ids(doc, page);
    let mut removed: HashSet<ObjectId> = HashSet::new();
    let mut widgets: Vec<ObjectId> = Vec::new();
    let mut keep: Vec<bool> = Vec::with_capacity(entries.len());
    for e in &entries {
        let id = e.as_reference().ok();
        let Object::Dictionary(a) = deref(doc, e) else {
            keep.push(true);
            continue;
        };
        let sub = subtype_of(a);
        let hit = sub == b"Redact" || {
            let r = a
                .get(b"Rect")
                .ok()
                .map(|o| deref(doc, o))
                .and_then(|o| o.as_array().ok())
                .map(|v| v.iter().filter_map(number).collect::<Vec<_>>())
                .filter(|v| v.len() == 4)
                .map(|v| Rect::new(v[0], v[1], v[2], v[3]));
            r.is_some_and(|r| {
                rects
                    .iter()
                    .any(|m| m.overlaps(&r) || (r.area() == 0.0 && m.contains_point(r.x0, r.y0)))
            })
        };
        if hit {
            if let Some(id) = id {
                removed.insert(id);
                if sub == b"Widget" {
                    widgets.push(id);
                }
            }
            if sub != b"Redact" {
                report.annots_removed += 1;
            }
        }
        keep.push(!hit);
    }
    // Popups of removed annotations.
    for (k, e) in entries.iter().enumerate() {
        if !keep[k] {
            continue;
        }
        if let Object::Dictionary(a) = deref(doc, e) {
            if subtype_of(a) == b"Popup" {
                if let Ok(Object::Reference(parent)) = a.get(b"Parent") {
                    if removed.contains(parent) {
                        keep[k] = false;
                    }
                }
            }
        }
    }
    let kept: Vec<Object> = entries
        .into_iter()
        .zip(keep)
        .filter_map(|(e, k)| k.then_some(e))
        .collect();
    if let Ok(dict) = doc.get_dictionary_mut(page) {
        if kept.is_empty() {
            dict.remove(b"Annots");
        } else {
            dict.set("Annots", Object::Array(kept));
        }
    }
    for w in widgets {
        if remove_widget(doc, w) {
            report.fields_removed += 1;
        }
    }
}

/// The AcroForm dictionary, made direct in the catalog so it can be edited.
fn acroform_mut(doc: &mut Document) -> Option<&mut Dictionary> {
    let form = doc.catalog().ok()?.get(b"AcroForm").ok()?.clone();
    let id = match form {
        Object::Reference(id) => id,
        Object::Dictionary(_) => {
            return doc
                .catalog_mut()
                .ok()?
                .get_mut(b"AcroForm")
                .ok()?
                .as_dict_mut()
                .ok();
        }
        _ => return None,
    };
    doc.get_dictionary_mut(id).ok()
}

/// Remove `child` from the /Kids of `parent`, or from the AcroForm's /Fields
/// when it has no parent. Returns whether it was found.
fn detach(doc: &mut Document, parent: Option<ObjectId>, child: ObjectId) -> bool {
    let list = match parent {
        Some(p) => doc
            .get_dictionary(p)
            .ok()
            .and_then(|d| d.get(b"Kids").ok().cloned()),
        None => acroform_mut(doc).and_then(|f| f.get(b"Fields").ok().cloned()),
    };
    let (holder, array) = match list {
        Some(Object::Reference(id)) => (
            Some(id),
            doc.get_object(id)
                .ok()
                .and_then(|o| o.as_array().ok())
                .cloned(),
        ),
        Some(Object::Array(a)) => (None, Some(a)),
        _ => return false,
    };
    let Some(array) = array else {
        return false;
    };
    let before = array.len();
    let kept: Vec<Object> = array
        .into_iter()
        .filter(|o| o.as_reference().ok() != Some(child))
        .collect();
    if kept.len() == before {
        return false;
    }
    let left = kept.len();
    match holder {
        Some(id) => {
            if let Ok(o) = doc.get_object_mut(id) {
                *o = Object::Array(kept);
            }
        }
        None => match parent {
            Some(p) => {
                if let Ok(d) = doc.get_dictionary_mut(p) {
                    d.set("Kids", Object::Array(kept));
                }
            }
            None => {
                if let Some(f) = acroform_mut(doc) {
                    f.set("Fields", Object::Array(kept));
                }
            }
        },
    }
    // A field left with no widgets goes too.
    if left == 0 {
        if let Some(p) = parent {
            let grand = doc
                .get_dictionary(p)
                .ok()
                .and_then(|d| d.get(b"Parent").ok())
                .and_then(|o| o.as_reference().ok());
            detach(doc, grand, p);
        }
    }
    true
}

/// Remove a widget's field (or the widget from its field). Returns whether a
/// field was affected.
fn remove_widget(doc: &mut Document, widget: ObjectId) -> bool {
    let parent = doc
        .get_dictionary(widget)
        .ok()
        .and_then(|d| d.get(b"Parent").ok())
        .and_then(|o| o.as_reference().ok());
    detach(doc, parent, widget)
}

/// Every terminal field of the AcroForm with its inherited field type.
fn terminal_fields(doc: &Document) -> Vec<(ObjectId, Vec<u8>)> {
    let mut out = Vec::new();
    let Some(fields) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .map(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|f| f.get(b"Fields").ok())
        .map(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
    else {
        return out;
    };
    let mut stack: Vec<(ObjectId, Vec<u8>, u32)> = fields
        .iter()
        .filter_map(|o| o.as_reference().ok())
        .map(|id| (id, Vec::new(), 0))
        .collect();
    let mut seen = HashSet::new();
    while let Some((id, ft, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(id) {
            continue;
        }
        let Ok(d) = doc.get_dictionary(id) else {
            continue;
        };
        let ft = d
            .get(b"FT")
            .and_then(Object::as_name)
            .map(<[u8]>::to_vec)
            .unwrap_or(ft);
        let kids: Vec<ObjectId> = d
            .get(b"Kids")
            .ok()
            .map(|o| deref(doc, o))
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        let field_kids: Vec<ObjectId> = kids
            .iter()
            .copied()
            .filter(|k| {
                doc.get_dictionary(*k)
                    .map(|d| d.has(b"T") || d.has(b"Kids"))
                    .unwrap_or(false)
            })
            .collect();
        if field_kids.is_empty() {
            out.push((id, ft));
        } else {
            stack.extend(field_kids.into_iter().map(|k| (k, ft.clone(), depth + 1)));
        }
    }
    out
}

/// Whether the document carries a digital signature (a /Sig field with a
/// value). The tiny local check the plan asks for; `engine::signing` may
/// offer a fuller one.
pub fn has_signatures(doc: &Document) -> bool {
    terminal_fields(doc)
        .iter()
        .any(|(id, ft)| ft == b"Sig" && doc.get_dictionary(*id).is_ok_and(|d| d.has(b"V")))
}

/// Remove every signature value (and its appearance, which would otherwise
/// still show "signed by"), and the catalog's signature permissions.
fn remove_signatures(doc: &mut Document) -> u32 {
    let signed: Vec<ObjectId> = terminal_fields(doc)
        .into_iter()
        .filter(|(id, ft)| ft == b"Sig" && doc.get_dictionary(*id).is_ok_and(|d| d.has(b"V")))
        .map(|(id, _)| id)
        .collect();
    for id in &signed {
        let kids: Vec<ObjectId> = doc
            .get_dictionary(*id)
            .ok()
            .and_then(|d| d.get(b"Kids").ok())
            .map(|o| deref(doc, o))
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        if let Ok(d) = doc.get_dictionary_mut(*id) {
            d.remove(b"V");
            d.remove(b"Lock");
            d.remove(b"SV");
            d.remove(b"AP");
        }
        for k in kids {
            if let Ok(d) = doc.get_dictionary_mut(k) {
                d.remove(b"AP");
            }
        }
    }
    if !signed.is_empty() {
        if let Ok(c) = doc.catalog_mut() {
            c.remove(b"Perms");
        }
        if let Some(f) = acroform_mut(doc) {
            f.remove(b"SigFlags");
        }
    }
    signed.len() as u32
}

/// Write `doc` as a complete new file: no earlier revisions, no unreferenced
/// objects, objects renumbered from 1.
pub fn clean_rewrite(doc: &mut Document) -> Result<Vec<u8>, String> {
    for key in [b"Prev".as_slice(), b"XRefStm", b"Encrypt"] {
        doc.trailer.remove(key);
    }
    doc.prune_objects();
    doc.renumber_objects();
    doc.compress();
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| format!("cannot write: {e}"))?;
    Ok(out)
}

/// Finish a save: write the plan's redaction marks, apply them and remove
/// hidden information as the plan asks. Called by `save::build_output` on
/// its finished output.
pub fn finish_output(
    plan: &crate::engine::types::EditPlan,
    out: &std::path::Path,
) -> crate::errors::AppResult<()> {
    use crate::errors::AppError;
    let scrub = &plan.scrub;
    let pending = marks::plan_marks(plan);
    let rewrite = scrub.apply_redactions || scrub.remove_hidden.is_some();
    if pending.is_empty() && !rewrite {
        return Ok(());
    }
    let bytes = std::fs::read(out)?;
    if !rewrite {
        let updated = marks::append_marks_incremental(bytes, &pending)
            .map_err(|e| AppError::Internal(format!("cannot write redaction marks: {e}")))?;
        std::fs::write(out, updated)?;
        return Ok(());
    }
    let mut doc = Document::load_mem(&bytes)
        .map_err(|e| AppError::Malformed(format!("cannot read the document to clean it: {e}")))?;
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        return Err(AppError::Unsupported(
            "remove the document's password before applying redactions or removing hidden information".into(),
        ));
    }
    marks::add_marks(&mut doc, &pending).map_err(AppError::Internal)?;
    if scrub.apply_redactions {
        let report = apply_document(&mut doc).map_err(AppError::Internal)?;
        log::info!("redactions applied: {report:?}");
    }
    if let Some(options) = &scrub.remove_hidden {
        let removed = crate::engine::sanitize::clean(&mut doc, options);
        log::info!("hidden information removed: {removed:?}");
    }
    let bytes = clean_rewrite(&mut doc).map_err(AppError::Internal)?;
    std::fs::write(out, bytes)?;
    Ok(())
}

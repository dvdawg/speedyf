//! Signing a PDF with a certificate: PAdES B-B (ETSI.CAdES.detached), or
//! adbe.pkcs7.detached for readers that want the older form.
//!
//! The signature goes in as one incremental update appended to the file as
//! it is on disk, so earlier signatures stay valid and everything this does
//! not touch stays byte-identical:
//!
//! 1. `prepare` adds (or fills) a signature field and its widget with a
//!    visible appearance, and a signature dictionary whose `/Contents` is a
//!    hex placeholder of zeros, generously sized; `/ByteRange` covers the
//!    whole file except that placeholder.
//! 2. The two byte ranges are hashed with SHA-256.
//! 3. `cms::build` makes CMS SignedData over that digest with the signed
//!    attributes PAdES asks for (contentType, messageDigest,
//!    signingCertificateV2); an optional RFC 3161 timestamp is added as an
//!    unsigned attribute (tsa.rs).
//! 4. `embed` writes the DER into the placeholder in place: no offset moves.
//!
//! Certifying (DocMDP, P = 1, 2 or 3) and locking fields (FieldMDP) are
//! written into the same update, so they are covered by the signature.

mod appearance;
pub mod cms;
#[cfg(test)]
mod tests;
pub mod tsa;

use super::keys::CmsSigner;
use crate::engine::save::PageSpace;
use crate::engine::types::{RectDto, SignatureArt};
use crate::errors::{AppError, AppResult};
use lopdf::{dictionary, Dictionary, IncrementalDocument, Object, ObjectId, StringFormat};
use serde::Deserialize;
use sha2_11::{Digest, Sha256};

/// Which field the signature goes in.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldChoice {
    /// An empty signature field already in the form, by full name.
    Existing { name: String },
    /// A new field drawn on a page: the source page index and a rectangle
    /// in display space (y-up, from the crop box's displayed bottom-left).
    #[serde(rename_all = "camelCase")]
    New { page: usize, rect: RectDto },
}

/// Fields the signature locks against later changes (FieldMDP).
#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Lock {
    #[default]
    None,
    All,
    Include {
        fields: Vec<String>,
    },
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SubFilter {
    #[default]
    #[serde(rename = "cades")]
    Cades,
    #[serde(rename = "pkcs7")]
    Pkcs7,
}

impl SubFilter {
    fn name(self) -> &'static str {
        match self {
            SubFilter::Cades => "ETSI.CAdES.detached",
            SubFilter::Pkcs7 => "adbe.pkcs7.detached",
        }
    }
}

/// The user's visual signature, drawn beside the text in the appearance.
#[derive(Deserialize, Clone, Debug)]
pub struct ArtChoice {
    pub art: SignatureArt,
    pub color: String,
}

/// Everything the signing dialog decides.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignRequest {
    pub field: FieldChoice,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub contact: String,
    /// The date as the appearance shows it, in the user's local time; the
    /// signature's own `/M` is written by the engine.
    #[serde(default)]
    pub date_text: String,
    #[serde(default)]
    pub art: Option<ArtChoice>,
    /// DocMDP permissions when certifying: 1, 2 or 3.
    #[serde(default)]
    pub certify: Option<u8>,
    #[serde(default)]
    pub lock: Lock,
    #[serde(default)]
    pub sub_filter: SubFilter,
    /// An RFC 3161 time-stamping server. Only present when the user turned
    /// on the online timestamp in the dialog.
    #[serde(default)]
    pub tsa_url: Option<String>,
}

/// The file with its placeholder, ready to hash.
pub struct Prepared {
    pub bytes: Vec<u8>,
    /// `[start, len, start, len]`, as written into `/ByteRange`.
    pub byte_range: [usize; 4],
    /// Bytes available for the DER (half the hex placeholder).
    pub capacity: usize,
    /// Where the new update starts: everything before is the source file.
    pub prev_len: usize,
}

impl Prepared {
    /// SHA-256 of the two signed ranges.
    pub fn digest(&self) -> [u8; 32] {
        let [a, alen, b, blen] = self.byte_range;
        let mut hasher = Sha256::new();
        hasher.update(&self.bytes[a..a + alen]);
        hasher.update(&self.bytes[b..b + blen]);
        hasher.finalize().into()
    }
}

/// Placeholder size: the CMS is a few hundred bytes plus the certificates;
/// a timestamp token brings its own certificates.
fn placeholder_capacity(chain_bytes: usize, timestamp: bool) -> usize {
    let base = 8192 + 2 * chain_bytes;
    let extra = if timestamp { 16384 } else { 0 };
    (base + extra).div_ceil(1024) * 1024
}

/// Sentinel the byte range is written with before the real offsets are
/// known: ten digits each, so the patch never changes a length.
const RANGE_SENTINEL: i64 = 9_999_999_999;

fn lo(e: lopdf::Error) -> AppError {
    AppError::Malformed(format!("cannot sign this file: {e}"))
}

/// A PDF text string: PDFDocEncoding when ASCII, else UTF-16BE with a BOM.
fn text_string(s: &str) -> Object {
    if s.is_ascii() {
        return Object::string_literal(s.as_bytes().to_vec());
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in s.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, StringFormat::Hexadecimal)
}

fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

/// `D:YYYYMMDDHHmmSSZ` for a moment in UTC.
pub(crate) fn pdf_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Hinnant).
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Reading the form
// ---------------------------------------------------------------------------

/// A terminal field of the form: its object, full name and type.
#[derive(Clone, Debug)]
pub(crate) struct FieldRef {
    pub id: ObjectId,
    pub name: String,
    pub ft: Option<Vec<u8>>,
    pub signed: bool,
}

fn deref<'a>(doc: &'a lopdf::Document, obj: &'a Object) -> Option<&'a Object> {
    match obj {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn dict_of<'a>(doc: &'a lopdf::Document, obj: &'a Object) -> Option<&'a Dictionary> {
    deref(doc, obj).and_then(|o| o.as_dict().ok())
}

/// The AcroForm dictionary, if the document has one.
fn acroform<'a>(doc: &'a lopdf::Document, catalog: &'a Dictionary) -> Option<&'a Dictionary> {
    catalog.get(b"AcroForm").ok().and_then(|o| dict_of(doc, o))
}

/// Every terminal field, depth first, with inherited `/FT`.
pub(crate) fn terminal_fields(doc: &lopdf::Document) -> Vec<FieldRef> {
    let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return Vec::new();
    };
    let Ok(catalog) = doc.get_dictionary(root) else {
        return Vec::new();
    };
    let Some(form) = acroform(doc, catalog) else {
        return Vec::new();
    };
    let Ok(fields) = form.get(b"Fields").and_then(|f| match f {
        Object::Reference(id) => doc.get_object(*id).and_then(Object::as_array),
        other => other.as_array(),
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack: Vec<(ObjectId, String, Option<Vec<u8>>, usize)> = fields
        .iter()
        .rev()
        .filter_map(|f| f.as_reference().ok())
        .map(|id| (id, String::new(), None, 0))
        .collect();
    let mut seen = std::collections::HashSet::new();
    while let Some((id, parent_name, parent_ft, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(id) {
            continue;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        let partial = dict
            .get(b"T")
            .ok()
            .and_then(|t| deref(doc, t))
            .and_then(|t| t.as_str().ok())
            .map(decode_text);
        let name = match (&partial, parent_name.is_empty()) {
            (Some(p), true) => p.clone(),
            (Some(p), false) => format!("{parent_name}.{p}"),
            (None, _) => parent_name.clone(),
        };
        let ft = dict
            .get(b"FT")
            .ok()
            .and_then(|f| f.as_name().ok())
            .map(|n| n.to_vec())
            .or(parent_ft);
        // Kids that are fields (have /T) make this a non-terminal field;
        // kids that are only widgets do not.
        let kid_fields: Vec<ObjectId> = dict
            .get(b"Kids")
            .ok()
            .and_then(|k| deref(doc, k))
            .and_then(|k| k.as_array().ok())
            .map(|kids| {
                kids.iter()
                    .filter_map(|k| k.as_reference().ok())
                    .filter(|k| doc.get_dictionary(*k).map(|d| d.has(b"T")).unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default();
        if kid_fields.is_empty() {
            out.push(FieldRef {
                id,
                name,
                signed: dict.has(b"V") && ft.as_deref() == Some(b"Sig"),
                ft,
            });
        } else {
            for kid in kid_fields.into_iter().rev() {
                stack.push((kid, name.clone(), ft.clone(), depth + 1));
            }
        }
    }
    out
}

/// The widget annotation of a terminal field: the field itself when merged,
/// else its first kid.
fn widget_of(doc: &lopdf::Document, field: ObjectId) -> Option<ObjectId> {
    let dict = doc.get_dictionary(field).ok()?;
    if dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget") || dict.has(b"Rect") {
        return Some(field);
    }
    dict.get(b"Kids")
        .ok()
        .and_then(|k| deref(doc, k))
        .and_then(|k| k.as_array().ok())
        .and_then(|kids| kids.first())
        .and_then(|k| k.as_reference().ok())
}

/// A page attribute, inherited through the page tree.
fn inherited<'a>(doc: &'a lopdf::Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut id = page;
    for _ in 0..64 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(v) = dict.get(key) {
            return deref(doc, v);
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn rect_of(obj: &Object, doc: &lopdf::Document) -> Option<[f32; 4]> {
    let arr = deref(doc, obj)?.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    let mut v = [0f32; 4];
    for (i, o) in arr.iter().enumerate() {
        v[i] = deref(doc, o)?.as_float().ok()?;
    }
    Some([
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ])
}

/// The page's display mapping: crop box (else media box) and `/Rotate`.
fn page_space(doc: &lopdf::Document, page: ObjectId) -> (PageSpace, u16) {
    let crop = inherited(doc, page, b"CropBox")
        .and_then(|o| rect_of(o, doc))
        .or_else(|| inherited(doc, page, b"MediaBox").and_then(|o| rect_of(o, doc)))
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let rot = inherited(doc, page, b"Rotate")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360);
    let rot = match rot {
        90 => 90,
        180 => 180,
        270 => 270,
        _ => 0,
    };
    (PageSpace::from_box(crop, rot), rot)
}

/// The page a widget sits on: its `/P`, else the page whose `/Annots` has it.
fn page_of_widget(doc: &lopdf::Document, widget: ObjectId) -> Option<ObjectId> {
    let pages = doc.get_pages();
    if let Some(p) = doc
        .get_dictionary(widget)
        .ok()
        .and_then(|d| d.get(b"P").ok())
        .and_then(|p| p.as_reference().ok())
    {
        if pages.values().any(|v| *v == p) {
            return Some(p);
        }
    }
    pages.values().copied().find(|page| {
        doc.get_dictionary(*page)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .and_then(|a| deref(doc, a))
            .and_then(|a| a.as_array().ok())
            .is_some_and(|annots| annots.iter().any(|a| a.as_reference().ok() == Some(widget)))
    })
}

/// Empty signature fields a signature can go in, as (full name, source
/// page index, display rect).
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OpenSigField {
    pub name: String,
    pub page: usize,
    pub rect: RectDto,
}

pub fn open_signature_fields(bytes: &[u8]) -> AppResult<Vec<OpenSigField>> {
    let doc = lopdf::Document::load_mem(bytes).map_err(lo)?;
    let pages = doc.get_pages();
    let mut out = Vec::new();
    for f in terminal_fields(&doc) {
        if f.ft.as_deref() != Some(b"Sig") || f.signed {
            continue;
        }
        let Some(widget) = widget_of(&doc, f.id) else {
            continue;
        };
        let Some(page) = page_of_widget(&doc, widget) else {
            continue;
        };
        let Some(index) = pages.values().position(|p| *p == page) else {
            continue;
        };
        let Some(r) = doc
            .get_dictionary(widget)
            .ok()
            .and_then(|d| d.get(b"Rect").ok())
            .and_then(|r| rect_of(r, &doc))
        else {
            continue;
        };
        let (space, _) = page_space(&doc, page);
        let a = space.user_to_disp(r[0], r[1]);
        let b = space.user_to_disp(r[2], r[3]);
        out.push(OpenSigField {
            name: f.name,
            page: index,
            rect: RectDto {
                x: a.0.min(b.0),
                y: a.1.min(b.1),
                w: (a.0 - b.0).abs(),
                h: (a.1 - b.1).abs(),
            },
        });
    }
    Ok(out)
}

/// Whether the file already carries a signature (a signed field).
pub fn has_signatures(doc: &lopdf::Document) -> bool {
    terminal_fields(doc).iter().any(|f| f.signed)
}

// ---------------------------------------------------------------------------
// Writing the update
// ---------------------------------------------------------------------------

/// The signer's name as the appearance and `/Name` show it.
pub fn signer_name(signer: &dyn CmsSigner) -> String {
    signer
        .chain()
        .first()
        .and_then(|c| super::keys::describe(std::slice::from_ref(c), String::new(), "", false).ok())
        .map(|d| d.name)
        .unwrap_or_else(|| "Unknown signer".into())
}

/// Add the signature field, its appearance and the placeholder signature
/// dictionary as an incremental update to `src`.
pub fn prepare(
    src: &[u8],
    req: &SignRequest,
    name: &str,
    chain_bytes: usize,
    now_secs: i64,
) -> AppResult<Prepared> {
    let mut inc: IncrementalDocument = src.try_into().map_err(lo)?;
    let declared = inc
        .get_prev_documents()
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    // New objects are numbered past the previous trailer's /Size (lopdf does
    // not count a cross-reference stream as an object; see markup_export).
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    let prev = inc.get_prev_documents();
    if prev.is_encrypted() || prev.encryption_state.is_some() || prev.trailer.has(b"Encrypt") {
        return Err(AppError::Unsupported(
            "save this document without its password protection before signing it".into(),
        ));
    }

    let root = prev
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(lo)?;
    let fields = terminal_fields(prev);
    let already_signed = fields.iter().any(|f| f.signed);
    if req.certify.is_some() && already_signed {
        return Err(AppError::Unsupported(
            "a document that is already signed cannot be certified; sign it instead".into(),
        ));
    }
    if let Some(p) = req.certify {
        if !(1..=3).contains(&p) {
            return Err(AppError::Unsupported("unknown certification level".into()));
        }
    }
    if prev
        .get_dictionary(root)
        .ok()
        .and_then(|c| c.get(b"Perms").ok())
        .and_then(|p| dict_of(prev, p))
        .is_some_and(|p| p.has(b"DocMDP"))
    {
        // A certified document says itself which changes it allows; a new
        // signature field is a change P=1 forbids and the others may not.
        if req.certify.is_some() {
            return Err(AppError::Unsupported(
                "this document is already certified".into(),
            ));
        }
    }

    // Where the widget is, and which objects carry the field and widget.
    let (field_id, widget_id, page_id, user_rect, new_field) = match &req.field {
        FieldChoice::Existing { name: wanted } => {
            let field = fields.iter().find(|f| &f.name == wanted).ok_or_else(|| {
                AppError::Unsupported(format!("there is no field named {wanted}"))
            })?;
            if field.ft.as_deref() != Some(b"Sig") {
                return Err(AppError::Unsupported(format!(
                    "{wanted} is not a signature field"
                )));
            }
            if field.signed {
                return Err(AppError::Unsupported(format!("{wanted} is already signed")));
            }
            let widget = widget_of(prev, field.id)
                .ok_or_else(|| AppError::Malformed(format!("{wanted} has no widget")))?;
            let page = page_of_widget(prev, widget)
                .ok_or_else(|| AppError::Malformed(format!("{wanted} is not on a page")))?;
            let rect = prev
                .get_dictionary(widget)
                .ok()
                .and_then(|d| d.get(b"Rect").ok())
                .and_then(|r| rect_of(r, prev))
                .ok_or_else(|| AppError::Malformed(format!("{wanted} has no rectangle")))?;
            (Some(field.id), Some(widget), page, rect, None)
        }
        FieldChoice::New { page, rect } => {
            let page_id = *prev.get_pages().get(&(*page as u32 + 1)).ok_or_else(|| {
                AppError::Unsupported(format!("page {} does not exist", page + 1))
            })?;
            if rect.w < 4.0 || rect.h < 4.0 {
                return Err(AppError::Unsupported(
                    "the signature box is too small".into(),
                ));
            }
            let (space, _) = page_space(prev, page_id);
            let a = space.disp_to_user(rect.x, rect.y);
            let b = space.disp_to_user(rect.x + rect.w, rect.y + rect.h);
            let user = [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)];
            let mut n = 1;
            let taken: std::collections::HashSet<&str> =
                fields.iter().map(|f| f.name.as_str()).collect();
            while taken.contains(format!("Signature{n}").as_str()) {
                n += 1;
            }
            (None, None, page_id, user, Some(format!("Signature{n}")))
        }
    };
    let (_, rot) = page_space(prev, page_id);
    let has_acroform = prev
        .get_dictionary(root)
        .ok()
        .and_then(|c| acroform(prev, c))
        .is_some();

    // The appearance.
    let (rw, rh) = (user_rect[2] - user_rect[0], user_rect[3] - user_rect[1]);
    let (dw, dh) = if rot == 90 || rot == 270 {
        (rh, rw)
    } else {
        (rw, rh)
    };
    let mut lines = vec![format!("Digitally signed by {name}")];
    if !req.date_text.trim().is_empty() {
        lines.push(format!("Date: {}", req.date_text.trim()));
    }
    if !req.reason.trim().is_empty() {
        lines.push(format!("Reason: {}", req.reason.trim()));
    }
    if !req.location.trim().is_empty() {
        lines.push(format!("Location: {}", req.location.trim()));
    }
    let ap_id = appearance::write(&mut inc.new_document, dw, dh, rot, &lines, req.art.as_ref())?;

    // The signature dictionary, with its placeholders.
    let capacity = placeholder_capacity(chain_bytes, req.tsa_url.is_some());
    let mut sig = dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => Object::Name(req.sub_filter.name().as_bytes().to_vec()),
        "ByteRange" => vec![
            Object::Integer(0),
            Object::Integer(RANGE_SENTINEL),
            Object::Integer(RANGE_SENTINEL),
            Object::Integer(RANGE_SENTINEL),
        ],
        "Contents" => Object::String(vec![0u8; capacity], StringFormat::Hexadecimal),
        "M" => Object::string_literal(pdf_date(now_secs)),
        "Name" => text_string(name),
        "Prop_Build" => dictionary! {
            "App" => dictionary! { "Name" => "SpeedyF" },
        },
    };
    if !req.reason.trim().is_empty() {
        sig.set("Reason", text_string(req.reason.trim()));
    }
    if !req.location.trim().is_empty() {
        sig.set("Location", text_string(req.location.trim()));
    }
    if !req.contact.trim().is_empty() {
        sig.set("ContactInfo", text_string(req.contact.trim()));
    }
    let mut references: Vec<Object> = Vec::new();
    if let Some(p) = req.certify {
        references.push(Object::Dictionary(dictionary! {
            "Type" => "SigRef",
            "TransformMethod" => "DocMDP",
            "TransformParams" => dictionary! {
                "Type" => "TransformParams",
                "P" => p as i64,
                "V" => "1.2",
            },
        }));
    }
    let lock_dict = match &req.lock {
        Lock::None => None,
        Lock::All => Some(dictionary! { "Type" => "SigFieldLock", "Action" => "All" }),
        Lock::Include { fields: names } => {
            let names: Vec<Object> = names.iter().map(|n| text_string(n)).collect();
            Some(dictionary! {
                "Type" => "SigFieldLock",
                "Action" => "Include",
                "Fields" => names,
            })
        }
    };
    if let Some(lock) = &lock_dict {
        let mut params = dictionary! {
            "Type" => "TransformParams",
            "Action" => lock.get(b"Action").cloned().unwrap_or(Object::Name(b"All".to_vec())),
            "V" => "1.2",
        };
        if let Ok(f) = lock.get(b"Fields") {
            params.set("Fields", f.clone());
        }
        references.push(Object::Dictionary(dictionary! {
            "Type" => "SigRef",
            "TransformMethod" => "FieldMDP",
            "TransformParams" => params,
        }));
    }
    if !references.is_empty() {
        sig.set("Reference", Object::Array(references));
    }
    let sig_id = inc.new_document.add_object(sig);

    // The field and its widget.
    let widget = match (field_id, widget_id) {
        (Some(field), Some(widget)) => {
            inc.opt_clone_object_to_new_document(field).map_err(lo)?;
            let f = inc.new_document.get_dictionary_mut(field).map_err(lo)?;
            f.set("V", Object::Reference(sig_id));
            if let Some(lock) = &lock_dict {
                f.set("Lock", Object::Dictionary(lock.clone()));
            }
            if widget != field {
                inc.opt_clone_object_to_new_document(widget).map_err(lo)?;
            }
            let w = inc.new_document.get_dictionary_mut(widget).map_err(lo)?;
            w.set("AP", dictionary! { "N" => Object::Reference(ap_id) });
            // Printed, and locked so a reader does not let it be moved.
            let flags = w.get(b"F").and_then(Object::as_i64).unwrap_or(0);
            w.set("F", (flags | 4 | 128) & !2);
            widget
        }
        _ => {
            let mut w = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Widget",
                "FT" => "Sig",
                "T" => text_string(new_field.as_deref().unwrap_or("Signature1")),
                "F" => 132,
                "Rect" => user_rect.iter().map(|&v| Object::Real(v)).collect::<Vec<_>>(),
                "P" => Object::Reference(page_id),
                "V" => Object::Reference(sig_id),
                "AP" => dictionary! { "N" => Object::Reference(ap_id) },
            };
            if let Some(lock) = &lock_dict {
                w.set("Lock", Object::Dictionary(lock.clone()));
            }
            let id = inc.new_document.add_object(w);
            add_to_page(&mut inc, page_id, id)?;
            id
        }
    };

    // The AcroForm: the new field listed, SignaturesExist | AppendOnly.
    let new_field_id = if field_id.is_none() {
        Some(widget)
    } else {
        None
    };
    if has_acroform {
        let catalog_form = inc
            .get_prev_documents()
            .get_dictionary(root)
            .map_err(lo)?
            .get(b"AcroForm")
            .map_err(lo)?
            .clone();
        let (owner, inline) = match catalog_form {
            Object::Reference(id) => (id, false),
            _ => (root, true),
        };
        inc.opt_clone_object_to_new_document(owner).map_err(lo)?;
        // A /Fields array held as its own object is resolved and inlined.
        let fields_obj = {
            let doc = inc.get_prev_documents();
            let form = if inline {
                doc.get_dictionary(root)
                    .ok()
                    .and_then(|c| c.get(b"AcroForm").ok())
                    .and_then(|f| f.as_dict().ok())
            } else {
                doc.get_dictionary(owner).ok()
            };
            form.and_then(|f| f.get(b"Fields").ok())
                .and_then(|f| deref(doc, f))
                .and_then(|f| f.as_array().ok())
                .cloned()
                .unwrap_or_default()
        };
        let dict = inc.new_document.get_dictionary_mut(owner).map_err(lo)?;
        let form = if inline {
            dict.get_mut(b"AcroForm")
                .and_then(Object::as_dict_mut)
                .map_err(lo)?
        } else {
            dict
        };
        let mut fields_arr = fields_obj;
        if let Some(id) = new_field_id {
            fields_arr.push(Object::Reference(id));
        }
        form.set("Fields", Object::Array(fields_arr));
        let flags = form.get(b"SigFlags").and_then(Object::as_i64).unwrap_or(0);
        form.set("SigFlags", flags | 3);
    } else {
        let form_id = inc.new_document.add_object(dictionary! {
            "Fields" => vec![Object::Reference(widget)],
            "SigFlags" => 3,
        });
        inc.opt_clone_object_to_new_document(root).map_err(lo)?;
        inc.new_document
            .get_dictionary_mut(root)
            .map_err(lo)?
            .set("AcroForm", Object::Reference(form_id));
    }

    // Certify: the catalog points at this signature as the DocMDP one.
    if req.certify.is_some() {
        inc.opt_clone_object_to_new_document(root).map_err(lo)?;
        let catalog = inc.new_document.get_dictionary_mut(root).map_err(lo)?;
        catalog.set(
            "Perms",
            dictionary! { "DocMDP" => Object::Reference(sig_id) },
        );
    }

    // Locked fields become read-only in this same, signed revision.
    if let Some(lock) = &lock_dict {
        let include: Option<Vec<String>> = match &req.lock {
            Lock::Include { fields } => Some(fields.clone()),
            _ => None,
        };
        let _ = lock;
        for f in &fields {
            if f.ft.as_deref() == Some(b"Sig") {
                continue;
            }
            if let Some(names) = &include {
                if !names.iter().any(|n| n == &f.name) {
                    continue;
                }
            }
            inc.opt_clone_object_to_new_document(f.id).map_err(lo)?;
            let d = inc.new_document.get_dictionary_mut(f.id).map_err(lo)?;
            let ff = d.get(b"Ff").and_then(Object::as_i64).unwrap_or(0);
            d.set("Ff", ff | 1);
        }
    }

    let prev_len = src.len();
    let mut out = Vec::with_capacity(prev_len + capacity * 2 + 16 * 1024);
    inc.save_to(&mut out)
        .map_err(|e| AppError::Internal(format!("cannot write the signature: {e}")))?;
    let mut prepared = locate_placeholders(out, prev_len, capacity)?;
    prepared.prev_len = prev_len;
    Ok(prepared)
}

/// Add an annotation to a page's `/Annots`, whether that is inline, its own
/// object, or missing.
fn add_to_page(inc: &mut IncrementalDocument, page: ObjectId, annot: ObjectId) -> AppResult<()> {
    inc.opt_clone_object_to_new_document(page).map_err(lo)?;
    let existing = inc
        .new_document
        .get_dictionary(page)
        .map_err(lo)?
        .get(b"Annots")
        .ok()
        .cloned();
    match existing {
        Some(Object::Reference(array_id)) => {
            inc.opt_clone_object_to_new_document(array_id).map_err(lo)?;
            inc.new_document
                .get_object_mut(array_id)
                .and_then(Object::as_array_mut)
                .map_err(lo)?
                .push(Object::Reference(annot));
        }
        Some(Object::Array(mut arr)) => {
            arr.push(Object::Reference(annot));
            inc.new_document
                .get_dictionary_mut(page)
                .map_err(lo)?
                .set("Annots", Object::Array(arr));
        }
        _ => {
            inc.new_document
                .get_dictionary_mut(page)
                .map_err(lo)?
                .set("Annots", vec![Object::Reference(annot)]);
        }
    }
    Ok(())
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Find the placeholders lopdf wrote and fill in the real byte range.
fn locate_placeholders(
    mut bytes: Vec<u8>,
    prev_len: usize,
    capacity: usize,
) -> AppResult<Prepared> {
    let mut hex = Vec::with_capacity(capacity * 2 + 2);
    hex.push(b'<');
    hex.resize(capacity * 2 + 1, b'0');
    hex.push(b'>');
    let start = find(&bytes, &hex, prev_len)
        .ok_or_else(|| AppError::Internal("signature placeholder not found".into()))?;
    let end = start + hex.len();
    if find(&bytes, &hex, end).is_some() {
        return Err(AppError::Internal(
            "signature placeholder is not unique".into(),
        ));
    }
    let br = find(&bytes, b"/ByteRange", prev_len)
        .ok_or_else(|| AppError::Internal("byte range placeholder not found".into()))?;
    let open = find(&bytes, b"[", br)
        .ok_or_else(|| AppError::Internal("byte range placeholder not found".into()))?;
    let close = find(&bytes, b"]", open)
        .ok_or_else(|| AppError::Internal("byte range placeholder not found".into()))?;
    let sentinel = RANGE_SENTINEL.to_string();
    let inside = &bytes[open..=close];
    if inside
        .windows(sentinel.len())
        .filter(|w| *w == sentinel.as_bytes())
        .count()
        != 3
    {
        return Err(AppError::Internal(
            "byte range placeholder is malformed".into(),
        ));
    }
    let range = [0, start, end, bytes.len() - end];
    let mut text = format!("[0 {} {} {}", range[1], range[2], range[3]).into_bytes();
    let width = close - open;
    if text.len() > width {
        return Err(AppError::Internal("byte range does not fit".into()));
    }
    text.resize(width, b' ');
    text.push(b']');
    bytes[open..=close].copy_from_slice(&text);
    Ok(Prepared {
        bytes,
        byte_range: [range[0], range[1], range[2], range[3]],
        capacity,
        prev_len,
    })
}

/// Write the CMS into the placeholder. Nothing moves.
pub fn embed(prepared: &mut Prepared, cms_der: &[u8]) -> AppResult<()> {
    if cms_der.len() > prepared.capacity {
        return Err(AppError::Internal(format!(
            "the signature ({} bytes) is larger than the space reserved for it ({})",
            cms_der.len(),
            prepared.capacity
        )));
    }
    let at = prepared.byte_range[1] + 1;
    for (i, b) in cms_der.iter().enumerate() {
        let hexed = format!("{b:02X}");
        prepared.bytes[at + 2 * i..at + 2 * i + 2].copy_from_slice(hexed.as_bytes());
    }
    Ok(())
}

/// Sign `src` start to finish without a timestamp server: the path the tests
/// take, and the one the command takes when the online timestamp is off.
pub fn sign_bytes(
    src: &[u8],
    req: &SignRequest,
    signer: &dyn CmsSigner,
    now_secs: i64,
) -> AppResult<Vec<u8>> {
    let chain_bytes: usize = signer.chain().iter().map(Vec::len).sum();
    let name = signer_name(signer);
    let mut prepared = prepare(src, req, &name, chain_bytes, now_secs)?;
    let digest = prepared.digest();
    let parts = cms::build(signer, &digest, req.sub_filter, now_secs)?;
    let der = parts.to_der()?;
    embed(&mut prepared, &der)?;
    check(&prepared.bytes)?;
    Ok(prepared.bytes)
}

/// Read back the last signature in `bytes` and check it: the byte range
/// covers the file but the contents, the digest matches, and the signature
/// verifies under the signer's certificate. Run before anything is written.
pub fn check(bytes: &[u8]) -> AppResult<()> {
    let (range, contents) = last_signature(bytes)?;
    let [a, alen, b, blen] = range;
    if a != 0 || b + blen != bytes.len() || a + alen >= b {
        return Err(AppError::Internal(
            "the byte range does not cover the file".into(),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(&bytes[a..a + alen]);
    hasher.update(&bytes[b..b + blen]);
    let digest: [u8; 32] = hasher.finalize().into();
    cms::verify_detached(&contents, &digest)
}

/// The `/ByteRange` and decoded `/Contents` of the last signature dictionary
/// in the file, found textually (it is the one this module wrote last).
pub(crate) fn last_signature(bytes: &[u8]) -> AppResult<([usize; 4], Vec<u8>)> {
    let missing = || AppError::Internal("no signature found".into());
    let br = bytes
        .windows(10)
        .rposition(|w| w == b"/ByteRange")
        .ok_or_else(missing)?;
    let open = find(bytes, b"[", br).ok_or_else(missing)?;
    let close = find(bytes, b"]", open).ok_or_else(missing)?;
    let nums: Vec<usize> = std::str::from_utf8(&bytes[open + 1..close])
        .map_err(|_| missing())?
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    let range: [usize; 4] = nums.try_into().map_err(|_| missing())?;
    if range[1] >= bytes.len() || range[2] > bytes.len() || range[1] >= range[2] {
        return Err(missing());
    }
    let hex = &bytes[range[1]..range[2]];
    if hex.first() != Some(&b'<') || hex.last() != Some(&b'>') {
        return Err(AppError::Internal(
            "the contents are not a hex string".into(),
        ));
    }
    let digits: Vec<u8> = hex[1..hex.len() - 1]
        .iter()
        .copied()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let mut der = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks(2) {
        let s = std::str::from_utf8(pair).map_err(|_| missing())?;
        der.push(u8::from_str_radix(s, 16).map_err(|_| missing())?);
    }
    // Trailing zero padding is not part of the DER; the DER says how long it is.
    let len =
        cms::der_len(&der).ok_or_else(|| AppError::Internal("the contents are not DER".into()))?;
    der.truncate(len);
    Ok((range, der))
}

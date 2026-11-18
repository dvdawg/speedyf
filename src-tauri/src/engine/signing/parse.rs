//! Signature dictionaries as they sit in a PDF's bytes.
//!
//! A signed PDF signs bytes, not objects: the signature dictionary's
//! `/ByteRange [a b c d]` names two spans of the file, `[a, a+b)` and
//! `[c, c+d)`, and the gap between them is exactly the hex string of
//! `/Contents` that holds the CMS blob. Everything after `c+d` is a later
//! incremental update the signature does not cover.
//!
//! This module only reads: where each signature is, what its dictionary says,
//! which field and widget show it, and where every revision of the file ends.
//! Checking the cryptography is `verify.rs`; comparing revisions is
//! `changes.rs`.
//!
//! The object graph comes from lopdf. A file lopdf cannot load (a damaged one,
//! or one encrypted with a user password) still has its signatures found by a
//! plain byte scan, so their integrity can be checked even when nothing else
//! about them can be read.

use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What kind of signature a dictionary holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureKind {
    /// An ordinary ("approval") signature.
    Approval,
    /// The certification signature (the catalog's `/Perms /DocMDP`): it says
    /// which later changes the author allows.
    Certification,
    /// A document timestamp (`/Type /DocTimeStamp`, `ETSI.RFC3161`): an RFC
    /// 3161 token over the byte range rather than anyone's signature.
    DocTimestamp,
}

/// How a FieldMDP transform (or a signature field's `/Lock`) locks fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldLock {
    /// Every field in the document.
    All,
    /// Only the named fields.
    Include(Vec<String>),
    /// Every field except the named ones.
    Exclude(Vec<String>),
}

impl FieldLock {
    /// Whether this lock covers the field with the given fully qualified name.
    pub fn covers(&self, field: &str) -> bool {
        let named = |list: &[String]| {
            list.iter().any(|name| {
                // A lock on a parent covers its descendants too.
                field == name || field.starts_with(&format!("{name}."))
            })
        };
        match self {
            FieldLock::All => true,
            FieldLock::Include(list) => named(list),
            FieldLock::Exclude(list) => !named(list),
        }
    }
}

/// One signature value found in the file.
#[derive(Clone, Debug)]
pub struct SignatureInfo {
    /// The signature dictionary's object, when it is an indirect object.
    pub object: Option<ObjectId>,
    /// The signature field's fully qualified name, when a field holds it.
    pub field_name: Option<String>,
    /// Zero-based page index of the field's widget, in the final revision.
    pub page: Option<u32>,
    /// The widget's `/Rect` in PDF user space, `[x1 y1 x2 y2]`.
    pub rect: Option<[f32; 4]>,
    pub filter: Option<String>,
    pub sub_filter: Option<String>,
    pub kind: SignatureKind,
    /// The DocMDP permission level (1, 2 or 3) of a certification signature.
    pub doc_mdp: Option<u8>,
    /// Fields this signature locks (FieldMDP transform, or the field's
    /// `/Lock` dictionary).
    pub field_lock: Option<FieldLock>,
    /// `/ByteRange` as written: `[a, b, c, d]` when well formed.
    pub byte_range: Vec<i64>,
    /// The bytes of `/Contents`, decoded from the hex in the file's own gap,
    /// zero padding included.
    pub contents: Vec<u8>,
    /// `/Name`, `/Reason`, `/Location`, `/ContactInfo` as the signer wrote them.
    pub name: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact_info: Option<String>,
    /// `/M`, the claimed signing time, as a raw PDF date string.
    pub m: Option<String>,
    /// `/Cert` (only `adbe.x509.rsa_sha1` uses it): DER certificates.
    pub cert: Vec<Vec<u8>>,
    /// False when the byte range's gap holds some other value than this
    /// dictionary's own `/Contents` (a range copied from another signature).
    pub gap_is_own: bool,
}

impl SignatureInfo {
    /// The end of the revision this signature signs: `c + d`.
    pub fn signed_end(&self) -> Option<usize> {
        match self.byte_range[..] {
            [_, _, c, d] if c >= 0 && d >= 0 => usize::try_from(c.checked_add(d)?).ok(),
            _ => None,
        }
    }
}

/// How a byte range sits in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coverage {
    /// Where the signed revision ends.
    pub revision_end: usize,
    /// True when nothing follows the signed revision.
    pub whole_file: bool,
}

/// Every signature in `bytes`, including any that a later revision dropped
/// from the object graph, in file order of their signed revisions.
pub fn signatures_in(bytes: &[u8]) -> Vec<SignatureInfo> {
    if !has_signature_bytes(bytes) {
        return Vec::new();
    }
    signatures_with(bytes, load(bytes).as_ref())
}

/// `signatures_in` for a caller that already loaded the final revision (or
/// tried to: `None` when lopdf could not).
pub fn signatures_with(bytes: &[u8], doc: Option<&Document>) -> Vec<SignatureInfo> {
    let raw = raw_byte_ranges(bytes);
    let mut found = match doc {
        Some(doc) => from_document(doc, bytes),
        None => Vec::new(),
    };
    // A signature dictionary a later revision replaced or orphaned is not in
    // the final object graph. Look for it in the revision that wrote it.
    let known =
        |found: &[SignatureInfo], range: &[i64]| found.iter().any(|s| s.byte_range == range);
    if raw.iter().any(|(_, range)| !known(&found, range)) {
        for end in revision_ends(bytes) {
            if end >= bytes.len() {
                break;
            }
            let Some(doc) = load(&bytes[..end]) else {
                continue;
            };
            for sig in from_document(&doc, &bytes[..end]) {
                if !known(&found, &sig.byte_range) {
                    found.push(sig);
                }
            }
        }
    }
    // Whatever no revision could explain is still checked for integrity.
    for (at, range) in raw {
        if !known(&found, &range) {
            if let Some(sig) = from_raw(bytes, at, range) {
                found.push(sig);
            }
        }
    }
    found.sort_by_key(|s| s.signed_end().unwrap_or(usize::MAX));
    found
}

/// A cheap test for whether `bytes` can hold a signature value at all: every
/// one carries a `/ByteRange` in plain bytes, since the range has to point at
/// its own `/Contents` in the file.
pub fn has_signature_bytes(bytes: &[u8]) -> bool {
    memchr::memmem::find(bytes, b"/ByteRange").is_some()
}

/// Load `bytes` with lopdf, leniently.
pub fn load(bytes: &[u8]) -> Option<Document> {
    Document::load_mem(bytes).ok()
}

/// The two signed spans of a byte range, when it is well formed.
pub fn signed_spans<'a>(bytes: &'a [u8], range: &[i64]) -> Option<[&'a [u8]; 2]> {
    let [a, b, c, d] = <[i64; 4]>::try_from(range).ok()?;
    let to = |v: i64| usize::try_from(v).ok();
    let (a, b, c, d) = (to(a)?, to(b)?, to(c)?, to(d)?);
    let first = bytes.get(a..a.checked_add(b)?)?;
    let second = bytes.get(c..c.checked_add(d)?)?;
    Some([first, second])
}

/// Check that `range` is the shape every conforming signature has - the
/// whole file from its first byte up to the end of a revision, minus exactly
/// the `/Contents` hex string - and say where that revision ends.
pub fn check_coverage(bytes: &[u8], range: &[i64]) -> Result<Coverage, String> {
    let [a, b, c, d] = <[i64; 4]>::try_from(range)
        .map_err(|_| format!("the byte range has {} numbers, not 4", range.len()))?;
    if a != 0 {
        return Err("the signed bytes do not start at the beginning of the file".into());
    }
    if b < 0 || c < b || d < 0 {
        return Err("the byte range is not in order".into());
    }
    let (b, c, d) = (b as usize, c as usize, d as usize);
    let end = c.checked_add(d).ok_or("the byte range overflows")?;
    if end > bytes.len() {
        return Err("the byte range runs past the end of the file".into());
    }
    // The gap must be exactly one hex string: `<`, hex digits, `>`.
    let gap = &bytes[b..c];
    let hex_ok = gap.len() >= 2
        && gap[0] == b'<'
        && gap[gap.len() - 1] == b'>'
        && gap[1..gap.len() - 1]
            .iter()
            .all(|ch| ch.is_ascii_hexdigit() || ch.is_ascii_whitespace());
    if !hex_ok {
        return Err("the unsigned gap is not just the signature value".into());
    }
    if !ends_a_revision(bytes, end) {
        return Err("the signed bytes stop part-way through a revision".into());
    }
    Ok(Coverage {
        revision_end: end,
        whole_file: bytes[end..]
            .iter()
            .all(|ch| ch.is_ascii_whitespace() || *ch == 0),
    })
}

/// Decode the hex string in a signature's gap (the bytes between `<` and `>`).
pub fn contents_from_gap(bytes: &[u8], range: &[i64]) -> Option<Vec<u8>> {
    let [_, b, c, _] = <[i64; 4]>::try_from(range).ok()?;
    let gap = bytes.get(usize::try_from(b).ok()?..usize::try_from(c).ok()?)?;
    let inner = gap.strip_prefix(b"<")?.strip_suffix(b">")?;
    decode_hex(inner)
}

/// `bytes` without its trailing zero padding.
fn trim_zeros(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    &bytes[..end]
}

fn decode_hex(hex: &[u8]) -> Option<Vec<u8>> {
    let digits: Vec<u8> = hex
        .iter()
        .filter(|ch| !ch.is_ascii_whitespace())
        .map(|&ch| match ch {
            b'0'..=b'9' => Some(ch - b'0'),
            b'a'..=b'f' => Some(ch - b'a' + 10),
            b'A'..=b'F' => Some(ch - b'A' + 10),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let mut out = Vec::with_capacity(digits.len().div_ceil(2));
    for pair in digits.chunks(2) {
        let hi = pair[0];
        let lo = pair.get(1).copied().unwrap_or(0);
        out.push(hi << 4 | lo);
    }
    Some(out)
}

/// Byte offsets just past every `%%EOF` marker (and the end-of-line after
/// it), ascending: where each revision of the file ends.
pub fn revision_ends(bytes: &[u8]) -> Vec<usize> {
    let mut ends = Vec::new();
    for at in memchr::memmem::find_iter(bytes, b"%%EOF") {
        let mut end = at + 5;
        if bytes.get(end) == Some(&b'\r') {
            end += 1;
        }
        if bytes.get(end) == Some(&b'\n') {
            end += 1;
        }
        ends.push(end);
    }
    ends
}

/// Whether `end` is where a revision ends: just after a `%%EOF`, with or
/// without the end-of-line that follows it.
fn ends_a_revision(bytes: &[u8], end: usize) -> bool {
    let Some(head) = bytes.get(..end) else {
        return false;
    };
    let trimmed = head
        .strip_suffix(b"\r\n")
        .or_else(|| head.strip_suffix(b"\n"))
        .or_else(|| head.strip_suffix(b"\r"))
        .unwrap_or(head);
    trimmed.ends_with(b"%%EOF")
}

/// Every `/ByteRange [a b c d]` written in the file: where it is and what it
/// says.
fn raw_byte_ranges(bytes: &[u8]) -> Vec<(usize, Vec<i64>)> {
    let mut out = Vec::new();
    for at in memchr::memmem::find_iter(bytes, b"/ByteRange") {
        let rest = &bytes[at + 10..bytes.len().min(at + 200)];
        let Some(open) = rest.iter().position(|&ch| ch == b'[') else {
            continue;
        };
        if !rest[..open].iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Some(close) = rest[open..].iter().position(|&ch| ch == b']') else {
            continue;
        };
        let numbers: Option<Vec<i64>> = std::str::from_utf8(&rest[open + 1..open + close])
            .ok()
            .map(|text| {
                text.split_ascii_whitespace()
                    .map(|n| n.parse().ok())
                    .collect()
            })
            .and_then(|v: Vec<Option<i64>>| v.into_iter().collect());
        if let Some(numbers) = numbers {
            // A placeholder that was never filled in is not a signature.
            if numbers.len() == 4 && numbers.iter().any(|&n| n != 0) {
                out.push((at, numbers));
            }
        }
    }
    out
}

/// A signature known only from its bytes: the range, the contents in its gap
/// and the filter names written next to it.
fn from_raw(bytes: &[u8], at: usize, range: Vec<i64>) -> Option<SignatureInfo> {
    let contents = contents_from_gap(bytes, &range)?;
    // The dictionary is somewhere between the `obj` before and the `endobj`
    // after; good enough for two names.
    let start = memchr::memmem::rfind(&bytes[..at], b" obj").unwrap_or(at.saturating_sub(2048));
    let stop = memchr::memmem::find(&bytes[at..], b"endobj")
        .map(|o| at + o)
        .unwrap_or(bytes.len().min(at + 2048));
    let skip = |v: i64| usize::try_from(v).unwrap_or(0);
    // The hex itself is excluded: it can be tens of kilobytes of noise.
    let window: Vec<u8> = bytes[start..stop]
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let pos = start + i;
            !(pos >= skip(range[1]) && pos < skip(range[2]))
        })
        .map(|(_, &ch)| ch)
        .collect();
    let name_after = |key: &[u8]| -> Option<String> {
        let at = memchr::memmem::find(&window, key)? + key.len();
        let rest = &window[at..];
        let slash = rest.iter().position(|&ch| ch == b'/')?;
        if !rest[..slash].iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        let name: Vec<u8> = rest[slash + 1..]
            .iter()
            .take_while(|ch| !ch.is_ascii_whitespace() && !b"/[]<>()".contains(ch))
            .copied()
            .collect();
        String::from_utf8(name).ok()
    };
    let sub_filter = name_after(b"/SubFilter");
    let kind = if sub_filter.as_deref() == Some("ETSI.RFC3161") {
        SignatureKind::DocTimestamp
    } else {
        SignatureKind::Approval
    };
    Some(SignatureInfo {
        object: None,
        field_name: None,
        page: None,
        rect: None,
        filter: name_after(b"/Filter"),
        sub_filter,
        kind,
        doc_mdp: None,
        field_lock: None,
        byte_range: range,
        contents,
        name: None,
        reason: None,
        location: None,
        contact_info: None,
        m: None,
        cert: Vec::new(),
        gap_is_own: true,
    })
}

/// A field in the AcroForm tree, as far as signatures care.
struct FieldNode {
    name: String,
    /// The field's `/V` when it names a signature dictionary.
    value: Option<SigValue>,
    /// Widgets: the field itself when it is merged with its widget, or its
    /// kids that are widgets.
    widgets: Vec<ObjectId>,
    lock: Option<FieldLock>,
}

enum SigValue {
    Ref(ObjectId),
    Direct(Box<Dictionary>),
}

/// The signatures `doc` (loaded from `bytes`) holds in its final revision.
fn from_document(doc: &Document, bytes: &[u8]) -> Vec<SignatureInfo> {
    let fields = signature_fields(doc);
    let pages = annotation_pages(doc);
    let certification = doc
        .catalog()
        .ok()
        .and_then(|catalog| resolve(doc, catalog.get(b"Perms").ok()?))
        .and_then(|perms| match perms.as_dict().ok()?.get(b"DocMDP").ok()? {
            Object::Reference(id) => Some(*id),
            _ => None,
        });

    let mut out = Vec::new();
    let mut seen: HashSet<ObjectId> = HashSet::new();
    // Signatures held by fields first: they know their field and widget.
    for field in &fields {
        let (id, dict) = match &field.value {
            Some(SigValue::Ref(id)) => match doc.get_object(*id) {
                Ok(Object::Dictionary(dict)) => (Some(*id), dict),
                _ => continue,
            },
            Some(SigValue::Direct(dict)) => (None, dict.as_ref()),
            None => continue,
        };
        if let Some(id) = id {
            if !seen.insert(id) {
                continue;
            }
        }
        let Some(mut sig) = from_dictionary(doc, bytes, id, dict, certification) else {
            continue;
        };
        sig.field_name = Some(field.name.clone());
        if sig.field_lock.is_none() {
            sig.field_lock = field.lock.clone();
        }
        if let Some(&widget) = field.widgets.first() {
            sig.page = pages.get(&widget).copied().or_else(|| {
                let p = doc.get_dictionary(widget).ok()?.get(b"P").ok()?;
                let page = p.as_reference().ok()?;
                doc.get_pages()
                    .iter()
                    .find(|(_, id)| **id == page)
                    .map(|(n, _)| n - 1)
            });
            sig.rect = doc
                .get_dictionary(widget)
                .ok()
                .and_then(|w| rect_of(doc, w.get(b"Rect").ok()?));
        }
        out.push(sig);
    }
    // Then any signature dictionary no field points at.
    for (id, object) in &doc.objects {
        if seen.contains(id) {
            continue;
        }
        if let Object::Dictionary(dict) = object {
            if dict.has(b"ByteRange") && dict.has(b"Contents") {
                if let Some(sig) = from_dictionary(doc, bytes, Some(*id), dict, certification) {
                    out.push(sig);
                }
            }
        }
    }
    out
}

fn from_dictionary(
    doc: &Document,
    bytes: &[u8],
    id: Option<ObjectId>,
    dict: &Dictionary,
    certification: Option<ObjectId>,
) -> Option<SignatureInfo> {
    let byte_range: Vec<i64> = resolve(doc, dict.get(b"ByteRange").ok()?)?
        .as_array()
        .ok()?
        .iter()
        .map(|o| number(doc, o).map(|n| n as i64))
        .collect::<Option<_>>()?;
    if byte_range.iter().all(|&n| n == 0) {
        return None;
    }
    // The gap in the file is authoritative: it is what the range excludes,
    // and lopdf may have run a decryption filter over the parsed string.
    let own = resolve(doc, dict.get(b"Contents").ok()?)?
        .as_str()
        .ok()
        .map(<[u8]>::to_vec);
    let gap = contents_from_gap(bytes, &byte_range);
    // A range has to exclude this dictionary's own value. One copied from
    // another signature points at that signature's gap instead. (In an
    // encrypted file the parsed string may have been decrypted, so the two
    // cannot be compared there.)
    let encrypted = doc.trailer.has(b"Encrypt");
    let gap_is_own = encrypted
        || match (&own, &gap) {
            (Some(own), Some(gap)) => trim_zeros(own) == trim_zeros(gap),
            _ => true,
        };
    let contents = gap.or(own)?;
    let name_of = |key: &[u8]| -> Option<String> {
        match resolve(doc, dict.get(key).ok()?)? {
            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
            _ => None,
        }
    };
    let text_of = |key: &[u8]| -> Option<String> {
        let object = resolve(doc, dict.get(key).ok()?)?;
        lopdf::decode_text_string(object)
            .ok()
            .filter(|s| !s.trim().is_empty())
    };
    let sub_filter = name_of(b"SubFilter");
    let mut doc_mdp = None;
    let mut field_lock = None;
    if let Some(Object::Array(refs)) = dict.get(b"Reference").ok().and_then(|r| resolve(doc, r)) {
        for entry in refs {
            let Some(Object::Dictionary(reference)) = resolve(doc, entry) else {
                continue;
            };
            let method = match reference.get(b"TransformMethod") {
                Ok(Object::Name(n)) => n.as_slice(),
                _ => continue,
            };
            let params = reference
                .get(b"TransformParams")
                .ok()
                .and_then(|p| resolve(doc, p))
                .and_then(|p| p.as_dict().ok());
            match method {
                b"DocMDP" => {
                    let p = params
                        .and_then(|p| p.get(b"P").ok())
                        .and_then(|p| number(doc, p))
                        .map_or(2, |p| p as i64);
                    doc_mdp = Some(p.clamp(1, 3) as u8);
                }
                b"FieldMDP" => field_lock = params.and_then(|p| lock_of(doc, p)),
                _ => {}
            }
        }
    }
    let is_timestamp = matches!(dict.get(b"Type"), Ok(Object::Name(n)) if n == b"DocTimeStamp")
        || sub_filter.as_deref() == Some("ETSI.RFC3161");
    let certifies = id.is_some() && id == certification;
    let kind = if is_timestamp {
        SignatureKind::DocTimestamp
    } else if certifies || (certification.is_none() && doc_mdp.is_some()) {
        SignatureKind::Certification
    } else {
        SignatureKind::Approval
    };
    if kind == SignatureKind::Certification && doc_mdp.is_none() {
        doc_mdp = Some(2);
    }
    if kind != SignatureKind::Certification {
        doc_mdp = None;
    }
    let cert = match dict.get(b"Cert").ok().and_then(|c| resolve(doc, c)) {
        Some(Object::String(der, _)) => vec![der.clone()],
        Some(Object::Array(list)) => list
            .iter()
            .filter_map(|c| resolve(doc, c)?.as_str().ok().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    };
    Some(SignatureInfo {
        object: id,
        field_name: None,
        page: None,
        rect: None,
        filter: name_of(b"Filter"),
        sub_filter,
        kind,
        doc_mdp,
        field_lock,
        byte_range,
        contents,
        name: text_of(b"Name"),
        reason: text_of(b"Reason"),
        location: text_of(b"Location"),
        contact_info: text_of(b"ContactInfo"),
        m: dict
            .get(b"M")
            .ok()
            .and_then(|m| resolve(doc, m))
            .and_then(|m| m.as_str().ok())
            .map(|m| String::from_utf8_lossy(m).into_owned()),
        cert,
        gap_is_own,
    })
}

/// A FieldMDP `/TransformParams` or a field's `/Lock` dictionary.
pub fn lock_of(doc: &Document, params: &Dictionary) -> Option<FieldLock> {
    let action = match params.get(b"Action").ok().and_then(|a| resolve(doc, a))? {
        Object::Name(n) => n.clone(),
        _ => return None,
    };
    let names = || -> Vec<String> {
        params
            .get(b"Fields")
            .ok()
            .and_then(|f| resolve(doc, f))
            .and_then(|f| f.as_array().ok())
            .map(|list| {
                list.iter()
                    .filter_map(|n| lopdf::decode_text_string(resolve(doc, n)?).ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    match action.as_slice() {
        b"All" => Some(FieldLock::All),
        b"Include" => Some(FieldLock::Include(names())),
        b"Exclude" => Some(FieldLock::Exclude(names())),
        _ => None,
    }
}

/// Every signature field in the AcroForm tree (and only those), with its
/// fully qualified name.
fn signature_fields(doc: &Document) -> Vec<FieldNode> {
    let mut out = Vec::new();
    let Some(roots) = doc
        .catalog()
        .ok()
        .and_then(|c| resolve(doc, c.get(b"AcroForm").ok()?))
        .and_then(|a| a.as_dict().ok())
        .and_then(|a| resolve(doc, a.get(b"Fields").ok()?))
        .and_then(|f| f.as_array().ok())
    else {
        return out;
    };
    let mut visited = HashSet::new();
    for root in roots {
        walk_field(doc, root, "", None, &mut visited, &mut out, 0);
    }
    out
}

fn walk_field(
    doc: &Document,
    node: &Object,
    parent_name: &str,
    parent_ft: Option<&[u8]>,
    visited: &mut HashSet<ObjectId>,
    out: &mut Vec<FieldNode>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let id = match node {
        Object::Reference(id) => {
            if !visited.insert(*id) {
                return;
            }
            Some(*id)
        }
        _ => None,
    };
    let Some(dict) = resolve(doc, node).and_then(|n| n.as_dict().ok()) else {
        return;
    };
    let partial = dict
        .get(b"T")
        .ok()
        .and_then(|t| lopdf::decode_text_string(t).ok());
    let name = match (&partial, parent_name.is_empty()) {
        (Some(t), true) => t.clone(),
        (Some(t), false) => format!("{parent_name}.{t}"),
        (None, _) => parent_name.to_string(),
    };
    let ft = match dict.get(b"FT") {
        Ok(Object::Name(n)) => Some(n.as_slice()),
        _ => parent_ft,
    };
    let kids: Vec<&Object> = dict
        .get(b"Kids")
        .ok()
        .and_then(|k| resolve(doc, k))
        .and_then(|k| k.as_array().ok())
        .map(|k| k.iter().collect())
        .unwrap_or_default();
    // Kids that carry a /T are fields; kids without one are this field's
    // widgets.
    let mut widgets = Vec::new();
    if matches!(dict.get(b"Subtype"), Ok(Object::Name(n)) if n == b"Widget") {
        widgets.extend(id);
    }
    for kid in &kids {
        let Some(kid_dict) = resolve(doc, kid).and_then(|k| k.as_dict().ok()) else {
            continue;
        };
        if kid_dict.has(b"T") {
            walk_field(doc, kid, &name, ft, visited, out, depth + 1);
        } else if let Object::Reference(kid_id) = kid {
            widgets.push(*kid_id);
        }
    }
    if ft != Some(b"Sig".as_slice()) || partial.is_none() {
        return;
    }
    let value = match dict.get(b"V") {
        Ok(Object::Reference(id)) => Some(SigValue::Ref(*id)),
        Ok(Object::Dictionary(d)) => Some(SigValue::Direct(Box::new(d.clone()))),
        _ => None,
    };
    let lock = dict
        .get(b"Lock")
        .ok()
        .and_then(|l| resolve(doc, l))
        .and_then(|l| l.as_dict().ok())
        .and_then(|l| lock_of(doc, l));
    out.push(FieldNode {
        name,
        value,
        widgets,
        lock,
    });
}

/// Annotation object -> zero-based page index, for every page's `/Annots`.
pub fn annotation_pages(doc: &Document) -> HashMap<ObjectId, u32> {
    let mut out = HashMap::new();
    for (number, page) in doc.get_pages() {
        for annot in page_annots(doc, page) {
            out.entry(annot).or_insert(number - 1);
        }
    }
    out
}

/// The annotation references in a page's `/Annots`.
pub fn page_annots(doc: &Document, page: ObjectId) -> Vec<ObjectId> {
    doc.get_dictionary(page)
        .ok()
        .and_then(|p| resolve(doc, p.get(b"Annots").ok()?))
        .and_then(|a| a.as_array().ok())
        .map(|list| {
            list.iter()
                .filter_map(|a| match a {
                    Object::Reference(id) => Some(*id),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Page object ids in page order.
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    let pages: BTreeMap<u32, ObjectId> = doc.get_pages();
    pages.into_values().collect()
}

/// Follow references (a few hops at most) to the object they name.
pub fn resolve<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    let mut current = object;
    for _ in 0..8 {
        match current {
            Object::Reference(id) => current = doc.get_object(*id).ok()?,
            other => return Some(other),
        }
    }
    None
}

fn number(doc: &Document, object: &Object) -> Option<f64> {
    match resolve(doc, object)? {
        Object::Integer(n) => Some(*n as f64),
        Object::Real(n) => Some(f64::from(*n)),
        _ => None,
    }
}

fn rect_of(doc: &Document, object: &Object) -> Option<[f32; 4]> {
    let list = resolve(doc, object)?.as_array().ok()?;
    let values: Vec<f32> = list
        .iter()
        .map(|o| number(doc, o).map(|n| n as f32))
        .collect::<Option<_>>()?;
    let [x1, y1, x2, y2] = <[f32; 4]>::try_from(values).ok()?;
    Some([x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)])
}

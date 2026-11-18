//! Digital signatures: finding them (`parse`), checking them (`verify`) and
//! telling what changed after each one was made (`changes`).
//!
//! Everything here is offline. Trust in a signer's certificate comes from the
//! system's own roots on macOS; elsewhere a signature is checked for
//! integrity and the signer named, with the identity reported as not checked.

pub mod changes;
pub mod parse;
pub mod verify;

#[cfg(test)]
mod tests;

use crate::engine::save::PageSpace;
use crate::engine::types::RectDto;
use changes::{Change, Policy};
use lopdf::{Document, Object, ObjectId};
use parse::{SignatureInfo, SignatureKind};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use verify::Trust;

/// Whether the PDF in `bytes` carries at least one signature value.
///
/// A document with signatures must never be fully rewritten: a rewrite
/// renumbers and re-serialises every object, so no signed byte range survives
/// it. Anything that would rewrite a file (a restructuring save, encryption,
/// redaction) should ask this first.
pub fn has_signatures(bytes: &[u8]) -> bool {
    parse::has_signature_bytes(bytes) && !parse::signatures_in(bytes).is_empty()
}

/// `has_signatures` for a file on disk. A file that cannot be read has none.
pub fn file_has_signatures(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| has_signatures(&bytes))
}

#[derive(Serialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignerDto {
    pub name: Option<String>,
    pub email: Option<String>,
    pub organization: Option<String>,
    pub issuer: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimestampDto {
    pub time: Option<String>,
    pub valid: bool,
    pub authority: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDto {
    pub kind: &'static str,
    pub allowed: bool,
    pub page: Option<u32>,
    pub detail: String,
}

/// One signature, checked: what the Signatures panel shows.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SignatureReport {
    pub field_name: Option<String>,
    /// Zero-based source page of the signature's widget.
    pub page: Option<u32>,
    /// The widget, in display-normalized page space (as form widgets are).
    pub rect: Option<RectDto>,
    /// "approval", "certification" or "timestamp".
    pub kind: &'static str,
    pub sub_filter: Option<String>,
    /// "valid", "identityUnknown", "modified", "invalid" or "unsupported".
    pub status: &'static str,
    /// One sentence saying why the status is what it is.
    pub summary: String,
    pub signer: SignerDto,
    /// `YYYY-MM-DD HH:MM UTC`.
    pub time: Option<String>,
    /// "timestamp", "signer" (a signed attribute) or "pdf" (the unsigned /M).
    pub time_source: Option<&'static str>,
    pub timestamp: Option<TimestampDto>,
    /// The signed bytes are unchanged and the signature verifies.
    pub intact: bool,
    pub problem: Option<String>,
    /// Nothing was appended after the signed revision.
    pub covers_whole_file: bool,
    /// Which revision of the file this signature signed, 1-based, and how
    /// many there are.
    pub revision: u32,
    pub revisions: u32,
    /// "trusted", "untrusted" or "notChecked".
    pub trust: &'static str,
    pub trust_detail: Option<String>,
    /// DocMDP level of a certification signature.
    pub certification: Option<u8>,
    /// Which fields this signature locks, in words.
    pub locks: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub digest: Option<&'static str>,
    pub changes: Vec<ChangeDto>,
}

/// Check every signature in the PDF `bytes`.
pub fn check_document(bytes: &[u8]) -> Vec<SignatureReport> {
    if !parse::has_signature_bytes(bytes) {
        return Vec::new();
    }
    let final_doc = parse::load(bytes);
    let sigs = parse::signatures_with(bytes, final_doc.as_ref());
    if sigs.is_empty() {
        return Vec::new();
    }
    let ends = parse::revision_ends(bytes);
    let revisions = ends.len().max(1) as u32;
    // Each signed revision is loaded once, however many signatures end there.
    let mut revisions_loaded: HashMap<usize, Option<Document>> = HashMap::new();

    sigs.iter()
        .map(|sig| {
            let end = sig.signed_end();
            let policy = policy_for(&sigs, end);
            let changes = match (end, final_doc.as_ref()) {
                (Some(end), Some(final_doc)) if end < bytes.len() && trailing(bytes, end) => {
                    let old = revisions_loaded
                        .entry(end)
                        .or_insert_with(|| parse::load(&bytes[..end]));
                    match old {
                        Some(old) => changes::changes_between(old, final_doc, &policy),
                        None => vec![Change {
                            kind: changes::ChangeKind::Other,
                            allowed: false,
                            page: None,
                            detail: "The signed revision cannot be read to compare".into(),
                        }],
                    }
                }
                _ => Vec::new(),
            };
            report(bytes, sig, changes, &ends, revisions, final_doc.as_ref())
        })
        .collect()
}

/// Whether anything but whitespace follows `end`.
fn trailing(bytes: &[u8], end: usize) -> bool {
    bytes[end..]
        .iter()
        .any(|ch| !ch.is_ascii_whitespace() && *ch != 0)
}

/// The rules a signature ending at `end` lives under: the certification
/// level (from a certification signature at or before it) and every lock set
/// at or before it.
fn policy_for(sigs: &[SignatureInfo], end: Option<usize>) -> Policy {
    let end = end.unwrap_or(usize::MAX);
    let earlier = || {
        sigs.iter()
            .filter(|s| s.signed_end().is_some_and(|e| e <= end))
    };
    Policy {
        doc_mdp: earlier()
            .filter(|s| s.kind == SignatureKind::Certification)
            .find_map(|s| s.doc_mdp),
        locks: earlier().filter_map(|s| s.field_lock.clone()).collect(),
    }
}

fn report(
    bytes: &[u8],
    sig: &SignatureInfo,
    changes: Vec<Change>,
    ends: &[usize],
    revisions: u32,
    doc: Option<&Document>,
) -> SignatureReport {
    let coverage = parse::check_coverage(bytes, &sig.byte_range).and_then(|c| {
        if sig.gap_is_own {
            Ok(c)
        } else {
            Err("the byte range points at another signature's value, not this one's".into())
        }
    });
    let crypto = verify::check_crypto(bytes, sig);
    let intact = crypto.intact && coverage.is_ok();
    let problem = coverage.as_ref().err().cloned().or(crypto.problem.clone());

    // Trust is asked of the system only for a signature worth trusting.
    let trust = if intact {
        verify::system_trust(&crypto.chain, crypto.signing_time)
    } else {
        Trust::NotTrusted("the signature is not intact".into())
    };
    let disallowed = changes.iter().filter(|c| !c.allowed).count();
    let status = if !crypto.supported {
        "unsupported"
    } else if !intact {
        "invalid"
    } else if disallowed > 0 {
        "modified"
    } else if trust == Trust::Trusted {
        "valid"
    } else {
        "identityUnknown"
    };
    let trust_detail = match &trust {
        Trust::Trusted => None,
        Trust::NotTrusted(why) if intact => Some(format!(
            "The signer's certificate is not trusted on this Mac: {why}"
        )),
        Trust::NotTrusted(_) => None,
        Trust::NotChecked => Some(
            "Identity not checked: SpeedyF checks certificates against the system's trusted roots on macOS only".into(),
        ),
    };
    let after = if changes.is_empty() {
        String::new()
    } else if disallowed > 0 {
        " It was changed after signing in ways the signature does not allow.".into()
    } else {
        " Later changes are of a kind signatures allow.".into()
    };
    let summary = match status {
        "unsupported" => format!(
            "SpeedyF cannot check this kind of signature ({}).",
            sig.sub_filter.as_deref().unwrap_or("no SubFilter")
        ),
        "invalid" => format!(
            "The signature is broken: {}.",
            problem.as_deref().unwrap_or("it does not verify")
        ),
        "modified" => {
            "The signed version is intact, but later changes are not allowed by the signature."
                .into()
        }
        "valid" => format!("Signed by a trusted certificate; the signed bytes are intact.{after}"),
        _ => format!(
            "The signed bytes are intact, but the signer's identity could not be confirmed.{after}"
        ),
    };

    let revision = sig
        .signed_end()
        .and_then(|end| ends.iter().position(|e| *e >= end))
        .map_or(revisions, |i| i as u32 + 1);
    let rect = match (doc, sig.page, sig.rect) {
        (Some(doc), Some(page), Some(rect)) => display_rect(doc, page, rect),
        _ => None,
    };
    SignatureReport {
        field_name: sig.field_name.clone(),
        page: sig.page,
        rect,
        kind: match sig.kind {
            SignatureKind::Approval => "approval",
            SignatureKind::Certification => "certification",
            SignatureKind::DocTimestamp => "timestamp",
        },
        sub_filter: sig.sub_filter.clone(),
        status,
        summary,
        signer: SignerDto {
            name: crypto.signer.name.clone(),
            email: crypto.signer.email.clone(),
            organization: crypto.signer.organization.clone(),
            issuer: crypto.signer.issuer.clone(),
        },
        time: crypto.signing_time.map(verify::format_time),
        time_source: crypto.time_source,
        timestamp: crypto.timestamp.as_ref().map(|t| TimestampDto {
            time: t.time.map(verify::format_time),
            valid: t.valid,
            authority: t.authority.clone(),
        }),
        intact,
        problem,
        covers_whole_file: coverage.as_ref().is_ok_and(|c| c.whole_file),
        revision,
        revisions,
        trust: match trust {
            Trust::Trusted => "trusted",
            Trust::NotTrusted(_) => "untrusted",
            Trust::NotChecked => "notChecked",
        },
        trust_detail,
        certification: sig.doc_mdp,
        locks: sig.field_lock.as_ref().map(|lock| match lock {
            parse::FieldLock::All => "Locks every form field".to_string(),
            parse::FieldLock::Include(names) => format!("Locks {}", names.join(", ")),
            parse::FieldLock::Exclude(names) => {
                format!("Locks every form field except {}", names.join(", "))
            }
        }),
        reason: sig.reason.clone(),
        location: sig.location.clone(),
        digest: crypto.digest.map(verify::DigestAlg::name),
        changes: changes
            .into_iter()
            .map(|c| ChangeDto {
                kind: c.kind.as_str(),
                allowed: c.allowed,
                page: c.page,
                detail: c.detail,
            })
            .collect(),
    }
}

/// A widget rectangle in user space, mapped into the page's display space
/// (crop box origin, /Rotate applied) the way form widgets are reported.
fn display_rect(doc: &Document, page: u32, rect: [f32; 4]) -> Option<RectDto> {
    let page_id = *doc.get_pages().get(&(page + 1))?;
    let crop = inherited(doc, page_id, b"CropBox")
        .or_else(|| inherited(doc, page_id, b"MediaBox"))
        .and_then(|o| box_of(doc, o))
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let rot = inherited(doc, page_id, b"Rotate")
        .and_then(|o| match o {
            Object::Integer(n) => Some(n.rem_euclid(360) as u16),
            _ => None,
        })
        .unwrap_or(0);
    let space = PageSpace::from_box(crop, rot);
    let (x1, y1) = space.user_to_disp(rect[0], rect[1]);
    let (x2, y2) = space.user_to_disp(rect[2], rect[3]);
    Some(RectDto {
        x: x1.min(x2),
        y: y1.min(y2),
        w: (x2 - x1).abs(),
        h: (y2 - y1).abs(),
    })
}

/// A page attribute, looked up the page tree when the page does not set it.
fn inherited<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut current = page;
    for _ in 0..32 {
        let dict = doc.get_dictionary(current).ok()?;
        if let Ok(value) = dict.get(key) {
            return parse::resolve(doc, value);
        }
        current = match dict.get(b"Parent") {
            Ok(Object::Reference(parent)) => *parent,
            _ => return None,
        };
    }
    None
}

fn box_of(doc: &Document, object: &Object) -> Option<[f32; 4]> {
    let list = object.as_array().ok()?;
    let values: Vec<f32> = list
        .iter()
        .map(|o| match parse::resolve(doc, o)? {
            Object::Integer(n) => Some(*n as f32),
            Object::Real(n) => Some(*n),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let [x1, y1, x2, y2] = <[f32; 4]>::try_from(values).ok()?;
    Some([x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)])
}

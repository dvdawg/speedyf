//! Signed PDFs made here, with a test key and a self-signed certificate
//! (`tests/fixtures/signing`, made once with openssl and valid for a
//! century), so every case - tampering, later updates, certification, legacy
//! SHA-1 - is a few hundred bytes built in the test rather than a binary
//! fixture nobody can read.

use super::changes::ChangeKind;
use super::parse::{self, SignatureKind};
use super::verify::{self, DigestAlg};
use super::{check_document, has_signatures, SignatureReport};
use der::asn1::ObjectIdentifier as Oid;
use der::{Decode, Encode};
use x509_cert::Certificate;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/signing");

// ---------------------------------------------------------------------------
// DER, by hand: what is signed has to be exactly what was encoded.

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.extend([0x81, len as u8]);
    } else if len < 0x10000 {
        out.extend([0x82, (len >> 8) as u8, len as u8]);
    } else {
        out.extend([0x83, (len >> 16) as u8, (len >> 8) as u8, len as u8]);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn set(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x31, &parts.concat())
}

fn oid(text: &str) -> Vec<u8> {
    Oid::new_unwrap(text).to_der().unwrap()
}

fn octets(bytes: &[u8]) -> Vec<u8> {
    tlv(0x04, bytes)
}

fn int(n: u8) -> Vec<u8> {
    tlv(0x02, &[n])
}

fn alg(digest: DigestAlg) -> Vec<u8> {
    seq(&[digest.oid().to_der().unwrap(), vec![0x05, 0x00]])
}

fn attr(kind: &str, value: Vec<u8>) -> Vec<u8> {
    seq(&[oid(kind), set(&[value])])
}

const DATA: &str = "1.2.840.113549.1.7.1";
const SIGNED_DATA: &str = "1.2.840.113549.1.7.2";
const CONTENT_TYPE: &str = "1.2.840.113549.1.9.3";
const MESSAGE_DIGEST: &str = "1.2.840.113549.1.9.4";
const SIGNING_TIME: &str = "1.2.840.113549.1.9.5";
const TIMESTAMP_TOKEN: &str = "1.2.840.113549.1.9.16.2.14";
const TST_INFO: &str = "1.2.840.113549.1.9.16.1.4";

// ---------------------------------------------------------------------------
// Signers

enum Key {
    Ec(p256::ecdsa::SigningKey),
    Rsa(Box<rsa::RsaPrivateKey>),
}

struct Signer {
    key: Key,
    cert: Vec<u8>,
}

impl Signer {
    fn ec() -> Signer {
        let raw = std::fs::read(format!("{FIXTURES}/ec-key.raw")).unwrap();
        Signer {
            key: Key::Ec(p256::ecdsa::SigningKey::from_slice(&raw).unwrap()),
            cert: std::fs::read(format!("{FIXTURES}/ec-cert.der")).unwrap(),
        }
    }

    fn rsa() -> Signer {
        use rsa::pkcs1::DecodeRsaPrivateKey;
        let der = std::fs::read(format!("{FIXTURES}/rsa-key.der")).unwrap();
        Signer {
            key: Key::Rsa(Box::new(rsa::RsaPrivateKey::from_pkcs1_der(&der).unwrap())),
            cert: std::fs::read(format!("{FIXTURES}/rsa-cert.der")).unwrap(),
        }
    }

    fn sig_alg(&self) -> Vec<u8> {
        match self.key {
            Key::Ec(_) => seq(&[oid("1.2.840.10045.4.3.2")]),
            Key::Rsa(_) => seq(&[oid("1.2.840.113549.1.1.1"), vec![0x05, 0x00]]),
        }
    }

    /// Sign a digest already made with `digest`.
    fn sign(&self, digest: DigestAlg, hash: &[u8]) -> Vec<u8> {
        match &self.key {
            Key::Ec(key) => {
                use ecdsa::signature::hazmat::PrehashSigner;
                let sig: p256::ecdsa::Signature = key.sign_prehash(hash).unwrap();
                sig.to_der().as_bytes().to_vec()
            }
            Key::Rsa(key) => {
                use rsa::Pkcs1v15Sign;
                let scheme = match digest {
                    DigestAlg::Sha1 => Pkcs1v15Sign::new::<sha1::Sha1>(),
                    _ => Pkcs1v15Sign::new::<sha2::Sha256>(),
                };
                key.sign(scheme, hash).unwrap()
            }
        }
    }

    fn issuer_and_serial(&self) -> Vec<u8> {
        let cert = Certificate::from_der(&self.cert).unwrap();
        seq(&[
            cert.tbs_certificate.issuer.to_der().unwrap(),
            cert.tbs_certificate.serial_number.to_der().unwrap(),
        ])
    }

    /// A CMS SignedData. `econtent` is encapsulated when given; the signed
    /// attributes carry `content_digest` as the message digest.
    fn signed_data(
        &self,
        digest: DigestAlg,
        content_type: &str,
        econtent: Option<&[u8]>,
        content_digest: &[u8],
        unsigned: Option<Vec<u8>>,
    ) -> Vec<u8> {
        let attrs = [
            attr(CONTENT_TYPE, oid(content_type)),
            attr(SIGNING_TIME, tlv(0x17, b"261117120000Z")),
            attr(MESSAGE_DIGEST, octets(content_digest)),
        ]
        .concat();
        let signature = self.sign(digest, &digest.digest(&[&tlv(0x31, &attrs)]));
        let mut signer_info = vec![
            int(1),
            self.issuer_and_serial(),
            alg(digest),
            tlv(0xa0, &attrs),
            self.sig_alg(),
            octets(&signature),
        ];
        if let Some(unsigned) = unsigned {
            signer_info.push(tlv(0xa1, &unsigned));
        }
        let encap = match econtent {
            Some(content) => seq(&[oid(content_type), tlv(0xa0, &octets(content))]),
            None => seq(&[oid(content_type)]),
        };
        let signed_data = seq(&[
            int(1),
            set(&[alg(digest)]),
            encap,
            tlv(0xa0, &self.cert),
            set(&[seq(&signer_info)]),
        ]);
        seq(&[oid(SIGNED_DATA), tlv(0xa0, &signed_data)])
    }

    /// The SignerInfo signature value inside a SignedData made above.
    fn signature_value(signed_data: &[u8]) -> Vec<u8> {
        let info = cms::content_info::ContentInfo::from_der(signed_data).unwrap();
        let data: cms::signed_data::SignedData = info.content.decode_as().unwrap();
        let signer = data.signer_infos.0.iter().next().unwrap().clone();
        signer.signature.as_bytes().to_vec()
    }

    /// An RFC 3161 token stamping `imprint` (a SHA-256 digest), signed by
    /// this signer acting as the authority.
    fn timestamp_token(&self, imprint: &[u8]) -> Vec<u8> {
        let tst = seq(&[
            int(1),
            oid("1.2.3.4.1"),
            seq(&[alg(DigestAlg::Sha256), octets(imprint)]),
            int(7),
            tlv(0x18, b"20261117123000Z"),
        ]);
        let digest = DigestAlg::Sha256.digest(&[&tst]);
        self.signed_data(DigestAlg::Sha256, TST_INFO, Some(&tst), &digest, None)
    }
}

// ---------------------------------------------------------------------------
// PDFs

const PLACEHOLDER_RANGE: &str = "[0 0000000000 0000000000 0000000000]";
const CONTENTS_HEX: usize = 16384;

fn sig_dict(sub_filter: &str, extra: &str) -> Vec<u8> {
    format!(
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /{sub_filter} /ByteRange {PLACEHOLDER_RANGE} \
         /Contents <{}> /M (D:20261117120000Z) /Name (Test Signer) /Reason (Approved) {extra} >>",
        "0".repeat(CONTENTS_HEX)
    )
    .into_bytes()
}

fn stream(content: &str) -> Vec<u8> {
    format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len() + 1
    )
    .into_bytes()
}

fn write_objects(out: &mut Vec<u8>, objects: &[(u32, Vec<u8>)]) -> Vec<(u32, usize)> {
    let mut offsets = Vec::new();
    for (id, body) in objects {
        offsets.push((*id, out.len()));
        out.extend(format!("{id} 0 obj\n").as_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    offsets
}

fn write_xref(out: &mut Vec<u8>, offsets: &[(u32, usize)], size: u32, prev: Option<usize>) {
    let at = out.len();
    out.extend(b"xref\n");
    if prev.is_none() {
        out.extend(format!("0 {size}\n0000000000 65535 f \n").as_bytes());
        let mut sorted = offsets.to_vec();
        sorted.sort();
        for (_, offset) in sorted {
            out.extend(format!("{offset:010} 00000 n \n").as_bytes());
        }
    } else {
        for (id, offset) in offsets {
            out.extend(format!("{id} 1\n{offset:010} 00000 n \n").as_bytes());
        }
    }
    let prev = prev.map(|p| format!(" /Prev {p}")).unwrap_or_default();
    out.extend(
        format!("trailer\n<< /Size {size} /Root 1 0 R{prev} >>\nstartxref\n{at}\n%%EOF\n")
            .as_bytes(),
    );
}

/// A one-page document; objects must be numbered 1..=n.
fn base(objects: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let offsets = write_objects(&mut out, objects);
    write_xref(&mut out, &offsets, objects.len() as u32 + 1, None);
    out
}

/// Append an incremental update holding `objects`.
fn update(prev: &[u8], objects: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let text = String::from_utf8_lossy(prev);
    let start = text.rfind("startxref").unwrap();
    let prev_xref: usize = text[start + 9..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let size_at = text.rfind("/Size ").unwrap();
    let old_size: u32 = text[size_at + 6..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let size = objects
        .iter()
        .map(|(id, _)| id + 1)
        .max()
        .unwrap_or(0)
        .max(old_size);
    let mut out = prev.to_vec();
    let offsets = write_objects(&mut out, objects);
    write_xref(&mut out, &offsets, size, Some(prev_xref));
    out
}

/// Fill in the last placeholder signature in `pdf`; `cms` turns the signed
/// spans into the /Contents blob.
fn sign_last(mut pdf: Vec<u8>, cms: impl FnOnce(&[&[u8]]) -> Vec<u8>) -> Vec<u8> {
    let at = memchr::memmem::rfind(&pdf, PLACEHOLDER_RANGE.as_bytes()).unwrap();
    let open = at + memchr::memmem::find(&pdf[at..], b"/Contents <").unwrap() + 10;
    let close = open + 1 + CONTENTS_HEX;
    assert_eq!(pdf[close], b'>');
    let range = [0, open, close + 1, pdf.len() - (close + 1)];
    let text = format!("[0 {:010} {:010} {:010}]", range[1], range[2], range[3]);
    pdf[at..at + text.len()].copy_from_slice(text.as_bytes());
    let blob = {
        let spans: [&[u8]; 2] = [&pdf[..range[1]], &pdf[range[2]..]];
        cms(&spans)
    };
    let hex: String = blob.iter().map(|b| format!("{b:02X}")).collect();
    assert!(hex.len() <= CONTENTS_HEX);
    pdf[open + 1..open + 1 + hex.len()].copy_from_slice(hex.as_bytes());
    pdf
}

fn detached(signer: &Signer) -> impl FnOnce(&[&[u8]]) -> Vec<u8> + '_ {
    move |spans| {
        let digest = DigestAlg::Sha256.digest(spans);
        signer.signed_data(DigestAlg::Sha256, DATA, None, &digest, None)
    }
}

/// Catalog, page tree, a page with a text field, two signature fields (the
/// first holding signature object 9) and the rest.
fn form_objects(sig: Vec<u8>, catalog_extra: &str) -> Vec<(u32, Vec<u8>)> {
    vec![
        (
            1,
            format!("<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R {catalog_extra} >>").into_bytes(),
        ),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (3, page_dict("[6 0 R 7 0 R 10 0 R]")),
        (4, stream("BT /F1 24 Tf 72 700 Td (Signed paper) Tj ET")),
        (
            5,
            b"<< /Fields [6 0 R 7 0 R 10 0 R] /SigFlags 3 /DR << /Font << /Helv 8 0 R >> >> /DA (/Helv 0 Tf 0 g) >>"
                .to_vec(),
        ),
        (6, text_field("()", "")),
        (
            7,
            b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /Rect [72 500 300 550] /P 3 0 R /F 4 /V 9 0 R >>"
                .to_vec(),
        ),
        (8, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
        (9, sig),
        (
            10,
            b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature2) /Rect [320 500 540 550] /P 3 0 R /F 4 >>"
                .to_vec(),
        ),
    ]
}

fn page_dict(annots: &str) -> Vec<u8> {
    format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 8 0 R >> >> /Annots {annots} >>"
    )
    .into_bytes()
}

fn text_field(value: &str, extra: &str) -> Vec<u8> {
    format!(
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Name) /V {value} /Rect [72 600 300 620] \
         /P 3 0 R /F 4 /DA (/Helv 12 Tf 0 g) {extra} >>"
    )
    .into_bytes()
}

fn signed_form(sig_extra: &str, catalog_extra: &str) -> Vec<u8> {
    let signer = Signer::ec();
    let pdf = base(&form_objects(
        sig_dict("adbe.pkcs7.detached", sig_extra),
        catalog_extra,
    ));
    sign_last(pdf, detached(&signer))
}

/// An incremental update filling in the text field, with an appearance.
fn fill_name(pdf: &[u8]) -> Vec<u8> {
    update(
        pdf,
        &[
            (6, text_field("(Alice)", "/AP << /N 11 0 R >>")),
            (
                11,
                b"<< /Type /XObject /Subtype /Form /BBox [0 0 228 20] /Length 40 >>\nstream\n/Tx BMC BT /Helv 12 Tf 2 5 Td (Alice) Tj ET EMC\nendstream"
                    .to_vec(),
            ),
        ],
    )
}

fn add_comment(pdf: &[u8]) -> Vec<u8> {
    update(
        pdf,
        &[
            (3, page_dict("[6 0 R 7 0 R 10 0 R 12 0 R]")),
            (
                12,
                b"<< /Type /Annot /Subtype /Text /Rect [400 700 420 720] /Contents (Looks good) /P 3 0 R >>"
                    .to_vec(),
            ),
        ],
    )
}

fn only(pdf: &[u8]) -> SignatureReport {
    let mut reports = check_document(pdf);
    assert_eq!(reports.len(), 1, "{reports:#?}");
    reports.remove(0)
}

/// What a signature with an intact, self-signed certificate reports here:
/// identity unknown on macOS (the system does not trust the test root),
/// identity not checked elsewhere - the same status either way.
const INTACT: &str = "identityUnknown";

fn kinds(report: &SignatureReport) -> Vec<(&'static str, bool)> {
    report.changes.iter().map(|c| (c.kind, c.allowed)).collect()
}

// ---------------------------------------------------------------------------
// Cases

#[test]
fn a_valid_signature_is_intact_and_names_its_signer() {
    let pdf = signed_form("", "");
    let report = only(&pdf);
    assert!(report.intact, "{report:#?}");
    assert_eq!(report.status, INTACT);
    assert!(report.covers_whole_file);
    assert_eq!(report.signer.name.as_deref(), Some("Test Signer"));
    assert_eq!(report.signer.email.as_deref(), Some("signer@example.com"));
    assert_eq!(report.signer.organization.as_deref(), Some("SpeedyF Tests"));
    assert_eq!(report.time.as_deref(), Some("2026-11-17 12:00 UTC"));
    assert_eq!(report.time_source, Some("signer"));
    assert_eq!(report.field_name.as_deref(), Some("Signature1"));
    assert_eq!(report.page, Some(0));
    let rect = report.rect.unwrap();
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (72.0, 500.0, 228.0, 50.0));
    assert_eq!(report.kind, "approval");
    assert!(report.changes.is_empty());
    #[cfg(target_os = "macos")]
    assert_eq!(report.trust, "untrusted");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(report.trust, "notChecked");
    assert!(has_signatures(&pdf));
}

#[test]
fn an_unsigned_signature_field_is_not_a_signature() {
    let pdf = base(&form_objects(sig_dict("adbe.pkcs7.detached", ""), ""));
    assert!(check_document(&pdf).is_empty());
    assert!(!has_signatures(&pdf));
}

#[test]
fn one_changed_byte_breaks_the_signature() {
    let mut pdf = signed_form("", "");
    let at = memchr::memmem::find(&pdf, b"Signed paper").unwrap();
    pdf[at] = b'Z';
    let report = only(&pdf);
    assert!(!report.intact);
    assert_eq!(report.status, "invalid");
    assert!(report.problem.unwrap().contains("do not match"));
}

#[test]
fn a_changed_signature_value_breaks_the_signature() {
    let mut pdf = signed_form("", "");
    // Flip a byte of the signature itself, near the end of the DER blob.
    let sig = parse::signatures_in(&pdf).remove(0);
    let der_len = verify::trim_der(&sig.contents).unwrap().len();
    let hex_at = sig.byte_range[1] as usize + 1 + (der_len - 4) * 2;
    pdf[hex_at] = if pdf[hex_at] == b'0' { b'1' } else { b'0' };
    let report = only(&pdf);
    assert_eq!(report.status, "invalid", "{report:#?}");
}

#[test]
fn filling_in_the_form_after_signing_is_allowed() {
    let pdf = fill_name(&signed_form("", ""));
    let report = only(&pdf);
    assert!(report.intact);
    assert!(!report.covers_whole_file);
    assert_eq!(report.status, INTACT);
    assert_eq!(kinds(&report), vec![("formFill", true)]);
    assert!(report.changes[0].detail.contains("Name"));
    assert_eq!(report.revision, 1);
    assert_eq!(report.revisions, 2);
}

#[test]
fn adding_a_comment_after_signing_is_allowed() {
    let report = only(&add_comment(&signed_form("", "")));
    assert_eq!(report.status, INTACT);
    assert_eq!(kinds(&report), vec![("comment", true)]);
    assert_eq!(report.changes[0].page, Some(0));
}

#[test]
fn changing_page_content_after_signing_is_flagged() {
    let pdf = update(
        &signed_form("", ""),
        &[(4, stream("BT /F1 24 Tf 72 700 Td (Unsigned paper) Tj ET"))],
    );
    let report = only(&pdf);
    assert!(report.intact, "the signed revision itself is untouched");
    assert_eq!(report.status, "modified");
    assert_eq!(kinds(&report), vec![("pageContent", false)]);
}

#[test]
fn adding_a_page_after_signing_is_flagged() {
    let pdf = update(
        &signed_form("", ""),
        &[
            (
                2,
                b"<< /Type /Pages /Kids [3 0 R 13 0 R] /Count 2 >>".to_vec(),
            ),
            (
                13,
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            ),
        ],
    );
    let report = only(&pdf);
    assert_eq!(report.status, "modified");
    assert!(report
        .changes
        .iter()
        .any(|c| c.kind == "pages" && !c.allowed && c.detail == "1 page added"));
}

#[test]
fn a_second_signature_is_checked_and_allowed_after_the_first() {
    let signer = Signer::ec();
    let once = signed_form("", "");
    let twice = update(
        &once,
        &[
            (
                10,
                b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature2) /Rect [320 500 540 550] /P 3 0 R /F 4 /V 13 0 R >>"
                    .to_vec(),
            ),
            (13, sig_dict("ETSI.CAdES.detached", "")),
        ],
    );
    let twice = sign_last(twice, detached(&signer));
    let reports = check_document(&twice);
    assert_eq!(reports.len(), 2);
    let (first, second) = (&reports[0], &reports[1]);
    assert_eq!(first.field_name.as_deref(), Some("Signature1"));
    assert_eq!(first.status, INTACT);
    assert_eq!(kinds(first), vec![("signature", true)]);
    assert_eq!(second.field_name.as_deref(), Some("Signature2"));
    assert_eq!(second.sub_filter.as_deref(), Some("ETSI.CAdES.detached"));
    assert_eq!(second.status, INTACT);
    assert!(second.covers_whole_file);
    assert_eq!((second.revision, second.revisions), (2, 2));
}

fn certified(level: u8) -> Vec<u8> {
    let reference = format!(
        "/Reference [<< /Type /SigRef /TransformMethod /DocMDP /TransformParams \
         << /Type /TransformParams /P {level} /V /1.2 >> >>]"
    );
    signed_form(&reference, "/Perms << /DocMDP 9 0 R >>")
}

#[test]
fn certification_levels_decide_what_later_changes_are_allowed() {
    let report = only(&certified(2));
    assert_eq!(report.kind, "certification");
    assert_eq!(report.certification, Some(2));
    assert!(report.changes.is_empty());

    // Level 2: form filling yes, comments no.
    assert_eq!(only(&fill_name(&certified(2))).status, INTACT);
    let commented = only(&add_comment(&certified(2)));
    assert_eq!(commented.status, "modified");
    assert_eq!(kinds(&commented), vec![("comment", false)]);

    // Level 3: comments too.
    assert_eq!(only(&add_comment(&certified(3))).status, INTACT);

    // Level 1: nothing at all.
    let filled = only(&fill_name(&certified(1)));
    assert_eq!(filled.status, "modified");
    assert_eq!(kinds(&filled), vec![("formFill", false)]);
}

#[test]
fn a_locked_field_may_not_be_filled_in_later() {
    let lock = "/Reference [<< /Type /SigRef /TransformMethod /FieldMDP /TransformParams \
                << /Type /TransformParams /Action /Include /Fields [(Name)] /V /1.2 >> >>]";
    let pdf = signed_form(lock, "");
    assert_eq!(only(&pdf).locks.as_deref(), Some("Locks Name"));
    let report = only(&fill_name(&pdf));
    assert_eq!(report.status, "modified");
    assert_eq!(kinds(&report), vec![("formFill", false)]);
    assert!(report.changes[0].detail.starts_with("Changed locked"));
}

#[test]
fn legacy_sha1_signatures_verify() {
    let signer = Signer::rsa();
    let pdf = base(&form_objects(sig_dict("adbe.pkcs7.sha1", ""), ""));
    let pdf = sign_last(pdf, |spans| {
        let content = DigestAlg::Sha1.digest(spans);
        let digest = DigestAlg::Sha1.digest(&[&content]);
        signer.signed_data(DigestAlg::Sha1, DATA, Some(&content), &digest, None)
    });
    let report = only(&pdf);
    assert_eq!(report.status, INTACT, "{report:#?}");
    assert_eq!(report.digest, Some("SHA-1"));
    assert_eq!(report.signer.name.as_deref(), Some("Legacy Signer"));

    let mut tampered = pdf.clone();
    let at = memchr::memmem::find(&tampered, b"Signed paper").unwrap();
    tampered[at] = b'Z';
    assert_eq!(only(&tampered).status, "invalid");
}

#[test]
fn raw_rsa_signatures_are_reported_as_unsupported() {
    let pdf = base(&form_objects(sig_dict("adbe.x509.rsa_sha1", ""), ""));
    let pdf = sign_last(pdf, |spans| octets(&DigestAlg::Sha1.digest(spans)));
    let report = only(&pdf);
    assert_eq!(report.status, "unsupported");
    assert_eq!(report.time_source, Some("pdf"));
    assert!(report.summary.contains("adbe.x509.rsa_sha1"));
}

#[test]
fn an_embedded_timestamp_dates_the_signature() {
    let signer = Signer::ec();
    let make = |tamper: bool| {
        let pdf = base(&form_objects(sig_dict("ETSI.CAdES.detached", ""), ""));
        sign_last(pdf, |spans| {
            let digest = DigestAlg::Sha256.digest(spans);
            let plain = signer.signed_data(DigestAlg::Sha256, DATA, None, &digest, None);
            let value = Signer::signature_value(&plain);
            let mut imprint = DigestAlg::Sha256.digest(&[&value]);
            if tamper {
                imprint[0] ^= 1;
            }
            let token = signer.timestamp_token(&imprint);
            let unsigned = attr(TIMESTAMP_TOKEN, token);
            signer.signed_data(DigestAlg::Sha256, DATA, None, &digest, Some(unsigned))
        })
    };
    // ECDSA signing here is deterministic (RFC 6979), so the signature value
    // stamped above is the one in the final SignedData.
    let good = only(&make(false));
    assert_eq!(good.status, INTACT, "{good:#?}");
    let stamp = good.timestamp.clone().unwrap();
    assert!(stamp.valid);
    assert_eq!(stamp.time.as_deref(), Some("2026-11-17 12:30 UTC"));
    assert_eq!(good.time_source, Some("timestamp"));
    assert_eq!(good.time.as_deref(), Some("2026-11-17 12:30 UTC"));

    let bad = only(&make(true));
    assert!(
        bad.intact,
        "a bad timestamp does not break the signature itself"
    );
    assert!(!bad.timestamp.unwrap().valid);
    assert_eq!(bad.time_source, Some("signer"));
}

#[test]
fn a_document_timestamp_is_checked_against_the_byte_range() {
    let signer = Signer::ec();
    let pdf = base(&form_objects(
        sig_dict("ETSI.RFC3161", "").replace_range_type(),
        "",
    ));
    let pdf = sign_last(pdf, |spans| {
        signer.timestamp_token(&DigestAlg::Sha256.digest(spans))
    });
    let report = only(&pdf);
    assert_eq!(report.kind, "timestamp");
    assert_eq!(report.status, INTACT, "{report:#?}");
    assert_eq!(report.time.as_deref(), Some("2026-11-17 12:30 UTC"));
}

trait DocTimestamp {
    fn replace_range_type(self) -> Vec<u8>;
}

impl DocTimestamp for Vec<u8> {
    fn replace_range_type(self) -> Vec<u8> {
        String::from_utf8(self)
            .unwrap()
            .replace("/Type /Sig ", "/Type /DocTimeStamp ")
            .into_bytes()
    }
}

#[test]
fn a_byte_range_that_stops_short_of_the_revision_is_invalid() {
    let signer = Signer::ec();
    let pdf = base(&form_objects(sig_dict("adbe.pkcs7.detached", ""), ""));
    // Sign as usual, then claim a range one byte short and sign that.
    let mut pdf = sign_last(pdf, detached(&signer));
    let sig = parse::signatures_in(&pdf).remove(0);
    let short = sig.byte_range[3] - 7;
    let at = memchr::memmem::find(&pdf, b"/ByteRange [").unwrap() + 11;
    let text = format!(
        "[0 {:010} {:010} {:010}]",
        sig.byte_range[1], sig.byte_range[2], short
    );
    pdf[at..at + text.len()].copy_from_slice(text.as_bytes());
    let report = only(&pdf);
    assert_eq!(report.status, "invalid");
    assert!(report.problem.unwrap().contains("part-way"));
}

#[test]
fn a_signature_removed_from_its_field_later_is_still_found() {
    let pdf = update(
        &signed_form("", ""),
        &[(
            7,
            b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /Rect [72 500 300 550] /P 3 0 R /F 4 >>"
                .to_vec(),
        )],
    );
    let report = only(&pdf);
    assert_eq!(report.status, "modified", "{report:#?}");
    assert!(report.changes.iter().any(|c| c.kind == "other"));
}

#[test]
fn a_replaced_signature_object_is_found_in_its_own_revision() {
    // The later update overwrites object 9 with a dictionary that has no
    // byte range at all; the original lives only in revision one.
    let pdf = update(&signed_form("", ""), &[(9, b"<< /Type /Sig >>".to_vec())]);
    let sigs = parse::signatures_in(&pdf);
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].kind, SignatureKind::Approval);
    let report = only(&pdf);
    assert!(report.intact);
    assert_eq!(report.status, "modified", "{report:#?}");
}

#[test]
fn pdf_dates_and_times_read_back() {
    assert_eq!(
        verify::pdf_date("D:20261117120000Z").map(verify::format_time),
        Some("2026-11-17 12:00 UTC".into())
    );
    assert_eq!(
        verify::pdf_date("D:20261117140000+02'00'").map(verify::format_time),
        Some("2026-11-17 12:00 UTC".into())
    );
    assert_eq!(
        verify::pdf_date("D:2026").map(verify::format_time),
        Some("2026-01-01 00:00 UTC".into())
    );
    assert_eq!(verify::pdf_date("garbage"), None);
}

#[test]
fn change_kinds_name_themselves() {
    assert_eq!(ChangeKind::FormFill.as_str(), "formFill");
    assert_eq!(ChangeKind::PageContent.as_str(), "pageContent");
}

// ---------------------------------------------------------------------------
// Saving a signed document with SpeedyF's own save path

mod saving {
    use super::*;
    use crate::engine::pdfium_init;
    use crate::engine::save::{build_output, build_output_with, SaveOptions};
    use crate::engine::types::{EditPlan, PlanAnnot, PlanPage, RectDto};
    use crate::errors::AppError;
    use pdfium_render::prelude::Pdfium;

    fn pdfium() -> Pdfium {
        Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
    }

    fn page(src: u32, annots: Vec<PlanAnnot>) -> PlanPage {
        PlanPage {
            src_index: Some(src),
            width_pt: 612.0,
            height_pt: 792.0,
            rotation: 0,
            annots,
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }
    }

    fn plan(pages: Vec<PlanPage>, form: Vec<(String, String)>) -> EditPlan {
        EditPlan {
            pages,
            form,
            form_buttons: Vec::new(),
            form_choices: Vec::new(),
        }
    }

    fn highlight() -> PlanAnnot {
        PlanAnnot {
            kind: "highlight".into(),
            rect: RectDto {
                x: 72.0,
                y: 690.0,
                w: 150.0,
                h: 30.0,
            },
            color: "#ffd400".into(),
            opacity: 0.4,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
            signature: None,
            flatten: false,
        }
    }

    fn source(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("signed.pdf");
        std::fs::write(&path, signed_form("", "")).unwrap();
        path
    }

    #[test]
    fn a_highlight_and_a_filled_field_keep_the_signature_valid() {
        let _guard = pdfium_init::test_guard();
        let pdfium = pdfium();
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path());
        let out = dir.path().join("out.pdf");
        let plan = plan(
            vec![page(0, vec![highlight()])],
            vec![("Name".into(), "Alice".into())],
        );
        build_output(&pdfium, src.to_str().unwrap(), None, &plan, &out).expect("save");
        let bytes = std::fs::read(&out).unwrap();
        let original = std::fs::read(&src).unwrap();
        assert!(
            bytes.starts_with(&original),
            "the signed bytes are kept verbatim"
        );
        let report = only(&bytes);
        assert!(report.intact);
        assert_eq!(report.status, INTACT, "{report:#?}");
        let found: Vec<_> = kinds(&report);
        assert!(found.contains(&("comment", true)), "{found:?}");
        assert!(found.contains(&("formFill", true)), "{found:?}");
    }

    #[test]
    fn a_restructuring_save_is_refused_unless_a_copy_was_asked_for() {
        let _guard = pdfium_init::test_guard();
        let pdfium = pdfium();
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path());
        let out = dir.path().join("out.pdf");
        let two = plan(vec![page(0, Vec::new()), page(0, Vec::new())], Vec::new());
        let refused = build_output(&pdfium, src.to_str().unwrap(), None, &two, &out);
        assert!(matches!(refused, Err(AppError::Signed(_))), "{refused:?}");

        let copy = SaveOptions {
            allow_signed_rewrite: true,
        };
        build_output_with(&pdfium, src.to_str().unwrap(), None, &two, &out, copy).expect("copy");
        let reports = check_document(&std::fs::read(&out).unwrap());
        assert!(
            reports
                .iter()
                .all(|r| !r.intact || !r.covers_whole_file || r.status == "modified"),
            "a rewritten copy cannot claim an intact, unmodified signature: {reports:#?}"
        );
    }
}

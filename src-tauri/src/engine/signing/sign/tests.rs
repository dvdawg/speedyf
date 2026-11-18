//! Signing fixtures with a fresh self-signed identity, then checking every
//! output three ways: our own check, `openssl cms -verify` on the extracted
//! bytes (skipped when the openssl tool is missing), and PDFium reopening it.
//! Set SPEEDYF_SIGN_OUT to a directory to keep the outputs for pyHanko.

use super::*;
use crate::engine::pdfium_init;
use crate::engine::signing::keys::{make_self_signed, BoxedSigner, SoftwareSigner};
use crate::engine::types::{EditPlan, PlanPage, PointDto};
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;

const NOW: i64 = 1_795_000_000;

fn identity(name: &str) -> (BoxedSigner, Vec<u8>) {
    let (pkcs8, cert) = make_self_signed(name, "ada@example.org", "Analytical Engines").unwrap();
    let signer = SoftwareSigner::from_pkcs8(&pkcs8, vec![cert.clone()]).unwrap();
    (Box::new(signer), cert)
}

/// One page (optionally rotated), a text field `name`, and an empty
/// signature field `Approver`.
fn fixture(rotate: i32) -> Vec<u8> {
    let objects: Vec<(u32, String)> = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R 6 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 7 0 R >> >> >> >>".into()),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into()),
        (3, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Rotate {rotate} /Resources << /Font << /Helv 7 0 R >> >> /Contents 4 0 R /Annots [5 0 R 6 0 R] >>")),
        (4, "<< /Length 44 >>\nstream\nBT /Helv 12 Tf 20 370 Td (Agreement) Tj ET\nendstream".into()),
        (5, "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Before) /F 4 /Rect [40 300 250 320] /DA (/Helv 10 Tf 0 g) /P 3 0 R >>".into()),
        (6, "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Approver) /F 4 /Rect [40 100 240 160] /P 3 0 R >>".into()),
        (7, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into()),
    ];
    let mut out: Vec<u8> = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (n, body) in &objects {
        offsets.push(out.len());
        out.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let start = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for at in offsets {
        out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// The same document written by lopdf with a cross-reference stream and
/// object streams, the shape of most modern PDFs.
fn stream_fixture() -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(&fixture(0)).unwrap();
    let mut out = Vec::new();
    doc.save_modern(&mut out).unwrap();
    assert!(
        !out.windows(5).any(|w| w == b"\nxref"),
        "fixture is not stream-xref"
    );
    out
}

fn request(field: FieldChoice) -> SignRequest {
    SignRequest {
        field,
        reason: "I approve this document".into(),
        location: "Berkeley".into(),
        contact: String::new(),
        date_text: "2026.11.17 10:00:00 -08'00'".into(),
        art: None,
        certify: None,
        lock: Lock::None,
        sub_filter: SubFilter::Cades,
        tsa_url: None,
    }
}

fn new_box() -> FieldChoice {
    FieldChoice::New {
        page: 0,
        rect: RectDto {
            x: 150.0,
            y: 200.0,
            w: 120.0,
            h: 40.0,
        },
    }
}

/// Every signature in the file: (byte range, CMS DER), oldest first.
fn signatures(bytes: &[u8]) -> Vec<([usize; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(p) = find(bytes, b"/ByteRange", at) {
        let open = find(bytes, b"[", p).unwrap();
        let close = find(bytes, b"]", open).unwrap();
        let nums: Vec<usize> = std::str::from_utf8(&bytes[open + 1..close])
            .unwrap()
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        let range: [usize; 4] = nums.try_into().unwrap();
        let hex = &bytes[range[1] + 1..range[2] - 1];
        let mut der: Vec<u8> = hex
            .chunks(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        der.truncate(cms::der_len(&der).unwrap());
        out.push((range, der));
        at = close;
    }
    out
}

fn openssl() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `openssl cms -verify` on one signature, trusting the signer's own
/// certificate. The signed content is the covered revision: the file up to
/// the end of the byte range, minus the contents.
fn openssl_verifies(file: &[u8], range: [usize; 4], der: &[u8], cert: &[u8]) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let [a, alen, b, blen] = range;
    let mut content = file[a..a + alen].to_vec();
    content.extend_from_slice(&file[b..b + blen]);
    std::fs::write(dir.path().join("content.bin"), &content).unwrap();
    std::fs::write(dir.path().join("sig.der"), der).unwrap();
    std::fs::write(dir.path().join("cert.der"), cert).unwrap();
    let pem = Command::new("openssl")
        .args(["x509", "-inform", "DER", "-in"])
        .arg(dir.path().join("cert.der"))
        .arg("-out")
        .arg(dir.path().join("cert.pem"))
        .status()
        .unwrap();
    assert!(pem.success());
    let out = Command::new("openssl")
        .args([
            "cms", "-verify", "-binary", "-inform", "DER", "-purpose", "any", "-in",
        ])
        .arg(dir.path().join("sig.der"))
        .arg("-content")
        .arg(dir.path().join("content.bin"))
        .arg("-CAfile")
        .arg(dir.path().join("cert.pem"))
        .args(["-out", "/dev/null"])
        .output()
        .unwrap();
    if !out.status.success() {
        eprintln!("openssl: {}", String::from_utf8_lossy(&out.stderr));
    }
    out.status.success()
}

/// Every signature in `file` verifies, with openssl when it is installed.
fn assert_all_verify(file: &[u8], cert: &[u8], expected: usize) {
    let sigs = signatures(file);
    assert_eq!(sigs.len(), expected, "signature count");
    for (range, der) in &sigs {
        // The revision this signature covers ends where its range ends.
        let covered = &file[..range[2] + range[3]];
        let mut hasher = Sha256::new();
        hasher.update(&covered[range[0]..range[0] + range[1]]);
        hasher.update(&covered[range[2]..range[2] + range[3]]);
        let digest: [u8; 32] = hasher.finalize().into();
        cms::verify_detached(der, &digest).expect("our own check");
        if openssl() {
            assert!(
                openssl_verifies(covered, *range, der, cert),
                "openssl cms -verify"
            );
        }
    }
}

fn keep(name: &str, bytes: &[u8], cert: &[u8]) {
    if let Ok(dir) = std::env::var("SPEEDYF_SIGN_OUT") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), bytes).unwrap();
        std::fs::write(dir.join("signer.der"), cert).unwrap();
    }
}

fn pdfium_opens(path: &Path) -> usize {
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .expect("PDFium reopens the signed file");
    doc.pages().len() as usize
}

#[test]
fn signs_a_new_field_and_every_byte_before_stays() {
    let (signer, cert) = identity("Ada Lovelace");
    let src = fixture(0);
    let out = sign_bytes(&src, &request(new_box()), signer.as_ref(), NOW).unwrap();
    assert_eq!(
        &out[..src.len()],
        &src[..],
        "the source must stay byte-identical"
    );
    assert!(out.windows(19).any(|w| w == b"ETSI.CAdES.detached"));
    assert!(out.windows(10).any(|w| w == b"Signature1"));
    assert_all_verify(&out, &cert, 1);
    keep("new-field.pdf", &out, &cert);

    let _guard = pdfium_init::test_guard();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signed.pdf");
    std::fs::write(&path, &out).unwrap();
    assert_eq!(pdfium_opens(&path), 1);
    // lopdf reads it back as a form with a signed field.
    let doc = lopdf::Document::load_mem(&out).unwrap();
    assert!(has_signatures(&doc));
    let form = terminal_fields(&doc);
    assert!(form.iter().any(|f| f.name == "Signature1" && f.signed));
}

#[test]
fn signs_an_existing_empty_field_in_the_older_form() {
    let (signer, cert) = identity("Charles Babbage");
    let src = fixture(0);
    let open = open_signature_fields(&src).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].name, "Approver");
    assert_eq!(open[0].page, 0);
    let mut req = request(FieldChoice::Existing {
        name: "Approver".into(),
    });
    req.sub_filter = SubFilter::Pkcs7;
    req.contact = "babbage@example.org".into();
    req.art = Some(ArtChoice {
        art: SignatureArt::Drawn {
            w: 100.0,
            h: 40.0,
            width: 2.0,
            strokes: vec![vec![
                PointDto { x: 5.0, y: 30.0 },
                PointDto { x: 30.0, y: 8.0 },
                PointDto { x: 60.0, y: 32.0 },
                PointDto { x: 95.0, y: 10.0 },
            ]],
        },
        color: "#1a2b6d".into(),
    });
    let out = sign_bytes(&src, &req, signer.as_ref(), NOW).unwrap();
    assert!(out.windows(19).any(|w| w == b"adbe.pkcs7.detached"));
    assert_all_verify(&out, &cert, 1);
    assert!(
        open_signature_fields(&out).unwrap().is_empty(),
        "the field is now signed"
    );
    // Signing it twice is refused.
    assert!(sign_bytes(
        &out,
        &request(FieldChoice::Existing {
            name: "Approver".into()
        }),
        signer.as_ref(),
        NOW
    )
    .is_err());
    keep("existing-field-pkcs7.pdf", &out, &cert);
}

#[test]
fn a_second_signature_keeps_the_first_valid() {
    let (first, cert_a) = identity("First Signer");
    let (second, cert_b) = identity("Second Signer");
    let once = sign_bytes(
        &fixture(0),
        &request(FieldChoice::Existing {
            name: "Approver".into(),
        }),
        first.as_ref(),
        NOW,
    )
    .unwrap();
    let twice = sign_bytes(&once, &request(new_box()), second.as_ref(), NOW + 60).unwrap();
    assert_eq!(&twice[..once.len()], &once[..]);
    let sigs = signatures(&twice);
    assert_eq!(sigs.len(), 2);
    if openssl() {
        let covered = &twice[..sigs[0].0[2] + sigs[0].0[3]];
        assert!(openssl_verifies(covered, sigs[0].0, &sigs[0].1, &cert_a));
        assert!(openssl_verifies(&twice, sigs[1].0, &sigs[1].1, &cert_b));
    }
    // A signed document cannot then be certified.
    let mut certify = request(new_box());
    certify.certify = Some(2);
    assert!(sign_bytes(&twice, &certify, first.as_ref(), NOW).is_err());
    keep("two-signatures.pdf", &twice, &cert_b);
}

#[test]
fn certifies_and_locks_fields() {
    let (signer, cert) = identity("Notary");
    let mut req = request(new_box());
    req.certify = Some(2);
    req.lock = Lock::Include {
        fields: vec!["name".into()],
    };
    let out = sign_bytes(&fixture(0), &req, signer.as_ref(), NOW).unwrap();
    assert_all_verify(&out, &cert, 1);
    let doc = lopdf::Document::load_mem(&out).unwrap();
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let catalog = doc.get_dictionary(root).unwrap();
    let perms = catalog.get(b"Perms").unwrap().as_dict().unwrap();
    let sig_id = perms.get(b"DocMDP").unwrap().as_reference().unwrap();
    let sig = doc.get_dictionary(sig_id).unwrap();
    let refs = sig.get(b"Reference").unwrap().as_array().unwrap();
    assert_eq!(refs.len(), 2);
    let methods: Vec<&[u8]> = refs
        .iter()
        .map(|r| {
            r.as_dict()
                .unwrap()
                .get(b"TransformMethod")
                .unwrap()
                .as_name()
                .unwrap()
        })
        .collect();
    assert_eq!(methods, vec![&b"DocMDP"[..], &b"FieldMDP"[..]]);
    let name = terminal_fields(&doc)
        .into_iter()
        .find(|f| f.name == "name")
        .unwrap();
    let ff = doc
        .get_dictionary(name.id)
        .unwrap()
        .get(b"Ff")
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(ff & 1, 1, "the locked field is read-only");
    let sig_field = terminal_fields(&doc)
        .into_iter()
        .find(|f| f.name == "Signature1")
        .unwrap();
    assert!(doc.get_dictionary(sig_field.id).unwrap().has(b"Lock"));
    keep("certified.pdf", &out, &cert);
}

#[test]
fn signs_a_stream_xref_file_and_a_rotated_page() {
    let (signer, cert) = identity("Modern Reader");
    let src = stream_fixture();
    let out = sign_bytes(&src, &request(new_box()), signer.as_ref(), NOW).unwrap();
    assert_all_verify(&out, &cert, 1);
    keep("stream-xref.pdf", &out, &cert);
    // The update is a stream too, and lopdf reads the result.
    lopdf::Document::load_mem(&out).expect("lopdf reads the signed stream-xref file");

    let mut req = request(new_box());
    req.art = Some(ArtChoice {
        art: SignatureArt::Typed {
            w: 0.0,
            h: 0.0,
            text: "Rotated".into(),
            style: "script".into(),
            d: String::new(),
        },
        color: "#000000".into(),
    });
    let rotated = sign_bytes(&fixture(90), &req, signer.as_ref(), NOW).unwrap();
    assert_all_verify(&rotated, &cert, 1);
    assert!(rotated
        .windows(14)
        .any(|w| w == b"/Matrix[0 1 -1" || w == b"/Matrix [0 1 -"));
    keep("rotated.pdf", &rotated, &cert);

    let _guard = pdfium_init::test_guard();
    let dir = tempfile::tempdir().unwrap();
    for (name, bytes) in [("s.pdf", &out), ("r.pdf", &rotated)] {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(pdfium_opens(&path), 1);
    }
}

/// The save path after signing: a form fill is appended by PDFium as one
/// more incremental update, and the signature still verifies over the
/// revision it signed.
#[test]
fn a_form_fill_after_signing_keeps_the_signature_valid() {
    let _guard = pdfium_init::test_guard();
    let (signer, cert) = identity("Before The Fill");
    let signed = sign_bytes(&fixture(0), &request(new_box()), signer.as_ref(), NOW).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("signed.pdf");
    let filled = dir.path().join("filled.pdf");
    std::fs::write(&src, &signed).unwrap();

    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
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
        form: vec![("name".to_string(), "After".to_string())],
        form_buttons: Vec::new(),
        form_choices: Vec::new(),
        form_display: vec![("name".to_string(), "After (shown)".to_string(), None)],
        scrub: Default::default(),
    };
    crate::engine::save::build_output(&pdfium, src.to_str().unwrap(), None, &plan, &filled)
        .unwrap();
    let after = std::fs::read(&filled).unwrap();
    assert!(after.len() > signed.len(), "the fill was appended");
    assert_eq!(
        &after[..signed.len()],
        &signed[..],
        "the signed revision is untouched"
    );
    assert_all_verify(&after, &cert, 1);
    keep("signed-then-filled.pdf", &after, &cert);
    let doc = pdfium.load_pdf_from_file(&filled, None).unwrap();
    assert_eq!(
        doc.form().unwrap().field_values(doc.pages()).get("name"),
        Some(&Some("After".into()))
    );
}

#[test]
fn refuses_what_it_cannot_sign() {
    let (signer, _) = identity("Careful");
    let src = fixture(0);
    let missing = request(FieldChoice::Existing {
        name: "Nobody".into(),
    });
    assert!(sign_bytes(&src, &missing, signer.as_ref(), NOW).is_err());
    let text = request(FieldChoice::Existing {
        name: "name".into(),
    });
    assert!(sign_bytes(&src, &text, signer.as_ref(), NOW).is_err());
    let tiny = request(FieldChoice::New {
        page: 0,
        rect: RectDto {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
    });
    assert!(sign_bytes(&src, &tiny, signer.as_ref(), NOW).is_err());
    let far = request(FieldChoice::New {
        page: 3,
        rect: RectDto {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 40.0,
        },
    });
    assert!(sign_bytes(&src, &far, signer.as_ref(), NOW).is_err());
    // A corrupted signature is caught by the check.
    let mut out = sign_bytes(&src, &request(new_box()), signer.as_ref(), NOW).unwrap();
    let at = out.len() - 40;
    out[at] ^= 0x01;
    assert!(check(&out).is_err());
}

#[test]
fn dates_are_utc() {
    assert_eq!(pdf_date(0), "D:19700101000000Z");
    assert_eq!(pdf_date(1_795_000_000), "D:20261118110640Z");
    assert_eq!(civil_from_days(-1), (1969, 12, 31));
}

/// RFC 3161 against a local, offline time-stamping authority: openssl
/// answers the request, and the token lands in the CMS where openssl and
/// our own check still verify the signature.
#[test]
fn a_timestamp_token_is_requested_checked_and_embedded() {
    if !openssl() {
        eprintln!("skipping: no openssl");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let ok = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:P-256",
            "-nodes",
        ])
        .arg("-keyout")
        .arg(d.join("tsa.key"))
        .arg("-out")
        .arg(d.join("tsa.crt"))
        .args([
            "-subj",
            "/CN=Test TSA",
            "-days",
            "2",
            "-addext",
            "extendedKeyUsage=critical,timeStamping",
        ])
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    std::fs::write(d.join("serial"), "01\n").unwrap();
    let cfg = format!(
        "[ tsa ]\ndefault_tsa = tsa1\n[ tsa1 ]\nserial = {s}\nsigner_cert = {c}\nsigner_key = {k}\nsigner_digest = sha256\ndefault_policy = 1.2.3.4.1\ndigests = sha256\naccuracy = secs:1\ness_cert_id_alg = sha256\ncerts = {c}\n",
        s = d.join("serial").display(),
        c = d.join("tsa.crt").display(),
        k = d.join("tsa.key").display(),
    );
    std::fs::write(d.join("tsa.cnf"), cfg).unwrap();

    let (signer, cert) = identity("Timely");
    let src = fixture(0);
    let mut req = request(new_box());
    req.tsa_url = Some("http://127.0.0.1/unused".into());
    let chain_bytes: usize = signer.chain().iter().map(Vec::len).sum();
    let mut prepared = prepare(&src, &req, "Timely", chain_bytes, NOW).unwrap();
    let mut parts = cms::build(signer.as_ref(), &prepared.digest(), SubFilter::Cades, NOW).unwrap();

    let nonce = 0x8123_4567_89ab_cdefu64;
    std::fs::write(
        d.join("req.tsq"),
        tsa::request(parts.signature(), nonce).unwrap(),
    )
    .unwrap();
    let reply = Command::new("openssl")
        .args(["ts", "-reply", "-config"])
        .arg(d.join("tsa.cnf"))
        .args(["-section", "tsa1", "-queryfile"])
        .arg(d.join("req.tsq"))
        .arg("-out")
        .arg(d.join("resp.tsr"))
        .output()
        .unwrap();
    assert!(
        reply.status.success(),
        "{}",
        String::from_utf8_lossy(&reply.stderr)
    );
    let resp = std::fs::read(d.join("resp.tsr")).unwrap();
    // The wrong nonce or the wrong signature is refused.
    assert!(tsa::token_from_response(&resp, parts.signature(), nonce + 1).is_err());
    assert!(tsa::token_from_response(&resp, b"something else", nonce).is_err());
    let token = tsa::token_from_response(&resp, parts.signature(), nonce).unwrap();

    parts.add_timestamp(&token).unwrap();
    embed(&mut prepared, &parts.to_der().unwrap()).unwrap();
    check(&prepared.bytes).unwrap();
    assert_all_verify(&prepared.bytes, &cert, 1);
    keep("timestamped.pdf", &prepared.bytes, &cert);
    std::fs::copy(
        d.join("tsa.crt"),
        std::env::var("SPEEDYF_SIGN_OUT")
            .map(|o| PathBuf::from(o).join("tsa.crt"))
            .unwrap_or_else(|_| d.join("tsa-copy.crt")),
    )
    .unwrap();
}

/// The request exactly as the dialog sends it (certificate/api.ts).
#[test]
fn reads_the_request_the_dialog_sends() {
    let json = r##"{
        "field": { "kind": "new", "page": 1, "rect": { "x": 10, "y": 20, "w": 150, "h": 50 } },
        "reason": "Approved", "location": "", "contact": "",
        "dateText": "2026.01.02 00:00:00 +00'00'",
        "art": { "art": { "type": "drawn", "w": 10, "h": 5, "width": 1,
                          "strokes": [[{ "x": 1, "y": 1 }, { "x": 9, "y": 4 }]] },
                 "color": "#1a1a1a" },
        "certify": 2,
        "lock": { "kind": "include", "fields": ["name"] },
        "subFilter": "pkcs7",
        "tsaUrl": null
    }"##;
    let req: SignRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(req.field, FieldChoice::New { page: 1, .. }));
    assert_eq!(req.certify, Some(2));
    assert_eq!(
        req.lock,
        Lock::Include {
            fields: vec!["name".into()]
        }
    );
    assert_eq!(req.sub_filter, SubFilter::Pkcs7);
    assert!(req.tsa_url.is_none());
    assert!(req.art.is_some());
    let existing: SignRequest = serde_json::from_str(
        r#"{ "field": { "kind": "existing", "name": "Approver" }, "lock": { "kind": "all" }, "subFilter": "cades" }"#,
    )
    .unwrap();
    assert!(matches!(existing.field, FieldChoice::Existing { .. }));
    assert_eq!(existing.lock, Lock::All);
}

#[path = "mdp_tests.rs"]
mod mdp;

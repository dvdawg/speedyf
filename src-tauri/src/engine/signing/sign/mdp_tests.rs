//! What a later save does to a signature, as Acrobat-style checkers judge
//! it: sign a fixture (approval, or certified at DocMDP 2 or 3), make an
//! ordinary edit through the normal save path - fill a field, add a
//! comment, place a visual signature - and check that
//!
//! - every signature still verifies (our CMS check, and `openssl cms
//!   -verify` when openssl is installed);
//! - the update holds only what the edit changed: no page, page tree,
//!   catalog, content stream or signature appearance written back;
//! - our own change report (`changes.rs`) calls it what it is, and allowed;
//! - pyHanko agrees, when `SPEEDYF_PYHANKO` names a Python with pyhanko
//!   installed (dev only; skipped otherwise): modification level
//!   FORM_FILLING and DocMDP intact for a fill. pyHanko's default policy
//!   (0.38) has no rule that explains a new comment - its ANNOTATIONS level
//!   is unreachable, see the TODO in its form_field_rules.py - so any
//!   comment reads as OTHER there. For comments the check is instead that
//!   the only objects pyHanko cannot explain are the page (its /Annots) and
//!   the new annotation objects: nothing else was touched.

use super::*;
use crate::engine::save::build_output;
use crate::engine::signing::changes::ChangeKind;
use crate::engine::signing::check_document;
use crate::engine::types::{PlanAnnot, PlanFormButton};

/// What an edit should look like to a checker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    FormFilling,
    Annotations,
}

struct Case {
    name: &'static str,
    source: Vec<u8>,
    certify: Option<u8>,
    plan: EditPlan,
    expect: Expect,
}

fn plan_with(annots: Vec<PlanAnnot>, form: Vec<(String, String)>) -> EditPlan {
    EditPlan {
        pages: vec![PlanPage {
            src_index: Some(0),
            width_pt: 300.0,
            height_pt: 400.0,
            rotation: 0,
            annots,
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }],
        form,
        form_buttons: Vec::<PlanFormButton>::new(),
        form_choices: Vec::new(),
        form_display: Vec::new(),
        scrub: Default::default(),
        form_design: None,
    }
}

fn fill() -> EditPlan {
    plan_with(Vec::new(), vec![("name".into(), "Filled after".into())])
}

/// A fill whose field shows a formatted value: the extra appearance passes
/// (`forms::appearance`) run before and after PDFium's.
fn formatted_fill() -> EditPlan {
    let mut plan = fill();
    plan.form = vec![("name".into(), "-1234.5".into())];
    plan.form_display = vec![("name".into(), "(1,234.50)".into(), Some(true))];
    plan
}

fn comment() -> EditPlan {
    plan_with(
        vec![PlanAnnot {
            kind: "highlight".into(),
            rect: RectDto {
                x: 15.0,
                y: 365.0,
                w: 70.0,
                h: 16.0,
            },
            color: "#ffd400".into(),
            opacity: 0.4,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
            signature: None,
            flatten: false,
            mark: None,
        }],
        Vec::new(),
    )
}

fn visual_signature() -> EditPlan {
    let p = |x: f32, y: f32| PointDto { x, y };
    plan_with(
        vec![PlanAnnot {
            kind: "signature".into(),
            rect: RectDto {
                x: 40.0,
                y: 20.0,
                w: 120.0,
                h: 36.0,
            },
            color: "#1a1a1a".into(),
            opacity: 1.0,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
            signature: Some(crate::engine::types::SignatureArt::Drawn {
                w: 200.0,
                h: 60.0,
                strokes: vec![vec![p(10.0, 40.0), p(40.0, 10.0), p(70.0, 50.0)]],
                width: 4.0,
            }),
            flatten: false,
            mark: None,
        }],
        Vec::new(),
    )
}

/// The fixture with an XFA stream beside its AcroForm, as IRS forms carry:
/// the save drops it in an update of its own.
fn xfa_fixture() -> Vec<u8> {
    let mut doc = lopdf::Document::load_mem(&fixture(0)).unwrap();
    let xfa = doc.add_object(lopdf::Stream::new(
        lopdf::Dictionary::new(),
        b"<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\"/>".to_vec(),
    ));
    let form = doc
        .get_object_mut((1, 0))
        .and_then(Object::as_dict_mut)
        .unwrap()
        .get_mut(b"AcroForm")
        .and_then(Object::as_dict_mut)
        .unwrap();
    form.set("XFA", Object::Reference(xfa));
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn cases() -> Vec<Case> {
    let case = |name, source, certify, plan, expect| Case {
        name,
        source,
        certify,
        plan,
        expect,
    };
    use Expect::*;
    vec![
        case("approval-fill", fixture(0), None, fill(), FormFilling),
        case("certified2-fill", fixture(0), Some(2), fill(), FormFilling),
        case(
            "certified2-fill-xref-stream",
            stream_fixture(),
            Some(2),
            fill(),
            FormFilling,
        ),
        case(
            "certified2-fill-formatted",
            fixture(0),
            Some(2),
            formatted_fill(),
            FormFilling,
        ),
        case(
            "certified2-fill-xfa",
            xfa_fixture(),
            Some(2),
            fill(),
            FormFilling,
        ),
        case(
            "certified3-comment",
            fixture(0),
            Some(3),
            comment(),
            Annotations,
        ),
        case(
            "certified3-comment-xref-stream",
            stream_fixture(),
            Some(3),
            comment(),
            Annotations,
        ),
        case(
            "certified3-visual-signature",
            fixture(0),
            Some(3),
            visual_signature(),
            Annotations,
        ),
        case("approval-comment", fixture(0), None, comment(), Annotations),
    ]
}

/// What pyHanko says about the first signature.
#[derive(Debug)]
struct PyHanko {
    level: String,
    docmdp_ok: bool,
    /// Object numbers it found changed and could not explain.
    unexplained: Vec<u32>,
}

/// Ask pyHanko, or `None` when no pyHanko is configured.
fn pyhanko(path: &Path) -> Option<PyHanko> {
    let python = std::env::var("SPEEDYF_PYHANKO").ok()?;
    const SCRIPT: &str = "
import sys, re, logging
logging.disable(logging.CRITICAL)
from pyhanko.pdf_utils.reader import PdfFileReader
from pyhanko.sign.validation import validate_pdf_signature
from pyhanko.sign.diff_analysis import DEFAULT_DIFF_POLICY
with open(sys.argv[1], 'rb') as f:
    r = PdfFileReader(f, strict=False)
    sig = r.embedded_signatures[0]
    st = validate_pdf_signature(sig)
    try:
        res = DEFAULT_DIFF_POLICY.review_file(r, sig.signed_revision)
        msg = '' if hasattr(res, 'modification_level') else str(res)
    except Exception as e:
        msg = str(e)
    refs = re.findall(r'idnum=([0-9]+)', msg.split(chr(10))[0])
    print(st.modification_level.name, st.docmdp_ok, ','.join(refs) or '-')
";
    let out = Command::new(python)
        .args(["-I", "-c", SCRIPT])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!("pyhanko: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut words = text.split_whitespace();
    Some(PyHanko {
        level: words.next()?.to_string(),
        docmdp_ok: words.next()? == "True",
        unexplained: words
            .next()?
            .split(',')
            .filter_map(|n| n.parse().ok())
            .collect(),
    })
}

/// Object numbers written in `bytes[from..]`.
fn objects_written(bytes: &[u8], from: usize) -> Vec<u32> {
    let text = String::from_utf8_lossy(&bytes[from..]);
    let tokens: Vec<&str> = text.split_ascii_whitespace().collect();
    tokens
        .windows(3)
        .filter(|w| w[2] == "obj" && w[1].parse::<u32>().is_ok())
        .filter_map(|w| w[0].parse().ok())
        .collect()
}

/// Objects of the signed revision that must never be written again by a
/// fill or a comment: catalog, page tree, content stream, the signature
/// widget and its appearance. A page may be (its /Annots grows for a
/// comment), but not for a fill.
fn untouchable(signed: &[u8], expect: Expect) -> HashSet<u32> {
    let doc = lopdf::Document::load_mem(signed).unwrap();
    let mut ids: HashSet<u32> = HashSet::new();
    if let Ok(Object::Reference(root)) = doc.trailer.get(b"Root") {
        ids.insert(root.0);
    }
    let pages = doc.get_pages();
    for page in pages.values() {
        if expect == Expect::FormFilling {
            ids.insert(page.0);
        }
        let dict = doc.get_dictionary(*page).unwrap();
        if let Ok(parent) = dict.get(b"Parent").and_then(Object::as_reference) {
            ids.insert(parent.0);
        }
        if let Ok(contents) = dict.get(b"Contents").and_then(Object::as_reference) {
            ids.insert(contents.0);
        }
    }
    for field in terminal_fields(&doc).iter().filter(|f| f.signed) {
        if let Some(widget) = widget_of(&doc, field.id) {
            ids.insert(widget.0);
            let ap = doc
                .get_dictionary(widget)
                .ok()
                .and_then(|w| w.get(b"AP").ok())
                .and_then(|ap| match ap {
                    Object::Dictionary(d) => d.get(b"N").ok(),
                    _ => None,
                })
                .and_then(|n| n.as_reference().ok());
            if let Some(ap) = ap {
                ids.insert(ap.0);
            }
        }
    }
    ids
}

use std::collections::HashSet;

#[test]
fn edits_after_signing_read_as_form_fill_or_comments_to_every_checker() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let mut table = Vec::new();
    for case in cases() {
        let (signer, cert) = identity("Certifier");
        let mut req = request(new_box());
        req.certify = case.certify;
        let signed = sign_bytes(&case.source, &req, signer.as_ref(), NOW).unwrap();
        let src = dir.path().join(format!("{}-signed.pdf", case.name));
        let out = dir.path().join(format!("{}.pdf", case.name));
        std::fs::write(&src, &signed).unwrap();
        build_output(&pdfium, src.to_str().unwrap(), None, &case.plan, &out)
            .unwrap_or_else(|e| panic!("{}: save failed: {e:?}", case.name));
        let after = std::fs::read(&out).unwrap();
        keep(&format!("{}.pdf", case.name), &after, &cert);

        assert_eq!(&after[..signed.len()], &signed[..], "{}: prefix", case.name);
        assert!(
            after.len() > signed.len(),
            "{}: nothing appended",
            case.name
        );
        assert_all_verify(&after, &cert, 1);

        // Only what changed was written.
        let written = objects_written(&after, signed.len());
        let forbidden = untouchable(&signed, case.expect);
        let rewritten: Vec<u32> = written
            .iter()
            .copied()
            .filter(|n| forbidden.contains(n))
            .collect();
        assert!(
            rewritten.is_empty(),
            "{}: the update rewrote {rewritten:?} (all written: {written:?})",
            case.name
        );

        // Our own report: intact, and every change of the expected kind and
        // allowed.
        let report = check_document(&after);
        assert_eq!(report.len(), 1, "{}", case.name);
        let ours = &report[0];
        assert!(ours.intact, "{}", case.name);
        assert_ne!(ours.status, "modified", "{}: {:?}", case.name, ours.changes);
        let kind = match case.expect {
            Expect::FormFilling => ChangeKind::FormFill.as_str(),
            Expect::Annotations => ChangeKind::Comment.as_str(),
        };
        assert!(
            ours.changes.iter().all(|c| c.allowed),
            "{}: {:?}",
            case.name,
            ours.changes
        );
        assert!(
            ours.changes.iter().any(|c| c.kind == kind),
            "{}: expected a {kind} change, got {:?}",
            case.name,
            ours.changes
        );

        // pyHanko, when available.
        let theirs = pyhanko(&out);
        match (&theirs, case.expect) {
            (Some(p), Expect::FormFilling) => {
                assert_eq!(p.level, "FORM_FILLING", "{}: pyHanko {p:?}", case.name);
                assert!(p.docmdp_ok, "{}: pyHanko {p:?}", case.name);
            }
            (Some(p), Expect::Annotations) => {
                let signed_doc = lopdf::Document::load_mem(&signed).unwrap();
                let pages: HashSet<u32> = signed_doc.get_pages().values().map(|id| id.0).collect();
                let stray: Vec<u32> = p
                    .unexplained
                    .iter()
                    .copied()
                    .filter(|n| signed_doc.objects.contains_key(&(*n, 0)) && !pages.contains(n))
                    .collect();
                assert!(
                    stray.is_empty(),
                    "{}: pyHanko found signed objects other than the page changed: {p:?}",
                    case.name
                );
            }
            (None, _) => {}
        }
        table.push(format!(
            "{:32} written={:?} ours={} pyhanko={:?}",
            case.name, written, ours.status, theirs
        ));
    }
    eprintln!("{}", table.join("\n"));
}

/// The other side: a comment on a document certified for form filling only
/// is not allowed, and both checkers say so.
#[test]
fn a_comment_on_a_form_fill_certification_is_flagged() {
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
    let dir = tempfile::tempdir().unwrap();
    let (signer, cert) = identity("Strict Certifier");
    let mut req = request(new_box());
    req.certify = Some(2);
    let signed = sign_bytes(&fixture(0), &req, signer.as_ref(), NOW).unwrap();
    let src = dir.path().join("signed.pdf");
    let out = dir.path().join("commented.pdf");
    std::fs::write(&src, &signed).unwrap();
    build_output(&pdfium, src.to_str().unwrap(), None, &comment(), &out).unwrap();
    let after = std::fs::read(&out).unwrap();
    keep("certified2-comment.pdf", &after, &cert);
    assert_all_verify(&after, &cert, 1);
    let report = check_document(&after);
    assert_eq!(report[0].status, "modified", "{:#?}", report[0]);
    assert!(report[0]
        .changes
        .iter()
        .any(|c| c.kind == "comment" && !c.allowed));
    if let Some(p) = pyhanko(&out) {
        assert!(!p.docmdp_ok, "pyHanko should refuse a comment under P=2");
    }
}

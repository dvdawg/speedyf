//! Saving protected documents through the worker: protection is kept,
//! changed only with the permissions password, and reported on open.

use super::*;
use crate::engine::protect::tests::{
    fixture, page_texts, protection_of, restricted, set, test_pdfium, PAGE_TEXT,
};
use crate::engine::protect::{PermissionSet, ProtectRequest};

fn quiet(pdfium: &Pdfium) -> WorkerState<'_> {
    WorkerState::new(
        pdfium.bindings(),
        pdfium,
        OcrService::new(Arc::new(|_: &[u8]| None), Arc::new(|_| {})),
        false,
    )
}

/// Both fixture pages kept, a highlight added to the first.
fn highlight_plan() -> EditPlan {
    let page = |i: u32, annots: serde_json::Value| {
        serde_json::json!({
            "srcIndex": i, "widthPt": 300.0, "heightPt": 400.0, "rotation": 0,
            "annots": annots, "texts": [], "images": []
        })
    };
    let highlight = serde_json::json!([{
        "kind": "highlight",
        "rect": { "x": 30.0, "y": 290.0, "w": 120.0, "h": 24.0 },
        "color": "#ffd400", "opacity": 0.4
    }]);
    serde_json::from_value(serde_json::json!({
        "pages": [page(0, highlight), page(1, serde_json::json!([]))],
        "form": []
    }))
    .expect("plan")
}

fn annotation_count(pdfium: &Pdfium, path: &Path, password: Option<&str>) -> usize {
    let document = pdfium.load_pdf_from_file(path, password).expect("opens");
    let page = document.pages().get(0).expect("page");
    page.annotations().len()
}

/// The fixture, protected with an open password "user" and a permissions
/// password "owner" that withholds most rights.
fn protected_fixture(state: &mut WorkerState, dir: &Path) -> PathBuf {
    let plain = dir.join("plain.pdf");
    let src = dir.join("protected.pdf");
    fixture(state.pdfium, &plain);
    let doc = do_open(state, plain.to_str().unwrap().into(), None)
        .unwrap()
        .doc_id;
    let saved = do_save(
        state,
        doc,
        highlight_plan(),
        src.clone(),
        set("user", "owner", restricted()),
        false,
    )
    .expect("protect");
    assert_eq!(saved.reopen_password.as_deref(), Some("owner"));
    src
}

#[test]
fn a_protected_document_reports_its_protection_on_open() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let mut state = quiet(&pdfium);
    let src = protected_fixture(&mut state, dir.path());

    let plain = do_open(
        &mut state,
        dir.path().join("plain.pdf").to_str().unwrap().into(),
        None,
    )
    .unwrap();
    assert!(!plain.protection.encrypted);

    let user = do_open(
        &mut state,
        src.to_str().unwrap().into(),
        Some("user".into()),
    )
    .unwrap();
    let p = &user.protection;
    assert!(p.encrypted && p.open_password && !p.owner_unlocked && p.restricted());
    assert_eq!((p.revision, p.method.as_str()), (6, "AES-256"));
    assert_eq!(p.permissions, restricted());

    let owner = do_open(
        &mut state,
        src.to_str().unwrap().into(),
        Some("owner".into()),
    )
    .unwrap();
    assert!(owner.protection.owner_unlocked && owner.protection.open_password);
}

#[test]
fn saving_keeps_the_protection_it_was_opened_with() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let mut state = quiet(&pdfium);
    let src = protected_fixture(&mut state, dir.path());
    let out = dir.path().join("kept.pdf");

    let doc = do_open(
        &mut state,
        src.to_str().unwrap().into(),
        Some("user".into()),
    )
    .unwrap()
    .doc_id;
    let saved = do_save(
        &mut state,
        doc,
        highlight_plan(),
        out.clone(),
        ProtectRequest::Keep,
        false,
    )
    .expect("save");
    assert_eq!(saved.reopen_password.as_deref(), Some("user"));
    assert_eq!(page_texts(&pdfium, &out, Some("user")).unwrap(), PAGE_TEXT);
    assert!(
        page_texts(&pdfium, &out, None).is_err(),
        "still asks for a password"
    );
    let kept = protection_of(&pdfium, &out, Some("user")).unwrap();
    assert_eq!((kept.revision, kept.permissions), (6, restricted()));
    assert!(
        protection_of(&pdfium, &out, Some("owner"))
            .unwrap()
            .owner_unlocked
    );
    assert_eq!(
        annotation_count(&pdfium, &out, Some("user")),
        2,
        "both highlights"
    );

    // Over its own file, too.
    let saved = do_save(
        &mut state,
        doc,
        highlight_plan(),
        src.clone(),
        ProtectRequest::Keep,
        false,
    )
    .expect("save in place");
    let reopened = do_open(
        &mut state,
        src.to_str().unwrap().into(),
        saved.reopen_password.clone(),
    )
    .unwrap();
    assert!(reopened.protection.encrypted && reopened.protection.restricted());
}

#[test]
fn changing_a_restricted_document_takes_its_permissions_password() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let mut state = quiet(&pdfium);
    let src = protected_fixture(&mut state, dir.path());
    let out = dir.path().join("open.pdf");

    let doc = do_open(
        &mut state,
        src.to_str().unwrap().into(),
        Some("user".into()),
    )
    .unwrap()
    .doc_id;
    let refused = do_save(
        &mut state,
        doc,
        highlight_plan(),
        out.clone(),
        ProtectRequest::Remove,
        false,
    );
    assert!(matches!(refused, Err(AppError::Unsupported(_))));
    assert!(!out.exists());

    assert!(matches!(
        do_unlock_permissions(&mut state, doc, "user".into()),
        Err(AppError::PasswordRequired)
    ));
    let unlocked = do_unlock_permissions(&mut state, doc, "owner".into()).unwrap();
    assert!(unlocked.owner_unlocked && unlocked.open_password);

    let saved = do_save(
        &mut state,
        doc,
        highlight_plan(),
        out.clone(),
        ProtectRequest::Remove,
        false,
    )
    .expect("remove");
    assert_eq!(saved.reopen_password, None);
    assert_eq!(page_texts(&pdfium, &out, None).unwrap(), PAGE_TEXT);
    assert!(!protection_of(&pdfium, &out, None).unwrap().encrypted);
}

#[test]
fn new_protection_replaces_the_old() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let mut state = quiet(&pdfium);
    let src = protected_fixture(&mut state, dir.path());
    let out = dir.path().join("changed.pdf");

    let doc = do_open(
        &mut state,
        src.to_str().unwrap().into(),
        Some("owner".into()),
    )
    .unwrap()
    .doc_id;
    let saved = do_save(
        &mut state,
        doc,
        highlight_plan(),
        out.clone(),
        set("new open", "", PermissionSet::all()),
        false,
    )
    .expect("change");
    assert_eq!(saved.reopen_password.as_deref(), Some("new open"));
    assert!(page_texts(&pdfium, &out, Some("user")).is_err());
    assert!(page_texts(&pdfium, &out, Some("owner")).is_err());
    let now = protection_of(&pdfium, &out, Some("new open")).unwrap();
    assert!(now.encrypted && now.open_password && !now.restricted());
}

use super::*;
use crate::engine::pdfium_init;
use crate::engine::save::build_output;
use crate::engine::types::EditPlan;

pub(crate) fn test_pdfium() -> Pdfium {
    Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
}

pub(crate) const PAGE_TEXT: [&str; 2] = ["PROTECT-ONE", "PROTECT-TWO"];

/// Two pages of text, unencrypted, written by PDFium.
pub(crate) fn fixture(pdfium: &Pdfium, path: &Path) {
    let mut document = pdfium.create_new_pdf().expect("create fixture");
    let font = document.fonts_mut().helvetica();
    for label in PAGE_TEXT {
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(300.0),
                PdfPoints::new(400.0),
            ))
            .expect("page");
        let mut text =
            PdfPageTextObject::new(&document, label, font, PdfPoints::new(20.0)).expect("text");
        text.translate(PdfPoints::new(30.0), PdfPoints::new(300.0))
            .expect("move");
        page.objects_mut().add_text_object(text).expect("add text");
    }
    document.save_to_file(path).expect("save fixture");
}

/// The text of every page, or why the file would not open.
pub(crate) fn page_texts(
    pdfium: &Pdfium,
    path: &Path,
    password: Option<&str>,
) -> Result<Vec<String>, PdfiumError> {
    let document = pdfium.load_pdf_from_file(path, password)?;
    Ok(document
        .pages()
        .iter()
        .map(|page| page.text().map(|t| t.all()).unwrap_or_default())
        .map(|text| text.trim().to_owned())
        .collect())
}

/// Open `path` with PDFium's own API and read its protection.
pub(crate) fn protection_of(
    pdfium: &Pdfium,
    path: &Path,
    password: Option<&str>,
) -> Option<ProtectionDto> {
    let b = pdfium.bindings();
    let raw = b.FPDF_LoadDocument(path.to_str().unwrap(), password);
    if raw.is_null() {
        return None;
    }
    let open = needs_open_password(b, path.to_str().unwrap(), password);
    let protection = read(b, raw, open);
    b.FPDF_CloseDocument(raw);
    Some(protection)
}

pub(crate) fn set(open: &str, owner: &str, permissions: PermissionSet) -> ProtectRequest {
    ProtectRequest::Set {
        open_password: open.into(),
        permissions_password: owner.into(),
        permissions,
    }
}

pub(crate) fn restricted() -> PermissionSet {
    PermissionSet {
        print: PrintLevel::Low,
        copy: false,
        edit: false,
        annotate: false,
        fill_forms: true,
        assemble: false,
        accessibility: true,
    }
}

/// Save `src` (opened with `password`) to `out` the way the worker does:
/// build the output, then protect it.
fn save_protected(
    pdfium: &Pdfium,
    src: &Path,
    password: Option<&str>,
    request: &ProtectRequest,
    out: &Path,
) {
    let encrypted = protection_of(pdfium, src, password)
        .expect("source opens")
        .encrypted;
    let target = plan(request, src, password, encrypted).expect("plan");
    let pages = pdfium
        .load_pdf_from_file(src, password)
        .expect("source opens")
        .pages()
        .len();
    let plan = whole(pages.into());
    build_output(pdfium, src.to_str().unwrap(), password, &plan, out).expect("build");
    encrypt_file(out, &target).expect("encrypt");
}

/// A plan keeping `pages` pages as they are.
pub(crate) fn whole(pages: u32) -> EditPlan {
    let pages: Vec<serde_json::Value> = (0..pages)
        .map(|i| {
            serde_json::json!({
                "srcIndex": i, "widthPt": 300.0, "heightPt": 400.0, "rotation": 0,
                "annots": [], "texts": [], "images": []
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({ "pages": pages, "form": [] })).expect("plan")
}

fn encrypt_dictionary(path: &Path, password: &str) -> Dictionary {
    let doc = Document::load_with_options(path, LoadOptions::with_password(password)).unwrap();
    doc.encryption_state.expect("encrypted").encode().unwrap()
}

#[test]
fn aes256_round_trip_keeps_pages_and_text_and_refuses_wrong_passwords() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &src);
    save_protected(
        &pdfium,
        &src,
        None,
        &set("open sesame", "owner", restricted()),
        &out,
    );

    let bytes = std::fs::read(&out).unwrap();
    assert!(
        !bytes.windows(11).any(|w| w == b"PROTECT-ONE"),
        "no page text in the clear"
    );
    assert_eq!(
        page_texts(&pdfium, &out, Some("open sesame")).unwrap(),
        PAGE_TEXT
    );
    assert_eq!(page_texts(&pdfium, &out, Some("owner")).unwrap(), PAGE_TEXT);
    assert!(matches!(
        page_texts(&pdfium, &out, None),
        Err(PdfiumError::PdfiumLibraryInternalError(
            PdfiumInternalError::PasswordError
        ))
    ));
    assert!(page_texts(&pdfium, &out, Some("open sesame!")).is_err());

    let dict = encrypt_dictionary(&out, "open sesame");
    assert_eq!(dict.get(b"V").unwrap().as_i64().unwrap(), 5);
    assert_eq!(dict.get(b"R").unwrap().as_i64().unwrap(), 6);
    for key in [&b"O"[..], b"U", b"OE", b"UE", b"Perms"] {
        assert!(dict.has(key), "{}", String::from_utf8_lossy(key));
    }
    let cfm = dict
        .get(b"CF")
        .and_then(Object::as_dict)
        .and_then(|cf| cf.get(b"StdCF"))
        .and_then(Object::as_dict)
        .and_then(|f| f.get(b"CFM"))
        .and_then(Object::as_name)
        .unwrap();
    assert_eq!(cfm, b"AESV3");
}

#[test]
fn permissions_read_back_and_the_owner_password_unlocks_them() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &src);
    save_protected(
        &pdfium,
        &src,
        None,
        &set("user", "owner", restricted()),
        &out,
    );

    let as_user = protection_of(&pdfium, &out, Some("user")).unwrap();
    assert!(as_user.encrypted && as_user.open_password);
    assert_eq!(as_user.revision, 6);
    assert_eq!(as_user.method, "AES-256");
    assert!(!as_user.owner_unlocked);
    assert_eq!(as_user.permissions, restricted());
    assert!(as_user.restricted());

    let as_owner = protection_of(&pdfium, &out, Some("owner")).unwrap();
    assert!(as_owner.owner_unlocked && as_owner.open_password);
    assert!(!as_owner.restricted());
    assert!(is_owner_password(
        pdfium.bindings(),
        out.to_str().unwrap(),
        "owner"
    ));
    assert!(!is_owner_password(
        pdfium.bindings(),
        out.to_str().unwrap(),
        "user"
    ));
}

#[test]
fn a_permissions_password_alone_opens_without_asking() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &src);
    save_protected(&pdfium, &src, None, &set("", "owner", restricted()), &out);

    assert_eq!(page_texts(&pdfium, &out, None).unwrap(), PAGE_TEXT);
    let anyone = protection_of(&pdfium, &out, None).unwrap();
    assert!(anyone.encrypted && !anyone.open_password && anyone.restricted());
    let owner = protection_of(&pdfium, &out, Some("owner")).unwrap();
    assert!(owner.owner_unlocked && !owner.open_password);
}

#[test]
fn an_open_password_alone_grants_every_permission() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &src);
    // The permissions asked for are ignored: nobody could ever lift them.
    save_protected(&pdfium, &src, None, &set("only", "", restricted()), &out);
    let user = protection_of(&pdfium, &out, Some("only")).unwrap();
    assert!(user.open_password && !user.restricted());
}

#[test]
fn impossible_requests_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("none.pdf");
    let all = PermissionSet::all();
    assert!(plan(&set("", "", all), &src, None, false).is_err());
    assert!(plan(&set("same", "same", all), &src, None, false).is_err());
}

#[test]
fn keeping_an_aes256_source_reuses_its_handler() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.pdf");
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &plain);
    save_protected(
        &pdfium,
        &plain,
        None,
        &set("user", "owner", restricted()),
        &src,
    );

    // Opened with the open password only: the permissions password is never
    // known, yet it still works on the output.
    save_protected(&pdfium, &src, Some("user"), &ProtectRequest::Keep, &out);
    assert_eq!(page_texts(&pdfium, &out, Some("user")).unwrap(), PAGE_TEXT);
    assert!(
        protection_of(&pdfium, &out, Some("owner"))
            .unwrap()
            .owner_unlocked
    );
    assert_eq!(
        protection_of(&pdfium, &out, Some("user"))
            .unwrap()
            .permissions,
        restricted()
    );
    let (before, after) = (
        encrypt_dictionary(&src, "user"),
        encrypt_dictionary(&out, "user"),
    );
    for key in [&b"O"[..], b"U", b"OE", b"UE", b"Perms", b"P"] {
        assert_eq!(before.get(key).unwrap(), after.get(key).unwrap());
    }
}

#[test]
fn keeping_a_permissions_only_file_needs_no_password() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.pdf");
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &plain);
    save_protected(&pdfium, &plain, None, &set("", "owner", restricted()), &src);
    save_protected(&pdfium, &src, None, &ProtectRequest::Keep, &out);
    let anyone = protection_of(&pdfium, &out, None).unwrap();
    assert!(anyone.encrypted && !anyone.open_password);
    assert_eq!(anyone.permissions, restricted());
    assert!(
        protection_of(&pdfium, &out, Some("owner"))
            .unwrap()
            .owner_unlocked
    );
}

#[test]
fn removing_protection_opens_without_a_password() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.pdf");
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    fixture(&pdfium, &plain);
    save_protected(
        &pdfium,
        &plain,
        None,
        &set("user", "owner", restricted()),
        &src,
    );
    save_protected(&pdfium, &src, Some("owner"), &ProtectRequest::Remove, &out);
    assert_eq!(page_texts(&pdfium, &out, None).unwrap(), PAGE_TEXT);
    assert!(!protection_of(&pdfium, &out, None).unwrap().encrypted);
}

/// The fixture under an RC4 128-bit handler (revision 3), the way older
/// tools protect files.
fn rc4_fixture(pdfium: &Pdfium, path: &Path, permissions: Permissions) {
    fixture(pdfium, path);
    let mut doc = Document::load(path).unwrap();
    if !doc.trailer.has(b"ID") {
        doc.trailer.set("ID", fresh_id().unwrap());
    }
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner",
        user_password: "user",
        key_length: 128,
        permissions,
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    doc.save(path).unwrap();
}

#[test]
fn keeping_an_older_handler_upgrades_it_when_the_permissions_password_is_known() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    rc4_fixture(&pdfium, &src, restricted().to_permissions());
    assert_eq!(
        protection_of(&pdfium, &src, Some("user")).unwrap().revision,
        3
    );

    save_protected(&pdfium, &src, Some("owner"), &ProtectRequest::Keep, &out);
    let user = protection_of(&pdfium, &out, Some("user")).unwrap();
    assert_eq!(user.revision, 6, "moved up to AES-256");
    assert_eq!(user.permissions, restricted());
    assert!(
        protection_of(&pdfium, &out, Some("owner"))
            .unwrap()
            .owner_unlocked
    );
    assert_eq!(page_texts(&pdfium, &out, Some("user")).unwrap(), PAGE_TEXT);
    assert!(page_texts(&pdfium, &out, None).is_err());
}

#[test]
fn keeping_an_older_handler_reuses_it_when_only_the_open_password_is_known() {
    let _guard = pdfium_init::test_guard();
    let pdfium = test_pdfium();
    let dir = tempfile::tempdir().unwrap();
    let (src, out) = (dir.path().join("src.pdf"), dir.path().join("out.pdf"));
    rc4_fixture(&pdfium, &src, restricted().to_permissions());

    save_protected(&pdfium, &src, Some("user"), &ProtectRequest::Keep, &out);
    let user = protection_of(&pdfium, &out, Some("user")).unwrap();
    assert_eq!(user.revision, 3, "the source's own handler");
    assert_eq!(user.permissions, restricted());
    assert!(
        protection_of(&pdfium, &out, Some("owner"))
            .unwrap()
            .owner_unlocked
    );
    assert_eq!(page_texts(&pdfium, &out, Some("user")).unwrap(), PAGE_TEXT);
}

#[test]
fn permission_bits_round_trip() {
    let cases = [
        PermissionSet::all(),
        restricted(),
        PermissionSet {
            print: PrintLevel::None,
            copy: true,
            edit: false,
            annotate: true,
            fill_forms: false,
            assemble: true,
            accessibility: false,
        },
    ];
    for case in cases {
        let bits = case.to_permissions().bits() as u32;
        assert_eq!(PermissionSet::from_bits(bits), case);
    }
}

#[test]
fn recovered_passwords_lose_their_padding() {
    let mut padded = b"user".to_vec();
    padded.extend_from_slice(&PAD[..28]);
    assert_eq!(strip_padding(&padded), b"user");
    assert_eq!(strip_padding(&PAD), b"");
    assert_eq!(strip_padding(b"short"), b"short");
}

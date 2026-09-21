//! AcroForm plumbing shared between reading fields and writing them back.
//!
//! Everything here goes through PDFium's *form-fill environment* rather than
//! through `pdfium-render`'s typed field wrappers. That is not a stylistic
//! preference: at 0.8.37 the typed setters do not work for buttons, and the
//! text setter only writes /V on the widget, leaving /AP and parent fields stale.
//! `PdfFormCheckboxField::set_checked` hardcodes the on-state to `/Yes`, so a
//! checkbox whose on-state is named anything else is written to a state its
//! appearance dictionary has no entry for; and
//! `PdfFormRadioButtonField::set_checked` resolves the state to write by
//! reading the widget's *current* `/AS`, so it writes back whatever was
//! already there and always succeeds at doing nothing. Both fail silently,
//! and the radio path additionally prints debug output to stdout.
//!
//! The form-fill environment is what a real viewer drives, and it is the only
//! thing that gets a button right: it owns `/V` and `/AS` together, regenerates
//! the appearance stream, and enforces the exclusivity that makes selecting one
//! radio clear the rest of its group.

use crate::engine::types::PlanFormButton;
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::os::raw::{c_int, c_ulong, c_void};

/// The callback table PDFium requires to open a form-fill environment, with
/// every hook null.
///
/// Enough to *read* fields, which is all the enumeration in `worker.rs` needs.
/// Not enough to change one — see `FormSession`. PDFium retains this pointer
/// until `FPDFDOC_ExitFormFillEnvironment`, so whatever holds it has to keep
/// it at a fixed address for the whole session.
pub fn form_fill_info() -> FPDF_FORMFILLINFO {
    FPDF_FORMFILLINFO {
        version: 2,
        Release: None,
        FFI_Invalidate: None,
        FFI_OutputSelectedRect: None,
        FFI_SetCursor: None,
        FFI_SetTimer: None,
        FFI_KillTimer: None,
        FFI_GetLocalTime: None,
        FFI_OnChange: None,
        FFI_GetPage: None,
        FFI_GetCurrentPage: None,
        FFI_GetRotation: None,
        FFI_ExecuteNamedAction: None,
        FFI_SetTextFieldFocus: None,
        FFI_DoURIAction: None,
        FFI_DoGoToAction: None,
        m_pJsPlatform: std::ptr::null_mut(),
        xfa_disabled: 0,
        FFI_DisplayCaret: None,
        FFI_GetCurrentPageIndex: None,
        FFI_SetCurrentPage: None,
        FFI_GotoURL: None,
        FFI_GetPageViewRect: None,
        FFI_PageEvent: None,
        FFI_PopupMenu: None,
        FFI_OpenFile: None,
        FFI_EmailTo: None,
        FFI_UploadTo: None,
        FFI_GetPlatform: None,
        FFI_GetLanguage: None,
        FFI_DownloadFromURL: None,
        FFI_PostRequestURL: None,
        FFI_PutRequestURL: None,
        FFI_OnFocusChange: None,
        FFI_DoURIActionWithKeyboardModifier: None,
    }
}

/// A form-fill environment that can commit values and regenerate appearances.
///
/// The null callback table above is enough to read fields, and enough to
/// *check a checkbox* — but not enough to select a radio button. Selecting one
/// means clearing every other button in its group, and PDFium reaches those
/// siblings by asking the host for the page they sit on. With `FFI_GetPage`
/// left null it cannot, so it abandons the change and reports nothing: the
/// click is accepted, and the value silently stays where it was. Supplying the
/// hook is the whole difference between a radio group that answers to clicks
/// and one that ignores them.
///
/// The hook must hand back the *same* page handle each time. Loading a fresh
/// one per call gives PDFium a second view of a page it already has open, and
/// the change lands on the copy that is about to be thrown away.
///
/// `info` must stay the first field: PDFium is given a pointer to it and hands
/// that same pointer back to the callbacks, which cast it to the whole struct.
/// `#[repr(C)]` is what makes that cast sound.
#[repr(C)]
struct FormSession<'a> {
    info: FPDF_FORMFILLINFO,
    bindings: &'a dyn PdfiumLibraryBindings,
    /// Page handles opened on PDFium's behalf, closed when the session ends.
    pages: RefCell<Vec<(c_int, FPDF_PAGE)>>,
}

impl<'a> FormSession<'a> {
    fn new(bindings: &'a dyn PdfiumLibraryBindings) -> Box<Self> {
        let mut info = form_fill_info();
        info.FFI_GetPage = Some(ffi_get_page);
        info.FFI_GetCurrentPage = Some(ffi_get_current_page);
        Box::new(FormSession {
            info,
            bindings,
            pages: RefCell::new(Vec::new()),
        })
    }

    /// The one handle for `index`, opening it on first use.
    fn page(&self, document: FPDF_DOCUMENT, index: c_int) -> FPDF_PAGE {
        let mut pages = self.pages.borrow_mut();
        if let Some((_, page)) = pages.iter().find(|(i, _)| *i == index) {
            return *page;
        }
        let page = self.bindings.FPDF_LoadPage(document, index);
        if !page.is_null() {
            pages.push((index, page));
        }
        page
    }

    /// Close every page this session opened. Called once the form-fill
    /// environment is gone, never before: PDFium's page views outlive the
    /// individual clicks and still refer to these handles.
    fn close_pages(&self) {
        for (_, page) in self.pages.borrow_mut().drain(..) {
            self.bindings.FPDF_ClosePage(page);
        }
    }
}

/// Reinterpret the callback's `pThis` as the session that owns it.
///
/// Sound because the only pointer PDFium is ever given is `&mut session.info`,
/// and `info` is the first field of a `#[repr(C)]` `FormSession`.
unsafe fn session_of<'s>(this: *mut FPDF_FORMFILLINFO) -> Option<&'s FormSession<'s>> {
    (!this.is_null()).then(|| unsafe { &*(this as *const FormSession<'s>) })
}

unsafe extern "C" fn ffi_get_page(
    this: *mut FPDF_FORMFILLINFO,
    document: FPDF_DOCUMENT,
    page_index: c_int,
) -> FPDF_PAGE {
    match unsafe { session_of(this) } {
        Some(session) => session.page(document, page_index),
        None => std::ptr::null_mut(),
    }
}

unsafe extern "C" fn ffi_get_current_page(
    this: *mut FPDF_FORMFILLINFO,
    document: FPDF_DOCUMENT,
) -> FPDF_PAGE {
    unsafe { ffi_get_page(this, document, 0) }
}

/// Collects what PDFium writes during `FPDF_SaveAsCopy`.
///
/// `inner` must stay the first field: PDFium is handed a pointer to it and
/// hands that same pointer back to `write_block`, which casts it back to the
/// whole struct. That only holds while the two share an address.
#[repr(C)]
struct BufferWriter {
    inner: FPDF_FILEWRITE,
    buffer: Vec<u8>,
}

unsafe extern "C" fn write_block(
    this: *mut FPDF_FILEWRITE,
    data: *const c_void,
    size: c_ulong,
) -> c_int {
    if this.is_null() || data.is_null() {
        return 0;
    }
    // Safe by construction: `inner` is the first field of `BufferWriter`, and
    // the only pointer PDFium ever receives came from one.
    let writer = unsafe { &mut *(this as *mut BufferWriter) };
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
    writer.buffer.extend_from_slice(bytes);
    1
}

impl BufferWriter {
    fn new() -> Self {
        BufferWriter {
            inner: FPDF_FILEWRITE {
                version: 1,
                WriteBlock: Some(write_block),
            },
            buffer: Vec::new(),
        }
    }
}

/// Set text values and checkbox/radio states on the PDF at `path`,
/// rewriting the file in place.
///
/// Runs after the document has been written, because it needs a file PDFium
/// can open on its own terms — the form-fill environment attaches to a raw
/// document handle, which `pdfium-render` does not hand out.
///
/// A widget already in the requested state is left alone, so this is a no-op
/// for anything the user did not actually change.
pub fn apply_edits(
    b: &dyn PdfiumLibraryBindings,
    path: &std::path::Path,
    texts: &[(String, String)],
    buttons: &[PlanFormButton],
) -> AppResult<()> {
    if texts.is_empty() && buttons.is_empty() {
        return Ok(());
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| AppError::Io("form edit pass: bad path".into()))?;

    let doc = b.FPDF_LoadDocument(path_str, None);
    if doc.is_null() {
        return Err(AppError::Internal(
            "form edit pass: could not reopen the written file".into(),
        ));
    }
    // Every path from here has to close the document, so the work is done in
    // a helper and the result inspected only after closing.
    let saved = edited_document_bytes(b, doc, texts, buttons);
    b.FPDF_CloseDocument(doc);
    match saved? {
        Some(bytes) => {
            use std::io::Write;
            let mut file = std::fs::File::create(path)?;
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        }
        // Nothing needed changing, so the file already says what it should.
        None => Ok(()),
    }
}

/// Commit the requested edits and return the resulting document, or
/// `None` if every widget was already in the state the plan asked for.
fn edited_document_bytes(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    texts: &[(String, String)],
    buttons: &[PlanFormButton],
) -> AppResult<Option<Vec<u8>>> {
    let mut session = FormSession::new(b);
    let form = b.FPDFDOC_InitFormFillEnvironment(doc, &mut session.info);
    if form.is_null() {
        return Err(AppError::Internal(
            "form edit pass: document has no form-fill environment".into(),
        ));
    }

    // One page load per page that actually carries a change.
    let mut by_page: BTreeMap<u32, Vec<&PlanFormButton>> = BTreeMap::new();
    for button in buttons {
        by_page.entry(button.page).or_default().push(button);
    }

    // Inspect one page at a time without retaining every page in a large form.
    // Only pages with edits enter the session's stable handle cache.
    let mut text_by_page = BTreeMap::new();
    if !texts.is_empty() {
        for index in 0..b.FPDF_GetPageCount(doc) {
            let page = b.FPDF_LoadPage(doc, index);
            if page.is_null() {
                continue;
            }
            let mut edits = Vec::new();
            for ai in 0..b.FPDFPage_GetAnnotCount(page) {
                let annot = b.FPDFPage_GetAnnot(page, ai);
                if annot.is_null() {
                    continue;
                }
                if b.FPDFAnnot_GetFormFieldType(form, annot) == PdfFormFieldType::Text as i32 {
                    let name = read_utf16(|buffer, len| {
                        b.FPDFAnnot_GetFormFieldName(form, annot, buffer, len)
                    });
                    if let Some((_, value)) = texts.iter().find(|(field, _)| *field == name) {
                        edits.push((ai, name, value));
                    }
                }
                b.FPDFPage_CloseAnnot(annot);
            }
            b.FPDF_ClosePage(page);
            if !edits.is_empty() {
                by_page.entry(index as u32).or_default();
                text_by_page.insert(index as u32, edits);
            }
        }
    }

    // All widgets of an edited field must have live page views before the
    // first change. PDFium only refreshes sibling appearances in loaded views;
    // opening a sibling after /V changed leaves its old /AP behind.
    let mut loaded_pages = Vec::new();
    for index in by_page.keys() {
        let page = session.page(doc, *index as c_int);
        if !page.is_null() {
            b.FORM_OnAfterLoadPage(page, form);
            loaded_pages.push(page);
        }
    }

    let mut changed = 0usize;
    let mut edit_result = Ok(());
    for (page_index, page_buttons) in by_page {
        // Through the session, so PDFium and this loop are working on one
        // handle rather than two views of the same page.
        let page = session.page(doc, page_index as c_int);
        if page.is_null() {
            continue;
        }
        if let Some(edits) = text_by_page.get(&page_index) {
            for (index, name, value) in edits {
                match fill_text(b, form, page, *index, name, value) {
                    Ok(did_change) => changed += usize::from(did_change),
                    Err(error) => {
                        edit_result = Err(error);
                        break;
                    }
                }
            }
        }
        if edit_result.is_err() {
            break;
        }
        for button in page_buttons {
            if click_button(b, form, page, button) {
                changed += 1;
            }
        }
    }
    for page in loaded_pages {
        b.FORM_OnBeforeClosePage(page, form);
    }

    let saved = (edit_result.is_ok() && changed > 0).then(|| {
        let mut writer = BufferWriter::new();
        let ok = b.FPDF_SaveAsCopy(doc, &mut writer.inner, 0);
        b.is_true(ok).then_some(writer.buffer)
    });

    b.FPDFDOC_ExitFormFillEnvironment(form);
    session.close_pages();
    edit_result?;

    match saved {
        None => Ok(None),
        Some(Some(bytes)) => Ok(Some(bytes)),
        Some(None) => Err(AppError::Internal(
            "form edit pass: PDFium refused to save".into(),
        )),
    }
}

fn read_utf16(read: impl Fn(*mut u16, c_ulong) -> c_ulong) -> String {
    let length = read(std::ptr::null_mut(), 0);
    if length < 2 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize / 2];
    read(buffer.as_mut_ptr(), length);
    if buffer.last() == Some(&0) {
        buffer.pop();
    }
    String::from_utf16_lossy(&buffer)
}

/// Drive the same edit/blur sequence as a viewer. PDFium updates the canonical
/// field (including parent /V and sibling widgets) and generates font resources
/// and /AP streams. A dictionary-only setter does none of those things.
fn fill_text(
    b: &dyn PdfiumLibraryBindings,
    form: FPDF_FORMHANDLE,
    page: FPDF_PAGE,
    index: i32,
    name: &str,
    value: &str,
) -> AppResult<bool> {
    let annot = b.FPDFPage_GetAnnot(page, index);
    if annot.is_null() {
        return Err(AppError::Internal(format!(
            "cannot open text field '{name}'"
        )));
    }
    let result = (|| {
        let original =
            read_utf16(|buffer, len| b.FPDFAnnot_GetFormFieldValue(form, annot, buffer, len));
        if original == value {
            return Ok(false);
        }
        if !b.is_true(b.FORM_SetFocusedAnnot(form, annot))
            || !b.is_true(b.FORM_SelectAllText(form, page))
        {
            return Err(AppError::Internal(format!(
                "cannot edit text field '{name}'"
            )));
        }
        let units: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        b.FORM_ReplaceSelection(form, page, units.as_ptr());
        if !b.is_true(b.FORM_ForceToKillFocus(form)) {
            return Err(AppError::Internal(format!(
                "cannot commit text field '{name}'"
            )));
        }
        let saved =
            read_utf16(|buffer, len| b.FPDFAnnot_GetFormFieldValue(form, annot, buffer, len));
        // PDFium normalizes line endings for multiline fields.
        let normalize = |s: &str| s.replace("\r\n", "\n").replace('\r', "\n");
        if normalize(&saved) != normalize(value) {
            return Err(AppError::Internal(format!(
                "text field '{name}' did not accept the full value"
            )));
        }
        if b.FPDFAnnot_GetAP(annot, 0, std::ptr::null_mut(), 0) <= 2 {
            return Err(AppError::Internal(format!(
                "text field '{name}' has no saved appearance"
            )));
        }
        Ok(true)
    })();
    b.FPDFPage_CloseAnnot(annot);
    result
}

/// Press and release inside one widget, if its state needs to change.
fn click_button(
    b: &dyn PdfiumLibraryBindings,
    form: FPDF_FORMHANDLE,
    page: FPDF_PAGE,
    button: &PlanFormButton,
) -> bool {
    let annotation = b.FPDFPage_GetAnnot(page, button.index as i32);
    if annotation.is_null() {
        return false;
    }
    let already = b.FPDFAnnot_IsChecked(form, annotation) != 0;
    if already == button.checked {
        b.FPDFPage_CloseAnnot(annotation);
        return false;
    }
    let mut rect = FS_RECTF {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    let has_rect = b.FPDFAnnot_GetRect(annotation, &mut rect) != 0;
    b.FPDFPage_CloseAnnot(annotation);
    if !has_rect {
        return false;
    }
    // Page coordinates, which is what the form-fill entry points take.
    let x = ((rect.left + rect.right) / 2.0) as f64;
    let y = ((rect.bottom + rect.top) / 2.0) as f64;
    b.FORM_OnLButtonDown(form, page, 0, x, y);
    b.FORM_OnLButtonUp(form, page, 0, x, y);
    // Release the focus the click just took. A focused control holds its new
    // value in the form layer rather than in the document, so leaving it
    // focused both blocks the next click from landing on a different field
    // and leaves the change out of anything saved afterwards.
    b.FORM_ForceToKillFocus(form);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdfium_init;
    use crate::engine::types::{EditPlan, PlanPage};
    use std::path::Path;

    /// A small AcroForm: a checkbox, a two-button radio group, and a text
    /// field. Written by hand because PDFium can read form fields but cannot
    /// create them, so there is no way to build this through the library the
    /// rest of the engine uses.
    fn write_form_fixture(path: &Path) {
        let mut objects: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut add = |n: u32, body: &[u8]| objects.push((n, body.to_vec()));

        let on = b"q 0 0 0 rg 2 2 10 10 re f Q";
        let off = b"q Q";
        for (n, content) in [(20u32, &on[..]), (21, &off[..])] {
            let mut body = format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 14 14] /Length {} >>\nstream\n",
                content.len()
            )
            .into_bytes();
            body.extend_from_slice(content);
            body.extend_from_slice(b"\nendstream");
            add(n, &body);
        }

        add(
            1,
            b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [10 0 R 11 0 R 14 0 R]               /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 30 0 R >> >> >> >>",
        );
        add(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        add(
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400]               /Resources << /Font << /Helv 30 0 R >> >> /Contents 4 0 R               /Annots [10 0 R 12 0 R 13 0 R 14 0 R] >>",
        );
        add(4, b"<< /Length 0 >>\nstream\n\nendstream");
        add(
            30,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        );
        add(
            10,
            b"<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Off /AS /Off /F 4               /Rect [100 345 114 359] /AP << /N << /Yes 20 0 R /Off 21 0 R >> >> >>",
        );
        add(
            11,
            b"<< /FT /Btn /Ff 49152 /T (choice) /V /Off /Kids [12 0 R 13 0 R] >>",
        );
        add(
            12,
            b"<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /F 4               /Rect [100 295 114 309] /AP << /N << /a 20 0 R /Off 21 0 R >> >> >>",
        );
        add(
            13,
            b"<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /F 4               /Rect [140 295 154 309] /AP << /N << /b 20 0 R /Off 21 0 R >> >> >>",
        );
        add(
            14,
            b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Before) /F 4               /Rect [100 240 250 258] /DA (/Helv 10 Tf 0 g) /AP << /N 22 0 R >> >>",
        );
        let content = b"BT /Helv 10 Tf 0 g 2 5 Td (Before) Tj ET";
        let mut appearance = format!("<< /Type /XObject /Subtype /Form /BBox [0 0 150 18] /Resources << /Font << /Helv 30 0 R >> >> /Length {} >>\nstream\n", content.len()).into_bytes();
        appearance.extend_from_slice(content);
        appearance.extend_from_slice(b"\nendstream");
        add(22, &appearance);

        write_objects(path, objects);
    }

    fn write_objects(path: &Path, mut objects: Vec<(u32, Vec<u8>)>) {
        objects.sort_by_key(|(n, _)| *n);
        let mut out: Vec<u8> = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets: Vec<(u32, usize)> = Vec::new();
        for (n, body) in &objects {
            offsets.push((*n, out.len()));
            out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let size = objects.iter().map(|(n, _)| *n).max().unwrap() + 1;
        let start_xref = out.len();
        out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for n in 1..size {
            match offsets.iter().find(|(o, _)| *o == n) {
                Some((_, at)) => out.extend_from_slice(format!("{at:010} 00000 n \n").as_bytes()),
                None => out.extend_from_slice(b"0000000000 65535 f \n"),
            }
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{start_xref}\n%%EOF\n")
                .as_bytes(),
        );
        std::fs::write(path, out).expect("write form fixture");
    }

    fn write_shared_text_fixture(path: &Path) {
        let objects: Vec<(u32, &[u8])> = vec![
            (1, b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 8 0 R >> >> >> >>"),
            (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
            (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Annots [6 0 R] >>"),
            (4, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Annots [7 0 R] >>"),
            (5, b"<< /FT /Tx /T (shared) /V (Before) /Ff 4096 /MaxLen 40 /Kids [6 0 R 7 0 R] >>"),
            (6, b"<< /Type /Annot /Subtype /Widget /Parent 5 0 R /F 4 /Rect [40 200 260 280] /P 3 0 R >>"),
            (7, b"<< /Type /Annot /Subtype /Widget /Parent 5 0 R /F 4 /Rect [40 200 260 280] /P 4 0 R >>"),
            (8, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
        ];
        write_objects(
            path,
            objects
                .into_iter()
                .map(|(id, value)| (id, value.to_vec()))
                .collect(),
        );
    }

    /// Saving a form used to hand back a page of dead boxes: `FPDF_ImportPages`
    /// copies widget annotations but not the `/AcroForm` that gives them
    /// meaning, so the output stopped being a form at all — and the text
    /// values the user had typed went with it, silently, while the panel said
    /// they had been written.
    #[test]
    fn a_form_survives_a_save_with_its_values() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("form.pdf");
        let output = dir.path().join("out.pdf");
        write_form_fixture(&source);

        let before = pdfium.load_pdf_from_file(&source, None).expect("fixture");
        assert!(before.form().is_some(), "fixture is not a form");
        drop(before);

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
            form: vec![("name".to_string(), "Ada".to_string())],
            // Tick the checkbox, and select the SECOND radio so the assertion
            // below cannot pass by everything simply defaulting on.
            form_buttons: vec![
                PlanFormButton {
                    page: 0,
                    index: 0,
                    checked: true,
                },
                PlanFormButton {
                    page: 0,
                    index: 2,
                    checked: true,
                },
            ],
        };
        crate::engine::save::build_output(
            &pdfium,
            source.to_str().expect("source path"),
            None,
            &plan,
            &output,
        )
        .expect("build output");

        let after = pdfium.load_pdf_from_file(&output, None).expect("reopen");
        assert!(
            after.form().is_some(),
            "the output is no longer a form — the /AcroForm was dropped"
        );
        assert_eq!(
            after
                .form()
                .unwrap()
                .field_values(after.pages())
                .get("name"),
            Some(&Some("Ada".into()))
        );

        // Read the states back the way the engine itself does, through a
        // form-fill environment, rather than through the typed wrappers.
        let states = button_states(&pdfium, &output);
        assert_eq!(
            states,
            vec![true, false, true],
            "expected the checkbox on, the first radio off and the second on"
        );
    }

    #[test]
    fn text_field_updates_its_saved_appearance_without_button_edits() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("form.pdf");
        let output = dir.path().join("out.pdf");
        write_form_fixture(&source);
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
            form: vec![("name".into(), "Ada".into())],
            form_buttons: Vec::new(),
        };
        crate::engine::save::build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output)
            .expect("save text field");
        // Read /AP before loading a form page view or rendering: either can
        // synthesize an appearance in memory and hide a broken file on disk.
        let b = pdfium.bindings();
        let doc = b.FPDF_LoadDocument(output.to_str().unwrap(), None);
        assert!(!doc.is_null());
        let page = b.FPDF_LoadPage(doc, 0);
        let annot = b.FPDFPage_GetAnnot(page, 3);
        let len = b.FPDFAnnot_GetAP(annot, 0, std::ptr::null_mut(), 0);
        let mut buffer = vec![0u16; len as usize / 2];
        b.FPDFAnnot_GetAP(annot, 0, buffer.as_mut_ptr(), len);
        let appearance = String::from_utf16_lossy(&buffer);
        b.FPDFPage_CloseAnnot(annot);
        b.FPDF_ClosePage(page);
        b.FPDF_CloseDocument(doc);
        assert!(
            appearance.contains("Ada"),
            "saved appearance is stale: {appearance}"
        );
        assert!(!appearance.contains("Before"));
        let reopened = pdfium
            .load_pdf_from_file(&output, None)
            .expect("reopen form");
        assert_eq!(
            reopened
                .form()
                .unwrap()
                .field_values(reopened.pages())
                .get("name"),
            Some(&Some("Ada".into()))
        );
    }

    #[test]
    fn shared_text_fields_save_multiline_values_and_can_be_cleared() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        let filled = dir.path().join("filled.pdf");
        let cleared = dir.path().join("cleared.pdf");
        write_shared_text_fixture(&source);
        for (input, output, value) in [
            (&source, &filled, "Ada (Lovelace)\nSecond line"),
            (&filled, &cleared, ""),
        ] {
            let plan = EditPlan {
                pages: (0..2)
                    .map(|index| PlanPage {
                        src_index: Some(index),
                        width_pt: 300.0,
                        height_pt: 400.0,
                        rotation: 0,
                        annots: Vec::new(),
                        texts: Vec::new(),
                        images: Vec::new(),
                        drop_src_annots: Vec::new(),
                    })
                    .collect(),
                form: vec![("shared".into(), value.into())],
                form_buttons: Vec::new(),
            };
            crate::engine::save::build_output(
                &pdfium,
                input.to_str().unwrap(),
                None,
                &plan,
                output,
            )
            .unwrap();
            let b = pdfium.bindings();
            let doc = b.FPDF_LoadDocument(output.to_str().unwrap(), None);
            assert!(!doc.is_null());
            let mut info = Box::pin(form_fill_info());
            let form = b.FPDFDOC_InitFormFillEnvironment(doc, info.as_mut().get_mut());
            let mut states = Vec::new();
            for index in 0..2 {
                let page = b.FPDF_LoadPage(doc, index);
                let annot = b.FPDFPage_GetAnnot(page, 0);
                // Deliberately no OnAfterLoadPage: inspect only what was saved.
                let field_value = read_utf16(|buffer, len| {
                    b.FPDFAnnot_GetFormFieldValue(form, annot, buffer, len)
                });
                let appearance = read_utf16(|buffer, len| b.FPDFAnnot_GetAP(annot, 0, buffer, len));
                states.push((field_value, appearance));
                b.FPDFPage_CloseAnnot(annot);
                b.FPDF_ClosePage(page);
            }
            b.FPDFDOC_ExitFormFillEnvironment(form);
            b.FPDF_CloseDocument(doc);
            for (field_value, appearance) in states {
                assert_eq!(field_value.replace("\r\n", "\n").replace('\r', "\n"), value);
                assert!(!appearance.is_empty(), "missing saved appearance");
                assert!(!appearance.contains("Before"));
                assert_eq!(
                    appearance.contains("Ada"),
                    !value.is_empty(),
                    "stale appearance: {appearance}"
                );
            }
        }
    }

    #[test]
    fn a_rejected_text_value_preserves_the_destination() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        let destination = dir.path().join("destination.pdf");
        write_shared_text_fixture(&source);
        std::fs::write(&destination, b"original bytes").unwrap();
        let result = crate::engine::save::verified_atomic_replace(
            &destination,
            |temp| {
                std::fs::copy(&source, temp)?;
                apply_edits(
                    pdfium.bindings(),
                    temp,
                    &[("shared".into(), "x".repeat(100))],
                    &[],
                )
            },
            |_| Ok(()),
            || Ok(()),
        );
        assert!(
            result.is_err(),
            "a value exceeding /MaxLen must not silently truncate"
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"original bytes");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "temporary file must be removed"
        );
    }

    /// `(checkbox, radio a, radio b)` as PDFium reports them.
    fn button_states(pdfium: &Pdfium, path: &Path) -> Vec<bool> {
        let b = pdfium.bindings();
        let doc = b.FPDF_LoadDocument(path.to_str().expect("path"), None);
        assert!(!doc.is_null(), "reopen for verification");
        let mut info = Box::pin(form_fill_info());
        let form = b.FPDFDOC_InitFormFillEnvironment(doc, info.as_mut().get_mut());
        assert!(!form.is_null(), "no form-fill environment");
        let page = b.FPDF_LoadPage(doc, 0);
        b.FORM_OnAfterLoadPage(page, form);
        let mut out = Vec::new();
        for index in 0..b.FPDFPage_GetAnnotCount(page) {
            let annotation = b.FPDFPage_GetAnnot(page, index);
            let kind = b.FPDFAnnot_GetFormFieldType(form, annotation);
            if kind == 2 || kind == 3 {
                out.push(b.FPDFAnnot_IsChecked(form, annotation) != 0);
            }
            b.FPDFPage_CloseAnnot(annotation);
        }
        b.FORM_OnBeforeClosePage(page, form);
        b.FPDFDOC_ExitFormFillEnvironment(form);
        b.FPDF_ClosePage(page);
        b.FPDF_CloseDocument(doc);
        out
    }
}

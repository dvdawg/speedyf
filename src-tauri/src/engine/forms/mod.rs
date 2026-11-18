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
//!
//! `read` enumerates fields for the page overlay, `write` commits the user's
//! edits at save time.

mod appearance;
mod read;
pub mod scripts;
mod write;

#[cfg(test)]
mod behaviour_eval;
#[cfg(test)]
mod behaviour_tests;
#[cfg(test)]
mod eval;
#[cfg(test)]
mod tests;

pub use read::read_form;
pub use write::{apply_edits, FormEdits};

use pdfium_render::prelude::*;
use std::cell::RefCell;
use std::os::raw::{c_int, c_ulong, c_void};

/// The callback table PDFium requires to open a form-fill environment, with
/// every hook null.
///
/// Enough to *read* fields, which is all the enumeration in `worker.rs` needs.
/// Not enough to change one - see `FormSession`. PDFium retains this pointer
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
/// *check a checkbox* - but not enough to select a radio button. Selecting one
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

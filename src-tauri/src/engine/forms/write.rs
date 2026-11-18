//! Committing form edits through the form-fill environment at save time.

use super::{appearance, read_utf16, BufferWriter, FormSession};
use crate::engine::save::{mend_xref_stream_type, save_mode, SaveMode};
use crate::engine::types::{PlanFormButton, PlanFormChoice};
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::os::raw::c_int;

/// `FPDF_GetFormType` values for documents carrying XFA.
const FORMTYPE_XFA_FULL: c_int = 2;
const FORMTYPE_XFA_FOREGROUND: c_int = 3;

/// `FWL_VKEY_Delete` from fpdf_fwlevent.h.
const VK_DELETE: c_int = 0x2E;

/// Everything the user changed in a form, as the save plan carries it.
#[derive(Clone, Copy)]
pub struct FormEdits<'a> {
    pub texts: &'a [(String, String)],
    pub buttons: &'a [PlanFormButton],
    pub choices: &'a [PlanFormChoice],
    /// Text fields whose page appearance shows a formatted value while /V
    /// keeps the raw one: name, formatted text, and the text colour a red
    /// negative style asks for (None to leave it). See `appearance`.
    pub display: &'a [(String, String, Option<bool>)],
}

impl FormEdits<'_> {
    fn is_empty(&self) -> bool {
        self.texts.is_empty() && self.buttons.is_empty() && self.choices.is_empty()
    }
}

/// Set text values, choice selections and checkbox/radio states on the PDF
/// at `path`, updating the file in place.
///
/// Runs after the document has been written, because it needs a file PDFium
/// can open on its own terms - the form-fill environment attaches to a raw
/// document handle, which `pdfium-render` does not hand out.
///
/// The change is appended as an incremental update whenever the file allows
/// one (see `save::save_mode`), so the bytes before it - and any signature
/// over them - are left exactly as they were.
///
/// A widget already in the requested state is left alone, so this is a no-op
/// for anything the user did not actually change.
pub fn apply_edits(
    b: &dyn PdfiumLibraryBindings,
    path: &std::path::Path,
    edits: FormEdits<'_>,
) -> AppResult<()> {
    if edits.is_empty() {
        return Ok(());
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| AppError::Io("form edit pass: bad path".into()))?;

    // Formatted appearances need two small updates of their own around
    // PDFium's; on a file that cannot take them, fields are drawn raw.
    let formatted = !edits.display.is_empty() && appearance::can_update(path);
    let display: HashMap<&str, &str> = if formatted {
        edits
            .display
            .iter()
            .map(|(name, text, _)| (name.as_str(), text.as_str()))
            .collect()
    } else {
        HashMap::new()
    };
    if formatted {
        let colors: Vec<(&str, bool)> = edits
            .display
            .iter()
            .filter(|(name, _, _)| edits.texts.iter().any(|(t, _)| t == name))
            .filter_map(|(name, _, red)| red.map(|r| (name.as_str(), r)))
            .collect();
        appearance::set_text_colors(path, &colors)
            .map_err(|e| AppError::Internal(format!("form edit pass: {e}")))?;
    }

    let doc = b.FPDF_LoadDocument(path_str, None);
    if doc.is_null() {
        return Err(AppError::Internal(
            "form edit pass: could not reopen the written file".into(),
        ));
    }
    let mode = save_mode(b, doc);
    let form_type = b.FPDF_GetFormType(doc);
    let has_xfa = form_type == FORMTYPE_XFA_FULL || form_type == FORMTYPE_XFA_FOREGROUND;
    // Every path from here has to close the document, so the work is done in
    // a helper and the result inspected only after closing.
    let mut raw_values: Vec<(&str, &str)> = Vec::new();
    let saved = edited_document_bytes(b, doc, edits, mode, &display, &mut raw_values);
    b.FPDF_CloseDocument(doc);
    let restore = |raw_values: &[(&str, &str)]| {
        appearance::restore_raw_values(path, raw_values)
            .map(|_| ())
            .map_err(|e| AppError::Internal(format!("form edit pass: {e}")))
    };
    match saved? {
        Some(bytes) => {
            use std::io::Write;
            let mut file = std::fs::File::create(path)?;
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            if mode == SaveMode::Incremental && !mend_xref_stream_type(path)? {
                return Err(AppError::Internal(
                    "form edit pass: the appended update has an unexpected tail".into(),
                ));
            }
            // Not on a signed file: removing /XFA changes the form's
            // definition, which a signature checker (pyHanko; very likely
            // Acrobat) reports as a modification beyond form filling, and
            // on a certified document as breaking the certification.
            if has_xfa && !crate::engine::signing::file_has_signatures(path) {
                drop_xfa(path).map_err(|e| AppError::Internal(format!("form edit pass: {e}")))?;
            }
            restore(&raw_values)
        }
        // Nothing needed changing, so the file already says what it should.
        None => restore(&raw_values),
    }
}

/// Remove /XFA from the AcroForm, in an update appended to `path`.
///
/// A form that carries XFA beside its AcroForm (every IRS form does) keeps
/// its values twice, and Acrobat takes them from the XFA datasets, which the
/// form-fill environment never touches. Left in place, the XFA would show
/// the reader of the saved file the old, usually empty, values over the ones
/// SpeedyF wrote. Dropping it is what other AcroForm fillers do: the widgets,
/// their values and their appearances are all still there.
fn drop_xfa(path: &std::path::Path) -> Result<(), String> {
    use lopdf::{IncrementalDocument, Object};
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut inc: IncrementalDocument = bytes
        .as_slice()
        .try_into()
        .map_err(|e: lopdf::Error| e.to_string())?;
    let prev = inc.get_prev_documents();
    if prev.is_encrypted() {
        return Ok(());
    }
    // Anything lopdf numbers must come past the previous trailer's /Size:
    // it counts only the objects it parsed, and an xref stream is not one
    // of them, so on a stream-xref file it would reuse that stream's number
    // (see the same guard in markup_export).
    let declared = prev
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    let root = prev
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|e| e.to_string())?;
    let acroform = prev
        .get_dictionary(root)
        .and_then(|catalog| catalog.get(b"AcroForm"))
        .map_err(|e| e.to_string())?
        .clone();
    // The AcroForm is usually its own object; when it sits inline in the
    // catalog, the catalog is what gets a new revision.
    let (owner, inline) = match acroform {
        Object::Reference(id) => (id, false),
        Object::Dictionary(_) => (root, true),
        _ => return Ok(()),
    };
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    inc.opt_clone_object_to_new_document(owner)
        .map_err(|e| e.to_string())?;
    let dict = inc
        .new_document
        .get_object_mut(owner)
        .and_then(Object::as_dict_mut)
        .map_err(|e| e.to_string())?;
    let form = if inline {
        dict.get_mut(b"AcroForm")
            .and_then(Object::as_dict_mut)
            .map_err(|e| e.to_string())?
    } else {
        dict
    };
    if form.remove(b"XFA").is_none() {
        return Ok(());
    }
    let mut out = Vec::with_capacity(bytes.len() + 1024);
    inc.save_to(&mut out).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, &out).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

/// Where the edits land: per page, the annotation indices to fill.
#[derive(Default)]
struct PageWork<'a> {
    texts: Vec<(i32, &'a str, &'a str)>,
    choices: Vec<(i32, &'a PlanFormChoice)>,
    buttons: Vec<&'a PlanFormButton>,
}

/// Commit the requested edits and return the resulting document, or
/// `None` if every widget was already in the state the plan asked for.
///
/// A field in `display` is typed as its formatted text; each one that took
/// it is added to `raw_values`, for /V to be put back afterwards.
fn edited_document_bytes<'e>(
    b: &dyn PdfiumLibraryBindings,
    doc: FPDF_DOCUMENT,
    edits: FormEdits<'e>,
    mode: SaveMode,
    display: &HashMap<&str, &str>,
    raw_values: &mut Vec<(&'e str, &'e str)>,
) -> AppResult<Option<Vec<u8>>> {
    let mut session = FormSession::new(b);
    let form = b.FPDFDOC_InitFormFillEnvironment(doc, &mut session.info);
    if form.is_null() {
        return Err(AppError::Internal(
            "form edit pass: document has no form-fill environment".into(),
        ));
    }

    // One page load per page that actually carries a change.
    let mut by_page: BTreeMap<u32, PageWork<'_>> = BTreeMap::new();
    for button in edits.buttons {
        by_page.entry(button.page).or_default().buttons.push(button);
    }

    // Inspect one page at a time without retaining every page in a large form.
    // Only pages with edits enter the session's stable handle cache. A choice
    // field is set through its first widget; PDFium carries the value to the
    // rest.
    if !edits.texts.is_empty() || !edits.choices.is_empty() {
        let mut choice_seen = vec![false; edits.choices.len()];
        for index in 0..b.FPDF_GetPageCount(doc) {
            let page = b.FPDF_LoadPage(doc, index);
            if page.is_null() {
                continue;
            }
            let mut work = PageWork::default();
            for ai in 0..b.FPDFPage_GetAnnotCount(page) {
                let annot = b.FPDFPage_GetAnnot(page, ai);
                if annot.is_null() {
                    continue;
                }
                let kind = b.FPDFAnnot_GetFormFieldType(form, annot);
                let is_text = kind == PdfFormFieldType::Text as i32;
                let is_choice = kind == PdfFormFieldType::ComboBox as i32
                    || kind == PdfFormFieldType::ListBox as i32;
                if is_text || is_choice {
                    let name = read_utf16(|buffer, len| {
                        b.FPDFAnnot_GetFormFieldName(form, annot, buffer, len)
                    });
                    if is_text {
                        if let Some((field, value)) =
                            edits.texts.iter().find(|(field, _)| *field == name)
                        {
                            work.texts.push((ai, field.as_str(), value.as_str()));
                        }
                    } else if let Some(at) =
                        edits.choices.iter().position(|choice| choice.field == name)
                    {
                        if !choice_seen[at] {
                            choice_seen[at] = true;
                            work.choices.push((ai, &edits.choices[at]));
                        }
                    }
                }
                b.FPDFPage_CloseAnnot(annot);
            }
            b.FPDF_ClosePage(page);
            if !work.texts.is_empty() || !work.choices.is_empty() {
                let entry = by_page.entry(index as u32).or_default();
                entry.texts = work.texts;
                entry.choices = work.choices;
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
    'pages: for (page_index, work) in &by_page {
        // Through the session, so PDFium and this loop are working on one
        // handle rather than two views of the same page.
        let page = session.page(doc, *page_index as c_int);
        if page.is_null() {
            continue;
        }
        for (index, name, value) in &work.texts {
            let mut result = Err(AppError::Internal(String::new()));
            if let Some(shown) = display.get(name).filter(|shown| **shown != *value) {
                result = fill_text(b, form, page, *index, name, shown);
                if result.is_ok() && !raw_values.iter().any(|(n, _)| n == name) {
                    raw_values.push((*name, *value));
                }
            }
            // No formatted text, or one the field would not take whole (a
            // /MaxLen shorter than the formatting): draw the raw value.
            if result.is_err() {
                result = fill_text(b, form, page, *index, name, value);
            }
            match result {
                Ok(did_change) => changed += usize::from(did_change),
                Err(error) => {
                    edit_result = Err(error);
                    break 'pages;
                }
            }
        }
        for (index, choice) in &work.choices {
            match fill_choice(b, form, page, *index, choice) {
                Ok(did_change) => changed += usize::from(did_change),
                Err(error) => {
                    edit_result = Err(error);
                    break 'pages;
                }
            }
        }
        for button in &work.buttons {
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
        let ok = b.FPDF_SaveAsCopy(doc, &mut writer.inner, mode.flags());
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

/// The options of a choice widget that are selected now.
fn selected_options(
    b: &dyn PdfiumLibraryBindings,
    form: FPDF_FORMHANDLE,
    annot: FPDF_ANNOTATION,
) -> Vec<u32> {
    (0..b.FPDFAnnot_GetOptionCount(form, annot).max(0))
        .filter(|i| b.is_true(b.FPDFAnnot_IsOptionSelected(form, annot, *i)))
        .map(|i| i as u32)
        .collect()
}

/// Select options in a combo or list box, or type free text into an
/// editable combo, the way a viewer's own control would: focus the widget,
/// change it, and blur so PDFium commits /V and regenerates /AP.
fn fill_choice(
    b: &dyn PdfiumLibraryBindings,
    form: FPDF_FORMHANDLE,
    page: FPDF_PAGE,
    index: i32,
    choice: &PlanFormChoice,
) -> AppResult<bool> {
    let name = choice.field.as_str();
    let annot = b.FPDFPage_GetAnnot(page, index);
    if annot.is_null() {
        return Err(AppError::Internal(format!(
            "cannot open choice field '{name}'"
        )));
    }
    let result = (|| {
        let count = b.FPDFAnnot_GetOptionCount(form, annot).max(0) as u32;
        let value_now =
            || read_utf16(|buffer, len| b.FPDFAnnot_GetFormFieldValue(form, annot, buffer, len));
        let mut want: Vec<u32> = choice
            .selected
            .iter()
            .copied()
            .filter(|i| *i < count)
            .collect();
        want.sort_unstable();
        want.dedup();
        let is_combo =
            b.FPDFAnnot_GetFormFieldType(form, annot) == PdfFormFieldType::ComboBox as i32;
        let multi = !is_combo && b.FPDFAnnot_GetFormFieldFlags(form, annot) & (1 << 21) != 0;
        if !multi {
            want.truncate(1);
        }
        // A combo cannot be emptied through the form-fill environment:
        // deselecting its option is a no-op there. Typed text can still
        // replace an editable one; a fixed one keeps its value.
        if is_combo && want.is_empty() && choice.text.is_none() {
            return Ok(false);
        }

        if let Some(text) = &choice.text {
            if value_now() == *text {
                return Ok(false);
            }
        } else if selected_options(b, form, annot) == want {
            return Ok(false);
        }

        if !b.is_true(b.FORM_SetFocusedAnnot(form, annot)) {
            return Err(AppError::Internal(format!(
                "cannot edit choice field '{name}'"
            )));
        }
        if let Some(text) = &choice.text {
            if !b.is_true(b.FORM_SelectAllText(form, page)) {
                b.FORM_ForceToKillFocus(form);
                return Err(AppError::Internal(format!(
                    "choice field '{name}' does not take typed text"
                )));
            }
            // A combo's edit box ignores FORM_ReplaceSelection, so the text
            // is typed: over the selection, which the first character (or a
            // Delete, for no text at all) replaces.
            if text.is_empty() {
                b.FORM_OnKeyDown(form, page, VK_DELETE, 0);
            }
            for unit in text.encode_utf16() {
                b.FORM_OnChar(form, page, unit as c_int, 0);
            }
        } else {
            // Clear what should go before adding what should come, so a
            // single-select list never passes through an invalid state.
            for i in 0..count as c_int {
                let on = b.is_true(b.FORM_IsIndexSelected(form, page, i));
                if on && !want.contains(&(i as u32)) {
                    b.FORM_SetIndexSelected(form, page, i, 0);
                }
            }
            for i in &want {
                if !b.is_true(b.FORM_IsIndexSelected(form, page, *i as c_int)) {
                    b.FORM_SetIndexSelected(form, page, *i as c_int, 1);
                }
            }
        }
        if !b.is_true(b.FORM_ForceToKillFocus(form)) {
            return Err(AppError::Internal(format!(
                "cannot commit choice field '{name}'"
            )));
        }

        let accepted = match &choice.text {
            Some(text) => value_now() == *text,
            None => selected_options(b, form, annot) == want,
        };
        if !accepted {
            return Err(AppError::Internal(format!(
                "choice field '{name}' did not accept the selection: {:?} {:?}",
                value_now(),
                selected_options(b, form, annot)
            )));
        }
        Ok(true)
    })();
    b.FPDFPage_CloseAnnot(annot);
    result
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

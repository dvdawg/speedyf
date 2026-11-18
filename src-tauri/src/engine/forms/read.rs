//! Enumerating a document's form fields for the page overlay.
//!
//! Two sources, because neither has everything. PDFium's form-fill
//! environment knows the live state - names, values, flags, selections, the
//! font size in the default appearance - and is what the overlay must agree
//! with, since it is also what writes the values back. What it does not
//! expose is the rest of the field dictionaries: inherited /MaxLen and /Q,
//! /DV, the export half of /Opt pairs, the page's /Tabs, and the actions a
//! field carries. Those are read from the file itself with lopdf, matched to
//! PDFium's widgets by page and /Annots position (PDFium's annotation index
//! is the position in that array). When lopdf cannot read the file, the
//! fields still come back, with those extras at their defaults.

use super::{form_fill_info, read_utf16};
use crate::engine::render;
use crate::engine::text::DispMapper;
use crate::engine::types::{FormDto, FormFieldDto, FormOptionDto, RectDto};
use lopdf::{Dictionary, Document, LoadOptions, Object};
use pdfium_render::prelude::*;
use std::collections::HashMap;

/// `FPDF_GetFormType` values for documents carrying XFA.
const FORMTYPE_XFA_FULL: i32 = 2;
const FORMTYPE_XFA_FOREGROUND: i32 = 3;

/// Field flag bits (/Ff), PDF 32000 tables 221, 226 and 228.
const FF_READ_ONLY: i32 = 1 << 0;
const FF_REQUIRED: i32 = 1 << 1;
const FF_MULTILINE: i32 = 1 << 12;
const FF_PASSWORD: i32 = 1 << 13;
const FF_EDIT: i32 = 1 << 18;
const FF_MULTI_SELECT: i32 = 1 << 21;
const FF_DO_NOT_SCROLL: i32 = 1 << 23;
const FF_COMB: i32 = 1 << 24;

/// Annotation flags that keep a widget off the screen.
const ANNOT_HIDDEN: i32 = 1 << 1;
const ANNOT_NO_VIEW: i32 = 1 << 5;

/// Form-field additional-action events PDFium can report JavaScript for:
/// keystroke, format, validate, calculate.
const AA_EVENTS: [i32; 4] = [12, 13, 14, 15];

/// Files larger than this are not parsed a second time for the extras.
const MAX_DICT_PASS_BYTES: u64 = 128 << 20;

/// How deep a /Parent chain is followed before it is taken to be a loop.
const MAX_PARENT_DEPTH: usize = 32;

/// Every visible form widget in the document at `raw`, in display space, with
/// a document-wide Tab order.
pub fn read_form(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    page_count: u32,
    path: &str,
    password: Option<&str>,
) -> FormDto {
    let form_type = b.FPDF_GetFormType(raw);
    if form_type == 0 {
        return FormDto::default();
    }
    let xfa = form_type == FORMTYPE_XFA_FULL || form_type == FORMTYPE_XFA_FOREGROUND;

    // PDFium retains this pointer until ExitFormFillEnvironment(), so the
    // callback table must stay pinned for the complete scan.
    let mut info = Box::pin(form_fill_info());
    let form = b.FPDFDOC_InitFormFillEnvironment(raw, info.as_mut().get_mut());
    if form.is_null() {
        return FormDto {
            fields: Vec::new(),
            xfa,
        };
    }
    let dicts = FieldDicts::load(path, password);

    let mut fields = Vec::new();
    for pi in 0..page_count {
        let page = b.FPDF_LoadPage(raw, pi as i32);
        if page.is_null() {
            continue;
        }
        b.FORM_OnAfterLoadPage(page, form);
        // Widget rects come out of PDFium in user space; the frontend places
        // everything in display space, so they are mapped exactly as link
        // rects and character boxes are.
        let mapper = render::page_display_size(b, raw, pi as i32)
            .map(|[w, h]| DispMapper::new(b, page, w, h));
        for ai in 0..b.FPDFPage_GetAnnotCount(page).max(0) {
            let annot = b.FPDFPage_GetAnnot(page, ai);
            if annot.is_null() {
                continue;
            }
            let field_type = b.FPDFAnnot_GetFormFieldType(form, annot);
            let hidden = b.FPDFAnnot_GetFlags(annot) & (ANNOT_HIDDEN | ANNOT_NO_VIEW) != 0;
            if field_type >= 0 && !hidden {
                let extras = dicts.as_ref().and_then(|d| d.widgets.get(&(pi, ai as u32)));
                fields.push(read_widget(
                    b,
                    form,
                    annot,
                    field_type,
                    pi,
                    ai as u32,
                    mapper.as_ref(),
                    extras,
                ));
            }
            b.FPDFPage_CloseAnnot(annot);
        }
        b.FORM_OnBeforeClosePage(page, form);
        b.FPDF_ClosePage(page);
    }
    b.FPDFDOC_ExitFormFillEnvironment(form);

    let tabs = dicts.map(|d| d.tabs).unwrap_or_default();
    assign_tab_order(&mut fields, &tabs);
    FormDto { fields, xfa }
}

#[allow(clippy::too_many_arguments)]
fn read_widget(
    b: &dyn PdfiumLibraryBindings,
    form: FPDF_FORMHANDLE,
    annot: FPDF_ANNOTATION,
    field_type: i32,
    page: u32,
    index: u32,
    mapper: Option<&DispMapper>,
    extras: Option<&WidgetDict>,
) -> FormFieldDto {
    let kind = form_field_kind(field_type);
    let text =
        |read: &dyn Fn(*mut u16, std::os::raw::c_ulong) -> std::os::raw::c_ulong| read_utf16(read);
    let name = text(&|buffer, len| b.FPDFAnnot_GetFormFieldName(form, annot, buffer, len));
    let value = text(&|buffer, len| b.FPDFAnnot_GetFormFieldValue(form, annot, buffer, len));
    let export_value =
        text(&|buffer, len| b.FPDFAnnot_GetFormFieldExportValue(form, annot, buffer, len));
    let tooltip =
        text(&|buffer, len| b.FPDFAnnot_GetFormFieldAlternateName(form, annot, buffer, len));
    let flags = b.FPDFAnnot_GetFormFieldFlags(form, annot);
    let rect = mapper
        .and_then(|m| widget_rect(b, annot, m))
        .unwrap_or(RectDto {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });

    let mut font_size = 0f32;
    if !b.is_true(b.FPDFAnnot_GetFontSize(form, annot, &mut font_size)) || !font_size.is_finite() {
        font_size = 0.0;
    }
    let (mut r, mut g, mut bl) = (0u32, 0u32, 0u32);
    let color = if b.is_true(b.FPDFAnnot_GetFontColor(form, annot, &mut r, &mut g, &mut bl)) {
        format!("#{:02x}{:02x}{:02x}", r.min(255), g.min(255), bl.min(255))
    } else {
        "#000000".to_string()
    };

    let is_choice = kind == "combo" || kind == "list";
    let mut options = Vec::new();
    let mut selected = Vec::new();
    if is_choice {
        let count = b.FPDFAnnot_GetOptionCount(form, annot).max(0);
        for i in 0..count {
            let label =
                text(&|buffer, len| b.FPDFAnnot_GetOptionLabel(form, annot, i, buffer, len));
            let export = extras
                .and_then(|e| e.exports.get(i as usize))
                .cloned()
                .unwrap_or_else(|| label.clone());
            options.push(FormOptionDto { label, export });
            if b.is_true(b.FPDFAnnot_IsOptionSelected(form, annot, i)) {
                selected.push(i as u32);
            }
        }
    }

    let script_count = match extras {
        Some(e) => e.scripts,
        None => AA_EVENTS
            .iter()
            .filter(|event| {
                b.FPDFAnnot_GetFormAdditionalActionJavaScript(
                    form,
                    annot,
                    **event,
                    std::ptr::null_mut(),
                    0,
                ) > 2
            })
            .count() as u32,
    };

    let defaults: &[String] = extras.map(|e| e.default_value.as_slice()).unwrap_or(&[]);
    let default_selected: Vec<u32> = if is_choice {
        options
            .iter()
            .enumerate()
            .filter(|(_, o)| defaults.iter().any(|d| *d == o.export || *d == o.label))
            .map(|(i, _)| i as u32)
            .collect()
    } else {
        Vec::new()
    };
    let default_checked = (kind == "checkbox" || kind == "radio")
        && defaults.first().is_some_and(|d| {
            d != "Off"
                && (Some(d) == extras.and_then(|e| e.on_state.as_ref()) || *d == export_value)
        });
    let default_value = if kind == "text" || kind == "combo" {
        defaults.first().cloned().unwrap_or_default()
    } else {
        String::new()
    };

    FormFieldDto {
        name,
        kind: kind.into(),
        value,
        page,
        read_only: flags & FF_READ_ONLY != 0,
        index,
        rect,
        export_value,
        checked: b.FPDFAnnot_IsChecked(form, annot) != 0,
        signed: field_type == 7 && crate::engine::signature::field_is_signed(b, annot),
        tooltip,
        required: flags & FF_REQUIRED != 0,
        multiline: kind == "text" && flags & FF_MULTILINE != 0,
        password: kind == "text" && flags & FF_PASSWORD != 0,
        comb: kind == "text" && flags & FF_COMB != 0,
        no_scroll: kind == "text" && flags & FF_DO_NOT_SCROLL != 0,
        multi_select: kind == "list" && flags & FF_MULTI_SELECT != 0,
        editable: kind == "combo" && flags & FF_EDIT != 0,
        max_len: extras.map_or(0, |e| e.max_len),
        font_size,
        align: extras.map_or(0, |e| e.align),
        color,
        options,
        selected,
        default_value,
        default_selected,
        default_checked,
        script_count,
        tab_order: 0,
    }
}

/// One widget's rectangle, in display space, or `None` when PDFium reports
/// none or reports a degenerate one - a control with no area cannot be aimed
/// at, so offering it to the UI would only produce an invisible hit target.
fn widget_rect(
    b: &dyn PdfiumLibraryBindings,
    annotation: FPDF_ANNOTATION,
    mapper: &DispMapper,
) -> Option<RectDto> {
    let mut rect = FS_RECTF {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    if b.FPDFAnnot_GetRect(annotation, &mut rect) == 0 {
        return None;
    }
    let (x, y, w, h) = mapper.map_box(
        rect.left as f64,
        rect.right as f64,
        rect.bottom as f64,
        rect.top as f64,
    );
    (w > 0.0 && h > 0.0).then_some(RectDto { x, y, w, h })
}

pub(super) fn form_field_kind(field_type: i32) -> &'static str {
    match field_type {
        1 => "button",
        2 => "checkbox",
        3 => "radio",
        4 => "combo",
        5 => "list",
        6 => "text",
        7 => "signature",
        _ => "other",
    }
}

/// How a page orders its widgets for Tab (/Tabs, PDF 32000 12.5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabsOrder {
    /// Row by row, top to bottom, each left to right.
    Row,
    /// Column by column, left to right, each top to bottom.
    Column,
    /// The order of /Annots: what /S (structure) and an absent /Tabs come to
    /// for a form, whose widgets are listed in the order they were authored.
    Annots,
}

/// Number the fillable widgets in the order Tab visits them: page by page,
/// and within a page as its /Tabs says. Widgets nothing can focus (push
/// buttons, signatures) keep their place but are numbered too, after the
/// rest, so the order is total.
fn assign_tab_order(fields: &mut [FormFieldDto], tabs: &HashMap<u32, TabsOrder>) {
    let mut by_page: Vec<(u32, Vec<usize>)> = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        match by_page.last_mut() {
            Some((page, list)) if *page == f.page => list.push(i),
            _ => by_page.push((f.page, vec![i])),
        }
    }
    let mut next = 0u32;
    for (page, mut list) in by_page {
        let order = tabs.get(&page).copied().unwrap_or(TabsOrder::Annots);
        let rects: Vec<RectDto> = list.iter().map(|i| fields[*i].rect).collect();
        let positions = geometric_order(&rects, order);
        list = positions.into_iter().map(|p| list[p]).collect();
        for i in list {
            fields[i].tab_order = next;
            next += 1;
        }
    }
}

/// Positions into `rects` in visiting order.
pub(super) fn geometric_order(rects: &[RectDto], order: TabsOrder) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..rects.len()).collect();
    match order {
        TabsOrder::Annots => idx,
        TabsOrder::Row => {
            // Top edges in display space are y + h (y-up). Sort by top, then
            // sweep into rows: a widget whose top sits within half the
            // shorter height of the row's first one shares its line.
            let top = |i: usize| rects[i].y + rects[i].h;
            idx.sort_by(|a, b| top(*b).total_cmp(&top(*a)).then(a.cmp(b)));
            bands(idx, top, |i| rects[i].h, |i| rects[i].x, true)
        }
        TabsOrder::Column => {
            let left = |i: usize| rects[i].x;
            idx.sort_by(|a, b| left(*a).total_cmp(&left(*b)).then(a.cmp(b)));
            bands(
                idx,
                left,
                |i| rects[i].w,
                |i| -(rects[i].y + rects[i].h),
                false,
            )
        }
    }
}

/// Group `sorted` (ordered by `key`) into bands of widgets whose `key`s lie
/// within half the smaller `extent` of the band's first, and order each band
/// by `within`.
fn bands(
    sorted: Vec<usize>,
    key: impl Fn(usize) -> f32,
    extent: impl Fn(usize) -> f32,
    within: impl Fn(usize) -> f32,
    descending: bool,
) -> Vec<usize> {
    let mut out = Vec::with_capacity(sorted.len());
    let mut band: Vec<usize> = Vec::new();
    let flush = |band: &mut Vec<usize>, out: &mut Vec<usize>| {
        band.sort_by(|a, b| within(*a).total_cmp(&within(*b)).then(a.cmp(b)));
        out.append(band);
    };
    for i in sorted {
        if let Some(first) = band.first().copied() {
            let gap = if descending {
                key(first) - key(i)
            } else {
                key(i) - key(first)
            };
            if gap > 0.5 * extent(first).min(extent(i)) {
                flush(&mut band, &mut out);
            }
        }
        band.push(i);
    }
    flush(&mut band, &mut out);
    out
}

/// What the field dictionaries say about one widget beyond PDFium's reach.
#[derive(Debug, Default)]
pub(super) struct WidgetDict {
    pub max_len: u32,
    pub align: u8,
    /// /DV as strings: one for text, a combo or a button, several for a
    /// multi-select list.
    pub default_value: Vec<String>,
    /// Export values of /Opt, in order.
    pub exports: Vec<String>,
    /// This widget's "on" appearance state, for buttons.
    pub on_state: Option<String>,
    /// JavaScript actions on the widget and its field ancestry.
    pub scripts: u32,
}

/// The extras for every widget in a document, keyed by (page, /Annots index).
pub(super) struct FieldDicts {
    pub widgets: HashMap<(u32, u32), WidgetDict>,
    pub tabs: HashMap<u32, TabsOrder>,
}

impl FieldDicts {
    fn load(path: &str, password: Option<&str>) -> Option<FieldDicts> {
        let size = std::fs::metadata(path).ok()?.len();
        if size > MAX_DICT_PASS_BYTES {
            return None;
        }
        let options = match password {
            Some(password) => LoadOptions::with_password(password),
            None => LoadOptions::default(),
        };
        let doc = Document::load_with_options(path, options).ok()?;
        if doc.is_encrypted() && !doc.was_encrypted() {
            return None;
        }
        Some(Self::from_document(&doc))
    }

    pub(super) fn from_document(doc: &Document) -> FieldDicts {
        let acroform = doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"AcroForm").ok())
            .and_then(|o| deref_dict(doc, o));
        let form_q = acroform
            .and_then(|f| f.get(b"Q").ok())
            .and_then(|o| int_of(doc, o))
            .unwrap_or(0);

        let mut widgets = HashMap::new();
        let mut tabs = HashMap::new();
        for (number, page_id) in doc.get_pages() {
            let page_index = number.saturating_sub(1);
            let Ok(page) = doc.get_dictionary(page_id) else {
                continue;
            };
            let order = match page.get(b"Tabs").and_then(Object::as_name) {
                Ok(b"R") => TabsOrder::Row,
                Ok(b"C") => TabsOrder::Column,
                _ => TabsOrder::Annots,
            };
            tabs.insert(page_index, order);
            let Some(annots) = page
                .get(b"Annots")
                .ok()
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_array().ok())
            else {
                continue;
            };
            for (ai, entry) in annots.iter().enumerate() {
                let Some(widget) = deref_dict(doc, entry) else {
                    continue;
                };
                if !widget
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .is_ok_and(|n| n == b"Widget")
                {
                    continue;
                }
                widgets.insert(
                    (page_index, ai as u32),
                    widget_dict(doc, widget, form_q as u8),
                );
            }
        }
        FieldDicts { widgets, tabs }
    }
}

fn widget_dict(doc: &Document, widget: &Dictionary, form_q: u8) -> WidgetDict {
    let chain = ancestry(doc, widget);
    let inherited = |key: &[u8]| chain.iter().find_map(|d| d.get(key).ok());
    let max_len = inherited(b"MaxLen")
        .and_then(|o| int_of(doc, o))
        .map_or(0, |n| n.clamp(0, u32::MAX as i64) as u32);
    let align = inherited(b"Q")
        .and_then(|o| int_of(doc, o))
        .map_or(form_q, |q| q.clamp(0, 2) as u8);
    let default_value = inherited(b"DV")
        .and_then(|o| doc.dereference(o).ok())
        .map(|(_, o)| strings_of(doc, o))
        .unwrap_or_default();
    let exports = inherited(b"Opt")
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok())
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let item = doc.dereference(item).map_or(item, |(_, o)| o);
                    match item.as_array() {
                        Ok(pair) => pair.first().map(|o| text_of(doc, o)).unwrap_or_default(),
                        Err(_) => text_of(doc, item),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let on_state = widget
        .get(b"AP")
        .ok()
        .and_then(|o| deref_dict(doc, o))
        .and_then(|ap| ap.get(b"N").ok())
        .and_then(|o| deref_dict(doc, o))
        .and_then(|normal| {
            normal
                .iter()
                .map(|(k, _)| k)
                .find(|k| k.as_slice() != b"Off")
                .map(|k| String::from_utf8_lossy(k).into_owned())
        });
    let mut scripts = 0u32;
    if widget
        .get(b"A")
        .ok()
        .and_then(|o| deref_dict(doc, o))
        .is_some_and(is_javascript)
    {
        scripts += 1;
    }
    for dict in &chain {
        if let Some(aa) = dict.get(b"AA").ok().and_then(|o| deref_dict(doc, o)) {
            scripts += aa
                .iter()
                .filter(|(_, action)| deref_dict(doc, action).is_some_and(is_javascript))
                .count() as u32;
        }
    }
    WidgetDict {
        max_len,
        align,
        default_value,
        exports,
        on_state,
        scripts,
    }
}

/// The widget, then each field above it, nearest first.
fn ancestry<'d>(doc: &'d Document, widget: &'d Dictionary) -> Vec<&'d Dictionary> {
    let mut chain = vec![widget];
    let mut current = widget;
    while chain.len() < MAX_PARENT_DEPTH {
        let Some(parent) = current.get(b"Parent").ok().and_then(|o| deref_dict(doc, o)) else {
            break;
        };
        if chain.iter().any(|d| std::ptr::eq(*d, parent)) {
            break;
        }
        chain.push(parent);
        current = parent;
    }
    chain
}

fn is_javascript(action: &Dictionary) -> bool {
    action
        .get(b"S")
        .and_then(Object::as_name)
        .is_ok_and(|s| s == b"JavaScript")
}

fn deref_dict<'d>(doc: &'d Document, object: &'d Object) -> Option<&'d Dictionary> {
    doc.dereference(object).ok()?.1.as_dict().ok()
}

fn int_of(doc: &Document, object: &Object) -> Option<i64> {
    match doc.dereference(object).ok()?.1 {
        Object::Integer(n) => Some(*n),
        Object::Real(r) => Some(*r as i64),
        _ => None,
    }
}

fn text_of(doc: &Document, object: &Object) -> String {
    let object = doc.dereference(object).map_or(object, |(_, o)| o);
    match object {
        Object::Name(name) => String::from_utf8_lossy(name).into_owned(),
        Object::String(..) => lopdf::decode_text_string(object).unwrap_or_default(),
        _ => String::new(),
    }
}

fn strings_of(doc: &Document, object: &Object) -> Vec<String> {
    match object {
        Object::Array(items) => items.iter().map(|o| text_of(doc, o)).collect(),
        Object::String(..) | Object::Name(_) => vec![text_of(doc, object)],
        _ => Vec::new(),
    }
}

//! Prepare form mode's save: creating, changing, reordering and deleting
//! AcroForm fields.
//!
//! PDFium can fill a field but not make one, so this is written with lopdf,
//! as an incremental update appended to the file the rest of the save
//! produced. The bytes before it - and any signature over them - stay as
//! they were.
//!
//! What gets written is what Acrobat writes for the same fields: an
//! /AcroForm with /Fields, a default appearance (/DA) and resources (/DR)
//! holding Helvetica as /Helv and ZapfDingbats as /ZaDb; for each field a
//! dictionary with /FT, /T, /Ff, /V, /DV, /MaxLen, /Opt, /TU, /Q, /DA and,
//! for a format or a calculation, the standard AF* helper calls in /AA (and
//! /CO); and for each widget a /Rect on its page's /Annots, /MK, and a
//! generated /AP (`draw`). A radio group is one field with a widget kid per
//! button; every other new field is a single merged field-and-widget
//! dictionary.
//!
//! Existing widgets are found by field name and nearest rect, not by
//! annotation index: the save's own earlier steps can shift indices.

use super::appearance::text_string;
use super::draw::{self, Frame};
use crate::engine::save::PageSpace;
use crate::engine::types::{
    PlanDesignField, PlanFieldCalc, PlanFieldFormat, PlanFormDesign, PlanWidgetRef, RectDto,
};
use lopdf::{dictionary, Dictionary, IncrementalDocument, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};

/// Field flags (/Ff), PDF 32000 tables 221, 226, 228, 230.
pub(super) const FF_READ_ONLY: i64 = 1;
pub(super) const FF_REQUIRED: i64 = 1 << 1;
pub(super) const FF_MULTILINE: i64 = 1 << 12;
pub(super) const FF_NO_TOGGLE_OFF: i64 = 1 << 14;
pub(super) const FF_RADIO: i64 = 1 << 15;
pub(super) const FF_COMBO: i64 = 1 << 17;
pub(super) const FF_COMB: i64 = 1 << 24;

/// Annotation flag: print.
const ANNOT_PRINT: i64 = 4;

/// How deep a /Parent chain or a page tree is followed.
const MAX_DEPTH: usize = 32;

/// The default appearance a new AcroForm gets.
const DEFAULT_DA: &str = "/Helv 0 Tf 0 g";

/// The object store as this update sees it: what it already changed, else
/// the file as it was.
pub(super) struct Tx<'a> {
    pub inc: &'a mut IncrementalDocument,
}

impl Tx<'_> {
    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.inc
            .new_document
            .objects
            .get(&id)
            .or_else(|| self.inc.get_prev_documents().get_object(id).ok())
    }

    /// `object`, followed through references.
    pub fn resolve<'o>(&'o self, mut object: &'o Object) -> &'o Object {
        for _ in 0..MAX_DEPTH {
            match object {
                Object::Reference(id) => match self.get(*id) {
                    Some(next) => object = next,
                    None => return &Object::Null,
                },
                _ => return object,
            }
        }
        object
    }

    pub fn dict(&self, id: ObjectId) -> Option<Dictionary> {
        match self.get(id)? {
            Object::Dictionary(d) => Some(d.clone()),
            Object::Stream(s) => Some(s.dict.clone()),
            _ => None,
        }
    }

    pub fn put(&mut self, id: ObjectId, object: impl Into<Object>) {
        self.inc.new_document.objects.insert(id, object.into());
    }

    pub fn add(&mut self, object: impl Into<Object>) -> ObjectId {
        self.inc.new_document.add_object(object.into())
    }

    pub fn root(&self) -> Result<ObjectId, String> {
        self.inc
            .get_prev_documents()
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|e| e.to_string())
    }

    /// Page object ids in page order.
    pub fn pages(&self) -> Vec<ObjectId> {
        self.inc
            .get_prev_documents()
            .get_pages()
            .into_values()
            .collect()
    }

    /// A page attribute, looked up the page tree as /Rotate and the boxes
    /// are inherited.
    pub fn inherited(&self, page: ObjectId, key: &[u8]) -> Option<Object> {
        let mut id = page;
        for _ in 0..MAX_DEPTH {
            let d = self.dict(id)?;
            if let Ok(v) = d.get(key) {
                return Some(self.resolve(v).clone());
            }
            id = d.get(b"Parent").ok()?.as_reference().ok()?;
        }
        None
    }

    /// The display mapping and /Rotate of a page.
    pub fn page_space(&self, page: ObjectId) -> (PageSpace, u16) {
        let as_box = |o: Object| -> Option<[f32; 4]> {
            let a = o.as_array().ok()?;
            if a.len() != 4 {
                return None;
            }
            let v: Vec<f32> = a.iter().filter_map(|n| number(self.resolve(n))).collect();
            (v.len() == 4).then(|| {
                [
                    v[0].min(v[2]),
                    v[1].min(v[3]),
                    v[0].max(v[2]),
                    v[1].max(v[3]),
                ]
            })
        };
        let crop = self
            .inherited(page, b"CropBox")
            .and_then(as_box)
            .or_else(|| self.inherited(page, b"MediaBox").and_then(as_box))
            .unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let rot = self
            .inherited(page, b"Rotate")
            .and_then(|o| number(&o))
            .map_or(0, |r| ((r as i64).rem_euclid(360) / 90 * 90) as u16);
        (PageSpace::from_box(crop, rot), rot)
    }

    /// A page's /Annots, resolved.
    pub fn annots(&self, page: ObjectId) -> Vec<Object> {
        self.dict(page)
            .and_then(|d| d.get(b"Annots").ok().cloned())
            .map(|o| self.resolve(&o).clone())
            .and_then(|o| o.as_array().ok().cloned())
            .unwrap_or_default()
    }

    /// Replace a page's /Annots, in the array object it already uses when
    /// it is one.
    pub fn set_annots(&mut self, page: ObjectId, annots: Vec<Object>) {
        let Some(mut d) = self.dict(page) else {
            return;
        };
        if let Ok(Object::Reference(id)) = d.get(b"Annots") {
            let id = *id;
            if matches!(self.get(id), Some(Object::Array(_))) {
                self.put(id, Object::Array(annots));
                return;
            }
        }
        d.set("Annots", Object::Array(annots));
        self.put(page, d);
    }

    /// Text of a string or name object.
    pub fn text(&self, object: &Object) -> String {
        match self.resolve(object) {
            Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
            o @ Object::String(..) => lopdf::decode_text_string(o).unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The widget, then each field above it, nearest first.
    pub fn ancestry(&self, widget: ObjectId) -> Vec<ObjectId> {
        let mut chain = vec![widget];
        let mut current = widget;
        while chain.len() < MAX_DEPTH {
            let Some(parent) = self
                .dict(current)
                .and_then(|d| d.get(b"Parent").ok().and_then(|p| p.as_reference().ok()))
            else {
                break;
            };
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            current = parent;
        }
        chain
    }

    /// A field's fully qualified name.
    pub fn full_name(&self, id: ObjectId) -> String {
        let parts: Vec<String> = self
            .ancestry(id)
            .iter()
            .rev()
            .filter_map(|a| self.dict(*a)?.get(b"T").ok().map(|t| self.text(t)))
            .collect();
        parts.join(".")
    }

    /// The nearest dictionary in the ancestry that names a field.
    pub fn field_of(&self, widget: ObjectId) -> ObjectId {
        self.ancestry(widget)
            .into_iter()
            .find(|id| self.dict(*id).is_some_and(|d| d.has(b"T")))
            .unwrap_or(widget)
    }

    /// An attribute of a field, inherited down its ancestry.
    pub fn field_attr(&self, widget: ObjectId, key: &[u8]) -> Option<Object> {
        self.ancestry(widget).into_iter().find_map(|id| {
            self.dict(id)?
                .get(key)
                .ok()
                .map(|o| self.resolve(o).clone())
        })
    }
}

pub(super) fn number(object: &Object) -> Option<f32> {
    match object {
        Object::Integer(n) => Some(*n as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

fn rect_array(r: [f32; 4]) -> Object {
    Object::Array(r.iter().map(|v| Object::Real(*v)).collect())
}

/// A display-space rect as a user-space /Rect.
fn user_rect(space: &PageSpace, rect: &RectDto) -> [f32; 4] {
    let r = space.rect_to_user(rect);
    [
        r.left().value,
        r.bottom().value,
        r.right().value,
        r.top().value,
    ]
}

/// A widget already in the file.
#[derive(Clone, Debug)]
pub(super) struct Existing {
    pub page: usize,
    pub id: ObjectId,
    pub name: String,
    pub rect: RectDto,
}

/// Every widget on every page that is its own object.
pub(super) fn existing_widgets(tx: &Tx, pages: &[ObjectId]) -> Vec<Existing> {
    let mut out = Vec::new();
    for (pi, page) in pages.iter().enumerate() {
        let (space, _) = tx.page_space(*page);
        for entry in tx.annots(*page) {
            let Ok(id) = entry.as_reference() else {
                continue;
            };
            let Some(d) = tx.dict(id) else {
                continue;
            };
            if !d
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|n| n == b"Widget")
            {
                continue;
            }
            let rect = d
                .get(b"Rect")
                .ok()
                .map(|r| tx.resolve(r).clone())
                .and_then(|r| r.as_array().ok().cloned())
                .map(|a| {
                    a.iter()
                        .filter_map(|n| number(tx.resolve(n)))
                        .collect::<Vec<_>>()
                })
                .filter(|v| v.len() == 4)
                .map(|v| {
                    space.rect_from_user(&pdfium_render::prelude::PdfRect::new(
                        pdfium_render::prelude::PdfPoints::new(v[1].min(v[3])),
                        pdfium_render::prelude::PdfPoints::new(v[0].min(v[2])),
                        pdfium_render::prelude::PdfPoints::new(v[1].max(v[3])),
                        pdfium_render::prelude::PdfPoints::new(v[0].max(v[2])),
                    ))
                })
                .unwrap_or(RectDto {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                });
            out.push(Existing {
                page: pi,
                id,
                name: tx.full_name(id),
                rect,
            });
        }
    }
    out
}

/// Finds existing widgets for the frontend's references, each at most once.
pub(super) struct Resolver {
    widgets: Vec<Existing>,
    used: HashSet<ObjectId>,
    cache: HashMap<String, ObjectId>,
}

impl Resolver {
    pub fn new(widgets: Vec<Existing>) -> Self {
        Resolver {
            widgets,
            used: HashSet::new(),
            cache: HashMap::new(),
        }
    }

    pub fn page_of(&self, id: ObjectId) -> Option<usize> {
        self.widgets.iter().find(|w| w.id == id).map(|w| w.page)
    }

    pub fn names(&self) -> HashSet<String> {
        self.widgets.iter().map(|w| w.name.clone()).collect()
    }

    pub fn find(&mut self, r: &PlanWidgetRef) -> Option<ObjectId> {
        let key = format!(
            "{}|{}|{:.1},{:.1},{:.1},{:.1}",
            r.page, r.name, r.rect.x, r.rect.y, r.rect.w, r.rect.h
        );
        if let Some(id) = self.cache.get(&key) {
            return Some(*id);
        }
        let centre = |rc: &RectDto| (rc.x + rc.w / 2.0, rc.y + rc.h / 2.0);
        let (cx, cy) = centre(&r.rect);
        let distance = |w: &Existing| {
            let (x, y) = centre(&w.rect);
            (x - cx).abs()
                + (y - cy).abs()
                + (w.rect.w - r.rect.w).abs()
                + (w.rect.h - r.rect.h).abs()
        };
        let pick = |by_name: bool, used: &HashSet<ObjectId>| {
            self.widgets
                .iter()
                .filter(|w| w.page == r.page as usize && !used.contains(&w.id))
                .filter(|w| !by_name || w.name == r.name)
                .filter(|w| by_name || distance(w) < 2.0)
                .min_by(|a, b| distance(a).total_cmp(&distance(b)))
                .map(|w| w.id)
        };
        let id = pick(true, &self.used).or_else(|| pick(false, &self.used))?;
        self.used.insert(id);
        self.cache.insert(key, id);
        Some(id)
    }
}

/// The standard helper calls a format is written as: keystroke, format.
pub(super) fn format_calls(format: &PlanFieldFormat) -> (String, Option<String>) {
    let q = js_string;
    match format {
        PlanFieldFormat::Number {
            decimals,
            sep_style,
            neg_style,
            currency,
            prepend,
        } => {
            let args = format!(
                "{decimals}, {sep_style}, {neg_style}, 0, {}, {prepend}",
                q(currency)
            );
            (
                format!("AFNumber_Keystroke({args});"),
                Some(format!("AFNumber_Format({args});")),
            )
        }
        PlanFieldFormat::Percent {
            decimals,
            sep_style,
        } => (
            format!("AFPercent_Keystroke({decimals}, {sep_style});"),
            Some(format!("AFPercent_Format({decimals}, {sep_style});")),
        ),
        PlanFieldFormat::Date { format } => (
            format!("AFDate_KeystrokeEx({});", q(format)),
            Some(format!("AFDate_FormatEx({});", q(format))),
        ),
        PlanFieldFormat::Time { format } => (
            format!("AFTime_KeystrokeEx({});", q(format)),
            Some(format!("AFTime_FormatEx({});", q(format))),
        ),
        PlanFieldFormat::Special { style } => (
            format!("AFSpecial_Keystroke({style});"),
            Some(format!("AFSpecial_Format({style});")),
        ),
        // Acrobat's arbitrary mask is a keystroke script only.
        PlanFieldFormat::Mask { mask } => (format!("AFSpecial_KeystrokeEx({});", q(mask)), None),
    }
}

pub(super) fn calc_call(calc: &PlanFieldCalc) -> String {
    let names: Vec<String> = calc.fields.iter().map(|f| js_string(f)).collect();
    format!(
        "AFSimple_Calculate({}, new Array({}));",
        js_string(&calc.op.to_ascii_uppercase()),
        names.join(", ")
    )
}

fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn js_action(source: &str) -> Object {
    Object::Dictionary(dictionary! {
        "S" => "JavaScript",
        "JS" => text_string(source),
    })
}

/// `da` with its font size set to `size` (0 for auto), font and colour kept.
pub(super) fn da_with_size(da: &str, size: f32) -> String {
    let mut tokens: Vec<String> = da.split_whitespace().map(str::to_string).collect();
    if let Some(i) = tokens.iter().position(|t| t == "Tf") {
        if i >= 1 {
            tokens[i - 1] = draw::num(size);
            return tokens.join(" ");
        }
    }
    let rest = if tokens.is_empty() {
        "0 g".to_string()
    } else {
        tokens.join(" ")
    };
    format!("/Helv {} Tf {rest}", draw::num(size))
}

/// The AcroForm's fonts this update draws with.
struct Fonts {
    helv: Object,
    zadb: Object,
}

/// What is being built for the AcroForm, written at the end.
struct Form {
    id: ObjectId,
    dict: Dictionary,
    fields: Vec<Object>,
    calc_order: Vec<Object>,
    fonts: Fonts,
}

impl Form {
    fn open(tx: &mut Tx) -> Result<Form, String> {
        let root = tx.root()?;
        let mut catalog = tx.dict(root).ok_or("no catalog")?;
        let (id, dict) = match catalog.get(b"AcroForm").ok().cloned() {
            Some(Object::Reference(id)) => (id, tx.dict(id).unwrap_or_default()),
            Some(Object::Dictionary(d)) => {
                // Inline in the catalog: give it an object of its own.
                let id = tx.add(Object::Dictionary(d.clone()));
                catalog.set("AcroForm", id);
                tx.put(root, catalog);
                (id, d)
            }
            _ => {
                let id = tx.add(Object::Dictionary(Dictionary::new()));
                catalog.set("AcroForm", id);
                tx.put(root, catalog);
                (id, Dictionary::new())
            }
        };
        let list = |key: &[u8]| -> Vec<Object> {
            dict.get(key)
                .ok()
                .map(|o| tx.resolve(o).clone())
                .and_then(|o| o.as_array().ok().cloned())
                .unwrap_or_default()
        };
        let fields = list(b"Fields");
        let calc_order = list(b"CO");
        let mut form = Form {
            id,
            dict,
            fields,
            calc_order,
            fonts: Fonts {
                helv: Object::Null,
                zadb: Object::Null,
            },
        };
        form.fonts = form.ensure_fonts(tx);
        Ok(form)
    }

    /// /DR /Font with /Helv and /ZaDb in it, adding whichever is missing.
    fn ensure_fonts(&mut self, tx: &mut Tx) -> Fonts {
        let mut dr = self
            .dict
            .get(b"DR")
            .ok()
            .map(|o| tx.resolve(o).clone())
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default();
        let mut fonts = dr
            .get(b"Font")
            .ok()
            .map(|o| tx.resolve(o).clone())
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default();
        let mut entry = |tx: &mut Tx, key: &str, base: &str, winansi: bool| -> Object {
            if let Ok(existing) = fonts.get(key.as_bytes()) {
                return existing.clone();
            }
            let mut font = dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => base,
                "Name" => key,
            };
            if winansi {
                font.set("Encoding", "WinAnsiEncoding");
            }
            let id = tx.add(Object::Dictionary(font));
            fonts.set(key, id);
            Object::Reference(id)
        };
        let helv = entry(tx, "Helv", "Helvetica", true);
        let zadb = entry(tx, "ZaDb", "ZapfDingbats", false);
        dr.set("Font", Object::Dictionary(fonts));
        self.dict.set("DR", Object::Dictionary(dr));
        if !self.dict.has(b"DA") {
            self.dict.set("DA", text_string(DEFAULT_DA));
        }
        Fonts { helv, zadb }
    }

    fn close(mut self, tx: &mut Tx) {
        self.dict.set("Fields", Object::Array(self.fields));
        if self.calc_order.is_empty() {
            self.dict.remove(b"CO");
        } else {
            self.dict.set("CO", Object::Array(self.calc_order));
        }
        // Values written here would be shadowed by an XFA copy in Acrobat.
        self.dict.remove(b"XFA");
        tx.put(self.id, Object::Dictionary(self.dict));
    }
}

fn remove_ref(list: &mut Vec<Object>, id: ObjectId) -> bool {
    let before = list.len();
    list.retain(|o| o.as_reference().ok() != Some(id));
    list.len() != before
}

/// Remove a widget from its page and the field tree, and any field left
/// with no widgets.
fn delete_widget(tx: &mut Tx, form: &mut Form, page: ObjectId, widget: ObjectId) {
    let mut annots = tx.annots(page);
    if remove_ref(&mut annots, widget) {
        tx.set_annots(page, annots);
    }
    let mut child = widget;
    for _ in 0..MAX_DEPTH {
        let parent = tx
            .dict(child)
            .and_then(|d| d.get(b"Parent").ok().and_then(|p| p.as_reference().ok()));
        match parent {
            None => {
                remove_ref(&mut form.fields, child);
                remove_ref(&mut form.calc_order, child);
                return;
            }
            Some(parent) => {
                let Some(mut pd) = tx.dict(parent) else {
                    return;
                };
                let mut kids = pd
                    .get(b"Kids")
                    .ok()
                    .map(|o| tx.resolve(o).clone())
                    .and_then(|o| o.as_array().ok().cloned())
                    .unwrap_or_default();
                remove_ref(&mut kids, child);
                remove_ref(&mut form.calc_order, child);
                let empty = kids.is_empty();
                pd.set("Kids", Object::Array(kids));
                tx.put(parent, pd);
                if !empty {
                    return;
                }
                child = parent;
            }
        }
    }
}

/// /FT for a kind.
fn field_type(kind: &str) -> &'static str {
    match kind {
        "checkbox" | "radio" => "Btn",
        "combo" | "list" => "Ch",
        "signature" => "Sig",
        _ => "Tx",
    }
}

/// /Ff with the bits this kind's properties own set from `f`, the rest of
/// `old` kept.
fn flags(old: i64, f: &PlanDesignField) -> i64 {
    let mut ff = old & !(FF_READ_ONLY | FF_REQUIRED);
    if f.read_only {
        ff |= FF_READ_ONLY;
    }
    if f.required {
        ff |= FF_REQUIRED;
    }
    match f.kind.as_str() {
        "text" => {
            ff &= !(FF_MULTILINE | FF_COMB);
            if f.multiline {
                ff |= FF_MULTILINE;
            } else if f.comb && f.max_len > 0 {
                ff |= FF_COMB;
            }
        }
        "combo" => ff |= FF_COMBO,
        "list" => ff &= !FF_COMBO,
        "radio" => ff |= FF_RADIO | FF_NO_TOGGLE_OFF,
        _ => {}
    }
    ff
}

/// Field-level entries from `f` onto `d`: everything but the widget's own.
fn set_field_entries(d: &mut Dictionary, f: &PlanDesignField, old_ff: i64, da: &str, tx: &Tx) {
    d.set("FT", field_type(&f.kind));
    d.set("Ff", flags(old_ff, f));
    if f.tooltip.trim().is_empty() {
        d.remove(b"TU");
    } else {
        d.set("TU", text_string(f.tooltip.trim()));
    }
    match f.kind.as_str() {
        "text" if f.max_len > 0 => d.set("MaxLen", f.max_len as i64),
        "text" => {
            d.remove(b"MaxLen");
        }
        _ => {}
    }
    if matches!(f.kind.as_str(), "text" | "combo" | "list") {
        d.set("Q", f.align.min(2) as i64);
        d.set("DA", text_string(&da_with_size(da, f.font_size)));
    }
    if matches!(f.kind.as_str(), "combo" | "list") {
        let opts: Vec<Object> = f
            .options
            .iter()
            .map(|o| {
                if o.export == o.label || o.export.is_empty() {
                    text_string(&o.label)
                } else {
                    Object::Array(vec![text_string(&o.export), text_string(&o.label)])
                }
            })
            .collect();
        d.set("Opt", Object::Array(opts));
    }
    match f.kind.as_str() {
        "text" | "combo" | "list" => {
            if f.default_value.is_empty() {
                d.remove(b"DV");
            } else {
                d.set("DV", text_string(&f.default_value));
            }
        }
        "checkbox" | "radio" => {
            let on = !f.default_value.is_empty();
            d.set(
                "DV",
                Object::Name(if on {
                    f.default_value.as_bytes().to_vec()
                } else {
                    b"Off".to_vec()
                }),
            );
        }
        _ => {}
    }
    if f.format_set || f.calc_set {
        let mut aa = d
            .get(b"AA")
            .ok()
            .map(|o| tx.resolve(o).clone())
            .and_then(|o| o.as_dict().ok().cloned())
            .unwrap_or_default();
        if f.format_set {
            aa.remove(b"K");
            aa.remove(b"F");
            if let Some(format) = &f.format {
                let (k, fmt) = format_calls(format);
                aa.set("K", js_action(&k));
                if let Some(fmt) = fmt {
                    aa.set("F", js_action(&fmt));
                }
            }
        }
        if f.calc_set {
            aa.remove(b"C");
            if let Some(calc) = f.calc.as_ref().filter(|c| !c.fields.is_empty()) {
                aa.set("C", js_action(&calc_call(calc)));
            }
        }
        if aa.is_empty() {
            d.remove(b"AA");
        } else {
            d.set("AA", Object::Dictionary(aa));
        }
    }
}

/// /MK, /BS and /F for a widget.
fn set_widget_style(d: &mut Dictionary, f: &PlanDesignField, rot: u16, tx: &Tx) {
    let mut mk = d
        .get(b"MK")
        .ok()
        .map(|o| tx.resolve(o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    if f.border {
        mk.set("BC", Object::Array(vec![Object::Integer(0)]));
        d.set(
            "BS",
            Object::Dictionary(dictionary! { "W" => 1, "S" => "S" }),
        );
    } else {
        mk.remove(b"BC");
        d.remove(b"BS");
    }
    if rot != 0 {
        mk.set("R", rot as i64);
    } else {
        mk.remove(b"R");
    }
    match f.kind.as_str() {
        "checkbox" => mk.set("CA", text_string(&draw::CHECK_GLYPH.to_string())),
        "radio" => mk.set("CA", text_string(&draw::DOT_GLYPH.to_string())),
        _ => {}
    }
    d.set("MK", Object::Dictionary(mk));
}

/// A form XObject with `ops`, drawn in a `w` x `h` box on a page turned by
/// `rot`.
fn appearance_stream(
    tx: &mut Tx,
    fonts: &Fonts,
    ops: String,
    w: f32,
    h: f32,
    rot: u16,
) -> ObjectId {
    let mut font = Dictionary::new();
    font.set("Helv", fonts.helv.clone());
    font.set("ZaDb", fonts.zadb.clone());
    let mut d = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => rect_array([0.0, 0.0, w.max(0.0), h.max(0.0)]),
        "Resources" => dictionary! { "Font" => font },
    };
    if rot != 0 {
        d.set(
            "Matrix",
            Object::Array(
                draw::matrix(rot, w, h)
                    .iter()
                    .map(|v| Object::Real(*v))
                    .collect(),
            ),
        );
    }
    tx.add(Stream::new(d, ops.into_bytes()))
}

/// What a field holds now, for drawing it: /V as text and, for choices,
/// the selected option indices.
fn current_value(tx: &Tx, widget: ObjectId) -> Vec<String> {
    match tx.field_attr(widget, b"V") {
        Some(Object::Array(items)) => items.iter().map(|o| tx.text(o)).collect(),
        Some(o) => vec![tx.text(&o)],
        None => Vec::new(),
    }
}

/// Draw `f`'s appearance onto widget dictionary `d`. `values` is what the
/// field holds; `on` whether this button is on.
#[allow(clippy::too_many_arguments)]
fn set_appearance(
    tx: &mut Tx,
    fonts: &Fonts,
    d: &mut Dictionary,
    f: &PlanDesignField,
    values: &[String],
    on: bool,
    rot: u16,
) {
    let (w, h) = (f.rect.w, f.rect.h);
    let frame = Frame {
        w,
        h,
        border: f.border,
    };
    match f.kind.as_str() {
        "checkbox" | "radio" => {
            let glyph = if f.kind == "radio" {
                draw::DOT_GLYPH
            } else {
                draw::CHECK_GLYPH
            };
            let export = export_name(f);
            let on_id = appearance_stream(
                tx,
                fonts,
                draw::button_on_appearance(glyph, 0.0, &frame),
                w,
                h,
                rot,
            );
            let off_id = appearance_stream(tx, fonts, draw::off_appearance(&frame), w, h, rot);
            let mut normal = Dictionary::new();
            normal.set(export.as_bytes().to_vec(), on_id);
            normal.set("Off", off_id);
            d.set("AP", Object::Dictionary(dictionary! { "N" => normal }));
            d.set(
                "AS",
                Object::Name(if on {
                    export.into_bytes()
                } else {
                    b"Off".to_vec()
                }),
            );
        }
        kind => {
            let ops = match kind {
                "list" => {
                    let labels: Vec<String> = f.options.iter().map(|o| o.label.clone()).collect();
                    let selected: Vec<usize> = f
                        .options
                        .iter()
                        .enumerate()
                        .filter(|(_, o)| values.iter().any(|v| *v == o.export || *v == o.label))
                        .map(|(i, _)| i)
                        .collect();
                    draw::list_appearance(&labels, &selected, f.font_size, &frame)
                }
                "combo" => {
                    let value = values.first().cloned().unwrap_or_default();
                    let shown = f
                        .options
                        .iter()
                        .find(|o| o.export == value)
                        .map_or(value, |o| o.label.clone());
                    draw::text_appearance(&shown, f.font_size, &frame, f.align, false, 0)
                }
                "text" => draw::text_appearance(
                    values.first().map_or("", String::as_str),
                    f.font_size,
                    &frame,
                    f.align,
                    f.multiline,
                    if f.comb && !f.multiline { f.max_len } else { 0 },
                ),
                _ => draw::off_appearance(&frame),
            };
            let id = appearance_stream(tx, fonts, ops, w, h, rot);
            d.set("AP", Object::Dictionary(dictionary! { "N" => id }));
        }
    }
}

/// A button's on-state name: its export value, which must be a usable
/// name, or "Yes".
fn export_name(f: &PlanDesignField) -> String {
    let cleaned: String = f
        .export_value
        .chars()
        .filter(|c| c.is_ascii_graphic() && !"()<>[]{}/%#".contains(*c))
        .collect();
    if cleaned.is_empty() || cleaned == "Off" {
        "Yes".into()
    } else {
        cleaned
    }
}

/// Rename a button widget's on-state, in /AP, /AS and its field's /V.
fn rename_on_state(tx: &mut Tx, widget: ObjectId, d: &mut Dictionary, new: &str) {
    let Some(ap) = d
        .get(b"AP")
        .ok()
        .map(|o| tx.resolve(o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
    else {
        return;
    };
    let old = ap
        .get(b"N")
        .ok()
        .map(|o| tx.resolve(o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
        .and_then(|n| {
            n.iter()
                .map(|(k, _)| k.clone())
                .find(|k| k.as_slice() != b"Off")
        });
    let Some(old) = old else {
        return;
    };
    if old == new.as_bytes() {
        return;
    }
    let mut ap = ap;
    for key in [b"N".as_slice(), b"D".as_slice()] {
        let Some(mut states) = ap
            .get(key)
            .ok()
            .map(|o| tx.resolve(o).clone())
            .and_then(|o| o.as_dict().ok().cloned())
        else {
            continue;
        };
        if let Some(stream) = states.remove(&old) {
            states.set(new.as_bytes().to_vec(), stream);
        }
        ap.set(key.to_vec(), Object::Dictionary(states));
    }
    d.set("AP", Object::Dictionary(ap));
    if d.get(b"AS").and_then(Object::as_name).ok() == Some(old.as_slice()) {
        d.set("AS", Object::Name(new.as_bytes().to_vec()));
    }
    let field = tx.field_of(widget);
    if field != widget {
        if let Some(mut fd) = tx.dict(field) {
            if fd.get(b"V").and_then(Object::as_name).ok() == Some(old.as_slice()) {
                fd.set("V", Object::Name(new.as_bytes().to_vec()));
                tx.put(field, fd);
            }
        }
    } else if d.get(b"V").and_then(Object::as_name).ok() == Some(old.as_slice()) {
        d.set("V", Object::Name(new.as_bytes().to_vec()));
    }
}

/// `name` made unique against `taken`, by a numeric suffix.
fn unique(name: &str, taken: &HashSet<String>) -> String {
    let base = if name.trim().is_empty() {
        "Field"
    } else {
        name.trim()
    };
    // A dot would make the name a path in the field tree.
    let base = base.replace('.', " ");
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|n| !taken.contains(n))
        .unwrap_or(base)
}

/// Apply Prepare form mode's changes to the file at `path`. Returns whether
/// anything was written.
pub fn apply_design(path: &std::path::Path, design: &PlanFormDesign) -> Result<bool, String> {
    let structural =
        !design.fields.is_empty() || !design.deleted.is_empty() || !design.tab_orders.is_empty();
    if !structural {
        return Ok(false);
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut inc: IncrementalDocument = bytes
        .as_slice()
        .try_into()
        .map_err(|e: lopdf::Error| e.to_string())?;
    if inc.get_prev_documents().is_encrypted() {
        return Err("the form cannot be changed in an encrypted file".into());
    }
    // New objects are numbered past the previous trailer's /Size, which
    // counts an xref stream lopdf does not.
    let declared = inc
        .get_prev_documents()
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    {
        let mut tx = Tx { inc: &mut inc };
        build(&mut tx, design)?;
    }
    let mut out = Vec::with_capacity(bytes.len() + 16 * 1024);
    inc.save_to(&mut out).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, &out).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(true)
}

fn build(tx: &mut Tx, design: &PlanFormDesign) -> Result<(), String> {
    let pages = tx.pages();
    let mut resolver = Resolver::new(existing_widgets(tx, &pages));
    let mut taken = resolver.names();
    let mut form = Form::open(tx)?;
    let acro_da = form
        .dict
        .get(b"DA")
        .ok()
        .map(|o| tx.text(o))
        .unwrap_or_else(|| DEFAULT_DA.to_string());

    // Resolve every reference first, so a deletion cannot be mistaken for
    // the widget a later change means.
    let origins: Vec<Option<ObjectId>> = design
        .fields
        .iter()
        .map(|f| f.origin.as_ref().and_then(|o| resolver.find(o)))
        .collect();
    let deleted: Vec<ObjectId> = design
        .deleted
        .iter()
        .filter_map(|r| resolver.find(r))
        .collect();
    let mut tab_ids: Vec<(u32, String, Vec<Option<ObjectId>>)> = Vec::new();
    for order in &design.tab_orders {
        let ids = order
            .order
            .iter()
            .map(|e| e.widget.as_ref().and_then(|w| resolver.find(w)))
            .collect();
        tab_ids.push((order.page, order.mode.clone(), ids));
    }

    for id in &deleted {
        if let Some(page) = resolver.page_of(*id).and_then(|p| pages.get(p)) {
            delete_widget(tx, &mut form, *page, *id);
        }
    }

    let mut by_key: HashMap<String, ObjectId> = HashMap::new();
    // Radio groups made in this save, by name.
    let mut radio_parents: HashMap<String, ObjectId> = HashMap::new();
    let mut calc_fields: Vec<(ObjectId, bool)> = Vec::new();

    for (f, origin) in design.fields.iter().zip(&origins) {
        let Some(page) = pages.get(f.page as usize).copied() else {
            continue;
        };
        let (space, rot) = tx.page_space(page);
        let rect = user_rect(&space, &f.rect);
        match (f.origin.as_ref(), origin) {
            (Some(_), Some(widget)) => {
                change_existing(
                    tx, &form, &resolver, &pages, page, *widget, f, rect, rot, &acro_da,
                );
                by_key.insert(f.key.clone(), *widget);
                if f.calc_set {
                    calc_fields.push((tx.field_of(*widget), f.calc.is_some()));
                }
            }
            (Some(_), None) => {
                log::warn!("prepare form: field {:?} is no longer in the file", f.name);
            }
            (None, _) => {
                let widget = create(
                    tx,
                    &mut form,
                    &mut taken,
                    &mut radio_parents,
                    page,
                    f,
                    rect,
                    rot,
                );
                by_key.insert(f.key.clone(), widget);
                if f.calc.is_some() {
                    calc_fields.push((tx.field_of(widget), true));
                }
            }
        }
    }

    for (field, has) in calc_fields {
        remove_ref(&mut form.calc_order, field);
        if has {
            form.calc_order.push(Object::Reference(field));
        }
    }

    for ((page_index, mode, refs), order) in tab_ids.into_iter().zip(&design.tab_orders) {
        let Some(page) = pages.get(page_index as usize).copied() else {
            continue;
        };
        let wanted: Vec<ObjectId> = order
            .order
            .iter()
            .zip(refs)
            .filter_map(|(entry, r)| {
                r.or_else(|| entry.key.as_ref().and_then(|k| by_key.get(k).copied()))
            })
            .collect();
        reorder_widgets(tx, page, &wanted, &mode);
    }

    form.close(tx);
    Ok(())
}

/// Put a page's widgets in `wanted` order (others after them, as they
/// were), keeping every other annotation where it is, and set /Tabs.
pub(super) fn reorder_widgets(tx: &mut Tx, page: ObjectId, wanted: &[ObjectId], mode: &str) {
    let annots = tx.annots(page);
    let is_widget = |tx: &Tx, o: &Object| {
        o.as_reference()
            .ok()
            .and_then(|id| tx.dict(id))
            .is_some_and(|d| d.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget"))
    };
    let slots: Vec<usize> = annots
        .iter()
        .enumerate()
        .filter(|(_, o)| is_widget(tx, o))
        .map(|(i, _)| i)
        .collect();
    let present: Vec<ObjectId> = slots
        .iter()
        .filter_map(|i| annots[*i].as_reference().ok())
        .collect();
    let mut ordered: Vec<ObjectId> = Vec::new();
    for id in wanted {
        if present.contains(id) && !ordered.contains(id) {
            ordered.push(*id);
        }
    }
    for id in &present {
        if !ordered.contains(id) {
            ordered.push(*id);
        }
    }
    let mut next = annots.clone();
    for (slot, id) in slots.iter().zip(ordered) {
        next[*slot] = Object::Reference(id);
    }
    if next != annots {
        tx.set_annots(page, next);
    }
    let tabs = match mode {
        "R" => "R",
        "C" => "C",
        _ => "S",
    };
    if let Some(mut d) = tx.dict(page) {
        d.set("Tabs", tabs);
        tx.put(page, d);
    }
}

#[allow(clippy::too_many_arguments)]
fn change_existing(
    tx: &mut Tx,
    form: &Form,
    resolver: &Resolver,
    pages: &[ObjectId],
    page: ObjectId,
    widget: ObjectId,
    f: &PlanDesignField,
    rect: [f32; 4],
    rot: u16,
    acro_da: &str,
) {
    // Moved to another page: off the old /Annots, onto the new.
    let old_page = resolver.page_of(widget).and_then(|p| pages.get(p)).copied();
    if let Some(old) = old_page.filter(|p| *p != page) {
        let mut annots = tx.annots(old);
        remove_ref(&mut annots, widget);
        tx.set_annots(old, annots);
        let mut annots = tx.annots(page);
        annots.push(Object::Reference(widget));
        tx.set_annots(page, annots);
    }
    let field = tx.field_of(widget);
    let old_ff = tx
        .field_attr(widget, b"Ff")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0);
    let da = tx
        .field_attr(widget, b"DA")
        .map(|o| tx.text(&o))
        .unwrap_or_else(|| acro_da.to_string());

    // Field level, on the field dictionary (the widget itself when merged).
    let mut fd = tx.dict(field).unwrap_or_default();
    set_field_entries(&mut fd, f, old_ff, &da, tx);
    // A new name: the last part of the fully qualified one, under the
    // same parent.
    let old_full = tx.full_name(field);
    if f.name != old_full && !f.name.trim().is_empty() {
        let prefix = old_full
            .rsplit_once('.')
            .map(|(p, _)| format!("{p}."))
            .unwrap_or_default();
        let terminal = f.name.strip_prefix(&prefix).unwrap_or(&f.name);
        fd.set("T", text_string(&terminal.replace('.', " ")));
    }
    // Widget level.
    let mut wd = if field == widget {
        fd
    } else {
        tx.put(field, fd);
        tx.dict(widget).unwrap_or_default()
    };
    wd.set("Rect", rect_array(rect));
    wd.set("P", page);
    set_widget_style(&mut wd, f, rot, tx);
    let buttons = matches!(f.kind.as_str(), "checkbox" | "radio");
    if buttons && !f.export_value.is_empty() {
        rename_on_state(tx, widget, &mut wd, &export_name(f));
    }
    if f.restyle {
        let values = current_value(tx, widget);
        let on = buttons
            && wd
                .get(b"AS")
                .and_then(Object::as_name)
                .is_ok_and(|s| s != b"Off");
        set_appearance(tx, &form.fonts, &mut wd, f, &values, on, rot);
    }
    tx.put(widget, wd);
}

#[allow(clippy::too_many_arguments)]
fn create(
    tx: &mut Tx,
    form: &mut Form,
    taken: &mut HashSet<String>,
    radio_parents: &mut HashMap<String, ObjectId>,
    page: ObjectId,
    f: &PlanDesignField,
    rect: [f32; 4],
    rot: u16,
) -> ObjectId {
    let mut w = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "Rect" => rect_array(rect),
        "P" => page,
        "F" => ANNOT_PRINT,
    };
    set_widget_style(&mut w, f, rot, tx);
    let da = if matches!(f.kind.as_str(), "checkbox" | "radio") {
        "/ZaDb 0 Tf 0 g".to_string()
    } else {
        format!("/Helv {} Tf 0 g", draw::num(f.font_size))
    };

    let widget = if f.kind == "radio" {
        let name = f.name.trim().replace('.', " ");
        let parent = match radio_parents.get(&name) {
            Some(id) => *id,
            None => {
                let mut pd = Dictionary::new();
                set_field_entries(&mut pd, f, 0, &da, tx);
                pd.set("T", text_string(&name));
                pd.set("DA", text_string(&da));
                pd.set("Kids", Object::Array(Vec::new()));
                let on = !f.default_value.is_empty();
                pd.set(
                    "V",
                    Object::Name(if on {
                        f.default_value.as_bytes().to_vec()
                    } else {
                        b"Off".to_vec()
                    }),
                );
                let id = tx.add(Object::Dictionary(pd));
                form.fields.push(Object::Reference(id));
                taken.insert(name.clone());
                radio_parents.insert(name.clone(), id);
                id
            }
        };
        let on = !f.default_value.is_empty() && export_name(f) == f.default_value;
        set_appearance(tx, &form.fonts, &mut w, f, &[], on, rot);
        w.set("Parent", parent);
        let id = tx.add(Object::Dictionary(w));
        if let Some(mut pd) = tx.dict(parent) {
            let mut kids = pd
                .get(b"Kids")
                .ok()
                .and_then(|o| o.as_array().ok().cloned())
                .unwrap_or_default();
            kids.push(Object::Reference(id));
            pd.set("Kids", Object::Array(kids));
            tx.put(parent, pd);
        }
        id
    } else {
        let name = unique(&f.name, taken);
        taken.insert(name.clone());
        set_field_entries(&mut w, f, 0, &da, tx);
        w.set("T", text_string(&name));
        w.set("DA", text_string(&da));
        let values: Vec<String> = if f.default_value.is_empty() {
            Vec::new()
        } else {
            vec![f.default_value.clone()]
        };
        match f.kind.as_str() {
            "text" | "combo" | "list" if !f.default_value.is_empty() => {
                w.set("V", text_string(&f.default_value));
            }
            "checkbox" => {
                let on = !f.default_value.is_empty();
                w.set(
                    "V",
                    Object::Name(if on {
                        export_name(f).into_bytes()
                    } else {
                        b"Off".to_vec()
                    }),
                );
            }
            _ => {}
        }
        let on = f.kind == "checkbox" && !f.default_value.is_empty();
        set_appearance(tx, &form.fonts, &mut w, f, &values, on, rot);
        let id = tx.add(Object::Dictionary(w));
        form.fields.push(Object::Reference(id));
        id
    };
    let mut annots = tx.annots(page);
    annots.push(Object::Reference(widget));
    tx.set_annots(page, annots);
    widget
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_appearance_sizes() {
        assert_eq!(da_with_size("/Helv 0 Tf 0 g", 9.0), "/Helv 9 Tf 0 g");
        assert_eq!(
            da_with_size("/Cour 12 Tf 1 0 0 rg", 0.0),
            "/Cour 0 Tf 1 0 0 rg"
        );
        assert_eq!(da_with_size("0 g", 10.0), "/Helv 10 Tf 0 g");
        assert_eq!(da_with_size("", 8.5), "/Helv 8.5 Tf 0 g");
    }

    #[test]
    fn helper_calls_round_trip_through_the_reader() {
        use super::super::scripts::{FieldBehaviour, FieldFormat};
        let cases = [
            PlanFieldFormat::Number {
                decimals: 2,
                sep_style: 0,
                neg_style: 1,
                currency: "$".into(),
                prepend: true,
            },
            PlanFieldFormat::Percent {
                decimals: 1,
                sep_style: 1,
            },
            PlanFieldFormat::Date {
                format: "mm/dd/yyyy".into(),
            },
            PlanFieldFormat::Time {
                format: "HH:MM".into(),
            },
            PlanFieldFormat::Special { style: 3 },
            PlanFieldFormat::Mask {
                mask: "999-AAA".into(),
            },
        ];
        for case in cases {
            let (k, f) = format_calls(&case);
            let mut b = FieldBehaviour::default();
            assert!(b.take(b"K", &k), "{k}");
            if let Some(f) = f {
                assert!(b.take(b"F", &f), "{f}");
            }
            assert!(b.keystroke.is_some());
            if let PlanFieldFormat::Date { .. } = case {
                assert_eq!(
                    b.format,
                    Some(FieldFormat::Date {
                        format: "mm/dd/yyyy".into()
                    })
                );
            }
        }
        let calc = calc_call(&PlanFieldCalc {
            op: "sum".into(),
            fields: vec!["a".into(), "b \"c\"".into()],
        });
        let mut b = FieldBehaviour::default();
        assert!(b.take(b"C", &calc), "{calc}");
        let c = b.calculate.unwrap();
        assert_eq!(c.op, "SUM");
        assert_eq!(c.fields, vec!["a".to_string(), "b \"c\"".to_string()]);
    }

    #[test]
    fn names_are_made_unique() {
        let taken: HashSet<String> = ["Name".to_string(), "Name 2".to_string()].into();
        assert_eq!(unique("Name", &taken), "Name 3");
        assert_eq!(unique("a.b", &taken), "a b");
        assert_eq!(unique("  ", &taken), "Field");
    }
}

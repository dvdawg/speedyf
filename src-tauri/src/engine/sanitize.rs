//! Remove hidden information: what a document carries besides what its pages
//! show.
//!
//! `scan` lists what is present; `clean` removes the chosen kinds in place on
//! a lopdf document, which the caller then writes as a full clean rewrite
//! (`redact::clean_rewrite`) - so earlier revisions and unreferenced objects
//! go too, whatever else was chosen.

use crate::engine::redact::fonts::{read_font, FontInfo};
use crate::engine::redact::geom::{number, Matrix, Rect};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

/// The kinds of hidden information to remove.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SanitizeOptions {
    #[serde(default)]
    pub metadata: bool,
    #[serde(default)]
    pub attachments: bool,
    #[serde(default)]
    pub javascript: bool,
    #[serde(default)]
    pub hidden_layers: bool,
    #[serde(default)]
    pub hidden_text: bool,
    #[serde(default)]
    pub comments: bool,
    #[serde(default)]
    pub forms: bool,
    #[serde(default)]
    pub links: bool,
    #[serde(default)]
    pub bookmarks: bool,
    #[serde(default)]
    pub thumbnails: bool,
    #[serde(default)]
    pub private_data: bool,
}

/// One kind of hidden information found.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HiddenItemDto {
    /// The `SanitizeOptions` field it maps to, or "revisions".
    pub key: String,
    pub count: u32,
    /// A short list of what was found (key names, file names, hosts).
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct HiddenInfoDto {
    pub items: Vec<HiddenItemDto>,
    /// The document carries a digital signature; removing anything breaks it.
    pub signed: bool,
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

fn dict<'a>(doc: &'a Document, o: Option<&'a Object>) -> Option<&'a Dictionary> {
    match deref(doc, o?) {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn name_of<'a>(d: &'a Dictionary, key: &[u8]) -> &'a [u8] {
    d.get(key).and_then(Object::as_name).unwrap_or(b"")
}

const COMMENT_SUBTYPES: &[&[u8]] = &[
    b"Text",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Stamp",
    b"Caret",
    b"Ink",
    b"Popup",
    b"Sound",
];

const EXTERNAL_ACTIONS: &[&[u8]] = &[
    b"URI",
    b"Launch",
    b"GoToR",
    b"GoToE",
    b"SubmitForm",
    b"ImportData",
];

/// The names in a name tree (keys of /Names arrays, through /Kids).
fn name_tree_names(doc: &Document, root: &Dictionary, out: &mut Vec<String>, depth: u32) {
    if depth > 32 {
        return;
    }
    if let Some(Object::Array(names)) = root.get(b"Names").ok().map(|o| deref(doc, o)) {
        for pair in names.chunks(2) {
            if let Some(n) = pair.first().and_then(|o| o.as_str().ok()) {
                out.push(String::from_utf8_lossy(n).chars().take(60).collect());
            }
        }
    }
    if let Some(Object::Array(kids)) = root.get(b"Kids").ok().map(|o| deref(doc, o)) {
        for k in kids {
            if let Some(d) = dict(doc, Some(k)) {
                name_tree_names(doc, d, out, depth + 1);
            }
        }
    }
}

fn names_entry<'a>(doc: &'a Document, key: &[u8]) -> Option<&'a Dictionary> {
    let names = dict(doc, doc.catalog().ok()?.get(b"Names").ok())?;
    dict(doc, names.get(key).ok())
}

/// Visit every dictionary in the document, direct ones included.
fn each_dict(doc: &Document, mut f: impl FnMut(&Dictionary)) {
    fn walk(o: &Object, f: &mut dyn FnMut(&Dictionary), depth: u32) {
        if depth > 64 {
            return;
        }
        match o {
            Object::Dictionary(d) => {
                f(d);
                for (_, v) in d.iter() {
                    walk(v, f, depth + 1);
                }
            }
            Object::Stream(s) => {
                f(&s.dict);
                for (_, v) in s.dict.iter() {
                    walk(v, f, depth + 1);
                }
            }
            Object::Array(a) => {
                for v in a {
                    walk(v, f, depth + 1);
                }
            }
            _ => {}
        }
    }
    for o in doc.objects.values() {
        walk(o, &mut f, 0);
    }
    walk(&Object::Dictionary(doc.trailer.clone()), &mut f, 0);
}

fn each_dict_mut(doc: &mut Document, mut f: impl FnMut(&mut Dictionary)) {
    fn walk(o: &mut Object, f: &mut dyn FnMut(&mut Dictionary), depth: u32) {
        if depth > 64 {
            return;
        }
        match o {
            Object::Dictionary(d) => {
                f(d);
                for (_, v) in d.iter_mut() {
                    walk(v, f, depth + 1);
                }
            }
            Object::Stream(s) => {
                f(&mut s.dict);
                for (_, v) in s.dict.iter_mut() {
                    walk(v, f, depth + 1);
                }
            }
            Object::Array(a) => {
                for v in a {
                    walk(v, f, depth + 1);
                }
            }
            _ => {}
        }
    }
    for o in doc.objects.values_mut() {
        walk(o, &mut f, 0);
    }
}

fn is_action(doc: &Document, o: &Object, kinds: &[&[u8]]) -> bool {
    dict(doc, Some(o)).is_some_and(|a| kinds.contains(&name_of(a, b"S")))
}

fn host_of(uri: &[u8]) -> String {
    let s = String::from_utf8_lossy(uri);
    let rest = s.split("://").nth(1).unwrap_or(&s);
    rest.split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .chars()
        .take(60)
        .collect()
}

/// Every annotation of every page: `(page, annotation entry)`.
fn annotations(doc: &Document) -> Vec<(ObjectId, Object)> {
    let mut out = Vec::new();
    for page in doc.get_pages().values() {
        if let Some(Object::Array(a)) = doc
            .get_dictionary(*page)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .map(|o| deref(doc, o))
        {
            out.extend(a.iter().map(|e| (*page, e.clone())));
        }
    }
    out
}

/// The OCGs hidden in the default configuration.
fn hidden_ocgs(doc: &Document) -> HashSet<ObjectId> {
    let mut out = HashSet::new();
    let Some(props) = doc
        .catalog()
        .ok()
        .and_then(|c| dict(doc, c.get(b"OCProperties").ok()))
    else {
        return out;
    };
    let ids = |o: Option<&Object>| -> Vec<ObjectId> {
        match o.map(|o| deref(doc, o)) {
            Some(Object::Array(a)) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => Vec::new(),
        }
    };
    let all = ids(props.get(b"OCGs").ok());
    let Some(d) = dict(doc, props.get(b"D").ok()) else {
        return out;
    };
    if name_of(d, b"BaseState") == b"OFF" {
        let on: HashSet<ObjectId> = ids(d.get(b"ON").ok()).into_iter().collect();
        out.extend(all.into_iter().filter(|g| !on.contains(g)));
    } else {
        out.extend(ids(d.get(b"OFF").ok()));
    }
    out
}

/// Whether an /OC entry (an OCG or an OCMD) is hidden.
fn oc_hidden(doc: &Document, oc: &Object, hidden: &HashSet<ObjectId>) -> bool {
    if let Ok(id) = oc.as_reference() {
        if hidden.contains(&id) {
            return true;
        }
    }
    let Some(d) = dict(doc, Some(oc)) else {
        return false;
    };
    if name_of(d, b"Type") != b"OCMD" {
        return false;
    }
    let members: Vec<ObjectId> = match d.get(b"OCGs").ok() {
        Some(Object::Reference(id)) => vec![*id],
        Some(o) => match deref(doc, o) {
            Object::Array(a) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    };
    if members.is_empty() {
        return false;
    }
    let off = members.iter().filter(|m| hidden.contains(m)).count();
    match name_of(d, b"P") {
        b"AllOn" => off > 0,
        b"AnyOff" => off == 0,
        b"AllOff" => off < members.len(),
        _ => off == members.len(), // AnyOn
    }
}

/// Content streams to rewrite: every page's, and every form XObject's.
fn content_targets(doc: &Document) -> (Vec<ObjectId>, Vec<ObjectId>) {
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let forms = doc
        .objects
        .iter()
        .filter_map(|(id, o)| match o {
            Object::Stream(s) if name_of(&s.dict, b"Subtype") == b"Form" => Some(*id),
            _ => None,
        })
        .collect();
    (pages, forms)
}

fn page_resources(doc: &Document, page: ObjectId) -> Dictionary {
    match crate::engine::redact::inherited(doc, page, b"Resources") {
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    }
}

fn page_bytes(doc: &Document, page: ObjectId) -> Option<Vec<u8>> {
    let d = doc.get_dictionary(page).ok()?;
    let refs: Vec<Object> = match d.get(b"Contents").ok() {
        None => return Some(Vec::new()),
        Some(o) => match deref(doc, o) {
            Object::Array(a) => a.clone(),
            _ => vec![o.clone()],
        },
    };
    let mut out = Vec::new();
    for r in refs {
        if let Object::Stream(s) = deref(doc, &r) {
            out.extend(s.decompressed_content().ok()?);
            out.push(b'\n');
        }
    }
    Some(out)
}

#[derive(Default, Debug, Clone, Copy)]
struct ContentCounts {
    hidden_text_ops: u32,
    hidden_layer_sections: u32,
}

/// The shared content pass: counts (and with `apply`, removes) invisible
/// text (render mode 3) and content in hidden optional-content groups.
fn rewrite_ops(
    doc: &Document,
    ops: Vec<Operation>,
    resources: &Dictionary,
    hidden: &HashSet<ObjectId>,
    (remove_text, remove_layers): (bool, bool),
    fonts: &mut HashMap<Vec<u8>, FontInfo>,
    counts: &mut ContentCounts,
) -> (Vec<Operation>, bool) {
    let mut out = Vec::with_capacity(ops.len());
    let mut changed = false;
    // Render mode, font, size, spacing, scaling; pushed by q.
    #[derive(Clone)]
    struct T {
        tr: i64,
        font: Vec<u8>,
        size: f32,
        tc: f32,
        tw: f32,
        th: f32,
    }
    let mut t = T {
        tr: 0,
        font: Vec::new(),
        size: 0.0,
        tc: 0.0,
        tw: 0.0,
        th: 1.0,
    };
    let mut stack: Vec<T> = Vec::new();
    // Depth of marked content; Some(depth) while inside a hidden section.
    let mut depth = 0u32;
    let mut skipping: Option<u32> = None;
    let props = |n: &[u8]| -> Option<Object> {
        let p = dict(doc, resources.get(b"Properties").ok())?;
        p.get(n).ok().cloned()
    };
    let xobject = |n: &[u8]| -> Option<&Dictionary> {
        let x = dict(doc, resources.get(b"XObject").ok())?;
        dict(doc, x.get(n).ok())
    };
    for op in ops {
        let name = op.operator.as_str();
        match name {
            "BMC" | "BDC" => {
                depth += 1;
                if skipping.is_none() && name == "BDC" {
                    let tag = op.operands.first().and_then(|o| o.as_name().ok());
                    if tag == Some(b"OC".as_slice()) {
                        let oc = match op.operands.get(1) {
                            Some(Object::Name(n)) => props(n),
                            Some(o) => Some(o.clone()),
                            None => None,
                        };
                        if oc.is_some_and(|oc| oc_hidden(doc, &oc, hidden)) {
                            counts.hidden_layer_sections += 1;
                            if remove_layers {
                                skipping = Some(depth);
                                changed = true;
                                continue;
                            }
                        }
                    }
                }
            }
            "EMC" => {
                let closing = depth;
                depth = depth.saturating_sub(1);
                if let Some(d) = skipping {
                    if closing == d {
                        skipping = None;
                    }
                    continue;
                }
            }
            _ => {}
        }
        if skipping.is_some() {
            continue;
        }
        match name {
            "q" => stack.push(t.clone()),
            "Q" => {
                if let Some(s) = stack.pop() {
                    t = s;
                }
            }
            "Tr" => {
                t.tr = op
                    .operands
                    .first()
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(0)
            }
            "Tf" => {
                if let (Some(Object::Name(n)), Some(s)) =
                    (op.operands.first(), op.operands.get(1).and_then(number))
                {
                    t.font = n.clone();
                    t.size = s;
                }
            }
            "Tc" => t.tc = op.operands.first().and_then(number).unwrap_or(0.0),
            "Tw" => t.tw = op.operands.first().and_then(number).unwrap_or(0.0),
            "Tz" => t.th = op.operands.first().and_then(number).unwrap_or(100.0) / 100.0,
            "Do" if remove_layers => {
                if let Some(Object::Name(n)) = op.operands.first() {
                    if let Some(x) = xobject(n) {
                        if x.get(b"OC")
                            .ok()
                            .is_some_and(|oc| oc_hidden(doc, oc, hidden))
                        {
                            counts.hidden_layer_sections += 1;
                            changed = true;
                            continue;
                        }
                    }
                }
            }
            "Tj" | "TJ" | "'" | "\"" if t.tr == 3 && shows_glyphs(&op) => {
                counts.hidden_text_ops += 1;
                if remove_text {
                    changed = true;
                    if name == "\"" {
                        if let (Some(aw), Some(ac)) = (op.operands.first(), op.operands.get(1)) {
                            out.push(Operation::new("Tw", vec![aw.clone()]));
                            out.push(Operation::new("Tc", vec![ac.clone()]));
                            t.tw = number(aw).unwrap_or(0.0);
                            t.tc = number(ac).unwrap_or(0.0);
                        }
                    }
                    if name == "'" || name == "\"" {
                        out.push(Operation::new("T*", vec![]));
                    }
                    // Keep the advance so visible text after it does not move.
                    let font = fonts.entry(t.font.clone()).or_insert_with(|| {
                        dict(doc, resources.get(b"Font").ok())
                            .and_then(|f| dict(doc, f.get(&t.font).ok()))
                            .map(|d| read_font(doc, d))
                            .unwrap_or_else(FontInfo::unknown)
                    });
                    if font.reliable && t.size.abs() > 1e-6 {
                        let mut tj = 0.0f32;
                        let operand = match name {
                            "\"" => op.operands.get(2),
                            _ => op.operands.first(),
                        };
                        let items: Vec<Object> = match operand {
                            Some(Object::Array(a)) => a.clone(),
                            Some(o) => vec![o.clone()],
                            None => Vec::new(),
                        };
                        for item in items {
                            match item {
                                Object::String(s, _) => {
                                    for g in font.glyphs(&s) {
                                        let adv = g.width * t.size
                                            + t.tc
                                            + if g.space { t.tw } else { 0.0 };
                                        tj -= adv / t.size * 1000.0;
                                    }
                                }
                                o => tj += number(&o).unwrap_or(0.0),
                            }
                        }
                        out.push(Operation::new(
                            "TJ",
                            vec![Object::Array(vec![Object::Real(tj)])],
                        ));
                    }
                    continue;
                }
            }
            _ => {}
        }
        out.push(op);
    }
    (out, changed)
}

/// Whether a show operation has any string to show (a displacement-only TJ
/// left by an earlier clean does not).
fn shows_glyphs(op: &Operation) -> bool {
    op.operands.iter().any(|o| match o {
        Object::String(s, _) => !s.is_empty(),
        Object::Array(a) => a
            .iter()
            .any(|x| matches!(x, Object::String(s, _) if !s.is_empty())),
        _ => false,
    })
}

fn encode(ops: Vec<Operation>) -> Option<Vec<u8>> {
    Content { operations: ops }.encode().ok()
}

/// Run the content pass over every page and form XObject.
fn content_pass(doc: &mut Document, remove_text: bool, remove_layers: bool) -> ContentCounts {
    let hidden = hidden_ocgs(doc);
    let mut counts = ContentCounts::default();
    let (pages, forms) = content_targets(doc);
    for page in pages {
        let Some(bytes) = page_bytes(doc, page) else {
            continue;
        };
        let Ok(content) = Content::decode(&bytes) else {
            continue;
        };
        let resources = page_resources(doc, page);
        let mut fonts = HashMap::new();
        let (ops, changed) = rewrite_ops(
            doc,
            content.operations,
            &resources,
            &hidden,
            (remove_text, remove_layers),
            &mut fonts,
            &mut counts,
        );
        if changed {
            if let Some(bytes) = encode(ops) {
                let mut s = Stream::new(Dictionary::new(), bytes);
                let _ = s.compress();
                let id = doc.add_object(s);
                if let Ok(d) = doc.get_dictionary_mut(page) {
                    d.set("Contents", Object::Reference(id));
                }
            }
        }
    }
    for form in forms {
        let Ok(Object::Stream(s)) = doc.get_object(form) else {
            continue;
        };
        let Ok(bytes) = s.decompressed_content() else {
            continue;
        };
        let Ok(content) = Content::decode(&bytes) else {
            continue;
        };
        let resources = dict(doc, s.dict.get(b"Resources").ok())
            .cloned()
            .unwrap_or_default();
        let mut fonts = HashMap::new();
        let (ops, changed) = rewrite_ops(
            doc,
            content.operations,
            &resources,
            &hidden,
            (remove_text, remove_layers),
            &mut fonts,
            &mut counts,
        );
        if changed {
            if let (Some(bytes), Ok(Object::Stream(s))) = (encode(ops), doc.get_object_mut(form)) {
                s.dict.remove(b"Filter");
                s.dict.remove(b"DecodeParms");
                s.set_plain_content(bytes);
                let _ = s.compress();
            }
        }
    }
    counts
}

/// Earlier revisions: `%%EOF` markers beyond the first.
pub fn revision_count(bytes: &[u8]) -> u32 {
    let marker = b"%%EOF";
    let n = bytes.windows(marker.len()).filter(|w| w == marker).count() as u32;
    n.saturating_sub(1)
}

/// What hidden information `doc` (read from `bytes`) carries.
pub fn scan(doc: &Document, bytes: &[u8]) -> HiddenInfoDto {
    let mut items = Vec::new();
    let mut push = |key: &str, count: u32, detail: String| {
        if count > 0 {
            items.push(HiddenItemDto {
                key: key.into(),
                count,
                detail,
            });
        }
    };

    // Metadata.
    let mut keys = Vec::new();
    if let Some(info) = dict(doc, doc.trailer.get(b"Info").ok()) {
        keys.extend(
            info.iter()
                .map(|(k, _)| String::from_utf8_lossy(k).to_string()),
        );
    }
    let mut xmp = 0;
    each_dict(doc, |d| {
        if d.has(b"Metadata") {
            xmp += 1;
        }
    });
    if xmp > 0 {
        keys.push(if xmp == 1 {
            "XMP".into()
        } else {
            format!("XMP ({xmp})")
        });
    }
    push("metadata", keys.len() as u32, keys.join(", "));

    // Attachments.
    let mut files = Vec::new();
    if let Some(tree) = names_entry(doc, b"EmbeddedFiles") {
        name_tree_names(doc, tree, &mut files, 0);
    }
    let annots = annotations(doc);
    let annot_dicts: Vec<&Dictionary> = annots
        .iter()
        .filter_map(|(_, e)| dict(doc, Some(e)))
        .collect();
    for a in &annot_dicts {
        if name_of(a, b"Subtype") == b"FileAttachment" {
            let name = dict(doc, a.get(b"FS").ok())
                .and_then(|fs| fs.get(b"UF").or_else(|_| fs.get(b"F")).ok())
                .and_then(|o| o.as_str().ok())
                .map(|s| String::from_utf8_lossy(s).to_string())
                .unwrap_or_else(|| "attached file".into());
            files.push(name);
        }
    }
    push("attachments", files.len() as u32, files.join(", "));

    // JavaScript.
    let mut scripts = 0u32;
    if let Some(tree) = names_entry(doc, b"JavaScript") {
        let mut names = Vec::new();
        name_tree_names(doc, tree, &mut names, 0);
        scripts += names.len().max(1) as u32;
    }
    let mut triggers = 0u32;
    each_dict(doc, |d| {
        if name_of(d, b"S") == b"JavaScript" || d.has(b"JS") {
            scripts += 1;
        }
        if d.has(b"AA") {
            triggers += 1;
        }
    });
    push(
        "javascript",
        scripts + triggers,
        format!("{scripts} script(s), {triggers} trigger(s)"),
    );

    // Hidden layers and hidden text.
    let hidden = hidden_ocgs(doc);
    let mut counts = ContentCounts::default();
    let (pages, forms) = content_targets(doc);
    for page in pages {
        if let Some(c) = page_bytes(doc, page).and_then(|b| Content::decode(&b).ok()) {
            rewrite_ops(
                doc,
                c.operations,
                &page_resources(doc, page),
                &hidden,
                (false, false),
                &mut HashMap::new(),
                &mut counts,
            );
        }
    }
    for form in forms {
        if let Ok(Object::Stream(s)) = doc.get_object(form) {
            if let Some(c) = s
                .decompressed_content()
                .ok()
                .and_then(|b| Content::decode(&b).ok())
            {
                let res = dict(doc, s.dict.get(b"Resources").ok())
                    .cloned()
                    .unwrap_or_default();
                rewrite_ops(
                    doc,
                    c.operations,
                    &res,
                    &hidden,
                    (false, false),
                    &mut HashMap::new(),
                    &mut counts,
                );
            }
        }
    }
    let hidden_layer_names: Vec<String> = hidden
        .iter()
        .filter_map(|id| doc.get_dictionary(*id).ok())
        .filter_map(|d| d.get(b"Name").ok().and_then(|o| o.as_str().ok()))
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();
    push(
        "hiddenLayers",
        hidden.len() as u32,
        hidden_layer_names.join(", "),
    );
    push(
        "hiddenText",
        counts.hidden_text_ops,
        "invisible text, such as an OCR layer over a scan".into(),
    );

    // Comments.
    let mut kinds: BTreeSet<String> = BTreeSet::new();
    let comments = annot_dicts
        .iter()
        .filter(|a| {
            let s = name_of(a, b"Subtype");
            let hit = COMMENT_SUBTYPES.contains(&s) || s == b"FileAttachment";
            if hit && s != b"Popup" {
                kinds.insert(String::from_utf8_lossy(s).to_string());
            }
            hit && s != b"Popup"
        })
        .count();
    push(
        "comments",
        comments as u32,
        kinds.into_iter().collect::<Vec<_>>().join(", "),
    );

    // Forms.
    let widgets = annot_dicts
        .iter()
        .filter(|a| name_of(a, b"Subtype") == b"Widget")
        .count();
    let xfa = doc
        .catalog()
        .ok()
        .and_then(|c| dict(doc, c.get(b"AcroForm").ok()))
        .is_some_and(|f| f.has(b"XFA"));
    push(
        "forms",
        widgets as u32 + u32::from(xfa),
        if xfa {
            "includes an XFA form".into()
        } else {
            String::new()
        },
    );

    // External links and actions.
    let mut hosts: BTreeSet<String> = BTreeSet::new();
    let mut links = 0u32;
    each_dict(doc, |d| {
        let s = name_of(d, b"S");
        if EXTERNAL_ACTIONS.contains(&s) {
            links += 1;
            if let Ok(uri) = d.get(b"URI").and_then(Object::as_str) {
                hosts.insert(host_of(uri));
            }
        }
    });
    push(
        "links",
        links,
        hosts.into_iter().take(8).collect::<Vec<_>>().join(", "),
    );

    // Bookmarks.
    let outline = outline_count(doc);
    push("bookmarks", outline, String::new());

    // Thumbnails and private application data.
    let thumbs = doc
        .get_pages()
        .values()
        .filter(|p| doc.get_dictionary(**p).is_ok_and(|d| d.has(b"Thumb")))
        .count();
    push("thumbnails", thumbs as u32, String::new());
    let mut piece = 0u32;
    each_dict(doc, |d| {
        if d.has(b"PieceInfo") {
            piece += 1;
        }
    });
    push("privateData", piece, String::new());

    push("revisions", revision_count(bytes), String::new());

    HiddenInfoDto {
        items,
        signed: crate::engine::redact::has_signatures(doc),
    }
}

/// Remove the chosen kinds of hidden information. Returns what was done.
pub fn clean(doc: &mut Document, options: &SanitizeOptions) -> Vec<String> {
    let mut done = Vec::new();
    if options.hidden_layers || options.hidden_text {
        let counts = content_pass(doc, options.hidden_text, options.hidden_layers);
        if options.hidden_text && counts.hidden_text_ops > 0 {
            done.push(format!(
                "{} invisible text operations",
                counts.hidden_text_ops
            ));
        }
        if options.hidden_layers {
            let hidden = hidden_ocgs(doc);
            remove_ocgs(doc, &hidden);
            done.push(format!(
                "{} hidden layer sections",
                counts.hidden_layer_sections
            ));
        }
    }
    if options.forms {
        let n = flatten_forms(doc);
        done.push(format!("{n} form widgets flattened"));
    }
    if options.comments || options.attachments || options.links || options.hidden_layers {
        let hidden = hidden_ocgs(doc);
        let removed = filter_annotations(doc, |doc, a| {
            let s = name_of(a, b"Subtype");
            (options.comments && COMMENT_SUBTYPES.contains(&s))
                || (options.attachments && s == b"FileAttachment")
                || (options.links
                    && s == b"Link"
                    && a.get(b"A")
                        .ok()
                        .is_some_and(|o| is_action(doc, o, EXTERNAL_ACTIONS)))
                || (options.hidden_layers
                    && a.get(b"OC")
                        .ok()
                        .is_some_and(|oc| oc_hidden(doc, oc, &hidden)))
        });
        done.push(format!("{removed} annotations"));
    }
    if options.metadata {
        doc.trailer.remove(b"Info");
        each_dict_mut(doc, |d| {
            d.remove(b"Metadata");
        });
        done.push("metadata".into());
    }
    if options.attachments {
        if let Ok(c) = doc.catalog_mut() {
            c.remove(b"AF");
            c.remove(b"Collection");
        }
        remove_names_entry(doc, b"EmbeddedFiles");
        each_dict_mut(doc, |d| {
            d.remove(b"AF");
        });
        done.push("attachments".into());
    }
    if options.javascript {
        remove_names_entry(doc, b"JavaScript");
        let js: HashSet<ObjectId> = doc
            .objects
            .iter()
            .filter_map(|(id, o)| match o {
                Object::Dictionary(d) if name_of(d, b"S") == b"JavaScript" => Some(*id),
                _ => None,
            })
            .collect();
        each_dict_mut(doc, |d| {
            d.remove(b"AA");
            for key in [b"A".as_slice(), b"OpenAction", b"Next"] {
                let drop = match d.get(key) {
                    Ok(Object::Reference(id)) => js.contains(id),
                    Ok(Object::Dictionary(a)) => name_of(a, b"S") == b"JavaScript",
                    _ => false,
                };
                if drop {
                    d.remove(key);
                }
            }
        });
        done.push("JavaScript".into());
    }
    if options.links {
        let external: HashSet<ObjectId> = doc
            .objects
            .iter()
            .filter_map(|(id, o)| match o {
                Object::Dictionary(d) if EXTERNAL_ACTIONS.contains(&name_of(d, b"S")) => Some(*id),
                _ => None,
            })
            .collect();
        each_dict_mut(doc, |d| {
            for key in [b"A".as_slice(), b"OpenAction", b"Next"] {
                let drop = match d.get(key) {
                    Ok(Object::Reference(id)) => external.contains(id),
                    Ok(Object::Dictionary(a)) => EXTERNAL_ACTIONS.contains(&name_of(a, b"S")),
                    _ => false,
                };
                if drop {
                    d.remove(key);
                }
            }
        });
        done.push("external links".into());
    }
    if options.bookmarks {
        if let Ok(c) = doc.catalog_mut() {
            c.remove(b"Outlines");
            if c.get(b"PageMode").and_then(Object::as_name).ok() == Some(b"UseOutlines".as_slice())
            {
                c.remove(b"PageMode");
            }
        }
        done.push("bookmarks".into());
    }
    if options.thumbnails {
        for page in doc.get_pages().values().copied().collect::<Vec<_>>() {
            if let Ok(d) = doc.get_dictionary_mut(page) {
                d.remove(b"Thumb");
            }
        }
        done.push("thumbnails".into());
    }
    if options.private_data {
        each_dict_mut(doc, |d| {
            d.remove(b"PieceInfo");
        });
        done.push("private application data".into());
    }
    done
}

/// Outline items, walked through /First and /Next.
fn outline_count(doc: &Document) -> u32 {
    let Some(root) = doc
        .catalog()
        .ok()
        .and_then(|c| dict(doc, c.get(b"Outlines").ok()))
    else {
        return 0;
    };
    let mut count = 0u32;
    let mut stack: Vec<ObjectId> = root
        .get(b"First")
        .ok()
        .and_then(|o| o.as_reference().ok())
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || count > 100_000 {
            continue;
        }
        count += 1;
        if let Ok(d) = doc.get_dictionary(id) {
            for key in [b"First".as_slice(), b"Next"] {
                if let Ok(next) = d.get(key).and_then(Object::as_reference) {
                    stack.push(next);
                }
            }
        }
    }
    count
}

fn remove_names_entry(doc: &mut Document, key: &[u8]) {
    let names = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .cloned();
    match names {
        Some(Object::Reference(id)) => {
            if let Ok(d) = doc.get_dictionary_mut(id) {
                d.remove(key);
            }
        }
        Some(Object::Dictionary(_)) => {
            if let Ok(Object::Dictionary(d)) = doc.catalog_mut().and_then(|c| c.get_mut(b"Names")) {
                d.remove(key);
            }
        }
        _ => {}
    }
}

/// Drop hidden OCGs from the optional-content configuration.
fn remove_ocgs(doc: &mut Document, hidden: &HashSet<ObjectId>) {
    if hidden.is_empty() {
        return;
    }
    fn filter(o: &mut Object, hidden: &HashSet<ObjectId>) {
        if let Object::Array(a) = o {
            a.retain(|x| !x.as_reference().is_ok_and(|id| hidden.contains(&id)));
            for x in a.iter_mut() {
                filter(x, hidden);
            }
        } else if let Object::Dictionary(d) = o {
            for (_, v) in d.iter_mut() {
                filter(v, hidden);
            }
        }
    }
    let Some(props) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"OCProperties").ok())
        .cloned()
    else {
        return;
    };
    let mut target = match &props {
        Object::Reference(id) => doc.get_object(*id).ok().cloned(),
        o => Some(o.clone()),
    };
    let Some(target_obj) = target.as_mut() else {
        return;
    };
    // /D and /Configs may be references too; resolve them into the copy.
    if let Object::Dictionary(d) = target_obj {
        for key in [b"D".as_slice(), b"Configs"] {
            if let Ok(Object::Reference(id)) = d.get(key) {
                if let Ok(o) = doc.get_object(*id) {
                    d.set(key.to_vec(), o.clone());
                }
            }
        }
    }
    filter(target_obj, hidden);
    match props {
        Object::Reference(id) => {
            if let Ok(o) = doc.get_object_mut(id) {
                *o = target_obj.clone();
            }
        }
        _ => {
            if let Ok(c) = doc.catalog_mut() {
                c.set("OCProperties", target_obj.clone());
            }
        }
    }
}

/// Remove annotations matching `drop` from every page, with the popups of
/// removed ones. Returns how many were removed.
fn filter_annotations(doc: &mut Document, drop: impl Fn(&Document, &Dictionary) -> bool) -> u32 {
    let mut count = 0;
    for page in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let entries: Vec<Object> = match doc
            .get_dictionary(page)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .map(|o| deref(doc, o))
        {
            Some(Object::Array(a)) => a.clone(),
            _ => continue,
        };
        let mut removed: HashSet<ObjectId> = HashSet::new();
        let mut keep: Vec<bool> = entries
            .iter()
            .map(|e| {
                let hit = dict(doc, Some(e)).is_some_and(|a| drop(doc, a));
                if hit {
                    count += 1;
                    if let Ok(id) = e.as_reference() {
                        removed.insert(id);
                    }
                }
                !hit
            })
            .collect();
        for (k, e) in entries.iter().enumerate() {
            if let Some(a) = dict(doc, Some(e)) {
                if name_of(a, b"Subtype") == b"Popup"
                    && a.get(b"Parent")
                        .ok()
                        .and_then(|p| p.as_reference().ok())
                        .is_some_and(|p| removed.contains(&p))
                {
                    keep[k] = false;
                }
            }
        }
        if keep.iter().all(|k| *k) {
            continue;
        }
        let kept: Vec<Object> = entries
            .into_iter()
            .zip(keep)
            .filter_map(|(e, k)| k.then_some(e))
            .collect();
        if let Ok(d) = doc.get_dictionary_mut(page) {
            if kept.is_empty() {
                d.remove(b"Annots");
            } else {
                d.set("Annots", Object::Array(kept));
            }
        }
    }
    count
}

/// Draw every visible widget's appearance into its page, then remove the
/// widgets and the form. Returns the number of widgets flattened.
fn flatten_forms(doc: &mut Document) -> u32 {
    let mut count = 0;
    for page in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let entries: Vec<Object> = match doc
            .get_dictionary(page)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .map(|o| deref(doc, o))
        {
            Some(Object::Array(a)) => a.clone(),
            _ => continue,
        };
        let mut ops = String::new();
        let mut resources = page_resources(doc, page);
        let mut xobjects = dict(doc, resources.get(b"XObject").ok())
            .cloned()
            .unwrap_or_default();
        let mut kept = Vec::new();
        let mut seq = 0;
        for e in entries {
            let Some(a) = dict(doc, Some(&e)) else {
                kept.push(e);
                continue;
            };
            if name_of(a, b"Subtype") != b"Widget" {
                kept.push(e);
                continue;
            }
            count += 1;
            let flags = a.get(b"F").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
            if flags & 0b10 != 0 {
                continue; // hidden
            }
            let Some(rect) = a
                .get(b"Rect")
                .ok()
                .map(|o| deref(doc, o))
                .and_then(|o| o.as_array().ok())
                .map(|v| v.iter().filter_map(number).collect::<Vec<_>>())
                .filter(|v| v.len() == 4)
                .map(|v| Rect::new(v[0], v[1], v[2], v[3]))
            else {
                continue;
            };
            // The normal appearance, picking the /AS state of a state dict.
            let Some(n) = dict(doc, a.get(b"AP").ok()).and_then(|ap| ap.get(b"N").ok()) else {
                continue;
            };
            let ap_ref = match deref(doc, n) {
                Object::Stream(_) => n.as_reference().ok(),
                Object::Dictionary(states) => a
                    .get(b"AS")
                    .ok()
                    .and_then(|s| s.as_name().ok())
                    .and_then(|s| states.get(s).ok())
                    .and_then(|o| o.as_reference().ok()),
                _ => None,
            };
            let Some(ap_ref) = ap_ref else { continue };
            let Ok(Object::Stream(ap)) = doc.get_object(ap_ref) else {
                continue;
            };
            let bbox = ap
                .dict
                .get(b"BBox")
                .ok()
                .and_then(|o| o.as_array().ok())
                .map(|v| v.iter().filter_map(number).collect::<Vec<_>>())
                .filter(|v| v.len() == 4);
            let Some(bbox) = bbox else { continue };
            let matrix = ap
                .dict
                .get(b"Matrix")
                .ok()
                .and_then(|o| o.as_array().ok())
                .and_then(|a| Matrix::from_operands(a))
                .unwrap_or(Matrix::IDENTITY);
            // ISO 32000 12.5.5: map the transformed BBox onto the Rect.
            let t = matrix.map_box(bbox[0], bbox[1], bbox[2], bbox[3]);
            if t.width() <= 0.0 || t.height() <= 0.0 {
                continue;
            }
            let sx = rect.width() / t.width();
            let sy = rect.height() / t.height();
            let a_m = Matrix([sx, 0.0, 0.0, sy, rect.x0 - t.x0 * sx, rect.y0 - t.y0 * sy]);
            seq += 1;
            let name = format!("SpFlat{seq}");
            xobjects.set(name.clone().into_bytes(), Object::Reference(ap_ref));
            let [ma, mb, mc, md, me, mf] = a_m.0;
            ops.push_str(&format!(
                "q {ma} {mb} {mc} {md} {me} {mf} cm /{name} Do Q\n"
            ));
        }
        if !ops.is_empty() {
            resources.set("XObject", Object::Dictionary(xobjects));
            let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
            let mut close = Stream::new(Dictionary::new(), format!("\nQ\n{ops}").into_bytes());
            let _ = close.compress();
            let close = doc.add_object(close);
            let existing: Vec<Object> = match doc
                .get_dictionary(page)
                .ok()
                .and_then(|d| d.get(b"Contents").ok())
            {
                Some(Object::Reference(id)) => match doc.get_object(*id) {
                    Ok(Object::Array(a)) => a.clone(),
                    _ => vec![Object::Reference(*id)],
                },
                Some(Object::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let mut contents = vec![Object::Reference(open)];
            contents.extend(existing);
            contents.push(Object::Reference(close));
            if let Ok(d) = doc.get_dictionary_mut(page) {
                d.set("Contents", Object::Array(contents));
                d.set("Resources", Object::Dictionary(resources));
            }
        }
        if let Ok(d) = doc.get_dictionary_mut(page) {
            if kept.is_empty() {
                d.remove(b"Annots");
            } else {
                d.set("Annots", Object::Array(kept));
            }
        }
    }
    if let Ok(c) = doc.catalog_mut() {
        c.remove(b"AcroForm");
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// One page carrying a bit of every kind of hidden information.
    fn fixture() -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "FirstChar" => 32, "LastChar" => 126,
            "Widths" => Object::Array(vec![Object::Integer(600); 95]),
        });
        let ocg = doc.add_object(
            dictionary! { "Type" => "OCG", "Name" => Object::string_literal("Draft notes") },
        );
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 10 Tf 72 700 Td (Visible words) Tj ET \
              /OC /L1 BDC BT /F1 10 Tf 72 650 Td (HIDDENLAYERTEXT) Tj ET EMC \
              BT /F1 10 Tf 3 Tr 72 600 Td (INVISIBLETEXT) Tj 0 Tr ( after) Tj ET"
                .to_vec(),
        ));
        let comment = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Text",
            "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()],
            "Contents" => Object::string_literal("PRIVATECOMMENT"),
        });
        let link = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Link",
            "Rect" => vec![40.into(), 40.into(), 90.into(), 60.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://tracker.example/x") },
        });
        let thumb = doc.add_object(Stream::new(
            dictionary! { "Width" => 1, "Height" => 1 },
            vec![0, 0, 0],
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => Object::Reference(pages),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => Object::Reference(content),
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => Object::Reference(font) },
                "Properties" => dictionary! { "L1" => Object::Reference(ocg) },
            },
            "Annots" => vec![Object::Reference(comment), Object::Reference(link)],
            "Thumb" => Object::Reference(thumb),
            "PieceInfo" => dictionary! { "App" => dictionary! { "Private" => Object::string_literal("APPSECRET") } },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }),
        );
        let file = doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile" },
            b"ATTACHEDBYTES".to_vec(),
        ));
        let spec = doc.add_object(dictionary! {
            "Type" => "Filespec", "F" => Object::string_literal("notes.txt"),
            "EF" => dictionary! { "F" => Object::Reference(file) },
        });
        let js = doc.add_object(dictionary! { "S" => "JavaScript", "JS" => Object::string_literal("app.alert('JSPAYLOAD')") });
        let xmp = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>XMPAUTHOR</x:xmpmeta>".to_vec(),
        ));
        let outline_item = doc.new_object_id();
        let outlines = doc.add_object(dictionary! {
            "Type" => "Outlines", "First" => Object::Reference(outline_item),
            "Last" => Object::Reference(outline_item), "Count" => 1,
        });
        doc.objects.insert(
            outline_item,
            Object::Dictionary(dictionary! {
                "Title" => Object::string_literal("BOOKMARKTITLE"), "Parent" => Object::Reference(outlines),
            }),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => Object::Reference(pages),
            "Metadata" => Object::Reference(xmp),
            "OpenAction" => Object::Reference(js),
            "Outlines" => Object::Reference(outlines),
            "Names" => dictionary! {
                "EmbeddedFiles" => dictionary! {
                    "Names" => vec![Object::string_literal("notes.txt"), Object::Reference(spec)],
                },
            },
            "OCProperties" => dictionary! {
                "OCGs" => vec![Object::Reference(ocg)],
                "D" => dictionary! {
                    "OFF" => vec![Object::Reference(ocg)], "Order" => vec![Object::Reference(ocg)],
                },
            },
        });
        let info = doc.add_object(dictionary! { "Author" => Object::string_literal("INFOAUTHOR") });
        doc.trailer.set("Root", Object::Reference(catalog));
        doc.trailer.set("Info", Object::Reference(info));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn all_bytes(bytes: &[u8]) -> String {
        let mut doc = Document::load_mem(bytes).unwrap();
        doc.decompress();
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        String::from_utf8_lossy(&out).to_string()
    }

    #[test]
    fn scan_finds_every_kind() {
        let bytes = fixture();
        let doc = Document::load_mem(&bytes).unwrap();
        let info = scan(&doc, &bytes);
        let keys: Vec<&str> = info.items.iter().map(|i| i.key.as_str()).collect();
        for key in [
            "metadata",
            "attachments",
            "javascript",
            "hiddenLayers",
            "hiddenText",
            "comments",
            "links",
            "bookmarks",
            "thumbnails",
            "privateData",
        ] {
            assert!(keys.contains(&key), "{key} missing from {keys:?}");
        }
        assert!(!info.signed);
        let attachments = info.items.iter().find(|i| i.key == "attachments").unwrap();
        assert!(attachments.detail.contains("notes.txt"));
    }

    #[test]
    fn clean_removes_every_kind_and_keeps_the_page() {
        let bytes = fixture();
        let mut doc = Document::load_mem(&bytes).unwrap();
        let all = SanitizeOptions {
            metadata: true,
            attachments: true,
            javascript: true,
            hidden_layers: true,
            hidden_text: true,
            comments: true,
            forms: true,
            links: true,
            bookmarks: true,
            thumbnails: true,
            private_data: true,
        };
        clean(&mut doc, &all);
        let out = crate::engine::redact::clean_rewrite(&mut doc).unwrap();
        let raw = all_bytes(&out);
        for gone in [
            "INFOAUTHOR",
            "XMPAUTHOR",
            "ATTACHEDBYTES",
            "JSPAYLOAD",
            "HIDDENLAYERTEXT",
            "INVISIBLETEXT",
            "PRIVATECOMMENT",
            "tracker.example",
            "BOOKMARKTITLE",
            "APPSECRET",
        ] {
            assert!(!raw.contains(gone), "{gone} survived");
        }
        assert!(raw.contains("Visible words") && raw.contains("after"));
        let doc = Document::load_mem(&out).unwrap();
        let rescanned = scan(&doc, &out);
        assert!(rescanned.items.is_empty(), "{:?}", rescanned.items);
    }

    #[test]
    fn revision_count_counts_updates() {
        assert_eq!(revision_count(b"%PDF-1.7 ... %%EOF\n ... %%EOF\n"), 1);
    }
}

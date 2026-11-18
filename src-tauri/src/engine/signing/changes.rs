//! What changed after a signature was made.
//!
//! A signature covers its own revision of the file; later incremental updates
//! are outside it. Whether they matter depends on what they did: filling in a
//! form, adding a comment or another signature leaves the signed document as
//! it was, while changing a page's content or the page list does not. The
//! author of a certification signature narrows that further (DocMDP level),
//! and a signature can lock fields (FieldMDP).
//!
//! The comparison is object by object, between the signed revision and the
//! final file, both loaded by lopdf. Each object that differs is classified by
//! the role it played in the signed revision: part of a page's content, a
//! widget or its field, a comment, the catalog, the document info. A new
//! object only matters through what refers to it, and whatever refers to it
//! had to change too, so only changed objects that already existed (plus the
//! page list and each page's annotations) are classified.

use super::parse::{page_annots, page_ids, resolve, FieldLock};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// What sort of change a later update made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeKind {
    /// Form field values (and their appearances).
    FormFill,
    /// Comments and other annotations added, edited or removed.
    Comment,
    /// Further signatures or document timestamps.
    Signature,
    /// Document info or XMP metadata.
    Metadata,
    /// Page content: what a page draws, or its size or rotation.
    PageContent,
    /// Pages added, removed or reordered.
    Pages,
    /// New form fields.
    FieldAdded,
    /// Anything else.
    Other,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::FormFill => "formFill",
            ChangeKind::Comment => "comment",
            ChangeKind::Signature => "signature",
            ChangeKind::Metadata => "metadata",
            ChangeKind::PageContent => "pageContent",
            ChangeKind::Pages => "pages",
            ChangeKind::FieldAdded => "fieldAdded",
            ChangeKind::Other => "other",
        }
    }
}

/// One change, grouped: every field filled in, every comment on a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// Whether the signer's terms (or the defaults) allow it.
    pub allowed: bool,
    /// Zero-based page index in the final revision, when it is about a page.
    pub page: Option<u32>,
    /// One line, plain words.
    pub detail: String,
}

/// What changes a signature permits after it.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// The document's certification level, if a certification signature is
    /// at or before this one: 1 no changes, 2 form fill and signing, 3 also
    /// comments.
    pub doc_mdp: Option<u8>,
    /// Fields locked by this signature or an earlier one.
    pub locks: Vec<FieldLock>,
}

impl Policy {
    fn allows(&self, kind: ChangeKind) -> bool {
        match (kind, self.doc_mdp) {
            (ChangeKind::Signature, Some(1)) => false,
            (ChangeKind::Signature, _) => true,
            (ChangeKind::FormFill | ChangeKind::Metadata, Some(1)) => false,
            (ChangeKind::FormFill | ChangeKind::Metadata, _) => true,
            (ChangeKind::Comment, None | Some(3)) => true,
            (ChangeKind::Comment, _) => false,
            _ => false,
        }
    }

    fn locked(&self, field: &str) -> bool {
        self.locks.iter().any(|lock| lock.covers(field))
    }
}

/// The role an object played in the signed revision.
#[derive(Clone, Debug)]
enum Role {
    /// Content stream, resources, fonts, images of a page.
    PageContent(u32),
    Page(u32),
    PagesNode,
    /// A widget annotation, or something only its appearance uses.
    Widget(String),
    /// A field dictionary (not merged with a widget).
    Field(String),
    /// A markup or link annotation, or its popup or appearance.
    Annotation(u32),
    Catalog,
    AcroForm,
    /// Shared form resources (`/DR`) and other form housekeeping.
    FormResources,
    Info,
    Metadata,
    /// Validation data (`/DSS`): certificates, OCSP, CRLs added later.
    Dss,
}

/// The changes between `old` (a signed revision) and `new` (the final file).
pub fn changes_between(old: &Document, new: &Document, policy: &Policy) -> Vec<Change> {
    let roles = roles_of(old);
    let new_pages = page_ids(new);
    let new_page_index: HashMap<ObjectId, u32> = new_pages
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i as u32))
        .collect();
    let old_pages = page_ids(old);
    let old_to_new = |old_page: u32| -> Option<u32> {
        old_pages
            .get(old_page as usize)
            .and_then(|id| new_page_index.get(id).copied())
    };

    let mut found = Found {
        signed_before: all_fields(old)
            .into_iter()
            .filter(|(id, _, ft)| {
                ft.as_deref() == Some(b"Sig".as_slice())
                    && old
                        .get_object(*id)
                        .ok()
                        .and_then(dict_of)
                        .is_some_and(|d| d.has(b"V"))
            })
            .map(|(_, name, _)| name)
            .collect(),
        ..Found::default()
    };

    // The page list itself.
    if old_pages != new_pages {
        let old_set: HashSet<_> = old_pages.iter().collect();
        let new_set: HashSet<_> = new_pages.iter().collect();
        let added = new_pages.iter().filter(|p| !old_set.contains(p)).count();
        let removed = old_pages.iter().filter(|p| !new_set.contains(p)).count();
        let detail = match (added, removed) {
            (0, 0) => "Pages were reordered".to_string(),
            (a, 0) => format!("{} added", plural(a, "page", "pages")),
            (0, r) => format!("{} removed", plural(r, "page", "pages")),
            (a, r) => format!(
                "{} added and {} removed",
                plural(a, "page", "pages"),
                plural(r, "page", "pages")
            ),
        };
        found.push(ChangeKind::Pages, None, detail);
    }

    // The catalog may have been replaced outright.
    let old_root = root_id(old);
    let new_root = root_id(new);
    if old_root != new_root {
        found.push(
            ChangeKind::Other,
            None,
            "The document catalog was replaced".into(),
        );
    }

    let ids: BTreeSet<ObjectId> = old
        .objects
        .keys()
        .chain(new.objects.keys())
        .copied()
        .collect();
    for id in ids {
        let (Some(before), Some(after)) = (old.objects.get(&id), new.objects.get(&id)) else {
            // Added objects only matter through what refers to them; removed
            // ones through what stopped referring to them.
            continue;
        };
        if same(before, after) || is_bookkeeping(before) {
            continue;
        }
        match roles.get(&id) {
            Some(Role::PageContent(page)) => found.push(
                ChangeKind::PageContent,
                old_to_new(*page),
                "Page content was changed".into(),
            ),
            Some(Role::Page(page)) => {
                page_changed(old, new, id, old_to_new(*page), before, after, &mut found)
            }
            Some(Role::PagesNode) => {
                let strip = |o: &Object| strip_keys(o, &[b"Kids", b"Count"]);
                if strip(before) != strip(after) {
                    found.push(
                        ChangeKind::PageContent,
                        None,
                        "Page settings shared by every page were changed".into(),
                    );
                }
            }
            // A widget merged with its field is the field: a signature field
            // gaining its value is a signature, not form fill.
            Some(Role::Widget(name)) if dict_of(after).is_some_and(|d| d.has(b"T")) => {
                field_changed(old, new, name, before, after, &mut found);
            }
            Some(Role::Widget(name)) => found.field(name),
            Some(Role::Field(name)) => {
                field_changed(old, new, name, before, after, &mut found);
            }
            Some(Role::Annotation(page)) => found.push(
                ChangeKind::Comment,
                old_to_new(*page),
                "Comments were edited".into(),
            ),
            Some(Role::Catalog) => catalog_changed(old, new, before, after, &mut found),
            Some(Role::AcroForm) => acroform_changed(old, new, before, after, &mut found),
            Some(Role::FormResources | Role::Dss) => {}
            Some(Role::Info | Role::Metadata) => found.push(
                ChangeKind::Metadata,
                None,
                "Document information was changed".into(),
            ),
            None => found.push(
                ChangeKind::Other,
                None,
                format!("Object {} {} was changed", id.0, id.1),
            ),
        }
    }
    // The catalog's /Root moved: compare what the two catalogs say.
    if old_root != new_root {
        if let (Some(before), Some(after)) = (
            old_root.and_then(|id| old.objects.get(&id)),
            new_root.and_then(|id| new.objects.get(&id)),
        ) {
            catalog_changed(old, new, before, after, &mut found);
        }
    }
    found.finish(policy)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Changes collected while walking, grouped at the end.
#[derive(Default)]
struct Found {
    /// (kind, page) -> details, in first-seen order.
    groups: BTreeMap<(ChangeKind, Option<u32>), Vec<String>>,
    /// Fields whose value changed.
    filled: BTreeSet<String>,
    /// Signature fields that gained a value.
    signed: BTreeSet<String>,
    /// Signature fields that already held a signature in the signed revision.
    signed_before: BTreeSet<String>,
}

impl Found {
    fn push(&mut self, kind: ChangeKind, page: Option<u32>, detail: String) {
        let list = self.groups.entry((kind, page)).or_default();
        if !list.contains(&detail) {
            list.push(detail);
        }
    }

    fn field(&mut self, name: &str) {
        self.filled.insert(name.to_string());
    }

    fn finish(mut self, policy: &Policy) -> Vec<Change> {
        let mut out = Vec::new();
        // Signature fields that were signed are signatures, not form fill.
        for name in &self.signed {
            self.filled.remove(name);
        }
        // A signature already made cannot be redrawn: its appearance is what
        // the signer saw and signed.
        let redrawn: Vec<String> = self
            .filled
            .iter()
            .filter(|name| self.signed_before.contains(*name))
            .cloned()
            .collect();
        for name in redrawn {
            self.filled.remove(&name);
            self.groups
                .entry((ChangeKind::Other, None))
                .or_default()
                .push(format!("The appearance of signature {name} was changed"));
        }
        if !self.signed.is_empty() {
            let names: Vec<&String> = self.signed.iter().collect();
            let detail = if names.len() == 1 && !names[0].is_empty() {
                format!("Signed again ({})", names[0])
            } else {
                format!("{} added", plural(names.len(), "signature", "signatures"))
            };
            self.groups
                .entry((ChangeKind::Signature, None))
                .or_default()
                .push(detail);
        }
        let (locked, open): (Vec<String>, Vec<String>) = self
            .filled
            .into_iter()
            .partition(|name| policy.locked(name));
        if !open.is_empty() {
            out.push(Change {
                kind: ChangeKind::FormFill,
                allowed: policy.allows(ChangeKind::FormFill),
                page: None,
                detail: fields_detail("Filled in", &open),
            });
        }
        if !locked.is_empty() {
            out.push(Change {
                kind: ChangeKind::FormFill,
                allowed: false,
                page: None,
                detail: fields_detail("Changed locked", &locked),
            });
        }
        for ((kind, page), details) in self.groups {
            out.push(Change {
                kind,
                allowed: policy.allows(kind),
                page,
                detail: details.join("; "),
            });
        }
        out.sort_by_key(|c| (c.allowed, c.kind, c.page));
        out
    }
}

fn fields_detail(verb: &str, names: &[String]) -> String {
    const SHOWN: usize = 3;
    let mut shown: Vec<&str> = names.iter().take(SHOWN).map(String::as_str).collect();
    if names.len() > SHOWN {
        shown.push("...");
    }
    format!(
        "{verb} {} ({})",
        plural(names.len(), "form field", "form fields"),
        shown.join(", ")
    )
}

/// Whether two versions of an object say the same thing. A stream counts
/// as unchanged when it decodes to the same bytes: savers (PDFium among them)
/// re-compress streams they rewrite without changing what they hold.
fn same(before: &Object, after: &Object) -> bool {
    if before == after {
        return true;
    }
    let (Object::Stream(b), Object::Stream(a)) = (before, after) else {
        return false;
    };
    let coding: &[&[u8]] = &[b"Length", b"Filter", b"DecodeParms"];
    let strip = |d: &Dictionary| {
        let mut d = d.clone();
        for key in coding {
            d.remove(key);
        }
        d
    };
    if strip(&b.dict) != strip(&a.dict) {
        return false;
    }
    let decoded = |s: &lopdf::Stream| -> Option<Vec<u8>> {
        let bytes = if s.dict.has(b"Filter") {
            s.decompressed_content().ok()?
        } else {
            s.content.clone()
        };
        let end = bytes
            .iter()
            .rposition(|ch| !ch.is_ascii_whitespace())
            .map_or(0, |i| i + 1);
        Some(bytes[..end].to_vec())
    };
    matches!((decoded(b), decoded(a)), (Some(x), Some(y)) if x == y)
}

/// Cross-reference and object streams change with every update and say
/// nothing about the document.
fn is_bookkeeping(object: &Object) -> bool {
    match object {
        Object::Stream(stream) => matches!(
            stream.dict.get(b"Type"),
            Ok(Object::Name(n)) if n == b"XRef" || n == b"ObjStm"
        ),
        _ => false,
    }
}

fn root_id(doc: &Document) -> Option<ObjectId> {
    match doc.trailer.get(b"Root") {
        Ok(Object::Reference(id)) => Some(*id),
        _ => None,
    }
}

fn strip_keys(object: &Object, keys: &[&[u8]]) -> Option<Dictionary> {
    let mut dict = match object {
        Object::Dictionary(d) => d.clone(),
        Object::Stream(s) => s.dict.clone(),
        _ => return None,
    };
    for key in keys {
        dict.remove(key);
    }
    Some(dict)
}

fn dict_of(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn subtype(dict: &Dictionary) -> Option<&[u8]> {
    match dict.get(b"Subtype") {
        Ok(Object::Name(n)) => Some(n.as_slice()),
        _ => None,
    }
}

fn page_changed(
    old: &Document,
    new: &Document,
    id: ObjectId,
    page: Option<u32>,
    before: &Object,
    after: &Object,
    found: &mut Found,
) {
    let keep = |o: &Object| strip_keys(o, &[b"Annots"]);
    if keep(before) != keep(after) {
        found.push(
            ChangeKind::PageContent,
            page,
            "Page content was changed".into(),
        );
    }
    let before_annots: HashSet<ObjectId> = page_annots(old, id).into_iter().collect();
    let after_annots: Vec<ObjectId> = page_annots(new, id);
    let after_set: HashSet<ObjectId> = after_annots.iter().copied().collect();
    let signature_widgets = signature_widgets(new);
    for annot in after_annots.iter().filter(|a| !before_annots.contains(a)) {
        let Some(dict) = new.get_object(*annot).ok().and_then(dict_of) else {
            continue;
        };
        match subtype(dict) {
            Some(b"Widget") => {
                if let Some(name) = signature_widgets.get(annot) {
                    if is_signed_field(new, *annot) {
                        found.signed.insert(name.clone());
                    } else {
                        found.push(
                            ChangeKind::FieldAdded,
                            page,
                            "A signature field was added".into(),
                        );
                    }
                } else {
                    found.push(
                        ChangeKind::FieldAdded,
                        page,
                        "A form field was added".into(),
                    );
                }
            }
            _ => found.push(ChangeKind::Comment, page, "Comments were added".into()),
        }
    }
    for annot in before_annots.iter().filter(|a| !after_set.contains(a)) {
        let Some(dict) = old.get_object(*annot).ok().and_then(dict_of) else {
            continue;
        };
        if subtype(dict) == Some(b"Widget") {
            found.push(ChangeKind::Other, page, "A form field was removed".into());
        } else {
            found.push(ChangeKind::Comment, page, "Comments were removed".into());
        }
    }
}

fn field_changed(
    old: &Document,
    new: &Document,
    name: &str,
    before: &Object,
    after: &Object,
    found: &mut Found,
) {
    let (Some(b), Some(a)) = (dict_of(before), dict_of(after)) else {
        found.push(
            ChangeKind::Other,
            None,
            format!("Form field {name} was replaced"),
        );
        return;
    };
    let is_sig = matches!(a.get(b"FT"), Ok(Object::Name(n)) if n == b"Sig");
    if is_sig && b.get(b"V").ok() != a.get(b"V").ok() {
        if b.get(b"V").is_err() {
            found.signed.insert(name.to_string());
        } else {
            found.push(
                ChangeKind::Other,
                None,
                format!("The signature in {name} was replaced"),
            );
        }
    } else if is_sig && b.has(b"V") {
        // Same signature, something else about its field or widget changed:
        // finish() reports a signed field's redraw as such.
        found.field(name);
    }
    // New kids that are fields are new fields.
    let kids = |doc: &Document, d: &Dictionary| -> HashSet<ObjectId> {
        d.get(b"Kids")
            .ok()
            .and_then(|k| resolve(doc, k))
            .and_then(|k| k.as_array().ok())
            .map(|k| {
                k.iter()
                    .filter_map(|o| match o {
                        Object::Reference(id) => Some(*id),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let before_kids = kids(old, b);
    for kid in kids(new, a).difference(&before_kids) {
        let is_field = new
            .get_object(*kid)
            .ok()
            .and_then(dict_of)
            .is_some_and(|d| d.has(b"T"));
        if is_field {
            found.push(
                ChangeKind::FieldAdded,
                None,
                "A form field was added".into(),
            );
        }
    }
    let value_keys: &[&[u8]] = &[b"V", b"AP", b"AS", b"Kids", b"MK", b"DA"];
    if !is_sig && strip_keys(before, value_keys) == strip_keys(after, value_keys) {
        found.field(name);
    } else if !is_sig {
        // Flags, type or options changed: more than a value.
        found.field(name);
        let flags = |d: &Dictionary| d.get(b"Ff").ok().cloned();
        if flags(b) != flags(a) || b.get(b"FT").ok() != a.get(b"FT").ok() {
            found.push(
                ChangeKind::Other,
                None,
                format!("The settings of form field {name} were changed"),
            );
        }
    }
}

fn catalog_changed(
    old: &Document,
    new: &Document,
    before: &Object,
    after: &Object,
    found: &mut Found,
) {
    let (Some(b), Some(a)) = (dict_of(before), dict_of(after)) else {
        return;
    };
    // Keys an update may legitimately touch.
    let quiet: &[&[u8]] = &[b"AcroForm", b"DSS", b"Metadata", b"Extensions", b"Version"];
    let keys: BTreeSet<&Vec<u8>> = b.as_hashmap().keys().chain(a.as_hashmap().keys()).collect();
    for key in keys {
        if quiet.contains(&key.as_slice()) {
            continue;
        }
        if b.get(key).ok() != a.get(key).ok() {
            let what = String::from_utf8_lossy(key);
            let detail = match key.as_slice() {
                b"Pages" => "The page tree was replaced".to_string(),
                b"OpenAction" | b"AA" | b"Names" => {
                    format!("Document actions or names were changed ({what})")
                }
                _ => format!("Document settings were changed ({what})"),
            };
            found.push(ChangeKind::Other, None, detail);
        }
    }
    if b.get(b"Metadata").ok() != a.get(b"Metadata").ok() {
        found.push(
            ChangeKind::Metadata,
            None,
            "Document information was changed".into(),
        );
    }
    // A form added where there was none: its fields are new fields.
    let had_form = b.has(b"AcroForm");
    if !had_form && a.has(b"AcroForm") {
        let fields = a
            .get(b"AcroForm")
            .ok()
            .and_then(|f| resolve(new, f))
            .cloned();
        if let Some(after_form) = fields {
            acroform_changed(
                old,
                new,
                &Object::Dictionary(Dictionary::new()),
                &after_form,
                found,
            );
        }
    } else if let (Ok(Object::Dictionary(_)), Ok(Object::Dictionary(_))) =
        (b.get(b"AcroForm"), a.get(b"AcroForm"))
    {
        // An inline AcroForm changes with the catalog.
        let (bf, af) = (b.get(b"AcroForm").unwrap(), a.get(b"AcroForm").unwrap());
        if bf != af {
            acroform_changed(old, new, bf, af, found);
        }
    }
}

fn acroform_changed(
    old: &Document,
    new: &Document,
    before: &Object,
    after: &Object,
    found: &mut Found,
) {
    // `before`/`after` are the form dictionary, or its /Fields array when
    // that is an object of its own.
    let fields = |doc: &Document, o: &Object| -> HashSet<ObjectId> {
        let list = match o {
            Object::Array(list) => Some(list),
            _ => dict_of(o)
                .and_then(|d| resolve(doc, d.get(b"Fields").ok()?))
                .and_then(|f| f.as_array().ok()),
        };
        list.map(|f| {
            f.iter()
                .filter_map(|o| match o {
                    Object::Reference(id) => Some(*id),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
    };
    let before_fields = fields(old, before);
    let after_fields = fields(new, after);
    for added in after_fields.difference(&before_fields) {
        let Some(dict) = new.get_object(*added).ok().and_then(dict_of) else {
            continue;
        };
        let is_sig = matches!(dict.get(b"FT"), Ok(Object::Name(n)) if n == b"Sig");
        let name = dict
            .get(b"T")
            .ok()
            .and_then(|t| lopdf::decode_text_string(t).ok())
            .unwrap_or_default();
        if is_sig && dict.has(b"V") {
            found.signed.insert(name);
        } else if is_sig {
            found.push(
                ChangeKind::FieldAdded,
                None,
                "A signature field was added".into(),
            );
        } else {
            found.push(
                ChangeKind::FieldAdded,
                None,
                "A form field was added".into(),
            );
        }
    }
    if before_fields.difference(&after_fields).next().is_some() {
        found.push(ChangeKind::Other, None, "A form field was removed".into());
    }
    // XFA is a second copy of the form; changing it changes what XFA readers
    // show, which is still a form edit.
    let xfa = |o: &Object| dict_of(o).and_then(|d| d.get(b"XFA").ok().cloned());
    if xfa(before) != xfa(after) && xfa(after).is_some() {
        found.push(
            ChangeKind::Other,
            None,
            "The form's XFA data was changed".into(),
        );
    }
}

/// Widget annotation id -> fully qualified name, for every signature field.
fn signature_widgets(doc: &Document) -> HashMap<ObjectId, String> {
    let mut out = HashMap::new();
    for (id, name, ft) in all_fields(doc) {
        if ft.as_deref() == Some(b"Sig".as_slice()) {
            for widget in widgets_of(doc, id) {
                out.insert(widget, name.clone());
            }
        }
    }
    out
}

/// Whether the signature field behind widget `id` holds a value.
fn is_signed_field(doc: &Document, id: ObjectId) -> bool {
    let Some(dict) = doc.get_object(id).ok().and_then(dict_of) else {
        return false;
    };
    if dict.has(b"V") {
        return true;
    }
    match dict.get(b"Parent") {
        Ok(Object::Reference(parent)) => doc
            .get_object(*parent)
            .ok()
            .and_then(dict_of)
            .is_some_and(|p| p.has(b"V")),
        _ => false,
    }
}

/// Every field node: (object, fully qualified name, inherited /FT).
fn all_fields(doc: &Document) -> Vec<(ObjectId, String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let Some(roots) = doc
        .catalog()
        .ok()
        .and_then(|c| resolve(doc, c.get(b"AcroForm").ok()?))
        .and_then(|a| a.as_dict().ok())
        .and_then(|a| resolve(doc, a.get(b"Fields").ok()?))
        .and_then(|f| f.as_array().ok())
    else {
        return out;
    };
    let mut stack: Vec<(ObjectId, String, Option<Vec<u8>>, usize)> = roots
        .iter()
        .filter_map(|r| match r {
            Object::Reference(id) => Some((*id, String::new(), None, 0)),
            _ => None,
        })
        .collect();
    let mut seen = HashSet::new();
    while let Some((id, parent, parent_ft, depth)) = stack.pop() {
        if depth > 32 || !seen.insert(id) {
            continue;
        }
        let Some(dict) = doc.get_object(id).ok().and_then(dict_of) else {
            continue;
        };
        let partial = dict
            .get(b"T")
            .ok()
            .and_then(|t| lopdf::decode_text_string(t).ok());
        let name = match (&partial, parent.is_empty()) {
            (Some(t), true) => t.clone(),
            (Some(t), false) => format!("{parent}.{t}"),
            (None, _) => parent.clone(),
        };
        let ft = match dict.get(b"FT") {
            Ok(Object::Name(n)) => Some(n.clone()),
            _ => parent_ft,
        };
        if partial.is_some() {
            out.push((id, name.clone(), ft.clone()));
        }
        if let Some(kids) = dict
            .get(b"Kids")
            .ok()
            .and_then(|k| resolve(doc, k))
            .and_then(|k| k.as_array().ok())
        {
            for kid in kids {
                if let Object::Reference(kid) = kid {
                    stack.push((*kid, name.clone(), ft.clone(), depth + 1));
                }
            }
        }
    }
    out
}

/// A field's widgets: itself when merged, or its kids without a /T.
fn widgets_of(doc: &Document, field: ObjectId) -> Vec<ObjectId> {
    let Some(dict) = doc.get_object(field).ok().and_then(dict_of) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if subtype(dict) == Some(b"Widget") {
        out.push(field);
    }
    if let Some(kids) = dict
        .get(b"Kids")
        .ok()
        .and_then(|k| resolve(doc, k))
        .and_then(|k| k.as_array().ok())
    {
        for kid in kids {
            if let Object::Reference(kid) = kid {
                let is_widget = doc
                    .get_object(*kid)
                    .ok()
                    .and_then(dict_of)
                    .is_some_and(|d| !d.has(b"T"));
                if is_widget {
                    out.push(*kid);
                }
            }
        }
    }
    out
}

/// Every object reachable from `start`, not crossing into `/Parent` or `/P`
/// (which lead back up to pages and the whole document).
fn reach(doc: &Document, start: &Object, into: &mut HashSet<ObjectId>, limit: usize) {
    let mut stack: Vec<&Object> = vec![start];
    while let Some(object) = stack.pop() {
        if into.len() > limit {
            return;
        }
        match object {
            Object::Reference(id) if into.insert(*id) => {
                if let Ok(target) = doc.get_object(*id) {
                    stack.push(target);
                }
            }
            Object::Array(items) => stack.extend(items.iter()),
            Object::Dictionary(dict) => {
                for (key, value) in dict.iter() {
                    if key != b"Parent" && key != b"P" {
                        stack.push(value);
                    }
                }
            }
            Object::Stream(stream) => {
                for (key, value) in stream.dict.iter() {
                    if key != b"Parent" && key != b"P" {
                        stack.push(value);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The role each object of the signed revision played.
fn roles_of(doc: &Document) -> HashMap<ObjectId, Role> {
    const LIMIT: usize = 200_000;
    let mut roles: HashMap<ObjectId, Role> = HashMap::new();
    // Lower-priority roles first; later inserts win.
    let assign = |ids: HashSet<ObjectId>, role: Role, roles: &mut HashMap<ObjectId, Role>| {
        for id in ids {
            roles.insert(id, role.clone());
        }
    };

    if let Ok(Object::Reference(info)) = doc.trailer.get(b"Info") {
        roles.insert(*info, Role::Info);
    }
    let catalog = doc.catalog().ok();
    let root = root_id(doc);

    // Validation data and form resources: housekeeping.
    if let Some(catalog) = catalog {
        if let Ok(dss) = catalog.get(b"DSS") {
            let mut ids = HashSet::new();
            reach(doc, dss, &mut ids, LIMIT);
            assign(ids, Role::Dss, &mut roles);
        }
        if let Some(form) = catalog
            .get(b"AcroForm")
            .ok()
            .and_then(|f| resolve(doc, f))
            .and_then(dict_of)
        {
            if let Ok(dr) = form.get(b"DR") {
                let mut ids = HashSet::new();
                reach(doc, dr, &mut ids, LIMIT);
                assign(ids, Role::FormResources, &mut roles);
            }
        }
    }

    // Annotations and their appearances, page by page. A widget's
    // appearance objects are named after the widget once fields are known.
    let pages = page_ids(doc);
    let mut widget_parts: Vec<(ObjectId, HashSet<ObjectId>)> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        for annot in page_annots(doc, *page) {
            let Some(dict) = doc.get_object(annot).ok().and_then(dict_of) else {
                continue;
            };
            let mut ids = HashSet::new();
            ids.insert(annot);
            for key in [b"AP".as_slice(), b"Popup", b"MK"] {
                if let Ok(value) = dict.get(key) {
                    reach(doc, value, &mut ids, LIMIT);
                }
            }
            if subtype(dict) == Some(b"Widget") {
                widget_parts.push((annot, ids));
            } else {
                assign(ids, Role::Annotation(index as u32), &mut roles);
            }
        }
    }

    // Fields, by name; a merged field-widget keeps the name.
    for (id, name, _) in all_fields(doc) {
        let merged = doc
            .get_object(id)
            .ok()
            .and_then(dict_of)
            .is_some_and(|d| subtype(d) == Some(b"Widget"));
        if merged {
            roles.insert(id, Role::Widget(name.clone()));
        } else {
            roles.insert(id, Role::Field(name.clone()));
        }
        for widget in widgets_of(doc, id) {
            if widget != id {
                roles.insert(widget, Role::Widget(name.clone()));
            }
        }
    }
    // Each widget's appearance objects carry its field's name. A widget the
    // field tree does not reach is named by its own /T, if any.
    for (widget, parts) in widget_parts {
        let name = match roles.get(&widget) {
            Some(Role::Widget(name)) => name.clone(),
            _ => doc
                .get_object(widget)
                .ok()
                .and_then(dict_of)
                .and_then(|d| d.get(b"T").ok())
                .and_then(|t| lopdf::decode_text_string(t).ok())
                .unwrap_or_else(|| format!("unnamed field {}", widget.0)),
        };
        for id in parts {
            match roles.get(&id) {
                Some(Role::Field(_)) => {}
                _ => {
                    roles.insert(id, Role::Widget(name.clone()));
                }
            }
        }
    }

    // Page content wins over everything above: an object a page draws is
    // page content even if a widget's appearance also uses it.
    for (index, page) in pages.iter().enumerate() {
        let Some(dict) = doc.get_object(*page).ok().and_then(dict_of) else {
            continue;
        };
        let mut ids = HashSet::new();
        for key in [b"Contents".as_slice(), b"Resources", b"Group", b"Thumb"] {
            if let Ok(value) = dict.get(key) {
                reach(doc, value, &mut ids, LIMIT);
            }
        }
        // Inherited resources live on the Pages node; they are caught as
        // PagesNode changes.
        assign(ids, Role::PageContent(index as u32), &mut roles);
        roles.insert(*page, Role::Page(index as u32));
    }
    // Pages tree nodes.
    if let Some(catalog) = catalog {
        let mut stack: Vec<ObjectId> = match catalog.get(b"Pages") {
            Ok(Object::Reference(id)) => vec![*id],
            _ => Vec::new(),
        };
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) || seen.len() > LIMIT {
                continue;
            }
            let Some(dict) = doc.get_object(id).ok().and_then(dict_of) else {
                continue;
            };
            if matches!(dict.get(b"Type"), Ok(Object::Name(n)) if n == b"Pages") {
                roles.insert(id, Role::PagesNode);
                if let Some(kids) = dict
                    .get(b"Kids")
                    .ok()
                    .and_then(|k| resolve(doc, k))
                    .and_then(|k| k.as_array().ok())
                {
                    stack.extend(kids.iter().filter_map(|k| match k {
                        Object::Reference(id) => Some(*id),
                        _ => None,
                    }));
                }
            }
        }
        if let Ok(form) = catalog.get(b"AcroForm") {
            if let Object::Reference(id) = form {
                roles.insert(*id, Role::AcroForm);
            }
            // An indirect /Fields array is part of the form dictionary.
            if let Some(Object::Reference(fields)) = resolve(doc, form)
                .and_then(dict_of)
                .and_then(|d| d.get(b"Fields").ok())
            {
                roles.insert(*fields, Role::AcroForm);
            }
        }
        if let Ok(Object::Reference(meta)) = catalog.get(b"Metadata") {
            roles.insert(*meta, Role::Metadata);
        }
    }
    if let Some(root) = root {
        roles.insert(root, Role::Catalog);
    }
    roles
}

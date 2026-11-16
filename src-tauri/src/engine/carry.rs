//! Carrying the document-level structure across a page restructure.
//!
//! A save that deletes, reorders, duplicates or inserts pages assembles a new
//! document from imported pages (see `save::build_output`). `FPDF_ImportPages`
//! copies each page with its annotations but nothing from the catalog, so on
//! its own that output has no outline, no named destinations (hyperref links
//! survive but lead nowhere), no XMP metadata, no page labels and no form.
//!
//! This pass puts them back. It reads the source with lopdf, rebuilds each of
//! those structures against the new page order, and appends the result to
//! PDFium's output as one incremental update - the original output bytes are
//! never touched, so if anything here goes wrong the caller keeps what PDFium
//! wrote. What gets carried, and how:
//!
//! - outline: every entry is rebuilt with fresh links between siblings and
//!   parents. A destination on a page that is gone is removed; the entry
//!   itself is dropped unless it still has children, which keep their place
//!   under it. A duplicated page is reached at its first copy.
//! - named destinations, both the catalog's /Dests dictionary and the
//!   /Names /Dests name tree: re-pointed at the new pages, or removed when
//!   their page is gone. Links name them by string, and import keeps those
//!   strings, so they resolve again.
//! - page labels: recomputed page by page from the source's ranges and
//!   written as fresh ranges for the new order. Inserted blank pages get an
//!   empty label.
//! - links with an explicit page destination: rewritten from their source.
//!   Import re-points one only when its target page was imported before it,
//!   so a link to a later page used to lose its destination.
//! - XMP metadata, page mode and layout, viewer preferences, language and an
//!   open action that does not point at a deleted page: copied.
//! - the AcroForm: its field tree is rebuilt from the widgets that came across
//!   with their pages. Widgets on deleted pages are left out, and a field that
//!   ends up with no widgets goes with them.
//!
//! Objects are copied with a reference-following copier that maps source pages
//! to their new counterparts and never follows a reference into the page tree
//! or onto another annotation, so a copy cannot drag pages or stray widgets
//! along with it. New objects are numbered past the output's declared /Size.

use lopdf::{Dictionary, Document, IncrementalDocument, LoadOptions, Object, ObjectId};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// How deep a chain of references the generic copier follows. Anything this
/// deep in a catalog structure is malformed; it is cut off rather than
/// recursed into.
const MAX_COPY_DEPTH: usize = 64;
/// How deep an outline or name tree is followed.
const MAX_TREE_DEPTH: usize = 64;
/// Name tree leaves written per kid when the tree is rebuilt.
const NAMES_PER_LEAF: usize = 256;

/// What the pass carried over, for logs and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Carried {
    /// Outline entries written.
    pub outline_items: usize,
    /// Outline entries dropped because their page is gone.
    pub outline_dropped: usize,
    /// Named destinations written (dictionary and name tree together).
    pub named_dests: usize,
    /// Named destinations dropped because their page is gone.
    pub named_dests_dropped: usize,
    /// Whether page labels were written.
    pub page_labels: bool,
    /// Whether an XMP metadata stream was copied.
    pub metadata: bool,
    /// Form fields (terminal and not) in the rebuilt field tree.
    pub form_fields: usize,
}

impl Carried {
    fn is_empty(&self) -> bool {
        *self == Carried::default()
    }
}

/// Build the incremental update that gives `out_bytes` - a document whose
/// pages are `layout`, each the source page it names or a new blank one - the
/// catalog structure of the source at `src_path`.
///
/// Answers the whole new file (the output bytes followed by the update), or
/// `None` when the source has nothing worth carrying. An error means the
/// source or output could not be read in a shape this understands; the caller
/// then keeps the output as it was.
pub fn carry_catalog(
    src_path: &Path,
    password: Option<&str>,
    layout: &[Option<u32>],
    out_bytes: &[u8],
) -> Result<Option<(Vec<u8>, Carried)>, String> {
    let options = match password {
        Some(password) => LoadOptions::with_password(password),
        None => LoadOptions::default(),
    };
    let src = Document::load_with_options(src_path, options)
        .map_err(|e| format!("cannot read the source: {e}"))?;
    if src.is_encrypted() && !src.was_encrypted() {
        return Err("the source is encrypted and could not be decrypted".into());
    }
    let src_catalog = src
        .catalog()
        .map_err(|e| format!("source has no catalog: {e}"))?;

    let mut inc: IncrementalDocument = out_bytes
        .try_into()
        .map_err(|e: lopdf::Error| format!("cannot read the output: {e}"))?;
    if inc.get_prev_documents().is_encrypted() {
        return Err("the output is encrypted".into());
    }
    // New objects must be numbered past the previous trailer's /Size: lopdf
    // counts only the objects it parsed, and a cross-reference stream is not
    // among them (see the same guard in `markup_export`).
    let declared = inc
        .get_prev_documents()
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    let root_id = inc
        .get_prev_documents()
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|e| format!("output has no root: {e}"))?;

    let src_pages: Vec<ObjectId> = src.get_pages().into_values().collect();
    let out_pages: Vec<ObjectId> = inc.get_prev_documents().get_pages().into_values().collect();
    if out_pages.len() != layout.len() {
        return Err(format!(
            "output has {} pages where {} were planned",
            out_pages.len(),
            layout.len()
        ));
    }

    // Every copy of every kept source page, in output order.
    let mut copies: HashMap<ObjectId, Vec<(usize, ObjectId)>> = HashMap::new();
    for (index, (slot, &out_page)) in layout.iter().zip(&out_pages).enumerate() {
        let Some(src_index) = slot else { continue };
        let src_page = src_pages
            .get(*src_index as usize)
            .ok_or_else(|| format!("plan names source page {src_index}, which does not exist"))?;
        copies.entry(*src_page).or_default().push((index, out_page));
    }

    let in_order: Vec<(ObjectId, ObjectId)> = layout
        .iter()
        .zip(&out_pages)
        .filter_map(|(slot, out_page)| Some((*src_pages.get((*slot)? as usize)?, *out_page)))
        .collect();
    let widgets = match_annotations(&src, inc.get_prev_documents(), &in_order);

    let mut copier = Copier {
        src: &src,
        out: &mut inc.new_document,
        memo: HashMap::new(),
        pages: copies
            .iter()
            .map(|(src_page, list)| (*src_page, list[0].1))
            .collect(),
        page_index: src_pages
            .iter()
            .enumerate()
            .filter_map(|(i, id)| copies.get(id).map(|list| (i as i64, list[0].0 as i64)))
            .collect(),
        page_nodes: HashSet::new(),
        fixed: HashMap::new(),
        gone_names: HashSet::new(),
        claimed: Vec::new(),
    };
    copier.collect_page_nodes();

    let mut carried = Carried::default();
    let mut catalog_updates: Vec<(&'static str, Object)> = Vec::new();

    // Named destinations first: outline entries and the open action may name
    // one, and need to know whether it survived.
    if let Ok(dests) = deref_dict(&src, src_catalog.get(b"Dests").ok()) {
        let mut kept = Dictionary::new();
        for (name, value) in dests.iter() {
            match copier.dest(value) {
                Some(dest) => kept.set(name.clone(), dest),
                None => {
                    copier.gone_names.insert(name.clone());
                    carried.named_dests_dropped += 1;
                }
            }
        }
        carried.named_dests += kept.len();
        if !kept.is_empty() {
            let id = copier.out.add_object(kept);
            catalog_updates.push(("Dests", Object::Reference(id)));
        }
    }
    let src_names = deref_dict(&src, src_catalog.get(b"Names").ok()).ok();
    let mut names = Dictionary::new();
    if let Some(src_names) = src_names {
        if let Ok(tree) = deref_dict(&src, src_names.get(b"Dests").ok()) {
            let mut entries = Vec::new();
            flatten_name_tree(&src, tree, &mut entries, &mut HashSet::new(), 0);
            let mut kept: Vec<(Vec<u8>, Object)> = Vec::new();
            for (name, value) in entries {
                match copier.dest(&value) {
                    Some(dest) => kept.push((name, dest)),
                    None => {
                        copier.gone_names.insert(name);
                        carried.named_dests_dropped += 1;
                    }
                }
            }
            kept.sort_by(|a, b| a.0.cmp(&b.0));
            kept.dedup_by(|a, b| a.0 == b.0);
            carried.named_dests += kept.len();
            if !kept.is_empty() {
                let tree = write_name_tree(copier.out, kept);
                names.set("Dests", Object::Reference(tree));
            }
        }
        // Attachments travel too: they belong to no page.
        if let Ok(files) = src_names.get(b"EmbeddedFiles") {
            let files = copier.value(files, 0);
            if !matches!(files, Object::Null) {
                names.set("EmbeddedFiles", files);
            }
        }
    }
    if !names.is_empty() {
        catalog_updates.push(("Names", Object::Dictionary(names)));
    }

    if let Ok(outlines) = deref_dict(&src, src_catalog.get(b"Outlines").ok()) {
        let mut visited = HashSet::new();
        let first = outlines.get(b"First").and_then(Object::as_reference).ok();
        let items = copier.outline_items(first, 0, &mut visited, &mut carried.outline_dropped);
        if !items.is_empty() {
            let root = copier.out.new_object_id();
            let (first, last, visible, written) = write_outline_items(copier.out, items, root);
            carried.outline_items = written;
            let mut dict = Dictionary::new();
            dict.set("Type", Object::Name(b"Outlines".to_vec()));
            dict.set("First", Object::Reference(first));
            dict.set("Last", Object::Reference(last));
            dict.set("Count", Object::Integer(visible));
            copier.out.set_object(root, dict);
            catalog_updates.push(("Outlines", Object::Reference(root)));
        }
    }

    if let Ok(labels) = deref_dict(&src, src_catalog.get(b"PageLabels").ok()) {
        if let Some(nums) = remap_page_labels(&src, labels, layout) {
            let mut dict = Dictionary::new();
            dict.set("Nums", Object::Array(nums));
            let id = copier.out.add_object(dict);
            catalog_updates.push(("PageLabels", Object::Reference(id)));
            carried.page_labels = true;
        }
    }

    if let Ok(metadata) = src_catalog.get(b"Metadata") {
        let metadata = copier.value(metadata, 0);
        if !matches!(metadata, Object::Null) {
            catalog_updates.push(("Metadata", metadata));
            carried.metadata = true;
        }
    }

    if let Ok(form) = deref_dict(&src, src_catalog.get(b"AcroForm").ok()) {
        if let Some((form, fields)) = copier.form(form, &widgets) {
            carried.form_fields = fields;
            let id = copier.out.add_object(form);
            catalog_updates.push(("AcroForm", Object::Reference(id)));
        }
    }

    if let Ok(action) = src_catalog.get(b"OpenAction") {
        let action = match src.dereference(action) {
            Ok((_, array @ Object::Array(_))) => copier.dest(array),
            Ok((_, Object::Dictionary(_))) => copier.action(action),
            _ => None,
        };
        if let Some(action) = action {
            catalog_updates.push(("OpenAction", action));
        }
    }
    for key in ["PageMode", "PageLayout", "ViewerPreferences", "Lang"] {
        if let Ok(value) = src_catalog.get(key.as_bytes()) {
            let value = copier.value(value, 0);
            if !matches!(value, Object::Null) {
                catalog_updates.push((key, value));
            }
        }
    }
    // Links with an explicit page destination. Import re-points one only when
    // its target page was imported before it; one pointing further on loses
    // its destination. Rewrite each from its source, so it lands on the first
    // copy of its page, or leads nowhere if that page is gone.
    let mut link_fixes: Vec<(ObjectId, &'static str, Option<Object>)> = Vec::new();
    for (source, out_link) in &widgets.links {
        if let Ok(dest) = source.get(b"Dest") {
            if matches!(src.dereference(dest), Ok((_, Object::Array(_)))) {
                link_fixes.push((*out_link, "Dest", copier.dest(dest)));
            }
        } else if let Ok(action) = source.get(b"A") {
            let explicit = deref_dict(&src, Some(action)).is_ok_and(|action| {
                is_type(action, b"S", &[b"GoTo"])
                    && action
                        .get(b"D")
                        .and_then(|d| src.dereference(d))
                        .is_ok_and(|(_, d)| matches!(d, Object::Array(_)))
            });
            if explicit {
                link_fixes.push((*out_link, "A", copier.action(action)));
            }
        }
    }
    let mut fixes = std::mem::take(&mut copier.claimed);
    drop(copier);
    // A widget the field tree does not claim - a further copy of a widget
    // that is its own field, or one on a page whose widgets could not be
    // matched - must not point at a parent that is not in the tree.
    let claimed: HashSet<ObjectId> = fixes.iter().map(|(widget, _)| *widget).collect();
    for widget in &widgets.all_out {
        let has_parent = inc
            .get_prev_documents()
            .get_dictionary(*widget)
            .is_ok_and(|dict| dict.has(b"Parent"));
        if has_parent && !claimed.contains(widget) {
            fixes.push((*widget, None));
        }
    }

    if carried.is_empty() && catalog_updates.is_empty() {
        return Ok(None);
    }
    for (widget, parent) in fixes {
        inc.opt_clone_object_to_new_document(widget)
            .map_err(|e| format!("cannot update widget: {e}"))?;
        let dict = inc
            .new_document
            .get_dictionary_mut(widget)
            .map_err(|e| format!("cannot update widget: {e}"))?;
        match parent {
            Some(parent) => dict.set("Parent", Object::Reference(parent)),
            None => {
                dict.remove(b"Parent");
            }
        }
    }
    for (link, key, value) in link_fixes {
        inc.opt_clone_object_to_new_document(link)
            .map_err(|e| format!("cannot update link: {e}"))?;
        let dict = inc
            .new_document
            .get_dictionary_mut(link)
            .map_err(|e| format!("cannot update link: {e}"))?;
        match value {
            Some(value) => dict.set(key, value),
            None => {
                dict.remove(key.as_bytes());
            }
        }
    }
    inc.opt_clone_object_to_new_document(root_id)
        .map_err(|e| format!("cannot update the catalog: {e}"))?;
    let catalog = inc
        .new_document
        .get_dictionary_mut(root_id)
        .map_err(|e| format!("cannot update the catalog: {e}"))?;
    for (key, value) in catalog_updates {
        catalog.set(key, value);
    }

    let mut bytes = Vec::with_capacity(out_bytes.len() + 64 * 1024);
    inc.save_to(&mut bytes)
        .map_err(|e| format!("cannot write the update: {e}"))?;
    Ok(Some((bytes, carried)))
}

/// Source annotations matched to their copies in the output. Import keeps a
/// page's annotations in order, and the save only adds or removes annotation
/// kinds the editor owns - never widgets or links - so the n-th widget (or
/// link) of a source page is the n-th widget (or link) of each of its copies.
struct Widgets {
    /// Source widget -> its copies, in output page order.
    copies: HashMap<ObjectId, Vec<ObjectId>>,
    /// Every widget in the output, to clear stale /Parent links on the ones
    /// the rebuilt field tree does not claim.
    all_out: Vec<ObjectId>,
    /// Each output link that is an indirect object, with its source.
    links: Vec<(Dictionary, ObjectId)>,
}

/// The annotations of `subtype` on `page`: object id when indirect, and the
/// dictionary.
fn annots_of<'d>(
    doc: &'d Document,
    page: ObjectId,
    subtype: &[u8],
) -> Vec<(Option<ObjectId>, &'d Dictionary)> {
    let Ok(page) = doc.get_dictionary(page) else {
        return Vec::new();
    };
    let Ok((_, Object::Array(annots))) = page
        .get(b"Annots")
        .and_then(|annots| doc.dereference(annots))
    else {
        return Vec::new();
    };
    annots
        .iter()
        .filter_map(|annot| {
            let id = annot.as_reference().ok();
            let (_, object) = doc.dereference(annot).ok()?;
            let dict = object.as_dict().ok()?;
            is_type(dict, b"Subtype", &[subtype]).then_some((id, dict))
        })
        .collect()
}

/// `copies` in output order: (source page, output page) per output page.
fn match_annotations(src: &Document, out: &Document, in_order: &[(ObjectId, ObjectId)]) -> Widgets {
    let mut matched: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
    let mut all_out = Vec::new();
    let mut links = Vec::new();
    for &(src_page, out_page) in in_order {
        let src_widgets = annots_of(src, src_page, b"Widget");
        let out_widgets = annots_of(out, out_page, b"Widget");
        all_out.extend(out_widgets.iter().filter_map(|(id, _)| *id));
        if src_widgets.len() == out_widgets.len() {
            for ((from, _), (to, _)) in src_widgets.iter().zip(&out_widgets) {
                if let (Some(from), Some(to)) = (from, to) {
                    matched.entry(*from).or_default().push(*to);
                }
            }
        } else {
            log::warn!("widgets on a page did not survive import one for one; skipping them");
        }
        let src_links = annots_of(src, src_page, b"Link");
        let out_links = annots_of(out, out_page, b"Link");
        if src_links.len() == out_links.len() {
            for ((_, from), (to, _)) in src_links.iter().zip(&out_links) {
                if let Some(to) = to {
                    links.push(((*from).clone(), *to));
                }
            }
        } else {
            log::warn!("links on a page did not survive import one for one; skipping them");
        }
    }
    Widgets {
        copies: matched,
        all_out,
        links,
    }
}

/// A rebuilt outline entry, before it is numbered and linked.
struct Item {
    dict: Dictionary,
    open: bool,
    children: Vec<Item>,
}

struct Copier<'a> {
    src: &'a Document,
    out: &'a mut Document,
    /// Source object -> its copy in the output.
    memo: HashMap<ObjectId, ObjectId>,
    /// Source page -> its first copy in the output.
    pages: HashMap<ObjectId, ObjectId>,
    /// Source page index -> output index of its first copy.
    page_index: HashMap<i64, i64>,
    /// Every page tree node in the source, kept or not.
    page_nodes: HashSet<ObjectId>,
    /// Source objects already rebuilt by hand (form fields, widgets).
    fixed: HashMap<ObjectId, ObjectId>,
    /// Named destinations dropped because their page is gone.
    gone_names: HashSet<Vec<u8>>,
    /// Output widgets the rebuilt field tree claimed, with their new parent.
    claimed: Vec<(ObjectId, Option<ObjectId>)>,
}

fn is_type(dict: &Dictionary, key: &[u8], names: &[&[u8]]) -> bool {
    dict.get(key)
        .and_then(Object::as_name)
        .is_ok_and(|name| names.contains(&name))
}

fn deref_dict<'d>(doc: &'d Document, object: Option<&'d Object>) -> lopdf::Result<&'d Dictionary> {
    let object = object.ok_or(lopdf::Error::DictKey("missing".into()))?;
    doc.dereference(object)?.1.as_dict()
}

impl Copier<'_> {
    fn collect_page_nodes(&mut self) {
        for (id, object) in &self.src.objects {
            if let Ok(dict) = object.as_dict() {
                if is_type(dict, b"Type", &[b"Page", b"Pages"]) {
                    self.page_nodes.insert(*id);
                }
            }
        }
    }

    /// A copy of `object` for the output, references followed and remapped.
    fn value(&mut self, object: &Object, depth: usize) -> Object {
        match object {
            Object::Reference(id) => self.reference(*id, depth),
            Object::Array(items) => {
                Object::Array(items.iter().map(|item| self.value(item, depth)).collect())
            }
            Object::Dictionary(dict) => Object::Dictionary(self.dict(dict, depth)),
            Object::Stream(stream) => {
                let mut copy = stream.clone();
                copy.dict = self.dict(&stream.dict, depth);
                Object::Stream(copy)
            }
            other => other.clone(),
        }
    }

    fn dict(&mut self, dict: &Dictionary, depth: usize) -> Dictionary {
        let mut copy = Dictionary::new();
        for (key, value) in dict.iter() {
            copy.set(key.clone(), self.value(value, depth));
        }
        copy
    }

    fn reference(&mut self, id: ObjectId, depth: usize) -> Object {
        if let Some(page) = self.pages.get(&id) {
            return Object::Reference(*page);
        }
        if let Some(fixed) = self.fixed.get(&id) {
            return Object::Reference(*fixed);
        }
        if let Some(copy) = self.memo.get(&id) {
            return Object::Reference(*copy);
        }
        if self.page_nodes.contains(&id) || depth >= MAX_COPY_DEPTH {
            return Object::Null;
        }
        let src = self.src;
        let Ok(object) = src.get_object(id) else {
            return Object::Null;
        };
        let dict = match object {
            Object::Dictionary(dict) => Some(dict),
            Object::Stream(stream) => Some(&stream.dict),
            _ => None,
        };
        // Never drag an annotation along: the ones that matter came across
        // with their pages, and any other one belongs to a page that did not.
        if dict.is_some_and(|dict| {
            is_type(dict, b"Type", &[b"Annot", b"Page", b"Pages"])
                || is_type(dict, b"Subtype", &[b"Widget", b"Link"])
        }) {
            return Object::Null;
        }
        let copy_id = self.out.new_object_id();
        self.memo.insert(id, copy_id);
        let copy = self.value(object, depth + 1);
        self.out.set_object(copy_id, copy);
        Object::Reference(copy_id)
    }

    /// A destination re-pointed at the new pages: an explicit one, a
    /// dictionary holding one under /D, or a name. `None` when it leads to a
    /// page that is gone.
    fn dest(&mut self, dest: &Object) -> Option<Object> {
        let src = self.src;
        let (_, dest) = src.dereference(dest).ok()?;
        match dest {
            Object::Array(items) => {
                let page = match items.first()? {
                    Object::Reference(page) => Object::Reference(*self.pages.get(page)?),
                    Object::Integer(index) => Object::Integer(*self.page_index.get(index)?),
                    _ => return None,
                };
                let mut out = vec![page];
                out.extend(items[1..].iter().map(|item| self.value(item, 0)));
                Some(Object::Array(out))
            }
            Object::Dictionary(dict) => {
                let target = self.dest(dict.get(b"D").ok()?)?;
                let mut out = Dictionary::new();
                for (key, value) in dict.iter() {
                    if key.as_slice() != b"D" {
                        out.set(key.clone(), self.value(value, 0));
                    }
                }
                out.set("D", target);
                Some(Object::Dictionary(out))
            }
            Object::Name(name) | Object::String(name, _) => {
                if self.gone_names.contains(name) {
                    None
                } else {
                    Some(dest.clone())
                }
            }
            _ => None,
        }
    }

    /// An action re-pointed at the new pages. `None` when it is a GoTo whose
    /// destination is gone.
    fn action(&mut self, action: &Object) -> Option<Object> {
        let src = self.src;
        let (_, action) = src.dereference(action).ok()?;
        let dict = action.as_dict().ok()?;
        if !is_type(dict, b"S", &[b"GoTo"]) {
            return Some(self.value(action, 0));
        }
        let target = self.dest(dict.get(b"D").ok()?)?;
        let mut out = Dictionary::new();
        for (key, value) in dict.iter() {
            if key.as_slice() != b"D" {
                out.set(key.clone(), self.value(value, 0));
            }
        }
        out.set("D", target);
        Some(Object::Dictionary(out))
    }

    fn outline_items(
        &mut self,
        first: Option<ObjectId>,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
        dropped: &mut usize,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        if depth >= MAX_TREE_DEPTH {
            return items;
        }
        let src = self.src;
        let mut next = first;
        while let Some(id) = next {
            if !visited.insert(id) {
                break;
            }
            let Ok(dict) = src.get_dictionary(id) else {
                break;
            };
            next = dict.get(b"Next").and_then(Object::as_reference).ok();
            let child = dict.get(b"First").and_then(Object::as_reference).ok();
            let children = self.outline_items(child, depth + 1, visited, dropped);

            let mut out = Dictionary::new();
            let mut lost = false;
            for (key, value) in dict.iter() {
                match key.as_slice() {
                    // Rebuilt below, or (/SE, the structure element) not
                    // carried: the structure tree does not come across.
                    b"Parent" | b"First" | b"Last" | b"Prev" | b"Next" | b"Count" | b"SE" => {}
                    b"Dest" => match self.dest(value) {
                        Some(dest) => out.set("Dest", dest),
                        None => lost = true,
                    },
                    b"A" => match self.action(value) {
                        Some(action) => out.set("A", action),
                        None => lost = true,
                    },
                    _ => out.set(key.clone(), self.value(value, 0)),
                }
            }
            if lost && children.is_empty() {
                *dropped += 1;
                continue;
            }
            let open = dict
                .get(b"Count")
                .and_then(Object::as_i64)
                .is_ok_and(|count| count > 0);
            items.push(Item {
                dict: out,
                open,
                children,
            });
        }
        items
    }

    /// The rebuilt AcroForm and the number of fields in it, or `None` when no
    /// field survived.
    fn form(&mut self, form: &Dictionary, widgets: &Widgets) -> Option<(Dictionary, usize)> {
        let src = self.src;
        let fields = match form.get(b"Fields").and_then(|f| src.dereference(f)) {
            Ok((_, Object::Array(fields))) => fields.clone(),
            _ => return None,
        };
        let mut visited = HashSet::new();
        let mut count = 0;
        let mut top = Vec::new();
        for field in &fields {
            let Ok(id) = field.as_reference() else {
                continue;
            };
            top.extend(self.field(id, None, widgets, &mut visited, &mut count, 0));
        }
        if top.is_empty() {
            return None;
        }
        let mut out = Dictionary::new();
        for (key, value) in form.iter() {
            match key.as_slice() {
                // The field tree is rebuilt; an XFA form describes the old
                // one and would override it in readers that honour XFA.
                b"Fields" | b"XFA" => {}
                b"CO" => {
                    if let Ok((_, Object::Array(order))) = src.dereference(value) {
                        let order: Vec<Object> = order
                            .iter()
                            .filter_map(|field| field.as_reference().ok())
                            .filter_map(|id| self.fixed.get(&id).copied())
                            .map(Object::Reference)
                            .collect();
                        if !order.is_empty() {
                            out.set("CO", Object::Array(order));
                        }
                    }
                }
                _ => out.set(key.clone(), self.value(value, 0)),
            }
        }
        out.set(
            "Fields",
            Object::Array(top.into_iter().map(Object::Reference).collect()),
        );
        Some((out, count))
    }

    /// Rebuild the field `id` under `parent`, answering the output objects
    /// that take its place in the parent's /Kids (none when it is gone).
    fn field(
        &mut self,
        id: ObjectId,
        parent: Option<ObjectId>,
        widgets: &Widgets,
        visited: &mut HashSet<ObjectId>,
        count: &mut usize,
        depth: usize,
    ) -> Vec<ObjectId> {
        if depth >= MAX_TREE_DEPTH || !visited.insert(id) {
            return Vec::new();
        }
        let src = self.src;
        let Ok(dict) = src.get_dictionary(id) else {
            return Vec::new();
        };
        let is_field = dict.has(b"T") || dict.has(b"FT");
        if let Some(copies) = widgets.copies.get(&id) {
            // A widget on a kept page. If it is also the field (the merged
            // form), only its first copy is that field; a further copy of a
            // plain widget is simply another widget of the same field.
            let claimed: Vec<ObjectId> = if is_field {
                copies[..1].to_vec()
            } else {
                copies.clone()
            };
            for widget in &claimed {
                self.fixed.entry(id).or_insert(*widget);
                self.claimed.push((*widget, parent));
            }
            if is_field {
                *count += 1;
            }
            return claimed;
        }
        if is_type(dict, b"Subtype", &[b"Widget"]) {
            // A widget whose page is gone.
            return Vec::new();
        }
        let new_id = self.out.new_object_id();
        self.fixed.insert(id, new_id);
        let mut kids = Vec::new();
        let had_kids = match dict.get(b"Kids").and_then(|k| src.dereference(k)) {
            Ok((_, Object::Array(list))) => {
                for kid in list {
                    if let Ok(kid) = kid.as_reference() {
                        kids.extend(self.field(
                            kid,
                            Some(new_id),
                            widgets,
                            visited,
                            count,
                            depth + 1,
                        ));
                    }
                }
                true
            }
            _ => false,
        };
        if had_kids && kids.is_empty() {
            self.fixed.remove(&id);
            return Vec::new();
        }
        let mut out = Dictionary::new();
        for (key, value) in dict.iter() {
            if !matches!(key.as_slice(), b"Kids" | b"Parent") {
                out.set(key.clone(), self.value(value, 0));
            }
        }
        if had_kids {
            out.set(
                "Kids",
                Object::Array(kids.into_iter().map(Object::Reference).collect()),
            );
        }
        if let Some(parent) = parent {
            out.set("Parent", Object::Reference(parent));
        }
        self.out.set_object(new_id, out);
        *count += 1;
        vec![new_id]
    }
}

/// Number and link rebuilt outline entries under `parent`, answering the
/// first and last entry, the count of entries visible below `parent` when it
/// is open, and how many entries were written in all.
fn write_outline_items(
    out: &mut Document,
    items: Vec<Item>,
    parent: ObjectId,
) -> (ObjectId, ObjectId, i64, usize) {
    let ids: Vec<ObjectId> = items.iter().map(|_| out.new_object_id()).collect();
    let mut visible = 0;
    let mut written = 0;
    for (index, item) in items.into_iter().enumerate() {
        let id = ids[index];
        let mut dict = item.dict;
        dict.set("Parent", Object::Reference(parent));
        if index > 0 {
            dict.set("Prev", Object::Reference(ids[index - 1]));
        }
        if let Some(next) = ids.get(index + 1) {
            dict.set("Next", Object::Reference(*next));
        }
        visible += 1;
        written += 1;
        if !item.children.is_empty() {
            let (first, last, below, count) = write_outline_items(out, item.children, id);
            dict.set("First", Object::Reference(first));
            dict.set("Last", Object::Reference(last));
            dict.set(
                "Count",
                Object::Integer(if item.open { below } else { -below }),
            );
            if item.open {
                visible += below;
            }
            written += count;
        }
        out.set_object(id, dict);
    }
    (
        ids[0],
        *ids.last().expect("items are not empty"),
        visible,
        written,
    )
}

/// Every (name, destination) leaf of the name tree under `node`.
fn flatten_name_tree(
    doc: &Document,
    node: &Dictionary,
    entries: &mut Vec<(Vec<u8>, Object)>,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) {
    if depth >= MAX_TREE_DEPTH {
        return;
    }
    if let Ok((_, Object::Array(names))) = node.get(b"Names").and_then(|n| doc.dereference(n)) {
        for pair in names.chunks_exact(2) {
            let key = match doc.dereference(&pair[0]) {
                Ok((_, Object::String(key, _))) => key.clone(),
                Ok((_, Object::Name(key))) => key.clone(),
                _ => continue,
            };
            entries.push((key, pair[1].clone()));
        }
    }
    if let Ok((_, Object::Array(kids))) = node.get(b"Kids").and_then(|k| doc.dereference(k)) {
        for kid in kids {
            let Ok(id) = kid.as_reference() else { continue };
            if !visited.insert(id) {
                continue;
            }
            if let Ok(kid) = doc.get_dictionary(id) {
                flatten_name_tree(doc, kid, entries, visited, depth + 1);
            }
        }
    }
}

/// Write `entries` (sorted, unique) as a name tree, answering its root: one
/// flat leaf when small, otherwise a root over leaves with /Limits.
fn write_name_tree(out: &mut Document, entries: Vec<(Vec<u8>, Object)>) -> ObjectId {
    fn leaf(chunk: &[(Vec<u8>, Object)]) -> Vec<Object> {
        chunk
            .iter()
            .flat_map(|(key, value)| {
                [
                    Object::String(key.clone(), lopdf::StringFormat::Literal),
                    value.clone(),
                ]
            })
            .collect()
    }
    let mut root = Dictionary::new();
    if entries.len() <= NAMES_PER_LEAF {
        root.set("Names", Object::Array(leaf(&entries)));
        return out.add_object(root);
    }
    let mut kids = Vec::new();
    for chunk in entries.chunks(NAMES_PER_LEAF) {
        let mut node = Dictionary::new();
        let first = &chunk[0].0;
        let last = &chunk[chunk.len() - 1].0;
        node.set(
            "Limits",
            Object::Array(vec![
                Object::String(first.clone(), lopdf::StringFormat::Literal),
                Object::String(last.clone(), lopdf::StringFormat::Literal),
            ]),
        );
        node.set("Names", Object::Array(leaf(chunk)));
        kids.push(Object::Reference(out.add_object(node)));
    }
    root.set("Kids", Object::Array(kids));
    out.add_object(root)
}

/// One page's label: numbering style, prefix and number.
#[derive(Clone, Debug, PartialEq)]
struct Label {
    style: Option<Vec<u8>>,
    prefix: Option<Object>,
    number: i64,
}

/// The source's page labels re-expressed for `layout` as a /Nums array, or
/// `None` when the source has none.
///
/// Each kept page keeps the label it had; a new range starts wherever the
/// style or prefix changes or the numbers stop running on, so a deleted or
/// moved page never shifts the labels of the pages around it. Inserted blank
/// pages get an empty label.
fn remap_page_labels(
    doc: &Document,
    labels: &Dictionary,
    layout: &[Option<u32>],
) -> Option<Vec<Object>> {
    let mut ranges: Vec<(i64, Dictionary)> = Vec::new();
    flatten_number_tree(doc, labels, &mut ranges, &mut HashSet::new(), 0);
    if ranges.is_empty() {
        return None;
    }
    ranges.sort_by_key(|(start, _)| *start);

    let label_of = |page: i64| -> Option<Label> {
        let (start, range) = ranges.iter().rev().find(|(start, _)| *start <= page)?;
        let style = range
            .get(b"S")
            .and_then(Object::as_name)
            .ok()
            .map(<[u8]>::to_vec);
        let prefix = range
            .get(b"P")
            .ok()
            .and_then(|p| doc.dereference(p).ok())
            .map(|(_, p)| p.clone())
            .filter(|p| matches!(p, Object::String(..)));
        let first = range.get(b"St").and_then(Object::as_i64).unwrap_or(1);
        Some(Label {
            style,
            prefix,
            number: first + (page - start),
        })
    };

    let mut nums = Vec::new();
    let mut previous: Option<Option<Label>> = None;
    for (index, slot) in layout.iter().enumerate() {
        let label = slot.and_then(|page| label_of(page as i64));
        let continues = match (&previous, &label) {
            (Some(Some(prev)), Some(label)) => {
                prev.style == label.style
                    && prev.prefix == label.prefix
                    && (label.style.is_none() || prev.number + 1 == label.number)
            }
            (Some(None), None) => true,
            _ => false,
        };
        if !continues {
            let mut range = Dictionary::new();
            if let Some(label) = &label {
                if let Some(style) = &label.style {
                    range.set("S", Object::Name(style.clone()));
                    if label.number != 1 {
                        range.set("St", Object::Integer(label.number));
                    }
                }
                if let Some(prefix) = &label.prefix {
                    range.set("P", prefix.clone());
                }
            }
            nums.push(Object::Integer(index as i64));
            nums.push(Object::Dictionary(range));
        }
        previous = Some(label);
    }
    Some(nums)
}

/// Every (key, dictionary) leaf of the number tree under `node`.
fn flatten_number_tree(
    doc: &Document,
    node: &Dictionary,
    entries: &mut Vec<(i64, Dictionary)>,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) {
    if depth >= MAX_TREE_DEPTH {
        return;
    }
    if let Ok((_, Object::Array(nums))) = node.get(b"Nums").and_then(|n| doc.dereference(n)) {
        for pair in nums.chunks_exact(2) {
            let Ok(key) = pair[0].as_i64() else { continue };
            if let Ok((_, Object::Dictionary(range))) = doc.dereference(&pair[1]) {
                entries.push((key, range.clone()));
            }
        }
    }
    if let Ok((_, Object::Array(kids))) = node.get(b"Kids").and_then(|k| doc.dereference(k)) {
        for kid in kids {
            let Ok(id) = kid.as_reference() else { continue };
            if !visited.insert(id) {
                continue;
            }
            if let Ok(kid) = doc.get_dictionary(id) {
                flatten_number_tree(doc, kid, entries, visited, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdfium_init;
    use crate::engine::save::build_output;
    use crate::engine::types::{EditPlan, PlanPage};
    use lopdf::{dictionary, Stream, StringFormat};
    use pdfium_render::prelude::*;

    const XMP: &str = "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
        <x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF \
        xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
        rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>\
        <rdf:Alt><rdf:li xml:lang=\"x-default\">Carried Paper</rdf:li></rdf:Alt>\
        </dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>";

    fn s(text: &str) -> Object {
        Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
    }

    fn fit(page: ObjectId) -> Object {
        Object::Array(vec![page.into(), "Fit".into()])
    }

    fn goto(dest: Object) -> Object {
        Object::Dictionary(dictionary! { "S" => "GoTo", "D" => dest })
    }

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Object {
        Object::Array(vec![x0.into(), y0.into(), x1.into(), y1.into()])
    }

    /// A five-page paper with everything a restructure used to lose: a
    /// nested outline reaching pages explicitly, by name and through GoTo
    /// actions; named destinations in both the catalog dictionary and a
    /// two-leaf name tree; links using each; page labels (i, ii, 1, 2, 3);
    /// XMP metadata; and a form with a field on one page, a field with
    /// widgets on two pages and a field only on page 1.
    pub(crate) fn rich_fixture(xref_stream: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let page_ids: Vec<ObjectId> = (0..5).map(|_| doc.new_object_id()).collect();
        let p = |i: usize| page_ids[i];

        // Named destinations: a two-leaf name tree and the old dictionary.
        let sec_b_array = doc.add_object(fit(p(3)));
        let leaf_one = doc.add_object(dictionary! {
            "Limits" => vec![s("fig.1"), s("part.a")],
            "Names" => vec![
                s("fig.1"),
                Object::Dictionary(dictionary! {
                    "D" => vec![p(2).into(), "XYZ".into(), 0.into(), 300.into(), Object::Null],
                }),
                s("part.a"),
                fit(p(1)),
            ],
        });
        let leaf_two = doc.add_object(dictionary! {
            "Limits" => vec![s("sec.b"), s("sec.c")],
            "Names" => vec![s("sec.b"), sec_b_array.into(), s("sec.c"), fit(p(4))],
        });
        let dests_tree = doc.add_object(dictionary! {
            "Kids" => vec![leaf_one.into(), leaf_two.into()],
        });

        // Outline.
        let outlines = doc.new_object_id();
        let ids: Vec<ObjectId> = (0..7).map(|_| doc.new_object_id()).collect();
        let [front, part_a, a1, a2, part_b, b1, gone] = ids[..] else {
            unreachable!()
        };
        doc.set_object(
            outlines,
            dictionary! { "Type" => "Outlines", "First" => front, "Last" => gone, "Count" => 6 },
        );
        doc.set_object(
            front,
            dictionary! {
                "Title" => s("Front"), "Parent" => outlines, "Next" => part_a, "Dest" => fit(p(0)),
            },
        );
        doc.set_object(
            part_a,
            dictionary! {
                "Title" => s("Part A"), "Parent" => outlines, "Prev" => front, "Next" => part_b,
                "First" => a1, "Last" => a2, "Count" => 2, "Dest" => s("part.a"),
            },
        );
        doc.set_object(
            a1,
            dictionary! {
                "Title" => s("A.1"), "Parent" => part_a, "Next" => a2,
                "A" => goto(Object::Array(vec![
                    p(1).into(), "XYZ".into(), 0.into(), 400.into(), 0.into(),
                ])),
            },
        );
        doc.set_object(
            a2,
            dictionary! {
                "Title" => s("A.2"), "Parent" => part_a, "Prev" => a1, "Dest" => fit(p(2)),
            },
        );
        doc.set_object(
            part_b,
            dictionary! {
                "Title" => s("Part B"), "Parent" => outlines, "Prev" => part_a, "Next" => gone,
                "First" => b1, "Last" => b1, "Count" => -1, "A" => goto(s("sec.b")),
            },
        );
        doc.set_object(
            b1,
            dictionary! { "Title" => s("B.1"), "Parent" => part_b, "Dest" => fit(p(4)) },
        );
        doc.set_object(
            gone,
            dictionary! {
                "Title" => s("Gone"), "Parent" => outlines, "Prev" => part_b, "Dest" => fit(p(1)),
            },
        );

        // Links on page 0, top to bottom.
        let targets: [(&str, Object); 5] = [
            ("Dest", s("fig.1")),
            ("A", goto(s("sec.b"))),
            ("Dest", Object::Name(b"old.p2".to_vec())),
            ("Dest", fit(p(4))),
            ("Dest", s("part.a")),
        ];
        let links: Vec<Object> = targets
            .into_iter()
            .enumerate()
            .map(|(i, (key, target))| {
                let y = 300 - 30 * i as i64;
                let mut dict = dictionary! {
                    "Type" => "Annot", "Subtype" => "Link",
                    "Border" => vec![0.into(), 0.into(), 0.into()],
                    "Rect" => rect(40, y, 200, y + 20),
                };
                dict.set(key, target);
                doc.add_object(dict).into()
            })
            .collect();

        // Form: "name" on page 2, "choice" with widgets on pages 1 and 3,
        // "lost" only on page 1.
        let name = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => s("name"),
            "V" => s("Ada"), "F" => 4, "Rect" => rect(100, 240, 250, 258),
            "DA" => s("/Helv 10 Tf 0 g"), "P" => p(2),
        });
        let choice = doc.new_object_id();
        let choice_one = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => choice, "F" => 4,
            "Rect" => rect(100, 200, 250, 218), "P" => p(1),
        });
        let choice_three = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => choice, "F" => 4,
            "Rect" => rect(100, 200, 250, 218), "P" => p(3),
        });
        doc.set_object(
            choice,
            dictionary! {
                "FT" => "Tx", "T" => s("choice"), "V" => s("shared"),
                "Kids" => vec![choice_one.into(), choice_three.into()],
            },
        );
        let lost = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => s("lost"),
            "F" => 4, "Rect" => rect(100, 100, 250, 118), "P" => p(1),
        });

        for (i, &id) in page_ids.iter().enumerate() {
            let content = format!("BT /F1 18 Tf 40 360 Td (PAGE-{i}) Tj ET");
            let contents = doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
            let mut page = dictionary! {
                "Type" => "Page", "Parent" => pages_id, "MediaBox" => rect(0, 0, 300, 400),
                "Contents" => contents,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font, "Helv" => font } },
            };
            let annots: Vec<Object> = match i {
                0 => links.clone(),
                1 => vec![choice_one.into(), lost.into()],
                2 => vec![name.into()],
                3 => vec![choice_three.into()],
                _ => Vec::new(),
            };
            if !annots.is_empty() {
                page.set("Annots", annots);
            }
            doc.set_object(id, page);
        }
        doc.set_object(
            pages_id,
            dictionary! {
                "Type" => "Pages", "Count" => 5,
                "Kids" => page_ids.iter().map(|&id| id.into()).collect::<Vec<Object>>(),
            },
        );

        let metadata = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            XMP.as_bytes().to_vec(),
        ));
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id, "Outlines" => outlines,
            "PageMode" => "UseOutlines",
            "Names" => dictionary! { "Dests" => dests_tree },
            "Dests" => dictionary! { "old.p2" => fit(p(2)) },
            "PageLabels" => dictionary! { "Nums" => vec![
                0.into(), Object::Dictionary(dictionary! { "S" => "r" }),
                2.into(), Object::Dictionary(dictionary! { "S" => "D" }),
            ] },
            "Metadata" => metadata,
            "AcroForm" => dictionary! {
                "Fields" => vec![name.into(), choice.into(), lost.into()],
                "DA" => s("/Helv 0 Tf 0 g"),
                "DR" => dictionary! { "Font" => dictionary! { "Helv" => font } },
            },
        });
        let info = doc.add_object(dictionary! { "Title" => s("Carried Paper") });
        doc.trailer.set("Root", catalog);
        doc.trailer.set("Info", info);
        doc.reference_table.cross_reference_type = if xref_stream {
            lopdf::xref::XrefType::CrossReferenceStream
        } else {
            lopdf::xref::XrefType::CrossReferenceTable
        };
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("write fixture");
        bytes
    }

    fn page(src_index: Option<u32>) -> PlanPage {
        PlanPage {
            src_index,
            width_pt: 300.0,
            height_pt: 400.0,
            rotation: 0,
            annots: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }
    }

    fn restructure(
        pdfium: &Pdfium,
        source: &[u8],
        layout: &[Option<u32>],
        dir: &Path,
    ) -> std::path::PathBuf {
        let src = dir.join("source.pdf");
        let out = dir.join(format!("out-{}.pdf", layout.len()));
        std::fs::write(&src, source).expect("write source");
        let plan = EditPlan {
            pages: layout.iter().map(|slot| page(*slot)).collect(),
            form: Vec::new(),
            form_buttons: Vec::new(),
        };
        build_output(pdfium, src.to_str().unwrap(), None, &plan, &out).expect("save");
        if let Some(keep) = std::env::var_os("SPEEDYF_CARRY_KEEP") {
            let keep = Path::new(&keep);
            let _ = std::fs::create_dir_all(keep);
            let n = std::fs::read_dir(keep).map(|d| d.count()).unwrap_or(0);
            let _ = std::fs::copy(&out, keep.join(format!("fixture-{n}.pdf")));
            let _ = std::fs::copy(&src, keep.join(format!("fixture-{n}-source.pdf")));
        }
        out
    }

    /// (depth, title, target page) for every outline entry, depth first.
    fn outline(document: &PdfDocument<'_>) -> Vec<(usize, String, Option<u16>)> {
        fn walk(
            first: Option<PdfBookmark<'_>>,
            depth: usize,
            out: &mut Vec<(usize, String, Option<u16>)>,
        ) {
            let mut next = first;
            while let Some(bookmark) = next {
                let page = bookmark.destination().and_then(|d| d.page_index().ok());
                out.push((depth, bookmark.title().unwrap_or_default(), page));
                walk(bookmark.first_child(), depth + 1, out);
                next = bookmark.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(document.bookmarks().root(), 0, &mut out);
        out
    }

    /// The page each link on page `index` leads to, top to bottom.
    fn link_targets(document: &PdfDocument<'_>, index: u16) -> Vec<Option<u16>> {
        let page = document.pages().get(index).expect("page");
        let mut links: Vec<(f32, Option<u16>)> = page
            .links()
            .iter()
            .map(|link| {
                let top = link.rect().map(|r| r.top().value).unwrap_or(0.0);
                (top, link.destination().and_then(|d| d.page_index().ok()))
            })
            .collect();
        links.sort_by(|a, b| b.0.total_cmp(&a.0));
        links.into_iter().map(|(_, target)| target).collect()
    }

    fn labels(document: &PdfDocument<'_>) -> Vec<String> {
        document
            .pages()
            .iter()
            .map(|page| page.label().unwrap_or_default().to_string())
            .collect()
    }

    fn page_texts(document: &PdfDocument<'_>) -> Vec<String> {
        document
            .pages()
            .iter()
            .map(|page| page.text().expect("text").all().trim().to_string())
            .collect()
    }

    fn field_names(document: &PdfDocument<'_>) -> Vec<String> {
        let mut names: Vec<String> = document
            .form()
            .map(|form| form.field_values(document.pages()).into_keys().collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    fn has_xmp(path: &Path) -> bool {
        let doc = Document::load(path).expect("lopdf reads the output");
        let catalog = doc.catalog().expect("catalog");
        let Ok(id) = catalog.get(b"Metadata").and_then(Object::as_reference) else {
            return false;
        };
        doc.get_object(id)
            .and_then(Object::as_stream)
            .is_ok_and(|stream| String::from_utf8_lossy(&stream.content).contains("Carried Paper"))
    }

    fn entry(depth: usize, title: &str, page: Option<u16>) -> (usize, String, Option<u16>) {
        (depth, title.to_string(), page)
    }

    #[test]
    fn deleting_a_page_keeps_outline_names_labels_metadata_and_form() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        for xref_stream in [false, true] {
            let dir = tempfile::tempdir().expect("temp dir");
            let out = restructure(
                &pdfium,
                &rich_fixture(xref_stream),
                &[Some(0), Some(2), Some(3), Some(4)],
                dir.path(),
            );
            let document = pdfium.load_pdf_from_file(&out, None).expect("reopen");
            assert_eq!(
                page_texts(&document),
                ["PAGE-0", "PAGE-2", "PAGE-3", "PAGE-4"]
            );
            assert_eq!(
                outline(&document),
                vec![
                    entry(0, "Front", Some(0)),
                    // Its page is gone but its child's is not: kept, with no
                    // destination of its own. A.1 and Gone are dropped.
                    entry(0, "Part A", None),
                    entry(1, "A.2", Some(1)),
                    entry(0, "Part B", Some(2)),
                    entry(1, "B.1", Some(3)),
                ]
            );
            // fig.1, sec.b through an action, old.p2 from the catalog
            // dictionary, an explicit page, and part.a, whose page is gone.
            assert_eq!(
                link_targets(&document, 0),
                vec![Some(1), Some(2), Some(1), Some(3), None]
            );
            assert_eq!(labels(&document), ["i", "1", "2", "3"]);
            assert!(has_xmp(&out));
            assert_eq!(field_names(&document), ["choice", "name"]);
        }
    }

    #[test]
    fn reordering_duplicating_and_inserting_remap_to_the_new_pages() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().expect("temp dir");
        let source = rich_fixture(true);

        // Reversed.
        let out = restructure(
            &pdfium,
            &source,
            &[Some(4), Some(3), Some(2), Some(1), Some(0)],
            dir.path(),
        );
        let document = pdfium.load_pdf_from_file(&out, None).expect("reopen");
        assert_eq!(
            outline(&document),
            vec![
                entry(0, "Front", Some(4)),
                entry(0, "Part A", Some(3)),
                entry(1, "A.1", Some(3)),
                entry(1, "A.2", Some(2)),
                entry(0, "Part B", Some(1)),
                entry(1, "B.1", Some(0)),
                entry(0, "Gone", Some(3)),
            ]
        );
        assert_eq!(
            link_targets(&document, 4),
            vec![Some(2), Some(1), Some(2), Some(0), Some(3)]
        );
        assert_eq!(labels(&document), ["3", "2", "1", "ii", "i"]);
        assert_eq!(field_names(&document), ["choice", "lost", "name"]);
        drop(document);

        // Page 2 duplicated: everything reaches the first copy.
        let out = restructure(
            &pdfium,
            &source,
            &[Some(0), Some(1), Some(2), Some(2), Some(3), Some(4)],
            dir.path(),
        );
        let document = pdfium.load_pdf_from_file(&out, None).expect("reopen");
        assert_eq!(
            link_targets(&document, 0),
            vec![Some(2), Some(4), Some(2), Some(5), Some(1)]
        );
        assert_eq!(labels(&document), ["i", "ii", "1", "1", "2", "3"]);
        assert_eq!(outline(&document)[3], entry(1, "A.2", Some(2)));
        assert_eq!(field_names(&document), ["choice", "lost", "name"]);
        drop(document);

        // Blank pages inserted after the first and at the end.
        let out = restructure(
            &pdfium,
            &source,
            &[Some(0), None, Some(1), Some(2), Some(3), Some(4), None],
            dir.path(),
        );
        let document = pdfium.load_pdf_from_file(&out, None).expect("reopen");
        assert_eq!(
            link_targets(&document, 0),
            vec![Some(3), Some(4), Some(3), Some(5), Some(2)]
        );
        assert_eq!(labels(&document), ["i", "", "ii", "1", "2", "3", ""]);
        assert_eq!(outline(&document)[0], entry(0, "Front", Some(0)));
        assert!(has_xmp(&out));
    }

    #[test]
    fn a_form_on_a_restructured_document_still_takes_edits() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().expect("temp dir");
        let src = dir.path().join("source.pdf");
        let out = dir.path().join("out.pdf");
        std::fs::write(&src, rich_fixture(false)).expect("write source");
        let plan = EditPlan {
            pages: [Some(3), Some(2), Some(0)]
                .iter()
                .map(|slot| page(*slot))
                .collect(),
            form: vec![("name".into(), "Grace".into())],
            form_buttons: Vec::new(),
        };
        build_output(&pdfium, src.to_str().unwrap(), None, &plan, &out).expect("save");
        let document = pdfium.load_pdf_from_file(&out, None).expect("reopen");
        let values = document
            .form()
            .expect("still a form")
            .field_values(document.pages());
        assert_eq!(values.get("name"), Some(&Some("Grace".to_string())));
        assert_eq!(values.get("choice"), Some(&Some("shared".to_string())));
        assert!(!values.contains_key("lost"));
    }

    #[test]
    fn an_unreadable_output_is_refused_not_mangled() {
        let dir = tempfile::tempdir().expect("temp dir");
        let src = dir.path().join("source.pdf");
        std::fs::write(&src, rich_fixture(false)).expect("write");
        let result = carry_catalog(&src, None, &[Some(0)], b"%PDF-1.7\nnot a pdf");
        assert!(result.is_err());
        let result = carry_catalog(&src, None, &[Some(0), Some(1)], &rich_fixture(false));
        assert!(result.is_err(), "a page count mismatch must be refused");
    }

    #[test]
    fn a_source_without_catalog_structure_needs_no_update() {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "MediaBox" => rect(0, 0, 10, 10),
        });
        doc.set_object(
            pages,
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("write");
        let dir = tempfile::tempdir().expect("temp dir");
        let src = dir.path().join("plain.pdf");
        std::fs::write(&src, &bytes).expect("write");
        assert!(carry_catalog(&src, None, &[Some(0)], &bytes)
            .expect("carry")
            .is_none());
    }
}

/// Restructure real papers and count what still resolves, before and after.
///
/// `SPEEDYF_CARRY_EVAL` lists PDFs (separated by `:`); outputs go to
/// `SPEEDYF_CARRY_OUT` (default: a temp dir). Each file gets two plans -
/// page 2 deleted, and pages 3 and 4 swapped - saved once with the carry pass
/// and once without it (the old behaviour). An outline entry or link counts
/// as correct when it lands on a copy of the page it reached in the source.
#[cfg(test)]
mod eval {
    use crate::engine::pdfium_init;
    use crate::engine::save::{build_output, SKIP_CARRY};
    use crate::engine::types::{EditPlan, PlanPage};
    use pdfium_render::prelude::*;
    use std::path::{Path, PathBuf};

    #[derive(Default, Debug)]
    struct Counts {
        outline: usize,
        outline_ok: usize,
        links: usize,
        links_ok: usize,
        labels: bool,
        form_fields: usize,
    }

    fn walk(first: Option<PdfBookmark<'_>>, out: &mut Vec<(String, Option<u16>)>) {
        let mut next = first;
        let mut guard = 0;
        while let Some(bookmark) = next {
            guard += 1;
            if guard > 100_000 {
                return;
            }
            let page = bookmark.destination().and_then(|d| d.page_index().ok());
            out.push((bookmark.title().unwrap_or_default(), page));
            walk(bookmark.first_child(), out);
            next = bookmark.next_sibling();
        }
    }

    /// Internal links per page, in rect order, as their target page.
    fn page_links(page: &PdfPage<'_>) -> Vec<Option<u16>> {
        let mut links: Vec<((i64, i64), Option<u16>)> = page
            .links()
            .iter()
            .filter(|link| {
                // External links are not what this measures.
                !link.action().is_some_and(|a| {
                    a.as_uri_action().is_some() || a.as_remote_destination_action().is_some()
                })
            })
            .map(|link| {
                let key = link
                    .rect()
                    .map(|r| {
                        (
                            (-r.top().value * 10.0) as i64,
                            (r.left().value * 10.0) as i64,
                        )
                    })
                    .unwrap_or((0, 0));
                (key, link.destination().and_then(|d| d.page_index().ok()))
            })
            .collect();
        links.sort_by_key(|(key, _)| *key);
        links.into_iter().map(|(_, target)| target).collect()
    }

    /// Counts for `out` against `src`, where output page j shows source page
    /// `layout[j]`.
    fn count(pdfium: &Pdfium, src: &Path, out: &Path, layout: &[Option<u32>]) -> Counts {
        let source = pdfium.load_pdf_from_file(src, None).expect("source");
        let output = pdfium.load_pdf_from_file(out, None).expect("output");
        let back = |page: Option<u16>| page.and_then(|p| layout.get(p as usize).copied().flatten());
        let mut counts = Counts::default();

        let mut src_outline = Vec::new();
        walk(source.bookmarks().root(), &mut src_outline);
        let mut out_outline = Vec::new();
        walk(output.bookmarks().root(), &mut out_outline);
        counts.outline = out_outline.len();
        // Match by title, in order: each source entry whose page survived
        // should appear and land on that page.
        let mut cursor = 0;
        for (title, page) in &out_outline {
            let Some(at) = src_outline[cursor..].iter().position(|(t, _)| t == title) else {
                continue;
            };
            let expected = src_outline[cursor + at].1;
            cursor += at + 1;
            if page.is_some() && back(*page) == expected.map(u32::from) {
                counts.outline_ok += 1;
            }
        }

        for (j, slot) in layout.iter().enumerate() {
            let Some(s) = slot else { continue };
            let src_links = page_links(&source.pages().get(*s as u16).expect("page"));
            let out_links = page_links(&output.pages().get(j as u16).expect("page"));
            counts.links += out_links.len();
            for (want, got) in src_links.iter().zip(&out_links) {
                if got.is_some() && back(*got) == want.map(u32::from) {
                    counts.links_ok += 1;
                }
            }
        }
        counts.labels = output.pages().iter().any(|page| page.label().is_some());
        counts.form_fields = output
            .form()
            .map(|form| form.field_values(output.pages()).len())
            .unwrap_or(0);
        counts
    }

    fn identity_counts(pdfium: &Pdfium, src: &Path, pages: u16) -> Counts {
        let layout: Vec<Option<u32>> = (0..pages as u32).map(Some).collect();
        count(pdfium, src, src, &layout)
    }

    fn save(pdfium: &Pdfium, src: &Path, out: &Path, layout: &[Option<u32>], carry: bool) -> f64 {
        let plan = EditPlan {
            pages: layout
                .iter()
                .map(|slot| PlanPage {
                    src_index: *slot,
                    width_pt: 612.0,
                    height_pt: 792.0,
                    rotation: 0,
                    annots: Vec::new(),
                    texts: Vec::new(),
                    images: Vec::new(),
                    drop_src_annots: Vec::new(),
                })
                .collect(),
            form: Vec::new(),
            form_buttons: Vec::new(),
        };
        SKIP_CARRY.with(|skip| skip.set(!carry));
        let started = std::time::Instant::now();
        build_output(pdfium, src.to_str().unwrap(), None, &plan, out).expect("save");
        SKIP_CARRY.with(|skip| skip.set(false));
        started.elapsed().as_secs_f64()
    }

    #[test]
    #[ignore = "evaluation over local papers; set SPEEDYF_CARRY_EVAL"]
    fn restructure_real_papers() {
        let Ok(list) = std::env::var("SPEEDYF_CARRY_EVAL") else {
            return;
        };
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let temp = tempfile::tempdir().expect("temp dir");
        let out_dir = std::env::var_os("SPEEDYF_CARRY_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| temp.path().to_path_buf());
        std::fs::create_dir_all(&out_dir).expect("out dir");
        for src in list.split(':').filter(|s| !s.is_empty()) {
            let src = Path::new(src);
            let pages = pdfium
                .load_pdf_from_file(src, None)
                .expect("open")
                .pages()
                .len();
            let stem = src.file_stem().unwrap().to_string_lossy().to_string();
            println!("== {stem} ({pages} pages)");
            println!("   source: {:?}", identity_counts(&pdfium, src, pages));
            let delete: Vec<Option<u32>> =
                (0..pages as u32).filter(|&p| p != 1).map(Some).collect();
            let mut swap: Vec<Option<u32>> = (0..pages as u32).map(Some).collect();
            swap.swap(2, 3);
            for (name, layout) in [("delete-p2", delete), ("swap-p3-p4", swap)] {
                for carry in [false, true] {
                    let out = out_dir.join(format!(
                        "{stem}-{name}-{}.pdf",
                        if carry { "carried" } else { "old" }
                    ));
                    let secs = save(&pdfium, src, &out, &layout, carry);
                    let counts = count(&pdfium, src, &out, &layout);
                    println!(
                        "   {name} {}: {counts:?} in {secs:.2}s",
                        if carry { "carried" } else { "old    " }
                    );
                }
            }
        }
    }
}

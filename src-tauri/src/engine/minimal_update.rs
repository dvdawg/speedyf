//! Keeping the update appended to a signed document down to what changed.
//!
//! PDFium's incremental save (`FPDF_INCREMENTAL`) writes back every object it
//! loaded, not only the ones that changed: the catalog, the page tree, the
//! page, its content stream (re-compressed), a signature's widget and its
//! appearance. The bytes before the update stay as they were, so every
//! signature still verifies, but a checker that asks what the update did
//! (Acrobat, pyHanko) sees the page content and a signed appearance replaced
//! and reports the document as modified, where a form fill or a comment is
//! what actually happened. A certified document reads as broken.
//!
//! `minimal_update` takes the source and the file the save produced (the
//! source plus one or more appended updates) and rewrites everything after
//! the source as a single update holding only objects that say something
//! new: the ones that existed and now differ, and the new objects those
//! reach. An object that was only re-serialised - re-compressed, written with
//! hex strings for literal ones, keys reordered - is left out, so the reader
//! keeps finding the signed revision's own bytes for it.

use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Rewrite the updates appended to `source` in the file at `path` as one
/// minimal update (see the module docs). Leaves the file alone when it does
/// not extend `source`, or when either cannot be read.
pub(crate) fn minimize_file(source: &[u8], path: &Path) -> std::io::Result<()> {
    let output = std::fs::read(path)?;
    match minimal_update(source, &output) {
        Ok(Some(bytes)) if !reads_back(&bytes, &output) => {
            log::warn!("the minimised update did not read back; keeping PDFium's");
        }
        Ok(Some(bytes)) => {
            let mut file = std::fs::File::create(path)?;
            std::io::Write::write_all(&mut file, &bytes)?;
            file.sync_all()?;
        }
        Ok(None) => {}
        // The file as PDFium wrote it is still correct, and every signature
        // in it still verifies; only the report of what changed is coarser.
        Err(error) => log::warn!("could not minimise the appended update: {error}"),
    }
    Ok(())
}

/// Whether `minimal` parses and has the pages, and the same objects, that
/// `output` has (annotations lifted out of /Annots are new objects).
fn reads_back(minimal: &[u8], output: &[u8]) -> bool {
    let (Ok(a), Ok(b)) = (Document::load_mem(minimal), Document::load_mem(output)) else {
        return false;
    };
    a.get_pages().len() == b.get_pages().len()
        && b.objects
            .iter()
            .all(|(id, object)| match a.objects.get(id) {
                // PDFium's unused scratch objects are dropped.
                None => true,
                Some(_) if is_bookkeeping(object) => true,
                Some(Object::Dictionary(page)) if page.has(b"Annots") => true,
                Some(mine) => equivalent(mine, object),
            })
}

/// The source followed by one update holding only what `output` changed,
/// or `None` when `output` is not `source` with updates appended.
pub(crate) fn minimal_update(source: &[u8], output: &[u8]) -> Result<Option<Vec<u8>>, String> {
    if output.len() <= source.len() || !output.starts_with(source) {
        return Ok(None);
    }
    let old = Document::load_mem(source).map_err(|e| format!("source: {e}"))?;
    let new = Document::load_mem(output).map_err(|e| format!("output: {e}"))?;
    if old.is_encrypted() || new.is_encrypted() || old.trailer.has(b"Encrypt") {
        return Ok(None);
    }
    // An update chains to the source's own cross-reference section; one
    // lopdf had to rebuild by scanning has no offset to chain to.
    if old.xref_start == 0 {
        return Ok(None);
    }

    // Objects that existed and now say something else.
    let mut keep: BTreeMap<ObjectId, &Object> = BTreeMap::new();
    let mut fresh: HashSet<ObjectId> = HashSet::new();
    for (id, after) in &new.objects {
        if is_bookkeeping(after) {
            continue;
        }
        match old.objects.get(id) {
            Some(before) if equivalent(before, after) => {}
            Some(_) => {
                keep.insert(*id, after);
            }
            None => {
                fresh.insert(*id);
            }
        }
    }

    // The trailer's own references (a new /Info, say) and everything new a
    // kept object reaches. New objects nothing reaches are PDFium's scratch
    // (a resource dictionary it built and then did not use) and are dropped.
    let trailer = update_trailer(&old, &new);
    let mut stack: Vec<ObjectId> = Vec::new();
    let push_refs = |object: &Object, stack: &mut Vec<ObjectId>| {
        references(object, &mut |id| {
            if fresh.contains(&id) {
                stack.push(id);
            }
        })
    };
    for value in trailer.iter().map(|(_, v)| v) {
        push_refs(value, &mut stack);
    }
    for object in keep.values() {
        push_refs(object, &mut stack);
    }
    while let Some(id) = stack.pop() {
        if keep.contains_key(&id) {
            continue;
        }
        if let Some(object) = new.objects.get(&id) {
            keep.insert(id, object);
            push_refs(object, &mut stack);
        }
    }

    if keep.is_empty() {
        // Nothing changed at all: the source is the answer.
        return Ok(Some(source.to_vec()));
    }

    let declared = |d: &Dictionary| {
        d.get(b"Size")
            .and_then(Object::as_i64)
            .unwrap_or(0)
            .clamp(0, u32::MAX as i64) as u32
    };
    let mut ceiling = new
        .max_id
        .max(declared(&new.trailer).saturating_sub(1))
        .max(declared(&old.trailer).saturating_sub(1))
        .max(old.max_id);
    let mut keep: BTreeMap<ObjectId, Object> =
        keep.into_iter().map(|(id, o)| (id, o.clone())).collect();
    lift_direct_annotations(&old, &new, &mut keep, &mut ceiling);

    let mut inc = IncrementalDocument::create_from(source.to_vec(), old);
    inc.new_document.max_id = ceiling;
    inc.new_document.trailer = trailer;
    inc.new_document.objects.extend(keep);
    let mut out = Vec::with_capacity(source.len() + 4096);
    inc.save_to(&mut out).map_err(|e| format!("write: {e}"))?;
    out.push(b'\n');
    Ok(Some(out))
}

/// Give each annotation PDFium wrote inline in a changed page's /Annots an
/// object of its own.
///
/// PDFium adds an annotation as a direct dictionary inside /Annots. The
/// standard asks for indirect references there, and a signature checker
/// only recognises a comment as one when it is its own object (pyHanko
/// rejects the inline form as an unexplained change to the page). Lifting
/// them out changes nothing a reader shows.
fn lift_direct_annotations(
    old: &Document,
    new: &Document,
    keep: &mut BTreeMap<ObjectId, Object>,
    ceiling: &mut u32,
) {
    // Kept pages with a direct /Annots array, and kept arrays that are some
    // page's /Annots.
    let mut owners: Vec<ObjectId> = Vec::new();
    for (id, object) in &new.objects {
        let Object::Dictionary(dict) = object else {
            continue;
        };
        if !matches!(dict.get(b"Type"), Ok(Object::Name(n)) if n == b"Page") {
            continue;
        }
        match dict.get(b"Annots") {
            Ok(Object::Array(_)) if keep.contains_key(id) => owners.push(*id),
            Ok(Object::Reference(list)) if keep.contains_key(list) => owners.push(*list),
            _ => {}
        }
    }
    // Inline annotations the signed revision already had stay as they were.
    let before = |owner: ObjectId| -> Vec<Object> {
        let list = match old.objects.get(&owner) {
            Some(Object::Dictionary(page)) => match page.get(b"Annots") {
                Ok(Object::Array(list)) => Some(list),
                Ok(Object::Reference(id)) => old.get_object(*id).and_then(Object::as_array).ok(),
                _ => None,
            },
            Some(Object::Array(list)) => Some(list),
            _ => None,
        };
        list.map(|l| {
            l.iter()
                .filter(|o| matches!(o, Object::Dictionary(_)))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    };
    let mut lifted: Vec<(ObjectId, Object)> = Vec::new();
    for owner in owners {
        let existing = before(owner);
        let list = match keep.get_mut(&owner) {
            Some(Object::Dictionary(page)) => match page.get_mut(b"Annots") {
                Ok(Object::Array(list)) => list,
                _ => continue,
            },
            Some(Object::Array(list)) => list,
            _ => continue,
        };
        for entry in list.iter_mut() {
            if matches!(entry, Object::Dictionary(_))
                && !existing.iter().any(|e| equivalent(e, entry))
            {
                *ceiling += 1;
                let id = (*ceiling, 0);
                lifted.push((id, std::mem::replace(entry, Object::Reference(id))));
            }
        }
    }
    keep.extend(lifted);
}

/// The trailer for the update: chained to the source's cross-reference
/// section, with the final file's /Root, /Info and /ID.
fn update_trailer(old: &Document, new: &Document) -> Dictionary {
    let mut trailer = Dictionary::new();
    for key in [b"Root".as_slice(), b"Info", b"ID"] {
        if let Ok(value) = new.trailer.get(key) {
            trailer.set(key, value.clone());
        } else if let Ok(value) = old.trailer.get(key) {
            trailer.set(key, value.clone());
        }
    }
    trailer.set("Prev", Object::Integer(old.xref_start as i64));
    trailer
}

/// Every indirect reference inside `object`.
fn references(object: &Object, found: &mut impl FnMut(ObjectId)) {
    match object {
        Object::Reference(id) => found(*id),
        Object::Array(items) => items.iter().for_each(|item| references(item, found)),
        Object::Dictionary(dict) => dict.iter().for_each(|(_, v)| references(v, found)),
        Object::Stream(stream) => stream.dict.iter().for_each(|(_, v)| references(v, found)),
        _ => {}
    }
}

/// Cross-reference and object streams are how a file is laid out, not part
/// of the document.
fn is_bookkeeping(object: &Object) -> bool {
    match object {
        Object::Stream(stream) => matches!(
            stream.dict.get(b"Type"),
            Ok(Object::Name(n)) if n == b"XRef" || n == b"ObjStm"
        ),
        _ => false,
    }
}

/// Whether two versions of an object say the same thing, however each was
/// written: strings compare by their bytes (hex or literal), numbers by
/// value (`1` and `1.0`), dictionaries regardless of key order, and streams
/// by what they decode to (a saver may re-compress a stream it did not
/// change).
pub(crate) fn equivalent(a: &Object, b: &Object) -> bool {
    use Object::*;
    match (a, b) {
        (String(x, _), String(y, _)) => x == y,
        (Integer(x), Integer(y)) => x == y,
        (Integer(i), Real(r)) | (Real(r), Integer(i)) => close(*i as f64, f64::from(*r)),
        (Real(x), Real(y)) => close(f64::from(*x), f64::from(*y)),
        (Array(x), Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equivalent(p, q))
        }
        (Dictionary(x), Dictionary(y)) => same_dict(x, y, &[]),
        (Stream(x), Stream(y)) => {
            const CODING: &[&[u8]] = &[b"Length", b"Filter", b"DecodeParms"];
            if x.content == y.content && same_dict(&x.dict, &y.dict, &[b"Length"]) {
                return true;
            }
            same_dict(&x.dict, &y.dict, CODING)
                && matches!((decoded(x), decoded(y)), (Some(p), Some(q)) if p == q)
        }
        _ => a == b,
    }
}

fn close(x: f64, y: f64) -> bool {
    (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.0)
}

fn same_dict(x: &Dictionary, y: &Dictionary, skip: &[&[u8]]) -> bool {
    let keys = |d: &Dictionary| -> HashSet<Vec<u8>> {
        d.iter()
            .map(|(k, _)| k.clone())
            .filter(|k| !skip.contains(&k.as_slice()))
            .collect()
    };
    let kx = keys(x);
    kx == keys(y)
        && kx.iter().all(|k| match (x.get(k), y.get(k)) {
            (Ok(p), Ok(q)) => equivalent(p, q),
            _ => false,
        })
}

/// A stream's data with its filters undone and trailing white space
/// dropped (savers differ on the final newline), or `None` when lopdf cannot
/// undo them (the caller then treats a re-encoded stream as changed).
fn decoded(stream: &lopdf::Stream) -> Option<Vec<u8>> {
    let mut bytes = if stream.dict.has(b"Filter") {
        stream.decompressed_content().ok()?
    } else {
        stream.content.clone()
    };
    let end = bytes
        .iter()
        .rposition(|ch| !ch.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    bytes.truncate(end);
    Some(bytes)
}

#[cfg(test)]
#[path = "minimal_update_tests.rs"]
mod tests;

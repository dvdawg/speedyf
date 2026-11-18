//! Formatted appearances over raw values.
//!
//! A field with a format script keeps what was typed in /V (`1234.5`) and
//! shows it formatted on the page (`$1,234.50`). PDFium's form-fill
//! environment only ever draws /V, so the save does it in three steps, each
//! an incremental update:
//!
//! 1. `set_text_colors`: a number format whose negative style is red
//!    changes the field's text colour, as Acrobat's AFNumber_Format does
//!    (it sets `textColor`, which lives in /DA). Done before PDFium draws,
//!    so the new appearance is drawn in that colour.
//! 2. PDFium types the *formatted* text, and generates the appearance from
//!    the field's own /DA - font, size, alignment, comb, multiline - exactly
//!    as for any other value.
//! 3. `restore_raw_values` puts the raw value back in /V, leaving the
//!    appearance PDFium drew.
//!
//! What another reader shows is the appearance; what a script or a data
//! export reads is /V. That is the split Acrobat itself saves.

use lopdf::{Dictionary, IncrementalDocument, Object, ObjectId, StringFormat};
use std::collections::HashMap;

/// How deep the field tree is followed before it is taken to be a loop.
const MAX_DEPTH: usize = 32;

/// Field dictionaries by fully qualified name, with each one's widget kids.
struct FieldIndex {
    fields: HashMap<String, (ObjectId, Vec<ObjectId>)>,
}

fn index_fields(doc: &lopdf::Document) -> Option<FieldIndex> {
    let root = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok()?;
    let catalog = doc.get_dictionary(root).ok()?;
    let acroform = match catalog.get(b"AcroForm").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(d) => d,
        _ => return None,
    };
    let top = match acroform.get(b"Fields").ok()? {
        Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?,
        Object::Array(a) => a,
        _ => return None,
    };
    let mut index = FieldIndex {
        fields: HashMap::new(),
    };
    for entry in top {
        if let Ok(id) = entry.as_reference() {
            walk(doc, id, "", &mut index, 0);
        }
    }
    Some(index)
}

fn walk(doc: &lopdf::Document, id: ObjectId, parent: &str, index: &mut FieldIndex, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(dict) = doc.get_dictionary(id) else {
        return;
    };
    let name = match dict.get(b"T").ok().and_then(|t| decode(doc, t)) {
        Some(t) if parent.is_empty() => t,
        Some(t) => format!("{parent}.{t}"),
        None => parent.to_string(),
    };
    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .ok()
        .and_then(|k| match k {
            Object::Reference(r) => doc.get_object(*r).ok().and_then(|o| o.as_array().ok()),
            Object::Array(a) => Some(a),
            _ => None,
        })
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();
    let has_t = dict.has(b"T");
    if has_t {
        // A field: its widgets are the kids with no name of their own.
        let widgets = kids
            .iter()
            .copied()
            .filter(|k| doc.get_dictionary(*k).is_ok_and(|d| !d.has(b"T")))
            .collect();
        index.fields.entry(name.clone()).or_insert((id, widgets));
    }
    for kid in kids {
        if doc.get_dictionary(kid).is_ok_and(|d| d.has(b"T")) {
            walk(doc, kid, &name, index, depth + 1);
        }
    }
}

fn decode(doc: &lopdf::Document, object: &Object) -> Option<String> {
    let object = doc.dereference(object).map_or(object, |(_, o)| o);
    match object {
        Object::String(..) => lopdf::decode_text_string(object).ok(),
        _ => None,
    }
}

/// A PDF text string: PDFDocEncoding-safe ASCII as a literal, anything
/// else as UTF-16BE with a byte-order mark.
pub(super) fn text_string(s: &str) -> Object {
    if s.bytes()
        .all(|b| (0x20..0x7f).contains(&b) || b == b'\n' || b == b'\r' || b == b'\t')
    {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

/// `da` with its fill colour replaced: red, or black.
pub(super) fn recolor_da(da: &str, red: bool) -> String {
    let tokens: Vec<&str> = da.split_ascii_whitespace().collect();
    let mut out: Vec<&str> = Vec::with_capacity(tokens.len() + 4);
    for token in tokens {
        let operands = match token {
            "g" => 1,
            "rg" => 3,
            "k" => 4,
            _ => {
                out.push(token);
                continue;
            }
        };
        // Drop the operator and the numbers before it.
        let keep = out.len().saturating_sub(operands);
        if out[keep..].iter().all(|t| t.parse::<f64>().is_ok()) {
            out.truncate(keep);
        } else {
            out.push(token);
        }
    }
    let mut result = out.join(" ");
    if !result.is_empty() {
        result.push(' ');
    }
    result.push_str(if red { "1 0 0 rg" } else { "0 g" });
    result
}

/// Apply `edit` to the newest revision of the file at `path` and append the
/// result as an incremental update. Nothing is written when `edit` reports
/// no change. Encrypted files are left alone: an update to one would have to
/// be encrypted too.
fn update(
    path: &std::path::Path,
    edit: impl FnOnce(&mut IncrementalDocument, &FieldIndex) -> Result<bool, String>,
) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut inc: IncrementalDocument = bytes
        .as_slice()
        .try_into()
        .map_err(|e: lopdf::Error| e.to_string())?;
    let prev = inc.get_prev_documents();
    if prev.is_encrypted() {
        return Ok(false);
    }
    // New objects must be numbered past the previous trailer's /Size, which
    // counts an xref stream lopdf does not (see drop_xfa).
    let declared = prev
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    let Some(index) = index_fields(prev) else {
        return Ok(false);
    };
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    if !edit(&mut inc, &index)? {
        return Ok(false);
    }
    let mut out = Vec::with_capacity(bytes.len() + 4096);
    inc.save_to(&mut out).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, &out).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(true)
}

fn dict_mut(inc: &mut IncrementalDocument, id: ObjectId) -> Result<&mut Dictionary, String> {
    inc.opt_clone_object_to_new_document(id)
        .map_err(|e| e.to_string())?;
    inc.new_document
        .get_object_mut(id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| e.to_string())
}

/// Whether the file at `path` can take these updates at all.
pub(super) fn can_update(path: &std::path::Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| {
            let inc: IncrementalDocument = bytes.as_slice().try_into().ok()?;
            let prev = inc.get_prev_documents();
            (!prev.is_encrypted() && index_fields(prev).is_some()).then_some(())
        })
        .is_some()
}

/// Step 1: set each named field's text colour, red or black, where its
/// /DA says otherwise.
pub(super) fn set_text_colors(
    path: &std::path::Path,
    colors: &[(&str, bool)],
) -> Result<bool, String> {
    if colors.is_empty() {
        return Ok(false);
    }
    update(path, |inc, index| {
        let prev = inc.get_prev_documents();
        let acro_da = prev
            .catalog()
            .ok()
            .and_then(|c| c.get(b"AcroForm").ok())
            .and_then(|o| prev.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|f| f.get(b"DA").ok())
            .and_then(|o| o.as_str().ok())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default();
        let mut work: Vec<(ObjectId, String)> = Vec::new();
        for (name, red) in colors {
            let Some((field, widgets)) = index.fields.get(*name) else {
                continue;
            };
            let field_da = prev
                .get_dictionary(*field)
                .ok()
                .and_then(|d| d.get(b"DA").ok())
                .and_then(|o| o.as_str().ok())
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_else(|| acro_da.clone());
            let wanted = recolor_da(&field_da, *red);
            if wanted != field_da {
                work.push((*field, wanted));
            }
            // A widget with a /DA of its own overrides the field's.
            for widget in widgets {
                if let Some(da) = prev
                    .get_dictionary(*widget)
                    .ok()
                    .and_then(|d| d.get(b"DA").ok())
                    .and_then(|o| o.as_str().ok())
                {
                    let da = String::from_utf8_lossy(da).into_owned();
                    let wanted = recolor_da(&da, *red);
                    if wanted != da {
                        work.push((*widget, wanted));
                    }
                }
            }
        }
        let changed = !work.is_empty();
        for (id, da) in work {
            dict_mut(inc, id)?.set("DA", Object::String(da.into_bytes(), StringFormat::Literal));
        }
        Ok(changed)
    })
}

/// Step 3: put each named field's raw value back in /V.
pub(super) fn restore_raw_values(
    path: &std::path::Path,
    values: &[(&str, &str)],
) -> Result<bool, String> {
    if values.is_empty() {
        return Ok(false);
    }
    update(path, |inc, index| {
        let mut changed = false;
        for (name, raw) in values {
            let Some((field, _)) = index.fields.get(*name) else {
                continue;
            };
            dict_mut(inc, *field)?.set("V", text_string(raw));
            changed = true;
        }
        Ok(changed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recolors_default_appearance() {
        assert_eq!(recolor_da("/Helv 0 Tf 0 g", true), "/Helv 0 Tf 1 0 0 rg");
        assert_eq!(recolor_da("/Helv 9 Tf 1 0 0 rg", false), "/Helv 9 Tf 0 g");
        assert_eq!(
            recolor_da("/Cour 10 Tf 0 0 0 1 k", true),
            "/Cour 10 Tf 1 0 0 rg"
        );
        assert_eq!(recolor_da("/F1 12 Tf", true), "/F1 12 Tf 1 0 0 rg");
        assert_eq!(recolor_da("", false), "0 g");
    }

    #[test]
    fn text_strings() {
        assert_eq!(
            text_string("1234.5"),
            Object::String(b"1234.5".to_vec(), StringFormat::Literal)
        );
        assert_eq!(
            text_string("\u{20ac}"),
            Object::String(vec![0xFE, 0xFF, 0x20, 0xAC], StringFormat::Hexadecimal)
        );
    }
}

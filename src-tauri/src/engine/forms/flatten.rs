//! Flatten form: every widget's appearance merged into its page, and the
//! form removed.
//!
//! PDFium's `FPDFPage_Flatten` flattens *every* annotation on a page -
//! highlights, notes and stamps with the widgets - so it is not used. This
//! does for widgets only what it does: each widget's current appearance
//! stream (the /AS state of /AP /N) is drawn into the page as a form
//! XObject, placed by the PDF's own rule for appearances (the /BBox mapped
//! through /Matrix, then fitted to /Rect), and the widget leaves /Annots.
//! Hidden widgets just leave. Then /AcroForm goes from the catalog.
//!
//! Written as an incremental update like the rest of the form save.

use super::design::{number, Tx};
use lopdf::{dictionary, Dictionary, IncrementalDocument, Object, ObjectId, Stream};

/// Annotation flags that keep a widget off the page.
const ANNOT_HIDDEN: i64 = 1 << 1;
const ANNOT_NO_VIEW: i64 = 1 << 5;

#[cfg(test)]
fn mul(a: [f32; 6], b: [f32; 6]) -> [f32; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn floats(tx: &Tx, o: Option<&Object>, n: usize) -> Option<Vec<f32>> {
    let a = tx.resolve(o?).as_array().ok()?.clone();
    let v: Vec<f32> = a.iter().filter_map(|x| number(tx.resolve(x))).collect();
    (v.len() == n).then_some(v)
}

/// The matrix that fits an appearance stream with `bbox` and `matrix` to
/// `rect` (PDF 32000 12.5.5): the page's `cm` before `Do`, which applies
/// the stream's own /Matrix itself.
pub(super) fn placement(rect: [f32; 4], bbox: [f32; 4], matrix: [f32; 6]) -> Option<[f32; 6]> {
    let corners = [
        (bbox[0], bbox[1]),
        (bbox[2], bbox[1]),
        (bbox[0], bbox[3]),
        (bbox[2], bbox[3]),
    ];
    let mapped: Vec<(f32, f32)> = corners
        .iter()
        .map(|(x, y)| {
            (
                matrix[0] * x + matrix[2] * y + matrix[4],
                matrix[1] * x + matrix[3] * y + matrix[5],
            )
        })
        .collect();
    let x0 = mapped.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let x1 = mapped.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max);
    let y0 = mapped.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let y1 = mapped.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
    if x1 - x0 <= 1e-4 || y1 - y0 <= 1e-4 {
        return None;
    }
    let sx = (rect[2] - rect[0]) / (x1 - x0);
    let sy = (rect[3] - rect[1]) / (y1 - y0);
    Some([sx, 0.0, 0.0, sy, rect[0] - x0 * sx, rect[1] - y0 * sy])
}

/// The stream a widget currently shows: /AP /N, or its /AS state.
fn shown_appearance(tx: &Tx, widget: &Dictionary) -> Option<ObjectId> {
    let ap = tx.resolve(widget.get(b"AP").ok()?).as_dict().ok()?.clone();
    let normal = ap.get(b"N").ok()?;
    match normal {
        Object::Reference(id) => match tx.get(*id)? {
            Object::Stream(_) => Some(*id),
            Object::Dictionary(states) => {
                let state = widget.get(b"AS").and_then(Object::as_name).ok()?;
                states.get(state).ok()?.as_reference().ok()
            }
            _ => None,
        },
        Object::Dictionary(states) => {
            let state = widget.get(b"AS").and_then(Object::as_name).ok()?;
            states.get(state).ok()?.as_reference().ok()
        }
        _ => None,
    }
}

/// An appearance for a text or choice widget that has a value but no
/// appearance of its own (a form relying on /NeedAppearances): the value
/// drawn the way a reader regenerating it would.
fn drawn_value(
    tx: &mut Tx,
    widget: &Dictionary,
    rect: [f32; 4],
    helv: &mut Option<ObjectId>,
) -> Option<ObjectId> {
    let mut chain = vec![widget.clone()];
    for _ in 0..32 {
        let Some(parent) = chain
            .last()
            .and_then(|d| d.get(b"Parent").ok())
            .and_then(|o| o.as_reference().ok())
            .and_then(|id| tx.dict(id))
        else {
            break;
        };
        chain.push(parent);
    }
    let attr = |key: &[u8]| {
        chain
            .iter()
            .find_map(|d| d.get(key).ok().map(|o| tx.resolve(o).clone()))
    };
    let ft = attr(b"FT")?;
    let kind = ft.as_name().ok()?;
    if kind != b"Tx" && kind != b"Ch" {
        return None;
    }
    let mut value = match attr(b"V")? {
        Object::Array(items) => items.first().map(|o| tx.text(o)).unwrap_or_default(),
        other => tx.text(&other),
    };
    if value.is_empty() {
        return None;
    }
    if kind == b"Ch" {
        if let Some(Object::Array(opts)) = attr(b"Opt") {
            for o in opts {
                if let Object::Array(pair) = tx.resolve(&o) {
                    if pair.len() == 2 && tx.text(&pair[0]) == value {
                        value = tx.text(&pair[1]);
                        break;
                    }
                }
            }
        }
    }
    let da = attr(b"DA").map(|o| tx.text(&o)).unwrap_or_default();
    let tokens: Vec<&str> = da.split_whitespace().collect();
    let size = tokens
        .iter()
        .position(|t| *t == "Tf")
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| tokens[i].parse::<f32>().ok())
        .unwrap_or(0.0);
    let q = attr(b"Q")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .clamp(0, 2) as u8;
    let ff = attr(b"Ff").and_then(|o| o.as_i64().ok()).unwrap_or(0);
    let max_len = attr(b"MaxLen")
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .max(0) as u32;
    let multiline = kind == b"Tx" && ff & super::design::FF_MULTILINE != 0;
    let comb = if kind == b"Tx" && ff & super::design::FF_COMB != 0 {
        max_len
    } else {
        0
    };
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let frame = super::draw::Frame {
        w,
        h,
        border: false,
    };
    let ops = super::draw::text_appearance(&value, size, &frame, q, multiline, comb);
    let font = match helv {
        Some(id) => *id,
        None => {
            let id = tx.add(Object::Dictionary(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
                "Encoding" => "WinAnsiEncoding",
            }));
            *helv = Some(id);
            id
        }
    };
    let d = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![0.into(), 0.into(), Object::Real(w), Object::Real(h)],
        "Resources" => dictionary! { "Font" => dictionary! { "Helv" => font } },
    };
    Some(tx.add(Stream::new(d, ops.into_bytes())))
}

/// Flatten every widget of the document at `path`. Returns whether there
/// was a form to flatten.
pub fn flatten_form(path: &std::path::Path) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut inc: IncrementalDocument = bytes
        .as_slice()
        .try_into()
        .map_err(|e: lopdf::Error| e.to_string())?;
    if inc.get_prev_documents().is_encrypted() {
        return Err("an encrypted file's form cannot be flattened".into());
    }
    let declared = inc
        .get_prev_documents()
        .trailer
        .get(b"Size")
        .and_then(Object::as_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32;
    inc.new_document.max_id = inc.new_document.max_id.max(declared.saturating_sub(1));
    let changed = {
        let mut tx = Tx { inc: &mut inc };
        flatten(&mut tx)?
    };
    if !changed {
        return Ok(false);
    }
    let mut out = Vec::with_capacity(bytes.len() + 16 * 1024);
    inc.save_to(&mut out).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, &out).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(true)
}

fn flatten(tx: &mut Tx) -> Result<bool, String> {
    let root = tx.root()?;
    let mut catalog = tx.dict(root).ok_or("no catalog")?;
    let had_form = catalog.remove(b"AcroForm").is_some();
    let mut changed = had_form;
    for page in tx.pages() {
        changed |= flatten_page(tx, page);
    }
    if had_form {
        tx.put(root, catalog);
    }
    Ok(changed)
}

fn flatten_page(tx: &mut Tx, page: ObjectId) -> bool {
    let annots = tx.annots(page);
    let mut keep = Vec::with_capacity(annots.len());
    let mut draws: Vec<(ObjectId, [f32; 6])> = Vec::new();
    let mut removed = false;
    let mut helv: Option<ObjectId> = None;
    for entry in annots {
        let dict = match tx.resolve(&entry) {
            Object::Dictionary(d) => d.clone(),
            _ => {
                keep.push(entry);
                continue;
            }
        };
        if dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Widget") {
            keep.push(entry);
            continue;
        }
        removed = true;
        let flags = dict.get(b"F").and_then(Object::as_i64).unwrap_or(0);
        if flags & (ANNOT_HIDDEN | ANNOT_NO_VIEW) != 0 {
            continue;
        }
        let Some(rect) = floats(tx, dict.get(b"Rect").ok(), 4) else {
            continue;
        };
        let rect = [
            rect[0].min(rect[2]),
            rect[1].min(rect[3]),
            rect[0].max(rect[2]),
            rect[1].max(rect[3]),
        ];
        let Some(stream) =
            shown_appearance(tx, &dict).or_else(|| drawn_value(tx, &dict, rect, &mut helv))
        else {
            continue;
        };
        let Some(sd) = tx.dict(stream) else {
            continue;
        };
        let Some(bbox) = floats(tx, sd.get(b"BBox").ok(), 4) else {
            continue;
        };
        let matrix = floats(tx, sd.get(b"Matrix").ok(), 6)
            .map(|m| [m[0], m[1], m[2], m[3], m[4], m[5]])
            .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let Some(fit) = placement(rect, [bbox[0], bbox[1], bbox[2], bbox[3]], matrix) else {
            continue;
        };
        // An appearance stream is a form XObject in all but, sometimes,
        // its /Subtype; `Do` needs that.
        if sd.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form") {
            if let Some(Object::Stream(s)) = tx.get(stream).cloned() {
                let mut s = s;
                s.dict.set("Type", "XObject");
                s.dict.set("Subtype", "Form");
                tx.put(stream, Object::Stream(s));
            }
        }
        draws.push((stream, fit));
    }
    if !removed {
        return false;
    }
    tx.set_annots(page, keep);
    if draws.is_empty() {
        return true;
    }

    // Resources: the page's own (or inherited) copied onto the page, with
    // the appearances added under fresh names.
    let mut resources = tx
        .inherited(page, b"Resources")
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    let mut xobjects = resources
        .get(b"XObject")
        .ok()
        .map(|o| tx.resolve(o).clone())
        .and_then(|o| o.as_dict().ok().cloned())
        .unwrap_or_default();
    let mut ops = String::from("Q\n");
    let mut n = 0usize;
    for (stream, m) in draws {
        let name = loop {
            n += 1;
            let candidate = format!("SpFlat{n}");
            if !xobjects.has(candidate.as_bytes()) {
                break candidate;
            }
        };
        xobjects.set(name.as_bytes().to_vec(), stream);
        ops.push_str(&format!(
            "q {} {} {} {} {} {} cm /{name} Do Q\n",
            super::draw::num(m[0]),
            super::draw::num(m[1]),
            super::draw::num(m[2]),
            super::draw::num(m[3]),
            super::draw::num(m[4]),
            super::draw::num(m[5]),
        ));
    }
    resources.set("XObject", Object::Dictionary(xobjects));

    let Some(mut pd) = tx.dict(page) else {
        return true;
    };
    let mut contents: Vec<Object> = match pd.get(b"Contents").ok().cloned() {
        Some(Object::Array(a)) => a,
        Some(Object::Reference(id)) => match tx.get(id) {
            Some(Object::Array(a)) => a.clone(),
            _ => vec![Object::Reference(id)],
        },
        _ => Vec::new(),
    };
    // The page's own content may leave the graphics state changed; wrap it
    // so the appearances are drawn from a clean one.
    let open = tx.add(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let close = tx.add(Stream::new(dictionary! {}, ops.into_bytes()));
    contents.insert(0, Object::Reference(open));
    contents.push(Object::Reference(close));
    pd.set("Contents", Object::Array(contents));
    pd.set("Resources", Object::Dictionary(resources));
    tx.put(page, pd);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_fits_bbox_to_rect() {
        let id = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let m = placement([100.0, 200.0, 150.0, 220.0], [0.0, 0.0, 50.0, 20.0], id).unwrap();
        assert_eq!(m, [1.0, 0.0, 0.0, 1.0, 100.0, 200.0]);
        // A rotated appearance (page turned 90): box 50 x 20 drawn upright
        // ends up 20 wide and 50 tall.
        let r = [0.0, 1.0, -1.0, 0.0, 20.0, 0.0];
        let fit = placement([10.0, 10.0, 30.0, 60.0], [0.0, 0.0, 50.0, 20.0], r).unwrap();
        let both = mul(r, fit);
        let at = |x: f32, y: f32| {
            (
                both[0] * x + both[2] * y + both[4],
                both[1] * x + both[3] * y + both[5],
            )
        };
        let (ax, ay) = at(0.0, 0.0);
        let (bx, by) = at(50.0, 20.0);
        assert!((ax.min(bx) - 10.0).abs() < 1e-3 && (ax.max(bx) - 30.0).abs() < 1e-3);
        assert!((ay.min(by) - 10.0).abs() < 1e-3 && (ay.max(by) - 60.0).abs() < 1e-3);
    }
}

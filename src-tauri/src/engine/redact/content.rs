//! The content-stream filter at the heart of redaction.
//!
//! Walks a page's operators with a small graphics-state interpreter (CTM,
//! text state, text matrices) and rewrites what a mark covers:
//!
//! - text: each shown glyph gets a box from its font's advance width and the
//!   font's ascent/descent. Covered glyphs are cut out of the string and
//!   replaced by a TJ displacement of exactly their advance, so the glyphs that
//!   remain do not move. A font whose widths cannot be read loses the whole
//!   show operation when it comes near a mark.
//! - images: covered pixels are overwritten and the image re-encoded
//!   (`image.rs`); placements that cannot be decoded are dropped.
//! - vector paths: a painted path that lies inside a mark is dropped; one that
//!   crosses a mark's edge is kept but clipped so nothing paints under the
//!   mark. Its geometry stays in the file - documented trade-off: a table rule
//!   or page background running under a mark is not destroyed.
//! - form XObjects: descended into with their own matrix and resources. A
//!   changed form is written as a NEW object under a new resource name, so a
//!   form shared with other pages (or other placements on this page) keeps its
//!   original content there: copy-on-write.
//! - inline images: dropped when they touch a mark.
//! - shadings (`sh`): clipped out of the marks.
//! - marked content: /ActualText, /Alt and /E of a sequence that lost content
//!   are removed, so the text cannot survive as a replacement string.

use super::fonts::{read_font, FontInfo};
use super::geom::{number, Matrix, Rect};
use super::image::{redact_image, Decoder, ImageOutcome};
use super::Report;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::collections::HashMap;
use std::rc::Rc;

/// Form XObjects nest no deeper than this; deeper ones near a mark are
/// dropped.
const MAX_DEPTH: u32 = 12;
/// A glyph is removed when the mark covers this share of its box, or its
/// centre.
const GLYPH_COVER: f32 = 0.15;

/// One redaction region with the colour pixels under it are set to.
#[derive(Clone, Debug)]
pub struct MarkArea {
    pub rect: Rect,
    pub fill: [f32; 3],
}

#[derive(Clone)]
struct Gs {
    ctm: Matrix,
    font: Rc<FontInfo>,
    size: f32,
    tc: f32,
    tw: f32,
    th: f32,
    tl: f32,
    rise: f32,
    line_width: f32,
}

enum ShowItem {
    Text(Vec<u8>, StringFormat),
    Adjust(f32),
}

pub struct Filter<'a> {
    pub doc: &'a mut Document,
    pub marks: &'a [MarkArea],
    pub report: &'a mut Report,
    /// Decodes images lopdf cannot (PDFium, in the app).
    pub decoder: Option<Decoder<'a>>,
    fonts: HashMap<ObjectId, Rc<FontInfo>>,
    seq: u32,
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

fn op(name: &str, operands: Vec<Object>) -> Operation {
    Operation::new(name, operands)
}

/// Decode a content stream's bytes, strictly when possible. A stream that
/// only parses leniently loses whatever follows the error - over-removal -
/// and the report says so.
pub fn decode_ops(bytes: &[u8], report: &mut Report) -> Result<Vec<Operation>, String> {
    match Content::decode_strict(bytes) {
        Ok(c) => Ok(c.operations),
        Err(_) => {
            let c = Content::decode(bytes).map_err(|e| format!("unreadable content: {e}"))?;
            report.warnings.push(
                "a content stream was only partly readable; its unreadable tail was dropped".into(),
            );
            Ok(c.operations)
        }
    }
}

impl<'a> Filter<'a> {
    pub fn new(doc: &'a mut Document, marks: &'a [MarkArea], report: &'a mut Report) -> Self {
        Filter {
            doc,
            marks,
            report,
            decoder: None,
            fonts: HashMap::new(),
            seq: 0,
        }
    }

    fn rects(&self) -> Vec<Rect> {
        self.marks.iter().map(|m| m.rect).collect()
    }

    fn any_overlap(&self, b: &Rect) -> bool {
        self.marks.iter().any(|m| m.rect.overlaps(b))
    }

    fn glyph_hit(&self, b: &Rect) -> bool {
        let area = b.area();
        let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        self.marks.iter().any(|m| {
            m.rect.contains_point(cx, cy)
                || m.rect
                    .intersection(b)
                    .is_some_and(|i| area <= 1e-6 || i.area() >= GLYPH_COVER * area)
        })
    }

    fn font_for(&mut self, resources: &Dictionary, name: &[u8]) -> Rc<FontInfo> {
        let doc = &*self.doc;
        let fonts = resources.get(b"Font").ok().map(|o| deref(doc, o));
        let Some(Object::Dictionary(fonts)) = fonts else {
            return Rc::new(FontInfo::unknown());
        };
        let Ok(entry) = fonts.get(name) else {
            return Rc::new(FontInfo::unknown());
        };
        let id = entry.as_reference().ok();
        if let Some(id) = id {
            if let Some(f) = self.fonts.get(&id) {
                return f.clone();
            }
        }
        let info = match deref(doc, entry) {
            Object::Dictionary(d) => Rc::new(read_font(doc, d)),
            _ => Rc::new(FontInfo::unknown()),
        };
        if let Some(id) = id {
            self.fonts.insert(id, info.clone());
        }
        info
    }

    /// A fresh resource name not present in `dict`.
    fn fresh_name(&mut self, dict: &Dictionary) -> Vec<u8> {
        loop {
            self.seq += 1;
            let name = format!("SpRd{}", self.seq).into_bytes();
            if !dict.has(&name) {
                return name;
            }
        }
    }

    /// Register `id` under a new name in the resources' /XObject dictionary,
    /// which becomes a direct (copied) dictionary so a shared one is not
    /// modified.
    fn add_xobject(&mut self, resources: &mut Dictionary, id: ObjectId) -> Vec<u8> {
        let current = match resources.get(b"XObject").ok().map(|o| deref(self.doc, o)) {
            Some(Object::Dictionary(d)) => d.clone(),
            _ => Dictionary::new(),
        };
        let mut xobjects = current;
        let name = self.fresh_name(&xobjects);
        xobjects.set(name.clone(), Object::Reference(id));
        resources.set("XObject", Object::Dictionary(xobjects));
        name
    }

    /// Clip-out operators: one even-odd clip per mark, each a huge box with
    /// the mark cut out, in the user space of `ctm`. Applied in turn they
    /// leave everything outside every mark paintable.
    fn clip_out(&self, ctm: &Matrix, marks: &[Rect]) -> Vec<Operation> {
        let Some(inv) = ctm.invert() else {
            return Vec::new();
        };
        let mut ops = Vec::new();
        let big = Rect::new(-30000.0, -30000.0, 30000.0, 30000.0);
        for m in marks {
            for r in [big, *m] {
                let pts = [
                    inv.apply(r.x0, r.y0),
                    inv.apply(r.x1, r.y0),
                    inv.apply(r.x1, r.y1),
                    inv.apply(r.x0, r.y1),
                ];
                ops.push(op("m", vec![pts[0].0.into(), pts[0].1.into()]));
                for p in &pts[1..] {
                    ops.push(op("l", vec![p.0.into(), p.1.into()]));
                }
                ops.push(op("h", vec![]));
            }
            ops.push(op("W*", vec![]));
            ops.push(op("n", vec![]));
        }
        ops
    }

    /// Filter `ops` drawn with `resources` under `base`. Returns the new
    /// operators and whether anything changed; `resources` gains entries
    /// for any copied XObjects.
    pub fn filter(
        &mut self,
        ops: Vec<Operation>,
        resources: &mut Dictionary,
        base: Matrix,
        depth: u32,
    ) -> Result<(Vec<Operation>, bool), String> {
        let mut out: Vec<Operation> = Vec::with_capacity(ops.len());
        let mut changed = false;
        let mut gs = Gs {
            ctm: base,
            font: Rc::new(FontInfo::unknown()),
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            rise: 0.0,
            line_width: 1.0,
        };
        let mut stack: Vec<Gs> = Vec::new();
        let mut tm = Matrix::IDENTITY;
        let mut tlm = Matrix::IDENTITY;
        let mut path: Vec<Operation> = Vec::new();
        let mut points: Vec<(f32, f32)> = Vec::new();
        let mut clip: Option<&'static str> = None;
        // Open marked-content sequences: index of the opening op in `out`,
        // and whether content inside it was removed.
        let mut marked: Vec<(usize, bool)> = Vec::new();
        // XObject names whose placement was replaced or dropped.
        let mut replaced: Vec<Vec<u8>> = Vec::new();

        let nums = |o: &[Object]| -> Vec<f32> { o.iter().filter_map(number).collect() };

        for operation in ops {
            let name = operation.operator.as_str();
            let mut removed_here = false;
            match name {
                "q" => {
                    stack.push(gs.clone());
                    out.push(operation);
                }
                "Q" => {
                    if let Some(g) = stack.pop() {
                        gs = g;
                    }
                    out.push(operation);
                }
                "cm" => {
                    if let Some(m) = Matrix::from_operands(&operation.operands) {
                        gs.ctm = m.then(&gs.ctm);
                    }
                    out.push(operation);
                }
                "w" => {
                    if let Some(w) = operation.operands.first().and_then(number) {
                        gs.line_width = w;
                    }
                    out.push(operation);
                }
                "gs" => {
                    if let Some(Object::Name(n)) = operation.operands.first() {
                        self.ext_gstate(resources, n, &mut gs);
                    }
                    out.push(operation);
                }
                "BT" => {
                    tm = Matrix::IDENTITY;
                    tlm = Matrix::IDENTITY;
                    out.push(operation);
                }
                "Tf" => {
                    if let (Some(Object::Name(n)), Some(size)) = (
                        operation.operands.first(),
                        operation.operands.get(1).and_then(number),
                    ) {
                        gs.font = self.font_for(resources, n);
                        gs.size = size;
                    }
                    out.push(operation);
                }
                "Tc" | "Tw" | "Tz" | "TL" | "Ts" => {
                    if let Some(v) = operation.operands.first().and_then(number) {
                        match name {
                            "Tc" => gs.tc = v,
                            "Tw" => gs.tw = v,
                            "Tz" => gs.th = v / 100.0,
                            "TL" => gs.tl = v,
                            _ => gs.rise = v,
                        }
                    }
                    out.push(operation);
                }
                "Td" | "TD" => {
                    let v = nums(&operation.operands);
                    if v.len() == 2 {
                        if name == "TD" {
                            gs.tl = -v[1];
                        }
                        tlm = Matrix::translate(v[0], v[1]).then(&tlm);
                        tm = tlm;
                    }
                    out.push(operation);
                }
                "Tm" => {
                    if let Some(m) = Matrix::from_operands(&operation.operands) {
                        tlm = m;
                        tm = m;
                    }
                    out.push(operation);
                }
                "T*" => {
                    tlm = Matrix::translate(0.0, -gs.tl).then(&tlm);
                    tm = tlm;
                    out.push(operation);
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    let mut prefix = Vec::new();
                    let string_operand = match name {
                        "'" => operation.operands.first(),
                        "\"" => operation.operands.get(2),
                        _ => operation.operands.first(),
                    };
                    if name == "\"" {
                        let v = nums(&operation.operands[..operation.operands.len().min(2)]);
                        if v.len() == 2 {
                            gs.tw = v[0];
                            gs.tc = v[1];
                            prefix.push(op("Tw", vec![v[0].into()]));
                            prefix.push(op("Tc", vec![v[1].into()]));
                        }
                    }
                    if name == "'" || name == "\"" {
                        tlm = Matrix::translate(0.0, -gs.tl).then(&tlm);
                        tm = tlm;
                        prefix.push(op("T*", vec![]));
                    }
                    let items = match (name, string_operand) {
                        ("TJ", Some(Object::Array(a))) => a
                            .iter()
                            .filter_map(|o| match o {
                                Object::String(s, f) => Some(ShowItem::Text(s.clone(), *f)),
                                o => number(o).map(ShowItem::Adjust),
                            })
                            .collect(),
                        (_, Some(Object::String(s, f))) => vec![ShowItem::Text(s.clone(), *f)],
                        _ => Vec::new(),
                    };
                    match self.show(&gs, &mut tm, items) {
                        None => out.push(operation),
                        Some(array) => {
                            changed = true;
                            removed_here = true;
                            out.extend(prefix);
                            out.push(op("TJ", vec![Object::Array(array)]));
                        }
                    }
                }
                "m" | "l" | "c" | "v" | "y" | "re" | "h" => {
                    let v = nums(&operation.operands);
                    if name == "re" && v.len() == 4 {
                        points.extend([
                            (v[0], v[1]),
                            (v[0] + v[2], v[1]),
                            (v[0] + v[2], v[1] + v[3]),
                            (v[0], v[1] + v[3]),
                        ]);
                    } else {
                        points.extend(v.chunks_exact(2).map(|p| (p[0], p[1])));
                    }
                    path.push(operation);
                }
                "W" | "W*" => {
                    clip = Some(if name == "W" { "W" } else { "W*" });
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                    let built = std::mem::take(&mut path);
                    let pts = std::mem::take(&mut points);
                    let clip_op = clip.take();
                    let emit_clip = |out: &mut Vec<Operation>| {
                        if let Some(c) = clip_op {
                            out.extend(built.iter().cloned());
                            out.push(op(c, vec![]));
                            out.push(op("n", vec![]));
                        }
                    };
                    if name == "n" || pts.is_empty() {
                        out.extend(built.iter().cloned());
                        if let Some(c) = clip_op {
                            out.push(op(c, vec![]));
                        }
                        out.push(operation);
                        continue;
                    }
                    let mut bbox = Rect::bounding(
                        &pts.iter()
                            .map(|&(x, y)| gs.ctm.apply(x, y))
                            .collect::<Vec<_>>(),
                    );
                    if matches!(name, "S" | "s" | "B" | "B*" | "b" | "b*") {
                        let [a, b, c, d, _, _] = gs.ctm.0;
                        let pad = gs.line_width.max(1.0) / 2.0 * (a * d - b * c).abs().sqrt();
                        bbox =
                            Rect::new(bbox.x0 - pad, bbox.y0 - pad, bbox.x1 + pad, bbox.y1 + pad);
                    }
                    let hits: Vec<Rect> = self
                        .marks
                        .iter()
                        .map(|m| m.rect)
                        .filter(|m| m.overlaps(&bbox) || m.contains(&bbox))
                        .collect();
                    if hits.is_empty() {
                        out.extend(built.iter().cloned());
                        if let Some(c) = clip_op {
                            out.push(op(c, vec![]));
                        }
                        out.push(operation);
                        continue;
                    }
                    changed = true;
                    removed_here = true;
                    if hits.iter().any(|m| m.contains(&bbox)) {
                        self.report.paths_removed += 1;
                    } else {
                        self.report.paths_clipped += 1;
                        out.push(op("q", vec![]));
                        out.extend(self.clip_out(&gs.ctm, &hits));
                        out.extend(built.iter().cloned());
                        out.push(operation);
                        out.push(op("Q", vec![]));
                    }
                    emit_clip(&mut out);
                }
                "sh" => {
                    // A shading fills the clip region; keep it out of the marks.
                    changed = true;
                    out.push(op("q", vec![]));
                    let rects = self.rects();
                    out.extend(self.clip_out(&gs.ctm, &rects));
                    out.push(operation);
                    out.push(op("Q", vec![]));
                }
                "BI" => {
                    let placed = gs.ctm.map_box(0.0, 0.0, 1.0, 1.0);
                    if self.any_overlap(&placed) {
                        changed = true;
                        removed_here = true;
                        self.report.inline_images_removed += 1;
                    } else {
                        out.push(operation);
                    }
                }
                "Do" => {
                    let Some(Object::Name(xname)) = operation.operands.first() else {
                        out.push(operation);
                        continue;
                    };
                    match self.xobject(resources, xname, &gs.ctm, depth)? {
                        XResult::Keep => out.push(operation),
                        XResult::Drop => {
                            changed = true;
                            removed_here = true;
                            replaced.push(xname.clone());
                        }
                        XResult::Replace(new_name) => {
                            replaced.push(xname.clone());
                            changed = true;
                            removed_here = true;
                            out.push(op("Do", vec![Object::Name(new_name)]));
                        }
                    }
                }
                "BMC" | "BDC" => {
                    marked.push((out.len(), false));
                    out.push(operation);
                }
                "EMC" => {
                    if let Some((at, tainted)) = marked.pop() {
                        if tainted && self.strip_replacement_text(resources, &mut out[at]) {
                            changed = true;
                        }
                    }
                    out.push(operation);
                }
                _ => out.push(operation),
            }
            if removed_here {
                for frame in marked.iter_mut() {
                    frame.1 = true;
                }
            }
        }
        // Unterminated path construction at the end: keep as is.
        out.extend(path);
        // A replaced XObject's old name goes unless something still draws it,
        // or the original would survive in the file through the resources.
        if !replaced.is_empty() {
            let used: std::collections::HashSet<&[u8]> = out
                .iter()
                .filter(|o| o.operator == "Do")
                .filter_map(|o| o.operands.first().and_then(|n| n.as_name().ok()))
                .collect();
            if let Ok(Object::Dictionary(x)) = resources.get_mut(b"XObject") {
                for name in &replaced {
                    if !used.contains(name.as_slice()) {
                        x.remove(name);
                    }
                }
            }
        }
        Ok((out, changed))
    }

    fn ext_gstate(&mut self, resources: &Dictionary, name: &[u8], gs: &mut Gs) {
        let doc = &*self.doc;
        let Some(Object::Dictionary(states)) =
            resources.get(b"ExtGState").ok().map(|o| deref(doc, o))
        else {
            return;
        };
        let Some(Object::Dictionary(state)) = states.get(name).ok().map(|o| deref(doc, o)) else {
            return;
        };
        if let Some(lw) = state.get(b"LW").ok().and_then(number) {
            gs.line_width = lw;
        }
        if let Ok(Object::Array(font)) = state.get(b"Font") {
            if let (Some(Object::Reference(id)), Some(size)) =
                (font.first(), font.get(1).and_then(number))
            {
                let id = *id;
                let info = match self.fonts.get(&id) {
                    Some(f) => f.clone(),
                    None => {
                        let info = match doc.get_dictionary(id) {
                            Ok(d) => Rc::new(read_font(doc, d)),
                            Err(_) => Rc::new(FontInfo::unknown()),
                        };
                        self.fonts.insert(id, info.clone());
                        info
                    }
                };
                gs.font = info;
                gs.size = size;
            }
        }
    }

    /// Remove /ActualText, /Alt and /E from a marked-content opener whose
    /// sequence lost content. Returns whether anything was removed.
    fn strip_replacement_text(&self, resources: &Dictionary, opener: &mut Operation) -> bool {
        if opener.operator != "BDC" || opener.operands.len() < 2 {
            return false;
        }
        const KEYS: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];
        let props = match &opener.operands[1] {
            Object::Dictionary(d) => Some(d.clone()),
            Object::Name(n) => {
                let doc = &*self.doc;
                match resources.get(b"Properties").ok().map(|o| deref(doc, o)) {
                    Some(Object::Dictionary(p)) => match p.get(n).ok().map(|o| deref(doc, o)) {
                        Some(Object::Dictionary(d)) => Some(d.clone()),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        };
        let Some(mut props) = props else {
            return false;
        };
        if !KEYS.iter().any(|k| props.has(k)) {
            return false;
        }
        for k in KEYS {
            props.remove(k);
        }
        opener.operands[1] = Object::Dictionary(props);
        true
    }

    /// Place the glyphs of one show operation; `None` when nothing is
    /// removed, else the replacement TJ array. Advances `tm` either way.
    fn show(&mut self, gs: &Gs, tm: &mut Matrix, items: Vec<ShowItem>) -> Option<Vec<Object>> {
        let font = gs.font.clone();
        let trm = tm.then(&gs.ctm);
        let size = gs.size;
        let th = if font.vertical { 1.0 } else { gs.th };
        // Advance along the writing direction: rightwards, or downwards for
        // a vertical font.
        let mut disp = 0.0f32;
        let vertical = font.vertical;
        // A TJ number equivalent to `adv` of advance.
        let to_tj = |adv: f32| {
            if vertical {
                adv / size * 1000.0
            } else {
                -adv / (size * th) * 1000.0
            }
        };
        let mut hit_any = false;
        let mut out: Vec<Object> = Vec::new();
        let mut removed = 0u64;
        let mut pending: f32 = 0.0; // removed advance, text space units
        let usable = font.reliable && size.abs() > 1e-6;

        let glyph_box = |disp: f32, adv_em: f32| -> Rect {
            if vertical {
                let half = 0.5 * size.abs();
                trm.map_box(-half, -disp - size.abs(), half, -disp)
            } else {
                let w = (adv_em * size * th).abs().max(0.05 * size.abs());
                let y0 = font.descent * size + gs.rise;
                let y1 = font.ascent * size + gs.rise;
                trm.map_box(disp, y0, disp + w, y1)
            }
        };

        for item in &items {
            match item {
                ShowItem::Adjust(n) => {
                    let adv = if vertical {
                        n / 1000.0 * size
                    } else {
                        -n / 1000.0 * size * th
                    };
                    disp += adv;
                    if pending != 0.0 {
                        pending += adv;
                    } else {
                        out.push(Object::Real(*n));
                    }
                }
                ShowItem::Text(bytes, format) => {
                    let mut kept: Vec<u8> = Vec::new();
                    for g in font.glyphs(bytes) {
                        let adv_em = if usable { g.width } else { 1.0 };
                        let adv = (g.width * size + gs.tc + if g.space { gs.tw } else { 0.0 }) * th;
                        let b = glyph_box(disp, adv_em);
                        let hit = b.is_valid() && self.glyph_hit(&b);
                        if hit {
                            hit_any = true;
                            removed += 1;
                            if !kept.is_empty() {
                                out.push(Object::String(std::mem::take(&mut kept), *format));
                            }
                            pending += adv;
                        } else {
                            if pending != 0.0 {
                                out.push(Object::Real(to_tj(pending)));
                                pending = 0.0;
                            }
                            kept.extend_from_slice(&bytes[g.start..g.start + g.len]);
                        }
                        disp += adv;
                    }
                    if !kept.is_empty() {
                        out.push(Object::String(kept, *format));
                    }
                }
            }
        }
        *tm = if vertical {
            Matrix::translate(0.0, -disp).then(tm)
        } else {
            Matrix::translate(disp, 0.0).then(tm)
        };
        if !hit_any {
            return None;
        }
        if !usable {
            // Metrics unknown: the positions above are guesses, so nothing of
            // this operation is kept. The text after it on the same line may
            // shift; nothing under the mark survives.
            self.report.text_ops_removed += 1;
            self.report.glyphs_removed += items
                .iter()
                .map(|i| match i {
                    ShowItem::Text(b, _) => font.glyphs(b).len() as u64,
                    ShowItem::Adjust(_) => 0,
                })
                .sum::<u64>();
            return Some(Vec::new());
        }
        if pending != 0.0 {
            out.push(Object::Real(to_tj(pending)));
        }
        // Merge adjacent displacements.
        let mut merged: Vec<Object> = Vec::with_capacity(out.len());
        for o in out {
            if let (Some(Object::Real(prev)), Object::Real(n)) = (merged.last_mut(), &o) {
                *prev += n;
                continue;
            }
            merged.push(o);
        }
        self.report.glyphs_removed += removed;
        Some(merged)
    }

    fn xobject(
        &mut self,
        resources: &mut Dictionary,
        name: &[u8],
        ctm: &Matrix,
        depth: u32,
    ) -> Result<XResult, String> {
        let doc = &*self.doc;
        let entry = match resources.get(b"XObject").ok().map(|o| deref(doc, o)) {
            Some(Object::Dictionary(x)) => x.get(name).ok().cloned(),
            _ => None,
        };
        let Some(entry) = entry else {
            return Ok(XResult::Keep);
        };
        let Object::Stream(stream) = deref(doc, &entry) else {
            return Ok(XResult::Keep);
        };
        let subtype = stream
            .dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or(b"")
            .to_vec();
        match subtype.as_slice() {
            b"Image" => {
                let placed = ctm.map_box(0.0, 0.0, 1.0, 1.0);
                let hits: Vec<&MarkArea> = self
                    .marks
                    .iter()
                    .filter(|m| m.rect.overlaps(&placed))
                    .collect();
                if hits.is_empty() {
                    return Ok(XResult::Keep);
                }
                let rects: Vec<Rect> = hits.iter().map(|m| m.rect).collect();
                let fill = hits[0].fill;
                match redact_image(doc, stream, ctm, &rects, fill, self.decoder) {
                    ImageOutcome::Unchanged => Ok(XResult::Keep),
                    ImageOutcome::Remove => {
                        self.report.images_removed += 1;
                        Ok(XResult::Drop)
                    }
                    ImageOutcome::Replaced(mut image) => {
                        // The soft mask carries the same shapes; blank it too.
                        if let Ok(Object::Reference(smask_id)) = image.dict.get(b"SMask").cloned() {
                            if let Ok(Object::Stream(smask)) = doc.get_object(smask_id) {
                                match redact_image(
                                    doc,
                                    smask,
                                    ctm,
                                    &rects,
                                    [1.0, 1.0, 1.0],
                                    self.decoder,
                                ) {
                                    ImageOutcome::Replaced(s) => {
                                        let id = self.doc.add_object(s);
                                        image.dict.set("SMask", Object::Reference(id));
                                    }
                                    ImageOutcome::Remove => {
                                        image.dict.remove(b"SMask");
                                    }
                                    ImageOutcome::Unchanged => {}
                                }
                            }
                        }
                        self.report.images_blanked += 1;
                        let id = self.doc.add_object(image);
                        Ok(XResult::Replace(self.add_xobject(resources, id)))
                    }
                }
            }
            b"Form" => {
                let matrix = stream
                    .dict
                    .get(b"Matrix")
                    .ok()
                    .map(|o| deref(doc, o))
                    .and_then(|o| o.as_array().ok())
                    .and_then(|a| Matrix::from_operands(a))
                    .unwrap_or(Matrix::IDENTITY);
                let fctm = matrix.then(ctm);
                let bbox = stream
                    .dict
                    .get(b"BBox")
                    .ok()
                    .map(|o| deref(doc, o))
                    .and_then(|o| o.as_array().ok())
                    .map(|a| a.iter().filter_map(number).collect::<Vec<_>>())
                    .filter(|v| v.len() == 4)
                    .map(|v| fctm.map_box(v[0], v[1], v[2], v[3]));
                if let Some(b) = bbox {
                    if !self.any_overlap(&b) {
                        return Ok(XResult::Keep);
                    }
                }
                if depth >= MAX_DEPTH {
                    self.report.forms_removed += 1;
                    return Ok(XResult::Drop);
                }
                let bytes = stream
                    .decompressed_content()
                    .map_err(|e| format!("cannot decode a form XObject: {e}"))?;
                let mut form_res = match stream.dict.get(b"Resources").ok().map(|o| deref(doc, o)) {
                    Some(Object::Dictionary(d)) => d.clone(),
                    _ => resources.clone(),
                };
                let mut form_dict = stream.dict.clone();
                let ops = decode_ops(&bytes, self.report)?;
                let (new_ops, changed) = self.filter(ops, &mut form_res, fctm, depth + 1)?;
                if !changed {
                    return Ok(XResult::Keep);
                }
                let content = Content {
                    operations: new_ops,
                }
                .encode()
                .map_err(|e| format!("cannot encode a form XObject: {e}"))?;
                form_dict.remove(b"Filter");
                form_dict.remove(b"DecodeParms");
                form_dict.remove(b"Length");
                form_dict.set("Resources", Object::Dictionary(form_res));
                let mut form = Stream::new(form_dict, content);
                let _ = form.compress();
                self.report.forms_rewritten += 1;
                let id = self.doc.add_object(form);
                Ok(XResult::Replace(self.add_xobject(resources, id)))
            }
            _ => Ok(XResult::Keep),
        }
    }
}

enum XResult {
    Keep,
    Drop,
    Replace(Vec<u8>),
}

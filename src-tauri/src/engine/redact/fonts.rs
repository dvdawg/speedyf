//! Just enough of a font to place its glyphs: how a shown string splits into
//! character codes, how far each one advances, and how tall it stands.
//!
//! Redaction removes the glyphs a mark covers from the content stream. That
//! needs each glyph's box, which needs its advance width - from /Widths for
//! simple and Type 3 fonts, /W for CID fonts. A font whose metrics cannot be
//! read is marked unreliable, and the content filter then removes every show
//! operation of it that comes near a mark, whole: over-removal, never under.

use lopdf::{Dictionary, Document, Object};
use std::collections::HashMap;

use super::geom::number;

/// One character code of a shown string.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// Byte offset of the code in the string.
    pub start: usize,
    /// Byte length of the code (1 for simple fonts, 1-4 for CID fonts).
    pub len: usize,
    /// Horizontal advance (vertical, for vertical fonts) in text space units
    /// per unit of font size: 0.5 is half an em.
    pub width: f32,
    /// A single-byte code 32, the only code word spacing applies to.
    pub space: bool,
}

#[derive(Clone, Debug)]
enum Codes {
    /// One byte per code.
    Single,
    /// Code lengths from a CMap's codespace ranges: `(len, low, high)`.
    Ranges(Vec<(usize, Vec<u8>, Vec<u8>)>),
}

#[derive(Clone, Debug)]
pub struct FontInfo {
    codes: Codes,
    /// Advance per code (simple) or per CID (composite), in em.
    widths: HashMap<u32, f32>,
    default_width: f32,
    /// Code -> CID for composite fonts with an embedded CMap; identity
    /// otherwise.
    cids: Option<HashMap<u32, u32>>,
    pub vertical: bool,
    /// Glyph extent above and below the baseline, in em.
    pub ascent: f32,
    pub descent: f32,
    /// Whether the advances can be trusted.
    pub reliable: bool,
}

impl FontInfo {
    /// A font nothing is known about. Every show of it is unreliable.
    pub fn unknown() -> FontInfo {
        FontInfo {
            codes: Codes::Single,
            widths: HashMap::new(),
            default_width: 0.6,
            cids: None,
            vertical: false,
            ascent: 0.9,
            descent: -0.25,
            reliable: false,
        }
    }

    /// Split `bytes` into glyphs.
    pub fn glyphs(&self, bytes: &[u8]) -> Vec<Glyph> {
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let len = match &self.codes {
                Codes::Single => 1,
                Codes::Ranges(ranges) => code_len(ranges, &bytes[i..]),
            }
            .min(bytes.len() - i)
            .max(1);
            let code = bytes[i..i + len]
                .iter()
                .fold(0u32, |acc, b| (acc << 8) | u32::from(*b));
            let key = match &self.cids {
                Some(map) => map.get(&code).copied().unwrap_or(code),
                None => code,
            };
            let width = self.widths.get(&key).copied().unwrap_or(self.default_width);
            out.push(Glyph {
                start: i,
                len,
                width,
                space: len == 1 && code == 32,
            });
            i += len;
        }
        out
    }
}

/// The length of the code starting `bytes`, from codespace ranges: the
/// shortest range whose bounds contain the leading bytes.
fn code_len(ranges: &[(usize, Vec<u8>, Vec<u8>)], bytes: &[u8]) -> usize {
    for (len, low, high) in ranges {
        if bytes.len() < *len {
            continue;
        }
        if (0..*len).all(|k| bytes[k] >= low[k] && bytes[k] <= high[k]) {
            return *len;
        }
    }
    // No range matched: the shortest declared length, as PDF readers do.
    ranges.iter().map(|r| r.0).min().unwrap_or(1)
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

fn dict_of<'a>(doc: &'a Document, o: Option<&'a Object>) -> Option<&'a Dictionary> {
    match deref(doc, o?) {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn num_of(doc: &Document, o: Option<&Object>) -> Option<f32> {
    number(deref(doc, o?))
}

fn array_of<'a>(doc: &'a Document, o: Option<&'a Object>) -> Option<&'a Vec<Object>> {
    match deref(doc, o?) {
        Object::Array(a) => Some(a),
        _ => None,
    }
}

/// Ascent and descent in em, from a font descriptor, kept inside sane bounds
/// so a glyph box always covers its ink: some fonts declare 0 for both.
fn vertical_extent(doc: &Document, descriptor: Option<&Dictionary>, scale: f32) -> (f32, f32) {
    let asc = descriptor
        .and_then(|d| num_of(doc, d.get(b"Ascent").ok()))
        .map(|v| v * scale)
        .unwrap_or(0.9);
    let desc = descriptor
        .and_then(|d| num_of(doc, d.get(b"Descent").ok()))
        .map(|v| v * scale)
        .unwrap_or(-0.25);
    (asc.clamp(0.75, 1.3), desc.clamp(-0.6, -0.2))
}

/// Read a font dictionary.
pub fn read_font(doc: &Document, font: &Dictionary) -> FontInfo {
    let subtype = font
        .get(b"Subtype")
        .and_then(Object::as_name)
        .unwrap_or(b"");
    match subtype {
        b"Type0" => read_type0(doc, font),
        b"Type3" => read_type3(doc, font),
        _ => read_simple(doc, font),
    }
}

fn read_simple(doc: &Document, font: &Dictionary) -> FontInfo {
    let descriptor = dict_of(doc, font.get(b"FontDescriptor").ok());
    let (ascent, descent) = vertical_extent(doc, descriptor, 0.001);
    let mut info = FontInfo {
        ascent,
        descent,
        ..FontInfo::unknown()
    };
    let base = font
        .get(b"BaseFont")
        .and_then(Object::as_name)
        .map(|n| String::from_utf8_lossy(n).to_string())
        .unwrap_or_default();
    let missing = descriptor
        .and_then(|d| num_of(doc, d.get(b"MissingWidth").ok()))
        .unwrap_or(0.0)
        / 1000.0;
    if let (Some(first), Some(widths)) = (
        num_of(doc, font.get(b"FirstChar").ok()),
        array_of(doc, font.get(b"Widths").ok()),
    ) {
        let first = first.max(0.0) as u32;
        for (k, w) in widths.iter().enumerate() {
            if let Some(w) = number(deref(doc, w)) {
                info.widths.insert(first + k as u32, w / 1000.0);
            }
        }
        info.default_width = missing;
        info.reliable = true;
    } else if base.contains("Courier") {
        // The one standard font whose metrics need no table.
        info.default_width = 0.6;
        info.reliable = true;
    }
    info
}

fn read_type3(doc: &Document, font: &Dictionary) -> FontInfo {
    let matrix = array_of(doc, font.get(b"FontMatrix").ok())
        .and_then(|a| {
            let v: Vec<f32> = a.iter().filter_map(|o| number(deref(doc, o))).collect();
            (v.len() == 6).then_some(v)
        })
        .unwrap_or_else(|| vec![0.001, 0.0, 0.0, 0.001, 0.0, 0.0]);
    let (sx, sy) = (matrix[0], matrix[3]);
    let mut info = FontInfo::unknown();
    // Glyph height from the font's bounding box, in em after the matrix.
    if let Some(bbox) = array_of(doc, font.get(b"FontBBox").ok()) {
        let v: Vec<f32> = bbox.iter().filter_map(|o| number(deref(doc, o))).collect();
        if v.len() == 4 && (v[3] - v[1]).abs() > 0.0 {
            let (lo, hi) = ((v[1] * sy).min(v[3] * sy), (v[1] * sy).max(v[3] * sy));
            info.ascent = hi.max(0.75);
            info.descent = lo.min(-0.2);
        }
    }
    if let (Some(first), Some(widths)) = (
        num_of(doc, font.get(b"FirstChar").ok()),
        array_of(doc, font.get(b"Widths").ok()),
    ) {
        let first = first.max(0.0) as u32;
        for (k, w) in widths.iter().enumerate() {
            if let Some(w) = number(deref(doc, w)) {
                info.widths.insert(first + k as u32, w * sx);
            }
        }
        info.default_width = 0.0;
        info.reliable = true;
    }
    info
}

fn read_type0(doc: &Document, font: &Dictionary) -> FontInfo {
    let mut info = FontInfo::unknown();
    let descendant = array_of(doc, font.get(b"DescendantFonts").ok())
        .and_then(|a| a.first())
        .and_then(|o| dict_of(doc, Some(o)));
    let descriptor = descendant.and_then(|d| dict_of(doc, d.get(b"FontDescriptor").ok()));
    let (ascent, descent) = vertical_extent(doc, descriptor, 0.001);
    info.ascent = ascent;
    info.descent = descent;

    // How the string splits into codes, and which CID each code selects.
    let encoding = font.get(b"Encoding").ok().map(|o| deref(doc, o));
    let mut understood = false;
    match encoding {
        Some(Object::Name(name)) => {
            let name = name.as_slice();
            info.vertical = name.ends_with(b"-V");
            if name == b"Identity-H" || name == b"Identity-V" {
                info.codes = two_byte();
                understood = true;
            } else if name.starts_with(b"Uni") && (name.contains(&b'2') || name.contains(&b'6')) {
                // UCS-2 / UTF-16 predefined CMaps: two-byte codes. The CIDs
                // differ from the codes, so widths stay unknown (unreliable).
                info.codes = two_byte();
            }
        }
        Some(Object::Stream(stream)) => {
            if let Ok(bytes) = stream.decompressed_content() {
                let cmap = parse_cmap(&bytes);
                if !cmap.ranges.is_empty() {
                    info.codes = Codes::Ranges(cmap.ranges);
                    info.cids = Some(cmap.cids);
                    understood = true;
                }
            }
            info.vertical = stream
                .dict
                .get(b"WMode")
                .ok()
                .and_then(number)
                .is_some_and(|w| w == 1.0);
        }
        _ => {}
    }

    let Some(cid_font) = descendant else {
        return info;
    };
    if info.vertical {
        let dw2 = array_of(doc, cid_font.get(b"DW2").ok())
            .map(|a| {
                a.iter()
                    .filter_map(|o| number(deref(doc, o)))
                    .collect::<Vec<_>>()
            })
            .filter(|v| v.len() == 2)
            .unwrap_or_else(|| vec![880.0, -1000.0]);
        // Vertical advance is the (negative) w1y; the box spans one em.
        info.default_width = -dw2[1] / 1000.0;
        if let Some(w2) = array_of(doc, cid_font.get(b"W2").ok()) {
            read_w(doc, w2, 3, &mut info.widths);
            for v in info.widths.values_mut() {
                *v = -*v;
            }
        }
    } else {
        info.default_width = num_of(doc, cid_font.get(b"DW").ok()).unwrap_or(1000.0) / 1000.0;
        if let Some(w) = array_of(doc, cid_font.get(b"W").ok()) {
            read_w(doc, w, 1, &mut info.widths);
        }
    }
    info.reliable = understood;
    info
}

fn two_byte() -> Codes {
    Codes::Ranges(vec![(2, vec![0, 0], vec![0xff, 0xff])])
}

/// A /W (stride 1) or /W2 (stride 3, keeping only w1y) array.
fn read_w(doc: &Document, w: &[Object], stride: usize, out: &mut HashMap<u32, f32>) {
    let mut i = 0;
    while i < w.len() {
        let Some(first) = number(deref(doc, &w[i])) else {
            break;
        };
        let first = first.max(0.0) as u32;
        match w.get(i + 1).map(|o| deref(doc, o)) {
            Some(Object::Array(list)) => {
                let vals: Vec<f32> = list.iter().filter_map(|o| number(deref(doc, o))).collect();
                for (k, chunk) in vals.chunks(stride).enumerate() {
                    if let Some(v) = chunk.first() {
                        out.insert(first + k as u32, v / 1000.0);
                    }
                }
                i += 2;
            }
            Some(o) => {
                let Some(last) = number(o) else { break };
                let vals: Vec<f32> = w
                    .iter()
                    .skip(i + 2)
                    .take(stride)
                    .filter_map(|o| number(deref(doc, o)))
                    .collect();
                if let Some(v) = vals.first() {
                    let last = (last.max(0.0) as u32).min(first + 65_535);
                    for cid in first..=last {
                        out.insert(cid, v / 1000.0);
                    }
                }
                i += 2 + stride;
            }
            None => break,
        }
    }
}

struct CMap {
    ranges: Vec<(usize, Vec<u8>, Vec<u8>)>,
    cids: HashMap<u32, u32>,
}

/// Codespace ranges and CID mappings of an embedded CMap. Only the parts a
/// glyph split needs; `usecmap` and anything else are ignored.
fn parse_cmap(bytes: &[u8]) -> CMap {
    let text = String::from_utf8_lossy(bytes);
    let mut cmap = CMap {
        ranges: Vec::new(),
        cids: HashMap::new(),
    };
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '[' || c == ']')
        .filter(|t| !t.is_empty())
        .collect();
    let hex = |t: &str| -> Option<Vec<u8>> {
        let t = t.strip_prefix('<')?.strip_suffix('>')?;
        let t: String = t.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        (0..t.len() / 2)
            .map(|k| u8::from_str_radix(&t[2 * k..2 * k + 2], 16).ok())
            .collect()
    };
    let value = |b: &[u8]| b.iter().fold(0u32, |a, x| (a << 8) | u32::from(*x));
    let mut i = 0;
    let mut section = "";
    while i < tokens.len() {
        let t = tokens[i];
        match t {
            "begincodespacerange" | "begincidrange" | "begincidchar" => {
                section = t;
                i += 1;
                continue;
            }
            "endcodespacerange" | "endcidrange" | "endcidchar" => {
                section = "";
                i += 1;
                continue;
            }
            _ => {}
        }
        match section {
            "begincodespacerange" => {
                if let (Some(lo), Some(hi)) = (hex(t), tokens.get(i + 1).and_then(|t| hex(t))) {
                    if lo.len() == hi.len() && !lo.is_empty() {
                        cmap.ranges.push((lo.len(), lo, hi));
                    }
                    i += 2;
                    continue;
                }
            }
            "begincidrange" => {
                if let (Some(lo), Some(hi), Some(cid)) = (
                    hex(t),
                    tokens.get(i + 1).and_then(|t| hex(t)),
                    tokens.get(i + 2).and_then(|t| t.parse::<u32>().ok()),
                ) {
                    let (lo, hi) = (value(&lo), value(&hi));
                    if hi >= lo && hi - lo <= 65_535 {
                        for code in lo..=hi {
                            cmap.cids.insert(code, cid + (code - lo));
                        }
                    }
                    i += 3;
                    continue;
                }
            }
            "begincidchar" => {
                if let (Some(code), Some(cid)) = (
                    hex(t),
                    tokens.get(i + 1).and_then(|t| t.parse::<u32>().ok()),
                ) {
                    cmap.cids.insert(value(&code), cid);
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    cmap.ranges.sort_by_key(|r| r.0);
    cmap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmap_codespace_and_cids() {
        let cmap = parse_cmap(
            b"1 begincodespacerange <00> <80> <8140> <9ffc> endcodespacerange \
              1 begincidrange <8140> <817e> 633 endcidrange 1 begincidchar <20> 1 endcidchar",
        );
        assert_eq!(cmap.ranges.len(), 2);
        assert_eq!(code_len(&cmap.ranges, &[0x20, 0x81]), 1);
        assert_eq!(code_len(&cmap.ranges, &[0x81, 0x41]), 2);
        assert_eq!(cmap.cids.get(&0x8141), Some(&634));
        assert_eq!(cmap.cids.get(&0x20), Some(&1));
    }

    #[test]
    fn w_array_both_forms() {
        let doc = Document::with_version("1.7");
        let w = vec![
            Object::Integer(1),
            Object::Array(vec![Object::Integer(500), Object::Integer(600)]),
            Object::Integer(10),
            Object::Integer(12),
            Object::Integer(250),
        ];
        let mut out = HashMap::new();
        read_w(&doc, &w, 1, &mut out);
        assert_eq!(out.get(&2), Some(&0.6));
        assert_eq!(out.get(&12), Some(&0.25));
        assert_eq!(out.get(&13), None);
    }
}

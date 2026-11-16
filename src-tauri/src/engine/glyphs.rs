//! Glyph geometry for extensions that read mathematics (`document:layout`).
//!
//! Three things, all in display page points measured from the page's
//! top-left corner (y grows downward), which is the space the extension
//! contract states:
//!
//! - [`extract_page_glyphs`]: every character on a page with its font,
//!   size, loose box and baseline, plus the thin filled or stroked paths a
//!   TeX fraction bar or radical overbar is drawn with, as `__rule` glyphs.
//! - [`page_math_tags`]: the `Formula` elements of a tagged PDF's structure
//!   tree, with their Alt and ActualText and any MathML beneath them.
//! - [`equation_blocks`]: where a page's display equations are, found from
//!   the glyphs alone, for the viewer's "Copy LaTeX" hover button.
//!
//! The FFI is kept thin: what decides anything (rule detection, region
//! selection, MathML serialisation, equation finding) is a pure function
//! over plain data, tested without a PDF.

use crate::engine::layout_text::{char_matrix, font_info, is_bold_font_name};
use crate::engine::render::{apply, compose, Matrix, FPDF_PAGEOBJ_FORM, IDENTITY, MAX_FORM_DEPTH};
use crate::engine::tagged::{chars_by_mcid, element_type, utf16_string};
use crate::engine::text::{is_math_font, DispMapper};
use crate::engine::vision;
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_PAGE, FPDF_PAGEOBJECT, FPDF_STRUCTELEMENT, FPDF_TEXTPAGE,
    FS_MATRIX, FS_RECTF,
};
use serde::Serialize;
use std::collections::HashMap;
use std::os::raw::c_int;

/// The font name rule glyphs carry.
pub const RULE_FONT: &str = "__rule";
/// PDF font descriptor flags (PDF 1.7, table 123): Italic is bit 7,
/// ForceBold bit 19.
const ITALIC_FLAG: c_int = 1 << 6;
const FORCE_BOLD_FLAG: c_int = 1 << 18;
/// `FPDF_PAGEOBJ_PATH`.
const PAGEOBJ_PATH: c_int = 2;
/// `FPDF_PAGEOBJ_IMAGE`.
const PAGEOBJ_IMAGE: c_int = 3;
/// A path thinner than this, in points, can be a rule. A stroked bar's
/// bounds take in its line width on both sides: Acrobat Distiller's
/// fraction bars (0.53 pt lines) come out 1.05 pt thick.
const RULE_MAX_THICKNESS: f32 = 1.5;
/// Page objects examined for rules, as a backstop on a page drawn as
/// hundreds of thousands of paths (a plot).
const MAX_PAGE_OBJECTS: i32 = 50_000;

/// One glyph, or one rule, on a page.
///
/// `code` is the character code as far as PDFium will say. PDFium's text
/// API exposes no raw font code: for a character whose font has a Unicode
/// mapping, `code` is that Unicode scalar; for one without
/// (`FPDFText_HasUnicodeMapError`), PDFium reports the raw code in place of
/// the Unicode value, so `code` is the raw code and `text` is empty.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GlyphDto {
    pub text: String,
    pub code: u32,
    /// Base font name, subset prefix ("ABCDEF+") stripped; `__rule` for a
    /// fraction bar or overbar.
    pub font: String,
    /// Font size in points, scaled by the text matrix; 0 for a rule.
    pub size: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Baseline y (top-down); a rule's vertical centre.
    pub baseline: f32,
    pub bold: bool,
    pub italic: bool,
    /// The glyph's ink box `[x, y, w, h]` (top-left origin), when PDFium
    /// can say; `null` for a rule. The loose box is the font's ascent and
    /// descent, the same for every glyph of a font: only this says how tall
    /// a delimiter or a big operator is, or where a TeX extension-font
    /// glyph, hanging far below its origin, is drawn.
    pub ink: Option<[f32; 4]>,
}

impl GlyphDto {
    pub fn is_rule(&self) -> bool {
        self.font == RULE_FONT
    }
}

/// A `Formula` element of a tagged PDF.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MathTagDto {
    /// `[x, y, w, h]`, top-left origin; zeros when the element owns no
    /// characters on this page but carries an Alt or ActualText.
    pub bbox: [f32; 4],
    pub alt: Option<String>,
    pub actual_text: Option<String>,
    /// The MathML structure beneath the element, serialised. PDFium does
    /// not expose associated files, so MathML attached that way is not seen.
    pub mathml: Option<String>,
}

/// A display equation found on a page.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EquationBlockDto {
    /// `[x, y, w, h]`, top-left origin, without the equation number.
    pub bbox: [f32; 4],
    /// The equation number as printed inside its parentheses ("3", "2.4",
    /// "A.1", or a symbol tag, "*"), when there is one; for an `align`
    /// numbered row by row, its first row's.
    pub number: Option<String>,
}

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

/// Strip a subset tag: exactly six uppercase ASCII letters and a `+`; or
/// the tag Springer's tools run into the name, two random capitalised
/// six-letter words ("PwkwqfRgwrxjMTMI", "CylqyyWbfffhTimes-Roman").
pub fn strip_subset_prefix(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => {
            let mut rest = name;
            loop {
                let b = rest.as_bytes();
                let word = |at: usize| {
                    b.len() > at + 6
                        && b[at].is_ascii_uppercase()
                        && b[at + 1..at + 6].iter().all(u8::is_ascii_lowercase)
                };
                if b.len() > 12 && word(0) && word(6) && b[12].is_ascii_uppercase() {
                    rest = &rest[12..];
                } else {
                    return rest;
                }
            }
        }
    }
}

/// Whether a font's name says it is an italic or slanted face. TeX's math
/// italic (CMMI), text italic (CMTI) and slanted (CMSL) say so only by
/// family.
pub fn is_italic_font_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("italic")
        || lower.contains("oblique")
        || name.ends_with("-It")
        || ["cmmi", "cmti", "cmsl", "cmbxti", "cmbxsl", "lmromanslant"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
}

/// Every character and rule on a page, characters in PDFium's reading
/// order, rules after them.
pub fn extract_page_glyphs(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    page_w: f32,
    page_h: f32,
) -> Vec<GlyphDto> {
    let mapper = DispMapper::new(b, page, page_w, page_h);
    let mut out = Vec::new();
    let tp = b.FPDFText_LoadPage(page);
    if !tp.is_null() {
        out = char_glyphs(b, tp, &mapper, page_h);
        b.FPDFText_ClosePage(tp);
    }
    out.extend(page_rules(b, page, &mapper, page_h));
    out
}

/// A variation selector (U+FE00 to U+FE0F): which form of the character
/// before it to draw, never a character of its own.
pub fn is_variation_selector(value: u32) -> bool {
    (0xFE00..=0xFE0F).contains(&value)
}

fn char_glyphs(
    b: &dyn PdfiumLibraryBindings,
    tp: FPDF_TEXTPAGE,
    mapper: &DispMapper,
    page_h: f32,
) -> Vec<GlyphDto> {
    let count = b.FPDFText_CountChars(tp).max(0);
    let mut out = Vec::with_capacity(count as usize);
    // Font names repeat for every glyph of a run; judge each name once.
    let mut fonts: HashMap<String, (String, bool, bool)> = HashMap::new();
    let mut low_half: Option<i32> = None;
    for index in 0..count {
        if low_half == Some(index) || b.FPDFText_IsGenerated(tp, index) == 1 {
            continue;
        }
        let mut value = b.FPDFText_GetUnicode(tp, index);
        let unmapped = b.FPDFText_HasUnicodeMapError(tp, index) == 1;
        // PDFium hands a character outside the Basic Multilingual Plane (the
        // Mathematical Alphanumeric Symbols an OpenType math font sets, 𝑌)
        // as two characters, the halves of its UTF-16 surrogate pair, each
        // with the glyph's box. They are one glyph.
        if (0xD800..0xDC00).contains(&value) && index + 1 < count {
            let low = b.FPDFText_GetUnicode(tp, index + 1);
            if (0xDC00..0xE000).contains(&low) {
                value = surrogate_pair(value, low);
                low_half = Some(index + 1);
            }
        }
        let ch = char::from_u32(value);
        if matches!(ch, Some('\n' | '\r')) && !unmapped {
            continue;
        }
        // A variation selector a font's ToUnicode adds to a glyph's text
        // ("\u{2211}\u{FE01}"): PDFium gives it the glyph's box as if it
        // were a glyph of its own, which it is not.
        if is_variation_selector(value) {
            continue;
        }
        let mut loose = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        let rect = if b.FPDFText_GetLooseCharBox(tp, index, &mut loose) != 0
            && loose.right > loose.left
            && loose.top > loose.bottom
        {
            Some(mapper.map_box(
                loose.left as f64,
                loose.right as f64,
                loose.bottom as f64,
                loose.top as f64,
            ))
        } else {
            let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
            (b.FPDFText_GetCharBox(tp, index, &mut l, &mut r, &mut bo, &mut t) != 0
                && r > l
                && t > bo)
                .then(|| mapper.map_box(l, r, bo, t))
        };
        let Some((x, y_up, w, h)) = rect else {
            continue;
        };
        let top = page_h - (y_up + h);
        let ink = {
            let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
            (b.FPDFText_GetCharBox(tp, index, &mut l, &mut r, &mut bo, &mut t) != 0
                && r > l
                && t > bo)
                .then(|| {
                    let (ix, iy, iw, ih) = mapper.map_box(l, r, bo, t);
                    [ix, page_h - (iy + ih), iw, ih]
                })
        };

        let scale = char_matrix(b, tp, index).map_or(1.0, |m| (m.c * m.c + m.d * m.d).sqrt());
        let mut size = b.FPDFText_GetFontSize(tp, index) as f32 * scale;
        if !size.is_finite() || size <= 0.5 {
            size = h;
        }
        let (mut ox, mut oy) = (0f64, 0f64);
        let baseline = if b.FPDFText_GetCharOrigin(tp, index, &mut ox, &mut oy) != 0 {
            page_h - mapper.map(ox, oy).1
        } else {
            top + h
        };
        let weight = b.FPDFText_GetFontWeight(tp, index);
        let (raw_name, flags) = font_info(b, tp, index);
        let (font, named_bold, named_italic) = fonts
            .entry(raw_name)
            .or_insert_with_key(|raw| {
                let name = strip_subset_prefix(raw).to_string();
                let bold = is_bold_font_name(&name);
                let italic = is_italic_font_name(&name);
                (name, bold, italic)
            })
            .clone();

        out.push(GlyphDto {
            // A control character is no text either: PDFium reports some
            // unmapped codes (cmex's 0) as U+0000.
            text: if unmapped || ch.is_some_and(char::is_control) {
                String::new()
            } else {
                ch.map(String::from).unwrap_or_default()
            },
            code: value,
            font,
            size,
            x,
            y: top,
            w,
            h,
            baseline,
            bold: weight >= 600 || named_bold || flags & FORCE_BOLD_FLAG != 0,
            italic: named_italic || flags & ITALIC_FLAG != 0,
            ink,
        });
    }
    out
}

/// The code point a UTF-16 surrogate pair stands for.
pub fn surrogate_pair(high: u32, low: u32) -> u32 {
    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
}

/// Thin horizontal paths (and images) on the page, as rule glyphs. Paths inside form
/// XObjects count too: a page re-saved by a PDF tool is often one form
/// holding everything, the fraction bars included.
fn page_rules(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    mapper: &DispMapper,
    page_h: f32,
) -> Vec<GlyphDto> {
    let mut out = Vec::new();
    // (object, its parent form's transform to page space, depth)
    let mut stack: Vec<(FPDF_PAGEOBJECT, Matrix, u8)> = Vec::new();
    let count = b.FPDFPage_CountObjects(page).clamp(0, MAX_PAGE_OBJECTS);
    for index in (0..count).rev() {
        stack.push((b.FPDFPage_GetObject(page, index), IDENTITY, 0));
    }
    let mut seen = 0;
    while let Some((object, to_page, depth)) = stack.pop() {
        seen += 1;
        if object.is_null() || seen > MAX_PAGE_OBJECTS {
            continue;
        }
        match b.FPDFPageObj_GetType(object) {
            PAGEOBJ_PATH => {
                // A path neither filled nor stroked is a clip, and draws nothing.
                let mut fill: c_int = 0;
                let mut stroke = 0;
                if b.FPDFPath_GetDrawMode(object, &mut fill, &mut stroke) != 0
                    && fill == 0
                    && stroke == 0
                {
                    continue;
                }
            }
            // Ghostscript draws a TeX rule as a one-pixel image mask
            // stretched to the rule's size.
            PAGEOBJ_IMAGE => {}
            FPDF_PAGEOBJ_FORM if depth < MAX_FORM_DEPTH => {
                let mut m = FS_MATRIX {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                };
                let inner = if b.FPDFPageObj_GetMatrix(object, &mut m) != 0 {
                    [m.a, m.b, m.c, m.d, m.e, m.f]
                } else {
                    IDENTITY
                };
                let child_to_page = compose(&to_page, &inner);
                let children = b.FPDFFormObj_CountObjects(object).max(0);
                for i in (0..children).rev() {
                    stack.push((
                        b.FPDFFormObj_GetObject(object, i as std::os::raw::c_ulong),
                        child_to_page,
                        depth + 1,
                    ));
                }
                continue;
            }
            _ => continue,
        }
        let (mut l, mut bo, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
        if b.FPDFPageObj_GetBounds(object, &mut l, &mut bo, &mut r, &mut t) == 0 {
            continue;
        }
        // Into page space; a rule is axis-aligned, so its corners suffice.
        let corners = [apply(&to_page, l, bo), apply(&to_page, r, t)];
        let (l, r) = (
            corners[0].0.min(corners[1].0),
            corners[0].0.max(corners[1].0),
        );
        let (bo, t) = (
            corners[0].1.min(corners[1].1),
            corners[0].1.max(corners[1].1),
        );
        let (x, y_up, w, h) = mapper.map_box(l as f64, r as f64, bo as f64, t as f64);
        if let Some(rule) = rule_glyph(x, page_h - (y_up + h), w, h) {
            out.push(rule);
        }
    }
    out
}

/// A rule glyph for a path with this top-left box, if it is thin and wide
/// enough to be a fraction bar or an overbar.
pub fn rule_glyph(x: f32, y: f32, w: f32, h: f32) -> Option<GlyphDto> {
    let h = h.max(0.0);
    (h < RULE_MAX_THICKNESS && w > (h * 3.0).max(1.0)).then(|| GlyphDto {
        text: String::new(),
        code: 0,
        font: RULE_FONT.to_string(),
        size: 0.0,
        x,
        y,
        w,
        h,
        baseline: y + h / 2.0,
        bold: false,
        italic: false,
        ink: None,
    })
}

/// Where a character is, for picking it out of a region: the middle of its
/// ink when that is known and lies off its loose box's middle by more than
/// a third of its size (an extension-font glyph hanging below its origin),
/// else the middle of its loose box.
fn centre(g: &GlyphDto) -> (f32, f32) {
    let loose = (g.x + g.w / 2.0, g.y + g.h / 2.0);
    match g.ink {
        Some([x, y, w, h]) if ((y + h / 2.0) - loose.1).abs() > g.size.max(1.0) / 3.0 => {
            (x + w / 2.0, y + h / 2.0)
        }
        _ => loose,
    }
}

/// The glyphs inside any of `rects` (`[x, y, w, h]`, top-left origin): a
/// character when its box centre is inside, a rule when it touches. Order
/// is kept. No rects means the whole page.
pub fn select_in_rects(glyphs: &[GlyphDto], rects: &[[f32; 4]]) -> Vec<GlyphDto> {
    if rects.is_empty() {
        return glyphs.to_vec();
    }
    glyphs
        .iter()
        .filter(|g| {
            rects.iter().any(|&[rx, ry, rw, rh]| {
                if g.is_rule() {
                    g.x <= rx + rw && g.x + g.w >= rx && g.y <= ry + rh && g.y + g.h >= ry
                } else {
                    let (cx, cy) = centre(g);
                    cx >= rx && cx <= rx + rw && cy >= ry && cy <= ry + rh
                }
            })
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Math tags
// ---------------------------------------------------------------------------

/// One structure element, read out of PDFium into plain data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagNode {
    pub tag: String,
    pub alt: Option<String>,
    pub actual_text: Option<String>,
    /// Marked-content ids this element owns directly.
    pub mcids: Vec<i32>,
    pub children: Vec<TagNode>,
}

const MAX_TAG_DEPTH: usize = 48;
const MAX_TAG_ELEMENTS: usize = 20_000;

/// The `Formula` elements on a page. Empty for an untagged document, which
/// is most of them.
pub fn page_math_tags(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    page_w: f32,
    page_h: f32,
) -> Vec<MathTagDto> {
    let tree = b.FPDF_StructTree_GetForPage(page);
    if tree.is_null() {
        return Vec::new();
    }
    let mut budget = MAX_TAG_ELEMENTS;
    let mut roots = Vec::new();
    for i in 0..b.FPDF_StructTree_CountChildren(tree) {
        let child = b.FPDF_StructTree_GetChildAtIndex(tree, i);
        if !child.is_null() {
            if let Some(node) = read_tag(b, child, 0, &mut budget) {
                roots.push(node);
            }
        }
    }
    b.FPDF_StructTree_Close(tree);
    // A tagged page with no formula is the common case: its text is not
    // worth loading and boxing char by char.
    if !has_formula(&roots) {
        return Vec::new();
    }

    let tp = b.FPDFText_LoadPage(page);
    if tp.is_null() {
        return collect_formulas(&roots, &HashMap::new(), &[]);
    }
    let by_mcid = chars_by_mcid(b, tp);
    let mapper = DispMapper::new(b, page, page_w, page_h);
    let count = b.FPDFText_CountChars(tp).max(0);
    let mut chars = Vec::with_capacity(count as usize);
    for index in 0..count {
        let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
        let ch = char::from_u32(b.FPDFText_GetUnicode(tp, index)).unwrap_or(' ');
        let rect = (b.FPDFText_GetCharBox(tp, index, &mut l, &mut r, &mut bo, &mut t) != 0
            && r > l
            && t > bo)
            .then(|| {
                let (x, y, w, h) = mapper.map_box(l, r, bo, t);
                [x, page_h - (y + h), w, h]
            });
        chars.push((ch, rect));
    }
    b.FPDFText_ClosePage(tp);
    collect_formulas(&roots, &by_mcid, &chars)
}

fn non_empty(text: String) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn read_tag(
    b: &dyn PdfiumLibraryBindings,
    element: FPDF_STRUCTELEMENT,
    depth: usize,
    budget: &mut usize,
) -> Option<TagNode> {
    if depth > MAX_TAG_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    let mut node = TagNode {
        tag: element_type(b, element),
        alt: non_empty(utf16_string(|buffer, len| {
            b.FPDF_StructElement_GetAltText(element, buffer, len)
        })),
        actual_text: non_empty(utf16_string(|buffer, len| {
            b.FPDF_StructElement_GetActualText(element, buffer, len)
        })),
        ..TagNode::default()
    };
    for i in 0..b.FPDF_StructElement_CountChildren(element) {
        let child = b.FPDF_StructElement_GetChildAtIndex(element, i);
        if child.is_null() {
            let mcid = b.FPDF_StructElement_GetChildMarkedContentID(element, i);
            if mcid >= 0 {
                node.mcids.push(mcid);
            }
        } else if let Some(child) = read_tag(b, child, depth + 1, budget) {
            node.children.push(child);
        }
    }
    Some(node)
}

/// Formula elements in a structure tree, with their geometry and MathML.
///
/// A `Formula` is reported with its own Alt and ActualText; a MathML `math`
/// root that sits under no `Formula` (LaTeX's tagging can emit one bare) is
/// reported the same way. `chars` is the page's characters by index, each
/// with its top-left box when it has one.
/// Whether any element under `nodes` is a formula `collect_formulas` reads.
pub fn has_formula(nodes: &[TagNode]) -> bool {
    nodes
        .iter()
        .any(|n| n.tag == "Formula" || n.tag == "math" || has_formula(&n.children))
}

pub fn collect_formulas(
    roots: &[TagNode],
    by_mcid: &HashMap<i32, Vec<u32>>,
    chars: &[(char, Option<[f32; 4]>)],
) -> Vec<MathTagDto> {
    let mut out = Vec::new();
    for root in roots {
        collect_formula(root, by_mcid, chars, &mut out);
    }
    out
}

fn collect_formula(
    node: &TagNode,
    by_mcid: &HashMap<i32, Vec<u32>>,
    chars: &[(char, Option<[f32; 4]>)],
    out: &mut Vec<MathTagDto>,
) {
    if node.tag == "Formula" || node.tag == "math" {
        let mut indices = Vec::new();
        owned_chars(node, by_mcid, &mut indices);
        let bbox = union(indices.iter().filter_map(|&i| chars.get(i as usize)?.1));
        let mathml = find_math(node).map(|math| {
            let mut xml = String::new();
            write_mathml(math, by_mcid, chars, true, &mut xml);
            xml
        });
        if bbox.is_some() || node.alt.is_some() || node.actual_text.is_some() {
            out.push(MathTagDto {
                bbox: bbox.unwrap_or([0.0; 4]),
                alt: node.alt.clone(),
                actual_text: node.actual_text.clone(),
                mathml,
            });
        }
        return;
    }
    for child in &node.children {
        collect_formula(child, by_mcid, chars, out);
    }
}

fn owned_chars(node: &TagNode, by_mcid: &HashMap<i32, Vec<u32>>, out: &mut Vec<u32>) {
    for mcid in &node.mcids {
        if let Some(chars) = by_mcid.get(mcid) {
            out.extend(chars.iter().copied());
        }
    }
    for child in &node.children {
        owned_chars(child, by_mcid, out);
    }
}

fn find_math(node: &TagNode) -> Option<&TagNode> {
    if node.tag == "math" {
        return Some(node);
    }
    node.children.iter().find_map(find_math)
}

fn union(boxes: impl Iterator<Item = [f32; 4]>) -> Option<[f32; 4]> {
    boxes.fold(None, |acc, [x, y, w, h]| {
        Some(match acc {
            None => [x, y, w, h],
            Some([ax, ay, aw, ah]) => {
                let (l, t) = (ax.min(x), ay.min(y));
                let (r, b) = ((ax + aw).max(x + w), (ay + ah).max(y + h));
                [l, t, r - l, b - t]
            }
        })
    })
}

/// An element name safe to write as an XML tag; anything else becomes
/// `mrow`, which keeps its children without claiming a meaning.
fn xml_name(tag: &str) -> &str {
    let ok = !tag.is_empty()
        && tag.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        tag
    } else {
        "mrow"
    }
}

pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn write_mathml(
    node: &TagNode,
    by_mcid: &HashMap<i32, Vec<u32>>,
    chars: &[(char, Option<[f32; 4]>)],
    root: bool,
    out: &mut String,
) {
    let name = xml_name(&node.tag);
    out.push('<');
    out.push_str(name);
    if root {
        out.push_str(" xmlns=\"http://www.w3.org/1998/Math/MathML\"");
    }
    out.push('>');
    if node.children.is_empty() {
        let mut indices = Vec::new();
        owned_chars(node, by_mcid, &mut indices);
        indices.sort_unstable();
        let text: String = indices
            .iter()
            .filter_map(|&i| chars.get(i as usize).map(|(c, _)| *c))
            .collect();
        out.push_str(&xml_escape(text.trim()));
    } else {
        for child in &node.children {
            write_mathml(child, by_mcid, chars, false, out);
        }
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

// ---------------------------------------------------------------------------
// Display equations
// ---------------------------------------------------------------------------

/// Characters that mark a line as mathematics wherever they are set.
const BIG_OPERATORS: &[char] = &[
    '\u{2211}', // N-ARY SUMMATION
    '\u{220F}', // N-ARY PRODUCT
    '\u{2210}', // N-ARY COPRODUCT
    '\u{222B}', // INTEGRAL
    '\u{222C}', // DOUBLE INTEGRAL
    '\u{222D}', // TRIPLE INTEGRAL
    '\u{222E}', // CONTOUR INTEGRAL
    '\u{22C3}', // N-ARY UNION
    '\u{22C2}', // N-ARY INTERSECTION
    '\u{2A01}', // N-ARY CIRCLED PLUS
    '\u{2A02}', // N-ARY CIRCLED TIMES
    '\u{2A00}', // N-ARY CIRCLED DOT
    '\u{221A}', // SQUARE ROOT
];

/// Whether a font is set aside for mathematics: TeX's math families
/// (`text::is_math_font`), the AMS and Euler extras, and the OpenType math
/// fonts Word and XeTeX use (Cambria Math, STIX Two Math, Latin Modern
/// Math, XITS Math), which all say "Math" in their names.
pub fn is_math_font_name(name: &str) -> bool {
    if is_math_font(name) {
        return true;
    }
    let upper = name.to_ascii_uppercase();
    upper.contains("MATH")
        || [
            "EUFM", "EUSM", "EUEX", "RSFS", "CMBSY", "STMARY", "ESINT", "MTEXTRA", "MT-EXTRA",
            // MathTime (Springer, and RMTMI in some LaTeX journals).
            "MTMI", "MTSY", "MTEX", "RMTMI",
        ]
        .iter()
        .any(|family| upper.starts_with(family))
        // Symbol, SymbolMT (Word), MTSymbol (MathType).
        || (upper.contains("SYMBOL") && !upper.contains("APPLE"))
}

fn is_big_operator(g: &GlyphDto) -> bool {
    g.font.to_ascii_uppercase().starts_with("CMEX")
        || g.text.chars().any(|c| BIG_OPERATORS.contains(&c))
}

/// One character's vertical core: the band from a little under its
/// baseline to about its cap height, from its size rather than its box,
/// since a math font's stated ascent and descent can be anything.
fn core(g: &GlyphDto) -> (f32, f32) {
    if let (Some([_, y, _, h]), true) = (g.ink, is_big_operator(g)) {
        return (y, y + h);
    }
    let size = g.size.max(1.0);
    (g.baseline - 0.75 * size, g.baseline + 0.25 * size)
}

/// A horizontal run of a row's glyphs with no wide gap in it.
struct Segment<'a> {
    glyphs: Vec<&'a GlyphDto>,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    em: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Math,
    Number,
    Text,
}

impl Segment<'_> {
    fn text(&self) -> String {
        self.glyphs.iter().map(|g| g.text.as_str()).collect()
    }

    fn width(&self) -> f32 {
        self.right - self.left
    }
}

/// `(3)`, `(2.4)`, `(A.1)`, `(3a)`: what an equation number looks like.
/// Returns what is inside the parentheses.
fn equation_number(text: &str) -> Option<String> {
    let inner = text.trim().strip_prefix('(')?.strip_suffix(')')?;
    let ok = !inner.is_empty()
        && inner.len() <= 12
        && inner.chars().any(|c| c.is_ascii_digit())
        && inner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '\'' | '*'));
    // Or a symbol tag, `\tag{$\star$}`: "(*)", "(**)", "(dagger)".
    let symbol = (1..=3).contains(&inner.chars().count())
        && inner.chars().all(|c| {
            matches!(
                c,
                '*' | '\u{22C6}' | '\u{2217}' | '\u{2020}' | '\u{2021}' | '\u{2032}' | '\''
            )
        });
    (ok || symbol).then(|| inner.to_string())
}

/// A super- or subscript: letters set small ("eq" in a superscript, "max"
/// under an operator) label mathematics, and are no words of prose.
fn is_script(g: &GlyphDto, em: f32) -> bool {
    g.size < 0.85 * em
}

/// Words of prose among some glyphs: runs of three or more full-size
/// letters in the text font, set close on one baseline, that are no
/// operator names.
fn prose_words(glyphs: &[&GlyphDto], em: f32) -> usize {
    let mut sorted: Vec<&GlyphDto> = glyphs.to_vec();
    sorted.sort_by(|a, b| {
        (a.baseline / (0.3 * em))
            .round()
            .total_cmp(&(b.baseline / (0.3 * em)).round())
            .then(a.x.total_cmp(&b.x))
    });
    let operators = operator_letters(&sorted);
    // Scripts against the glyphs' own text size: a footnote's words are
    // words.
    let main = sorted.iter().map(|g| g.size).fold(0.0, f32::max);
    let letter = |g: &GlyphDto| {
        !is_math_font_name(&g.font)
            && !g.text.is_empty()
            && g.text.chars().all(char::is_alphabetic)
            && !is_script(g, main)
    };
    let mut words = 0;
    let mut run = 0;
    for (i, g) in sorted.iter().enumerate() {
        let joins = i > 0 && {
            let p = sorted[i - 1];
            (p.baseline - g.baseline).abs() < 0.1 * em && g.x - (p.x + p.w) < 0.15 * em
        };
        if letter(g) && !operators[i] {
            run = if joins { run + 1 } else { 1 };
            if run == 3 {
                words += 1;
            }
        } else {
            run = 0;
        }
    }
    words
}

/// Operator names, set upright in the text font: mathematics, not words.
const OPERATOR_NAMES: &[&str] = &[
    "arccos", "arcsin", "arctan", "arg", "argmax", "argmin", "cos", "cosh", "cot", "coth", "csc",
    "deg", "det", "diag", "diam", "dim", "dist", "exp", "gcd", "hom", "inf", "ker", "lcm", "lg",
    "lim", "liminf", "limsup", "ln", "log", "max", "min", "mod", "Pr", "sec", "sin", "sinh", "sup",
    "supp", "tan", "tanh", "tr", "Tr", "Var", "Cov", "var", "cov", "rank", "sgn", "sign", "span",
    "vol", "poly", "polylog", "softmax", "KL", "TV", "err", "Ber", "Bern", "Bin", "Unif", "Lap",
];

/// Which of a left-to-right run of glyphs spell an operator name: a run of
/// text-font letters set close on one baseline, "log", "sup", "Pr".
fn operator_letters(glyphs: &[&GlyphDto]) -> Vec<bool> {
    let mut out = vec![false; glyphs.len()];
    let letter = |g: &GlyphDto| {
        !is_math_font_name(&g.font) && g.text.chars().all(char::is_alphabetic) && !g.text.is_empty()
    };
    let mut at = 0;
    while at < glyphs.len() {
        if !letter(glyphs[at]) {
            at += 1;
            continue;
        }
        let start = at;
        let mut word = glyphs[at].text.clone();
        while at + 1 < glyphs.len() && letter(glyphs[at + 1]) {
            let (p, g) = (glyphs[at], glyphs[at + 1]);
            let em = p.size.max(1.0);
            if (p.baseline - g.baseline).abs() > 0.1 * em || g.x - (p.x + p.w) > 0.15 * em {
                break;
            }
            word.push_str(&g.text);
            at += 1;
        }
        at += 1;
        if OPERATOR_NAMES.contains(&word.as_str()) {
            for flag in &mut out[start..at] {
                *flag = true;
            }
        }
    }
    out
}

fn classify(segment: &Segment, rules: &[&GlyphDto]) -> Kind {
    let text = segment.text();
    if equation_number(&text).is_some() {
        return Kind::Number;
    }
    let n = segment.glyphs.len() as f32;
    let math = segment
        .glyphs
        .iter()
        .filter(|g| is_math_font_name(&g.font))
        .count() as f32;
    let operators = operator_letters(&segment.glyphs);
    let letters = segment
        .glyphs
        .iter()
        .zip(&operators)
        .filter(|(g, op)| {
            !**op
                && !is_math_font_name(&g.font)
                && g.text.chars().any(char::is_alphabetic)
                && !is_script(g, largest(segment))
        })
        .count() as f32;
    let big = segment.glyphs.iter().any(|g| is_big_operator(g));
    let bar = rules.iter().any(|r| {
        let centre = r.x + r.w / 2.0;
        centre >= segment.left
            && centre <= segment.right
            && r.w >= 0.5 * segment.em
            && r.baseline >= segment.top - 0.5 * segment.em
            && r.baseline <= segment.bottom + 0.5 * segment.em
    });
    // "0 =" set in the text font: no letters, and an operator.
    let operator = letters == 0.0
        && segment.glyphs.iter().any(|g| {
            g.text
                .chars()
                .any(|c| "=+<>\u{2212}\u{2264}\u{2265}".contains(c))
        });
    let mathy = big
        || operator
        || (bar && letters / n <= 0.5)
        || math / n >= 0.5
        || (math >= 1.0 && letters / n <= 0.5);
    if mathy {
        Kind::Math
    } else {
        Kind::Text
    }
}

fn median(mut values: Vec<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

/// Group characters into rows: a character joins a row whose largest
/// character's core it overlaps by half of the smaller core, so scripts
/// stay on their line and a fraction's numerator and denominator get rows
/// of their own. Larger characters are placed first, so a line exists
/// before its scripts come to it; a script that could join two rows (the
/// columns of a two-column page are rarely level) joins the one with a
/// character nearest it.
fn rows(chars: &[&GlyphDto]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..chars.len()).collect();
    // Tall delimiters and operators last: a row is a line of text, and one
    // started by a tall glyph would take in every line it spans.
    let tall = |g: &GlyphDto| {
        let (top, bottom) = core(g);
        bottom - top > 1.5 * g.size.max(1.0)
    };
    order.sort_by(|&a, &b| {
        tall(chars[a])
            .cmp(&tall(chars[b]))
            .then(chars[b].size.total_cmp(&chars[a].size))
            .then(chars[a].baseline.total_cmp(&chars[b].baseline))
    });
    // (reference core, members), kept sorted by the reference's top.
    let mut rows: Vec<((f32, f32), Vec<usize>)> = Vec::new();
    for index in order {
        let g = chars[index];
        let (top, bottom) = core(g);
        let reach = 2.0 * (bottom - top);
        // Rows are sorted by top; only those whose core can overlap this one.
        let from = rows.partition_point(|((rt, _), _)| *rt < top - reach);
        let em = g.size.max(1.0);
        let mut best: Option<(usize, (bool, f32))> = None;
        for (at, ((rt, rb), members)) in rows.iter().enumerate().skip(from) {
            if *rt > bottom {
                break;
            }
            let overlap = bottom.min(*rb) - top.max(*rt);
            let smaller = (bottom - top).min(rb - rt);
            if overlap < 0.5 * smaller {
                continue;
            }
            let gap = members
                .iter()
                .map(|&m| {
                    let o = chars[m];
                    (o.x - (g.x + g.w)).max(g.x - (o.x + o.w)).max(0.0)
                })
                .fold(f32::MAX, f32::min);
            let near = gap < 4.0 * em;
            // A row with a character on this very baseline is this line.
            let exact = members
                .iter()
                .any(|&m| (chars[m].baseline - g.baseline).abs() < 0.1 * em);
            let level = near && exact;
            // A character as large as the row's that is level with it only
            // in part belongs to another line: the columns of a two-column
            // page are seldom level, and their gutter is narrow. A smaller
            // one is a script, raised or lowered, of a line near it; far off
            // (a label under a brace, level with the other column's text)
            // it is not.
            let row_size = members.iter().map(|&m| chars[m].size).fold(0.0, f32::max);
            let script = g.size < 0.85 * row_size;
            let fits = tall(g)
                || (near && (exact || overlap > 0.8 * smaller || script))
                || (!near && !script && (exact || overlap > 0.8 * smaller));
            if !fits {
                continue;
            }
            // A tall glyph is centred on its line's axis: among the rows
            // near it, the one whose middle is nearest its middle.
            let key = if tall(g) {
                (near, -((rt + rb) / 2.0 - (top + bottom) / 2.0).abs())
            } else {
                (level, -gap)
            };
            if best.is_none_or(|(_, b_key)| key > b_key) {
                best = Some((at, key));
            }
        }
        match best {
            Some((at, ..)) => rows[at].1.push(index),
            None => {
                let at = rows.partition_point(|((rt, _), _)| *rt < top);
                rows.insert(at, ((top, bottom), vec![index]));
            }
        }
    }
    // A row of nothing but bare extension-font glyphs (a \Big( and its
    // \Big) set before the line they belong to existed) joins that line.
    let mut at = 0;
    while at < rows.len() {
        let bare_row = rows[at].1.iter().all(|&m| chars[m].text.is_empty());
        let ((rt, rb), _) = rows[at];
        let target = bare_row
            .then(|| {
                (0..rows.len()).find(|&other| {
                    let ((ot, ob), members) = &rows[other];
                    other != at
                        && !members.iter().all(|&m| chars[m].text.is_empty())
                        && rb.min(*ob) - rt.max(*ot) >= 0.5 * (rb - rt).min(ob - ot)
                        && rows[at].1.iter().any(|&b| {
                            let g = chars[b];
                            members.iter().any(|&m| {
                                let o = chars[m];
                                (o.x - (g.x + g.w)).max(g.x - (o.x + o.w)) < 4.0 * g.size.max(1.0)
                            })
                        })
                })
            })
            .flatten();
        match target {
            Some(other) => {
                let moved = std::mem::take(&mut rows[at].1);
                rows[other].1.extend(moved);
                rows.remove(at);
            }
            None => at += 1,
        }
    }
    rows.into_iter().map(|(_, members)| members).collect()
}

/// Split a row at gaps wider than 1.2 em into segments, left to right.
fn segments<'a>(chars: &[&'a GlyphDto], row: &[usize], gutters: &[(f32, f32)]) -> Vec<Segment<'a>> {
    let mut members: Vec<&GlyphDto> = row.iter().map(|&i| chars[i]).collect();
    members.sort_by(|a, b| a.x.total_cmp(&b.x));
    let em = median(members.iter().map(|g| g.size).collect()).max(1.0);
    let mut out: Vec<Segment> = Vec::new();
    for g in members {
        let (top, bottom) = core(g);
        match out.last_mut() {
            // Nor across a column gutter, however narrow: a space there
            // wider than a display's own spaces parts two columns' lines.
            Some(segment)
                if g.x - segment.right <= 1.2 * em
                    && !(g.x - segment.right >= 0.5 * em
                        && gutters
                            .iter()
                            .any(|&(g0, g1)| segment.right <= g1 && g.x >= g0)) =>
            {
                segment.glyphs.push(g);
                segment.right = segment.right.max(g.x + g.w);
                segment.top = segment.top.min(top);
                segment.bottom = segment.bottom.max(bottom);
            }
            _ => out.push(Segment {
                glyphs: vec![g],
                left: g.x,
                right: g.x + g.w,
                top,
                bottom,
                em,
            }),
        }
    }
    out.into_iter().flat_map(split_number).collect()
}

/// A segment ending in an equation number set a little apart, "... x(27)",
/// as two: the equation and its number.
fn split_number<'a>(segment: Segment<'a>) -> Vec<Segment<'a>> {
    let glyphs = &segment.glyphs;
    let sep = TAG_SEP_EMS * segment.em;
    // Trailing, "... x  (27)"; or leading, "(27)  x ..." (`leqno`).
    let trailing = glyphs
        .iter()
        .rposition(|g| g.text == "(")
        .filter(|&at| at > 0)
        .filter(|&at| {
            let tail: String = glyphs[at..].iter().map(|g| g.text.as_str()).collect();
            let before = glyphs[at - 1];
            equation_number(&tail).is_some() && glyphs[at].x - (before.x + before.w) >= sep
        });
    let leading = glyphs
        .iter()
        .position(|g| g.text == ")")
        .filter(|&close| glyphs[0].text == "(" && close + 1 < glyphs.len())
        .filter(|&close| {
            let head: String = glyphs[..=close].iter().map(|g| g.text.as_str()).collect();
            let end = glyphs[close].x + glyphs[close].w;
            equation_number(&head).is_some() && glyphs[close + 1].x - end >= sep
        })
        .map(|close| close + 1);
    let Some(at) = trailing.or(leading) else {
        return vec![segment];
    };
    let part = |members: &[&'a GlyphDto]| -> Option<Segment<'a>> {
        let first = members.first()?;
        let mut out = Segment {
            glyphs: vec![first],
            left: first.x,
            right: first.x + first.w,
            top: core(first).0,
            bottom: core(first).1,
            em: segment.em,
        };
        for g in &members[1..] {
            let (top, bottom) = core(g);
            out.glyphs.push(g);
            out.right = out.right.max(g.x + g.w);
            out.top = out.top.min(top);
            out.bottom = out.bottom.max(bottom);
        }
        Some(out)
    };
    let (head, number) = glyphs.split_at(at);
    [part(head), part(number)].into_iter().flatten().collect()
}

fn bare(segment: &Segment) -> usize {
    segment
        .glyphs
        .iter()
        .filter(|g| g.text.is_empty() && is_big_operator(g))
        .count()
}

fn largest(segment: &Segment) -> f32 {
    segment.glyphs.iter().map(|g| g.size).fold(0.0, f32::max)
}

/// Letters and digits in a segment: a block of nothing but marks (a column
/// of footnote asterisks) is no equation.
fn alnum(segment: &Segment) -> usize {
    segment
        .glyphs
        .iter()
        .filter(|g| g.text.chars().any(char::is_alphanumeric))
        .count()
}

/// A glyph's box for an equation's outline: its loose box, held to the band
/// from about its cap height to its descenders, since a math font's loose
/// box can reach far past the ink, and an outline reaching up into the line
/// of text above would take that line's characters into the equation. Big
/// operators and delimiters keep their ink whole.
fn outline_box(g: &GlyphDto) -> [f32; 4] {
    if is_big_operator(g) {
        return g.ink.unwrap_or([g.x, g.y, g.w, g.h]);
    }
    let size = g.size.max(1.0);
    let top = g.y.max(g.baseline - 0.8 * size);
    let bottom = (g.y + g.h).min(g.baseline + 0.3 * size).max(top);
    [g.x, top, g.w, bottom - top]
}

/// The most ink between the pieces of one stack (a numerator and its bar,
/// a sum and its limits), in ems.
const STACK_GAP_EMS: f32 = 0.6;
/// The most ink between two rows of one display (the lines of an `align`,
/// of a `multline`), in ems: consecutive displays are further apart, by the
/// skips set above and below each.
const ROW_GAP_EMS: f32 = 1.0;
/// The same between the raster's lines, whose ink takes in descenders,
/// tall delimiters and limits that the glyphs' outline leaves out.
const INK_ROW_GAP_EMS: f32 = 1.0;
/// The least space before an equation number, in ems: amsmath's
/// `\mintagsep` is half a quad.
const TAG_SEP_EMS: f32 = 0.45;

struct Candidate {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    em: f32,
    boxes: Vec<[f32; 4]>,
    /// Characters in it; a block of nothing but a rule is an underline.
    chars: usize,
    /// Of those, TeX extension-font glyphs with no text (a row of nothing
    /// but wide hats over running text is no display).
    bare: usize,
    /// Of those, letters and digits.
    alnum: usize,
    /// The largest character in it: a block of nothing but script-size
    /// characters is a stray superscript, not a display.
    largest: f32,
    number: Option<String>,
    /// Where its relation symbols stand: the alignment points an `align`
    /// lines its rows up on.
    relations: Vec<f32>,
    /// Where its stretches after a wide space start: the columns of an
    /// `alignat`.
    columns: Vec<f32>,
}

impl Candidate {
    /// Whether two candidates are one equation: stacked closely with their
    /// widths overlapping (the lines of an `align`, a numerator over its
    /// bar), or level with each other a little apart (`f(x) =` beside the
    /// fraction it equals), or on one line however far apart (the columns
    /// of an `align` set side by side).
    fn joins(&self, other: &Candidate, reach: f32) -> bool {
        let em = self.em.max(other.em);
        let v_gap = (other.top - self.bottom).max(self.top - other.bottom);
        let h_gap = (other.left - self.right).max(self.left - other.right);
        let level = -v_gap >= 0.6 * (self.bottom - self.top).min(other.bottom - other.top)
            && self.chars.min(other.chars) >= 3;
        (v_gap <= reach * em && h_gap <= 1.0 * em)
            || (v_gap < 0.0 && h_gap <= 4.0 * em)
            || (level && h_gap <= 15.0 * em)
    }

    fn absorb(&mut self, other: Candidate) {
        self.left = self.left.min(other.left);
        self.right = self.right.max(other.right);
        self.top = self.top.min(other.top);
        self.bottom = self.bottom.max(other.bottom);
        self.em = self.em.max(other.em);
        self.boxes.extend(other.boxes);
        self.chars += other.chars;
        self.bare += other.bare;
        self.alnum += other.alnum;
        self.largest = self.largest.max(other.largest);
        self.relations.extend(other.relations);
        self.columns.extend(other.columns);
        if self.number.is_none() {
            self.number = other.number;
        }
    }

    /// Whether two candidates share an alignment point: a relation symbol
    /// at the same place, as the rows of an `align` have.
    fn aligned_with(&self, other: &Candidate) -> bool {
        let near = |a: &f32, b: &f32| (a - b).abs() < 1.0;
        self.relations
            .iter()
            .any(|a| other.relations.iter().any(|b| near(a, b)))
            || self
                .columns
                .iter()
                .any(|a| near(a, &other.left) || other.columns.iter().any(|b| near(a, b)))
            || other.columns.iter().any(|b| near(b, &self.left))
    }
}

/// A line of running text between two candidates keeps them apart: two
/// displays a sentence apart are two equations.
type Barrier = (f32, f32, f32, f32);

fn separated(a: &Candidate, b: &Candidate, barriers: &[Barrier]) -> bool {
    let (upper, lower) = if a.top <= b.top { (a, b) } else { (b, a) };
    barriers.iter().any(|&(left, right, top, bottom)| {
        let middle = (top + bottom) / 2.0;
        middle > upper.bottom
            && middle < lower.top
            && left < upper.right.min(lower.right)
            && right > upper.left.max(lower.left)
    })
}

/// Whether a column gutter runs between two candidates: one wholly on
/// each side of it.
fn across_gutter(a: (f32, f32), b: (f32, f32), gutters: &[(f32, f32)]) -> bool {
    match (
        column_side(a.0, a.1, gutters),
        column_side(b.0, b.1, gutters),
    ) {
        (Some(x), Some(y)) => x != y,
        _ => false,
    }
}

/// Which column a stretch from `x0` to `x1` is in, counted across the
/// gutters; `None` for one set across a gutter, from the column before it
/// into the column after. Reaching into a gutter is not crossing it: a
/// display can run a little past its column's edge.
fn column_side(x0: f32, x1: f32, gutters: &[(f32, f32)]) -> Option<usize> {
    let mut side = 0;
    for &(g0, g1) in gutters {
        if x0 < g0 - 0.5 && x1 > g1 + 0.5 {
            return None;
        }
        if (x0 + x1) / 2.0 >= (g0 + g1) / 2.0 {
            side += 1;
        }
    }
    Some(side)
}

/// How far a number stands from a row of mathematics beside it, right of
/// it or (`leqno`) left of it; `None` when it is not beside it.
fn tag_gap(segment: &Segment, c: &Candidate) -> Option<f32> {
    if segment.left > c.right {
        Some(segment.left - c.right)
    } else if segment.right < c.left {
        Some(c.left - segment.right)
    } else {
        None
    }
}

/// An equation number's box, `(left, right, top, bottom)`.
type Tag = (f32, f32, f32, f32);

/// Whether a number set on a line of its own lies between two rows: an
/// `align` row too long for its number pushes the number down a line,
/// and the next row with it.
fn bridged(a: &Candidate, b: &Candidate, reach: f32, tags: &[Tag]) -> bool {
    let (upper, lower) = if a.top <= b.top { (a, b) } else { (b, a) };
    let em = a.em.max(b.em);
    let h_gap = (b.left - a.right).max(a.left - b.right);
    h_gap <= em
        && tags.iter().any(|&(left, _, top, bottom)| {
            top >= upper.bottom - 0.2 * em
                && bottom <= lower.top + 0.2 * em
                && top - upper.bottom <= reach * em
                && lower.top - bottom <= reach * em
                && left > upper.left.max(lower.left)
        })
}

/// Whether a candidate is level with a line of prose that runs across it.
fn inline_with(c: &Candidate, barriers: &[Barrier]) -> bool {
    barriers.iter().any(|&(left, right, top, bottom)| {
        left <= c.left
            && right >= c.right
            && c.bottom.min(bottom) - c.top.max(top) > 0.3 * (c.bottom - c.top)
    })
}

/// Merge candidates until no two join. Two candidates in different
/// columns never join; nor do two equations each with its own number,
/// unless they line up on a relation as the rows of an `align` do.
fn merge_candidates(
    mut pending: Vec<Candidate>,
    barriers: &[Barrier],
    gutters: &[(f32, f32)],
    tags: &[Tag],
    reach: f32,
) -> Vec<Candidate> {
    // How far apart two candidates that may join are, when they may: the
    // nearest pair is merged first, so a sum's limits go to the sum and not
    // to the equation set close under it.
    let distance = |a: &Candidate, b: &Candidate| -> Option<f32> {
        let allowed = (a.joins(b, reach) || bridged(a, b, reach, tags))
            && !separated(a, b, barriers)
            && !across_gutter((a.left, a.right), (b.left, b.right), gutters)
            && !(a.number.is_some() && b.number.is_some() && !a.aligned_with(b));
        let em = a.em.max(b.em);
        let v_gap = (b.top - a.bottom).max(a.top - b.bottom).max(0.0);
        let h_gap = (b.left - a.right).max(a.left - b.right).max(0.0);
        allowed.then_some((v_gap + 0.1 * h_gap) / em)
    };
    let n = pending.len();
    let mut alive: Vec<Option<Candidate>> = pending.drain(..).map(Some).collect();
    let mut gaps: Vec<Vec<Option<f32>>> = vec![vec![None; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let d = distance(
                alive[i].as_ref().expect("alive"),
                alive[j].as_ref().expect("alive"),
            );
            gaps[i][j] = d;
            gaps[j][i] = d;
        }
    }
    loop {
        let mut best: Option<(f32, usize, usize)> = None;
        for (i, row) in gaps.iter().enumerate() {
            for (j, d) in row.iter().enumerate().skip(i + 1) {
                if let Some(d) = *d {
                    if best.is_none_or(|(b, ..)| d < b) {
                        best = Some((d, i, j));
                    }
                }
            }
        }
        let Some((_, i, j)) = best else { break };
        let other = alive[j].take().expect("alive");
        alive[i].as_mut().expect("alive").absorb(other);
        gaps[j].fill(None);
        for row in gaps.iter_mut() {
            row[j] = None;
        }
        for k in 0..n {
            if k == i {
                continue;
            }
            let d = match (&alive[i], &alive[k]) {
                (Some(a), Some(b)) => distance(a, b),
                _ => None,
            };
            gaps[i][k] = d;
            gaps[k][i] = d;
        }
    }
    let mut out: Vec<Candidate> = alive.into_iter().flatten().collect();
    out.sort_by(|a, b| a.top.total_cmp(&b.top));
    out
}

/// The gutters between a page's text columns, `(left, right)`: stretches
/// across the page its characters leave empty from top to bottom but for a
/// few lines set across them (a title, a full-width display), with text on
/// either side. None on a page of one column.
fn gutters(chars: &[&GlyphDto], page_w: f32) -> Vec<(f32, f32)> {
    let n = page_w.ceil().max(1.0) as usize;
    let mut cover = vec![0f32; n];
    // Of it, digits: a review copy's margin of line numbers.
    let mut digits = vec![0f32; n];
    for g in chars {
        let a = (g.x.max(0.0) as usize).min(n);
        let b = ((g.x + g.w).ceil().max(0.0) as usize).min(n);
        let digit = g.text.chars().all(|c| c.is_ascii_digit()) && !g.text.is_empty();
        for c in &mut cover[a..b] {
            *c += g.size.max(1.0);
        }
        if digit {
            for c in &mut digits[a..b] {
                *c += g.size.max(1.0);
            }
        }
    }
    let total: f32 = cover.iter().sum();
    let mut filled: Vec<f32> = cover.iter().copied().filter(|&c| c > 0.0).collect();
    if filled.is_empty() {
        return Vec::new();
    }
    filled.sort_by(f32::total_cmp);
    let peak = filled[filled.len() * 9 / 10];
    let em = median(chars.iter().map(|g| g.size).collect()).max(1.0);
    let mut out = Vec::new();
    let mut x = 0;
    while x < n {
        if cover[x] > 0.2 * peak {
            x += 1;
            continue;
        }
        let start = x;
        while x < n && cover[x] <= 0.2 * peak {
            x += 1;
        }
        let width = (x - start) as f32;
        let left: f32 = cover[..start].iter().sum();
        let right: f32 = cover[x..].iter().sum();
        let numbers = |range: std::ops::Range<usize>, mass: f32| {
            mass > 0.0 && digits[range].iter().sum::<f32>() >= 0.9 * mass
        };
        let sides = (left >= 0.2 * total || numbers(0..start, left))
            && (right >= 0.2 * total || numbers(x..n, right));
        if width >= 0.5 * em && width <= 0.15 * page_w && sides {
            out.push((start as f32, x as f32));
        }
    }
    out
}

/// Relation symbols: where the rows of an `align` line up.
fn is_relation(g: &GlyphDto) -> bool {
    g.text.chars().count() == 1
        && g.text.chars().all(|c| {
            "=<>\u{2264}\u{2265}\u{2248}\u{2261}\u{221D}\u{2243}\u{2245}\u{2260}\u{226A}\u{226B}\u{227A}\u{227B}\u{2AAF}\u{2AB0}\u{2192}\u{2190}\u{21D2}\u{21D4}\u{2208}\u{2286}\u{2282}"
                .contains(c)
        })
}

fn relations(segment: &Segment) -> Vec<f32> {
    segment
        .glyphs
        .iter()
        .filter(|g| is_relation(g))
        .map(|g| g.x)
        .collect()
}

/// Display equations on a page, found from its glyphs.
///
/// Conservative by design: a block is reported only for a run of rows that
/// reads as mathematics - mostly math fonts, a big operator, or a fraction
/// bar, without much running text (operator names and scripts are no
/// words) - and stands apart from the prose as a display does: centred in
/// its text column, or indented or flush with space above and below and no
/// words in it, or as wide as the column with a number. A `(n)` at the
/// right (or, `leqno`, the left), or on a line of its own under a row too
/// long for it, is taken as its number.
///
/// Rows join in two passes, nearest first: each equation's stacks (a
/// fraction's numerator and denominator, an operator's limits), then the
/// rows of a display (the lines of an `align`, of a `multline`). Two
/// equations each with its own number stay two unless they line up on a
/// relation, as an `align` numbered line by line does; nothing joins across
/// a column gutter; pieces level with a line of prose (its inline scripts)
/// are dropped.
pub fn equation_blocks(glyphs: &[GlyphDto], page_w: f32, _page_h: f32) -> Vec<EquationBlockDto> {
    let chars: Vec<&GlyphDto> = glyphs
        .iter()
        // A glyph with no Unicode (`text` empty) is still ink: a TeX
        // extension-font delimiter or operator usually has none.
        .filter(|g| !g.is_rule() && g.w > 0.0 && (g.text.is_empty() || !g.text.trim().is_empty()))
        .collect();
    let rules: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| g.is_rule() && g.w < 0.6 * page_w)
        .collect();
    if chars.is_empty() {
        return Vec::new();
    }

    let gutters = gutters(&chars, page_w);
    let all: Vec<Vec<Segment>> = rows(&chars)
        .iter()
        .map(|row| segments(&chars, row, &gutters))
        .collect();
    let page_em = median(chars.iter().map(|g| g.size).collect()).max(1.0);

    // Text columns, from the lines of running text: wide segments that are
    // not mathematics.
    let body: Vec<(f32, f32)> = all
        .iter()
        .flatten()
        .filter(|s| s.width() >= 0.25 * page_w && classify(s, &rules) == Kind::Text)
        .map(|s| (s.left, s.right))
        .collect();
    let page_left = all
        .iter()
        .flatten()
        .map(|s| s.left)
        .fold(f32::MAX, f32::min);
    let page_right = all
        .iter()
        .flatten()
        .map(|s| s.right)
        .fold(f32::MIN, f32::max);
    let column = |centre: f32| -> (f32, f32) {
        let around: Vec<&(f32, f32)> = body
            .iter()
            .filter(|(l, r)| *l <= centre && centre <= *r)
            .collect();
        if around.len() >= 2 {
            (
                median(around.iter().map(|(l, _)| *l).collect()),
                median(around.iter().map(|(_, r)| *r).collect()),
            )
        } else {
            (page_left, page_right)
        }
    };

    // Per row: math segments joined into a candidate, with a number when
    // the row's last segment is one; unclaimed narrow text segments are
    // kept to be absorbed as numerators and denominators.
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut loose: Vec<&Segment> = Vec::new();
    let mut numbers: Vec<&Segment> = Vec::new();
    for row in &all {
        // A line that is mostly words is running text, however much inline
        // mathematics it holds; so is one with some words that starts flush
        // against its column's left edge, or runs from a paragraph indent to
        // the right margin: displays are centred or indented, and their
        // words are few ("for all", "and").
        let cols: Vec<(f32, f32)> = row
            .iter()
            .map(|s| column((s.left + s.right) / 2.0))
            .collect();
        let kinds: Vec<Kind> = row
            .iter()
            .zip(&cols)
            .map(|(s, &col)| {
                let kind = classify(s, &rules);
                if kind != Kind::Math {
                    return kind;
                }
                let line: Vec<&Segment> = row
                    .iter()
                    .zip(&cols)
                    .filter(|(o, c)| **c == col && equation_number(&o.text()).is_none())
                    .map(|(o, _)| o)
                    .collect();
                let left = line.iter().map(|o| o.left).fold(f32::MAX, f32::min);
                let last = line.iter().max_by(|a, b| a.right.total_cmp(&b.right));
                let right = last.map_or(s.right, |o| o.right);
                let numbered = last.is_some_and(|o| {
                    let text = o.text();
                    text.trim_end()
                        .rfind('(')
                        .is_some_and(|at| equation_number(&text[at..]).is_some())
                });
                let glyphs: Vec<&GlyphDto> =
                    line.iter().flat_map(|o| o.glyphs.iter().copied()).collect();
                let operators = operator_letters(&glyphs);
                let main = glyphs.iter().map(|g| g.size).fold(0.0, f32::max);
                let (n, words) =
                    glyphs
                        .iter()
                        .zip(&operators)
                        .fold((0usize, 0usize), |(n, w), (g, op)| {
                            let word = !op
                                && !is_math_font_name(&g.font)
                                && g.text.chars().any(char::is_alphabetic)
                                && !is_script(g, main);
                            (n + 1, w + usize::from(word))
                        });
                let wordy = words as f32 / n.max(1) as f32;
                // A line running on past its column belongs to none: an
                // equation set across both columns of a two-column page.
                let within = right <= col.1 + 0.5 * s.em;
                // A numbered line is a display even flush left (`fleqn`), its
                // few words ("ex" in E_ex) set in the text font.
                let has_number = numbered
                    || row
                        .iter()
                        .zip(&cols)
                        .any(|(o, c)| *c == col && equation_number(&o.text()).is_some());
                let flush = within && left - col.0 < 0.3 * s.em && !has_number;
                let justified =
                    within && left - col.0 < 2.0 * s.em && col.1 - right < 0.5 * s.em && !numbered;
                // A line ending in a colon introduces what follows: the end
                // of a sentence, "(writing L_λ = L_0):", not a display.
                let colon = last.is_some_and(|o| o.text().trim_end().ends_with(':'));
                // Indented from both margins, a line can be a display with
                // operator names in it ("err_P(f) = Ω(d/n)"); only all but
                // wholly prose is text there.
                let inset = left - col.0 > 2.0 * s.em && col.1 - right > 2.0 * s.em;
                let prose = if inset { wordy > 0.8 } else { wordy > 0.5 };
                if prose || ((flush || justified) && wordy > 0.15) || colon {
                    Kind::Text
                } else {
                    kind
                }
            })
            .collect();
        let mut current: Option<Candidate> = None;
        let mut row_candidates: Vec<Candidate> = Vec::new();
        for (segment, kind) in row.iter().zip(&kinds) {
            match kind {
                Kind::Math => {
                    let boxes: Vec<[f32; 4]> =
                        segment.glyphs.iter().map(|g| outline_box(g)).collect();
                    match current.as_mut() {
                        Some(c)
                            if segment.left - c.right <= 4.0 * segment.em
                                && !across_gutter(
                                    (c.left, c.right),
                                    (segment.left, segment.right),
                                    &gutters,
                                ) =>
                        {
                            c.right = segment.right;
                            c.top = c.top.min(segment.top);
                            c.bottom = c.bottom.max(segment.bottom);
                            c.boxes.extend(boxes);
                            c.chars += segment.glyphs.len();
                            c.bare += bare(segment);
                            c.alnum += alnum(segment);
                            c.largest = c.largest.max(largest(segment));
                            c.relations.extend(relations(segment));
                            c.columns.push(segment.left);
                        }
                        _ => {
                            if let Some(c) = current.take() {
                                row_candidates.push(c);
                            }
                            current = Some(Candidate {
                                left: segment.left,
                                right: segment.right,
                                top: segment.top,
                                bottom: segment.bottom,
                                em: segment.em,
                                boxes,
                                chars: segment.glyphs.len(),
                                bare: bare(segment),
                                alnum: alnum(segment),
                                largest: largest(segment),
                                number: None,
                                relations: relations(segment),
                                columns: Vec::new(),
                            });
                        }
                    }
                }
                Kind::Number => numbers.push(segment),
                Kind::Text => {
                    if let Some(c) = current.take() {
                        row_candidates.push(c);
                    }
                    loose.push(segment);
                }
            }
        }
        if let Some(c) = current.take() {
            row_candidates.push(c);
        }
        candidates.extend(row_candidates);
    }
    // A fraction bar with characters just above and just below it is a
    // candidate of its own, even when its numerator and denominator are
    // set in a text font.
    for rule in &rules {
        let stacked = |above: bool| {
            chars.iter().any(|g| {
                let centre = g.x + g.w / 2.0;
                let (top, bottom) = core(g);
                let near = if above {
                    bottom <= rule.baseline + 0.3 * g.size && rule.baseline - bottom <= 1.0 * g.size
                } else {
                    top >= rule.baseline - 0.3 * g.size && top - rule.baseline <= 1.0 * g.size
                };
                near && centre >= rule.x - 1.0 && centre <= rule.x + rule.w + 1.0
            })
        };
        // A bar level with a line of running text is an inline fraction.
        let inline = loose.iter().any(|t| {
            t.top - 0.3 * t.em <= rule.baseline
                && rule.baseline <= t.bottom + 0.3 * t.em
                && t.left - t.em <= rule.x + rule.w
                && t.right + t.em >= rule.x
        });
        if !inline && stacked(true) && stacked(false) {
            candidates.push(Candidate {
                left: rule.x,
                right: rule.x + rule.w,
                top: rule.baseline,
                bottom: rule.baseline,
                em: page_em,
                boxes: vec![[rule.x, rule.y, rule.w, rule.h]],
                chars: 0,
                bare: 0,
                alnum: 0,
                largest: 0.0,
                number: None,
                relations: Vec::new(),
                columns: Vec::new(),
            });
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    let barriers: Vec<Barrier> = loose
        .iter()
        .filter(|s| s.width() >= 6.0 * s.em)
        .map(|s| (s.left, s.right, s.top, s.bottom))
        .collect();
    // Pieces level with a line of prose running across them are its inline
    // mathematics - scripts, a tall delimiter - split off its row; set
    // close above a display, they would join it.
    candidates.retain(|c| !inline_with(c, &barriers));
    // A number level with a row of mathematics belongs to it from the
    // start, so two equations each numbered stay two; one between rows (a
    // `split` numbered in the middle) waits for the rows to merge.
    let tags: Vec<Tag> = numbers
        .iter()
        .map(|t| (t.left, t.right, t.top, t.bottom))
        .collect();
    let mut numbers: Vec<&Segment> = numbers
        .into_iter()
        .filter(|segment| {
            let middle = (segment.top + segment.bottom) / 2.0;
            let owner = candidates
                .iter_mut()
                .filter(|c| {
                    c.number.is_none()
                        && middle >= c.top
                        && middle <= c.bottom
                        && tag_gap(segment, c).is_some_and(|gap| gap < 0.5 * page_w)
                        && !across_gutter(
                            (c.left, c.right),
                            (segment.left, segment.right),
                            &gutters,
                        )
                })
                .min_by(|a, b| {
                    let gap = |c| tag_gap(segment, c).unwrap_or(f32::MAX);
                    gap(a).total_cmp(&gap(b))
                });
            match owner {
                Some(c) => {
                    c.number = equation_number(&segment.text());
                    false
                }
                None => true,
            }
        })
        .collect();
    // First each equation's own stacks - numerators, limits, scripts, set
    // close - then the rows of a display, a little further apart. Two
    // equations one under the other are further apart still, and an
    // equation's limits are never taken by the one below it.
    let merged = merge_candidates(candidates, &barriers, &gutters, &[], STACK_GAP_EMS);
    let mut merged = merge_candidates(merged, &barriers, &gutters, &tags, ROW_GAP_EMS);
    // The space between two lines of a paragraph: a display stands further
    // from the text above it than that (double-spaced theses set it wide).
    let leading = {
        let mut lines: Vec<&Barrier> = barriers.iter().collect();
        lines.sort_by(|a, b| a.2.total_cmp(&b.2));
        let gaps: Vec<f32> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                lines[i + 1..]
                    .iter()
                    .find(|b| b.2 >= a.3 && b.0 < a.1 && b.1 > a.0)
                    .map(|b| b.2 - a.3)
                    .filter(|&gap| gap < 2.0 * page_em)
            })
            .collect();
        if gaps.len() >= 3 {
            median(gaps).min(1.5 * page_em)
        } else {
            0.1 * page_em
        }
    };

    // A short text segment hugging a candidate from above or below and
    // lying within its width is part of it: a fraction's "1" over "2".
    let mut absorbed = false;
    for segment in loose {
        // A numerator set in the text font is a few symbols, not prose:
        // "ratios:" ending the line above a display is not part of it.
        let words = segment
            .glyphs
            .iter()
            .filter(|g| g.text.chars().any(char::is_alphabetic))
            .count();
        let prose = segment.em >= 0.9 * page_em
            && (words >= 3 || segment.text().trim_end().ends_with([':', ',', ';']));
        let fits = |c: &Candidate| {
            let gap = (segment.top - c.bottom).max(c.top - segment.bottom);
            gap <= 0.6 * c.em
                && segment.left >= c.left - c.em
                && segment.right <= c.right + c.em
                && segment.width() < 0.8 * (c.right - c.left).max(c.em)
        };
        if let Some(c) = merged.iter_mut().find(|c| !prose && fits(c)) {
            c.top = c.top.min(segment.top);
            c.bottom = c.bottom.max(segment.bottom);
            c.chars += segment.glyphs.len();
            c.bare += bare(segment);
            c.alnum += alnum(segment);
            c.largest = c.largest.max(largest(segment));
            c.boxes
                .extend(segment.glyphs.iter().map(|g| outline_box(g)));
            absorbed = true;
        }
    }
    if absorbed {
        merged = merge_candidates(merged, &barriers, &gutters, &tags, ROW_GAP_EMS);
    }
    // A number belongs to the nearest block it sits level with, to its
    // left (on a two-column page the left column's display is level with
    // the right one's number too).
    for segment in numbers.drain(..) {
        let middle = (segment.top + segment.bottom) / 2.0;
        let owner = merged
            .iter_mut()
            .filter(|c| {
                c.number.is_none()
                    && middle >= c.top - 0.5 * c.em
                    && middle <= c.bottom + 0.5 * c.em
                    // Right of it, or of most of it: a number centred on
                    // two lines may sit beside the shorter one; or left of
                    // it (`leqno`).
                    && ((segment.right > c.right && segment.left > (c.left + c.right) / 2.0)
                        || (segment.left < c.left && segment.right < (c.left + c.right) / 2.0))
                    && !across_gutter((c.left, c.right), (segment.left, segment.right), &gutters)
            })
            .max_by(|a, b| a.right.total_cmp(&b.right));
        if let Some(c) = owner {
            c.number = equation_number(&segment.text());
            continue;
        }
        // A number its equation left no room for, on a line of its own:
        // under the equation's end, or (`leqno`) over its start.
        let owner = merged
            .iter_mut()
            .filter(|c| {
                let below = segment.top >= c.bottom - 0.2 * c.em
                    && segment.top - c.bottom <= ROW_GAP_EMS * c.em
                    && segment.right >= c.right - 2.0 * c.em
                    && segment.left > (c.left + c.right) / 2.0;
                let above = segment.bottom <= c.top + 0.2 * c.em
                    && c.top - segment.bottom <= ROW_GAP_EMS * c.em
                    && segment.left <= c.left + c.em
                    && segment.right < (c.left + c.right) / 2.0;
                c.number.is_none()
                    && (below || above)
                    && !across_gutter((c.left, c.right), (segment.left, segment.right), &gutters)
            })
            .min_by(|a, b| {
                let gap = |c: &Candidate| (segment.top - c.bottom).max(c.top - segment.bottom);
                gap(a).total_cmp(&gap(b))
            });
        if let Some(c) = owner {
            c.number = equation_number(&segment.text());
        }
    }

    // Rules inside a block widen it to the bar's full length.
    for c in &mut merged {
        for rule in &rules {
            let centre = rule.x + rule.w / 2.0;
            if centre >= c.left
                && centre <= c.right
                && rule.baseline >= c.top
                && rule.baseline <= c.bottom
            {
                c.boxes.push([rule.x, rule.y, rule.w, rule.h]);
            }
        }
    }

    merged
        .into_iter()
        .filter_map(|c| {
            if c.chars - c.bare < 2 || c.alnum == 0 || c.largest < 0.8 * page_em {
                return None;
            }
            if inline_with(&c, &barriers) {
                return None;
            }
            let [x, y, w, h] = union(c.boxes.iter().copied())?;
            let (col_left, col_right) = column(x + w / 2.0);
            let col_width = (col_right - col_left).max(1.0);
            let centre_offset = ((x + w / 2.0) - (col_left + col_right) / 2.0).abs();
            let centred = centre_offset <= 0.15 * col_width
                && w < 0.96 * col_width
                && x > col_left + 0.02 * col_width;
            // Some journals (Elsevier's, `fleqn`) set displays flush left;
            // its lines are already known to be mathematics, not prose.
            // Unnumbered, it must stand apart from the text above it: a
            // paragraph's last line, all mathematics, sits one line under the
            // text before it, a display further.
            let set_apart = c.number.is_some()
                || !barriers.iter().any(|&(left, right, _, bottom)| {
                    bottom <= c.top + 0.1 * c.em
                        && c.top - bottom < leading + 0.4 * c.em
                        && left < c.right
                        && right > c.left
                });
            // And from the text below it.
            let set_apart_below = c.number.is_some()
                || !barriers.iter().any(|&(left, right, top, _)| {
                    top >= c.bottom - 0.1 * c.em
                        && top - c.bottom < leading + 0.4 * c.em
                        && left < c.right
                        && right > c.left
                });
            // Words of prose in it: a list item's run-in heading, a line
            // of a paragraph indented under its item.
            let words = prose_words(
                &chars
                    .iter()
                    .copied()
                    .filter(|g| inside(centre_of(&drawn(g)), &[x, y, w, h]))
                    .collect::<Vec<_>>(),
                page_em,
            );
            let flush_left = (x - col_left).abs() < 0.05 * col_width
                && w < 0.9 * col_width
                && set_apart
                && words == 0;
            // Indented by `fleqn`'s \mathindent, clear of the right margin.
            let indented = set_apart
                && words == 0
                && x - col_left >= 0.8 * c.em
                && x - col_left <= 6.0 * c.em
                && col_right - (x + w) >= 2.0 * c.em;
            // A numbered equation as wide as its column (a narrow column of
            // a two-column page): the number says it is a display; so does
            // space above and below an unnumbered one with no words in it.
            let wide = x >= col_left - 0.5 * c.em && x + w <= col_right + 2.0 * c.em;
            let numbered_wide = wide && c.number.is_some();
            let apart_wide = wide && set_apart && set_apart_below && words == 0;
            // A line of words that happens to sit mid-column (a list item
            // with a formula in it) is no display; a display's words are a
            // few: "for all", "subject to", "otherwise".
            let centred = centred && (words < 3 || c.number.is_some());
            (centred || flush_left || indented || numbered_wide || apart_wide).then_some(
                EquationBlockDto {
                    bbox: [x, y, w, h],
                    number: c.number,
                },
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Displays from the page's pixels
// ---------------------------------------------------------------------------

/// A line of a column as the raster shows it: strips joined through their
/// stacks, its ink box (points), and what the glyphs in it say.
#[derive(Clone, Debug)]
struct InkLine {
    /// `[x, y, w, h]` of the ink, without an equation number.
    bbox: [f32; 4],
    number: Option<String>,
    display: bool,
    /// Mathematics by what it holds, display or not.
    mathy: bool,
    /// The median size of its glyphs.
    size: f32,
    /// Where its relation symbols stand.
    relations: Vec<f32>,
    /// Set as close under the line above as a paragraph's lines are.
    glued: bool,
    /// Centred in its column.
    centred: bool,
}

/// Where a glyph is drawn: its ink box when PDFium gave one, else its loose
/// box held to the band around its baseline.
fn drawn(g: &GlyphDto) -> [f32; 4] {
    g.ink.unwrap_or_else(|| outline_box(g))
}

fn centre_of(r: &[f32; 4]) -> (f32, f32) {
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn inside(p: (f32, f32), r: &[f32; 4]) -> bool {
    p.0 >= r[0] && p.0 <= r[0] + r[2] && p.1 >= r[1] && p.1 <= r[1] + r[3]
}

fn union_pt(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (l, t) = (a[0].min(b[0]), a[1].min(b[1]));
    let (r, bo) = (
        (a[0] + a[2]).max(b[0] + b[2]),
        (a[1] + a[3]).max(b[1] + b[3]),
    );
    [l, t, r - l, bo - t]
}

fn overlaps_pt(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

/// Characters that make a line mathematics whatever font sets them.
fn is_math_char(c: char) -> bool {
    "=+<>\u{2212}\u{00B1}\u{00D7}\u{00F7}\u{2211}\u{220F}\u{222B}\u{221A}\u{221E}\u{2202}\u{2207}\u{2264}\u{2265}\u{2260}\u{2248}\u{2261}\u{2208}\u{2209}\u{2282}\u{2286}\u{2192}\u{21D2}\u{2200}\u{2203}\u{22C5}"
        .contains(c)
        || ('\u{0391}'..='\u{03F6}').contains(&c)
        || ('\u{1D400}'..='\u{1D7FF}').contains(&c)
}

/// The displays of a page read from its raster, with the glyphs saying
/// which lines are mathematics.
///
/// Recursive XY-cut finds the columns and their strips of ink; strips are
/// joined into lines through fraction stacks and nested accents and limits
/// ([`vision::logical_lines`]). A line is a display when it is
/// mathematics (math fonts, operators, Greek, a bar; few words) and sits
/// apart from the column's text: indented from both edges, or numbered at
/// the right margin, or flush left (`fleqn`) with space above and below.
/// Consecutive display lines with no prose between them are one display,
/// unless each has its own number.
pub fn raster_displays(glyphs: &[GlyphDto], raster: &vision::PageRaster) -> Vec<EquationBlockDto> {
    let chars: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| !g.is_rule() && g.w > 0.0)
        .collect();
    let gutters = gutters(&chars, raster.bits.w as f32 / raster.scale);
    displays_from_lines(glyphs, &ink_lines(glyphs, raster, &gutters))
}

/// The lines of running text the raster found, `[x, y, w, h]`.
fn text_lines(lines: &[(vision::PxBox, Vec<InkLine>)], page_em: f32) -> Vec<[f32; 4]> {
    // Full-size and long: a label under a brace or a limit is small, a
    // "for all x" beside a formula short.
    lines
        .iter()
        .flat_map(|(_, ls)| {
            ls.iter()
                .filter(|l| {
                    !l.display && !l.mathy && l.size >= 0.9 * page_em && l.bbox[2] >= 15.0 * page_em
                })
                .map(|l| l.bbox)
        })
        .collect()
}

fn displays_from_lines(
    glyphs: &[GlyphDto],
    lines: &[(vision::PxBox, Vec<InkLine>)],
) -> Vec<EquationBlockDto> {
    let lines = lines.to_vec();
    let mut out: Vec<EquationBlockDto> = Vec::new();
    let mut current: Option<(EquationBlockDto, f32, Vec<f32>)> = None;
    let page_em = median(
        glyphs
            .iter()
            .filter(|g| !g.is_rule())
            .map(|g| g.size)
            .collect(),
    )
    .max(1.0);
    for (column, column_lines) in lines {
        let _ = column;
        let mut prose_above = false;
        for line in column_lines {
            // An unnumbered line set close under a line of prose goes on
            // with it: an item's second line, a paragraph's last.
            let continues = line.glued && !line.centred && line.number.is_none() && prose_above;
            prose_above = !line.display && !line.mathy;
            if !line.display || continues {
                if let Some((b, ..)) = current.take() {
                    out.push(b);
                }
                continue;
            }
            let bottom = line.bbox[1] + line.bbox[3];
            match current.as_mut() {
                Some((b, prev_bottom, relations))
                    if line.bbox[1] - *prev_bottom <= INK_ROW_GAP_EMS * page_em
                        && (b.number.is_none()
                            || line.number.is_none()
                            || relations
                                .iter()
                                .any(|a| line.relations.iter().any(|r| (a - r).abs() < 1.0))) =>
                {
                    b.bbox = union_pt(b.bbox, line.bbox);
                    if b.number.is_none() {
                        b.number = line.number.clone();
                    }
                    *prev_bottom = bottom;
                    relations.extend(line.relations.iter().copied());
                }
                _ => {
                    if let Some((b, ..)) = current.take() {
                        out.push(b);
                    }
                    current = Some((
                        EquationBlockDto {
                            bbox: line.bbox,
                            number: line.number.clone(),
                        },
                        bottom,
                        line.relations.clone(),
                    ));
                }
            }
        }
        if let Some((b, ..)) = current.take() {
            out.push(b);
        }
    }
    out.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
    out
}

/// The lines of each column, top down, classified.
fn ink_lines(
    glyphs: &[GlyphDto],
    raster: &vision::PageRaster,
    gutters: &[(f32, f32)],
) -> Vec<(vision::PxBox, Vec<InkLine>)> {
    let chars: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| !g.is_rule() && g.w > 0.0 && (g.text.is_empty() || !g.text.trim().is_empty()))
        .collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let em = median(chars.iter().map(|g| g.size).collect()).max(1.0);
    let em_px = raster.px(em);
    let bits = &raster.bits;
    let page = vision::PxBox::new(0, 0, bits.w, bits.h);
    let strips = vision::xy_cut(
        bits,
        page,
        vision::CutOptions {
            min_gutter: raster.px(1.2 * em),
            min_gutter_height: raster.px(2.5 * em),
            band_gap: raster.px(1.5 * em),
            min_column_width: raster.px(10.0 * em),
        },
    );
    // The PDF's own rules, and long thin runs in the raster.
    let mut rules: Vec<vision::PxBox> = glyphs
        .iter()
        .filter(|g| g.is_rule())
        .map(|g| raster.to_px([g.x, g.y, g.w, g.h.max(0.5)]))
        .collect();
    // Raster rules that are a glyph's own strokes (the bar of an E, a
    // minus sign) are not bars.
    let inks: Vec<vision::PxBox> = chars.iter().map(|g| raster.to_px(drawn(g))).collect();
    for s in &strips {
        rules.extend(
            vision::horizontal_rules(
                bits,
                s.bbox,
                raster.px(0.5 * em),
                raster.px(0.12 * em).max(2) + 1,
            )
            .into_iter()
            .filter(|r| {
                !inks
                    .iter()
                    .any(|i| i.intersection_area(r) * 2 >= r.w() * r.h())
            }),
        );
    }
    // Columns: the XY-cut finds them band by band; the bands' columns at
    // the same place across the page are one column.
    let tol = raster.px(2.0 * em);
    let mut columns: Vec<vision::PxBox> = Vec::new();
    let mut key_of: Vec<usize> = Vec::with_capacity(strips.len());
    for s in &strips {
        let c = s.column;
        let at = columns
            .iter()
            .position(|k| k.x0.abs_diff(c.x0) <= tol && k.x1.abs_diff(c.x1) <= tol);
        match at {
            Some(at) => {
                columns[at] = columns[at].union(&c);
                key_of.push(at);
            }
            None => {
                columns.push(c);
                key_of.push(columns.len() - 1);
            }
        }
    }
    let mut out = Vec::new();
    for (key, &column) in columns.iter().enumerate() {
        let col_strips: Vec<vision::PxBox> = strips
            .iter()
            .zip(&key_of)
            .filter(|(_, &k)| k == key)
            .map(|(s, _)| s.bbox)
            .collect();
        let groups = vision::logical_lines(
            &col_strips,
            &rules,
            em_px,
            raster.px(0.7 * em),
            raster.px(0.35 * em),
        );
        let boxes: Vec<vision::PxBox> = groups
            .iter()
            .map(|g| {
                g.iter()
                    .map(|&i| col_strips[i])
                    .reduce(|a, b| a.union(&b))
                    .expect("a line has a strip")
            })
            .collect();
        // The column's text edges: where its wide lines start and end.
        let widest = boxes.iter().map(|b| b.w()).max().unwrap_or(0);
        let wide: Vec<&vision::PxBox> = boxes.iter().filter(|b| b.w() * 10 >= widest * 6).collect();
        let col_left = median(wide.iter().map(|b| b.x0 as f32).collect()) / raster.scale;
        let col_right = median(wide.iter().map(|b| b.x1 as f32).collect()) / raster.scale;
        // The gutters this column of the cut runs across: where the cut
        // failed to part two columns (their lines level, a display in each).
        let col_pt = raster.to_pt(&column);
        let inner: Vec<(f32, f32)> = gutters
            .iter()
            .copied()
            .filter(|&(g0, g1)| g0 > col_pt[0] && g1 < col_pt[0] + col_pt[2])
            .collect();
        // Lines by the side of the gutters they are on; a line across one
        // keeps to itself.
        let mut sides: std::collections::BTreeMap<usize, Vec<InkLine>> = Default::default();
        // The space between the lines of the column's paragraphs.
        let leading = {
            let gaps: Vec<f32> = boxes
                .windows(2)
                .map(|w| (w[1].y0 as f32 - w[0].y1 as f32) / raster.scale)
                .filter(|&g| g > 0.0 && g < 2.0 * em)
                .collect();
            if gaps.len() >= 3 {
                median(gaps).min(1.5 * em)
            } else {
                0.0
            }
        };
        for (k, b) in boxes.iter().enumerate() {
            let pt = raster.to_pt(b);
            let members: Vec<&GlyphDto> = chars
                .iter()
                .copied()
                .filter(|g| inside(centre_of(&drawn(g)), &pt))
                .collect();
            let gap_above = if k > 0 {
                (b.y0 as f32 - boxes[k - 1].y1 as f32) / raster.scale
            } else {
                f32::MAX
            };
            let gap_below = if k + 1 < boxes.len() {
                (boxes[k + 1].y0 as f32 - b.y1 as f32) / raster.scale
            } else {
                f32::MAX
            };
            for (piece, piece_members, side) in split_at_gutters(pt, &members, &inner) {
                let (left, right) = match side {
                    Some(i) => (
                        if i == 0 { col_left } else { inner[i - 1].1 },
                        inner.get(i).map_or(col_right, |g| g.0),
                    ),
                    None => (col_left, col_right),
                };
                let piece_px = raster.to_px(piece);
                let has_rule = rules
                    .iter()
                    .any(|r| r.intersection_area(&piece_px) > 0 && r.w() > em_px / 2);
                sides
                    .entry(side.unwrap_or(usize::MAX))
                    .or_default()
                    .push(classify_line(
                        piece,
                        &piece_members,
                        has_rule,
                        left,
                        right,
                        em,
                        gap_above,
                        gap_below,
                        leading,
                    ));
            }
        }
        for (_, lines) in sides {
            out.push((column, lines));
        }
    }
    out
}

/// A line of the raster cut at the gutters it crosses, when nothing of it
/// runs through them: two columns' lines the cut took for one. Each piece
/// with its glyphs and the side of the gutters it is on (`None` for a line
/// set across them).
fn split_at_gutters<'a>(
    pt: [f32; 4],
    members: &[&'a GlyphDto],
    gutters: &[(f32, f32)],
) -> Vec<([f32; 4], Vec<&'a GlyphDto>, Option<usize>)> {
    let side_of = |x0: f32, x1: f32| column_side(x0, x1, gutters);
    if let Some(side) = side_of(pt[0], pt[0] + pt[2]) {
        return vec![(pt, members.to_vec(), Some(side))];
    }
    // No clear space anywhere in a gutter it crosses: a line set across
    // the columns, not two columns' lines side by side (one of which may
    // run a little into the gutter).
    let em = median(members.iter().map(|g| g.size).collect()).max(1.0);
    let crossed = gutters.iter().any(|&(g0, g1)| {
        if !(pt[0] < g0 && pt[0] + pt[2] > g1) {
            return false;
        }
        let mut inks: Vec<(f32, f32)> = members
            .iter()
            .map(|g| drawn(g))
            .filter(|d| d[0] < g1 && d[0] + d[2] > g0)
            .map(|d| (d[0].max(g0), (d[0] + d[2]).min(g1)))
            .collect();
        inks.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut edge = g0;
        let mut widest = 0.0f32;
        for (a, b) in inks {
            widest = widest.max(a - edge);
            edge = edge.max(b);
        }
        widest = widest.max(g1 - edge);
        widest < 0.4 * em
    });
    if crossed || members.is_empty() {
        return vec![(pt, members.to_vec(), None)];
    }
    let mut pieces: std::collections::BTreeMap<usize, Vec<&GlyphDto>> = Default::default();
    for g in members {
        let d = drawn(g);
        if let Some(side) = side_of(d[0], d[0] + d[2]) {
            pieces.entry(side).or_default().push(g);
        }
    }
    pieces
        .into_iter()
        .filter_map(|(side, glyphs)| {
            let ink = glyphs.iter().map(|g| drawn(g)).reduce(union_pt)?;
            // The piece's ink across, the line's band down.
            Some(([ink[0], pt[1], ink[2], pt[3]], glyphs, Some(side)))
        })
        .collect()
}

/// What a line of a column is: its ink without a trailing equation number,
/// the number, and whether it is a display.
#[allow(clippy::too_many_arguments)]
fn classify_line(
    pt: [f32; 4],
    members: &[&GlyphDto],
    has_rule: bool,
    col_left: f32,
    col_right: f32,
    em: f32,
    gap_above: f32,
    gap_below: f32,
    leading: f32,
) -> InkLine {
    let mut bbox = pt;
    // A trailing "(n)" set apart at the right: the number.
    let mut sorted: Vec<&GlyphDto> = members.to_vec();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut number = None;
    if let Some(at) = sorted.iter().rposition(|g| g.text == "(") {
        let tail: String = sorted[at..].iter().map(|g| g.text.as_str()).collect();
        let before = sorted[..at]
            .iter()
            .map(|g| g.x + g.w)
            .fold(f32::MIN, f32::max);
        let level = sorted[at..]
            .iter()
            .all(|g| (g.baseline - sorted[at].baseline).abs() < 0.1 * em);
        if let (Some(n), true) = (equation_number(&tail), level) {
            if at > 0
                && sorted[at].x - before >= TAG_SEP_EMS * em
                && pt[0] + pt[2] >= col_right - em
            {
                number = Some(n);
                let keep: Vec<[f32; 4]> = sorted[..at].iter().map(|g| drawn(g)).collect();
                if let Some(u) = keep.into_iter().reduce(union_pt) {
                    // Only narrow the ink to the left of the number.
                    let right = u[0] + u[2];
                    bbox[2] = (right - bbox[0]).max(0.0);
                }
            }
        }
    }
    // Or a leading one at the left margin (`leqno`).
    if number.is_none() {
        if let Some(close) = sorted.iter().position(|g| g.text == ")") {
            let head: String = sorted[..=close].iter().map(|g| g.text.as_str()).collect();
            let after = sorted.get(close + 1).map(|g| g.x);
            let level = sorted[..=close]
                .iter()
                .all(|g| (g.baseline - sorted[0].baseline).abs() < 0.1 * em);
            if let (Some(n), Some(next), true) = (equation_number(&head), after, level) {
                let end = sorted[close].x + sorted[close].w;
                if next - end >= 0.8 * em && pt[0] <= col_left + em {
                    number = Some(n);
                    let keep: Vec<[f32; 4]> =
                        sorted[close + 1..].iter().map(|g| drawn(g)).collect();
                    if let Some(u) = keep.into_iter().reduce(union_pt) {
                        let right = bbox[0] + bbox[2];
                        bbox[0] = u[0].max(bbox[0]);
                        bbox[2] = (right - bbox[0]).max(0.0);
                    }
                }
            }
        }
    }
    let body: Vec<&&GlyphDto> = sorted
        .iter()
        .filter(|g| g.x + g.w <= bbox[0] + bbox[2] + 0.5 && g.x >= bbox[0] - 0.5)
        .collect();
    let n = body.len().max(1) as f32;
    let math = body
        .iter()
        .filter(|g| {
            is_math_font_name(&g.font) || g.text.is_empty() || g.text.chars().any(is_math_char)
        })
        .count();
    let body_glyphs: Vec<&GlyphDto> = body.iter().map(|g| **g).collect();
    let main = body_glyphs.iter().map(|g| g.size).fold(0.0, f32::max);
    let operators = operator_letters(&body_glyphs);
    let operator = |g: &GlyphDto| {
        body_glyphs
            .iter()
            .zip(&operators)
            .any(|(o, op)| *op && std::ptr::eq(*o, g))
    };
    let wordy = |g: &GlyphDto| {
        !is_math_font_name(&g.font)
            && g.text
                .chars()
                .any(|c| c.is_alphabetic() && !is_math_char(c))
            && !operator(g)
            && !is_script(g, main)
    };
    let words = body.iter().filter(|g| wordy(g)).count() as f32 / n;
    // Letters in long words (five letters or more, run together on one
    // baseline): prose. Operator names (ln, exp, err, diam) are short.
    let mut prose_letters = 0usize;
    let mut run: Vec<&GlyphDto> = Vec::new();
    let mut flush_run = |run: &mut Vec<&GlyphDto>| {
        if run.len() >= 5 {
            prose_letters += run.len();
        }
        run.clear();
    };
    for g in body.iter().map(|g| **g) {
        let joins = run.last().is_some_and(|p: &&GlyphDto| {
            (p.baseline - g.baseline).abs() < 0.1 * em && g.x - (p.x + p.w) < 0.15 * em
        });
        if wordy(g) {
            if !joins {
                flush_run(&mut run);
            }
            run.push(g);
        } else {
            flush_run(&mut run);
        }
    }
    flush_run(&mut run);
    let prose = prose_letters as f32 / n;
    let alnum = body
        .iter()
        .filter(|g| g.text.chars().any(char::is_alphanumeric))
        .count();
    let left = bbox[0] - col_left;
    let right = col_right - (bbox[0] + bbox[2]);
    let inset = left > 1.5 * em && right > 1.5 * em;
    let mathy = (math > 0 || has_rule)
        && body.len() >= 2
        && alnum > 0
        && words <= if inset { 0.8 } else { 0.5 }
        && prose <= if inset { 0.35 } else { 0.25 };
    // Ending in a colon, a line introduces what follows.
    let colon = body.last().is_some_and(|g| g.text == ":");
    // Further from its neighbours than the lines of a paragraph are.
    let apart_by = (0.45 * em).max(leading + 0.3 * em);
    let apart = gap_above > apart_by && gap_below > apart_by;
    // Indented and full of words, a line is a list item: "Exact
    // Agnostic: for all finite S, ...".
    let item = prose_words(&body_glyphs, em) >= 3;
    let display = mathy
        && !colon
        && ((inset && !item)
            || (number.is_some() && left > -0.5 * em)
            || (apart && left.abs() < 0.5 * em && right > 3.0 * em && words <= 0.2));
    InkLine {
        bbox,
        number,
        display,
        mathy,
        size: median(body.iter().map(|g| g.size).collect()),
        relations: body
            .iter()
            .filter(|g| is_relation(g))
            .map(|g| g.x)
            .collect(),
        glued: gap_above <= leading + 0.15 * em,
        centred: ((bbox[0] + bbox[2] / 2.0) - (col_left + col_right) / 2.0).abs()
            <= 0.1 * (col_right - col_left),
    }
}

/// Whether most of a block lies inside one of a page's floats. A numbered
/// display is never in one: a table's cells and a plot's labels carry no
/// number at the column's edge, and a float found a little too large must
/// not take the equation under it.
fn in_float(block: &EquationBlockDto, floats: &[[f32; 4]]) -> bool {
    let b = &block.bbox;
    if block.number.is_some() {
        return false;
    }
    let area = b[2].max(0.0) * b[3].max(0.0);
    floats.iter().any(|f| {
        let w = (b[0] + b[2]).min(f[0] + f[2]) - b[0].max(f[0]);
        let h = (b[1] + b[3]).min(f[1] + f[3]) - b[1].max(f[1]);
        w > 0.0 && h > 0.0 && w * h >= 0.5 * area
    })
}

/// The display equations of a page: [`equation_blocks_with_raster`] with
/// the page's floats (figures, tables, algorithms, found from their
/// captions in `runs`) kept out. An axis label set as `$\tau(s)$`, a row
/// of a table of numbers, a line of an algorithm are mathematics standing
/// apart from the prose, and no display.
pub fn page_equation_blocks(
    glyphs: &[GlyphDto],
    raster: Option<&vision::PageRaster>,
    runs: &[crate::engine::types::TextRun],
    page_w: f32,
    page_h: f32,
) -> Vec<EquationBlockDto> {
    // y-up to top-left
    let floats: Vec<[f32; 4]> = crate::engine::floats::float_regions(runs, page_w, page_h)
        .into_iter()
        .map(|[x, y, w, h]| [x, page_h - y - h, w, h])
        .collect();
    equation_blocks_with_raster(glyphs, raster, page_w, page_h, &floats)
}

/// Display equations of a page, from its glyphs and, when there is one,
/// its raster. The raster's displays are taken whole: their extent comes
/// from the ink, numerators, limits and every line of the display
/// included - unless that takes in a line of prose, or a second equation
/// the glyphs found apart. A block found from the glyphs alone is kept
/// where the raster has no display of its own over it. Blocks mostly inside
/// one of `floats` (`[x, y, w, h]`, top-left origin) are dropped, unless
/// numbered.
pub fn equation_blocks_with_raster(
    glyphs: &[GlyphDto],
    raster: Option<&vision::PageRaster>,
    page_w: f32,
    page_h: f32,
    floats: &[[f32; 4]],
) -> Vec<EquationBlockDto> {
    let mut from_glyphs = equation_blocks(glyphs, page_w, page_h);
    from_glyphs.retain(|b| !in_float(b, floats));
    let Some(raster) = raster else {
        return from_glyphs;
    };
    let chars: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| !g.is_rule() && g.w > 0.0)
        .collect();
    let lines = ink_lines(glyphs, raster, &gutters(&chars, page_w));
    let page_em = median(
        glyphs
            .iter()
            .filter(|g| !g.is_rule())
            .map(|g| g.size)
            .collect(),
    )
    .max(1.0);
    let prose = text_lines(&lines, page_em);
    let mut out = displays_from_lines(glyphs, &lines);
    out.retain(|b| !in_float(b, floats));
    // The glyphs' blocks as found, to tell a raster display over two of
    // them: two equations set close, which the raster took for one.
    let found: Vec<[f32; 4]> = from_glyphs.iter().map(|b| b.bbox).collect();
    for mut b in from_glyphs {
        let own = b.bbox;
        // A line of prose the raster saw is no part of a display: the
        // glyphs' outline is cut back from it, or dropped if inside one.
        for t in &prose {
            let x_share = (b.bbox[0] + b.bbox[2]).min(t[0] + t[2]) - b.bbox[0].max(t[0]);
            if !overlaps_pt(&b.bbox, t) || x_share < 0.5 * b.bbox[2].min(t[2]) {
                continue;
            }
            let (top, bottom) = (b.bbox[1], b.bbox[1] + b.bbox[3]);
            let (t_top, t_bottom) = (t[1], t[1] + t[3]);
            // Most of the block inside the "prose": the raster has run the
            // display into a line of text; trust the glyphs.
            if t_bottom.min(bottom) - t_top.max(top) > 0.5 * b.bbox[3] {
                continue;
            }
            if t_top - top > bottom - t_bottom {
                // The prose is in the lower part: end at its upper edge (a
                // little into it, where a limit tucked against it ends).
                b.bbox[3] = (t_top + 0.2 * page_em - top).max(0.0);
            } else {
                let new_top = t_bottom - 0.2 * page_em;
                b.bbox[3] = (bottom - new_top).max(0.0);
                b.bbox[1] = new_top;
            }
        }
        if b.bbox[3] <= 0.0 {
            continue;
        }
        // Where both found it, the union: the raster adds what the glyphs'
        // outline cut off, the glyphs a line the raster judged otherwise.
        // A glyph block over several raster lines (an `align` numbered line
        // by line) makes them one.
        let (hit, mut rest): (Vec<EquationBlockDto>, Vec<EquationBlockDto>) =
            out.into_iter().partition(|r| overlaps_pt(&r.bbox, &b.bbox));
        let mut merged = b;
        let mut numbers = Vec::new();
        for r in hit {
            // A raster line that ran the display into the line of prose
            // above or below it adds only its number.
            let around = merged.bbox;
            let outside: Vec<&GlyphDto> = chars
                .iter()
                .copied()
                .filter(|g| {
                    let c = centre_of(&drawn(g));
                    inside(c, &r.bbox) && !inside(c, &around)
                })
                .collect();
            let shared = found
                .iter()
                .any(|o| *o != own && overlaps_pt(o, &r.bbox) && !overlaps_pt(o, &own));
            if prose_words(&outside, page_em) < 4 && !shared {
                merged.bbox = union_pt(merged.bbox, r.bbox);
            }
            if !shared {
                numbers.push((r.bbox[1], r.number));
            }
        }
        if merged.number.is_none() {
            numbers.sort_by(|a, b| a.0.total_cmp(&b.0));
            merged.number = numbers.into_iter().find_map(|(_, n)| n);
        }
        rest.push(merged);
        out = rest;
    }
    out.sort_by(|a, b| a.bbox[1].total_cmp(&b.bbox[1]));
    out
}

// ---------------------------------------------------------------------------
// The glyphs of a region, grown and measured on the raster
// ---------------------------------------------------------------------------

/// Glyphs for a selection or an equation, with what the raster says of
/// their layout.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GlyphLayoutDto {
    /// The glyphs, with `ink` measured from the raster where it could be,
    /// and bars the raster found that the PDF did not draw as paths added
    /// as `__rule` glyphs.
    pub glyphs: Vec<GlyphDto>,
    /// `[x, y, w, h]` of each 2-D stack: a fraction (numerator, bar and
    /// denominator) or an operator with its limits, fused by dilation.
    pub groups: Vec<[f32; 4]>,
    /// `[x, y, w, h]` of each line, top down: strips of ink joined through
    /// their stacks. More than one is a multi-line display.
    pub lines: Vec<[f32; 4]>,
}

/// The glyphs of an equation whose block is `bbox`, tolerantly: any glyph
/// whose ink touches it; then, until nothing changes, the glyphs on the
/// same baseline as one already taken and next to it (the left-hand side
/// an outline missed), and those stacked on a bar already taken (its
/// numerator and denominator).
pub fn grow_region(all: &[GlyphDto], bbox: [f32; 4]) -> Vec<GlyphDto> {
    // The block's rows: the baselines of the glyphs drawn inside it.
    let rows: Vec<(f32, f32)> = all
        .iter()
        .filter(|g| !g.is_rule() && inside(centre_of(&drawn(g)), &bbox))
        .map(|g| (g.baseline, g.size.max(1.0)))
        .collect();
    let in_rows = |g: &GlyphDto| rows.iter().any(|&(b, s)| (g.baseline - b).abs() < 0.15 * s);
    let mut taken: Vec<bool> = all
        .iter()
        .map(|g| {
            if g.is_rule() {
                overlaps_pt(&[g.x, g.y, g.w, g.h.max(0.1)], &bbox)
            } else {
                let d = drawn(g);
                inside(centre_of(&d), &bbox) || (overlaps_pt(&d, &bbox) && in_rows(g))
            }
        })
        .collect();
    for _ in 0..6 {
        let mut grew = false;
        for (i, g) in all.iter().enumerate() {
            if taken[i] || g.is_rule() {
                continue;
            }
            let em = g.size.max(1.0);
            let d = drawn(g);
            // Its band must meet the block's: a line of text above or below
            // is not grown into.
            if d[1] > bbox[1] + bbox[3] + em || d[1] + d[3] < bbox[1] - em {
                continue;
            }
            let joins = all.iter().enumerate().any(|(j, o)| {
                if !taken[j] {
                    return false;
                }
                if o.is_rule() {
                    // Stacked on a bar: over or under it, within its length.
                    let (cx, _) = centre_of(&d);
                    let gap = (o.y - (d[1] + d[3])).max(d[1] - (o.y + o.h));
                    let stacked =
                        cx >= o.x && cx <= o.x + o.w && gap <= 0.45 * em && o.w >= 0.4 * em;
                    // Or on the line the bar sits on (its axis a quarter em
                    // over the baseline), beside it: "l =" before a fraction.
                    let beside = (d[0] - (o.x + o.w)).max(o.x - (d[0] + d[2])) <= 1.0 * em;
                    let on_axis = g.baseline >= o.y && g.baseline - o.y <= 0.6 * em;
                    return stacked || (beside && on_axis && o.w >= 0.4 * em);
                }
                let od = drawn(o);
                let level = (o.baseline - g.baseline).abs() < 0.15 * em;
                let gap = (od[0] - (d[0] + d[2])).max(d[0] - (od[0] + od[2]));
                level && gap <= 1.0 * em
            });
            if joins {
                taken[i] = true;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    all.iter()
        .zip(&taken)
        .filter(|(_, &t)| t)
        .map(|(g, _)| g.clone())
        .collect()
}

/// The layout of a region on the raster: every glyph's ink from the
/// connected components it covers, bars found by run-length analysis,
/// stacks by dilation, lines from the strips.
pub fn region_layout(mut glyphs: Vec<GlyphDto>, raster: &vision::PageRaster) -> GlyphLayoutDto {
    let chars: Vec<usize> = (0..glyphs.len())
        .filter(|&i| !glyphs[i].is_rule())
        .collect();
    if chars.is_empty() {
        return GlyphLayoutDto {
            glyphs,
            ..GlyphLayoutDto::default()
        };
    }
    let em = median(chars.iter().map(|&i| glyphs[i].size).collect()).max(1.0);
    let region = glyphs
        .iter()
        .map(|g| {
            if g.is_rule() {
                [g.x, g.y, g.w, g.h]
            } else {
                drawn(g)
            }
        })
        .reduce(union_pt)
        .expect("glyphs");
    let pad = 0.3 * em;
    let region_px = raster.to_px([
        region[0] - pad,
        region[1] - pad,
        region[2] + 2.0 * pad,
        region[3] + 2.0 * pad,
    ]);
    let comps = vision::components(&raster.bits, region_px);
    // Each component to the glyph whose drawn box it overlaps most.
    let boxes: Vec<vision::PxBox> = chars
        .iter()
        .map(|&i| raster.to_px(drawn(&glyphs[i])))
        .collect();
    let mut ink: Vec<Option<vision::PxBox>> = vec![None; chars.len()];
    let mut loose_parts: Vec<vision::PxBox> = Vec::new();
    let grown = |b: &vision::PxBox| {
        vision::PxBox::new(
            b.x0.saturating_sub(1),
            b.y0.saturating_sub(1),
            b.x1 + 1,
            b.y1 + 1,
        )
    };
    for c in &comps {
        let best = boxes
            .iter()
            .enumerate()
            .map(|(k, b)| (k, grown(b).intersection_area(&c.bbox)))
            .filter(|&(_, a)| a > 0)
            .max_by_key(|&(_, a)| a);
        // A component is a glyph's ink when it lies mostly in the glyph's
        // box, or holds the glyph's box (a minus sign whose PDF box is a
        // hairline); one reaching far outside it is not (a bar under a
        // letter, a radical's sign).
        let holds = |k: usize| {
            let b = boxes[k];
            let (cx, cy) = ((b.x0 + b.x1) / 2, (b.y0 + b.y1) / 2);
            cx >= c.bbox.x0
                && cx < c.bbox.x1 + 1
                && cy + 1 >= c.bbox.y0
                && cy < c.bbox.y1 + 1
                && c.bbox.w() <= b.w() + 4
        };
        match best {
            Some((k, a)) if a * 2 >= c.bbox.w() * c.bbox.h() || holds(k) => {
                ink[k] = Some(ink[k].map_or(c.bbox, |b| b.union(&c.bbox)));
            }
            _ => loose_parts.push(c.bbox),
        }
    }
    for (k, &i) in chars.iter().enumerate() {
        if let Some(b) = ink[k] {
            glyphs[i].ink = Some(raster.to_pt(&b));
        }
    }
    // Bars: thin wide components no glyph claims, and long thin runs; not
    // where the PDF already has a rule.
    let thin = raster.px(0.15 * em).max(2) + 1;
    let mut bars: Vec<vision::PxBox> = loose_parts
        .iter()
        .copied()
        .filter(|b| b.h() <= thin && b.w() >= 4 * b.h() && b.w() >= raster.px(0.4 * em))
        .collect();
    let char_inks: Vec<vision::PxBox> = chars
        .iter()
        .map(|&i| raster.to_px(drawn(&glyphs[i])))
        .collect();
    for r in vision::horizontal_rules(&raster.bits, region_px, raster.px(0.5 * em), thin) {
        // A bar is a component of its own: a run that is the foot of a ∑
        // or the bar of a letter belongs to a taller shape.
        let part_of_shape = comps.iter().any(|c| {
            c.bbox.x0 <= r.x0
                && c.bbox.x1 >= r.x1
                && c.bbox.y0 <= r.y0
                && c.bbox.y1 >= r.y1
                && c.bbox.h() > thin + 1
        });
        let in_glyph =
            part_of_shape || char_inks.iter().any(|i| grown(i).intersection_area(&r) > 0);
        if !in_glyph && !bars.iter().any(|b| b.intersection_area(&r) > 0) {
            bars.push(r);
        }
    }
    let existing: Vec<vision::PxBox> = glyphs
        .iter()
        .filter(|g| g.is_rule())
        .map(|g| raster.to_px([g.x, g.y, g.w, g.h.max(0.5)]))
        .collect();
    // A bar has something under it, a denominator or what it overlines,
    // among the glyphs; a stray stroke at the region's edge does not.
    let inner = raster.to_px(region);
    bars.retain(|b| {
        b.y0 >= inner.y0
            && b.y1 <= inner.y1
            && char_inks
                .iter()
                .any(|i| i.x_overlap(b) > 0 && i.y0 >= b.y1 && i.y0 - b.y1 <= raster.px(0.8 * em))
    });
    for b in &bars {
        let dup = existing.iter().any(|e| {
            e.x_overlap(b) * 2 >= b.w().min(e.w()) && e.y0 <= b.y1 + 2 && b.y0 <= e.y1 + 2
        });
        if !dup {
            let [x, y, w, h] = raster.to_pt(b);
            if let Some(rule) = rule_glyph(x, y, w, h.min(RULE_MAX_THICKNESS - 0.01)) {
                glyphs.push(rule);
            }
        }
    }
    // Stacks: a small vertical dilation fuses a numerator, its bar and its
    // denominator, an operator and its limits; only groups holding a bar
    // or a tall symbol are stacks.
    let all_boxes: Vec<vision::PxBox> = comps
        .iter()
        .map(|c| c.bbox)
        .chain(bars.iter().copied())
        .collect();
    let ids = vision::group_by_dilation(&all_boxes, 1, raster.px(0.3 * em));
    let tall = raster.px(1.3 * em);
    let mut groups: Vec<(vision::PxBox, bool, usize)> = Vec::new();
    for (k, b) in all_boxes.iter().enumerate() {
        let id = ids[k];
        if groups.len() <= id {
            groups.resize(id + 1, (*b, false, 0));
        }
        let g = &mut groups[id];
        g.0 = if g.2 == 0 { *b } else { g.0.union(b) };
        g.2 += 1;
        let is_bar = b.h() <= thin && b.w() >= 4 * b.h() && b.w() >= raster.px(0.4 * em);
        g.1 |= is_bar || b.h() >= tall;
    }
    let groups: Vec<[f32; 4]> = groups
        .into_iter()
        .filter(|(b, stack, n)| *stack && *n >= 2 && b.h() >= raster.px(1.2 * em))
        .map(|(b, _, _)| raster.to_pt(&b))
        .collect();
    // Lines: strips between blank rows, joined through stacks.
    let rows = vision::valleys(&vision::row_profile(&raster.bits, region_px), 1);
    let mut strips = Vec::new();
    let mut y = region_px.y0;
    for (s0, e0) in rows
        .iter()
        .map(|&(s0, e0)| (region_px.y0 + s0, region_px.y0 + e0))
    {
        if let Some(t) = vision::trim(
            &raster.bits,
            vision::PxBox::new(region_px.x0, y, region_px.x1, s0),
        ) {
            strips.push(t);
        }
        y = e0;
    }
    if let Some(t) = vision::trim(
        &raster.bits,
        vision::PxBox::new(region_px.x0, y, region_px.x1, region_px.y1),
    ) {
        strips.push(t);
    }
    let rule_boxes: Vec<vision::PxBox> = existing.iter().chain(bars.iter()).copied().collect();
    let lines = vision::logical_lines(
        &strips,
        &rule_boxes,
        raster.px(em),
        raster.px(0.7 * em),
        raster.px(0.35 * em),
    )
    .into_iter()
    .map(|l| {
        let b = l
            .iter()
            .map(|&i| strips[i])
            .reduce(|a, b| a.union(&b))
            .expect("a strip");
        raster.to_pt(&b)
    })
    .collect();
    GlyphLayoutDto {
        glyphs,
        groups,
        lines,
    }
}

/// The glyphs in `rects` (a selection) or, with `grow`, of the equation
/// whose block is `rects[0]`, laid out on the raster when there is one.
pub fn glyph_layout(
    all: &[GlyphDto],
    raster: Option<&vision::PageRaster>,
    rects: &[[f32; 4]],
    grow: bool,
) -> GlyphLayoutDto {
    let glyphs = match (grow, rects.first()) {
        (true, Some(&bbox)) => grow_region(all, bbox),
        _ => select_in_rects(all, rects),
    };
    match raster {
        Some(r) if !rects.is_empty() && !glyphs.is_empty() => region_layout(glyphs, r),
        _ => GlyphLayoutDto {
            glyphs,
            ..GlyphLayoutDto::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(text: &str, font: &str, size: f32, x: f32, baseline: f32) -> GlyphDto {
        let w = 0.5 * size * text.chars().count() as f32;
        GlyphDto {
            text: text.to_string(),
            code: text.chars().next().map_or(0, |c| c as u32),
            font: font.to_string(),
            size,
            x,
            y: baseline - 0.9 * size,
            w,
            h: 1.2 * size,
            baseline,
            bold: false,
            italic: false,
            ink: None,
        }
    }

    /// One glyph per character, packed left to right from `x`.
    fn set(text: &str, font: &str, x: f32, baseline: f32) -> Vec<GlyphDto> {
        let mut out = Vec::new();
        let mut at = x;
        for c in text.chars() {
            if c == ' ' {
                at += 3.0;
                continue;
            }
            out.push(glyph(&c.to_string(), font, 10.0, at, baseline));
            at += 5.0;
        }
        out
    }

    /// Body text filling the column 72..540 on a 612-wide page.
    fn body_line(baseline: f32) -> Vec<GlyphDto> {
        let words = "the quick brown fox jumps over the lazy dog and keeps running far ".repeat(2);
        let mut line = set(&words, "Times-Roman", 72.0, baseline);
        line.retain(|g| g.x + g.w <= 540.0);
        line
    }

    #[test]
    fn strips_only_a_real_subset_tag() {
        assert_eq!(strip_subset_prefix("EWJNXA+CMMI10"), "CMMI10");
        assert_eq!(strip_subset_prefix("CMMI10"), "CMMI10");
        assert_eq!(strip_subset_prefix("PwkwqfRgwrxjMTMI"), "MTMI");
        assert_eq!(
            strip_subset_prefix("CylqyyWbfffhTimes-Roman"),
            "Times-Roman"
        );
        assert_eq!(strip_subset_prefix("TimesNewRoman"), "TimesNewRoman");
        assert_eq!(
            strip_subset_prefix("XnmylnHcvjmcSvysrsSfgwymAdvPSSym"),
            "AdvPSSym"
        );
        assert_eq!(strip_subset_prefix("abcdef+CMMI10"), "abcdef+CMMI10");
        assert_eq!(strip_subset_prefix("ABCDE+CMMI10"), "ABCDE+CMMI10");
        assert_eq!(
            strip_subset_prefix("ABCDEF+STIXTwoMath-Regular"),
            "STIXTwoMath-Regular"
        );
    }

    #[test]
    fn italic_faces_by_name() {
        assert!(is_italic_font_name("CMMI10"));
        assert!(is_italic_font_name("Times-Italic"));
        assert!(is_italic_font_name("MinionPro-It"));
        assert!(is_italic_font_name("Helvetica-Oblique"));
        assert!(!is_italic_font_name("CMR10"));
        assert!(!is_italic_font_name("CMSY10"));
    }

    #[test]
    fn math_fonts_include_the_opentype_math_faces() {
        for name in [
            "CMMI10",
            "CMEX10",
            "MSBM10",
            "CambriaMath",
            "STIXTwoMath-Regular",
            "XITSMath",
            "SymbolMT",
            "MTSymbol",
            "MathematicalPi-Six",
        ] {
            assert!(is_math_font_name(name), "{name}");
        }
        assert!(!is_math_font_name("CMR10"));
        assert!(!is_math_font_name("Times-Roman"));
        assert!(!is_math_font_name("AppleSymbols"));
    }

    #[test]
    fn thin_wide_paths_are_rules_and_others_are_not() {
        let bar = rule_glyph(100.0, 200.0, 30.0, 0.4).expect("a fraction bar");
        assert_eq!(bar.font, RULE_FONT);
        assert_eq!(bar.text, "");
        assert_eq!(bar.code, 0);
        assert!((bar.baseline - 200.2).abs() < 1e-4);
        // A hairline with no height at all is still a rule.
        assert!(rule_glyph(0.0, 0.0, 10.0, 0.0).is_some());
        // A stroked bar's bounds include its line width twice over.
        assert!(rule_glyph(63.5, 385.4, 5.04, 1.05).is_some());
        // Too thick, too short, or a dot.
        assert!(rule_glyph(0.0, 0.0, 30.0, 2.0).is_none());
        assert!(rule_glyph(0.0, 0.0, 2.0, 0.8).is_none());
        assert!(rule_glyph(0.0, 0.0, 0.5, 0.0).is_none());
    }

    #[test]
    fn selects_characters_by_centre_and_rules_by_touch() {
        let inside = glyph("x", "CMMI10", 10.0, 10.0, 20.0); // centre (12.5, 17)
        let outside = glyph("y", "CMMI10", 10.0, 50.0, 20.0);
        let straddling = glyph("z", "CMMI10", 10.0, 28.0, 20.0); // centre 30.5
        let bar = rule_glyph(25.0, 15.0, 40.0, 0.4).unwrap();
        let glyphs = vec![inside.clone(), outside, straddling, bar.clone()];
        let picked = select_in_rects(&glyphs, &[[0.0, 0.0, 30.0, 30.0]]);
        assert_eq!(picked, vec![inside, bar]);
        assert_eq!(select_in_rects(&glyphs, &[]).len(), 4);
    }

    fn leaf(tag: &str, mcid: i32) -> TagNode {
        TagNode {
            tag: tag.into(),
            mcids: vec![mcid],
            ..TagNode::default()
        }
    }

    fn element(tag: &str, children: Vec<TagNode>) -> TagNode {
        TagNode {
            tag: tag.into(),
            children,
            ..TagNode::default()
        }
    }

    #[test]
    fn collects_formulas_with_alt_bbox_and_escaped_mathml() {
        // x < 2 as MathML, inside a Formula carrying Alt text.
        let math = element(
            "math",
            vec![element(
                "mrow",
                vec![leaf("mi", 0), leaf("mo", 1), leaf("mn", 2)],
            )],
        );
        let formula = TagNode {
            tag: "Formula".into(),
            alt: Some("x less than 2".into()),
            children: vec![math],
            ..TagNode::default()
        };
        let doc = element(
            "Document",
            vec![element("P", vec![leaf("Span", 3)]), formula],
        );
        let by_mcid: HashMap<i32, Vec<u32>> =
            [(0, vec![0]), (1, vec![1]), (2, vec![2]), (3, vec![3])].into();
        let chars = vec![
            ('x', Some([10.0, 20.0, 5.0, 8.0])),
            ('<', Some([16.0, 21.0, 6.0, 6.0])),
            ('2', Some([23.0, 20.0, 5.0, 8.0])),
            ('A', Some([200.0, 300.0, 5.0, 8.0])),
        ];
        let tags = collect_formulas(&[doc], &by_mcid, &chars);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].alt.as_deref(), Some("x less than 2"));
        assert_eq!(tags[0].actual_text, None);
        assert_eq!(tags[0].bbox, [10.0, 20.0, 18.0, 8.0]);
        assert_eq!(
            tags[0].mathml.as_deref(),
            Some(
                "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mrow><mi>x</mi>\
                 <mo>&lt;</mo><mn>2</mn></mrow></math>"
            )
        );
    }

    #[test]
    fn a_page_without_a_formula_is_known_before_its_text_is_read() {
        let prose = element("Document", vec![element("P", vec![leaf("Span", 0)])]);
        assert!(!has_formula(std::slice::from_ref(&prose)));
        let nested = element("Document", vec![element("Div", vec![leaf("math", 1)])]);
        assert!(has_formula(&[prose, nested]));
    }

    #[test]
    fn a_bare_math_root_counts_and_a_formula_without_mathml_has_none() {
        let bare = element("math", vec![leaf("mi", 0)]);
        let plain = TagNode {
            tag: "Formula".into(),
            actual_text: Some("E = mc^2".into()),
            mcids: vec![1],
            ..TagNode::default()
        };
        let by_mcid: HashMap<i32, Vec<u32>> = [(0, vec![0]), (1, vec![1])].into();
        let chars = vec![('y', Some([1.0, 2.0, 3.0, 4.0])), ('E', None)];
        let tags = collect_formulas(&[bare, plain], &by_mcid, &chars);
        assert_eq!(tags.len(), 2);
        assert!(tags[0].mathml.as_deref().unwrap().contains("<mi>y</mi>"));
        assert_eq!(tags[1].mathml, None);
        assert_eq!(tags[1].actual_text.as_deref(), Some("E = mc^2"));
        // No character with a box: reported for its text, with an empty box.
        assert_eq!(tags[1].bbox, [0.0; 4]);
    }

    #[test]
    fn an_untagged_formula_with_nothing_to_say_is_skipped() {
        let empty = element("Formula", vec![]);
        assert!(collect_formulas(&[empty], &HashMap::new(), &[]).is_empty());
        assert_eq!(xml_name("bad tag"), "mrow");
        assert_eq!(xml_escape("a&b\"'>"), "a&amp;b&quot;&apos;&gt;");
    }

    /// A page: body text, a centred display fraction numbered (3), more body
    /// text with inline math.
    fn page() -> Vec<GlyphDto> {
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        // f(x) = a / b    (3)
        // numerator "a" over a bar over denominator "b", with f(x) = on the
        // main line.
        glyphs.extend(set("f", "CMMI10", 280.0, 180.0));
        glyphs.extend(set("(", "CMR10", 285.0, 180.0));
        glyphs.extend(set("x", "CMMI10", 290.0, 180.0));
        glyphs.extend(set(")", "CMR10", 295.0, 180.0));
        glyphs.extend(set("=", "CMR10", 303.0, 180.0));
        glyphs.extend(set("a+1", "CMR10", 313.0, 173.0));
        glyphs.push(rule_glyph(312.0, 177.0, 17.0, 0.4).unwrap());
        glyphs.extend(set("b", "CMMI10", 318.0, 189.0));
        glyphs.extend(set("(3)", "CMR10", 525.0, 180.0));
        // Running text with an inline x: never a display.
        let mut inline = body_line(220.0);
        for g in inline.iter_mut().skip(10).take(1) {
            g.font = "CMMI10".into();
        }
        glyphs.extend(inline);
        glyphs.extend(body_line(232.0));
        glyphs
    }

    #[test]
    fn finds_a_numbered_display_fraction_and_not_inline_math() {
        let blocks = equation_blocks(&page(), 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let block = &blocks[0];
        assert_eq!(block.number.as_deref(), Some("3"));
        let [x, y, w, h] = block.bbox;
        // From f to the end of the bar, without the number.
        assert!((x - 280.0).abs() < 0.01, "{x}");
        assert!(x + w < 340.0 && x + w >= 329.0, "{}", x + w);
        // Numerator to denominator.
        assert!(y < 173.0 - 5.0 && y + h > 189.0, "{y} {h}");
    }

    #[test]
    fn merges_the_lines_of_an_align() {
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("x=y+z", "CMMI10", 280.0, 180.0));
        glyphs.extend(set("y=2z", "CMMI10", 285.0, 194.0));
        glyphs.extend(set("(1.2)", "CMR10", 520.0, 187.0));
        for i in 0..3 {
            glyphs.extend(body_line(220.0 + 12.0 * i as f32));
        }
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].number.as_deref(), Some("1.2"));
        let [_, y, _, h] = blocks[0].bbox;
        assert!(y < 175.0 && y + h > 194.0);
    }

    #[test]
    fn a_left_aligned_math_line_is_not_a_display() {
        // A paragraph's last line, all mathematics, one line under the text.
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("x=y+z", "CMMI10", 72.0, 148.0));
        assert!(equation_blocks(&glyphs, 612.0, 792.0).is_empty());
        // Set apart from the text, flush left, numbered: a `fleqn` display,
        // the way Elsevier sets them.
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("x=y+z", "CMMI10", 72.0, 168.0));
        glyphs.extend(set("(4)", "CMR10", 525.0, 168.0));
        glyphs.extend(body_line(190.0));
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].number.as_deref(), Some("4"));
    }

    /// Words from `x0` to about `x1`, the way a justified column sets them.
    fn text_line(x0: f32, x1: f32, baseline: f32) -> Vec<GlyphDto> {
        let words = "the quick brown fox jumps over the lazy dog and keeps running far ".repeat(3);
        let mut line = set(&words, "Times-Roman", x0, baseline);
        line.retain(|g| g.x + g.w <= x1);
        line
    }

    #[test]
    fn two_columns_level_only_in_part_stay_apart() {
        // A two-column page whose right column's baselines sit 5 pt under
        // the left's: half a line, which a page-wide row would join.
        let mut glyphs = Vec::new();
        for i in 0..12 {
            glyphs.extend(text_line(54.0, 290.0, 100.0 + 12.0 * i as f32));
        }
        for i in 0..4 {
            glyphs.extend(text_line(320.0, 560.0, 105.0 + 12.0 * i as f32));
        }
        // A display in the right column, with a superscript level with
        // nothing in the left column, and its number.
        glyphs.extend(set("y=x", "CMMI10", 410.0, 168.0));
        let mut sup = glyph("2", "CMR7", 7.0, 425.0, 164.0);
        sup.w = 3.5;
        glyphs.push(sup);
        glyphs.extend(set("(5)", "CMR10", 540.0, 168.0));
        for i in 0..3 {
            glyphs.extend(text_line(320.0, 560.0, 189.0 + 12.0 * i as f32));
        }
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].number.as_deref(), Some("5"));
        let [x, y, w, h] = blocks[0].bbox;
        assert!(x >= 409.0 && x + w < 440.0, "{x} {w}");
        assert!(y > 155.0 && y + h < 175.0, "{y} {h}");
    }

    #[test]
    fn a_line_ending_in_a_colon_introduces_a_display_and_is_not_part_of_it() {
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        // "... (writing L = M):" as the paragraph's last, mathematical line.
        glyphs.extend(set("(L=M):", "CMMI10", 72.0, 148.0));
        glyphs.extend(set("x=y+z", "CMMI10", 280.0, 166.0));
        glyphs.extend(set("(2)", "CMR10", 525.0, 166.0));
        glyphs.extend(body_line(186.0));
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let [x, y, ..] = blocks[0].bbox;
        assert!(x >= 279.0 && y > 155.0, "{:?}", blocks[0].bbox);
    }

    #[test]
    fn digits_and_an_operator_in_the_text_font_are_mathematics() {
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        // "0 = a - b", the "0 =" and "-" in the roman font.
        glyphs.extend(set("0=", "CMR10", 270.0, 170.0));
        glyphs.extend(set("a", "CMMI10", 285.0, 170.0));
        glyphs.extend(set("+", "CMR10", 300.0, 170.0));
        glyphs.extend(set("b", "CMMI10", 315.0, 170.0));
        glyphs.extend(body_line(190.0));
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert!(
            (blocks[0].bbox[0] - 270.0).abs() < 0.5,
            "{:?}",
            blocks[0].bbox
        );
    }

    #[test]
    fn a_centred_line_of_operator_names_is_still_a_display() {
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        // "err\Big(f\Big) = Omega", most of its symbols letters of the text
        // font, the big parentheses from the extension font, unmapped.
        let big = |x: f32| {
            let mut g = glyph("", "CMEX10", 10.0, x, 161.0);
            g.w = 5.0;
            g.ink = Some([x, 160.5, 5.0, 13.0]);
            g
        };
        glyphs.extend(set("err", "CMR10", 260.0, 170.0));
        glyphs.push(big(275.0));
        glyphs.extend(set("f", "CMMI10", 280.0, 170.0));
        glyphs.push(big(285.0));
        glyphs.extend(set("=Omega", "CMR10", 292.0, 170.0));
        glyphs.extend(body_line(190.0));
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
    }

    /// A raster at 2 px/pt with every glyph's box inked (a glyph's ink is
    /// its band from 0.7 em above the baseline to the baseline), and these
    /// extra point boxes, bars drawn as no PDF path.
    fn raster_of(glyphs: &[GlyphDto], extra: &[[f32; 4]], w: f32, h: f32) -> vision::PageRaster {
        let scale = 2.0;
        let mut bits = vision::Bits::new((w * scale) as usize, (h * scale) as usize);
        let mut paint = |r: [f32; 4]| {
            let x0 = (r[0] * scale) as usize;
            let y0 = (r[1] * scale) as usize;
            let x1 = (((r[0] + r[2]) * scale).ceil() as usize).min(bits.w);
            let y1 = (((r[1] + r[3]) * scale).ceil() as usize).min(bits.h);
            for y in y0..y1 {
                for x in x0..x1 {
                    bits.set(x, y, true);
                }
            }
        };
        for g in glyphs {
            if g.is_rule() {
                paint([g.x, g.y, g.w, g.h.max(0.5)]);
            } else {
                // Letters with a little space between them.
                paint([
                    g.x + 0.5,
                    g.baseline - 0.7 * g.size,
                    (g.w - 1.0).max(0.5),
                    0.7 * g.size,
                ]);
            }
        }
        for r in extra {
            paint(*r);
        }
        vision::PageRaster { bits, scale }
    }

    #[test]
    fn an_equation_grows_to_its_numerators_and_left_hand_side() {
        // l = (a+b) / c, the bar a PDF rule.
        let mut glyphs = set("l=", "CMMI10", 100.0, 200.0);
        glyphs.extend(set("a+b", "CMMI10", 120.0, 192.0));
        glyphs.push(rule_glyph(118.0, 196.0, 18.0, 0.4).unwrap());
        glyphs.extend(set("c", "CMMI10", 125.0, 208.0));
        // A text line well above.
        glyphs.extend(set("text", "Times-Roman", 100.0, 170.0));
        // The block as an outline clamped to the bar's band would give it.
        let grown = grow_region(&glyphs, [117.0, 195.0, 20.0, 15.0]);
        let texts: String = grown.iter().map(|g| g.text.as_str()).collect();
        assert!(
            texts.contains("l=") && texts.contains("a+b") && texts.contains('c'),
            "{texts}"
        );
        assert!(!texts.contains("text"), "{texts}");
    }

    #[test]
    fn region_layout_measures_ink_finds_a_bar_and_the_stack() {
        let mut glyphs = set("x=", "CMMI10", 100.0, 200.0);
        glyphs.extend(set("ab", "CMMI10", 120.0, 192.0));
        glyphs.extend(set("c", "CMMI10", 123.0, 208.0));
        // The bar is drawn, but not as a path: only the raster has it.
        let raster = raster_of(&glyphs, &[[118.0, 195.5, 14.0, 0.5]], 300.0, 300.0);
        let layout = region_layout(glyphs.clone(), &raster);
        let rules: Vec<&GlyphDto> = layout.glyphs.iter().filter(|g| g.is_rule()).collect();
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert!(
            (rules[0].x - 118.0).abs() < 1.0 && (rules[0].w - 14.0).abs() < 1.5,
            "{rules:?}"
        );
        // Ink from the raster: the letter's band.
        let a = layout.glyphs.iter().find(|g| g.text == "a").unwrap();
        let [_, iy, _, ih] = a.ink.unwrap();
        assert!(
            (iy - 185.0).abs() < 1.0 && (ih - 7.0).abs() < 1.0,
            "{:?}",
            a.ink
        );
        // One stack: numerator, bar, denominator.
        assert_eq!(layout.groups.len(), 1, "{:?}", layout.groups);
        let [gx, gy, gw, gh] = layout.groups[0];
        assert!(gx <= 118.5 && gx + gw >= 131.5 && gy <= 185.5 && gy + gh >= 207.5);
        assert_eq!(layout.lines.len(), 1, "{:?}", layout.lines);
    }

    #[test]
    fn raster_displays_find_a_two_line_display_whole() {
        // A column of text, then two centred lines of a display, the second
        // with a fraction, then text again.
        let mut glyphs = Vec::new();
        for i in 0..5 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("lnx+lnN=y", "CMMI10", 250.0, 172.0));
        glyphs.extend(set("l=", "CMMI10", 240.0, 194.0));
        glyphs.extend(set("lnN", "CMMI10", 262.0, 187.0));
        glyphs.push(rule_glyph(260.0, 191.0, 20.0, 0.4).unwrap());
        glyphs.extend(set("k", "CMMI10", 268.0, 202.0));
        for i in 0..4 {
            glyphs.extend(body_line(224.0 + 12.0 * i as f32));
        }
        let raster = raster_of(&glyphs, &[], 612.0, 400.0);
        let blocks = raster_displays(&glyphs, &raster);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let [x, y, w, h] = blocks[0].bbox;
        // From the first line's top to the denominator's foot, "l =" included.
        assert!(y <= 166.0 && y + h >= 201.0, "{:?}", blocks[0].bbox);
        assert!(x <= 241.0 && x + w >= 279.0, "{:?}", blocks[0].bbox);
    }

    #[test]
    fn surrogate_halves_make_one_character() {
        // U+1D44C MATHEMATICAL ITALIC CAPITAL Y, as PDFium reports it.
        assert_eq!(surrogate_pair(0xD835, 0xDC4C), 0x1D44C);
        assert_eq!(char::from_u32(surrogate_pair(0xD835, 0xDF00)), Some('𝜀'));
    }

    #[test]
    fn equation_numbers_look_like_numbers() {
        assert_eq!(equation_number("(3)").as_deref(), Some("3"));
        assert_eq!(equation_number("(A.1)").as_deref(), Some("A.1"));
        assert_eq!(equation_number("(2.3a)").as_deref(), Some("2.3a"));
        assert_eq!(equation_number("(x)"), None);
        assert_eq!(equation_number("3"), None);
        assert_eq!(equation_number("(see the appendix 3)"), None);
    }

    // ---------- display regressions, modelled on real pages ----------

    /// A two-column page: `lines` lines of text in each column, the left
    /// from 54 to 290, the right from 320 to 556, from `top` down.
    fn two_columns(top: f32, lines: usize) -> Vec<GlyphDto> {
        let mut glyphs = Vec::new();
        for i in 0..lines {
            glyphs.extend(text_line(54.0, 290.0, top + 12.0 * i as f32));
            glyphs.extend(text_line(320.0, 556.0, top + 12.0 * i as f32));
        }
        glyphs
    }

    #[test]
    fn two_numbered_equations_set_close_keep_their_own_limits() {
        // Two displays one under the other (IEEE's narrow column), each
        // numbered; the first's limits sit closer to it than to the second
        // equation's numerator, which used to take them and both lines.
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("R=", "CMMI10", 250.0, 200.0));
        let mut sum = glyph("\u{2211}", "CMEX10", 10.0, 262.0, 200.0);
        sum.ink = Some([262.0, 190.0, 8.0, 13.0]);
        glyphs.push(sum);
        glyphs.extend(set("f", "CMMI10", 272.0, 200.0));
        glyphs.push(glyph("i=1", "CMR7", 7.0, 260.0, 209.0));
        glyphs.extend(set("(2)", "CMR10", 525.0, 200.0));
        glyphs.extend(set("1", "CMR10", 290.0, 222.0));
        glyphs.push(rule_glyph(286.0, 225.0, 14.0, 0.4).unwrap());
        glyphs.extend(set("s", "CMMI10", 290.0, 235.0));
        glyphs.extend(set("S=2+", "CMMI10", 240.0, 229.0));
        glyphs.extend(set("(3)", "CMR10", 525.0, 229.0));
        for i in 0..3 {
            glyphs.extend(body_line(260.0 + 12.0 * i as f32));
        }
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        let numbers: Vec<Option<&str>> = blocks.iter().map(|b| b.number.as_deref()).collect();
        assert_eq!(numbers, [Some("2"), Some("3")], "{blocks:?}");
        // The limits are the first equation's.
        let [_, y, _, h] = blocks[0].bbox;
        assert!(y + h >= 209.0 && y + h < 214.0, "{:?}", blocks[0].bbox);
        assert!(blocks[1].bbox[1] > 210.0, "{:?}", blocks[1].bbox);
    }

    #[test]
    fn rows_numbered_line_by_line_are_one_display_when_aligned() {
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        // An `align`: the second row's "=" under the first's.
        glyphs.extend(set("x=y+z", "CMMI10", 280.0, 180.0));
        glyphs.extend(set("=2z", "CMMI10", 285.0, 194.0));
        glyphs.extend(set("(1)", "CMR10", 525.0, 180.0));
        glyphs.extend(set("(2)", "CMR10", 525.0, 194.0));
        // A `gather`: each row centred on its own.
        glyphs.extend(set("a=b", "CMMI10", 295.0, 260.0));
        glyphs.extend(set("cd+ef=g", "CMMI10", 285.0, 274.0));
        glyphs.extend(set("(3)", "CMR10", 525.0, 260.0));
        glyphs.extend(set("(4)", "CMR10", 525.0, 274.0));
        for i in 0..3 {
            glyphs.extend(body_line(300.0 + 12.0 * i as f32));
        }
        let mut text = body_line(220.0);
        text.extend(body_line(232.0));
        glyphs.extend(text);
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        let numbers: Vec<Option<&str>> = blocks.iter().map(|b| b.number.as_deref()).collect();
        assert_eq!(numbers, [Some("1"), Some("3"), Some("4")], "{blocks:?}");
        let [_, y, _, h] = blocks[0].bbox;
        assert!(y < 175.0 && y + h > 194.0, "{:?}", blocks[0].bbox);
    }

    #[test]
    fn displays_level_in_two_columns_stay_two() {
        let mut glyphs = two_columns(100.0, 5);
        glyphs.extend(set("x=y+z", "CMMI10", 140.0, 172.0));
        glyphs.extend(set("(1)", "CMR10", 275.0, 172.0));
        glyphs.extend(set("u=v-w", "CMMI10", 410.0, 172.0));
        glyphs.extend(set("(5)", "CMR10", 541.0, 172.0));
        glyphs.extend(two_columns(196.0, 5));
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        let numbers: Vec<Option<&str>> = blocks.iter().map(|b| b.number.as_deref()).collect();
        assert_eq!(numbers, [Some("1"), Some("5")], "{blocks:?}");
        assert!(blocks.iter().all(|b| b.bbox[2] < 60.0), "{blocks:?}");
    }

    #[test]
    fn a_number_pushed_under_its_long_row_still_numbers_it() {
        // A row as wide as its narrow column, its number on the line below
        // (2512.24540, eq. 48).
        let mut glyphs = two_columns(100.0, 5);
        glyphs.extend(set(
            "fgh=pq+rs+tu+vw+xy+ab+cd+ef+gh+ij+kl+mn+op+qr+st",
            "CMMI10",
            54.0,
            172.0,
        ));
        glyphs.extend(set("(48)", "CMR10", 272.0, 184.0));
        glyphs.extend(text_line(320.0, 556.0, 172.0));
        glyphs.extend(text_line(320.0, 556.0, 184.0));
        glyphs.extend(two_columns(208.0, 5));
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].number.as_deref(), Some("48"));
        let [x, y, w, h] = blocks[0].bbox;
        assert!(x <= 55.0 && x + w > 280.0, "{:?}", blocks[0].bbox);
        // The number is no part of the box.
        assert!(y + h < 180.0, "{:?}", blocks[0].bbox);
    }

    #[test]
    fn a_number_on_the_left_numbers_its_display() {
        // `leqno`: "(6)" at the left margin, level with the equation.
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("(6)", "CMR10", 72.0, 180.0));
        glyphs.extend(set("x=y+z", "CMMI10", 280.0, 180.0));
        for i in 0..3 {
            glyphs.extend(body_line(210.0 + 12.0 * i as f32));
        }
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].number.as_deref(), Some("6"));
        assert!(blocks[0].bbox[0] >= 279.0, "{:?}", blocks[0].bbox);
    }

    #[test]
    fn a_list_item_with_a_formula_is_no_display() {
        // "Exact Agnostic: for all finite S, L(S) <= inf L(c)." set apart
        // as an item, near the middle of the column (attias25a, p. 5).
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set(
            "Exact Agnostic: For all finite",
            "Times-Bold",
            140.0,
            174.0,
        ));
        glyphs.extend(set("S,L(S)<=infL(c).", "CMMI10", 282.0, 174.0));
        for i in 0..3 {
            glyphs.extend(body_line(196.0 + 12.0 * i as f32));
        }
        assert!(equation_blocks(&glyphs, 612.0, 792.0).is_empty());
    }

    #[test]
    fn scripts_split_off_a_line_of_prose_are_no_display() {
        // An inline "A_i" lowered by a delimiter falls into a row of its own
        // under its line; it is the line's, not a display.
        let mut glyphs = Vec::new();
        for i in 0..6 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("A", "CMMI10", 300.0, 166.0));
        glyphs.push(glyph("i", "CMMI7", 7.0, 306.0, 168.0));
        glyphs.extend(set("B", "CMMI10", 320.0, 166.0));
        glyphs.push(glyph("i", "CMMI7", 7.0, 326.0, 168.0));
        assert!(equation_blocks(&glyphs, 612.0, 792.0).is_empty());
    }

    #[test]
    fn upright_superscripts_and_operator_names_are_no_words() {
        // A display set flush with the column's edge (`fleqn`, a long row),
        // its exponent and the "eq" over each density in the text font, its
        // "log" upright: mathematics, not a line of words.
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        let mut x = 72.0;
        for _ in 0..4 {
            glyphs.extend(set("p(t)=", "CMMI10", x, 180.0));
            glyphs.extend(set("log", "CMR10", x + 26.0, 180.0));
            glyphs.push(glyph("eq", "CMR7", 7.0, x + 42.0, 176.0));
            glyphs.push(glyph("Lu", "CMMI7", 7.0, x + 50.0, 176.0));
            x += 70.0;
        }
        glyphs.extend(set("(51)", "CMR10", 520.0, 180.0));
        for i in 0..3 {
            glyphs.extend(body_line(204.0 + 12.0 * i as f32));
        }
        let blocks = equation_blocks(&glyphs, 612.0, 792.0);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!(blocks[0].number.as_deref(), Some("51"));
    }

    #[test]
    fn a_floats_labels_are_no_display() {
        // "tau(s)" centred under a plot: mathematics apart from the text,
        // inside a figure.
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("\u{03C4}(s)", "CMMI10", 296.0, 300.0));
        for i in 0..3 {
            glyphs.extend(body_line(340.0 + 12.0 * i as f32));
        }
        assert_eq!(
            equation_blocks_with_raster(&glyphs, None, 612.0, 792.0, &[]).len(),
            1
        );
        let figure = [72.0, 160.0, 468.0, 170.0];
        assert!(equation_blocks_with_raster(&glyphs, None, 612.0, 792.0, &[figure]).is_empty());
    }

    #[test]
    fn gutters_part_columns_and_margins_of_line_numbers() {
        let glyphs = two_columns(100.0, 20);
        let chars: Vec<&GlyphDto> = glyphs.iter().collect();
        let found = gutters(&chars, 612.0);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].0 >= 285.0 && found[0].1 <= 325.0, "{found:?}");
        // One column, with a review copy's line numbers in the margin.
        let mut glyphs = Vec::new();
        for i in 0..20 {
            let baseline = 100.0 + 12.0 * i as f32;
            glyphs.extend(body_line(baseline));
            glyphs.extend(set(&format!("{}", 100 + i), "Times-Roman", 30.0, baseline));
        }
        let chars: Vec<&GlyphDto> = glyphs.iter().collect();
        let found = gutters(&chars, 612.0);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].1 <= 73.0, "{found:?}");
        // One column alone has none.
        let glyphs: Vec<GlyphDto> = (0..20)
            .flat_map(|i| body_line(100.0 + 12.0 * i as f32))
            .collect();
        let chars: Vec<&GlyphDto> = glyphs.iter().collect();
        assert!(gutters(&chars, 612.0).is_empty());
    }

    #[test]
    fn a_raster_line_across_two_columns_is_cut_at_the_gutter() {
        let gutters = [(291.0, 319.0)];
        let mut members = set("x=y", "CMMI10", 140.0, 172.0);
        members.extend(text_line(320.0, 556.0, 172.0));
        let refs: Vec<&GlyphDto> = members.iter().collect();
        let pieces = split_at_gutters([140.0, 162.0, 416.0, 14.0], &refs, &gutters);
        let sides: Vec<Option<usize>> = pieces.iter().map(|p| p.2).collect();
        assert_eq!(sides, [Some(0), Some(1)]);
        assert!(pieces[0].0[0] + pieces[0].0[2] < 160.0, "{:?}", pieces[0].0);
        // A display set across both columns stays whole.
        let wide = set(
            "a+b+c+d+e+f+g+h+i+j+k+l+m+n+o+p+q+r+s",
            "CMMI10",
            200.0,
            172.0,
        );
        let refs: Vec<&GlyphDto> = wide.iter().collect();
        let pieces = split_at_gutters([200.0, 162.0, 190.0, 14.0], &refs, &gutters);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].2, None);
    }

    #[test]
    fn words_are_counted_at_their_own_size() {
        // A footnote's words are words; a superscript's letters and an
        // operator's name are not.
        let footnote = set("case, we can interpret it", "Times-Roman", 72.0, 700.0);
        let small: Vec<GlyphDto> = footnote
            .into_iter()
            .map(|mut g| {
                g.size = 8.0;
                g
            })
            .collect();
        let refs: Vec<&GlyphDto> = small.iter().collect();
        assert_eq!(prose_words(&refs, 10.0), 3);
        let mut math = set("log", "CMR10", 72.0, 200.0);
        math.push(glyph("max", "CMR7", 7.0, 90.0, 203.0));
        let refs: Vec<&GlyphDto> = math.iter().collect();
        assert_eq!(prose_words(&refs, 10.0), 0);
    }

    #[test]
    fn symbol_tags_are_numbers() {
        assert_eq!(equation_number("(\u{22C6})").as_deref(), Some("\u{22C6}"));
        assert_eq!(equation_number("(**)").as_deref(), Some("**"));
        assert_eq!(equation_number("(a)"), None);
    }

    // ---------- through PDFium ----------

    use crate::engine::{pdfium_init, render};
    use pdfium_render::prelude::Pdfium;
    use std::io::Write as _;

    /// A page with "a" over a thin filled bar over "b" in Times-Italic,
    /// centred, then "(1)" in Times-Roman at the right margin.
    fn fraction_pdf() -> Vec<u8> {
        let content = "BT /F1 12 Tf 300 712 Td (a) Tj ET \
                       298 708 12 0.5 re f \
                       BT /F1 12 Tf 300 696 Td (b) Tj ET \
                       BT /F2 12 Tf 520 704 Td (\\(1\\)) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Italic >>".to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>".to_string(),
        ];
        let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(&mut bytes, "{} 0 obj\n{}\nendobj", index + 1, object).unwrap();
        }
        let xref = bytes.len();
        writeln!(&mut bytes, "xref\n0 {}", objects.len() + 1).unwrap();
        writeln!(&mut bytes, "0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(&mut bytes, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            &mut bytes,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .unwrap();
        bytes
    }

    /// A page drawn the way a PDF re-saved by a PDF tool draws it, inside
    /// one form XObject, with a fraction bar in it; and a rule Ghostscript
    /// style, a one-pixel image mask stretched to the bar.
    fn wrapped_rules_pdf() -> Vec<u8> {
        let form = "298 708 12 0.5 re f";
        let content = "q 1 0 0 1 10 -20 cm /Fm1 Do Q \
                       q 30 0 0 0.6 100 600 cm BI /IM true /W 1 /H 1 /BPC 1 ID \x00 EI Q";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            format!(
                "<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Length {} >>\nstream\n{form}\nendstream",
                form.len()
            ),
        ];
        let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(&mut bytes, "{} 0 obj\n{}\nendobj", index + 1, object).unwrap();
        }
        let xref = bytes.len();
        writeln!(&mut bytes, "xref\n0 {}", objects.len() + 1).unwrap();
        writeln!(&mut bytes, "0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(&mut bytes, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            &mut bytes,
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .unwrap();
        bytes
    }

    #[test]
    fn finds_rules_inside_forms_and_drawn_as_image_masks() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wrapped.pdf");
        std::fs::write(&path, wrapped_rules_pdf()).unwrap();
        let b = pdfium.bindings();
        let document = render::open_document(b, path.to_str().unwrap(), None).expect("open");
        let page = b.FPDF_LoadPage(document, 0);
        let [w, h] = render::page_display_size(b, document, 0).unwrap();
        let glyphs = extract_page_glyphs(b, page, w, h);
        b.FPDF_ClosePage(page);
        b.FPDF_CloseDocument(document);

        let mut rules: Vec<&GlyphDto> = glyphs.iter().filter(|g| g.is_rule()).collect();
        rules.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert_eq!(rules.len(), 2, "{rules:?}");
        // The image mask: 30 wide at x = 100, 0.6 high at y = 600 up.
        assert!((rules[0].x - 100.0).abs() < 0.1 && (rules[0].w - 30.0).abs() < 0.1);
        assert!((rules[0].y - (792.0 - 600.6)).abs() < 0.1, "{}", rules[0].y);
        // The form's bar, moved by the form's placement (10 right, 20 down).
        assert!((rules[1].x - 308.0).abs() < 0.1 && (rules[1].w - 12.0).abs() < 0.1);
        assert!((rules[1].y - (792.0 - 688.5)).abs() < 0.1, "{}", rules[1].y);
    }

    #[test]
    fn extracts_glyph_fields_and_the_fraction_bar_from_a_pdf() {
        let _guard = pdfium_init::test_guard();
        let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("bundled PDFium"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fraction.pdf");
        std::fs::write(&path, fraction_pdf()).unwrap();
        let b = pdfium.bindings();
        let document = render::open_document(b, path.to_str().unwrap(), None).expect("open");
        let page = b.FPDF_LoadPage(document, 0);
        let [w, h] = render::page_display_size(b, document, 0).unwrap();
        let glyphs = extract_page_glyphs(b, page, w, h);
        let tags = page_math_tags(b, page, w, h);
        b.FPDF_ClosePage(page);
        b.FPDF_CloseDocument(document);

        let a = glyphs
            .iter()
            .find(|g| g.text == "a")
            .expect("the numerator");
        assert_eq!(a.code, 'a' as u32);
        assert_eq!(a.font, "Times-Italic");
        assert!(a.italic && !a.bold);
        assert!((a.size - 12.0).abs() < 0.01, "{}", a.size);
        // Baseline at y = 712 up from the bottom of a 792-high page.
        assert!((a.baseline - 80.0).abs() < 0.1, "{}", a.baseline);
        assert!(a.y < a.baseline && a.y + a.h > a.baseline);
        assert!((a.x - 300.0).abs() < 1.0, "{}", a.x);
        // The ink box: the letter's own, inside its loose one.
        let [ix, iy, iw, ih] = a.ink.expect("an ink box");
        assert!(ix >= a.x - 0.5 && ix + iw <= a.x + a.w + 0.5, "{:?}", a.ink);
        assert!(
            iy >= a.y - 0.5 && iy + ih <= a.baseline + 0.5 && ih < a.h,
            "{:?}",
            a.ink
        );

        let one = glyphs.iter().find(|g| g.text == "1").expect("the number");
        assert_eq!(one.font, "Times-Roman");
        assert!(!one.italic);

        let bar = glyphs
            .iter()
            .find(|g| g.is_rule())
            .expect("the fraction bar");
        assert!((bar.x - 298.0).abs() < 0.1 && (bar.w - 12.0).abs() < 0.1);
        assert!((bar.y - (792.0 - 708.5)).abs() < 0.1, "{}", bar.y);
        assert!(bar.h < 1.0);

        // The bar lies between the two letters, so a region around the
        // fraction picks up all three.
        let region = select_in_rects(&glyphs, &[[290.0, 60.0, 30.0, 40.0]]);
        let texts: Vec<&str> = region.iter().map(|g| g.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b", ""]);

        // Untagged: no formulas.
        assert!(tags.is_empty());
    }
}

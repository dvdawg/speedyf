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
use crate::engine::tagged::{chars_by_mcid, element_type, utf16_string};
use crate::engine::text::{is_math_font, DispMapper};
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_PAGE, FPDF_STRUCTELEMENT, FPDF_TEXTPAGE, FS_RECTF,
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
/// A path thinner than this, in points, can be a rule.
const RULE_MAX_THICKNESS: f32 = 1.0;
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
    /// "A.1"), when there is one.
    pub number: Option<String>,
}

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

/// Strip a subset tag: exactly six uppercase ASCII letters and a `+`.
pub fn strip_subset_prefix(name: &str) -> &str {
    match name.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => name,
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
    for index in 0..count {
        if b.FPDFText_IsGenerated(tp, index) == 1 {
            continue;
        }
        let value = b.FPDFText_GetUnicode(tp, index);
        let unmapped = b.FPDFText_HasUnicodeMapError(tp, index) == 1;
        let ch = char::from_u32(value);
        if matches!(ch, Some('\n' | '\r')) && !unmapped {
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
            text: if unmapped {
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
        });
    }
    out
}

/// Thin horizontal paths at the top level of the page, as rule glyphs.
fn page_rules(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    mapper: &DispMapper,
    page_h: f32,
) -> Vec<GlyphDto> {
    let mut out = Vec::new();
    let count = b.FPDFPage_CountObjects(page).clamp(0, MAX_PAGE_OBJECTS);
    for index in 0..count {
        let object = b.FPDFPage_GetObject(page, index);
        if object.is_null() || b.FPDFPageObj_GetType(object) != PAGEOBJ_PATH {
            continue;
        }
        // A path neither filled nor stroked is a clip, and draws nothing.
        let mut fill: c_int = 0;
        let mut stroke = 0;
        if b.FPDFPath_GetDrawMode(object, &mut fill, &mut stroke) != 0 && fill == 0 && stroke == 0 {
            continue;
        }
        let (mut l, mut bo, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
        if b.FPDFPageObj_GetBounds(object, &mut l, &mut bo, &mut r, &mut t) == 0 {
            continue;
        }
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
    (h < RULE_MAX_THICKNESS && w > (h * 5.0).max(1.0)).then(|| GlyphDto {
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
    })
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
                    let (cx, cy) = (g.x + g.w / 2.0, g.y + g.h / 2.0);
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
        ]
        .iter()
        .any(|family| upper.starts_with(family))
        || upper == "SYMBOL"
        || upper.starts_with("SYMBOL,")
}

fn is_big_operator(g: &GlyphDto) -> bool {
    g.font.to_ascii_uppercase().starts_with("CMEX")
        || g.text.chars().any(|c| BIG_OPERATORS.contains(&c))
}

/// One character's vertical core: the band from a little under its
/// baseline to about its cap height, from its size rather than its box,
/// since a math font's stated ascent and descent can be anything.
fn core(g: &GlyphDto) -> (f32, f32) {
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
    ok.then(|| inner.to_string())
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
    let letters = segment
        .glyphs
        .iter()
        .filter(|g| !is_math_font_name(&g.font) && g.text.chars().any(char::is_alphabetic))
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
    let mathy = big
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

/// Group characters into rows: a character joins the row whose largest
/// character's core it overlaps by half of the smaller core, so scripts
/// stay on their line and a fraction's numerator and denominator get rows
/// of their own.
fn rows(chars: &[&GlyphDto]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..chars.len()).collect();
    order.sort_by(|&a, &b| chars[a].baseline.total_cmp(&chars[b].baseline));
    // (reference core, members)
    let mut rows: Vec<((f32, f32), Vec<usize>)> = Vec::new();
    for index in order {
        let (top, bottom) = core(chars[index]);
        let found = rows.iter_mut().rev().take(8).find(|((rt, rb), _)| {
            let overlap = bottom.min(*rb) - top.max(*rt);
            let smaller = (bottom - top).min(rb - rt);
            overlap >= 0.5 * smaller
        });
        match found {
            Some((reference, members)) => {
                let largest = members
                    .iter()
                    .map(|&m| chars[m].size)
                    .fold(0.0f32, f32::max);
                if chars[index].size > largest + 0.5 {
                    *reference = (top, bottom);
                }
                members.push(index);
            }
            None => rows.push(((top, bottom), vec![index])),
        }
    }
    rows.into_iter().map(|(_, members)| members).collect()
}

/// Split a row at gaps wider than 1.2 em into segments, left to right.
fn segments<'a>(chars: &[&'a GlyphDto], row: &[usize]) -> Vec<Segment<'a>> {
    let mut members: Vec<&GlyphDto> = row.iter().map(|&i| chars[i]).collect();
    members.sort_by(|a, b| a.x.total_cmp(&b.x));
    let em = median(members.iter().map(|g| g.size).collect()).max(1.0);
    let mut out: Vec<Segment> = Vec::new();
    for g in members {
        let (top, bottom) = core(g);
        match out.last_mut() {
            Some(segment) if g.x - segment.right <= 1.2 * em => {
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
    out
}

/// A glyph's box for an equation's outline: its loose box, held to a band
/// around its baseline, since a math font's loose box can reach far past
/// the ink. Big operators keep theirs whole.
fn outline_box(g: &GlyphDto) -> [f32; 4] {
    if is_big_operator(g) {
        return [g.x, g.y, g.w, g.h];
    }
    let size = g.size.max(1.0);
    let top = g.y.max(g.baseline - 1.2 * size);
    let bottom = (g.y + g.h).min(g.baseline + 0.5 * size).max(top);
    [g.x, top, g.w, bottom - top]
}

struct Candidate {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    em: f32,
    boxes: Vec<[f32; 4]>,
    /// Characters in it; a block of nothing but a rule is an underline.
    chars: usize,
    number: Option<String>,
}

impl Candidate {
    /// Whether two candidates are one equation: stacked closely with their
    /// widths overlapping (the lines of an `align`, a numerator over its
    /// bar), or level with each other a little apart (`f(x) =` beside the
    /// fraction it equals).
    fn joins(&self, other: &Candidate) -> bool {
        let em = self.em.max(other.em);
        let v_gap = (other.top - self.bottom).max(self.top - other.bottom);
        let h_gap = (other.left - self.right).max(self.left - other.right);
        (v_gap <= 1.5 * em && h_gap <= 1.0 * em) || (v_gap < 0.0 && h_gap <= 4.0 * em)
    }

    fn absorb(&mut self, other: Candidate) {
        self.left = self.left.min(other.left);
        self.right = self.right.max(other.right);
        self.top = self.top.min(other.top);
        self.bottom = self.bottom.max(other.bottom);
        self.em = self.em.max(other.em);
        self.boxes.extend(other.boxes);
        self.chars += other.chars;
        if self.number.is_none() {
            self.number = other.number;
        }
    }
}

/// Merge candidates until no two join.
fn merge_candidates(mut pending: Vec<Candidate>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    while let Some(mut current) = pending.pop() {
        while let Some(at) = pending.iter().position(|other| current.joins(other)) {
            current.absorb(pending.swap_remove(at));
        }
        // Something merged earlier may now reach this one.
        match out.iter().position(|done| done.joins(&current)) {
            Some(at) => pending.push(out.swap_remove(at).merged_with(current)),
            None => out.push(current),
        }
    }
    out.sort_by(|a, b| a.top.total_cmp(&b.top));
    out
}

impl Candidate {
    fn merged_with(mut self, other: Candidate) -> Candidate {
        self.absorb(other);
        self
    }
}

/// Display equations on a page, found from its glyphs.
///
/// Conservative by design: a block is reported only for a run of rows that
/// reads as mathematics — mostly math fonts, a big operator, or a fraction
/// bar, without much running text — and sits centred in its text column
/// without spanning it. A right-hand `(n)` is taken as its number. Rows a
/// little apart (the lines of an `align`, a fraction's numerator and
/// denominator) merge into one block.
pub fn equation_blocks(glyphs: &[GlyphDto], page_w: f32, _page_h: f32) -> Vec<EquationBlockDto> {
    let chars: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| !g.is_rule() && g.w > 0.0 && !g.text.trim().is_empty())
        .collect();
    let rules: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| g.is_rule() && g.w < 0.6 * page_w)
        .collect();
    if chars.is_empty() {
        return Vec::new();
    }

    let all: Vec<Vec<Segment>> = rows(&chars)
        .iter()
        .map(|row| segments(&chars, row))
        .collect();

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
        let kinds: Vec<Kind> = row.iter().map(|s| classify(s, &rules)).collect();
        let mut current: Option<Candidate> = None;
        let mut row_candidates: Vec<Candidate> = Vec::new();
        for (segment, kind) in row.iter().zip(&kinds) {
            match kind {
                Kind::Math => {
                    let boxes: Vec<[f32; 4]> =
                        segment.glyphs.iter().map(|g| outline_box(g)).collect();
                    match current.as_mut() {
                        Some(c) if segment.left - c.right <= 4.0 * segment.em => {
                            c.right = segment.right;
                            c.top = c.top.min(segment.top);
                            c.bottom = c.bottom.max(segment.bottom);
                            c.boxes.extend(boxes);
                            c.chars += segment.glyphs.len();
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
                                number: None,
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
    let page_em = median(chars.iter().map(|g| g.size).collect()).max(1.0);
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
        if stacked(true) && stacked(false) {
            candidates.push(Candidate {
                left: rule.x,
                right: rule.x + rule.w,
                top: rule.baseline,
                bottom: rule.baseline,
                em: page_em,
                boxes: vec![[rule.x, rule.y, rule.w, rule.h]],
                chars: 0,
                number: None,
            });
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut merged = merge_candidates(candidates);

    // A short text segment hugging a candidate from above or below and
    // lying within its width is part of it: a fraction's "1" over "2".
    let mut absorbed = false;
    for segment in loose {
        let fits = |c: &Candidate| {
            let gap = (segment.top - c.bottom).max(c.top - segment.bottom);
            gap <= 0.6 * c.em
                && segment.left >= c.left - c.em
                && segment.right <= c.right + c.em
                && segment.width() < 0.8 * (c.right - c.left).max(c.em)
        };
        if let Some(c) = merged.iter_mut().find(|c| fits(c)) {
            c.top = c.top.min(segment.top);
            c.bottom = c.bottom.max(segment.bottom);
            c.chars += segment.glyphs.len();
            c.boxes
                .extend(segment.glyphs.iter().map(|g| outline_box(g)));
            absorbed = true;
        }
    }
    if absorbed {
        merged = merge_candidates(merged);
    }
    // A number belongs to the block it sits level with, to its right.
    for segment in numbers {
        let middle = (segment.top + segment.bottom) / 2.0;
        if let Some(c) = merged.iter_mut().find(|c| {
            c.number.is_none()
                && middle >= c.top - 0.5 * c.em
                && middle <= c.bottom + 0.5 * c.em
                && segment.left > c.right
        }) {
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
            if c.chars < 2 {
                return None;
            }
            let [x, y, w, h] = union(c.boxes.iter().copied())?;
            let (col_left, col_right) = column(x + w / 2.0);
            let col_width = (col_right - col_left).max(1.0);
            let centre_offset = ((x + w / 2.0) - (col_left + col_right) / 2.0).abs();
            let centred = centre_offset <= 0.15 * col_width
                && w < 0.9 * col_width
                && x > col_left + 0.03 * col_width;
            centred.then_some(EquationBlockDto {
                bbox: [x, y, w, h],
                number: c.number,
            })
        })
        .collect()
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
        ] {
            assert!(is_math_font_name(name), "{name}");
        }
        assert!(!is_math_font_name("CMR10"));
        assert!(!is_math_font_name("Times-Roman"));
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
        let mut glyphs = Vec::new();
        for i in 0..4 {
            glyphs.extend(body_line(100.0 + 12.0 * i as f32));
        }
        glyphs.extend(set("x=y+z", "CMMI10", 72.0, 180.0));
        assert!(equation_blocks(&glyphs, 612.0, 792.0).is_empty());
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

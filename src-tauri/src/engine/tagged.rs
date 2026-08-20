//! Reading mathematics out of a tagged PDF.
//!
//! pdfTeX throws the LaTeX source away, which is why the rest of this engine
//! reconstructs scripts from glyph geometry. A *tagged* document is the
//! exception: LaTeX's tagging project (`\DocumentMetadata{tagging=on}`) emits
//! a structure tree carrying MathML — `msup`, `mfrac`, `msqrt` and friends —
//! alongside the glyphs. Where that exists it is not a guess but the author's
//! own markup, so it wins over anything geometry can infer.
//!
//! Still rare in the wild: arXiv papers today generally carry no structure
//! tree at all. This is a path that lights up when a document has one.

use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE, FPDF_STRUCTELEMENT, FPDF_TEXTPAGE};
use std::os::raw::{c_ulong, c_void};

/// Depth cap: a malformed tree must not send us recursing forever.
const MAX_DEPTH: usize = 32;
/// Elements examined per page, as a backstop on pathological documents.
const MAX_ELEMENTS: usize = 20_000;

/// A MathML node recovered from the structure tree.
#[derive(Clone, Debug, PartialEq)]
pub enum MathNode {
    /// a leaf carrying text: mi, mn, mo, mtext
    Leaf { tag: String, text: String },
    /// a container: math, mrow, msup, msub, mfrac, msqrt, …
    Row {
        tag: String,
        children: Vec<MathNode>,
    },
}

/// One run of mathematics on a page, with the characters it covers.
#[derive(Clone, Debug, PartialEq)]
pub struct MathSpan {
    pub latex: String,
    /// character range in the page's stream, so a line can find its math
    pub start: u32,
    pub end: u32,
}

/// MathML element names LaTeX's tagging emits. Anything outside this set is
/// structure rather than mathematics and is walked through, not into.
fn is_math_tag(tag: &str) -> bool {
    matches!(
        tag,
        "math"
            | "mrow"
            | "mi"
            | "mn"
            | "mo"
            | "mtext"
            | "msup"
            | "msub"
            | "msubsup"
            | "mfrac"
            | "msqrt"
            | "mroot"
            | "munder"
            | "mover"
            | "munderover"
            | "mfenced"
            | "mspace"
            // Display equations arrive wrapped in a table — one row, one cell
            // per aligned part — so leaving these out drops every `equation`
            // environment in a document.
            | "mtable"
            | "mtr"
            | "mtd"
            | "mstyle"
            | "mpadded"
            | "mphantom"
            | "menclose"
            | "semantics"
    )
}

/// Render a MathML tree as the LaTeX that produced it.
///
/// Only the constructs LaTeX's own tagging emits are handled, because those
/// are the only ones that can appear here. An unrecognized node degrades to
/// its children's text rather than failing — a title showing plain characters
/// beats a title showing nothing.
pub fn to_latex(node: &MathNode) -> String {
    match node {
        MathNode::Leaf { text, .. } => text.clone(),
        MathNode::Row { tag, children } => {
            let parts: Vec<String> = children.iter().map(to_latex).collect();
            match tag.as_str() {
                "msup" if parts.len() == 2 => format!("{}^{}", parts[0], brace(&parts[1])),
                "msub" if parts.len() == 2 => format!("{}_{}", parts[0], brace(&parts[1])),
                "msubsup" if parts.len() == 3 => {
                    format!("{}_{}^{}", parts[0], brace(&parts[1]), brace(&parts[2]))
                }
                "mfrac" if parts.len() == 2 => {
                    format!("\\frac{{{}}}{{{}}}", parts[0], parts[1])
                }
                "msqrt" => format!("\\sqrt{{{}}}", parts.concat()),
                // Phantoms reserve space without printing; concatenating one
                // would inject text the reader cannot see on the page.
                "mphantom" => String::new(),
                "mroot" if parts.len() == 2 => {
                    format!("\\sqrt[{}]{{{}}}", parts[1], parts[0])
                }
                "munder" if parts.len() == 2 => format!("{}_{}", parts[0], brace(&parts[1])),
                "mover" if parts.len() == 2 => format!("{}^{}", parts[0], brace(&parts[1])),
                "munderover" if parts.len() == 3 => {
                    format!("{}_{}^{}", parts[0], brace(&parts[1]), brace(&parts[2]))
                }
                _ => parts.concat(),
            }
        }
    }
}

/// Wrap in braces unless it is a single character, matching how LaTeX is
/// written by hand: `x^2`, but `x^{10}`.
fn brace(part: &str) -> String {
    if part.chars().count() == 1 {
        part.to_string()
    } else {
        format!("{{{part}}}")
    }
}

/// UTF-16LE string out of a PDFium two-call getter.
fn utf16_string(fetch: impl Fn(*mut c_void, c_ulong) -> c_ulong) -> String {
    let len = fetch(std::ptr::null_mut(), 0) as usize;
    if !(2..=4096).contains(&len) {
        return String::new();
    }
    let mut buffer = vec![0u8; len];
    let written = fetch(buffer.as_mut_ptr().cast(), len as c_ulong) as usize;
    if written < 2 {
        return String::new();
    }
    let units: Vec<u16> = buffer[..written.min(len)]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn element_type(b: &dyn PdfiumLibraryBindings, element: FPDF_STRUCTELEMENT) -> String {
    utf16_string(|buffer, len| b.FPDF_StructElement_GetType(element, buffer, len))
}

/// Characters belonging to each marked-content id on this page.
///
/// The structure tree points at content by id; the text page addresses it by
/// character. This is the join between them.
fn chars_by_mcid(
    b: &dyn PdfiumLibraryBindings,
    text_page: FPDF_TEXTPAGE,
) -> std::collections::HashMap<i32, Vec<u32>> {
    let mut map: std::collections::HashMap<i32, Vec<u32>> = std::collections::HashMap::new();
    let count = b.FPDFText_CountChars(text_page).max(0);
    for index in 0..count {
        let object = b.FPDFText_GetTextObject(text_page, index);
        if object.is_null() {
            continue;
        }
        let mcid = b.FPDFPageObj_GetMarkedContentID(object);
        if mcid >= 0 {
            map.entry(mcid).or_default().push(index as u32);
        }
    }
    map
}

/// How far back a span will reach to pick up the decoration drawn beside it.
const ARTIFACT_LOOKBACK: u32 = 6;

struct Walker<'a> {
    b: &'a dyn PdfiumLibraryBindings,
    text: &'a str,
    by_mcid: &'a std::collections::HashMap<i32, Vec<u32>>,
    /// every character any structure element claims — anything else on the
    /// page is an artifact
    claimed: &'a std::collections::HashSet<u32>,
    budget: usize,
    covered: Vec<u32>,
}

impl Walker<'_> {
    /// Text of a leaf, gathered from the characters its marked content owns.
    fn leaf_text(&mut self, element: FPDF_STRUCTELEMENT) -> String {
        let children = self.b.FPDF_StructElement_CountChildren(element);
        let mut indices: Vec<u32> = Vec::new();
        for i in 0..children {
            let mcid = self
                .b
                .FPDF_StructElement_GetChildMarkedContentID(element, i);
            if mcid >= 0 {
                if let Some(chars) = self.by_mcid.get(&mcid) {
                    indices.extend(chars.iter().copied());
                }
            }
        }
        indices.sort_unstable();
        self.covered.extend(indices.iter().copied());
        indices
            .iter()
            .filter_map(|i| self.text.chars().nth(*i as usize))
            .collect()
    }

    fn node(&mut self, element: FPDF_STRUCTELEMENT, depth: usize) -> Option<MathNode> {
        if depth > MAX_DEPTH || self.budget == 0 {
            return None;
        }
        self.budget -= 1;
        let tag = element_type(self.b, element);
        if !is_math_tag(&tag) {
            return None;
        }
        let child_count = self.b.FPDF_StructElement_CountChildren(element);
        let mut children = Vec::new();
        for i in 0..child_count {
            let child = self.b.FPDF_StructElement_GetChildAtIndex(element, i);
            if child.is_null() {
                continue;
            }
            if let Some(node) = self.node(child, depth + 1) {
                children.push(node);
            }
        }
        if children.is_empty() {
            let text = self.leaf_text(element);
            if text.is_empty() {
                return None;
            }
            return Some(MathNode::Leaf { tag, text });
        }
        Some(MathNode::Row { tag, children })
    }

    /// Descend through non-math structure looking for math roots.
    fn collect(&mut self, element: FPDF_STRUCTELEMENT, depth: usize, out: &mut Vec<MathSpan>) {
        if depth > MAX_DEPTH || self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let tag = element_type(self.b, element);
        if is_math_tag(&tag) {
            self.covered.clear();
            if let Some(node) = self.node(element, depth) {
                let latex = to_latex(&node);
                if !latex.trim().is_empty() && !self.covered.is_empty() {
                    let mut start = *self.covered.iter().min().unwrap();
                    let end = *self.covered.iter().max().unwrap();
                    // A radical sign, a big brace, an over-bar: TeX draws
                    // these as artifacts beside the marked content rather
                    // than inside it. Left out of the span they survive the
                    // substitution and print next to the markup that already
                    // describes them — "√\sqrt{x}". Everything real in a
                    // tagged document is claimed by some element, so an
                    // unclaimed glyph hugging the span is decoration.
                    let floor = start.saturating_sub(ARTIFACT_LOOKBACK);
                    let mut candidate = start;
                    while candidate > floor {
                        let index = candidate - 1;
                        // Claimed text belongs to something else; stop there.
                        if self.claimed.contains(&index) {
                            break;
                        }
                        candidate = index;
                        // Step over the line breaks PDFium puts between text
                        // objects, but only pull the span back as far as an
                        // actual glyph.
                        if self
                            .text
                            .chars()
                            .nth(index as usize)
                            .is_some_and(|c| !c.is_whitespace())
                        {
                            start = index;
                        }
                    }
                    out.push(MathSpan {
                        latex,
                        start,
                        end: end + 1,
                    });
                }
            }
            return;
        }
        let child_count = self.b.FPDF_StructElement_CountChildren(element);
        for i in 0..child_count {
            let child = self.b.FPDF_StructElement_GetChildAtIndex(element, i);
            if !child.is_null() {
                self.collect(child, depth + 1, out);
            }
        }
    }
}

/// Math on a page, recovered from its structure tree. Empty when the document
/// carries no tagging, which is the common case.
pub fn page_math(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    text_page: FPDF_TEXTPAGE,
    raw: &str,
) -> Vec<MathSpan> {
    let tree = b.FPDF_StructTree_GetForPage(page);
    if tree.is_null() {
        return Vec::new();
    }
    let by_mcid = chars_by_mcid(b, text_page);
    let claimed: std::collections::HashSet<u32> = by_mcid.values().flatten().copied().collect();
    let mut walker = Walker {
        b,
        text: raw,
        by_mcid: &by_mcid,
        claimed: &claimed,
        budget: MAX_ELEMENTS,
        covered: Vec::new(),
    };
    let mut spans = Vec::new();
    let roots = b.FPDF_StructTree_CountChildren(tree);
    for i in 0..roots {
        let child = b.FPDF_StructTree_GetChildAtIndex(tree, i);
        if !child.is_null() {
            walker.collect(child, 0, &mut spans);
        }
    }
    b.FPDF_StructTree_Close(tree);
    spans.sort_by_key(|span| span.start);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(tag: &str, text: &str) -> MathNode {
        MathNode::Leaf {
            tag: tag.into(),
            text: text.into(),
        }
    }
    fn row(tag: &str, children: Vec<MathNode>) -> MathNode {
        MathNode::Row {
            tag: tag.into(),
            children,
        }
    }

    #[test]
    fn renders_a_superscript_the_way_it_was_written() {
        let node = row("msup", vec![leaf("mi", "L"), leaf("mn", "2")]);
        assert_eq!(to_latex(&node), "L^2");
    }

    #[test]
    fn braces_a_multi_character_script() {
        let node = row("msup", vec![leaf("mi", "x"), leaf("mn", "10")]);
        assert_eq!(to_latex(&node), "x^{10}");
        let sub = row("msub", vec![leaf("mi", "α"), leaf("mtext", "ext")]);
        assert_eq!(to_latex(&sub), "α_{ext}");
    }

    #[test]
    fn renders_the_structures_geometry_could_never_recover() {
        // The whole reason a tagged document is worth reading: a fraction and
        // a radical are invisible to glyph positions.
        let frac = row("mfrac", vec![leaf("mi", "a"), leaf("mi", "b")]);
        assert_eq!(to_latex(&frac), "\\frac{a}{b}");
        let sqrt = row("msqrt", vec![leaf("mi", "x")]);
        assert_eq!(to_latex(&sqrt), "\\sqrt{x}");
        let root = row("mroot", vec![leaf("mi", "x"), leaf("mn", "3")]);
        assert_eq!(to_latex(&root), "\\sqrt[3]{x}");
    }

    #[test]
    fn renders_a_sub_and_superscript_pair() {
        let node = row(
            "msubsup",
            vec![leaf("mi", "y"), leaf("mi", "i"), leaf("mn", "2")],
        );
        assert_eq!(to_latex(&node), "y_i^2");
    }

    #[test]
    fn a_row_is_just_its_children_in_order() {
        let node = row(
            "math",
            vec![row(
                "mrow",
                vec![leaf("mi", "a"), leaf("mo", "+"), leaf("mi", "b")],
            )],
        );
        assert_eq!(to_latex(&node), "a+b");
    }

    #[test]
    fn an_unknown_container_degrades_to_its_text() {
        // Better a plain title than an empty one.
        let node = row("mfenced", vec![leaf("mi", "x"), leaf("mi", "y")]);
        assert_eq!(to_latex(&node), "xy");
    }

    #[test]
    fn a_display_equation_survives_its_table_wrapper() {
        // LaTeX wraps `equation` bodies in mtable/mtr/mtd; the structures are
        // meaningless here but dropping them drops the whole equation.
        let node = row(
            "math",
            vec![row(
                "mtable",
                vec![row(
                    "mtr",
                    vec![row(
                        "mtd",
                        vec![
                            row("mfrac", vec![leaf("mi", "a"), leaf("mi", "b")]),
                            leaf("mo", "="),
                            row("msqrt", vec![leaf("mi", "x")]),
                        ],
                    )],
                )],
            )],
        );
        assert_eq!(to_latex(&node), "\\frac{a}{b}=\\sqrt{x}");
    }

    #[test]
    fn a_phantom_prints_nothing() {
        let node = row(
            "mrow",
            vec![leaf("mi", "a"), row("mphantom", vec![leaf("mi", "zzz")])],
        );
        assert_eq!(to_latex(&node), "a");
    }

    #[test]
    fn only_mathml_elements_are_treated_as_mathematics() {
        for tag in ["math", "msup", "mfrac", "mi", "mo"] {
            assert!(is_math_tag(tag), "{tag}");
        }
        for tag in ["P", "Sect", "H1", "Formula", "Figure", "Span", ""] {
            assert!(!is_math_tag(tag), "{tag}");
        }
    }
}

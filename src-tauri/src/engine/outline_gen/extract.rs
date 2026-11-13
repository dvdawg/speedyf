//! One page as the outline generator reads it: typeset lines with the font
//! facts a heading is recognised by, the page's label, and - in a tagged
//! document - the headings its structure tree names.
//!
//! The line grouping is `layout_text`'s own (`line_groups`, `build_line`),
//! so a line here is exactly the line the title reader sees; what is added
//! is per line: which font most of it is set in, how much of it is
//! mathematics, italic or small caps, and the bold words a run-in heading
//! opens with.

use super::detect::{Lead, Line, Page, TaggedHeading};
use crate::engine::layout_text::{self, LayoutGlyph};
use crate::engine::tagged;
use crate::engine::text::{is_math_font, DispMapper};
use pdfium_render::prelude::{
    PdfiumLibraryBindings, FPDF_DOCUMENT, FPDF_PAGE, FPDF_STRUCTELEMENT, FPDF_TEXTPAGE, FS_RECTF,
};
use std::collections::HashMap;
use std::os::raw::c_int;

/// PDF font descriptor flags (PDF 1.7, table 123).
const FORCE_BOLD_FLAG: c_int = 1 << 18;
const ITALIC_FLAG: c_int = 1 << 6;
const SMALL_CAP_FLAG: c_int = 1 << 17;
/// Structure elements examined per page, as a backstop.
const MAX_ELEMENTS: usize = 20_000;
const MAX_DEPTH: usize = 32;

/// What a font's name and flags say about it, judged once per font.
#[derive(Clone, Copy, Debug, Default)]
pub struct FontFacts {
    pub bold: bool,
    pub italic: bool,
    pub smallcaps: bool,
    pub math: bool,
    /// The invisible font OCR tools lay a text layer down in: its sizes and
    /// weights say nothing about the printed page.
    pub glyphless: bool,
}

/// Every font a document's pages have used so far, by family name with the
/// subset tag removed, so "ABCDEF+CMBX12" on one page and "GHIJKL+CMBX12"
/// on the next are one style.
#[derive(Default)]
pub struct FontTable {
    ids: HashMap<String, u32>,
    pub facts: Vec<FontFacts>,
    /// Family names by id, for diagnostics.
    pub names: Vec<String>,
}

impl FontTable {
    fn intern(&mut self, name: &str, flags: c_int) -> u32 {
        let family = name.split_once('+').map_or(name, |(_, rest)| rest);
        if let Some(id) = self.ids.get(family) {
            return *id;
        }
        let lower = family.to_ascii_lowercase();
        let facts = FontFacts {
            bold: layout_text::is_bold_font_name(family) || flags & FORCE_BOLD_FLAG != 0,
            italic: crate::engine::glyphs::is_italic_font_name(family) || flags & ITALIC_FLAG != 0,
            smallcaps: flags & SMALL_CAP_FLAG != 0
                || lower.contains("csc")
                || lower.contains("smallcap")
                || lower.ends_with("-sc")
                || lower.ends_with("sc10")
                || lower.contains("caps"),
            math: is_math_font(family)
                || lower.contains("math")
                || lower.starts_with("symbol")
                || lower.starts_with("euclid")
                || lower.contains("mt-extra")
                || lower.starts_with("rsfs")
                || lower.starts_with("eufm")
                || lower.starts_with("stixmath"),
            glyphless: lower.contains("glyphless"),
        };
        let id = self.facts.len() as u32;
        self.ids.insert(family.to_owned(), id);
        self.facts.push(facts);
        self.names.push(family.to_owned());
        id
    }
}

/// Read one page. `tagged` asks for the structure tree's headings too.
pub fn read_page(
    b: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    index: u32,
    fonts: &mut FontTable,
    tagged: bool,
) -> Page {
    let [width, height] = crate::engine::render::page_display_size(b, document, index as i32)
        .unwrap_or([612.0, 792.0]);
    let label = page_label(b, document, index);
    let mut page = Page {
        index,
        width,
        height,
        label,
        ..Page::default()
    };
    let handle = b.FPDF_LoadPage(document, index as i32);
    if handle.is_null() {
        return page;
    }
    let tp = b.FPDFText_LoadPage(handle);
    if !tp.is_null() {
        let mapper = DispMapper::new(b, handle, width, height);
        let (lines, chars) = read_lines(b, tp, &mapper, height, fonts);
        page.lines = lines;
        page.chars = chars;
        if tagged {
            page.tagged = tagged_headings(b, handle, tp, &mapper, height);
        }
        b.FPDFText_ClosePage(tp);
    }
    b.FPDF_ClosePage(handle);
    page
}

/// The page's label ("iv", "17", "A-3"), when the document defines labels.
fn page_label(
    b: &dyn PdfiumLibraryBindings,
    document: FPDF_DOCUMENT,
    index: u32,
) -> Option<String> {
    let label = tagged::utf16_string(|buffer, len| {
        b.FPDF_GetPageLabel(document, index as i32, buffer, len)
    });
    let label = label.trim();
    (!label.is_empty()).then(|| label.to_owned())
}

/// The page's glyphs exactly as `layout_text::extract_lines` reads them,
/// plus each one's font, then grouped into lines the same way.
fn read_lines(
    b: &dyn PdfiumLibraryBindings,
    tp: FPDF_TEXTPAGE,
    mapper: &DispMapper,
    page_h: f32,
    fonts: &mut FontTable,
) -> (Vec<Line>, u32) {
    let count = b.FPDFText_CountChars(tp).max(0);
    let mut glyphs: Vec<LayoutGlyph> = Vec::with_capacity(count as usize);
    let mut font_of: Vec<u32> = Vec::with_capacity(count as usize);
    let mut by_name: HashMap<String, u32> = HashMap::new();
    let mut inked_chars = 0u32;
    for index in 0..count {
        let ch = char::from_u32(b.FPDFText_GetUnicode(tp, index)).unwrap_or(' ');
        if ch == '\n' || ch == '\r' {
            glyphs.push(LayoutGlyph {
                ch,
                size: 0.0,
                weight: 0,
                bold: false,
                upright: true,
                baseline: 0.0,
                bbox: None,
            });
            font_of.push(u32::MAX);
            continue;
        }
        let matrix = layout_text::char_matrix(b, tp, index);
        let nominal = b.FPDFText_GetFontSize(tp, index) as f32;
        let (mut ox, mut oy) = (0f64, 0f64);
        let has_origin = b.FPDFText_GetCharOrigin(tp, index, &mut ox, &mut oy) != 0;
        let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
        let has_box = b.FPDFText_GetCharBox(tp, index, &mut l, &mut r, &mut bo, &mut t) != 0
            && (r - l).abs() > f64::EPSILON
            && (t - bo).abs() > f64::EPSILON;
        let mut loose = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        let has_loose = b.FPDFText_GetLooseCharBox(tp, index, &mut loose) != 0
            && loose.right > loose.left
            && loose.top > loose.bottom;
        let bbox = has_box.then(|| {
            let (x, y, w, h) = mapper.map_box(l, r, bo, t);
            let (left, right) = if has_loose {
                let (lx, _, lw, _) = mapper.map_box(
                    loose.left as f64,
                    loose.right as f64,
                    loose.bottom as f64,
                    loose.top as f64,
                );
                (lx, lx + lw)
            } else {
                (x, x + w)
            };
            [left, page_h - (y + h), right, page_h - y]
        });
        let upright = matrix.is_none_or(|matrix| layout_text::runs_left_to_right(mapper, &matrix));
        let scale = matrix.map_or(1.0, |matrix| {
            (matrix.c * matrix.c + matrix.d * matrix.d).sqrt()
        });
        let mut size = nominal * scale;
        if size.is_nan() || size <= 0.5 {
            size = bbox.map_or(0.0, |[_, top, _, bottom]| (bottom - top) / 0.7);
        }
        let baseline = if has_origin {
            page_h - mapper.map(ox, oy).1
        } else {
            bbox.map_or(0.0, |[_, _, _, bottom]| bottom)
        };
        let weight = b.FPDFText_GetFontWeight(tp, index);
        let (name, flags) = layout_text::font_info(b, tp, index);
        let font = match by_name.get(&name) {
            Some(id) => *id,
            None => {
                let id = fonts.intern(&name, flags);
                by_name.insert(name, id);
                id
            }
        };
        let facts = fonts.facts[font as usize];
        if bbox.is_some() && !ch.is_whitespace() {
            inked_chars += 1;
        }
        glyphs.push(LayoutGlyph {
            ch,
            size,
            weight: weight.max(0),
            bold: weight >= 600 || facts.bold,
            upright,
            baseline,
            bbox,
        });
        font_of.push(font);
    }
    let lines = layout_text::line_groups(&glyphs)
        .iter()
        .filter_map(|group| line_from_group(&glyphs, &font_of, group, fonts))
        .collect();
    (lines, inked_chars)
}

/// One line and its font facts from the glyphs `line_groups` put in it.
fn line_from_group(
    glyphs: &[LayoutGlyph],
    font_of: &[u32],
    group: &[usize],
    fonts: &FontTable,
) -> Option<Line> {
    let members: Vec<&LayoutGlyph> = group.iter().map(|&i| &glyphs[i]).collect();
    let line = layout_text::build_line(&members)?;
    let inked: Vec<usize> = group
        .iter()
        .copied()
        .filter(|&i| glyphs[i].bbox.is_some() && !glyphs[i].ch.is_whitespace())
        .collect();
    if inked.is_empty() {
        return None;
    }
    // Only glyphs at the line's own size vote on its style: a footnote
    // marker's font says nothing about the heading it hangs off.
    let full: Vec<usize> = inked
        .iter()
        .copied()
        .filter(|&i| glyphs[i].size >= 0.85 * line.size)
        .collect();
    let voters = if full.is_empty() { &inked } else { &full };
    let share = |pick: fn(&FontFacts) -> bool| {
        voters
            .iter()
            .filter(|&&i| fonts.facts.get(font_of[i] as usize).is_some_and(pick))
            .count() as f32
            / voters.len() as f32
    };
    let mut votes: HashMap<u32, usize> = HashMap::new();
    for &i in voters {
        *votes.entry(font_of[i]).or_default() += 1;
    }
    let font = votes
        .into_iter()
        .max_by_key(|(id, n)| (*n, std::cmp::Reverse(*id)))
        .map_or(0, |(id, _)| id);
    let math = inked
        .iter()
        .filter(|&&i| {
            fonts
                .facts
                .get(font_of[i] as usize)
                .is_some_and(|facts| facts.math)
        })
        .count() as f32
        / inked.len() as f32;
    Some(Line {
        size: line.size,
        bold: line.bold,
        bold_named: share(|f| f.bold) > 0.5,
        italic: share(|f| f.italic) > 0.5,
        smallcaps: share(|f| f.smallcaps) > 0.5,
        math,
        font,
        top: line.top,
        bottom: line.bottom,
        left: line.left,
        right: line.right,
        lead: run_in_lead(glyphs, group, line.size),
        text: line.text,
    })
}

/// The bold words a paragraph opens with, when the rest of the line is not
/// bold: the head of a run-in heading ("Model. We consider...").
fn run_in_lead(glyphs: &[LayoutGlyph], group: &[usize], size: f32) -> Option<Lead> {
    let inked = |i: usize| glyphs[i].bbox.is_some() && !glyphs[i].ch.is_whitespace();
    let first = group.iter().position(|&i| inked(i))?;
    if !glyphs[group[first]].bold {
        return None;
    }
    let end = group[first..]
        .iter()
        .position(|&i| inked(i) && !glyphs[i].bold && glyphs[i].size >= 0.85 * size)
        .map(|offset| first + offset)?;
    let rest = group[end..].iter().filter(|&&i| inked(i)).count();
    if rest < 3 {
        return None;
    }
    let members: Vec<&LayoutGlyph> = group[first..end].iter().map(|&i| &glyphs[i]).collect();
    let head = layout_text::build_line(&members)?;
    Some(Lead {
        text: head.text,
        size: head.size,
    })
}

/// Headings the structure tree names on this page: `H`, `H1`..`H6`, with
/// their text and where the first of their characters stands.
fn tagged_headings(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    tp: FPDF_TEXTPAGE,
    mapper: &DispMapper,
    page_h: f32,
) -> Vec<TaggedHeading> {
    let tree = b.FPDF_StructTree_GetForPage(page);
    if tree.is_null() {
        return Vec::new();
    }
    let by_mcid = tagged::chars_by_mcid(b, tp);
    let mut walker = HeadingWalker {
        b,
        tp,
        mapper,
        page_h,
        by_mcid: &by_mcid,
        budget: MAX_ELEMENTS,
        out: Vec::new(),
    };
    let roots = b.FPDF_StructTree_CountChildren(tree);
    for i in 0..roots {
        let child = b.FPDF_StructTree_GetChildAtIndex(tree, i);
        if !child.is_null() {
            walker.walk(child, 0);
        }
    }
    b.FPDF_StructTree_Close(tree);
    walker.out
}

struct HeadingWalker<'a> {
    b: &'a dyn PdfiumLibraryBindings,
    tp: FPDF_TEXTPAGE,
    mapper: &'a DispMapper,
    page_h: f32,
    by_mcid: &'a HashMap<i32, Vec<u32>>,
    budget: usize,
    out: Vec<TaggedHeading>,
}

impl HeadingWalker<'_> {
    fn walk(&mut self, element: FPDF_STRUCTELEMENT, depth: usize) {
        if depth > MAX_DEPTH || self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let kind = tagged::element_type(self.b, element);
        if let Some(level) = heading_level(&kind) {
            let mut chars = Vec::new();
            self.collect_chars(element, depth, &mut chars);
            chars.sort_unstable();
            chars.dedup();
            if let Some(heading) = self.heading(level, &chars) {
                self.out.push(heading);
            }
            return;
        }
        let children = self.b.FPDF_StructElement_CountChildren(element);
        for i in 0..children {
            let child = self.b.FPDF_StructElement_GetChildAtIndex(element, i);
            if !child.is_null() {
                self.walk(child, depth + 1);
            }
        }
    }

    fn collect_chars(&mut self, element: FPDF_STRUCTELEMENT, depth: usize, out: &mut Vec<u32>) {
        if depth > MAX_DEPTH || self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let direct = self.b.FPDF_StructElement_GetMarkedContentIdCount(element);
        for i in 0..direct.max(0) {
            let mcid = self
                .b
                .FPDF_StructElement_GetMarkedContentIdAtIndex(element, i);
            if let Some(chars) = self.by_mcid.get(&mcid) {
                out.extend(chars.iter().copied());
            }
        }
        let children = self.b.FPDF_StructElement_CountChildren(element);
        for i in 0..children {
            let mcid = self
                .b
                .FPDF_StructElement_GetChildMarkedContentID(element, i);
            if mcid >= 0 {
                if let Some(chars) = self.by_mcid.get(&mcid) {
                    out.extend(chars.iter().copied());
                }
                continue;
            }
            let child = self.b.FPDF_StructElement_GetChildAtIndex(element, i);
            if !child.is_null() {
                self.collect_chars(child, depth + 1, out);
            }
        }
    }

    fn heading(&self, level: u8, chars: &[u32]) -> Option<TaggedHeading> {
        let mut text = String::new();
        let mut top = f32::INFINITY;
        for &index in chars {
            let ch =
                char::from_u32(self.b.FPDFText_GetUnicode(self.tp, index as i32)).unwrap_or(' ');
            text.push(if ch.is_control() { ' ' } else { ch });
            let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
            if self
                .b
                .FPDFText_GetCharBox(self.tp, index as i32, &mut l, &mut r, &mut bo, &mut t)
                != 0
                && (t - bo).abs() > f64::EPSILON
            {
                let (_, y, _, h) = self.mapper.map_box(l, r, bo, t);
                top = top.min(self.page_h - (y + h));
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!text.is_empty() && top.is_finite()).then_some(TaggedHeading { level, text, top })
    }
}

/// `H1`..`H6` give their level; a bare `H` (nested sections) is level 1.
fn heading_level(kind: &str) -> Option<u8> {
    match kind {
        "H" => Some(1),
        _ => {
            let digit = kind.strip_prefix('H')?;
            let level: u8 = digit.parse().ok()?;
            (1..=6).contains(&level).then_some(level)
        }
    }
}

/// Whether the document carries a structure tree at all.
pub fn is_tagged(b: &dyn PdfiumLibraryBindings, document: FPDF_DOCUMENT) -> bool {
    b.FPDFCatalog_IsTagged(document) != 0
}

#[cfg(test)]
mod tests {
    use super::heading_level;

    #[test]
    fn structure_heading_levels() {
        assert_eq!(heading_level("H1"), Some(1));
        assert_eq!(heading_level("H3"), Some(3));
        assert_eq!(heading_level("H"), Some(1));
        assert_eq!(heading_level("H7"), None);
        assert_eq!(heading_level("Hyperlink"), None);
        assert_eq!(heading_level("P"), None);
    }
}

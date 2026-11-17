//! Reading a page for the hover detectors: every character with its box,
//! size, font, weight and whether its font is a math font. The one part of
//! the hover code that touches PDFium.

use super::page::{Glyph, PageInput};
use crate::engine::glyphs::{is_math_font_name, strip_subset_prefix};
use crate::engine::text::DispMapper;
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE, FPDF_TEXTPAGE};
use std::collections::HashMap;
use std::os::raw::{c_int, c_ulong};

/// PDF font descriptor flag: the font is bold however its name reads.
const FORCE_BOLD: c_int = 1 << 18;

fn font_info(b: &dyn PdfiumLibraryBindings, tp: FPDF_TEXTPAGE, index: i32) -> (String, c_int) {
    let mut flags: c_int = 0;
    let mut buffer = [0u8; 128];
    let written = b.FPDFText_GetFontInfo(
        tp,
        index,
        buffer.as_mut_ptr().cast(),
        buffer.len() as c_ulong,
        &mut flags,
    ) as usize;
    if written == 0 || written > buffer.len() {
        return (String::new(), flags);
    }
    (
        String::from_utf8_lossy(&buffer[..written.saturating_sub(1)]).into_owned(),
        flags,
    )
}

/// Whether a font's name says it is a bold face.
pub fn bold_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "bold", "black", "heavy", "semibold", "demi", "-bd", "cmbx", "cmb10", "-medi",
    ]
    .iter()
    .any(|w| lower.contains(w))
}

pub fn extract(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    disp_w: f32,
    disp_h: f32,
) -> PageInput {
    let tp = b.FPDFText_LoadPage(page);
    if tp.is_null() {
        return PageInput {
            width: disp_w,
            height: disp_h,
            glyphs: Vec::new(),
            fonts: Vec::new(),
        };
    }
    let count = b.FPDFText_CountChars(tp).max(0);
    let mapper = DispMapper::new(b, page, disp_w, disp_h);
    // (name, bold) -> (id, math)
    let mut fonts: HashMap<(String, bool), (u16, bool)> = HashMap::new();
    let mut glyphs = Vec::with_capacity(count as usize);
    for index in 0..count {
        let ch = char::from_u32(b.FPDFText_GetUnicode(tp, index)).unwrap_or(' ');
        let size = b.FPDFText_GetFontSize(tp, index) as f32;
        let (name, flags) = font_info(b, tp, index);
        let name = strip_subset_prefix(&name).to_string();
        let weight = b.FPDFText_GetFontWeight(tp, index);
        let bold = weight >= 600 || flags & FORCE_BOLD != 0 || bold_name(&name);
        let next_id = fonts.len() as u16 + 1;
        let (font, math) = *fonts
            .entry((name.clone(), bold))
            .or_insert_with(|| (next_id, is_math_font_name(&name)));
        let mut rect = [0.0f32; 4];
        if ch != '\n' && ch != '\r' {
            let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
            let ok = b.FPDFText_GetCharBox(tp, index, &mut l, &mut r, &mut bo, &mut t) != 0;
            if ok && ((r - l).abs() > f64::EPSILON || (t - bo).abs() > f64::EPSILON) {
                let (x, y, w, h) = mapper.map_box(l, r, bo, t);
                rect = [x, y, w, h];
            }
        }
        glyphs.push(Glyph {
            ch,
            rect,
            size: if size.is_finite() { size } else { 0.0 },
            font,
            bold,
            math,
        });
    }
    b.FPDFText_ClosePage(tp);
    let mut names = vec![String::new(); fonts.len()];
    for ((name, _), (id, _)) in fonts {
        if let Some(slot) = names.get_mut(usize::from(id).wrapping_sub(1)) {
            *slot = name;
        }
    }
    PageInput {
        width: disp_w,
        height: disp_h,
        glyphs,
        fonts: names,
    }
}

/// Display equations found on a page (`glyphs::EquationBlockDto`, top-left
/// origin) in the y-up space of the hover lines, with their numbers: what
/// `Page::set_displays` takes.
pub fn displays_y_up(
    blocks: &[crate::engine::glyphs::EquationBlockDto],
    page_h: f32,
) -> Vec<([f32; 4], Option<String>)> {
    blocks
        .iter()
        .map(|b| {
            let [x, y, w, h] = b.bbox;
            ([x, page_h - y - h, w, h], b.number.clone())
        })
        .collect()
}

/// A page's display equations, read from its glyphs and raster as the
/// equation layer reads them (`glyphs::page_equation_blocks`), ready for
/// `Page::set_displays`. `runs` are the page's text runs, for its floats;
/// without `raster` the glyphs alone are read - quicker, a little less sure.
pub fn displays(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    disp_w: f32,
    disp_h: f32,
    runs: &[crate::engine::types::TextRun],
    raster: bool,
) -> Vec<([f32; 4], Option<String>)> {
    use crate::engine::{glyphs, vision};
    let all = glyphs::extract_page_glyphs(b, page, disp_w, disp_h);
    let raster = raster
        .then(|| vision::render_raster(b, page, disp_w, disp_h))
        .flatten();
    let blocks = glyphs::page_equation_blocks(&all, raster.as_ref(), runs, disp_w, disp_h);
    displays_y_up(&blocks, disp_h)
}

/// The displays the hover index reads a page with: from its glyphs alone,
/// quick enough for every page, and none looked for on a page without a
/// character of mathematics. A preview settles its crop against the
/// raster's as well (`displays`).
pub fn index_displays(
    b: &dyn PdfiumLibraryBindings,
    handle: FPDF_PAGE,
    page: &super::page::Page,
) -> Vec<([f32; 4], Option<String>)> {
    if !page.lines.iter().any(|l| l.math_count > 0) {
        return Vec::new();
    }
    let runs = super::xref::runs_of(page);
    displays(b, handle, page.width, page.height, &runs, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_faces_by_name() {
        assert!(bold_name("Times-Bold"));
        assert!(bold_name("CMBX10"));
        assert!(bold_name("NimbusRomNo9L-Medi"));
        assert!(!bold_name("CMR10"));
        assert!(!bold_name("CMTI10"));
    }
}

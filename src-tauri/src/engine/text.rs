//! Text extraction on the engine thread. Character boxes come back in raw
//! PDF user space; we convert them through FPDF_PageToDevice (at 100×
//! precision) into display-normalized page space (origin bottom-left of the
//! displayed page, y-up, points) so the frontend never sees crop-box or
//! /Rotate weirdness. The raw character stream is kept 1:1 with PDFium char
//! indices so search matches can be mapped back to page rects.

use crate::engine::types::TextRun;
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE, FPDF_TEXTPAGE};
use std::os::raw::{c_int, c_ulong};

const PREC: f64 = 100.0;

pub struct ExtractedPage {
    /// one Rust char per PDFium char index (controls/unmappables become ' ')
    pub raw: String,
    pub runs: Vec<TextRun>,
    /// display-space character boxes keyed by PDFium character index; a zero
    /// box means PDFium reported no usable geometry for that character.
    pub boxes: Vec<[f32; 4]>,
    /// nominal point size of each character's font.
    ///
    /// This is what a superscript actually is — a glyph set at a smaller size
    /// — and it is not recoverable from the character box, whose height is a
    /// property of the glyph's shape. A period and a hyphen are short at any
    /// size, which is exactly how they came to be read as raised scripts.
    pub sizes: Vec<f32>,
    /// whether each character is set in a math font (Computer Modern Math,
    /// Symbol, Extension, or the AMS symbol fonts). Font identity is a fact
    /// where glyph geometry is only ever an inference.
    pub math: Vec<bool>,
    pub char_count: u32,
}

/// Font families TeX reserves for mathematics. `CMMI` carries variables,
/// `CMSY` operators and relations, `CMEX` the large braces and integrals,
/// `MSAM`/`MSBM` the AMS symbols. A subsetted font keeps the family name after
/// its six-letter tag — "EWJNXA+CMMI10".
const MATH_FONT_FAMILIES: &[&str] = &[
    "CMMI",
    "CMSY",
    "CMEX",
    "MSAM",
    "MSBM",
    "MathItalic",
    "MathSymbol",
    "Math-Italic",
];

/// Whether `font_name` belongs to a family used for mathematics.
pub fn is_math_font(font_name: &str) -> bool {
    let family = font_name
        .split_once('+')
        .map_or(font_name, |(_, rest)| rest);
    MATH_FONT_FAMILIES.iter().any(|candidate| {
        family
            .to_ascii_uppercase()
            .contains(&candidate.to_ascii_uppercase())
    })
}

/// Name of the font a character is set in, or an empty string.
fn font_name_at(b: &dyn PdfiumLibraryBindings, tp: FPDF_TEXTPAGE, index: i32) -> String {
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
        return String::new();
    }
    // The name comes back NUL-terminated and the count includes it.
    String::from_utf8_lossy(&buffer[..written.saturating_sub(1)]).into_owned()
}

pub struct ExtractedSearchText {
    pub raw: String,
    pub char_count: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct DispMapper {
    origin: (f64, f64),
    x_axis: (f64, f64),
    y_axis: (f64, f64),
    disp_h: f32,
}

impl DispMapper {
    pub(crate) fn new(
        b: &dyn PdfiumLibraryBindings,
        page: FPDF_PAGE,
        disp_w: f32,
        disp_h: f32,
    ) -> Self {
        let dw = (disp_w as f64 * PREC) as i32;
        let dh = (disp_h as f64 * PREC) as i32;
        let device = |ux: f64, uy: f64| {
            let (mut dx, mut dy) = (0i32, 0i32);
            b.FPDF_PageToDevice(page, 0, 0, dw, dh, 0, ux, uy, &mut dx, &mut dy);
            (dx as f64 / PREC, dy as f64 / PREC)
        };
        let origin = device(0.0, 0.0);
        let x_unit = device(1.0, 0.0);
        let y_unit = device(0.0, 1.0);
        DispMapper {
            origin,
            x_axis: (x_unit.0 - origin.0, x_unit.1 - origin.1),
            y_axis: (y_unit.0 - origin.0, y_unit.1 - origin.1),
            disp_h,
        }
    }

    /// user-space point → display-normalized (y-up) point
    pub(crate) fn map(&self, ux: f64, uy: f64) -> (f32, f32) {
        let dx = self.origin.0 + ux * self.x_axis.0 + uy * self.y_axis.0;
        let dy = self.origin.1 + ux * self.x_axis.1 + uy * self.y_axis.1;
        (dx as f32, self.disp_h - dy as f32)
    }

    /// user-space char box → display rect (x, y_bottom, w, h), y-up
    pub(crate) fn map_box(&self, l: f64, r: f64, b_: f64, t: f64) -> (f32, f32, f32, f32) {
        let (x1, y1) = self.map(l, b_);
        let (x2, y2) = self.map(r, t);
        let x = x1.min(x2);
        let y = y1.min(y2);
        (x, y, (x1 - x2).abs(), (y1 - y2).abs())
    }
}

/// Extract only the character stream needed by the background search index.
/// Foreground layout extraction additionally asks PDFium for character boxes;
/// the indexer deliberately avoids those thousands of FFI calls per page.
pub fn extract_search_text(b: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> ExtractedSearchText {
    let text_page = b.FPDFText_LoadPage(page);
    if text_page.is_null() {
        return ExtractedSearchText {
            raw: String::new(),
            char_count: 0,
        };
    }
    let count = b.FPDFText_CountChars(text_page).max(0);
    let mut raw = String::with_capacity(count as usize);
    for index in 0..count {
        raw.push(char::from_u32(b.FPDFText_GetUnicode(text_page, index)).unwrap_or(' '));
    }
    b.FPDFText_ClosePage(text_page);
    ExtractedSearchText {
        raw,
        char_count: count as u32,
    }
}

pub fn extract_page(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    disp_w: f32,
    disp_h: f32,
) -> ExtractedPage {
    let tp = b.FPDFText_LoadPage(page);
    if tp.is_null() {
        return ExtractedPage {
            raw: String::new(),
            runs: Vec::new(),
            boxes: Vec::new(),
            sizes: Vec::new(),
            math: Vec::new(),
            char_count: 0,
        };
    }
    let n = b.FPDFText_CountChars(tp).max(0);
    let mapper = DispMapper::new(b, page, disp_w, disp_h);

    let mut raw = String::with_capacity(n as usize);
    let mut runs: Vec<TextRun> = Vec::new();
    let mut boxes = vec![[0.0; 4]; n as usize];
    let mut sizes = vec![0.0f32; n as usize];
    let mut math = vec![false; n as usize];
    // Font names repeat for every glyph of a run; classify each name once.
    let mut math_by_font: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();

    struct Run {
        text: String,
        start: u32,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        last_right: f32,
        char_h: f32,
    }
    let mut cur: Option<Run> = None;

    let flush = |cur: &mut Option<Run>, runs: &mut Vec<TextRun>| {
        if let Some(r) = cur.take() {
            if !r.text.trim().is_empty() && r.x1 > r.x0 && r.y1 > r.y0 {
                runs.push(TextRun {
                    text: r.text,
                    start: r.start,
                    x: r.x0,
                    y: r.y0,
                    w: r.x1 - r.x0,
                    h: r.y1 - r.y0,
                });
            }
        }
    };

    for i in 0..n {
        let u = b.FPDFText_GetUnicode(tp, i);
        let ch = char::from_u32(u).unwrap_or(' ');
        sizes[i as usize] = b.FPDFText_GetFontSize(tp, i) as f32;
        let font = font_name_at(b, tp, i);
        math[i as usize] = *math_by_font
            .entry(font)
            .or_insert_with_key(|name| is_math_font(name));
        if ch == '\n' || ch == '\r' {
            raw.push(ch);
            flush(&mut cur, &mut runs);
            continue;
        }
        raw.push(ch);

        let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
        let ok = b.FPDFText_GetCharBox(tp, i, &mut l, &mut r, &mut bo, &mut t) != 0;
        if !ok || (r - l).abs() < f64::EPSILON && (t - bo).abs() < f64::EPSILON {
            // zero-size char (e.g. some spaces): keep in raw, do not extend run
            continue;
        }
        let (x, y, w, h) = mapper.map_box(l, r, bo, t);
        boxes[i as usize] = [x, y, w, h];

        let split = match &cur {
            None => true,
            Some(run) => {
                let vertical_drift =
                    ((y + h / 2.0) - (run.y0 + (run.y1 - run.y0) / 2.0)).abs() > run.char_h * 0.6;
                let gap = x - run.last_right;
                vertical_drift || gap > run.char_h * 0.6 || gap < -run.char_h * 2.0
            }
        };
        if split {
            flush(&mut cur, &mut runs);
            cur = Some(Run {
                text: String::new(),
                start: i as u32,
                x0: x,
                y0: y,
                x1: x + w,
                y1: y + h,
                last_right: x + w,
                char_h: h.max(1.0),
            });
        }
        let run = cur.as_mut().unwrap();
        run.text.push(ch);
        run.x0 = run.x0.min(x);
        run.y0 = run.y0.min(y);
        run.x1 = run.x1.max(x + w);
        run.y1 = run.y1.max(y + h);
        run.last_right = x + w;
        run.char_h = run.char_h.max(h);
    }
    flush(&mut cur, &mut runs);
    b.FPDFText_ClosePage(tp);

    ExtractedPage {
        raw,
        runs,
        boxes,
        sizes,
        math,
        char_count: n as u32,
    }
}

/// Merge cached character boxes into visual-line rectangles for a match.
/// This avoids reopening a PDFium text page for every visible search hit.
pub fn merge_match_rects(boxes: &[[f32; 4]], start: u32, len: u32) -> Vec<[f32; 4]> {
    let mut rects: Vec<[f32; 4]> = Vec::new();
    let end = start.saturating_add(len).min(boxes.len() as u32);
    for i in start..end {
        let [x, y, w, h] = boxes[i as usize];
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        // merge into the previous rect when on the same visual line
        if let Some(prev) = rects.last_mut() {
            let same_line =
                (prev[1] + prev[3] / 2.0 - (y + h / 2.0)).abs() < h * 0.6 && x >= prev[0] - h * 0.5;
            if same_line {
                let right = (prev[0] + prev[2]).max(x + w);
                let top = (prev[1] + prev[3]).max(y + h);
                prev[0] = prev[0].min(x);
                prev[1] = prev[1].min(y);
                prev[2] = right - prev[0];
                prev[3] = top - prev[1];
                continue;
            }
        }
        rects.push([x, y, w, h]);
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_families_tex_sets_mathematics_in() {
        // Names arrive subsetted, with a six-letter tag: "EWJNXA+CMMI10".
        for name in [
            "EWJNXA+CMMI10",
            "AUHLHB+CMSY10",
            "ZUKTGO+CMEX10",
            "TONINC+MSAM10",
            "XSBJNE+MSBM10",
            "CMMI7",
        ] {
            assert!(is_math_font(name), "{name}");
        }
    }

    #[test]
    fn does_not_mistake_text_families_for_mathematics() {
        for name in [
            "FVZQGE+NimbusRomNo9L-Regu",
            "XKNVGO+CMR10",
            "Times-Roman",
            "Helvetica",
            "XTETFA+NimbusMonL-Regu",
            "",
        ] {
            assert!(!is_math_font(name), "{name}");
        }
    }

    #[test]
    fn cached_match_boxes_merge_visual_lines_without_pdfium_reload() {
        let boxes = vec![
            [10.0, 20.0, 5.0, 8.0],
            [15.0, 20.0, 6.0, 8.0],
            [2.0, 5.0, 4.0, 8.0],
        ];
        assert_eq!(
            merge_match_rects(&boxes, 0, 3),
            vec![[10.0, 20.0, 11.0, 8.0], [2.0, 5.0, 4.0, 8.0]]
        );
    }
}

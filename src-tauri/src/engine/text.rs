//! Text extraction on the engine thread. Character boxes come back in raw
//! PDF user space; we convert them through FPDF_PageToDevice (at 100×
//! precision) into display-normalized page space (origin bottom-left of the
//! displayed page, y-up, points) so the frontend never sees crop-box or
//! /Rotate weirdness. The raw character stream is kept 1:1 with PDFium char
//! indices so search matches can be mapped back to page rects.

use crate::engine::types::TextRun;
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE};

const PREC: f64 = 100.0;

pub struct ExtractedPage {
    /// one Rust char per PDFium char index (controls/unmappables become ' ')
    pub raw: String,
    pub runs: Vec<TextRun>,
    pub char_count: u32,
}

struct DispMapper<'a> {
    b: &'a dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    dw: i32,
    dh: i32,
    disp_h: f32,
}

impl<'a> DispMapper<'a> {
    fn new(b: &'a dyn PdfiumLibraryBindings, page: FPDF_PAGE, disp_w: f32, disp_h: f32) -> Self {
        DispMapper {
            b,
            page,
            dw: (disp_w as f64 * PREC) as i32,
            dh: (disp_h as f64 * PREC) as i32,
            disp_h,
        }
    }

    /// user-space point → display-normalized (y-up) point
    fn map(&self, ux: f64, uy: f64) -> (f32, f32) {
        let (mut dx, mut dy) = (0i32, 0i32);
        self.b.FPDF_PageToDevice(
            self.page, 0, 0, self.dw, self.dh, 0, ux, uy, &mut dx, &mut dy,
        );
        (
            (dx as f64 / PREC) as f32,
            self.disp_h - (dy as f64 / PREC) as f32,
        )
    }

    /// user-space char box → display rect (x, y_bottom, w, h), y-up
    fn map_box(&self, l: f64, r: f64, b_: f64, t: f64) -> (f32, f32, f32, f32) {
        let (x1, y1) = self.map(l, b_);
        let (x2, y2) = self.map(r, t);
        let x = x1.min(x2);
        let y = y1.min(y2);
        (x, y, (x1 - x2).abs(), (y1 - y2).abs())
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
            char_count: 0,
        };
    }
    let n = b.FPDFText_CountChars(tp).max(0);
    let mapper = DispMapper::new(b, page, disp_w, disp_h);

    let mut raw = String::with_capacity(n as usize);
    let mut runs: Vec<TextRun> = Vec::new();

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

    let mut flush = |cur: &mut Option<Run>, runs: &mut Vec<TextRun>| {
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
        char_count: n as u32,
    }
}

/// Merged line rects (display space) for a match range of PDFium chars.
pub fn match_rects(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    disp_w: f32,
    disp_h: f32,
    start: u32,
    len: u32,
) -> Vec<[f32; 4]> {
    let tp = b.FPDFText_LoadPage(page);
    if tp.is_null() {
        return Vec::new();
    }
    let n = b.FPDFText_CountChars(tp).max(0) as u32;
    let mapper = DispMapper::new(b, page, disp_w, disp_h);
    let mut rects: Vec<[f32; 4]> = Vec::new();
    let end = (start + len).min(n);
    for i in start..end {
        let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
        if b.FPDFText_GetCharBox(tp, i as i32, &mut l, &mut r, &mut bo, &mut t) == 0 {
            continue;
        }
        let (x, y, w, h) = mapper.map_box(l, r, bo, t);
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        // merge into the previous rect when on the same visual line
        if let Some(prev) = rects.last_mut() {
            let same_line = (prev[1] + prev[3] / 2.0 - (y + h / 2.0)).abs() < h * 0.6
                && x >= prev[0] - h * 0.5;
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
    b.FPDFText_ClosePage(tp);
    rects
}

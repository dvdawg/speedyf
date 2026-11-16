//! How well `glyphs::page_equation_blocks` finds a page's display
//! equations, measured against known displays: precision and recall of the
//! boxes it reports (a match is IoU >= 0.5), how tight the matched boxes
//! are, whether they keep the equation number out (and report it), and
//! whether they take in text that is no part of the display.
//!
//! Truth comes from one of two places:
//!
//! - a PDF typeset with `evalmark.sty` (src-tauri/tests/displays/, built by
//!   its build.sh): every display k is set in RGB (k, 0, 0) and its number
//!   in (k, 0, 40), all else black, so the displays are read off the
//!   characters' and paths' colours;
//! - hand labels for pages of real papers, in
//!   src-tauri/tests/displays/labels.json: `{"set", "file", "page",
//!   "displays": [{"bbox": [x, y, w, h], "number"}]}`, top-left origin
//!   points, the box the display's ink without its number. "tuning" pages
//!   are the ones the detector was developed against, "second" pages were
//!   labelled later and looked at, "holdout" pages were only measured. The
//!   files are looked for in SPEEDYF_DISPLAY_PDFS (copies of papers from
//!   ~/Downloads; nothing is written beside them).
//!
//! Read only. Run with:
//!
//!   SPEEDYF_DISPLAY_EVAL="$(bash src-tauri/tests/displays/build.sh /tmp/d)" \
//!   SPEEDYF_DISPLAY_PDFS=/path/to/copies \
//!     cargo test --manifest-path src-tauri/Cargo.toml display_eval -- --ignored --nocapture
//!
//! Each `;`-separated entry of SPEEDYF_DISPLAY_EVAL is a colour-marked PDF
//! (every page is read); the labelled pages are read when
//! SPEEDYF_DISPLAY_PDFS is set. SPEEDYF_DISPLAY_VERBOSE=1 prints each miss,
//! false alarm and loose box with its text; SPEEDYF_DISPLAY_DEBUG="name#page,..."
//! prints a page's floats and what the glyphs and the raster each found.

use crate::engine::glyphs::{self, EquationBlockDto, GlyphDto};
use crate::engine::text::DispMapper;
use crate::engine::{pdfium_init, render, vision};
use pdfium_render::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct LabelledPage {
    /// "tuning" for the pages the detector was developed against,
    /// "holdout" for pages labelled afterwards and only measured.
    set: String,
    file: String,
    page: u32,
    displays: Vec<Truth>,
}

#[derive(Deserialize, Clone, Debug)]
struct Truth {
    bbox: [f32; 4],
    number: Option<String>,
}

/// A page's truth: its displays, their number boxes, and the ink of
/// everything else on it.
struct PageTruth {
    displays: Vec<Truth>,
    numbers: Vec<Option<[f32; 4]>>,
    other: Vec<[f32; 4]>,
}

#[derive(Default)]
struct Tally {
    pages: usize,
    truth: usize,
    found: usize,
    matched: usize,
    iou_sum: f32,
    numbered: usize,
    number_right: usize,
    swallows_number: usize,
    swallows_text: usize,
    covered: usize,
}

impl Tally {
    fn add(&mut self, o: &Tally) {
        self.pages += o.pages;
        self.truth += o.truth;
        self.found += o.found;
        self.matched += o.matched;
        self.iou_sum += o.iou_sum;
        self.numbered += o.numbered;
        self.number_right += o.number_right;
        self.swallows_number += o.swallows_number;
        self.swallows_text += o.swallows_text;
        self.covered += o.covered;
    }

    fn line(&self, name: &str) -> String {
        let p = self.matched as f32 / self.found.max(1) as f32;
        let r = self.matched as f32 / self.truth.max(1) as f32;
        format!(
            "{name:<34} pages {:>3} truth {:>4} found {:>4} | P {:.3} R {:.3} F1 {:.3} | covered {:.3} | IoU {:.3} | num {}/{} | eats num {} text {}",
            self.pages,
            self.truth,
            self.found,
            p,
            r,
            if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 },
            self.covered as f32 / self.truth.max(1) as f32,
            self.iou_sum / self.matched.max(1) as f32,
            self.number_right,
            self.numbered,
            self.swallows_number,
            self.swallows_text,
        )
    }
}

fn area(r: &[f32; 4]) -> f32 {
    r[2].max(0.0) * r[3].max(0.0)
}

fn intersection(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    w.max(0.0) * h.max(0.0)
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let i = intersection(a, b);
    i / (area(a) + area(b) - i).max(1e-3)
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (l, t) = (a[0].min(b[0]), a[1].min(b[1]));
    let (r, bo) = (
        (a[0] + a[2]).max(b[0] + b[2]),
        (a[1] + a[3]).max(b[1] + b[3]),
    );
    [l, t, r - l, bo - t]
}

fn centre_in(r: &[f32; 4], b: &[f32; 4]) -> bool {
    let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    x > b[0] && x < b[0] + b[2] && y > b[1] && y < b[1] + b[3]
}

/// Score one page's blocks against its truth.
/// A page's tally, with the displays missed, the blocks that match none, and
/// the matched blocks that take in their number or other text.
type Scored = (Tally, Vec<usize>, Vec<usize>, Vec<usize>);

fn score(blocks: &[EquationBlockDto], truth: &PageTruth) -> Scored {
    let mut t = Tally {
        pages: 1,
        truth: truth.displays.len(),
        found: blocks.len(),
        ..Tally::default()
    };
    // Greedy by IoU.
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        for (j, d) in truth.displays.iter().enumerate() {
            let v = iou(&b.bbox, &d.bbox);
            if v >= 0.5 {
                pairs.push((v, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut used_b = vec![false; blocks.len()];
    let mut used_t = vec![false; truth.displays.len()];
    let mut loose = Vec::new();
    for (v, i, j) in pairs {
        if used_b[i] || used_t[j] {
            continue;
        }
        used_b[i] = true;
        used_t[j] = true;
        t.matched += 1;
        t.iou_sum += v;
        let d = &truth.displays[j];
        let b = &blocks[i];
        if d.number.is_some() {
            t.numbered += 1;
            if b.number == d.number {
                t.number_right += 1;
            }
        }
        if let Some(n) = truth.numbers[j] {
            if intersection(&b.bbox, &n) > 0.3 * area(&n) {
                t.swallows_number += 1;
                loose.push(i);
            }
        }
        let eats = truth
            .other
            .iter()
            .any(|o| centre_in(o, &b.bbox) && !centre_in(o, &d.bbox));
        if eats {
            t.swallows_text += 1;
            loose.push(i);
        }
    }
    // Covered: most of a display's area under blocks that lie mostly in it
    // (a display found as two pieces, or with a loose edge).
    for d in &truth.displays {
        let share: f32 = blocks
            .iter()
            .filter(|b| intersection(&b.bbox, &d.bbox) >= 0.6 * area(&b.bbox))
            .map(|b| intersection(&b.bbox, &d.bbox))
            .sum();
        if share >= 0.7 * area(&d.bbox) {
            t.covered += 1;
        }
    }
    let misses = (0..truth.displays.len()).filter(|&j| !used_t[j]).collect();
    let alarms = (0..blocks.len()).filter(|&i| !used_b[i]).collect();
    (t, misses, alarms, loose)
}

/// The text of the glyphs whose centres lie in `r`, top down.
fn text_in(glyphs: &[GlyphDto], r: &[f32; 4]) -> String {
    let mut inside: Vec<&GlyphDto> = glyphs
        .iter()
        .filter(|g| !g.is_rule() && centre_in(&[g.x, g.y, g.w, g.h], r))
        .collect();
    inside.sort_by(|a, b| {
        (a.baseline / 4.0)
            .round()
            .total_cmp(&(b.baseline / 4.0).round())
            .then(a.x.total_cmp(&b.x))
    });
    let s: String = inside.iter().map(|g| g.text.as_str()).collect();
    s.chars().take(90).collect()
}

/// Truth from colour: display k in (k, 0, 0), its number in (k, 0, 40).
///
/// A display ending a page leaves its colour on pdfTeX's colour stack for
/// the top of the next page (a float placed there comes out in it), so a
/// display already seen on an earlier page of the document is not taken
/// again.
fn colour_truth(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    w: f32,
    h: f32,
    seen: &mut std::collections::HashSet<u32>,
) -> PageTruth {
    let mapper = DispMapper::new(b, page, w, h);
    let mut bodies: BTreeMap<u32, [f32; 4]> = BTreeMap::new();
    let mut numbers: BTreeMap<u32, ([f32; 4], String)> = BTreeMap::new();
    let mut other = Vec::new();
    let tp = b.FPDFText_LoadPage(page);
    let count = b.FPDFText_CountChars(tp);
    for i in 0..count {
        let ch = char::from_u32(b.FPDFText_GetUnicode(tp, i)).unwrap_or(' ');
        if ch.is_whitespace() || b.FPDFText_IsGenerated(tp, i) == 1 {
            continue;
        }
        let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
        if b.FPDFText_GetCharBox(tp, i, &mut l, &mut r, &mut bo, &mut t) == 0 || r <= l || t <= bo {
            continue;
        }
        let (x, y_up, bw, bh) = mapper.map_box(l, r, bo, t);
        let rect = [x, h - (y_up + bh), bw, bh];
        let (mut cr, mut cg, mut cb, mut ca) = (0u32, 0u32, 0u32, 0u32);
        b.FPDFText_GetFillColor(tp, i, &mut cr, &mut cg, &mut cb, &mut ca);
        if (1..=200).contains(&cr) && cg == 0 && cb == 0 {
            let e = bodies.entry(cr).or_insert(rect);
            *e = union(*e, rect);
        } else if (1..=200).contains(&cr) && cg == 0 && cb == 40 {
            let e = numbers.entry(cr).or_insert((rect, String::new()));
            e.0 = union(e.0, rect);
            e.1.push(ch);
        } else {
            other.push(rect);
        }
    }
    b.FPDFText_ClosePage(tp);
    // Fraction bars, radicals' overbars: filled paths in a display's colour.
    for index in 0..b.FPDFPage_CountObjects(page) {
        let object = b.FPDFPage_GetObject(page, index);
        if b.FPDFPageObj_GetType(object) != 2 {
            continue;
        }
        let (mut cr, mut cg, mut cb, mut ca) = (0u32, 0u32, 0u32, 0u32);
        if b.FPDFPageObj_GetFillColor(object, &mut cr, &mut cg, &mut cb, &mut ca) == 0
            || !(1..=200).contains(&cr)
            || cg != 0
            || cb != 0
        {
            continue;
        }
        let (mut l, mut bo, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
        if b.FPDFPageObj_GetBounds(object, &mut l, &mut bo, &mut r, &mut t) == 0 {
            continue;
        }
        let (x, y_up, bw, bh) = mapper.map_box(l as f64, r as f64, bo as f64, t as f64);
        let rect = [x, h - (y_up + bh), bw, bh];
        if let Some(e) = bodies.get_mut(&cr) {
            *e = union(*e, rect);
        }
    }
    bodies.retain(|k, _| !seen.contains(k));
    let mut displays = Vec::new();
    let mut number_boxes = Vec::new();
    seen.extend(bodies.keys().copied());
    for (k, bbox) in bodies {
        let n = numbers.get(&k);
        displays.push(Truth {
            bbox,
            // The first of an `align`'s numbers; a tag of nothing but a
            // symbol, "(*)", is no number.
            number: n
                .and_then(|(_, s)| {
                    let inner = s.strip_prefix('(')?;
                    Some(inner[..inner.find(')').unwrap_or(inner.len())].to_string())
                })
                .filter(|s| s.chars().any(|c| c.is_ascii_digit())),
        });
        number_boxes.push(n.map(|(r, _)| *r));
    }
    PageTruth {
        displays,
        numbers: number_boxes,
        other,
    }
}

fn label_truth(glyphs: &[GlyphDto], displays: Vec<Truth>) -> PageTruth {
    let other = glyphs
        .iter()
        .filter(|g| !g.is_rule())
        .map(|g| g.ink.unwrap_or([g.x, g.y, g.w, g.h]))
        .filter(|r| !displays.iter().any(|d| centre_in(r, &d.bbox)))
        .collect();
    // A number is what sits level with the display to its right as "(n)":
    // part of `other`, so a block taking it in shows as eating text, which
    // is close enough for hand labels.
    let numbers = displays.iter().map(|_| None).collect();
    PageTruth {
        displays,
        numbers,
        other,
    }
}

#[test]
#[ignore]
fn display_eval() {
    let marked = std::env::var("SPEEDYF_DISPLAY_EVAL").unwrap_or_default();
    let copies = std::env::var("SPEEDYF_DISPLAY_PDFS").ok();
    let verbose = std::env::var("SPEEDYF_DISPLAY_VERBOSE").is_ok();
    let debug = std::env::var("SPEEDYF_DISPLAY_DEBUG").ok();
    let _guard = pdfium_init::test_guard();
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("pdfium"));
    let b = pdfium.bindings();

    // (name, path, page, (set, labels) or None for colour)
    type Job = (String, String, u32, Option<(String, Vec<Truth>)>);
    let mut jobs: Vec<Job> = Vec::new();
    for path in marked.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let raw = render::open_document(b, path, None).expect("open");
        let n = b.FPDF_GetPageCount(raw);
        b.FPDF_CloseDocument(raw);
        let name = std::path::Path::new(path)
            .file_name()
            .map_or(path.to_string(), |f| f.to_string_lossy().into_owned());
        for page in 0..n as u32 {
            jobs.push((name.clone(), path.to_string(), page, None));
        }
    }
    if let Some(dir) = &copies {
        let labels: Vec<LabelledPage> =
            serde_json::from_str(include_str!("../../tests/displays/labels.json"))
                .expect("labels.json");
        for l in labels {
            let path = std::path::Path::new(dir).join(&l.file);
            jobs.push((
                l.file.clone(),
                path.to_string_lossy().into_owned(),
                l.page,
                Some((l.set, l.displays)),
            ));
        }
    }

    let mut by_doc: BTreeMap<String, Tally> = BTreeMap::new();
    let mut sets: BTreeMap<String, Tally> = BTreeMap::new();
    let mut seen: std::collections::HashMap<String, std::collections::HashSet<u32>> =
        std::collections::HashMap::new();
    for (name, path, page, labels) in jobs {
        let raw = render::open_document(b, &path, None).expect("open");
        let Some([w, h]) = render::page_display_size(b, raw, page as i32) else {
            b.FPDF_CloseDocument(raw);
            continue;
        };
        let handle = b.FPDF_LoadPage(raw, page as i32);
        let all = glyphs::extract_page_glyphs(b, handle, w, h);
        let raster = vision::render_raster(b, handle, w, h);
        let runs = crate::engine::text::extract_page(b, handle, w, h).runs;
        let blocks = glyphs::page_equation_blocks(&all, raster.as_ref(), &runs, w, h);
        if debug
            .as_deref()
            .is_some_and(|d| d.split(",").any(|p| p == format!("{name}#{page}")))
        {
            for f in crate::engine::floats::float_regions(&runs, w, h) {
                let f = [f[0], h - f[1] - f[3], f[2], f[3]];
                println!(
                    "FLOAT {name}#{page} {:?} {}",
                    f.map(|v| v.round()),
                    text_in(&all, &f)
                );
            }
            for k in glyphs::equation_blocks(&all, w, h) {
                println!(
                    "GLYPH {name}#{page} {:?} {:?} {}",
                    k.bbox.map(|v| v.round()),
                    k.number,
                    text_in(&all, &k.bbox)
                );
            }
            if let Some(r) = raster.as_ref() {
                for k in glyphs::raster_displays(&all, r) {
                    println!(
                        "RASTER {name}#{page} {:?} {:?} {}",
                        k.bbox.map(|v| v.round()),
                        k.number,
                        text_in(&all, &k.bbox)
                    );
                }
            }
        }
        let set = labels.as_ref().map_or("generated".to_string(), |(set, _)| {
            format!("labelled {set}")
        });
        let truth = match labels {
            Some((_, l)) => label_truth(&all, l),
            None => colour_truth(b, handle, w, h, seen.entry(path.clone()).or_default()),
        };
        b.FPDF_ClosePage(handle);
        b.FPDF_CloseDocument(raw);
        let (t, misses, alarms, loose) = score(&blocks, &truth);
        if verbose {
            for j in misses {
                let d = &truth.displays[j];
                println!(
                    "MISS  {name}#{page} {:?} ({:?}) {}",
                    d.bbox.map(|v| v.round()),
                    d.number,
                    text_in(&all, &d.bbox)
                );
            }
            for i in loose {
                let k = &blocks[i];
                println!(
                    "LOOSE {name}#{page} {:?} ({:?}) {}",
                    k.bbox.map(|v| v.round()),
                    k.number,
                    text_in(&all, &k.bbox)
                );
            }
            for i in alarms {
                let k = &blocks[i];
                println!(
                    "FALSE {name}#{page} {:?} ({:?}) {}",
                    k.bbox.map(|v| v.round()),
                    k.number,
                    text_in(&all, &k.bbox)
                );
            }
        }
        sets.entry(set).or_default().add(&t);
        by_doc.entry(name).or_default().add(&t);
    }
    for (name, t) in &by_doc {
        println!("{}", t.line(name));
    }
    for (set, t) in &sets {
        println!("{}", t.line(&format!("ALL {set}")));
    }
}

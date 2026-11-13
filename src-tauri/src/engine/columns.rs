//! Reading segments for column zoom: where a page's columns are, and the
//! order to read them in.
//!
//! A two-column paper is read down the left column, then down the right,
//! then on to the next page; a title, an abstract, a wide figure or a
//! footnote run across both columns interrupts that flow and is read where
//! it stands. This module turns one page into that ordered list - full-width
//! blocks and column blocks, each with a rectangle - so the viewer can fit
//! one at a time to the window and step from each to the next.
//!
//! The rules work on boxes alone, so they are ordinary unit tests:
//!
//! 1. Text runs are joined into line pieces (a gap wider than about an em
//!    ends a piece, so a line stops at a column gutter).
//! 2. A coverage profile across the page counts the pieces over each x. A
//!    gutter is an interior valley in it, a strip that almost no piece
//!    crosses, with text on both sides. Lines wider than most of the text
//!    area (a title, an abstract) are left out of the count, so they cannot
//!    fill a gutter in.
//! 3. The valleys cut the text area into slots. A slot much narrower than
//!    the widest - line numbers, margin notes, an arXiv stamp up the side -
//!    is the margin, not a column, and is ignored.
//! 4. Every piece and every graphic (an image, a drawing) that crosses a
//!    gutter is full-width; the rest belong to their slot. Full-width items
//!    stack into blocks; the column items between two blocks form a band,
//!    read slot by slot.
//! 5. A band must read like prose - most of its lines span much of their
//!    column - or it is a table, and becomes a full-width block.
//!
//! When a page will not answer confidently (slides, a poster, a page with no
//! text) it says so with `None`, and the viewer fits the page width instead:
//! a wrong crop is worse than no crop. A page without a text layer is read
//! from its pixels with the same rules (`boxes_from_raster`); skew there
//! fills the gutters in, so a crooked scan comes out single-column, which is
//! the safe answer.
//!
//! Coordinates are display points from the top-left (`Bx`); the DTO turns
//! them into the y-up `[x, y_bottom, w, h]` that text runs use.

use crate::engine::text::DispMapper;
use crate::engine::vision::{self, PageRaster, PxBox};
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE};
use serde::Serialize;
use std::os::raw::c_int;

/// A box in display points, top-left origin, `y0 < y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bx {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bx {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Bx { x0, y0, x1, y1 }
    }
    fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    fn cx(&self) -> f32 {
        (self.x0 + self.x1) / 2.0
    }
    fn cy(&self) -> f32 {
        (self.y0 + self.y1) / 2.0
    }
    fn union(&self, o: &Bx) -> Bx {
        Bx::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }
    /// From a y-up `[x, y_bottom, w, h]` on a page `page_h` tall.
    pub fn from_up(r: [f32; 4], page_h: f32) -> Bx {
        Bx::new(r[0], page_h - (r[1] + r[3]), r[0] + r[2], page_h - r[1])
    }
    /// To a y-up `[x, y_bottom, w, h]` on a page `page_h` tall.
    pub fn to_up(&self, page_h: f32) -> [f32; 4] {
        [self.x0, page_h - self.y1, self.w(), self.h()]
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PageLayoutKind {
    /// No confident reading: fit the page width.
    None,
    /// One text column: its block, margins trimmed.
    Single,
    /// Columns, possibly with full-width blocks between them.
    Columns,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    /// Full width of the text area: a title, an abstract, a wide figure, a
    /// footnote run across the page; or a single-column page's text block.
    Block,
    Column,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingSegmentDto {
    pub kind: SegmentKind,
    /// y-up `[x, y_bottom, w, h]`, display points, like text runs.
    pub rect: [f32; 4],
    /// Which band (top-down) a column belongs to; blocks count as bands too.
    pub band: u32,
    /// Slot within its band, left to right; 0 for a block.
    pub column: u32,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnLayoutDto {
    pub src: u32,
    pub kind: PageLayoutKind,
    /// Columns the page is set in (0 for `None`, 1 for `Single`).
    pub columns: u32,
    /// Column bands; more than one is a page whose columns are interrupted
    /// by something full-width.
    pub bands: u32,
    /// In reading order.
    pub segments: Vec<ReadingSegmentDto>,
    /// "text" when read from the text layer, "image" from the pixels.
    pub source: &'static str,
    /// Why a page is `None`, for diagnostics: "empty", "slides", "poster".
    pub reason: &'static str,
}

/// What one page holds, in display points from the top-left.
#[derive(Clone, Debug, Default)]
pub struct PageContent {
    pub page_w: f32,
    pub page_h: f32,
    /// Text runs (a few words each), or word boxes read off a raster.
    pub text: Vec<Bx>,
    /// Images, drawings and other non-text objects.
    pub graphics: Vec<Bx>,
}

/// One page's reading, before it is put into a DTO.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub kind: PageLayoutKind,
    pub columns: u32,
    pub bands: u32,
    pub segments: Vec<(SegmentKind, Bx, u32, u32)>,
    pub reason: &'static str,
}

impl Analysis {
    fn none(reason: &'static str) -> Self {
        Analysis {
            kind: PageLayoutKind::None,
            columns: 0,
            bands: 0,
            segments: Vec::new(),
            reason,
        }
    }

    pub fn into_dto(self, src: u32, page_h: f32, source: &'static str) -> ColumnLayoutDto {
        ColumnLayoutDto {
            src,
            kind: self.kind,
            columns: self.columns,
            bands: self.bands,
            segments: self
                .segments
                .into_iter()
                .map(|(kind, b, band, column)| ReadingSegmentDto {
                    kind,
                    rect: b.to_up(page_h),
                    band,
                    column,
                })
                .collect(),
            source,
            reason: self.reason,
        }
    }
}

/// A page with fewer text runs than this is read from its pixels instead.
pub const MIN_TEXT_RUNS: usize = 8;
/// A page larger than this either way is a poster or a drawing.
const POSTER_MIN_PT: f32 = 1500.0;
/// A gap wider than this many line heights ends a line piece. An
/// interword space, even a stretched one, stays under it; a column gutter
/// (an em and a quarter at the narrowest, IEEE's) does not.
const LINE_JOIN_GAP: f32 = 0.9;
/// Pieces wider than this share of the text area are full-width lines and
/// stay out of the coverage profile.
const WIDE_LINE: f32 = 0.7;
/// A bin crossed by at most this share of the profile's typical height is
/// a valley.
const VALLEY_SHARE: f32 = 0.35;
/// A columns reading may leave at most this share of the page's text, by
/// width, outside all of its segments.
const MAX_LOST_SHARE: f32 = 0.02;
/// A band with a full-width rule this many line heights above and below it
/// is a table.
const TABLE_RULE_GAP: f32 = 4.5;
/// At least this share of a page's text, by width, is in its columns, or
/// the page is single-column.
const COLUMN_SHARE: f32 = 0.4;
/// A gutter runs uncrossed for at least this many line heights somewhere,
/// and a column band is at least this tall.
const GUTTER_RUN: f32 = 5.0;
/// How far past a gutter's edges, in points, a line must reach to cross it.
const CROSS_REACH: f32 = 1.0;
/// A gutter is at least this many line heights wide...
const MIN_GUTTER: f32 = 0.6;
/// ...and each column at least this many line heights wide: about twelve
/// ems, wider than any table column and narrower than any text column.
const MIN_COLUMN: f32 = 14.0;
/// ...and at least this share of the text area: the label column of a table
/// of contents is not a column of text. Four columns is the most this reads.
const MIN_COLUMN_SHARE: f32 = 0.22;
/// A slot narrower than this share of the widest is margin material.
const NARROW_SLOT: f32 = 0.4;
/// Full-width items closer than this many line heights are one block.
const BLOCK_JOIN: f32 = 1.2;
/// In a prose column at least this share of the text, by width, is in lines
/// spanning half of it.
const PROSE_SHARE: f32 = 0.4;
/// A single-column block is never narrower than this share of the page: a
/// title page's short centred lines must not zoom to a headline crop.
const MIN_SINGLE_SHARE: f32 = 0.55;
/// Running heads and page numbers sit within this share of the page edge.
const FURNITURE_EDGE: f32 = 0.08;

/// Read one page from its text layer, or from its pixels (`raster`, asked
/// for only then) when the text layer is missing, nearly empty or garbled.
/// Returns the reading and where it came from: "text" or "image".
pub fn analyze_page<R: std::ops::Deref<Target = PageRaster>>(
    mut content: PageContent,
    raster: impl FnOnce() -> Option<R>,
) -> (Analysis, &'static str) {
    if content.text.len() >= MIN_TEXT_RUNS {
        let a = analyze(&content);
        if a.reason != "garbled" && a.reason != "empty" {
            return (a, "text");
        }
    }
    match raster() {
        Some(r) => {
            (content.text, content.graphics) = boxes_from_raster(&r);
            (analyze(&content), "image")
        }
        None => (analyze(&content), "text"),
    }
}

/// Read one page.
pub fn analyze(page: &PageContent) -> Analysis {
    let (pw, ph) = (page.page_w, page.page_h);
    if !(pw > 1.0 && ph > 1.0) {
        return Analysis::none("empty");
    }
    if pw.max(ph) > POSTER_MIN_PT {
        return Analysis::none("poster");
    }
    let clip = |b: &Bx| {
        let c = Bx::new(b.x0.max(0.0), b.y0.max(0.0), b.x1.min(pw), b.y1.min(ph));
        (c.w() > 0.2 && c.h() > 0.2).then_some(c)
    };
    let runs: Vec<Bx> = page.text.iter().filter_map(clip).collect();
    let mut lines = join_lines(&runs);
    let graphics: Vec<Bx> = page
        .graphics
        .iter()
        .filter_map(clip)
        .filter(|g| g.w() * g.h() < 0.5 * pw * ph && (g.w() > 1.5 || g.h() > 1.5))
        .collect();
    if lines.len() < 3 {
        return Analysis::none("empty");
    }
    // A text layer whose boxes are mostly degenerate (an OCR layer set in
    // zero-height type, glyphs piled at one spot) says nothing about where
    // the text is.
    if runs.len() * 2 < page.text.len() {
        return Analysis::none("garbled");
    }
    // Dot leaders, rules and stray punctuation are pieces too; the line
    // height is read from the ones tall enough to be letters, which must be
    // most of the text by width.
    let letters = |l: &&Bx| l.h() >= 3.0;
    let tall_w: f32 = lines.iter().filter(letters).map(|l| l.w()).sum();
    if tall_w < 0.5 * lines.iter().map(|l| l.w()).sum::<f32>() {
        return Analysis::none("garbled");
    }
    let line_h = median(lines.iter().filter(letters).map(|l| l.h()).collect()).max(1.0);
    if pw > 1.15 * ph && (line_h >= 11.0 || lines.len() < 25 || pw < 500.0) {
        return Analysis::none("slides");
    }

    let mut graphics = graphics;
    drop_furniture(&mut lines, &mut graphics, ph, line_h);
    if lines.len() < 3 {
        return Analysis::none("empty");
    }

    let slots = find_slots(&lines, &runs, line_h);
    for _ in 0..slots.gutters.len() {
        lines = split_at_gutters(&lines, &runs, &slots.gutters);
    }
    let items = classify(&lines, &graphics, &slots);
    let body: Vec<Bx> = items
        .iter()
        .filter(|(_, c, _)| *c != Class::Margin)
        .map(|(b, _, _)| *b)
        .collect();
    let Some(area) = bounds(&body) else {
        return Analysis::none("empty");
    };
    if slots.kept.len() < 2 {
        return single(area, pw);
    }

    let entries = bands(&items, &slots, line_h);
    // Columns must carry the page: a single-column page with two figures
    // side by side, or a split display line, is still single-column.
    let columns: Vec<Bx> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Band(cols) => Some(cols.iter().flatten().copied()),
            Entry::Block(_) => None,
        })
        .flatten()
        .collect();
    let (mut inside, mut total) = (0.0f32, 0.0f32);
    for (b, c, text) in &items {
        if !*text || *c == Class::Margin {
            continue;
        }
        total += b.w();
        if columns
            .iter()
            .any(|r| b.cx() >= r.x0 && b.cx() <= r.x1 && b.cy() >= r.y0 && b.cy() <= r.y1)
        {
            inside += b.w();
        }
    }
    // And somewhere two of them must stand side by side; a band holding
    // one column alone is only the end of a column the page also has.
    let side_by_side = entries.iter().any(|e| match e {
        Entry::Band(cols) => cols.iter().flatten().count() >= 2,
        Entry::Block(_) => false,
    });
    if !side_by_side || inside < COLUMN_SHARE * total {
        return single(area, pw);
    }
    let mut segments = Vec::new();
    let mut band_count = 0;
    for (band, entry) in entries.iter().enumerate() {
        match entry {
            Entry::Block(b) => segments.push((
                SegmentKind::Block,
                Bx::new(area.x0, b.y0, area.x1, b.y1),
                band as u32,
                0,
            )),
            Entry::Band(cols) => {
                band_count += 1;
                for (column, b) in cols.iter().enumerate() {
                    if let Some(b) = b {
                        segments.push((SegmentKind::Column, *b, band as u32, column as u32));
                    }
                }
            }
        }
    }
    // The last word: whatever the reading leaves out of every segment is a
    // crop the reader never sees. A page that loses more than a sliver of
    // its text that way (a garbled OCR layer, a layout the rules misread)
    // is read as one block instead.
    let lost: f32 = items
        .iter()
        .filter(|(b, c, text)| {
            *text
                && *c != Class::Margin
                && !segments.iter().any(|s| {
                    s.1.x0 <= b.x0 + 2.0
                        && s.1.x1 >= b.x1 - 2.0
                        && s.1.y0 <= b.cy()
                        && s.1.y1 >= b.cy()
                })
        })
        .map(|(b, _, _)| b.w())
        .sum();
    if lost > MAX_LOST_SHARE * total {
        return single(area, pw);
    }
    Analysis {
        kind: PageLayoutKind::Columns,
        columns: slots.kept.len() as u32,
        bands: band_count,
        segments,
        reason: "",
    }
}

fn single(area: Bx, page_w: f32) -> Analysis {
    let min_w = MIN_SINGLE_SHARE * page_w;
    let block = if area.w() >= min_w {
        area
    } else {
        let x0 = (area.cx() - min_w / 2.0).clamp(0.0, page_w - min_w);
        Bx::new(x0, area.y0, x0 + min_w, area.y1)
    };
    Analysis {
        kind: PageLayoutKind::Single,
        columns: 1,
        bands: 0,
        segments: vec![(SegmentKind::Block, block, 0, 0)],
        reason: "",
    }
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn percentile(mut v: Vec<f32>, p: f32) -> f32 {
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * p).round() as usize]
}

fn bounds(boxes: &[Bx]) -> Option<Bx> {
    let mut it = boxes.iter();
    let first = *it.next()?;
    Some(it.fold(first, |acc, b| acc.union(b)))
}

/// Runs joined into line pieces: runs on one baseline whose gap is under
/// `LINE_JOIN_GAP` line heights.
fn join_lines(runs: &[Bx]) -> Vec<Bx> {
    let mut sorted: Vec<Bx> = runs.to_vec();
    sorted.sort_by(|a, b| a.cy().total_cmp(&b.cy()));
    let mut rows: Vec<Vec<Bx>> = Vec::new();
    let (mut row_cy, mut row_h) = (0.0f32, 0.0f32);
    for r in sorted {
        if let Some(row) = rows.last_mut() {
            if (r.cy() - row_cy).abs() <= 0.35 * r.h().max(row_h) {
                row.push(r);
                row_h = row_h.max(r.h());
                continue;
            }
        }
        row_cy = r.cy();
        row_h = r.h();
        rows.push(vec![r]);
    }
    let mut out = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        let mut cur: Option<Bx> = None;
        for r in row {
            cur = Some(match cur {
                Some(c) if r.x0 - c.x1 <= LINE_JOIN_GAP * c.h().min(r.h()).max(1.0) => c.union(&r),
                Some(c) => {
                    out.push(c);
                    r
                }
                None => r,
            });
        }
        out.extend(cur);
    }
    out
}

/// Running heads, folios and the rule under a head: one short line hard
/// against the top or bottom edge, set off from the body.
fn drop_furniture(lines: &mut Vec<Bx>, graphics: &mut Vec<Bx>, page_h: f32, line_h: f32) {
    for top in [true, false] {
        let mut all: Vec<Bx> = lines.iter().chain(graphics.iter()).copied().collect();
        if all.len() < 4 {
            return;
        }
        all.sort_by(|a, b| {
            if top {
                a.y0.total_cmp(&b.y0)
            } else {
                b.y1.total_cmp(&a.y1)
            }
        });
        // The group of items within half a line of the outermost one.
        let mut group = all[0];
        let mut n = 1;
        for b in &all[1..] {
            let near = if top {
                b.y0 <= group.y1 + 0.5 * line_h
            } else {
                b.y1 >= group.y0 - 0.5 * line_h
            };
            if !near {
                break;
            }
            group = group.union(b);
            n += 1;
        }
        let Some(next) = all.get(n) else { return };
        let gap = if top {
            next.y0 - group.y1
        } else {
            group.y0 - next.y1
        };
        let at_edge = if top {
            group.y1 <= FURNITURE_EDGE * page_h
        } else {
            group.y0 >= (1.0 - FURNITURE_EDGE) * page_h
        };
        let text_h = lines
            .iter()
            .filter(|l| l.y0 >= group.y0 && l.y1 <= group.y1)
            .map(|l| l.h())
            .fold(0.0f32, f32::max);
        if at_edge && gap >= 0.8 * line_h && group.h() <= 2.0 * line_h && text_h <= 1.3 * line_h {
            let inside = |b: &Bx| b.y0 >= group.y0 - 0.01 && b.y1 <= group.y1 + 0.01;
            lines.retain(|b| !inside(b));
            graphics.retain(|b| !inside(b));
        }
    }
}

/// Valleys of the coverage profile, and the slots between them.
#[derive(Clone, Debug, Default)]
struct Slots {
    /// Kept slots' text extents, left to right.
    kept: Vec<(f32, f32)>,
    /// Gutters between consecutive kept slots: `(start, end)`.
    gutters: Vec<(f32, f32)>,
    /// Extents of slots dropped as margin material.
    margins: Vec<(f32, f32)>,
}

fn find_slots(lines: &[Bx], runs: &[Bx], line_h: f32) -> Slots {
    let Some(area) = bounds(lines) else {
        return Slots::default();
    };
    let (tx0, tw) = (area.x0.floor(), area.w());
    let n = (area.x1.ceil() - tx0).clamp(1.0, 4000.0) as usize;
    let mut cov = vec![0u32; n];
    let bin = |x: f32| ((x - tx0).max(0.0) as usize).min(n);
    // Counted over joined pieces, not runs: word spaces that happen to line
    // up (a typewritten page, a column of figures) are not gutters. A line
    // that tall type or an OCR layer joined across a gutter is wide, and so
    // left out like a title.
    for l in lines.iter().filter(|l| l.w() < WIDE_LINE * tw) {
        for c in &mut cov[bin(l.x0)..bin(l.x1).max(bin(l.x0) + 1).min(n)] {
            *c += 1;
        }
    }
    let mut filled: Vec<u32> = cov.iter().copied().filter(|&c| c > 0).collect();
    filled.sort_unstable();
    let peak = filled
        .get(filled.len() * 4 / 5)
        .copied()
        .unwrap_or(0)
        .max(1);
    if peak < 3 {
        return single_slot(area);
    }
    let limit = ((peak as f32) * VALLEY_SHARE).max(1.0) as u32;
    let min_gutter = (MIN_GUTTER * line_h).max(3.0);
    // Shallow valleys first; then strictly empty strips, which catch a gutter
    // whose shallow valley ran on into a sparse margin (notes beside a
    // column cover about as little as a crossing title does). Each must
    // stand between columns somewhere on the page: a tall stretch no line
    // crosses, with lines on both sides of it.
    let mut valleys: Vec<(f32, f32)> = Vec::new();
    for limit in [limit, 0] {
        for (start, end) in runs_at_most(&cov, limit) {
            // Interior only: the text area's own edges are not gutters.
            if start == 0 || end == n {
                continue;
            }
            let (s, e) = (tx0 + start as f32, tx0 + end as f32);
            if e - s < min_gutter || valleys.iter().any(|&(a, b)| s < b && e > a) {
                continue;
            }
            if separates_columns(lines, runs, s, e, line_h) {
                valleys.push((s, e));
            }
        }
    }
    valleys.sort_by(|a, b| a.0.total_cmp(&b.0));
    if valleys.is_empty() {
        return single_slot(area);
    }

    // Slots between valleys, measured by the pieces whose centre is in them.
    let mut split = lines.to_vec();
    for _ in 0..valleys.len() {
        split = split_at_gutters(&split, runs, &valleys);
    }
    let mut raw: Vec<Option<(f32, f32)>> = vec![None; valleys.len() + 1];
    for l in &split {
        if valleys.iter().any(|&(s, e)| crosses(l, s, e)) {
            continue;
        }
        let k = valleys
            .iter()
            .filter(|&&(s, e)| (s + e) / 2.0 <= l.cx())
            .count();
        raw[k] = Some(match raw[k] {
            Some((a, b)) => (a.min(l.x0), b.max(l.x1)),
            None => (l.x0, l.x1),
        });
    }
    let widest = raw
        .iter()
        .flatten()
        .map(|(a, b)| b - a)
        .fold(0.0f32, f32::max);
    let narrow = (NARROW_SLOT * widest)
        .max(MIN_COLUMN * line_h)
        .max(MIN_COLUMN_SHARE * area.w());
    let mut slots = Slots::default();
    let mut last_kept: Option<usize> = None;
    for (k, extent) in raw.iter().enumerate() {
        let Some((a, b)) = *extent else { continue };
        if b - a < narrow {
            // Page numbers a table of contents leads to with dots are part
            // of the text: neither a column nor margin to cut away.
            if !led_to(&split, a, b, line_h) {
                slots.margins.push((a, b));
            }
            continue;
        }
        if let Some(prev) = last_kept {
            // Everything between the two kept slots is gutter, including
            // a dropped slot of line numbers set between them.
            slots.gutters.push((valleys[prev].0, valleys[k - 1].1));
        }
        slots.kept.push((a, b));
        last_kept = Some(k);
    }
    if slots.kept.len() < 2 {
        // One column, with its margin material set aside. Lines crossing
        // into the margin (a wide equation) still belong to the body.
        if slots.kept.is_empty() {
            slots.kept.push((area.x0, area.x1));
        }
        slots.gutters.clear();
    }
    slots
}

/// Whether the strip `s..e` runs between columns somewhere: a stretch at
/// least `GUTTER_RUN` lines tall that no line crosses, with lines on both
/// sides of it. A title and an abstract cross a gutter above the columns;
/// a stray wide line crosses it in the middle; neither crosses all of it.
fn separates_columns(lines: &[Bx], runs: &[Bx], s: f32, e: f32, line_h: f32) -> bool {
    let c = (s + e) / 2.0;
    let half = ((e - s) / 4.0).max(0.5);
    let mut blockers: Vec<(f32, f32)> = lines
        .iter()
        .filter(|l| l.x0 < c - half && l.x1 > c + half && !gapped(l, runs, s, e))
        .map(|l| (l.y0, l.y1))
        .collect();
    blockers.sort_by(|a, b| a.0.total_cmp(&b.0));
    let top = lines.iter().map(|l| l.y0).fold(f32::MAX, f32::min);
    let bottom = lines.iter().map(|l| l.y1).fold(f32::MIN, f32::max);
    let mut gaps = Vec::new();
    let mut from = top - 1.0;
    for (a, b) in blockers {
        if a > from {
            gaps.push((from, a));
        }
        from = from.max(b);
    }
    gaps.push((from, bottom + 1.0));
    gaps.into_iter().any(|(ga, gb)| {
        if gb - ga < GUTTER_RUN * line_h {
            return false;
        }
        let inside = |l: &&Bx| l.y0 >= ga - 0.5 && l.y1 <= gb + 0.5;
        let left = runs.iter().filter(inside).filter(|l| l.x1 <= c).count();
        let right = runs.iter().filter(inside).filter(|l| l.x0 >= c).count();
        left >= 3 && right >= 3
    })
}

/// Whether the pieces in `a..b` are mostly led to by dot leaders: a run
/// of low marks on the same line, to the left. Line numbers and margin
/// notes have none.
fn led_to(lines: &[Bx], a: f32, b: f32, line_h: f32) -> bool {
    let inside: Vec<&Bx> = lines
        .iter()
        .filter(|l| l.x0 >= a - 0.5 && l.x1 <= b + 0.5)
        .collect();
    let led = inside
        .iter()
        .filter(|item| {
            lines.iter().any(|d| {
                d.h() < 0.45 * line_h
                    && d.x1 <= item.x0 + 0.5
                    && (d.cy() - item.cy()).abs() <= 0.6 * item.h().max(line_h)
            })
        })
        .count();
    !inside.is_empty() && led * 10 >= inside.len() * 3
}

/// Maximal runs of bins whose count is at most `limit`, as `(start, end)`.
fn runs_at_most(cov: &[u32], limit: u32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut x = 0;
    while x < cov.len() {
        if cov[x] > limit {
            x += 1;
            continue;
        }
        let start = x;
        while x < cov.len() && cov[x] <= limit {
            x += 1;
        }
        out.push((start, x));
    }
    out
}

fn single_slot(area: Bx) -> Slots {
    Slots {
        kept: vec![(area.x0, area.x1)],
        ..Slots::default()
    }
}

/// Whether a box runs right across the strip `s..e`, reaching a little into
/// the text on both sides: the last word of a line poking into the gutter
/// does not cross it.
fn crosses(b: &Bx, s: f32, e: f32) -> bool {
    b.x0 < s - CROSS_REACH && b.x1 > e + CROSS_REACH
}

/// Whether a line piece across the strip `s..e` is really two lines side by
/// side: its runs leave all of the strip empty. An OCR text layer, or tall
/// type, can join a left-column line to its right-column neighbour; an
/// abstract's word space at the same place is far narrower than a gutter.
fn gapped(piece: &Bx, runs: &[Bx], s: f32, e: f32) -> bool {
    let c = (s + e) / 2.0;
    let (mut left, mut right) = (f32::MIN, f32::MAX);
    for r in runs {
        if r.cy() < piece.y0 || r.cy() > piece.y1 || r.x1 < piece.x0 || r.x0 > piece.x1 {
            continue;
        }
        if r.x0 < c && r.x1 > c {
            return false;
        }
        if r.x1 <= c {
            left = left.max(r.x1);
        } else {
            right = right.min(r.x0);
        }
    }
    // The gap must cover the whole strip, as the space between two column
    // lines does; a display equation's operator spacing near the middle of
    // the page does not.
    left <= s + 1.0 && right >= e - 1.0
}

/// Line pieces that only seem to cross a gutter (`gapped`) are cut there,
/// back into the column lines they were joined from.
fn split_at_gutters(lines: &[Bx], runs: &[Bx], gutters: &[(f32, f32)]) -> Vec<Bx> {
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let Some(&(s, e)) = gutters
            .iter()
            .find(|&&(s, e)| crosses(l, s, e) && gapped(l, runs, s, e))
        else {
            out.push(*l);
            continue;
        };
        let c = (s + e) / 2.0;
        let mine = |r: &&Bx| r.cy() >= l.y0 && r.cy() <= l.y1 && r.x1 >= l.x0 && r.x0 <= l.x1;
        let left: Vec<Bx> = runs
            .iter()
            .filter(mine)
            .filter(|r| r.x1 <= c)
            .copied()
            .collect();
        let right: Vec<Bx> = runs
            .iter()
            .filter(mine)
            .filter(|r| r.x0 >= c)
            .copied()
            .collect();
        out.extend(bounds(&left));
        out.extend(bounds(&right));
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Span,
    Slot(usize),
    Margin,
}

/// Every line and graphic with its class, and whether it is text.
fn classify(lines: &[Bx], graphics: &[Bx], slots: &Slots) -> Vec<(Bx, Class, bool)> {
    let in_margin = |b: &Bx| {
        slots
            .margins
            .iter()
            .any(|&(a, e)| b.x0 >= a - 1.0 && b.x1 <= e + 1.0)
            && !slots
                .kept
                .iter()
                .any(|&(a, e)| b.x1 > a + 1.0 && b.x0 < e - 1.0)
    };
    let class = |b: &Bx| {
        if in_margin(b) {
            return Class::Margin;
        }
        if slots.gutters.iter().any(|&(s, e)| crosses(b, s, e)) {
            return Class::Span;
        }
        Class::Slot(
            slots
                .gutters
                .iter()
                .filter(|&&(s, e)| (s + e) / 2.0 <= b.cx())
                .count(),
        )
    };
    lines
        .iter()
        .map(|b| (*b, class(b), true))
        .chain(graphics.iter().map(|b| (*b, class(b), false)))
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Block(Bx),
    /// One box per kept slot; `None` for an empty one.
    Band(Vec<Option<Bx>>),
}

/// Full-width blocks and column bands, top-down.
fn bands(items: &[(Bx, Class, bool)], slots: &Slots, line_h: f32) -> Vec<Entry> {
    let mut spans: Vec<Bx> = items
        .iter()
        .filter(|(_, c, _)| *c == Class::Span)
        .map(|(b, _, _)| *b)
        .collect();
    spans.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    // Horizontal rules across the gutter: a table's top and bottom.
    let rules: Vec<Bx> = items
        .iter()
        .filter(|(b, c, text)| *c == Class::Span && !*text && b.h() <= 2.0)
        .map(|(b, _, _)| *b)
        .collect();
    let mut blocks = merge_vertical(&spans, BLOCK_JOIN * line_h);

    // A column item that sits inside a block (a label in a wide figure, a
    // cell of a wide table) belongs to the block.
    let mut column_items: Vec<(Bx, usize, bool)> = Vec::new();
    for (b, c, text) in items {
        let Class::Slot(k) = *c else { continue };
        if let Some(block) = blocks
            .iter_mut()
            .find(|blk| b.cy() >= blk.y0 && b.cy() <= blk.y1)
        {
            block.y0 = block.y0.min(b.y0);
            block.y1 = block.y1.max(b.y1);
            continue;
        }
        column_items.push((*b, k, *text));
    }
    absorb_block_tails(&mut blocks, &mut column_items, line_h);
    blocks = merge_vertical(&blocks, 0.0);

    // Column items between consecutive blocks form one band.
    let n_bands = blocks.len() + 1;
    let mut grouped: Vec<Vec<(Bx, usize, bool)>> = vec![Vec::new(); n_bands];
    for item in column_items {
        let above = blocks.iter().filter(|blk| blk.y1 <= item.0.cy()).count();
        grouped[above].push(item);
    }
    let widths: Vec<f32> = slots.kept.iter().map(|(a, b)| b - a).collect();
    let mut entries: Vec<(f32, Entry)> = blocks.iter().map(|b| (b.y0, Entry::Block(*b))).collect();
    for group in grouped.into_iter().filter(|g| !g.is_empty()) {
        let all = bounds(&group.iter().map(|g| g.0).collect::<Vec<_>>()).expect("non-empty group");
        let n = slots.kept.len();
        let (mut text, mut drawn): (Vec<Option<Bx>>, Vec<Option<Bx>>) =
            (vec![None; n], vec![None; n]);
        let mut edges: Vec<(Vec<f32>, Vec<f32>)> = vec![(Vec::new(), Vec::new()); n];
        for (b, k, is_text) in &group {
            let k = (*k).min(n - 1);
            let into = if *is_text { &mut text } else { &mut drawn };
            into[k] = Some(into[k].map_or(*b, |c| c.union(b)));
            if *is_text {
                edges[k].0.push(b.x0);
                edges[k].1.push(b.x1);
            }
        }
        // Columns whose text overlaps side to side are not columns: a
        // skewed scan, or lines that wander across the gutter. Judged on
        // most of each column's edge, so one word poking into the gutter
        // does not decide it.
        let populated: Vec<(f32, f32)> = edges
            .into_iter()
            .filter(|(starts, _)| !starts.is_empty())
            .map(|(starts, ends)| (percentile(starts, 0.1), percentile(ends, 0.9)))
            .collect();
        let apart = populated.windows(2).all(|p| p[0].1 < p[1].0);
        // Ruled off above and below by full-width rules, it is the body of
        // a table, however much its cells read like prose.
        let ruled = |above: bool| {
            rules.iter().any(|r| {
                let gap = if above { all.y0 - r.y1 } else { r.y0 - all.y1 };
                (-0.5..=TABLE_RULE_GAP * line_h).contains(&gap)
            })
        };
        let table = ruled(true) && ruled(false);
        let entry = if apart && !table && is_prose_band(&group, &widths, line_h) {
            Entry::Band(column_boxes(text, drawn, slots))
        } else {
            Entry::Block(all)
        };
        entries.push((all.y0, entry));
    }
    entries.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Neighbouring blocks (a demoted table under its caption) are one.
    let mut out: Vec<Entry> = Vec::new();
    for (_, entry) in entries {
        match (out.last_mut(), &entry) {
            (Some(Entry::Block(prev)), Entry::Block(b)) => *prev = prev.union(b),
            _ => out.push(entry),
        }
    }
    out
}

/// The short last line of a full-width paragraph (an abstract, a caption)
/// does not cross the gutter, so it arrives as a column item. It belongs to
/// the block when it sits closer to the block than to anything in its own
/// column.
fn absorb_block_tails(blocks: &mut [Bx], items: &mut Vec<(Bx, usize, bool)>, line_h: f32) {
    loop {
        let mut moved = false;
        let mut i = 0;
        while i < items.len() {
            let (b, k, _) = items[i];
            let nearest_block = blocks
                .iter()
                .enumerate()
                .map(|(j, blk)| (j, (b.y0 - blk.y1).max(blk.y0 - b.y1)))
                .filter(|&(_, gap)| gap >= -0.5)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((j, gap)) = nearest_block else {
                i += 1;
                continue;
            };
            let neighbour = items
                .iter()
                .enumerate()
                .filter(|&(n, o)| n != i && o.1 == k)
                .map(|(_, o)| (b.y0 - o.0.y1).max(o.0.y0 - b.y1))
                .fold(f32::MAX, f32::min);
            if gap <= 0.6 * line_h && gap < neighbour - 0.1 {
                blocks[j] = blocks[j].union(&b);
                items.swap_remove(i);
                moved = true;
            } else {
                i += 1;
            }
        }
        if !moved {
            return;
        }
    }
}

/// Each column's box: all of its text, and its graphics trimmed to the slot
/// (a figure a little wider than its column must not widen the column).
/// Text is never trimmed: whatever is read must be inside the box.
fn column_boxes(
    text: Vec<Option<Bx>>,
    graphics: Vec<Option<Bx>>,
    slots: &Slots,
) -> Vec<Option<Bx>> {
    text.into_iter()
        .zip(graphics)
        .enumerate()
        .map(|(k, (t, g))| {
            let g = g.map(|b| {
                let lo = if k == 0 {
                    f32::MIN
                } else {
                    slots.gutters[k - 1].1 - 1.0
                };
                let hi = slots.gutters.get(k).map_or(f32::MAX, |&(s, _)| s + 1.0);
                let (lo, hi) = (lo.max(slots.kept[k].0 - 2.0), hi.min(slots.kept[k].1 + 2.0));
                Bx::new(b.x0.max(lo), b.y0, b.x1.min(hi).max(b.x0.max(lo)), b.y1)
            });
            match (t, g) {
                (Some(t), Some(g)) => Some(t.union(&g)),
                (t, g) => t.or(g),
            }
        })
        .collect()
}

/// Whether a band reads as prose columns rather than a table: some column
/// of it is several lines tall, and in each well-filled slot most of the
/// text is in lines spanning much of the column. A display line split by a
/// wide space, or a row of a table, is a band one line tall.
fn is_prose_band(group: &[(Bx, usize, bool)], widths: &[f32], line_h: f32) -> bool {
    let lines: Vec<&(Bx, usize, bool)> = group.iter().filter(|g| g.2).collect();
    // Somewhere a paragraph: at least three long lines in a row, each
    // within a line height of the last. The titles and axis labels of a
    // grid of plots are as wide, but never stacked like that.
    let paragraph = widths
        .iter()
        .enumerate()
        .map(|(k, width)| {
            let mut rows: Vec<Bx> = lines
                .iter()
                .filter(|g| g.1 == k && g.0.w() >= 0.5 * width)
                .map(|g| g.0)
                .collect();
            rows.sort_by(|a, b| a.y0.total_cmp(&b.y0));
            let (mut best, mut run) = (rows.len().min(1), 1);
            for pair in rows.windows(2) {
                run = if pair[1].y0 - pair[0].y1 <= line_h {
                    run + 1
                } else {
                    1
                };
                best = best.max(run);
            }
            best
        })
        .max()
        .unwrap_or(0);
    if paragraph < 3 {
        return false;
    }
    let mut checked = 0;
    for (k, width) in widths.iter().enumerate() {
        let in_slot: Vec<&&(Bx, usize, bool)> = lines.iter().filter(|g| g.1 == k).collect();
        if in_slot.len() < 4 {
            continue;
        }
        checked += 1;
        // By width, so the scattered scripts and primes of a page of
        // mathematics, each a piece of its own, do not outvote its lines.
        let total: f32 = in_slot.iter().map(|g| g.0.w()).sum();
        let long: f32 = in_slot
            .iter()
            .map(|g| g.0.w())
            .filter(|&w| w >= 0.5 * width)
            .sum();
        if long < PROSE_SHARE * total {
            return false;
        }
    }
    checked > 0 || lines.len() >= 3
}

/// Boxes sorted by top, joined when the gap between them is under `gap`.
fn merge_vertical(sorted: &[Bx], gap: f32) -> Vec<Bx> {
    let mut v: Vec<Bx> = sorted.to_vec();
    v.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    let mut out: Vec<Bx> = Vec::new();
    for b in v {
        match out.last_mut() {
            Some(last) if b.y0 <= last.y1 + gap => *last = last.union(&b),
            _ => out.push(b),
        }
    }
    out
}

// ---------- reading a page ----------

/// `FPDF_PAGEOBJ_TEXT`.
const PAGEOBJ_TEXT: c_int = 1;
/// `FPDF_PAGEOBJ_PATH`.
const PAGEOBJ_PATH: c_int = 2;
/// Objects past this many are not looked at: a dense plot's thousandth path
/// adds nothing its first ones did not.
const MAX_OBJECTS: i32 = 20_000;

/// Bounds of a page's top-level non-text objects (images, drawings, forms),
/// in display points from the top-left.
pub fn page_graphics(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    page_w: f32,
    page_h: f32,
) -> Vec<Bx> {
    let mapper = DispMapper::new(b, page, page_w, page_h);
    let count = b.FPDFPage_CountObjects(page).clamp(0, MAX_OBJECTS);
    let mut out = Vec::new();
    for index in 0..count {
        let object = b.FPDFPage_GetObject(page, index);
        if object.is_null() {
            continue;
        }
        match b.FPDFPageObj_GetType(object) {
            PAGEOBJ_TEXT => continue,
            PAGEOBJ_PATH => {
                // Neither filled nor stroked: a clip, which draws nothing.
                let (mut fill, mut stroke): (c_int, i32) = (0, 0);
                if b.FPDFPath_GetDrawMode(object, &mut fill, &mut stroke) != 0
                    && fill == 0
                    && stroke == 0
                {
                    continue;
                }
            }
            _ => {}
        }
        let (mut l, mut bo, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
        if b.FPDFPageObj_GetBounds(object, &mut l, &mut bo, &mut r, &mut t) == 0 {
            continue;
        }
        let (x, y, w, h) = mapper.map_box(l as f64, r as f64, bo as f64, t as f64);
        out.push(Bx::from_up([x, y, w, h], page_h));
    }
    out
}

/// Word boxes and graphics read off a page's pixels, for a page with no
/// text layer: connected components the size of characters, joined into
/// words; anything much larger is a graphic.
pub fn boxes_from_raster(raster: &PageRaster) -> (Vec<Bx>, Vec<Bx>) {
    let bits = &raster.bits;
    let all = vision::components(bits, PxBox::new(0, 0, bits.w, bits.h));
    let px = |pt: f32| raster.px(pt);
    let (min_h, max_h, max_w) = (px(2.0), px(30.0), px(40.0));
    let mut chars: Vec<PxBox> = Vec::new();
    let mut graphics = Vec::new();
    for c in all {
        let b = c.bbox;
        if b.h() > max_h || b.w() > max_w {
            graphics.push(b);
        } else if b.h() >= min_h && c.area >= 3 {
            chars.push(b);
        }
    }
    let char_h = {
        let mut hs: Vec<usize> = chars.iter().map(|b| b.h()).collect();
        hs.sort_unstable();
        hs.get(hs.len() / 2).copied().unwrap_or(1).max(1)
    };
    let groups = vision::group_by_dilation(&chars, (char_h / 3).max(1), 0);
    let mut words: Vec<Option<PxBox>> = vec![None; groups.iter().max().map_or(0, |m| m + 1)];
    for (b, g) in chars.iter().zip(groups) {
        words[g] = Some(words[g].map_or(*b, |w| w.union(b)));
    }
    let to_bx = |b: &PxBox| {
        let [x, y, w, h] = raster.to_pt(b);
        Bx::new(x, y, x + w, y + h)
    };
    (
        words.iter().flatten().map(to_bx).collect(),
        graphics.iter().map(to_bx).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: (f32, f32) = (612.0, 792.0);

    /// Lines of prose: `n` lines starting at `top`, `x0..x1`, 9pt tall on a
    /// 12pt pitch, each split into word runs. The last line of every
    /// paragraph of `para` lines is short.
    fn prose(x0: f32, x1: f32, top: f32, n: usize, para: usize) -> Vec<Bx> {
        let mut out = Vec::new();
        for i in 0..n {
            let y = top + 12.0 * i as f32;
            let end = if para > 0 && i % para == para - 1 {
                x0 + 0.4 * (x1 - x0)
            } else {
                x1
            };
            let mut x = x0;
            while x < end {
                let w = (end - x).min(28.0);
                out.push(Bx::new(x, y, x + w, y + 9.0));
                x += w + 3.0;
            }
        }
        out
    }

    fn page(text: Vec<Bx>, graphics: Vec<Bx>) -> PageContent {
        PageContent {
            page_w: LETTER.0,
            page_h: LETTER.1,
            text,
            graphics,
        }
    }

    fn kinds(a: &Analysis) -> Vec<(SegmentKind, u32)> {
        a.segments.iter().map(|s| (s.0, s.3)).collect()
    }

    const L: (f32, f32) = (54.0, 294.0);
    const R: (f32, f32) = (318.0, 558.0);

    #[test]
    fn two_columns_read_left_then_right() {
        let mut text = prose(L.0, L.1, 72.0, 52, 8);
        text.extend(prose(R.0, R.1, 72.0, 52, 7));
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Columns);
        assert_eq!(a.columns, 2);
        assert_eq!(
            kinds(&a),
            vec![(SegmentKind::Column, 0), (SegmentKind::Column, 1)]
        );
        let left = a.segments[0].1;
        assert!(
            (left.x0 - L.0).abs() < 1.0 && (left.x1 - L.1).abs() < 1.0,
            "{left:?}"
        );
        let right = a.segments[1].1;
        assert!((right.x0 - R.0).abs() < 1.0 && (right.x1 - R.1).abs() < 1.0);
    }

    #[test]
    fn title_and_abstract_come_before_the_columns() {
        let mut text = vec![Bx::new(150.0, 60.0, 460.0, 78.0)]; // title, large
        text.extend(prose(L.0, R.1, 100.0, 8, 0)); // abstract, full width
        text.extend(prose(L.0, L.1, 220.0, 40, 9));
        text.extend(prose(R.0, R.1, 220.0, 40, 9));
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Columns);
        assert_eq!(
            kinds(&a),
            vec![
                (SegmentKind::Block, 0),
                (SegmentKind::Column, 0),
                (SegmentKind::Column, 1)
            ]
        );
        assert!(a.segments[0].1.y0 <= 60.0 && a.segments[0].1.y1 >= 190.0);
    }

    #[test]
    fn a_wide_figure_between_two_bands_is_read_band_by_band() {
        let mut text = prose(L.0, L.1, 72.0, 20, 6);
        text.extend(prose(R.0, R.1, 72.0, 20, 6));
        // Figure artwork across both columns, then its two-line caption.
        let figure = Bx::new(80.0, 330.0, 530.0, 480.0);
        text.extend(prose(L.0, R.1, 490.0, 2, 2));
        text.extend(prose(L.0, L.1, 530.0, 18, 6));
        text.extend(prose(R.0, R.1, 530.0, 18, 6));
        let a = analyze(&page(text, vec![figure]));
        assert_eq!(a.bands, 2);
        assert_eq!(
            a.segments
                .iter()
                .map(|s| (s.0, s.2, s.3))
                .collect::<Vec<_>>(),
            vec![
                (SegmentKind::Column, 0, 0),
                (SegmentKind::Column, 0, 1),
                (SegmentKind::Block, 1, 0),
                (SegmentKind::Column, 2, 0),
                (SegmentKind::Column, 2, 1),
            ]
        );
        let block = a.segments[2].1;
        assert!(block.y0 <= 330.0 && block.y1 >= 500.0, "{block:?}");
    }

    #[test]
    fn an_unbalanced_last_page_keeps_its_short_column() {
        let mut text = prose(L.0, L.1, 72.0, 50, 8);
        text.extend(prose(R.0, R.1, 72.0, 9, 5));
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.columns, 2);
        assert_eq!(a.segments.len(), 2);
        assert!(a.segments[1].1.y1 < 200.0);
    }

    #[test]
    fn a_band_with_one_empty_column_reads_the_other() {
        // Two columns above a full-width table caption; below it, only the
        // left column continues.
        let mut text = prose(L.0, L.1, 72.0, 30, 8);
        text.extend(prose(R.0, R.1, 72.0, 30, 8));
        text.extend(prose(L.0, R.1, 450.0, 2, 2));
        text.extend(prose(L.0, L.1, 490.0, 12, 6));
        let a = analyze(&page(text, vec![]));
        let last = a.segments.last().unwrap();
        assert_eq!((last.0, last.3), (SegmentKind::Column, 0));
        assert!(last.1.x1 < 300.0);
    }

    #[test]
    fn column_lines_joined_across_a_narrow_gutter_are_split_again() {
        // A 12pt gutter, and an OCR layer whose boxes are taller on every
        // fourth line: there each left line sits within a line height of its
        // right neighbour, and the two join into one piece across the gutter.
        let mut text = Vec::new();
        for i in 0..45 {
            let y = 60.0 + 16.0 * i as f32;
            let h = if i % 4 == 0 { 14.0 } else { 9.0 };
            text.push(Bx::new(54.0, y, 294.0, y + h));
            text.push(Bx::new(306.0, y, 546.0, y + h));
        }
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Columns);
        assert_eq!(
            kinds(&a),
            vec![(SegmentKind::Column, 0), (SegmentKind::Column, 1)]
        );
    }

    #[test]
    fn a_last_page_with_one_empty_column_reads_its_text() {
        let text = prose(L.0, L.1, 72.0, 30, 8);
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Single);
        let b = a.segments[0].1;
        // Never narrower than the floor, and centred on the text.
        assert!(b.x0 <= L.0 && b.x1 >= L.1, "{b:?}");
        assert!((b.w() - MIN_SINGLE_SHARE * LETTER.0).abs() < 1.0);
    }

    #[test]
    fn a_ruled_table_of_prose_cells_is_not_a_band() {
        // A single-column page with a two-column table of sentences.
        let mut text = prose(72.0, 540.0, 72.0, 12, 6);
        text.extend(prose(80.0, 330.0, 240.0, 14, 2));
        text.extend(prose(350.0, 530.0, 240.0, 14, 2));
        text.extend(prose(72.0, 540.0, 440.0, 20, 6));
        let rules = vec![
            Bx::new(72.0, 225.0, 540.0, 225.6),
            Bx::new(72.0, 420.0, 540.0, 420.6),
        ];
        let a = analyze(&page(text, rules));
        assert_eq!(a.kind, PageLayoutKind::Single);
    }

    #[test]
    fn three_columns_read_left_to_right() {
        let cols = [(40.0, 205.0), (223.0, 388.0), (406.0, 571.0)];
        let mut text = Vec::new();
        for (a, b) in cols {
            text.extend(prose(a, b, 72.0, 50, 7));
        }
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.columns, 3);
        let xs: Vec<f32> = a.segments.iter().map(|s| s.1.x0).collect();
        assert_eq!(xs.len(), 3);
        assert!(xs[0] < xs[1] && xs[1] < xs[2]);
    }

    #[test]
    fn margin_notes_and_line_numbers_are_left_out() {
        let mut text = prose(L.0, L.1, 72.0, 50, 8);
        text.extend(prose(R.0, R.1, 72.0, 50, 8));
        // Line numbers in the left margin, margin notes in the right.
        for i in 0..50 {
            let y = 72.0 + 12.0 * i as f32;
            text.push(Bx::new(28.0, y, 40.0, y + 9.0));
        }
        for i in 0..6 {
            let y = 100.0 + 40.0 * i as f32;
            text.push(Bx::new(572.0, y, 604.0, y + 9.0));
        }
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.columns, 2);
        assert_eq!(a.segments.len(), 2);
        assert!(a.segments[0].1.x0 >= L.0 - 1.0);
        assert!(a.segments[1].1.x1 <= R.1 + 1.0, "{:?}", a.segments);
    }

    #[test]
    fn a_numbered_review_manuscript_is_single_without_its_numbers() {
        let mut text = prose(90.0, 520.0, 72.0, 50, 8);
        for i in 0..50 {
            let y = 72.0 + 12.0 * i as f32;
            text.push(Bx::new(60.0, y, 74.0, y + 9.0));
        }
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Single);
        assert!(a.segments[0].1.x0 >= 89.0, "{:?}", a.segments[0].1);
    }

    #[test]
    fn a_contents_page_keeps_its_page_numbers() {
        let mut text = Vec::new();
        for i in 0..30 {
            let y = 72.0 + 20.0 * i as f32;
            let end = 200.0 + (i % 5) as f32 * 30.0;
            text.push(Bx::new(72.0, y, end, y + 9.0));
            let mut x = end + 6.0;
            while x < 480.0 {
                text.push(Bx::new(x, y + 7.0, x + 1.0, y + 8.0));
                x += 4.0;
            }
            text.push(Bx::new(500.0, y, 515.0, y + 9.0));
        }
        let a = analyze(&page(text, vec![]));
        assert_ne!(a.kind, PageLayoutKind::Columns);
        if let Some(seg) = a.segments.first() {
            assert!(seg.1.x1 >= 515.0, "{:?}", seg.1);
        }
    }

    #[test]
    fn spanning_footnotes_are_read_last() {
        let mut text = prose(L.0, L.1, 72.0, 45, 8);
        text.extend(prose(R.0, R.1, 72.0, 45, 8));
        let rule = Bx::new(L.0, 630.0, 200.0, 630.5);
        let notes: Vec<Bx> = prose(L.0, R.1, 640.0, 3, 3)
            .into_iter()
            .map(|b| Bx::new(b.x0, b.y0, b.x1, b.y0 + 7.0))
            .collect();
        text.extend(notes);
        let a = analyze(&page(text, vec![rule]));
        let last = a.segments.last().unwrap();
        assert_eq!(last.0, SegmentKind::Block);
        assert!(last.1.y0 >= 600.0);
        assert_eq!(a.segments.len(), 3);
    }

    #[test]
    fn a_figure_inside_a_column_stays_in_it() {
        let mut text = prose(L.0, L.1, 72.0, 15, 6);
        text.extend(prose(L.0, L.1, 420.0, 25, 6));
        text.extend(prose(R.0, R.1, 72.0, 50, 6));
        let figure = Bx::new(60.0, 260.0, 296.0, 400.0); // overhangs a little
        let a = analyze(&page(text, vec![figure]));
        assert_eq!(
            kinds(&a),
            vec![(SegmentKind::Column, 0), (SegmentKind::Column, 1)]
        );
        assert!(a.segments[0].1.x1 <= R.0, "{:?}", a.segments[0].1);
    }

    #[test]
    fn a_table_on_a_single_column_page_is_not_columns() {
        let mut text = prose(72.0, 540.0, 72.0, 20, 6);
        // A table: short cells in three tall columns.
        for i in 0..30 {
            let y = 330.0 + 12.0 * i as f32;
            text.push(Bx::new(72.0, y, 120.0, y + 9.0));
            text.push(Bx::new(260.0, y, 290.0, y + 9.0));
            text.push(Bx::new(450.0, y, 500.0, y + 9.0));
        }
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Single);
    }

    #[test]
    fn a_full_page_table_does_not_become_columns() {
        let mut text = Vec::new();
        for i in 0..50 {
            let y = 72.0 + 12.0 * i as f32;
            text.push(Bx::new(72.0, y, 220.0, y + 9.0));
            text.push(Bx::new(330.0, y, 370.0, y + 9.0));
            text.push(Bx::new(460.0, y, 530.0, y + 9.0));
        }
        let a = analyze(&page(text, vec![]));
        assert_ne!(a.kind, PageLayoutKind::Columns);
    }

    #[test]
    fn a_single_column_page_trims_its_margins() {
        let text = prose(90.0, 522.0, 80.0, 50, 8);
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.kind, PageLayoutKind::Single);
        let b = a.segments[0].1;
        assert!(
            (b.x0 - 90.0).abs() < 1.0 && (b.x1 - 522.0).abs() < 1.5,
            "{b:?}"
        );
    }

    #[test]
    fn running_heads_and_folios_are_not_segments() {
        let mut text = vec![
            Bx::new(54.0, 36.0, 200.0, 44.0),
            Bx::new(540.0, 36.0, 558.0, 44.0),
            Bx::new(300.0, 750.0, 312.0, 758.0),
        ];
        text.extend(prose(L.0, L.1, 72.0, 50, 8));
        text.extend(prose(R.0, R.1, 72.0, 50, 8));
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.segments.len(), 2);
        assert!(a.segments.iter().all(|s| s.1.y0 >= 70.0 && s.1.y1 < 700.0));
    }

    #[test]
    fn a_line_running_into_the_gutter_does_not_split_the_band() {
        let mut text = prose(L.0, L.1, 72.0, 50, 8);
        text.extend(prose(R.0, R.1, 72.0, 50, 8));
        text.push(Bx::new(200.0, 300.0, 305.0, 309.0)); // an overlong URL
        let a = analyze(&page(text, vec![]));
        assert_eq!(a.bands, 1);
        assert_eq!(a.segments.len(), 2);
    }

    #[test]
    fn slides_and_posters_have_no_column_mode() {
        let slide = PageContent {
            page_w: 720.0,
            page_h: 405.0,
            text: prose(40.0, 680.0, 60.0, 8, 0)
                .into_iter()
                .map(|b| Bx::new(b.x0, b.y0 * 1.4, b.x1, b.y0 * 1.4 + 18.0))
                .collect(),
            graphics: vec![],
        };
        assert_eq!(analyze(&slide).reason, "slides");
        let mut poster_text = Vec::new();
        for (a, b) in [(60.0, 800.0), (860.0, 1600.0), (1660.0, 2400.0)] {
            poster_text.extend(prose(a, b, 300.0, 100, 10));
        }
        let poster = PageContent {
            page_w: 2448.0,
            page_h: 3168.0,
            text: poster_text,
            graphics: vec![],
        };
        assert_eq!(analyze(&poster).reason, "poster");
    }

    #[test]
    fn a_landscape_page_of_prose_is_read_normally() {
        let landscape = PageContent {
            page_w: 792.0,
            page_h: 612.0,
            text: prose(72.0, 720.0, 72.0, 38, 8),
            graphics: vec![],
        };
        assert_eq!(analyze(&landscape).kind, PageLayoutKind::Single);
    }

    #[test]
    fn an_empty_page_has_no_reading() {
        assert_eq!(analyze(&page(vec![], vec![])).kind, PageLayoutKind::None);
        let figure_only = page(vec![], vec![Bx::new(72.0, 72.0, 540.0, 600.0)]);
        assert_eq!(analyze(&figure_only).kind, PageLayoutKind::None);
    }

    #[test]
    fn rects_go_out_y_up_like_text_runs() {
        let b = Bx::new(10.0, 20.0, 110.0, 70.0);
        assert_eq!(b.to_up(792.0), [10.0, 722.0, 100.0, 50.0]);
        assert_eq!(Bx::from_up(b.to_up(792.0), 792.0), b);
    }

    /// A page of ink: blobs the size of 10pt words in the given columns,
    /// at 2px per point, each line shifted right by `skew` px per line.
    fn scanned(cols: &[(usize, usize)], skew: usize) -> PageRaster {
        let (w, h) = (1224usize, 1584usize);
        let mut bits = vision::Bits::new(w, h);
        for line in 0..48 {
            let y = 150 + line * 26;
            for &(a, b) in cols {
                let mut x = a + line * skew;
                while x + 40 < b + line * skew {
                    for yy in y..y + 16 {
                        for xx in x..x + 34 {
                            if xx < w && (xx + yy) % 3 != 0 {
                                bits.set(xx, yy, true);
                            }
                        }
                    }
                    x += 46;
                }
            }
        }
        PageRaster { bits, scale: 2.0 }
    }

    /// Reads real PDFs, read-only, and reports each page's reading: run with
    /// `SPEEDYF_COLUMN_EVAL=<file listing one PDF path per line>` and
    /// `SPEEDYF_COLUMN_EVAL_OUT=<dir>`. Writes `pages.tsv`, and a PNG per
    /// page listed in `SPEEDYF_COLUMN_EVAL_DRAW` (`<file index>:<page>`
    /// pairs, comma separated) with its segments outlined in reading order:
    /// red, green, blue, magenta, cyan, orange, then grey.
    #[test]
    #[ignore = "evaluation over local PDFs"]
    fn evaluate_local_pdfs() {
        use crate::engine::{pdfium_init, render, text};
        use std::fmt::Write as _;
        let (Ok(list), Ok(out)) = (
            std::env::var("SPEEDYF_COLUMN_EVAL"),
            std::env::var("SPEEDYF_COLUMN_EVAL_OUT"),
        ) else {
            return;
        };
        let draw: Vec<(usize, i32)> = std::env::var("SPEEDYF_COLUMN_EVAL_DRAW")
            .unwrap_or_default()
            .split(',')
            .filter_map(|p| {
                let (f, pg) = p.split_once(':')?;
                Some((f.trim().parse().ok()?, pg.trim().parse().ok()?))
            })
            .collect();
        let _guard = pdfium_init::test_guard();
        let pdfium = pdfium_render::prelude::Pdfium::new(pdfium_init::init_bindings(&[]).unwrap());
        let b = pdfium.bindings();
        let paths = std::fs::read_to_string(list).unwrap();
        let mut tsv =
            String::from("file\tpage\tkind\tcolumns\tbands\tsource\treason\tsegments\tuncovered\n");
        for (fi, path) in paths.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            let doc = b.FPDF_LoadDocument(path.trim(), None);
            if doc.is_null() {
                continue;
            }
            let count = b.FPDF_GetPageCount(doc).min(400);
            for index in 0..count {
                let Some([w, h]) = render::page_display_size(b, doc, index) else {
                    continue;
                };
                let page = b.FPDF_LoadPage(doc, index);
                if page.is_null() {
                    continue;
                }
                let runs = text::extract_page(b, page, w, h).runs;
                let content = PageContent {
                    page_w: w,
                    page_h: h,
                    text: runs
                        .iter()
                        .map(|r| Bx::from_up([r.x, r.y, r.w, r.h], h))
                        .collect(),
                    graphics: page_graphics(b, page, w, h),
                };
                let text = content.text.clone();
                let (a, source) = analyze_page(content, || {
                    vision::render_raster(b, page, w, h).map(Box::new)
                });
                let segs: Vec<String> = a
                    .segments
                    .iter()
                    .map(|s| {
                        format!(
                            "{}{}:{:.0},{:.0},{:.0},{:.0}",
                            if s.0 == SegmentKind::Block { "B" } else { "C" },
                            s.3,
                            s.1.x0,
                            s.1.y0,
                            s.1.x1,
                            s.1.y1
                        )
                    })
                    .collect();
                // Text a reader of the segments would never see: runs whose
                // centre is in no segment, or that stick out of it sideways.
                let uncovered = text
                    .iter()
                    .filter(|r| {
                        !a.segments.iter().any(|s| {
                            s.1.x0 <= r.x0 + 2.0
                                && s.1.x1 >= r.x1 - 2.0
                                && s.1.y0 <= r.cy()
                                && s.1.y1 >= r.cy()
                        })
                    })
                    .count();
                let _ = writeln!(
                    tsv,
                    "{fi}\t{index}\t{:?}\t{}\t{}\t{source}\t{}\t{}\t{uncovered}",
                    a.kind,
                    a.columns,
                    a.bands,
                    a.reason,
                    segs.join(" ")
                );
                if draw.contains(&(fi, index)) {
                    let png = render::render_page(b, page, [w, h], 1.5, 0, None, false, || false)
                        .unwrap()
                        .png;
                    let mut img = image::load_from_memory(&png).unwrap().to_rgba8();
                    let colors: [[u8; 4]; 7] = [
                        [230, 0, 0, 255],
                        [0, 170, 0, 255],
                        [0, 60, 230, 255],
                        [200, 0, 200, 255],
                        [0, 180, 200, 255],
                        [240, 140, 0, 255],
                        [120, 120, 120, 255],
                    ];
                    for (n, s) in a.segments.iter().enumerate() {
                        let c = image::Rgba(colors[n.min(6)]);
                        let (x0, y0) = ((s.1.x0 * 1.5) as i64, (s.1.y0 * 1.5) as i64);
                        let (x1, y1) = ((s.1.x1 * 1.5) as i64, (s.1.y1 * 1.5) as i64);
                        for t in 0..3 {
                            for x in x0..=x1 {
                                for y in [y0 - t, y1 + t] {
                                    if x >= 0
                                        && y >= 0
                                        && (x as u32) < img.width()
                                        && (y as u32) < img.height()
                                    {
                                        img.put_pixel(x as u32, y as u32, c);
                                    }
                                }
                            }
                            for y in y0..=y1 {
                                for x in [x0 - t, x1 + t] {
                                    if x >= 0
                                        && y >= 0
                                        && (x as u32) < img.width()
                                        && (y as u32) < img.height()
                                    {
                                        img.put_pixel(x as u32, y as u32, c);
                                    }
                                }
                            }
                        }
                    }
                    img.save(format!("{out}/f{fi}_p{index}.png")).unwrap();
                }
                b.FPDF_ClosePage(page);
            }
            b.FPDF_CloseDocument(doc);
        }
        std::fs::write(format!("{out}/pages.tsv"), tsv).unwrap();
    }

    #[test]
    fn a_garbled_text_layer_is_read_from_its_pixels_instead() {
        // An OCR layer set in zero-height type: boxes that say nothing.
        let text: Vec<Bx> = (0..40)
            .map(|i| Bx::new(60.0 + i as f32, 100.0, 61.0 + i as f32, 100.05))
            .collect();
        let (a, source) = analyze_page(page(text, vec![]), || {
            Some(Box::new(scanned(&[(108, 588), (636, 1116)], 0)))
        });
        assert_eq!(source, "image");
        assert_eq!(a.kind, PageLayoutKind::Columns);
        // A good text layer is read as it is, without rendering anything.
        let (a, source) = analyze_page(
            page(prose(90.0, 522.0, 80.0, 50, 8), vec![]),
            || -> Option<Box<PageRaster>> { panic!("rendered a page with a text layer") },
        );
        assert_eq!((a.kind, source), (PageLayoutKind::Single, "text"));
    }

    #[test]
    fn a_scan_is_read_from_its_pixels() {
        let raster = scanned(&[(108, 588), (636, 1116)], 0);
        let (text, graphics) = boxes_from_raster(&raster);
        let a = analyze(&PageContent {
            page_w: 612.0,
            page_h: 792.0,
            text,
            graphics,
        });
        assert_eq!(a.kind, PageLayoutKind::Columns);
        assert_eq!(a.columns, 2);
    }

    #[test]
    fn a_skewed_scan_is_never_cropped() {
        // Mild skew may still read as columns; strong skew fills the gutter
        // and falls back to the whole text block. Either way every word is
        // inside the reading.
        for skew in [1, 2, 4] {
            let raster = scanned(&[(60, 560), (600, 1090)], skew);
            let (text, graphics) = boxes_from_raster(&raster);
            let a = analyze(&PageContent {
                page_w: 612.0,
                page_h: 792.0,
                text: text.clone(),
                graphics,
            });
            for w in &text {
                assert!(
                    a.segments.iter().any(|s| s.1.x0 <= w.x0 + 0.5
                        && s.1.x1 >= w.x1 - 0.5
                        && s.1.y0 <= w.cy()
                        && s.1.y1 >= w.cy()),
                    "skew {skew}: {w:?} is outside {:?}",
                    a.segments
                );
            }
            if skew == 4 {
                assert_ne!(a.kind, PageLayoutKind::Columns);
            }
        }
    }
}

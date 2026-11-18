//! Finding the blanks a printed form *draws* rather than declares.
//!
//! A fillable PDF names its fields; a printed one only draws them: a rule to
//! write on, a box to write in, a row of little boxes for one digit each.
//! This module reads those drawings back as fill targets for the Fill tool,
//! the way `bubbles.rs` reads drawn bubbles out of the character stream.
//!
//! Two passes over the page. [`collect`] is the thin FFI half: every path
//! object (inside form XObjects too) flattened into horizontal and vertical
//! rules, plus the characters with their boxes and sizes and the bounds of
//! anything else drawn (curves, images, dark fills). [`detect`] is the pure
//! half that decides, tested on synthetic pages:
//!
//! - underline blanks: a rule with nothing sitting on it, or the free part of
//!   one beside a label; runs of `_` glyphs count as rules;
//! - empty boxes, and small square ones as checkboxes;
//! - table cells: closed cells with no text, or the empty part of a cell
//!   holding only a label (the IRS layout: label at the top, room below);
//! - comb rows: equal small boxes side by side, or a rule divided by ticks.
//!
//! Every rect is in display-normalized page space, y-up, the space text runs
//! and form widgets use. Nothing here writes: a target only says where the
//! Fill tool should put an ordinary text box.

use super::layout_text::char_matrix;
use super::render::{apply, compose, Matrix, FPDF_PAGEOBJ_FORM, IDENTITY, MAX_FORM_DEPTH};
use super::text::DispMapper;
use super::types::{BlankDto, RectDto};
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE, FPDF_PAGEOBJECT, FS_MATRIX};
use std::os::raw::{c_int, c_uint};

/// `FPDF_PAGEOBJ_PATH` / `FPDF_PAGEOBJ_IMAGE`.
const PAGEOBJ_PATH: c_int = 2;
const PAGEOBJ_IMAGE: c_int = 3;
/// `FPDF_SEGMENT_*`.
const SEGMENT_LINETO: c_int = 0;
const SEGMENT_BEZIERTO: c_int = 1;
const SEGMENT_MOVETO: c_int = 2;
/// Backstops for pages drawn as a plot: objects and path segments examined.
const MAX_OBJECTS: usize = 50_000;
const MAX_SEGMENTS: usize = 400_000;

/// A filled bar thinner than this is a rule, not a box.
const RULE_MAX: f32 = 2.5;
/// Coordinates closer than this are the same edge.
const SNAP: f32 = 1.6;
/// Smallest side of a box or cell worth writing in.
const MIN_CELL: f32 = 5.5;
/// Shortest rule offered as an underline blank.
const MIN_UNDERLINE: f32 = 14.0;
/// Range a suggested font size is clamped to.
const MIN_FONT: f32 = 8.0;
const MAX_FONT: f32 = 12.0;
/// Size assumed when a page has no text to learn from.
const FALLBACK_FONT: f32 = 10.0;

/// A horizontal rule from `x0` to `x1` at height `y` (y-up), `t` thick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HLine {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub t: f32,
}

/// A vertical rule from `y0` up to `y1` at `x`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VLine {
    pub x: f32,
    pub y0: f32,
    pub y1: f32,
    pub t: f32,
}

/// An axis-aligned box, y-up: `y0` is the bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bounds {
    fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    fn area(&self) -> f32 {
        self.w().max(0.0) * self.h().max(0.0)
    }
    fn cx(&self) -> f32 {
        (self.x0 + self.x1) / 2.0
    }
    fn cy(&self) -> f32 {
        (self.y0 + self.y1) / 2.0
    }
    fn intersection(&self, o: &Bounds) -> f32 {
        let w = self.x1.min(o.x1) - self.x0.max(o.x0);
        let h = self.y1.min(o.y1) - self.y0.max(o.y0);
        if w <= 0.0 || h <= 0.0 {
            0.0
        } else {
            w * h
        }
    }
    fn contains_point(&self, x: f32, y: f32) -> bool {
        x > self.x0 && x < self.x1 && y > self.y0 && y < self.y1
    }
    fn rect(&self) -> RectDto {
        RectDto {
            x: self.x0,
            y: self.y0,
            w: self.w(),
            h: self.h(),
        }
    }
}

/// One character: its tight box (y-up, `y` the bottom) and nominal size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub b: Bounds,
    pub size: f32,
}

/// Everything detection reads from one page.
#[derive(Default, Debug)]
pub struct PageInput {
    pub width: f32,
    pub height: f32,
    pub hlines: Vec<HLine>,
    pub vlines: Vec<VLine>,
    pub glyphs: Vec<Glyph>,
    /// Anything else drawn: curves, images, diagonal strokes, dark fills.
    /// A box with any of this inside is not empty.
    pub ink: Vec<Bounds>,
    /// Small closed round outlines: drawn radio buttons.
    pub rings: Vec<Bounds>,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Kinds, most specific first: on overlap the earlier one wins.
const KINDS: [&str; 5] = ["comb", "check", "box", "cell", "line"];

#[derive(Clone, Debug)]
struct Target {
    b: Bounds,
    kind: &'static str,
    font: f32,
    cells: u32,
}

/// Every fill target on a page, in reading order.
pub fn detect(input: &PageInput) -> Vec<BlankDto> {
    let mut hlines = input.hlines.clone();
    let mut text: Vec<Glyph> = Vec::with_capacity(input.glyphs.len());
    let mut targets: Vec<Target> = Vec::new();

    // Underscores are rules typed as text; ballot boxes are checkboxes.
    let mut run: Vec<&Glyph> = Vec::new();
    let flush = |run: &mut Vec<&Glyph>, hlines: &mut Vec<HLine>| {
        let wide: f32 = run.iter().map(|g| g.b.w()).sum();
        if run.len() >= 2 || wide >= MIN_UNDERLINE {
            let x0 = run.iter().map(|g| g.b.x0).fold(f32::MAX, f32::min);
            let x1 = run.iter().map(|g| g.b.x1).fold(f32::MIN, f32::max);
            let y = run.iter().map(|g| g.b.y0).fold(f32::MAX, f32::min);
            hlines.push(HLine { x0, x1, y, t: 0.6 });
        }
        run.clear();
    };
    for g in &input.glyphs {
        if g.ch == '_' && g.b.w() > 0.0 {
            if let Some(last) = run.last() {
                let joined = (g.b.y0 - last.b.y0).abs() < 1.0 && g.b.x0 - last.b.x1 < 2.0;
                if !joined {
                    flush(&mut run, &mut hlines);
                }
            }
            run.push(g);
            continue;
        }
        if g.ch.is_whitespace() || g.b.w() <= 0.0 || g.b.h() <= 0.0 {
            continue;
        }
        if matches!(
            g.ch,
            '\u{2610}' | '\u{25A1}' | '\u{25FB}' | '\u{274F}' | '\u{2751}'
        ) && g.b.w() >= 5.0
            && g.b.w() <= 24.0
        {
            let font = clamp_font(g.size, g.b.h());
            targets.push(Target {
                b: g.b,
                kind: "check",
                font,
                cells: 0,
            });
            continue;
        }
        text.push(*g);
    }
    flush(&mut run, &mut hlines);

    let hlines = merge_h(hlines);
    let vlines = merge_v(input.vlines.clone());
    let body_font = median_size(&text);
    let page = Bounds {
        x0: 0.0,
        y0: 0.0,
        x1: input.width,
        y1: input.height,
    };

    for ring in &input.rings {
        if !text.iter().any(|g| ring.contains_point(g.b.cx(), g.b.cy())) {
            targets.push(Target {
                b: *ring,
                kind: "check",
                font: body_font,
                cells: 0,
            });
        }
    }

    // Cells and boxes.
    // A cell divided by rules running all the way across it is a stack of
    // cells (an address box: street, then city, state and ZIP).
    let cells: Vec<Bounds> = find_cells(&hlines, &vlines)
        .into_iter()
        .flat_map(|c| {
            let mut cuts: Vec<f32> = hlines
                .iter()
                .filter(|l| {
                    l.y > c.y0 + MIN_CELL
                        && l.y < c.y1 - MIN_CELL
                        && l.x0 <= c.x0 + 0.06 * c.w()
                        && l.x1 >= c.x1 - 0.06 * c.w()
                })
                .map(|l| l.y)
                .collect();
            cuts.sort_by(f32::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() < SNAP);
            let mut bands = Vec::with_capacity(cuts.len() + 1);
            let mut floor = c.y0;
            for y in cuts.into_iter().chain(std::iter::once(c.y1)) {
                bands.push(Bounds {
                    y0: floor,
                    y1: y,
                    ..c
                });
                floor = y;
            }
            bands
        })
        .collect();
    let mut empty: Vec<Bounds> = Vec::new();
    let mut labelled: Vec<(Bounds, Vec<&Glyph>)> = Vec::new();
    for c in &cells {
        if c.h() > 0.45 * page.h() || (c.w() > 0.85 * page.w() && c.h() > 0.2 * page.h()) {
            continue;
        }
        if input.ink.iter().any(|i| c.contains_point(i.cx(), i.cy())) {
            continue;
        }
        // A cell ruled inside (lines for handwriting) leaves those lines to
        // speak for themselves.
        let ruled = hlines.iter().any(|l| {
            l.y > c.y0 + SNAP && l.y < c.y1 - SNAP && l.x0 < c.x0 + 0.5 * c.w() && l.x1 > c.cx()
        });
        if ruled {
            continue;
        }
        let inner: Vec<&Glyph> = text
            .iter()
            .filter(|g| c.contains_point(g.b.cx(), g.b.cy()))
            .collect();
        if inner.is_empty() {
            empty.push(*c);
        } else {
            labelled.push((*c, inner));
        }
    }
    for (c, inner) in &labelled {
        // A data table, not a form: a neighbour in the row is written down
        // to its bottom, or the column holds entries above or below.
        let low = |g: &[&Glyph]| g.iter().map(|g| g.b.y0).fold(f32::MAX, f32::min);
        let full_row = labelled.iter().any(|(o, og)| {
            o != c
                && (o.y0 - c.y0).abs() < SNAP
                && (o.y1 - c.y1).abs() < SNAP
                && low(og) - o.y0 < 6.0
        });
        let column = cells.iter().any(|o| {
            o != c
                && (o.x0 - c.x0).abs() < SNAP
                && (o.x1 - c.x1).abs() < SNAP
                && ((o.y1 - c.y0).abs() < SNAP || (o.y0 - c.y1).abs() < SNAP)
        });
        let colon = inner
            .iter()
            .max_by(|a, b| a.b.x1.total_cmp(&b.b.x1))
            .is_some_and(|g| g.ch == ':');
        // Printed separators standing alone - the point of a money cell,
        // the slashes of a date - divide the room, they do not fill it.
        let (mut seps, mut label): (Vec<&Glyph>, Vec<&Glyph>) = inner
            .iter()
            .copied()
            .partition(|g| is_lone_separator(g, inner));
        // Separators close together in a row are a dot leader, which is
        // text; a date's slashes stand a field apart.
        if seps.len() > 3 {
            let mut xs: Vec<f32> = seps.iter().map(|g| g.b.cx()).collect();
            xs.sort_by(f32::total_cmp);
            let mut gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
            gaps.sort_by(f32::total_cmp);
            if gaps[gaps.len() / 2] < 16.0 {
                label.append(&mut seps);
            }
        }
        let found = if label.is_empty() {
            vec![Target {
                b: Bounds {
                    x0: c.x0 + 0.5,
                    y0: c.y0 + 0.5,
                    x1: c.x1 - 0.5,
                    y1: c.y1 - 0.5,
                },
                kind: "box",
                font: clamp_font(body_font, c.h()),
                cells: 0,
            }]
        } else {
            cell_remainder(c, &label, !full_row, colon || !column)
        };
        for t in found {
            targets.extend(split_at(t, &seps));
        }
    }

    // Combs: equal small empty cells side by side.
    let (combs, singles) = comb_rows(&empty);
    for (b, n) in combs {
        let font = clamp_font(nearby_size(&text, &b).unwrap_or(body_font), b.h());
        targets.push(Target {
            b,
            kind: "comb",
            font,
            cells: n,
        });
    }
    for b in singles {
        let ratio = b.w() / b.h();
        let check = (6.0..=22.0).contains(&b.w())
            && (6.0..=22.0).contains(&b.h())
            && ratio > 0.6
            && ratio < 1.6;
        // A sliver between doubled rules is not a box to write in.
        if !check && (b.h() < 9.0 || b.w() < 12.0) {
            continue;
        }
        let font = clamp_font(nearby_size(&text, &b).unwrap_or(body_font), b.h());
        targets.push(Target {
            b,
            kind: if check { "check" } else { "box" },
            font,
            cells: 0,
        });
    }

    // Rules divided by ticks, then plain underlines.
    targets.extend(tick_combs(&hlines, &vlines, &text, body_font));
    for l in &hlines {
        targets.extend(underline_targets(
            l, &hlines, &vlines, &text, &input.ink, body_font, &page,
        ));
    }

    let mut kept = dedupe(targets);
    reading_order(&mut kept);
    kept.into_iter()
        .map(|t| BlankDto {
            rect: t.b.rect(),
            kind: t.kind.to_string(),
            font_size_pt: (t.font * 2.0).round() / 2.0,
            cells: t.cells,
        })
        .collect()
}

/// Merge collinear horizontal pieces (a dashed rule, a rule drawn in parts,
/// the four edges of adjoining boxes) into whole rules.
fn merge_h(mut lines: Vec<HLine>) -> Vec<HLine> {
    lines.retain(|l| l.x1 - l.x0 > 0.5);
    lines.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x0.total_cmp(&b.x0)));
    let mut out: Vec<HLine> = Vec::with_capacity(lines.len());
    // Bucket by y so merging is a sweep along each bucket.
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j].y - lines[j - 1].y <= 0.8 {
            j += 1;
        }
        let mut row: Vec<HLine> = lines[i..j].to_vec();
        row.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        let mut cur = row[0];
        for l in &row[1..] {
            if l.x0 <= cur.x1 + 1.2 {
                cur.x1 = cur.x1.max(l.x1);
                cur.t = cur.t.max(l.t);
            } else {
                out.push(cur);
                cur = *l;
            }
        }
        out.push(cur);
        i = j;
    }
    out
}

fn merge_v(mut lines: Vec<VLine>) -> Vec<VLine> {
    lines.retain(|l| l.y1 - l.y0 > 0.5);
    lines.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y0.total_cmp(&b.y0)));
    let mut out: Vec<VLine> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j].x - lines[j - 1].x <= 0.8 {
            j += 1;
        }
        let mut col: Vec<VLine> = lines[i..j].to_vec();
        col.sort_by(|a, b| a.y0.total_cmp(&b.y0));
        let mut cur = col[0];
        for l in &col[1..] {
            if l.y0 <= cur.y1 + 1.2 {
                cur.y1 = cur.y1.max(l.y1);
                cur.t = cur.t.max(l.t);
            } else {
                out.push(cur);
                cur = *l;
            }
        }
        out.push(cur);
        i = j;
    }
    out
}

/// The smallest cells the rules enclose.
///
/// For each rule taken as a top edge, the verticals hanging from it split it
/// into spans; each span is closed by the nearest rule below that crosses it
/// and that both of its verticals reach. A rule's own end counts as an open
/// side when the rule below ends there too - IRS forms leave the page margin
/// unruled, so their first column has no left edge at all.
fn find_cells(hlines: &[HLine], vlines: &[VLine]) -> Vec<Bounds> {
    let mut by_y: Vec<&HLine> = hlines.iter().filter(|l| l.x1 - l.x0 >= MIN_CELL).collect();
    by_y.sort_by(|a, b| b.y.total_cmp(&a.y));
    let mut out: Vec<Bounds> = Vec::new();
    for (ti, top) in by_y.iter().enumerate() {
        // Verticals that cross this rule and go on down from it.
        let mut xs: Vec<(f32, Option<&VLine>)> = vlines
            .iter()
            .filter(|v| {
                v.x >= top.x0 - SNAP
                    && v.x <= top.x1 + SNAP
                    && v.y1 >= top.y - SNAP
                    && v.y0 <= top.y - MIN_CELL
            })
            .map(|v| (v.x, Some(v)))
            .collect();
        xs.push((top.x0, None));
        xs.push((top.x1, None));
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Collapse an open end onto a vertical standing at the same place.
        let mut edges: Vec<(f32, Option<&VLine>)> = Vec::with_capacity(xs.len());
        for e in xs {
            match edges.last_mut() {
                Some(last) if (e.0 - last.0).abs() <= SNAP => {
                    if last.1.is_none() {
                        *last = e;
                    }
                }
                _ => edges.push(e),
            }
        }
        for pair in edges.windows(2) {
            let (left, right) = (pair[0], pair[1]);
            if right.0 - left.0 < MIN_CELL {
                continue;
            }
            for bot in by_y[ti + 1..].iter() {
                if bot.y > top.y - MIN_CELL {
                    continue;
                }
                let spans = bot.x0 <= left.0 + SNAP && bot.x1 >= right.0 - SNAP;
                if !spans {
                    // A rule part-way across is a line to write on inside
                    // the cell, not its bottom; stop only at one that
                    // blocks the whole span.
                    continue;
                }
                let reaches = |side: (f32, Option<&VLine>), open_end: f32| match side.1 {
                    Some(v) => v.y0 <= bot.y + SNAP,
                    // An open side needs the rule below to end there too.
                    None => (open_end - side.0).abs() <= 3.0 * SNAP,
                };
                let left_ok = reaches(left, bot.x0);
                let right_ok = reaches(right, bot.x1);
                // An open side is a margin, which only a form row of
                // ordinary height has; a tall open "cell" is two section
                // rules with a page of text between them.
                let open = left.1.is_none() || right.1.is_none();
                if left_ok && right_ok && (!open || top.y - bot.y <= 72.0) {
                    out.push(Bounds {
                        x0: left.0,
                        y0: bot.y,
                        x1: right.0,
                        y1: top.y,
                    });
                }
                break;
            }
        }
    }
    out.sort_by(|a, b| {
        a.y0.total_cmp(&b.y0)
            .then(a.x0.total_cmp(&b.x0))
            .then(a.x1.total_cmp(&b.x1))
    });
    out.dedup_by(|a, b| {
        (a.x0 - b.x0).abs() < 0.5
            && (a.x1 - b.x1).abs() < 0.5
            && (a.y0 - b.y0).abs() < 0.5
            && (a.y1 - b.y1).abs() < 0.5
    });
    out
}

/// The room a labelled cell leaves to write in: below its lowest line of
/// text, or to the right of its text, whichever is larger.
fn cell_remainder(
    c: &Bounds,
    inner: &[&Glyph],
    allow_below: bool,
    allow_beside: bool,
) -> Vec<Target> {
    let text_area: f32 = inner.iter().map(|g| g.b.area()).sum();
    if text_area > 0.35 * c.area() {
        return Vec::new();
    }
    let label_font = median_size_of(inner.iter().map(|g| g.size));
    let low = inner.iter().map(|g| g.b.y0).fold(f32::MAX, f32::min);
    let right = inner.iter().map(|g| g.b.x1).fold(f32::MIN, f32::max);
    let below = Bounds {
        x0: c.x0 + 1.0,
        y0: c.y0 + 0.5,
        x1: c.x1 - 1.0,
        y1: low - 1.0,
    };
    let beside = Bounds {
        x0: right + 2.0,
        y0: c.y0 + 0.5,
        x1: c.x1 - 1.0,
        y1: c.y1 - 0.5,
    };
    let below_ok = allow_below && below.h() >= 7.0 && below.w() >= 12.0;
    let beside_ok = allow_beside && beside.w() >= 24.0_f32.max(0.3 * c.w()) && beside.h() >= 7.0;
    let pick = match (below_ok, beside_ok) {
        // The IRS layout - label on top, the rest of the cell to write in -
        // wins whenever it leaves a real line's worth of room.
        (true, true) if below.h() < 0.4 * c.h() && beside.area() > below.area() => beside,
        (true, _) => below,
        (false, true) => beside,
        _ => return Vec::new(),
    };
    let font = clamp_font(label_font.max(MIN_FONT), pick.h());
    let whole = Target {
        b: pick,
        kind: "cell",
        font,
        cells: 0,
    };
    if pick != below {
        return vec![whole];
    }
    // Labels standing apart on the lowest line ("City  State  ZIP code")
    // each head a column of their own.
    let bottom_row: Vec<&&Glyph> = inner
        .iter()
        .filter(|g| g.b.y0 < low + 0.5 * g.size.max(4.0))
        .collect();
    let mut xs: Vec<(f32, f32, f32)> = bottom_row
        .iter()
        .map(|g| (g.b.x0, g.b.x1, g.size.max(4.0)))
        .collect();
    xs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut starts: Vec<f32> = Vec::new();
    let mut end = f32::MIN;
    for (x0, x1, s) in xs {
        if end > f32::MIN && x0 > end + 2.5 * s {
            starts.push(x0 - 2.0);
        }
        end = end.max(x1);
    }
    if starts.is_empty() {
        return vec![whole];
    }
    let mut out = Vec::new();
    let mut at = pick.x0;
    for x in starts.into_iter().chain(std::iter::once(pick.x1)) {
        if x - at >= 12.0 {
            out.push(Target {
                b: Bounds {
                    x0: at,
                    x1: x,
                    ..pick
                },
                ..whole.clone()
            });
        }
        at = x;
    }
    out
}

/// A `.` `,` `/` or `-` with no other character beside it on its line.
fn is_lone_separator(g: &Glyph, all: &[&Glyph]) -> bool {
    matches!(g.ch, '.' | ',' | '/' | '-')
        && g.b.w() < 5.0
        && !all.iter().any(|o| {
            !std::ptr::eq(*o, g)
                && (o.b.cy() - g.b.cy()).abs() < 3.0
                && o.b.x1 > g.b.x0 - 3.0
                && o.b.x0 < g.b.x1 + 3.0
        })
}

/// A target cut into pieces at the separators that stand inside it.
fn split_at(t: Target, seps: &[&Glyph]) -> Vec<Target> {
    let mut cuts: Vec<(f32, f32)> = seps
        .iter()
        .filter(|g| t.b.contains_point(g.b.cx(), g.b.cy()))
        .map(|g| (g.b.x0, g.b.x1))
        .collect();
    if cuts.is_empty() {
        return vec![t];
    }
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut at = t.b.x0;
    for (a, b) in cuts.into_iter().chain(std::iter::once((t.b.x1, t.b.x1))) {
        if a - at >= 8.0 {
            out.push(Target {
                b: Bounds {
                    x0: at,
                    x1: a,
                    ..t.b
                },
                ..t.clone()
            });
        }
        at = at.max(b);
    }
    out
}

/// Runs of three or more equal small boxes side by side become one comb
/// target; everything else stays a box of its own.
fn comb_rows(empty: &[Bounds]) -> (Vec<(Bounds, u32)>, Vec<Bounds>) {
    let mut sorted: Vec<Bounds> = empty.to_vec();
    sorted.sort_by(|a, b| {
        a.y0.total_cmp(&b.y0)
            .then(a.y1.total_cmp(&b.y1))
            .then(a.x0.total_cmp(&b.x0))
    });
    let mut used = vec![false; sorted.len()];
    let mut combs = Vec::new();
    for i in 0..sorted.len() {
        if used[i] {
            continue;
        }
        let first = sorted[i];
        if first.w() > 36.0 || first.h() > 40.0 {
            continue;
        }
        let mut run = vec![i];
        let mut last = first;
        // Boxes touch, or stand apart by one steady gap (the IRS draws its
        // EIN boxes a few points apart).
        let mut pitch_gap: Option<f32> = None;
        for (j, b) in sorted.iter().enumerate().skip(i + 1) {
            if used[j] {
                continue;
            }
            let same_row = (b.y0 - first.y0).abs() < SNAP && (b.y1 - first.y1).abs() < SNAP;
            if !same_row {
                continue;
            }
            let gap = b.x0 - last.x1;
            let steady = match pitch_gap {
                None => gap > -SNAP && gap <= 0.6 * first.w(),
                Some(g) => (gap - g).abs() < SNAP,
            };
            let similar = b.w() <= first.w() * 1.4 && b.w() >= first.w() / 1.4;
            if steady && similar {
                pitch_gap.get_or_insert(gap.max(0.0));
                run.push(j);
                last = *b;
            } else if gap > SNAP {
                break;
            }
        }
        if run.len() >= 3 {
            for &k in &run {
                used[k] = true;
            }
            combs.push((
                Bounds {
                    x0: first.x0,
                    y0: first.y0,
                    x1: last.x1,
                    y1: first.y1,
                },
                run.len() as u32,
            ));
        }
    }
    let singles = sorted
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(b, _)| *b)
        .collect();
    (combs, singles)
}

/// A rule with short ticks rising from it at a steady pitch: one character a
/// gap, as on the "SSN" and date lines of many printed forms.
fn tick_combs(hlines: &[HLine], vlines: &[VLine], text: &[Glyph], body: f32) -> Vec<Target> {
    let mut out = Vec::new();
    for l in hlines {
        if l.x1 - l.x0 < 3.0 * MIN_CELL {
            continue;
        }
        let mut ticks: Vec<f32> = vlines
            .iter()
            .filter(|v| {
                (v.y0 - l.y).abs() <= SNAP
                    && v.y1 - v.y0 <= 14.0
                    && v.y1 - v.y0 >= 2.0
                    && v.x >= l.x0 - SNAP
                    && v.x <= l.x1 + SNAP
            })
            .map(|v| v.x)
            .collect();
        if ticks.len() < 4 {
            continue;
        }
        // An axis: numbers under its ticks.
        if columns_below(text, l.y, l.x0, l.x1) >= 2 {
            continue;
        }
        ticks.sort_by(f32::total_cmp);
        let gaps: Vec<f32> = ticks.windows(2).map(|w| w[1] - w[0]).collect();
        let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
        // Character cells, not the dividers of a row of fields.
        if !(5.0..=30.0).contains(&mean) || gaps.iter().any(|g| (g - mean).abs() > 0.3 * mean) {
            continue;
        }
        let tall = vlines
            .iter()
            .filter(|v| (v.y0 - l.y).abs() <= SNAP && ticks.contains(&v.x))
            .map(|v| v.y1 - v.y0)
            .fold(0.0_f32, f32::max);
        let b = Bounds {
            x0: ticks[0],
            y0: l.y,
            x1: *ticks.last().unwrap_or(&l.x1),
            y1: l.y + tall.max(mean.min(16.0)),
        };
        let font = clamp_font(nearby_size(text, &b).unwrap_or(body), b.h().max(10.0));
        out.push(Target {
            b,
            kind: "comb",
            font,
            cells: gaps.len() as u32,
        });
    }
    out
}

/// The writable stretches of one rule.
fn underline_targets(
    l: &HLine,
    hlines: &[HLine],
    vlines: &[VLine],
    text: &[Glyph],
    ink: &[Bounds],
    body: f32,
    page: &Bounds,
) -> Vec<Target> {
    let len = l.x1 - l.x0;
    if len < MIN_UNDERLINE || l.t > 3.0 {
        return Vec::new();
    }
    // A rule across the whole page in its top or bottom margin frames the
    // page (the IRS footer rule over "Cat. No."); it is not a blank.
    let margin = 0.08 * page.h();
    if len > 0.8 * page.w() && (l.y < margin || l.y > page.h() - margin) {
        return Vec::new();
    }
    // Text standing on the rule: its bottom at the rule, give or take a
    // descender, and above it rather than hanging below.
    let on: Vec<&Glyph> = text
        .iter()
        .filter(|g| {
            let s = g.size.max(g.b.h());
            g.b.x1 > l.x0 && g.b.x0 < l.x1 && g.b.y0 >= l.y - 0.35 * s && g.b.y0 <= l.y + 0.45 * s
        })
        .collect();
    let mut covered: Vec<(f32, f32)> = on
        .iter()
        .map(|g| {
            let pad = 0.3 * g.size.max(4.0);
            ((g.b.x0 - pad).max(l.x0), (g.b.x1 + pad).min(l.x1))
        })
        .collect();
    covered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (a, b) in covered {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let covered_len: f32 = merged.iter().map(|(a, b)| b - a).sum();
    // Entries in columns standing on the rule: the last rule of a table.
    let runs = merged.windows(2).filter(|w| w[1].0 - w[0].1 > 12.0).count()
        + usize::from(!merged.is_empty());
    let starts_at_text = merged.first().is_some_and(|f| f.0 <= l.x0 + 12.0);
    if runs >= 3 || (runs == 2 && starts_at_text) {
        return Vec::new();
    }
    if covered_len > 0.5 * len {
        // Underlined text, or a table rule under a row of entries.
        return Vec::new();
    }
    // A long rule with a heading over its start is a section rule.
    let heading = text.iter().any(|g| {
        let s = g.size.max(g.b.h());
        g.b.x0 <= l.x0 + 0.5 * s
            && g.b.x1 > l.x0
            && g.b.y0 >= l.y - 0.35 * s
            && g.b.y0 <= l.y + 1.3 * s
    });
    if heading && len > 0.45 * page.w() {
        return Vec::new();
    }
    // The top edge of a box or table: verticals hang down from it and none
    // stand on it.
    let down = vlines
        .iter()
        .filter(|v| {
            (v.y1 - l.y).abs() <= SNAP
                && v.y0 < l.y - 3.0
                && v.x >= l.x0 - SNAP
                && v.x <= l.x1 + SNAP
        })
        .count();
    let up = vlines
        .iter()
        .filter(|v| {
            (v.y0 - l.y).abs() <= SNAP
                && v.y1 > l.y + 3.0
                && v.x >= l.x0 - SNAP
                && v.x <= l.x1 + SNAP
        })
        .count();
    if down > 0 && up == 0 {
        return Vec::new();
    }
    // The bottom edge of a box: closed at both ends by verticals standing on
    // it. The box is found as a cell; its floor is not also a line.
    let stands = |x: f32| {
        vlines.iter().any(|v| {
            (v.y0 - l.y).abs() <= SNAP && v.y1 > l.y + MIN_CELL && (v.x - x).abs() <= 2.0 * SNAP
        })
    };
    if stands(l.x0) && stands(l.x1) {
        return Vec::new();
    }

    let mut free: Vec<(f32, f32)> = Vec::new();
    let mut at = l.x0;
    for (a, b) in &merged {
        if *a > at {
            free.push((at, *a));
        }
        at = at.max(*b);
    }
    if at < l.x1 {
        free.push((at, l.x1));
    }

    let mut out = Vec::new();
    for (a, b) in free {
        if b - a < MIN_UNDERLINE {
            continue;
        }
        let probe = Bounds {
            x0: a,
            y0: l.y,
            x1: b,
            y1: l.y + 20.0,
        };
        let font = clamp_font(nearby_size(text, &probe).unwrap_or(body), 40.0);
        // Room above: up to the nearest thing drawn over the stretch.
        let mut ceiling = l.y + (font * 1.6).max(11.0);
        for g in text {
            if g.b.x1 > a && g.b.x0 < b && g.b.y0 > l.y + 0.5 {
                ceiling = ceiling.min(g.b.y0 - 0.5);
            }
        }
        for o in hlines {
            if o.y > l.y + 0.5 && o.x1 > a + 1.0 && o.x0 < b - 1.0 {
                ceiling = ceiling.min(o.y);
            }
        }
        for i in ink {
            if i.x1 > a && i.x0 < b && i.y0 > l.y + 0.5 {
                ceiling = ceiling.min(i.y0);
            }
        }
        // A rule between a heading row and a row of entries in columns is
        // a table's; one caption under it ("Signature of applicant") or a
        // line of small print with nothing over the stretch is not.
        let text_over = text.iter().any(|g| {
            let s = g.size.max(g.b.h());
            g.b.x1 > a && g.b.x0 < b && g.b.y0 > l.y && g.b.y0 <= l.y + 1.5 * s
        });
        if text_over && columns_below(text, l.y, a, b) >= 2 {
            continue;
        }
        let room = ceiling - l.y;
        if room < 8.0_f32.max(0.75 * font) {
            continue;
        }
        let b = Bounds {
            x0: a,
            y0: l.y,
            x1: b,
            y1: ceiling,
        };
        out.push(Target {
            b,
            kind: "line",
            font: clamp_font(font, room / 1.1),
            cells: 0,
        });
    }
    out
}

/// How many separate runs of text sit just under a stretch of rule.
fn columns_below(text: &[Glyph], y: f32, a: f32, b: f32) -> usize {
    let mut spans: Vec<(f32, f32, f32)> = text
        .iter()
        .filter(|g| {
            let s = g.size.max(g.b.h());
            g.b.x1 > a && g.b.x0 < b && g.b.y1 <= y - 0.3 && g.b.y1 >= y - 1.6 * s
        })
        .map(|g| (g.b.x0, g.b.x1, g.size.max(4.0)))
        .collect();
    if spans.is_empty() {
        return 0;
    }
    spans.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut runs = 1;
    let mut end = spans[0].1;
    for (x0, x1, s) in &spans[1..] {
        if *x0 > end + 2.0 * s {
            runs += 1;
        }
        end = end.max(*x1);
    }
    runs
}

/// The size of the text nearest to a target on its own line: the label
/// beside it, which is what the writing should match.
fn nearby_size(text: &[Glyph], b: &Bounds) -> Option<f32> {
    let mut best: Option<(f32, f32)> = None;
    for g in text {
        let cy = g.b.cy();
        if cy < b.y0 - 3.0 || cy > b.y1 + 3.0 {
            continue;
        }
        let dx = if g.b.x1 <= b.x0 {
            b.x0 - g.b.x1
        } else if g.b.x0 >= b.x1 {
            g.b.x0 - b.x1
        } else {
            0.0
        };
        if dx > 160.0 {
            continue;
        }
        if best.is_none_or(|(d, _)| dx < d) {
            best = Some((dx, g.size));
        }
    }
    best.map(|(_, s)| s).filter(|s| *s > 0.0)
}

fn median_size_of(sizes: impl Iterator<Item = f32>) -> f32 {
    let mut v: Vec<f32> = sizes.filter(|s| *s > 0.0).collect();
    if v.is_empty() {
        return FALLBACK_FONT;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

fn median_size(text: &[Glyph]) -> f32 {
    median_size_of(text.iter().map(|g| g.size))
}

/// A writing size from a label size, kept readable and inside the box.
fn clamp_font(size: f32, room: f32) -> f32 {
    let size = if size.is_finite() && size > 0.0 {
        size
    } else {
        FALLBACK_FONT
    };
    let fit = (room - 2.0) / 1.15;
    size.clamp(MIN_FONT, MAX_FONT).min(fit.max(5.0))
}

/// Overlapping targets: keep the more specific kind, then the larger.
fn dedupe(mut targets: Vec<Target>) -> Vec<Target> {
    let rank = |k: &str| KINDS.iter().position(|x| *x == k).unwrap_or(KINDS.len());
    targets.sort_by(|a, b| {
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then(b.b.area().total_cmp(&a.b.area()))
    });
    let mut kept: Vec<Target> = Vec::with_capacity(targets.len());
    for t in targets {
        if t.b.w() <= 0.0 || t.b.h() <= 0.0 {
            continue;
        }
        let clash = kept.iter().any(|k| {
            let i = k.b.intersection(&t.b);
            i > 0.5 * k.b.area().min(t.b.area())
        });
        if !clash {
            kept.push(t);
        }
    }
    kept
}

/// Top to bottom by rows, left to right within a row.
fn reading_order(targets: &mut [Target]) {
    targets.sort_by(|a, b| b.b.y1.total_cmp(&a.b.y1));
    let mut row_of = vec![0usize; targets.len()];
    let mut row = 0;
    let mut floor = f32::MAX;
    for (i, t) in targets.iter().enumerate() {
        if i > 0 && t.b.cy() < floor {
            row += 1;
            floor = f32::MAX;
        }
        floor = floor.min(t.b.y0.max(t.b.cy() - 2.0));
        row_of[i] = row;
    }
    let mut keyed: Vec<(usize, Target)> = row_of.into_iter().zip(targets.iter().cloned()).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.b.x0.total_cmp(&b.1.b.x0)));
    for (slot, (_, t)) in targets.iter_mut().zip(keyed) {
        *slot = t;
    }
}

/// Drop targets an AcroForm widget already covers: the form's own field is
/// the better place to type.
pub fn drop_covered(blanks: Vec<BlankDto>, widgets: &[RectDto]) -> Vec<BlankDto> {
    if widgets.is_empty() {
        return blanks;
    }
    let bounds = |r: &RectDto| Bounds {
        x0: r.x,
        y0: r.y,
        x1: r.x + r.w,
        y1: r.y + r.h,
    };
    blanks
        .into_iter()
        .filter(|blank| {
            let b = bounds(&blank.rect);
            !widgets
                .iter()
                .any(|w| bounds(w).intersection(&b) > 0.3 * b.area().min(bounds(w).area()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Collection (FFI)
// ---------------------------------------------------------------------------

/// One subpath in display space.
struct Sub {
    points: Vec<(f32, f32)>,
    curved: bool,
    closed: bool,
}

/// Read a page's characters, rules and other ink for [`detect`].
pub fn collect(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    disp_w: f32,
    disp_h: f32,
) -> PageInput {
    let mapper = DispMapper::new(b, page, disp_w, disp_h);
    let mut input = PageInput {
        width: disp_w,
        height: disp_h,
        ..PageInput::default()
    };

    let tp = b.FPDFText_LoadPage(page);
    if !tp.is_null() {
        let n = b.FPDFText_CountChars(tp).max(0);
        input.glyphs.reserve(n as usize);
        for i in 0..n {
            let ch = char::from_u32(b.FPDFText_GetUnicode(tp, i)).unwrap_or(' ');
            if ch.is_whitespace() || ch.is_control() {
                continue;
            }
            let (mut l, mut r, mut bo, mut t) = (0f64, 0f64, 0f64, 0f64);
            if b.FPDFText_GetCharBox(tp, i, &mut l, &mut r, &mut bo, &mut t) == 0 {
                continue;
            }
            let (x, y, w, h) = mapper.map_box(l, r, bo, t);
            if w <= 0.0 && h <= 0.0 {
                continue;
            }
            input.glyphs.push(Glyph {
                ch,
                b: Bounds {
                    x0: x,
                    y0: y,
                    x1: x + w,
                    y1: y + h,
                },
                // The nominal size times the text matrix: forms set text at
                // 1 pt and scale it, which PDFium reports as 1.
                size: {
                    let scale =
                        char_matrix(b, tp, i).map_or(1.0, |m| (m.c * m.c + m.d * m.d).sqrt());
                    let size = b.FPDFText_GetFontSize(tp, i) as f32 * scale;
                    if size.is_finite() && size > 0.5 {
                        size
                    } else {
                        h / 0.7
                    }
                },
            });
        }
        b.FPDFText_ClosePage(tp);
    }

    let mut stack: Vec<(FPDF_PAGEOBJECT, Matrix, u8)> = Vec::new();
    let count = b.FPDFPage_CountObjects(page).clamp(0, MAX_OBJECTS as c_int);
    for index in (0..count).rev() {
        stack.push((b.FPDFPage_GetObject(page, index), IDENTITY, 0));
    }
    let (mut seen, mut segments) = (0usize, 0usize);
    while let Some((object, to_page, depth)) = stack.pop() {
        seen += 1;
        if object.is_null() || seen > MAX_OBJECTS {
            continue;
        }
        let mut m = FS_MATRIX {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        let own = if b.FPDFPageObj_GetMatrix(object, &mut m) != 0 {
            [m.a, m.b, m.c, m.d, m.e, m.f]
        } else {
            IDENTITY
        };
        match b.FPDFPageObj_GetType(object) {
            PAGEOBJ_PATH if segments < MAX_SEGMENTS => {
                let matrix = compose(&to_page, &own);
                segments += read_path(b, object, &matrix, &mapper, &mut input);
            }
            PAGEOBJ_IMAGE => {
                let (mut l, mut bo, mut r, mut t) = (0f32, 0f32, 0f32, 0f32);
                if b.FPDFPageObj_GetBounds(object, &mut l, &mut bo, &mut r, &mut t) != 0 {
                    let a = mapper_point(&mapper, &to_page, l, bo);
                    let c = mapper_point(&mapper, &to_page, r, t);
                    input.ink.push(bounds_of(&[a, c]));
                }
            }
            FPDF_PAGEOBJ_FORM if depth < MAX_FORM_DEPTH => {
                let child_to_page = compose(&to_page, &own);
                let children = b.FPDFFormObj_CountObjects(object).max(0);
                for i in (0..children).rev() {
                    stack.push((
                        b.FPDFFormObj_GetObject(object, i as std::os::raw::c_ulong),
                        child_to_page,
                        depth + 1,
                    ));
                }
            }
            _ => {}
        }
    }
    input
}

fn mapper_point(mapper: &DispMapper, m: &Matrix, x: f32, y: f32) -> (f32, f32) {
    let (px, py) = apply(m, x, y);
    mapper.map(px as f64, py as f64)
}

fn bounds_of(points: &[(f32, f32)]) -> Bounds {
    let mut b = Bounds {
        x0: f32::MAX,
        y0: f32::MAX,
        x1: f32::MIN,
        y1: f32::MIN,
    };
    for &(x, y) in points {
        b.x0 = b.x0.min(x);
        b.y0 = b.y0.min(y);
        b.x1 = b.x1.max(x);
        b.y1 = b.y1.max(y);
    }
    b
}

/// Luminance of an object's fill or stroke colour, 0 (black) to 1 (white).
fn luma(r: c_uint, g: c_uint, b: c_uint) -> f32 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0
}

/// Read one path object into rules and ink. Returns the segments read.
fn read_path(
    b: &dyn PdfiumLibraryBindings,
    object: FPDF_PAGEOBJECT,
    matrix: &Matrix,
    mapper: &DispMapper,
    input: &mut PageInput,
) -> usize {
    let mut fill_mode: c_int = 0;
    let mut stroke = 0;
    if b.FPDFPath_GetDrawMode(object, &mut fill_mode, &mut stroke) == 0 {
        return 0;
    }
    let stroked = stroke != 0 && {
        let (mut r, mut g, mut bl, mut a) = (0, 0, 0, 0);
        b.FPDFPageObj_GetStrokeColor(object, &mut r, &mut g, &mut bl, &mut a) == 0
            || (a > 0 && luma(r, g, bl) < 0.97)
    };
    let fill_luma = if fill_mode != 0 {
        let (mut r, mut g, mut bl, mut a) = (0, 0, 0, 0);
        if b.FPDFPageObj_GetFillColor(object, &mut r, &mut g, &mut bl, &mut a) == 0 {
            Some(0.0)
        } else if a == 0 {
            None
        } else if r.abs_diff(g).max(g.abs_diff(bl)).max(r.abs_diff(bl)) < 24
            && luma(r, g, bl) < 0.97
        {
            // Neutral grey: the IRS shades what is *not* to be written in,
            // so it reads as ink. Only a tint (pale blue, yellow) marks a
            // box to fill.
            Some(luma(r, g, bl).min(0.5))
        } else {
            Some(luma(r, g, bl))
        }
    } else {
        None
    };
    let filled = fill_luma.is_some_and(|l| l < 0.97);
    if !stroked && !filled {
        return 0;
    }
    let scale = ((matrix[0] * matrix[3] - matrix[1] * matrix[2]).abs()).sqrt();
    let width = if stroked {
        let mut w = 0f32;
        if b.FPDFPageObj_GetStrokeWidth(object, &mut w) == 0 {
            w = 1.0;
        }
        (w * scale).max(0.1)
    } else {
        0.0
    };

    let n = b.FPDFPath_CountSegments(object).max(0);
    let mut subs: Vec<Sub> = Vec::new();
    for i in 0..n {
        let seg = b.FPDFPath_GetPathSegment(object, i);
        if seg.is_null() {
            continue;
        }
        let (mut x, mut y) = (0f32, 0f32);
        if b.FPDFPathSegment_GetPoint(seg, &mut x, &mut y) == 0 {
            continue;
        }
        let p = mapper_point(mapper, matrix, x, y);
        let kind = b.FPDFPathSegment_GetType(seg);
        if kind == SEGMENT_MOVETO || subs.is_empty() {
            subs.push(Sub {
                points: vec![p],
                curved: false,
                closed: false,
            });
        } else if let Some(sub) = subs.last_mut() {
            if kind == SEGMENT_BEZIERTO {
                sub.curved = true;
            } else if kind != SEGMENT_LINETO {
                continue;
            }
            sub.points.push(p);
        }
        if b.FPDFPathSegment_GetClose(seg) != 0 {
            if let Some(sub) = subs.last_mut() {
                sub.closed = true;
            }
        }
    }

    for sub in subs {
        add_subpath(&sub, stroked, width, fill_luma.filter(|_| filled), input);
    }
    n as usize
}

/// Classify one subpath: rules from its straight edges, a ring, or ink.
fn add_subpath(sub: &Sub, stroked: bool, width: f32, fill: Option<f32>, input: &mut PageInput) {
    if sub.points.len() < 2 {
        return;
    }
    let bb = bounds_of(&sub.points);
    if sub.curved {
        let ring = stroked
            && fill.is_none()
            && (5.0..=20.0).contains(&bb.w())
            && (5.0..=20.0).contains(&bb.h())
            && (bb.w() / bb.h() - 1.0).abs() < 0.3;
        if ring {
            input.rings.push(bb);
        } else {
            input.ink.push(bb);
        }
        return;
    }
    if let Some(l) = fill {
        if !stroked {
            let rect = is_axis_rect(&sub.points);
            if rect && bb.h() <= RULE_MAX && bb.w() >= 3.0 * bb.h() {
                input.hlines.push(HLine {
                    x0: bb.x0,
                    x1: bb.x1,
                    y: bb.cy(),
                    t: bb.h(),
                });
            } else if rect && bb.w() <= RULE_MAX && bb.h() >= 3.0 * bb.w() {
                input.vlines.push(VLine {
                    x: bb.cx(),
                    y0: bb.y0,
                    y1: bb.y1,
                    t: bb.w(),
                });
            } else if rect && l >= 0.72 {
                // A light shaded panel: forms tint the boxes to write in.
                push_rect_edges(&bb, input);
            } else if bb.w() > 1.0 && bb.h() > 1.0 {
                input.ink.push(bb);
            }
            return;
        }
        if l < 0.72 && bb.w() > RULE_MAX && bb.h() > RULE_MAX {
            input.ink.push(bb);
            return;
        }
    }
    let mut points = sub.points.clone();
    if sub.closed || fill.is_some() {
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
    }
    for w in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
        if dx < 0.3 && dy < 0.3 {
            continue;
        }
        if dy <= 0.35 && dx > dy {
            input.hlines.push(HLine {
                x0: x0.min(x1),
                x1: x0.max(x1),
                y: (y0 + y1) / 2.0,
                t: width,
            });
        } else if dx <= 0.35 && dy > dx {
            input.vlines.push(VLine {
                x: (x0 + x1) / 2.0,
                y0: y0.min(y1),
                y1: y0.max(y1),
                t: width,
            });
        } else if dx > 2.0 || dy > 2.0 {
            input.ink.push(bounds_of(&[w[0], w[1]]));
        }
    }
}

fn push_rect_edges(b: &Bounds, input: &mut PageInput) {
    for y in [b.y0, b.y1] {
        input.hlines.push(HLine {
            x0: b.x0,
            x1: b.x1,
            y,
            t: 0.5,
        });
    }
    for x in [b.x0, b.x1] {
        input.vlines.push(VLine {
            x,
            y0: b.y0,
            y1: b.y1,
            t: 0.5,
        });
    }
}

/// Whether a closed polyline is an axis-aligned rectangle.
fn is_axis_rect(points: &[(f32, f32)]) -> bool {
    let mut pts: Vec<(f32, f32)> = points.to_vec();
    if pts.len() >= 2 {
        let (a, z) = (pts[0], pts[pts.len() - 1]);
        if (a.0 - z.0).abs() < 0.3 && (a.1 - z.1).abs() < 0.3 {
            pts.pop();
        }
    }
    if pts.len() != 4 {
        return false;
    }
    (0..4).all(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        (a.0 - b.0).abs() < 0.3 || (a.1 - b.1).abs() < 0.3
    })
}

#[cfg(test)]
#[path = "blanks_tests.rs"]
mod tests;

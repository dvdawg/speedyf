//! Classical image analysis of a rendered page, for reading mathematics.
//!
//! A PDF says which glyphs are where, but not how they are drawn together:
//! its boxes are the fonts' ascents and descents, a fraction bar may be a
//! path, an image or a glyph, and nothing says that a numerator belongs to
//! the bar under it. The page's pixels do. This module renders a page to a
//! binary raster and reads its layout back with the textbook tools, each a
//! small pure function over plain data:
//!
//! - [`otsu`] and [`binarize`]: a grey page to ink and paper;
//! - [`xy_cut`]: recursive XY-cut on projection profiles, a page into
//!   columns and the columns into horizontal strips of ink;
//! - [`components`]: 8-connected components by two-pass labelling with
//!   union-find, the tight ink box of every symbol, bar and radical;
//! - [`horizontal_rules`]: run-length analysis, long thin horizontal runs
//!   that are fraction bars or overlines however the PDF drew them;
//! - [`group_by_dilation`]: boxes fused by a small vertical dilation with a
//!   horizontal limit, so a numerator, its bar and its denominator, or an
//!   operator and its limits, come out as one 2-D group;
//! - [`logical_lines`]: strips joined into the lines of a display through
//!   those stacks.
//!
//! Coordinates are raster pixels, top-left origin; [`PageRaster`] converts
//! to and from page points. The FFI is one function, [`render_raster`].

use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_PAGE};

/// Pixels per point the page is analysed at.
pub const ANALYSIS_SCALE: f32 = 2.0;
/// Ceiling on a raster's pixel count; past it the scale is lowered.
const MAX_PIXELS: f32 = 12_000_000.0;

/// A binary raster: `true` is ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Bits {
    pub w: usize,
    pub h: usize,
    pub on: Vec<bool>,
}

impl Bits {
    pub fn new(w: usize, h: usize) -> Self {
        Bits {
            w,
            h,
            on: vec![false; w * h],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.on[y * self.w + x]
    }

    pub fn set(&mut self, x: usize, y: usize, v: bool) {
        self.on[y * self.w + x] = v;
    }

    /// From rows of `#` (ink) and anything else (paper), for tests.
    pub fn from_art(rows: &[&str]) -> Self {
        let h = rows.len();
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut bits = Bits::new(w, h);
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                if c == b'#' {
                    bits.set(x, y, true);
                }
            }
        }
        bits
    }
}

/// A pixel box, `x0..x1` × `y0..y1` (exclusive ends).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PxBox {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl PxBox {
    pub fn new(x0: usize, y0: usize, x1: usize, y1: usize) -> Self {
        PxBox { x0, y0, x1, y1 }
    }
    pub fn w(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }
    pub fn h(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }
    pub fn is_empty(&self) -> bool {
        self.w() == 0 || self.h() == 0
    }
    pub fn union(&self, o: &PxBox) -> PxBox {
        PxBox::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }
    pub fn intersection_area(&self, o: &PxBox) -> usize {
        let w = self.x1.min(o.x1).saturating_sub(self.x0.max(o.x0));
        let h = self.y1.min(o.y1).saturating_sub(self.y0.max(o.y0));
        w * h
    }
    /// Horizontal overlap, in pixels.
    pub fn x_overlap(&self, o: &PxBox) -> usize {
        self.x1.min(o.x1).saturating_sub(self.x0.max(o.x0))
    }
}

/// Otsu's threshold: the grey level that best splits the histogram into
/// two classes (maximal between-class variance). Pixels at or below it are
/// ink. A page with one tone only gets a fixed mid-grey.
pub fn otsu(gray: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for &v in gray {
        hist[v as usize] += 1;
    }
    let total = gray.len() as f64;
    if total == 0.0 {
        return 160;
    }
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut w_b, mut sum_b) = (0.0, 0.0);
    let (mut best, mut best_t) = (-1.0f64, 160u8);
    for (t, &count) in hist.iter().enumerate() {
        w_b += count as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * count as f64;
        let m_b = sum_b / w_b;
        let m_f = (sum_all - sum_b) / w_f;
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    // A blank page has no second class; anti-aliasing must not become ink.
    if best <= 0.0 {
        160
    } else {
        best_t.clamp(64, 200)
    }
}

/// Ink where the grey level is at or below `threshold`.
pub fn binarize(gray: &[u8], w: usize, h: usize, threshold: u8) -> Bits {
    Bits {
        w,
        h,
        on: gray.iter().take(w * h).map(|&v| v <= threshold).collect(),
    }
}

/// Ink per row of `region`.
pub fn row_profile(bits: &Bits, region: PxBox) -> Vec<u32> {
    (region.y0..region.y1)
        .map(|y| (region.x0..region.x1).filter(|&x| bits.get(x, y)).count() as u32)
        .collect()
}

/// Ink per column of `region`.
pub fn col_profile(bits: &Bits, region: PxBox) -> Vec<u32> {
    let mut out = vec![0u32; region.w()];
    for y in region.y0..region.y1 {
        for (i, x) in (region.x0..region.x1).enumerate() {
            if bits.get(x, y) {
                out[i] += 1;
            }
        }
    }
    out
}

/// The tight box of the ink in `region`, if it has any.
pub fn trim(bits: &Bits, region: PxBox) -> Option<PxBox> {
    let rows = row_profile(bits, region);
    let top = rows.iter().position(|&c| c > 0)?;
    let bottom = rows.iter().rposition(|&c| c > 0)?;
    let cols = col_profile(bits, region);
    let left = cols.iter().position(|&c| c > 0)?;
    let right = cols.iter().rposition(|&c| c > 0)?;
    Some(PxBox::new(
        region.x0 + left,
        region.y0 + top,
        region.x0 + right + 1,
        region.y0 + bottom + 1,
    ))
}

/// Runs of zeros strictly inside a profile (ink on both sides): `(start,
/// end)` exclusive, as offsets into the profile.
pub fn valleys(profile: &[u32], min_len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut seen_ink = false;
    for (i, &v) in profile.iter().enumerate() {
        if v == 0 {
            if seen_ink && start.is_none() {
                start = Some(i);
            }
        } else {
            if let Some(s) = start.take() {
                if i - s >= min_len {
                    out.push((s, i));
                }
            }
            seen_ink = true;
        }
    }
    out
}

/// Settings for [`xy_cut`], in pixels.
#[derive(Clone, Copy, Debug)]
pub struct CutOptions {
    /// A vertical valley at least this wide is a column gutter...
    pub min_gutter: usize,
    /// ...in a region at least this tall (a short line's wide space is not).
    pub min_gutter_height: usize,
    /// A horizontal valley at least this tall separates bands, cut first.
    pub band_gap: usize,
    /// Columns are at least this wide: a wide space inside a display line
    /// does not make two columns of it.
    pub min_column_width: usize,
}

/// A horizontal strip of ink found by [`xy_cut`], and the column it is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    pub bbox: PxBox,
    pub column: PxBox,
}

/// Recursive XY-cut. A region is trimmed to its ink; if it is tall enough
/// and a vertical valley as wide as a gutter crosses it, it is cut into
/// columns; otherwise wide horizontal valleys cut it into bands, each cut
/// again; a region with neither is cut at every blank row into strips, the
/// leaves. Each strip carries the column it was found in.
pub fn xy_cut(bits: &Bits, region: PxBox, opts: CutOptions) -> Vec<Strip> {
    let mut out = Vec::new();
    cut(bits, region, region, opts, 0, &mut out);
    out.sort_by_key(|s| (s.column.x0, s.bbox.y0, s.bbox.x0));
    out
}

fn cut(
    bits: &Bits,
    region: PxBox,
    column: PxBox,
    opts: CutOptions,
    depth: usize,
    out: &mut Vec<Strip>,
) {
    let Some(region) = trim(bits, region) else {
        return;
    };
    let column = if depth == 0 { region } else { column };
    if depth < 24 && region.h() >= opts.min_gutter_height {
        let mut cols = valleys(&col_profile(bits, region), opts.min_gutter);
        // Every piece the gutters leave must be as wide as a column.
        let mut edges = vec![0];
        for &(s, e) in &cols {
            edges.push(s);
            edges.push(e);
        }
        edges.push(region.w());
        if edges.chunks(2).any(|p| p[1] - p[0] < opts.min_column_width) {
            cols.clear();
        }
        if !cols.is_empty() {
            let mut x = region.x0;
            for (s, e) in cols.iter().map(|&(s, e)| (region.x0 + s, region.x0 + e)) {
                let piece = PxBox::new(x, region.y0, s, region.y1);
                if let Some(p) = trim(bits, piece) {
                    cut(bits, p, p, opts, depth + 1, out);
                }
                x = e;
            }
            let last = PxBox::new(x, region.y0, region.x1, region.y1);
            if let Some(p) = trim(bits, last) {
                cut(bits, p, p, opts, depth + 1, out);
            }
            return;
        }
    }
    let rows = valleys(&row_profile(bits, region), 1);
    // A gutter crossed by a few full-width lines (a display set across both
    // columns): cut those lines out as bands of their own, so the rest can
    // be cut into columns.
    if depth < 24 && region.h() >= opts.min_gutter_height {
        if let Some(cuts) = crossing_cuts(bits, region, &rows, opts) {
            let mut y = region.y0;
            for c in cuts {
                cut(
                    bits,
                    PxBox::new(region.x0, y, region.x1, c),
                    column,
                    opts,
                    depth + 1,
                    out,
                );
                y = c;
            }
            cut(
                bits,
                PxBox::new(region.x0, y, region.x1, region.y1),
                column,
                opts,
                depth + 1,
                out,
            );
            return;
        }
    }
    let bands: Vec<(usize, usize)> = rows
        .iter()
        .copied()
        .filter(|&(s, e)| e - s >= opts.band_gap)
        .collect();
    let splits = if !bands.is_empty() && depth < 24 {
        bands
    } else {
        // Leaves: every blank row ends a strip.
        let mut y = region.y0;
        for (s, e) in rows.iter().map(|&(s, e)| (region.y0 + s, region.y0 + e)) {
            if let Some(b) = trim(bits, PxBox::new(region.x0, y, region.x1, s)) {
                out.push(Strip { bbox: b, column });
            }
            y = e;
        }
        if let Some(b) = trim(bits, PxBox::new(region.x0, y, region.x1, region.y1)) {
            out.push(Strip { bbox: b, column });
        }
        return;
    };
    let mut y = region.y0;
    for (s, e) in splits.iter().map(|&(s, e)| (region.y0 + s, region.y0 + e)) {
        cut(
            bits,
            PxBox::new(region.x0, y, region.x1, s),
            column,
            opts,
            depth + 1,
            out,
        );
        y = e;
    }
    cut(
        bits,
        PxBox::new(region.x0, y, region.x1, region.y1),
        column,
        opts,
        depth + 1,
        out,
    );
}

/// Where to cut a region so that a nearly clear gutter becomes a clear one:
/// a vertical valley as wide as a gutter, clear in all but a few rows, and
/// the rows crossing it cut away at the blank rows around them. `rows` are
/// the region's blank-row valleys (offsets). None when there is no such
/// gutter, or the crossings cannot be cut out.
fn crossing_cuts(
    bits: &Bits,
    region: PxBox,
    rows: &[(usize, usize)],
    opts: CutOptions,
) -> Option<Vec<usize>> {
    let profile = col_profile(bits, region);
    let ink_rows = row_profile(bits, region).iter().filter(|&&c| c > 0).count();
    let limit = (ink_rows as u32 / 20).max(1);
    // Nearly clear columns, as runs; the clearest wins.
    let mut best: Option<(usize, usize, f32)> = None;
    let mut start = None;
    for (i, &c) in profile.iter().chain(std::iter::once(&u32::MAX)).enumerate() {
        if c <= limit && i < profile.len() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            let wide = i - s >= opts.min_gutter;
            let sides = s >= opts.min_column_width && profile.len() - i >= opts.min_column_width;
            let ink = profile[s..i].iter().sum::<u32>() as f32 / (i - s) as f32;
            if wide && sides && best.is_none_or(|(_, _, b)| ink < b) {
                best = Some((s, i, ink));
            }
        }
    }
    let (gs, ge, _) = best?;
    // Its middle half: the columns' ragged edges may reach into its sides.
    let quarter = (ge - gs) / 4;
    let gutter = PxBox::new(
        region.x0 + gs + quarter,
        region.y0,
        region.x0 + ge - quarter,
        region.y1,
    );
    let crossing: Vec<bool> = row_profile(bits, gutter).iter().map(|&c| c > 0).collect();
    if !crossing.iter().any(|&c| c) {
        return None;
    }
    // Each run of crossing rows, widened to the blank rows around it.
    let mut cuts = Vec::new();
    let mut k = 0;
    while k < crossing.len() {
        if !crossing[k] {
            k += 1;
            continue;
        }
        let a = k;
        while k < crossing.len() && crossing[k] {
            k += 1;
        }
        let b = k;
        let above = rows
            .iter()
            .rev()
            .find(|&&(_, e)| e <= a)
            .map(|&(s, e)| (s + e) / 2);
        let below = rows
            .iter()
            .find(|&&(s, _)| s >= b)
            .map(|&(s, e)| (s + e) / 2);
        if let Some(y) = above {
            cuts.push(region.y0 + y);
        }
        if let Some(y) = below {
            cuts.push(region.y0 + y);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    (!cuts.is_empty()).then_some(cuts)
}

/// A connected component: its tight box and pixel count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Component {
    pub bbox: PxBox,
    pub area: u32,
}

/// Union-find with path halving.
struct Dsu(Vec<u32>);

impl Dsu {
    fn new(n: usize) -> Self {
        Dsu((0..n as u32).collect())
    }
    fn find(&mut self, mut a: u32) -> u32 {
        while self.0[a as usize] != a {
            self.0[a as usize] = self.0[self.0[a as usize] as usize];
            a = self.0[a as usize];
        }
        a
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            self.0[hi as usize] = lo;
        }
    }
}

/// 8-connected components of the ink in `region`, by two-pass labelling:
/// the first pass gives each pixel the smallest label among its labelled
/// neighbours (west, north-west, north, north-east) and records the others
/// as equivalent; the second resolves the equivalences and accumulates each
/// component's box. Ordered by top, then left.
pub fn components(bits: &Bits, region: PxBox) -> Vec<Component> {
    let (w, h) = (region.w(), region.h());
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut labels = vec![0u32; w * h];
    let mut dsu = Dsu::new(1);
    let mut next = 1u32;
    for y in 0..h {
        for x in 0..w {
            if !bits.get(region.x0 + x, region.y0 + y) {
                continue;
            }
            let mut found = 0u32;
            let neighbour = |l: u32, found: &mut u32, dsu: &mut Dsu| {
                if l == 0 {
                    return;
                }
                if *found == 0 {
                    *found = l;
                } else if l != *found {
                    dsu.union(*found, l);
                }
            };
            if x > 0 {
                neighbour(labels[y * w + x - 1], &mut found, &mut dsu);
            }
            if y > 0 {
                if x > 0 {
                    neighbour(labels[(y - 1) * w + x - 1], &mut found, &mut dsu);
                }
                neighbour(labels[(y - 1) * w + x], &mut found, &mut dsu);
                if x + 1 < w {
                    neighbour(labels[(y - 1) * w + x + 1], &mut found, &mut dsu);
                }
            }
            if found == 0 {
                found = next;
                next += 1;
                dsu.0.push(found);
            }
            labels[y * w + x] = found;
        }
    }
    let mut boxes: Vec<Option<Component>> = vec![None; next as usize];
    for y in 0..h {
        for x in 0..w {
            let l = labels[y * w + x];
            if l == 0 {
                continue;
            }
            let root = dsu.find(l) as usize;
            let (px, py) = (region.x0 + x, region.y0 + y);
            let c = boxes[root].get_or_insert(Component {
                bbox: PxBox::new(px, py, px + 1, py + 1),
                area: 0,
            });
            c.bbox = c.bbox.union(&PxBox::new(px, py, px + 1, py + 1));
            c.area += 1;
        }
    }
    let mut out: Vec<Component> = boxes.into_iter().flatten().collect();
    out.sort_by_key(|c| (c.bbox.y0, c.bbox.x0));
    out
}

/// Fraction bars and overlines by run-length analysis: horizontal runs of
/// ink at least `min_len` long, stacked over at most `max_thick` rows, are
/// one rule; returned as boxes. Strokes of letters are short, and a filled
/// shape is thick: neither is a rule.
pub fn horizontal_rules(
    bits: &Bits,
    region: PxBox,
    min_len: usize,
    max_thick: usize,
) -> Vec<PxBox> {
    // Long runs, row by row.
    let mut runs: Vec<PxBox> = Vec::new();
    for y in region.y0..region.y1 {
        let mut x = region.x0;
        while x < region.x1 {
            if bits.get(x, y) {
                let s = x;
                while x < region.x1 && bits.get(x, y) {
                    x += 1;
                }
                if x - s >= min_len {
                    runs.push(PxBox::new(s, y, x, y + 1));
                }
            } else {
                x += 1;
            }
        }
    }
    // Runs on consecutive rows that mostly coincide are one rule.
    let mut rules: Vec<PxBox> = Vec::new();
    for r in runs {
        let joined = rules
            .iter_mut()
            .rev()
            .find(|q| q.y1 == r.y0 && q.x_overlap(&r) * 10 >= 8 * q.w().min(r.w()));
        match joined {
            Some(q) => *q = q.union(&r),
            None => rules.push(r),
        }
    }
    rules.retain(|q| q.h() <= max_thick && q.w() >= q.h() * 4);
    // A rule must not continue into ink above or below beyond its ends'
    // anti-aliasing: a box drawn around text is not a rule but its edges
    // are; those have ink running vertically off their ends.
    rules
        .into_iter()
        .filter(|q| {
            let above = q.y0 > region.y0
                && (q.x0..q.x1).filter(|&x| bits.get(x, q.y0 - 1)).count() * 2 > q.w();
            let below =
                q.y1 < region.y1 && (q.x0..q.x1).filter(|&x| bits.get(x, q.y1)).count() * 2 > q.w();
            !(above || below)
        })
        .collect()
}

/// Groups of boxes that touch once each is grown by `dy` up and down and
/// `dx` left and right: a numerator, its bar and its denominator; an
/// operator and its limits. Returns a group number per box (groups numbered
/// from 0 in order of their first box).
pub fn group_by_dilation(boxes: &[PxBox], dx: usize, dy: usize) -> Vec<usize> {
    let n = boxes.len();
    let mut dsu = Dsu::new(n);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| boxes[i].x0);
    // Sweep by x so only boxes that can meet horizontally are compared.
    for (k, &i) in order.iter().enumerate() {
        let a = boxes[i];
        for &j in &order[k + 1..] {
            let b = boxes[j];
            if b.x0 > a.x1 + 2 * dx {
                break;
            }
            let meets_x = b.x0 < a.x1 + 2 * dx && a.x0 < b.x1 + 2 * dx;
            let meets_y = b.y0 < a.y1 + 2 * dy && a.y0 < b.y1 + 2 * dy;
            if meets_x && meets_y {
                dsu.union(i as u32, j as u32);
            }
        }
    }
    let mut ids: Vec<Option<usize>> = vec![None; n];
    let mut next = 0;
    (0..n)
        .map(|i| {
            let root = dsu.find(i as u32) as usize;
            *ids[root].get_or_insert_with(|| {
                next += 1;
                next - 1
            })
        })
        .collect()
}

/// Strips joined into lines. Two consecutive strips of a column are one
/// line when one is stacked on the other: the lower holds a rule under the
/// upper (a numerator over its bar) or the upper a rule over the lower,
/// within `stack_gap`; or one is a narrow strip nested within the other's
/// extent within `nest_gap` (an accent, a limit, a script set apart). The
/// closer partner wins. Returns the strips' indices per line, top down.
pub fn logical_lines(
    strips: &[PxBox],
    rules: &[PxBox],
    em: usize,
    stack_gap: usize,
    nest_gap: usize,
) -> Vec<Vec<usize>> {
    let n = strips.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| strips[i].y0);
    let rule_in = |s: &PxBox| -> Vec<PxBox> {
        rules
            .iter()
            .copied()
            .filter(|r| r.y0 >= s.y0 && r.y1 <= s.y1 && r.x0 >= s.x0 && r.x1 <= s.x1)
            .collect()
    };
    // Only a small strip nests: an accent, a limit, a script set apart; a
    // whole line of a display under a wider line of text does not.
    let nested = |inner: &PxBox, outer: &PxBox| {
        inner.h() * 4 <= em * 3
            && inner.x0 + em / 2 >= outer.x0
            && inner.x1 <= outer.x1 + em / 2
            && inner.w() * 5 <= outer.w() * 4
    };
    // Whether order[k] and order[k+1] are one line, and how close.
    let joins = |a: &PxBox, b: &PxBox| -> Option<usize> {
        let gap = b.y0.saturating_sub(a.y1);
        let over_bar = rule_in(b)
            .iter()
            .any(|r| a.x0 + em / 4 >= r.x0 && a.x1 <= r.x1 + em / 4 && r.y0 <= b.y0 + em);
        let under_bar = rule_in(a)
            .iter()
            .any(|r| b.x0 + em / 4 >= r.x0 && b.x1 <= r.x1 + em / 4 && r.y1 + em >= a.y1);
        if (over_bar || under_bar) && gap <= stack_gap {
            return Some(gap);
        }
        if gap <= nest_gap && (nested(a, b) || nested(b, a)) {
            return Some(gap);
        }
        None
    };
    let pairs: Vec<Option<usize>> = order
        .windows(2)
        .map(|w| joins(&strips[w[0]], &strips[w[1]]))
        .collect();
    let mut lines: Vec<Vec<usize>> = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        let up = if k > 0 { pairs[k - 1] } else { None };
        let down = pairs.get(k).copied().flatten();
        // A strip that could join both the line above and the strip below
        // goes with the closer: an accent over a numerator belongs below.
        // Clearly closer: a pixel or two either way is a tie, and a tie
        // stays with the line above (a limit under a sum).
        let prefer_down = matches!((up, down), (Some(gu), Some(gd)) if gd + 2 < gu);
        if up.is_some() && !prefer_down {
            lines.last_mut().expect("a line above").push(i);
        } else {
            lines.push(vec![i]);
        }
    }
    lines
}

/// A page rendered for analysis, and its scale.
#[derive(Clone, Debug)]
pub struct PageRaster {
    pub bits: Bits,
    /// Pixels per point.
    pub scale: f32,
}

impl PageRaster {
    /// A pixel box in page points `[x, y, w, h]`.
    pub fn to_pt(&self, b: &PxBox) -> [f32; 4] {
        let s = self.scale;
        [
            b.x0 as f32 / s,
            b.y0 as f32 / s,
            b.w() as f32 / s,
            b.h() as f32 / s,
        ]
    }
    /// Page points `[x, y, w, h]` as a pixel box, clipped to the raster.
    pub fn to_px(&self, r: [f32; 4]) -> PxBox {
        let s = self.scale;
        let clip = |v: f32, max: usize| (v.max(0.0) as usize).min(max);
        PxBox::new(
            clip((r[0] * s).floor(), self.bits.w),
            clip((r[1] * s).floor(), self.bits.h),
            clip(((r[0] + r[2]) * s).ceil(), self.bits.w),
            clip(((r[1] + r[3]) * s).ceil(), self.bits.h),
        )
    }
    /// Pixels in a point.
    pub fn px(&self, pt: f32) -> usize {
        (pt * self.scale).round().max(1.0) as usize
    }
}

/// Render a page in display orientation (its /Rotate applied) to grey and
/// binarize it with Otsu's threshold. No annotations: the page's own ink.
pub fn render_raster(
    b: &dyn PdfiumLibraryBindings,
    page: FPDF_PAGE,
    page_w: f32,
    page_h: f32,
) -> Option<PageRaster> {
    let mut scale = ANALYSIS_SCALE;
    if page_w * page_h * scale * scale > MAX_PIXELS {
        scale = (MAX_PIXELS / (page_w * page_h)).sqrt();
    }
    let (w, h) = (
        (page_w * scale).round().max(1.0) as usize,
        (page_h * scale).round().max(1.0) as usize,
    );
    let bitmap = b.FPDFBitmap_Create(w as i32, h as i32, 0);
    if bitmap.is_null() {
        return None;
    }
    b.FPDFBitmap_FillRect(bitmap, 0, 0, w as i32, h as i32, 0xFFFF_FFFF);
    b.FPDF_RenderPageBitmap(bitmap, page, 0, 0, w as i32, h as i32, 0, 0);
    let stride = b.FPDFBitmap_GetStride(bitmap) as usize;
    let buffer = b.FPDFBitmap_GetBuffer(bitmap) as *const u8;
    if buffer.is_null() {
        b.FPDFBitmap_Destroy(bitmap);
        return None;
    }
    // SAFETY: the buffer holds stride*h bytes until FPDFBitmap_Destroy.
    let src = unsafe { std::slice::from_raw_parts(buffer, stride * h) };
    let mut gray = vec![255u8; w * h];
    for y in 0..h {
        let row = &src[y * stride..y * stride + w * 4];
        for x in 0..w {
            let (bl, g, r) = (
                row[x * 4] as u32,
                row[x * 4 + 1] as u32,
                row[x * 4 + 2] as u32,
            );
            gray[y * w + x] = ((r * 299 + g * 587 + bl * 114) / 1000) as u8;
        }
    }
    b.FPDFBitmap_Destroy(bitmap);
    let threshold = otsu(&gray);
    Some(PageRaster {
        bits: binarize(&gray, w, h, threshold),
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otsu_splits_ink_from_paper() {
        let mut gray = vec![250u8; 900];
        gray.extend(vec![20u8; 100]);
        let t = otsu(&gray);
        assert!((20..250).contains(&t), "{t}");
        assert!(binarize(&gray, 1000, 1, t).on[950]);
        assert!(!binarize(&gray, 1000, 1, t).on[0]);
        // One tone: nothing to split.
        assert_eq!(otsu(&[255u8; 50]), 160);
    }

    #[test]
    fn valleys_are_inside_the_ink_only() {
        assert_eq!(
            valleys(&[0, 3, 0, 0, 2, 0, 1, 0, 0], 1),
            vec![(2, 4), (5, 6)]
        );
        assert_eq!(valleys(&[0, 3, 0, 0, 2, 0, 1, 0, 0], 2), vec![(2, 4)]);
    }

    #[test]
    fn labels_eight_connected_components() {
        let bits = Bits::from_art(&[
            "##....#", //
            ".#...#.", // the diagonal joins
            "....#..", //
            "##.....", //
        ]);
        let cs = components(&bits, PxBox::new(0, 0, 7, 4));
        assert_eq!(cs.len(), 3, "{cs:?}");
        assert_eq!(cs[0].bbox, PxBox::new(0, 0, 2, 2));
        assert_eq!(cs[0].area, 3);
        assert_eq!(cs[1].bbox, PxBox::new(4, 0, 7, 3));
        assert_eq!(cs[2].bbox, PxBox::new(0, 3, 2, 4));
    }

    #[test]
    fn a_u_shape_is_one_component() {
        // Two labels meet only at the bottom: the equivalence must resolve.
        let bits = Bits::from_art(&["#...#", "#...#", "#####"]);
        let cs = components(&bits, PxBox::new(0, 0, 5, 3));
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].area, 9);
    }

    #[test]
    fn finds_long_thin_runs_as_rules() {
        let bits = Bits::from_art(&[
            "..#.....#.......",
            "..##############", // a bar, two rows thick
            "...#############",
            "........#.......",
            "#..#............", // short strokes are not rules
        ]);
        let rules = horizontal_rules(&bits, PxBox::new(0, 0, 16, 5), 8, 3);
        assert_eq!(rules, vec![PxBox::new(2, 1, 16, 3)]);
        // Too thick to be a bar.
        let block = Bits::from_art(&["##########", "##########", "##########", "##########"]);
        assert!(horizontal_rules(&block, PxBox::new(0, 0, 10, 4), 8, 3).is_empty());
    }

    #[test]
    fn dilation_fuses_a_fraction_stack_and_not_its_neighbours() {
        let boxes = [
            PxBox::new(10, 0, 20, 8),   // numerator
            PxBox::new(8, 10, 22, 11),  // bar
            PxBox::new(12, 13, 18, 21), // denominator
            PxBox::new(0, 6, 6, 14),    // "=" to the left, apart
            PxBox::new(40, 0, 50, 8),   // another word
        ];
        let g = group_by_dilation(&boxes, 1, 2);
        assert_eq!(g[0], g[1]);
        assert_eq!(g[1], g[2]);
        assert_ne!(g[3], g[0]);
        assert_ne!(g[4], g[0]);
        assert_ne!(g[3], g[4]);
    }

    #[test]
    fn xy_cut_finds_columns_then_strips() {
        // Two columns of two lines each, then a full-width line.
        let mut rows: Vec<String> = Vec::new();
        for _ in 0..2 {
            rows.push("#####......#####".into());
            rows.push("................".into());
        }
        rows.push("#####......#####".into());
        for _ in 0..4 {
            rows.push("................".into());
        }
        rows.push("################".into());
        let art: Vec<&str> = rows.iter().map(String::as_str).collect();
        let bits = Bits::from_art(&art);
        let strips = xy_cut(
            &bits,
            PxBox::new(0, 0, 16, art.len()),
            CutOptions {
                min_gutter: 4,
                min_gutter_height: 4,
                band_gap: 3,
                min_column_width: 3,
            },
        );
        // The band of columns, cut at the gutter: 3 strips each; then the
        // full-width line.
        assert_eq!(strips.len(), 7, "{strips:?}");
        let left: Vec<&Strip> = strips.iter().filter(|s| s.column.x1 <= 5).collect();
        assert_eq!(left.len(), 3);
        assert!(strips.iter().any(|s| s.bbox == PxBox::new(0, 9, 16, 10)));
    }

    #[test]
    fn a_line_across_the_gutter_is_cut_out_and_the_columns_found() {
        let mut rows: Vec<String> = Vec::new();
        for _ in 0..3 {
            rows.push("#####......#####".into());
            rows.push("................".into());
        }
        rows.push("################".into()); // across both columns
        rows.push("................".into());
        for _ in 0..3 {
            rows.push("#####......#####".into());
            rows.push("................".into());
        }
        let art: Vec<&str> = rows.iter().map(String::as_str).collect();
        let bits = Bits::from_art(&art);
        let strips = xy_cut(
            &bits,
            PxBox::new(0, 0, 16, art.len()),
            CutOptions {
                min_gutter: 4,
                min_gutter_height: 4,
                band_gap: 30,
                min_column_width: 3,
            },
        );
        let left = strips.iter().filter(|s| s.column.x1 <= 5).count();
        let right = strips.iter().filter(|s| s.column.x0 >= 11).count();
        assert_eq!((left, right), (6, 6), "{strips:?}");
        assert!(strips.iter().any(|s| s.bbox.w() == 16));
    }

    #[test]
    fn joins_a_numerator_to_its_bar_and_keeps_text_lines_apart() {
        let strips = [
            PxBox::new(0, 0, 100, 10),  // a line of text
            PxBox::new(40, 20, 60, 28), // numerator
            PxBox::new(20, 30, 80, 40), // the main line, holding the bar
            PxBox::new(42, 42, 58, 50), // denominator
            PxBox::new(0, 60, 100, 70), // text again
        ];
        let rules = [PxBox::new(38, 34, 62, 35)];
        let lines = logical_lines(&strips, &rules, 10, 7, 3);
        assert_eq!(lines, vec![vec![0], vec![1, 2, 3], vec![4]]);
    }
}

//! Destination-aware preview crop geometry. This module is PDFium-free so the
//! column and vertical-boundary heuristics can be exercised as ordinary unit
//! tests.

use super::types::{TextRun, TileRect};

pub const PREVIEW_SCALE_MILLI: u32 = 2_000;

pub struct CropInput<'a> {
    pub page_w_pt: f32,
    pub page_h_pt: f32,
    pub dest_x: Option<f32>,
    pub dest_y: Option<f32>,
    pub runs: &'a [TextRun],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub start: f32,
    pub end: f32,
}

impl Interval {
    pub(crate) fn width(self) -> f32 {
        self.end - self.start
    }

    /// How far `x` lies outside the interval; zero inside it.
    fn distance(self, x: f32) -> f32 {
        (self.start - x).max(x - self.end).max(0.0)
    }
}

pub(crate) fn median_height(runs: &[&TextRun]) -> f32 {
    if runs.is_empty() {
        return 11.0;
    }
    let mut heights: Vec<f32> = runs.iter().map(|run| run.h.max(1.0)).collect();
    heights.sort_by(f32::total_cmp);
    let middle = heights.len() / 2;
    if heights.len() % 2 == 0 {
        (heights[middle - 1] + heights[middle]) / 2.0
    } else {
        heights[middle]
    }
}

/// Fewest prose lines a side of a gutter needs before it counts as a column.
const MIN_COLUMN_LINES: usize = 4;
/// The narrowest gutter: LaTeX's default `\columnsep` is 10pt, journals set
/// 12-24pt.
const MIN_GUTTER_PT: f32 = 7.0;
/// Runs on one row closer than this are one stretch of text. PDFium breaks
/// a line into runs at any gap wider than ~0.6 of its glyph height, which
/// for a word without ascenders is an ordinary word space - so a run is
/// often one word, and only rows say where a line begins and ends. A
/// justified line's widest spaces stay under this; a gutter never does.
const JOIN_GAP_PT: f32 = MIN_GUTTER_PT - 0.5;

/// The page's runs joined into stretches of text along each row, wherever
/// the gap between them is narrower than any gutter.
pub(crate) fn segments(runs: &[TextRun]) -> Vec<TextRun> {
    rows_of(runs, &[])
}

/// As `segments`, knowing the page's columns. A display overrunning its
/// column can end within a word space of the next column's line on the same
/// baseline, and the two would read as one full-width row. So a run
/// starting flush at a column's left edge joins what lies across the gutter
/// only when the run before it is of a like height - a full-width caption
/// that happens to break between words right there - not a display's tall
/// delimiter or a trailing full stop.
fn rows_of(runs: &[TextRun], columns: &[Interval]) -> Vec<TextRun> {
    rows_with_members(runs, columns, false).0
}

/// As `rows_of`, and the row each run went into (`None` for a run with no
/// extent, which joins none). With `split_at_gutters`, a run starting flush
/// at a column's edge never joins across the gutter, whatever its height:
/// where the gutter is narrower than a word space, the lines of two
/// columns would otherwise run together.
pub(crate) fn rows_with_members(
    runs: &[TextRun],
    columns: &[Interval],
    split_at_gutters: bool,
) -> (Vec<TextRun>, Vec<Option<usize>>) {
    let flush_across = |last: &TextRun, run: &TextRun| {
        columns.windows(2).any(|pair| {
            let edge = pair[1].start;
            let flush = (edge - 1.0..=edge + 1.5).contains(&run.x);
            let same_type = (last.h - run.h).abs() <= 0.5 * run.h.max(last.h);
            last.x + last.w < edge && flush && (split_at_gutters || !same_type)
        })
    };
    let mut sorted: Vec<usize> = (0..runs.len())
        .filter(|&i| runs[i].w > 0.0 && runs[i].h > 0.0)
        .collect();
    sorted.sort_by(|&left, &right| runs[left].x.total_cmp(&runs[right].x));
    let mut member: Vec<Option<usize>> = vec![None; runs.len()];
    let mut out: Vec<TextRun> = Vec::new();
    // The last run joined to each segment.
    let mut last: Vec<&TextRun> = Vec::new();
    // Segments still open to the right: a run joins the nearest one on its
    // row whose end it starts just after.
    let mut open: Vec<usize> = Vec::new();
    for index in sorted {
        let run = &runs[index];
        let joined = open.iter().copied().find(|&i| {
            let seg = &out[i];
            let gap = run.x - (seg.x + seg.w);
            let overlap = (run.y + run.h).min(seg.y + seg.h) - run.y.max(seg.y);
            (-2.0..=JOIN_GAP_PT).contains(&gap)
                && overlap >= 0.5 * run.h.min(seg.h)
                && !flush_across(last[i], run)
        });
        match joined {
            Some(i) => {
                let seg = &mut out[i];
                let x1 = (seg.x + seg.w).max(run.x + run.w);
                let y0 = seg.y.min(run.y);
                let y1 = (seg.y + seg.h).max(run.y + run.h);
                seg.w = x1 - seg.x;
                seg.y = y0;
                seg.h = y1 - y0;
                last[i] = run;
                member[index] = Some(i);
            }
            None => {
                out.push(TextRun {
                    text: String::new(),
                    start: run.start,
                    x: run.x,
                    y: run.y,
                    w: run.w,
                    h: run.h,
                    edges: Vec::new(),
                    math: Vec::new(),
                });
                last.push(run);
                open.push(out.len() - 1);
                member[index] = Some(out.len() - 1);
            }
        }
        // A segment that ends well left of this run can take no more.
        open.retain(|&i| out[i].x + out[i].w >= run.x - JOIN_GAP_PT);
    }
    (out, member)
}

/// The page's text columns, left to right, read from the whole page.
///
/// A gutter is an x band in the middle of a text block that the fewest rows
/// of text cross, with prose lines - stretches most of a column wide - set
/// on both sides of it and the right-hand lines starting flush. Full-width
/// material (a title block and abstract, a `figure*` caption, a wide table
/// or display) crosses the gutter, but it crosses every other x of the
/// block too, so it raises the floor without moving the minimum.
/// Clustering the runs around one point instead (what this replaced) let a
/// single full-width line anywhere near that point fuse two columns into
/// one. Requiring prose-wide stretches keeps the aligned cells of a table
/// from reading as columns. A single-column page returns one column.
pub fn page_columns(runs: &[TextRun], page_w_pt: f32) -> Vec<Interval> {
    columns_of(&segments(runs), page_w_pt)
}

fn columns_of(segments: &[TextRun], page_w_pt: f32) -> Vec<Interval> {
    let page_w = page_w_pt.max(1.0);
    let usable: Vec<Interval> = segments
        .iter()
        .map(|run| Interval {
            start: run.x.clamp(0.0, page_w),
            end: (run.x + run.w).clamp(0.0, page_w),
        })
        .filter(|interval| interval.end > interval.start)
        .collect();
    let mut out = Vec::new();
    let page = Interval {
        start: 0.0,
        end: page_w,
    };
    split_columns(&usable, 0, page, &mut out);
    out.sort_by(|left, right| left.start.total_cmp(&right.start));
    out
}

fn extent(runs: &[Interval]) -> Option<Interval> {
    let start = runs.iter().map(|run| run.start).reduce(f32::min)?;
    let end = runs.iter().map(|run| run.end).reduce(f32::max)?;
    Some(Interval { start, end })
}

/// `runs`' columns, kept within `bounds`: the margins the gutters either side
/// of this block set.
fn split_columns(runs: &[Interval], depth: u32, bounds: Interval, out: &mut Vec<Interval>) {
    let Some(block) = extent(runs) else {
        return;
    };
    match (depth < 2).then(|| find_gutter(runs, block)).flatten() {
        Some(gutter) => {
            let left = Interval {
                start: bounds.start,
                end: gutter.left_margin,
            };
            let right = Interval {
                start: gutter.right_margin,
                end: bounds.end,
            };
            split_columns(&gutter.left, depth + 1, left, out);
            split_columns(&gutter.right, depth + 1, right, out);
        }
        None => out.push(Interval {
            start: block.start.max(bounds.start),
            end: block.end.min(bounds.end),
        }),
    }
}

/// A gutter found in a block of text.
struct Gutter {
    left: Vec<Interval>,
    right: Vec<Interval>,
    /// Where the left-hand column's lines end: its justified right margin.
    left_margin: f32,
    /// Where the right-hand column's lines start: its flush left margin.
    right_margin: f32,
}

/// The edge most of `edges` share, to within two points, and how many
/// share it: the outermost of them, so that every flush line is inside.
fn shared_edge(edges: impl Iterator<Item = f32>, outermost_is_max: bool) -> Option<(f32, usize)> {
    let mut values: Vec<f32> = edges.collect();
    values.sort_by(f32::total_cmp);
    let mut best: Option<(f32, usize)> = None;
    for i in 0..values.len() {
        let within = values[i..]
            .iter()
            .take_while(|&&v| v - values[i] <= 2.0)
            .count();
        if best.is_none_or(|(_, count)| within > count) {
            let edge = if outermost_is_max {
                values[i + within - 1]
            } else {
                values[i]
            };
            best = Some((edge, within));
        }
    }
    best
}

/// The gutter of a block of text, when it has one.
fn find_gutter(runs: &[Interval], block: Interval) -> Option<Gutter> {
    if runs.len() < 2 * MIN_COLUMN_LINES || block.width() < 4.0 * MIN_GUTTER_PT {
        return None;
    }
    // How many stretches cross each x in the middle half of the block, at
    // one-point steps.
    let from = block.start + 0.25 * block.width();
    let to = block.end - 0.25 * block.width();
    let steps = ((to - from).max(0.0) as usize).min(2_000);
    let crossing = |x: f32| {
        runs.iter()
            .filter(|run| run.start < x - 0.5 && run.end > x + 0.5)
            .count()
    };
    let counts: Vec<usize> = (0..=steps).map(|i| crossing(from + i as f32)).collect();
    let least = *counts.iter().min()?;
    // The widest stretch at (about) that floor is the gutter. The slack
    // lets a line or two overrunning into the gutter not cut it in half,
    // which would leave a hanging label's gap beside it as the widest.
    let ceiling = least + 2 + runs.len() / 100;
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < counts.len() {
        if counts[i] > ceiling {
            i += 1;
            continue;
        }
        let first = i;
        while i + 1 < counts.len() && counts[i + 1] <= ceiling {
            i += 1;
        }
        if best.is_none_or(|(a, b)| i - first > b - a) {
            best = Some((first, i));
        }
        i += 1;
    }
    let (first, last) = best?;
    let (gap_start, gap_end) = (from + first as f32, from + last as f32);
    if gap_end - gap_start + 1.0 < MIN_GUTTER_PT {
        return None;
    }
    let mid = (gap_start + gap_end) / 2.0;
    // Something set inside the gutter itself (a line number, a stray mark)
    // belongs to neither column, and must not widen either.
    let in_gutter = |run: &Interval| run.start >= gap_start - 0.5 && run.end <= gap_end + 0.5;
    let left: Vec<Interval> = runs
        .iter()
        .copied()
        .filter(|run| {
            run.end <= gap_end + 0.5 && run.start + run.end < 2.0 * mid && !in_gutter(run)
        })
        .collect();
    let right: Vec<Interval> = runs
        .iter()
        .copied()
        .filter(|run| {
            run.start >= gap_start - 0.5 && run.start + run.end >= 2.0 * mid && !in_gutter(run)
        })
        .collect();
    let left_block = extent(&left)?;
    let right_block = extent(&right)?;
    let prose = |side: &[Interval], block: Interval| {
        side.iter()
            .filter(|run| run.width() >= 0.6 * block.width())
            .count()
    };
    // The columns' inner margins: where most of the left column's lines end
    // and the right column's start. The right-hand column's lines start
    // flush, as a table's centred cells or a single column's paragraph ends
    // do not; a ragged-right left column ends where its longest line does.
    let wide = |side: &[Interval], block: Interval| -> Vec<Interval> {
        side.iter()
            .copied()
            .filter(|run| run.width() >= 0.4 * block.width())
            .collect()
    };
    let (right_margin, flush) =
        shared_edge(wide(&right, right_block).iter().map(|r| r.start), false)?;
    let left_margin = match shared_edge(wide(&left, left_block).iter().map(|r| r.end), true) {
        Some((edge, count)) if count >= MIN_COLUMN_LINES => edge,
        _ => left_block.end,
    };
    if right_margin - left_margin < MIN_GUTTER_PT {
        return None;
    }
    // Each column reaches as far into the gutter as its own text does - a
    // hanging reference label, a line overrunning its margin - but not to
    // within a few points of the other column's margin.
    let reach = 0.5 * MIN_GUTTER_PT;
    let left_margin = left
        .iter()
        .map(|run| run.end)
        .filter(|&end| end <= right_margin - reach)
        .fold(left_margin, f32::max);
    let right_margin = right
        .iter()
        .map(|run| run.start)
        .filter(|&start| start >= left_margin + reach)
        .fold(right_margin, f32::min);
    let columns = prose(&left, left_block).min(prose(&right, right_block));
    // A column is a real share of the block (not a strip of equation
    // numbers down the margin).
    let shares =
        (left_margin - left_block.start).min(right_block.end - right_margin) >= 0.3 * block.width();
    // Full-width stretches cross the gutter; when they outnumber the lines
    // of a column, the page is a single column with a narrow gap somewhere.
    let enough = columns >= MIN_COLUMN_LINES
        && flush >= MIN_COLUMN_LINES
        && shares
        && least <= left.len().min(right.len());
    enough.then_some(Gutter {
        left,
        right,
        left_margin,
        right_margin,
    })
}

/// The column a run at `x` belongs to: the one holding it, or the nearest.
pub(crate) fn nearest_column(columns: &[Interval], x: f32) -> usize {
    (0..columns.len())
        .min_by(|&a, &b| columns[a].distance(x).total_cmp(&columns[b].distance(x)))
        .unwrap_or(0)
}

/// Whether a stretch of text reaches across a gutter into a neighbouring
/// column.
pub(crate) fn spans_gutter(columns: &[Interval], index: usize, run: &TextRun) -> bool {
    let left = index
        .checked_sub(1)
        .and_then(|i| columns.get(i))
        .is_some_and(|other| run.x < other.end - 2.0);
    let right = columns
        .get(index + 1)
        .is_some_and(|other| run.x + run.w > other.start + 2.0);
    left || right
}

/// The first row at or below `anchor_y` inside `area`: the top of the
/// target a destination points at.
fn first_run_below(runs: &[TextRun], area: Interval, anchor_y: f32) -> Option<&TextRun> {
    runs.iter()
        .filter(|run| {
            run.w > 0.0
                && run.h > 0.0
                && run.x + run.w > area.start
                && run.x < area.end
                && run.y + run.h <= anchor_y + 0.6 * run.h.max(4.0)
        })
        .max_by(|left, right| (left.y + left.h).total_cmp(&(right.y + right.h)))
}

/// Which of `columns` a destination at (`dest_x`, `anchor_y`) previews, or
/// None when it is a full-width element.
fn choose_column(
    segments: &[TextRun],
    columns: &[Interval],
    anchor_y: f32,
    dest_x: Option<f32>,
) -> Option<usize> {
    let full = Interval {
        start: columns.first()?.start,
        end: columns.last()?.end,
    };
    let index = match dest_x {
        Some(x) => nearest_column(columns, x),
        None => {
            // No x (a /FitH destination): the column of the row nearest
            // under the anchor.
            let row = first_run_below(segments, full, anchor_y)?;
            nearest_column(columns, row.x + row.w.min(20.0) / 2.0)
        }
    };
    // Walk down from the anchor. A prose line in the column says it is
    // column text; a row reaching across the gutter says it is full-width -
    // a `figure*` or `table*` caption, where hypcap anchors at the float's
    // top, above artwork whose labels and cells are narrow rows that say
    // nothing either way.
    let column = columns[index];
    let below = |row: &TextRun| {
        row.w > 0.0 && row.h > 0.0 && row.y + row.h <= anchor_y + 0.6 * row.h.max(4.0)
    };
    let mut rows: Vec<&TextRun> = segments
        .iter()
        .filter(|row| {
            below(row)
                && row.x + row.w > column.start
                && row.x < column.end
                && row.y + row.h >= anchor_y - FULL_WIDTH_REACH_PT
        })
        .collect();
    rows.sort_by(|left, right| (right.y + right.h).total_cmp(&(left.y + left.h)));
    for row in rows {
        if spans_gutter(columns, index, row) {
            // Full width only if no column has prose between the anchor
            // and that row: else the row is a float further down, and the
            // target the column text above it.
            let prose_between = segments.iter().any(|other| {
                let top = other.y + other.h;
                below(other) && top > row.y + row.h && {
                    let i = nearest_column(columns, other.x);
                    !spans_gutter(columns, i, other) && other.w >= 0.6 * columns[i].width()
                }
            });
            return prose_between.then_some(index);
        }
        if row.w >= 0.6 * column.width() {
            return Some(index);
        }
    }
    Some(index)
}

/// How far below a destination its first decisive row may be: the height of
/// a float whose artwork fills most of a page.
const FULL_WIDTH_REACH_PT: f32 = 720.0;

/// Where a destination's preview sits across the page.
struct Area {
    /// The column, or the full text width.
    bounds: Interval,
    columns: Vec<Interval>,
    /// Which of `columns` `bounds` is, when it is one of several.
    index: Option<usize>,
    segments: Vec<TextRun>,
}

fn target_area(runs: &[TextRun], page_w_pt: f32, anchor_y: f32, dest_x: Option<f32>) -> Area {
    let columns = columns_of(&segments(runs), page_w_pt);
    let segments = rows_of(runs, &columns);
    if columns.len() < 2 {
        let bounds = single_column_bounds(runs, page_w_pt, anchor_y, dest_x);
        return Area {
            bounds,
            columns,
            index: None,
            segments,
        };
    }
    let index = choose_column(&segments, &columns, anchor_y, dest_x);
    let bounds = match index {
        Some(i) => columns[i],
        None => Interval {
            start: columns[0].start,
            end: columns[columns.len() - 1].end,
        },
    };
    Area {
        bounds,
        columns,
        index,
        segments,
    }
}

/// Horizontal extent of the text column holding `anchor_y` (and `dest_x`, when
/// the destination named one): one column of a multi-column page, or the
/// full text width when the target spans the columns (a `figure*`, a
/// title block, a wide display).
///
/// Shared with the figure crop, which needs the same answer for a caption,
/// and the equation hover, for a display.
pub fn column_bounds(
    runs: &[TextRun],
    page_w_pt: f32,
    anchor_y: f32,
    dest_x: Option<f32>,
) -> Interval {
    target_area(runs, page_w_pt, anchor_y, dest_x).bounds
}

/// A single-column page: the run clusters around the anchor, so a margin
/// note or line numbers stay out of a body crop.
fn single_column_bounds(
    runs: &[TextRun],
    page_w_pt: f32,
    anchor_y: f32,
    dest_x: Option<f32>,
) -> Interval {
    let page_w = page_w_pt.max(1.0);
    let mut intervals: Vec<Interval> = runs
        .iter()
        .filter(|run| run.w > 0.0 && run.y >= anchor_y - 400.0 && run.y <= anchor_y + 8.0)
        .map(|run| Interval {
            start: run.x.clamp(0.0, page_w),
            end: (run.x + run.w).clamp(0.0, page_w),
        })
        .filter(|interval| interval.end > interval.start)
        .collect();
    intervals.sort_by(|left, right| left.start.total_cmp(&right.start));

    let gap_limit = 0.02 * page_w;
    let mut clusters: Vec<Interval> = Vec::new();
    for interval in intervals {
        if let Some(last) = clusters.last_mut() {
            if interval.start - last.end < gap_limit {
                last.end = last.end.max(interval.end);
                continue;
            }
        }
        clusters.push(interval);
    }

    let fallback = Interval {
        start: 0.06 * page_w,
        end: 0.94 * page_w,
    };
    let widest = clusters
        .iter()
        .copied()
        .max_by(|left, right| left.width().total_cmp(&right.width()));
    match (dest_x, widest) {
        (_, None) => fallback,
        // A destination's x is where its anchor sits, which can be a hair
        // left of the text (an indent, a hanging label): the nearest
        // cluster, not only one holding it exactly - but an x nowhere near
        // any (a generator writing 0) says nothing, and gets the body.
        (Some(x), Some(widest)) => {
            let nearest = clusters[nearest_column(&clusters, x)];
            if nearest.distance(x) <= 12.0 {
                nearest
            } else {
                widest
            }
        }
        (None, Some(widest)) => widest,
    }
}

pub fn crop_rect(input: &CropInput<'_>) -> [f32; 4] {
    let page_w = input.page_w_pt.max(1.0);
    let page_h = input.page_h_pt.max(1.0);
    let anchor_y = input.dest_y.unwrap_or(page_h).clamp(0.0, page_h);

    // A figure or table: the float itself, artwork and whole caption, at
    // whatever height it takes - the popover scales a tall one down.
    if input.dest_y.is_some() {
        if let Some(rect) =
            super::floats::float_for_anchor(input.runs, page_w, page_h, input.dest_x, anchor_y)
        {
            return rect;
        }
    }

    let area = target_area(input.runs, page_w, anchor_y, input.dest_x);
    let column = area.bounds;
    // In one column of several, a row reaching into the next column belongs
    // to a full-width element (a `figure*` caption, a wide table): a column
    // crop could only show a slice of it.
    let foreign = |row: &TextRun| {
        area.index
            .is_some_and(|i| spans_gutter(&area.columns, i, row))
    };
    // Rows, not runs: a full-width caption is broken into runs at its word
    // spaces, and only its row reaches across the gutter.
    let rows: &[TextRun] = if area.index.is_some() {
        &area.segments
    } else {
        input.runs
    };

    // Leave headroom above the linked line itself, not just a hairline sliver,
    // so the preview shows a bit of what precedes the target (context: the
    // end of the prior sentence/heading), not just the destination pinned to
    // the very top edge - but no slice of a full-width element above it.
    let headroom = (anchor_y + 30.0).min(page_h);
    let y_top = rows
        .iter()
        .filter(|row| {
            row.w > 0.0
                && row.x + row.w > column.start
                && row.x < column.end
                && row.y > anchor_y
                && row.y < headroom
                && foreign(row)
        })
        .map(|row| row.y - 1.0)
        .fold(headroom, f32::min);
    let mut vertical: Vec<&TextRun> = rows
        .iter()
        .filter(|run| {
            run.w > 0.0
                && run.h > 0.0
                && run.y <= y_top
                && run.x + run.w > column.start
                && run.x < column.end
        })
        .collect();
    vertical.sort_by(|left, right| right.y.total_cmp(&left.y));

    let median_h = median_height(&vertical);
    let min_h = 90.0_f32.min(page_h);
    let max_h = (0.32 * page_h).clamp(120.0, 400.0).min(page_h);
    let mut accepted_bottom: Option<f32> = None;
    for run in vertical {
        let run_bottom = run.y.max(0.0);
        let run_top = (run.y + run.h).min(page_h);
        if run_top > y_top + median_h {
            continue;
        }
        // A full-width element below the target ends the column's text.
        if foreign(run) {
            if accepted_bottom.is_some() {
                break;
            }
            continue;
        }
        if let Some(current_bottom) = accepted_bottom {
            let accumulated = y_top - current_bottom;
            let gap = current_bottom - run_top;
            if gap > 2.2 * median_h && accumulated >= min_h {
                break;
            }
            if y_top - run_bottom > max_h {
                break;
            }
            accepted_bottom = Some(current_bottom.min(run_bottom));
        } else {
            accepted_bottom = Some(run_bottom.max(y_top - max_h));
        }
    }

    let y_bottom = accepted_bottom.unwrap_or_else(|| (y_top - min_h).max(0.0));
    let left = (column.start - 4.0).max(0.0);
    let right = (column.end + 4.0).min(page_w);
    let bottom = (y_bottom - 4.0).max(0.0);
    let top = (y_top + 4.0).min(page_h);
    [
        left,
        bottom,
        (right - left).max(1.0),
        (top - bottom).max(1.0),
    ]
}

pub fn rect_to_tile(rect: [f32; 4], page_h_pt: f32, scale_milli: u32) -> TileRect {
    let scale = scale_milli as f32 / 1_000.0;
    let [x, y, w, h] = rect;
    TileRect {
        x: (x * scale).round().max(0.0) as u32,
        y: ((page_h_pt - y - h) * scale).round().max(0.0) as u32,
        w: (w * scale).round().clamp(1.0, 4_096.0) as u32,
        h: (h * scale).round().clamp(1.0, 4_096.0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextRun {
        TextRun {
            text: text.into(),
            start: 0,
            x,
            y,
            w,
            h,
            edges: Vec::new(),
            math: Vec::new(),
        }
    }

    #[test]
    fn crops_single_column_body_text() {
        let runs: Vec<_> = (0..16)
            .map(|line| run("body", 55.0, 690.0 - line as f32 * 14.0, 490.0, 10.0))
            .collect();
        let rect = crop_rect(&CropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            dest_x: Some(80.0),
            dest_y: Some(700.0),
            runs: &runs,
        });
        assert!(rect[0] < 55.0 && rect[0] > 45.0);
        assert!(rect[2] > 490.0);
        assert!(rect[3] >= 120.0 && rect[3] <= 270.0);
    }

    #[test]
    fn right_column_destination_excludes_left_column() {
        let mut runs = Vec::new();
        for line in 0..12 {
            let y = 700.0 - line as f32 * 15.0;
            runs.push(run("left", 42.0, y, 235.0, 10.0));
            runs.push(run("right", 330.0, y, 235.0, 10.0));
        }
        let rect = crop_rect(&CropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            dest_x: Some(410.0),
            dest_y: Some(700.0),
            runs: &runs,
        });
        assert!(rect[0] > 300.0, "right-column crop started at {}", rect[0]);
        assert!(rect[0] + rect[2] <= 575.0);
    }

    #[test]
    fn a_narrow_two_column_gutter_still_separates_the_columns() {
        // Real arXiv geometry: 233pt columns with an 18pt gutter. The old 4%
        // threshold (24pt on this page) swallowed the gutter, merging both
        // columns into one cluster and doubling the width of every crop.
        let mut runs = Vec::new();
        for line in 0..14 {
            let y = 700.0 - line as f32 * 14.0;
            runs.push(run("left", 55.0, y, 233.0, 10.0));
            runs.push(run("right", 307.0, y, 233.0, 10.0));
        }
        let rect = crop_rect(&CropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            dest_x: Some(330.0),
            dest_y: Some(700.0),
            runs: &runs,
        });
        assert!(
            rect[0] > 295.0,
            "crop reached into the left column: {rect:?}"
        );
        assert!(rect[2] < 260.0, "crop spans both columns: {rect:?}");
    }

    #[test]
    fn reference_entry_stops_at_large_gap_after_minimum_height() {
        let runs = vec![
            run("entry 1", 60.0, 690.0, 480.0, 12.0),
            run("entry 1 continued", 60.0, 650.0, 480.0, 12.0),
            run("entry 1 final", 60.0, 605.0, 480.0, 12.0),
            run("next entry", 60.0, 520.0, 480.0, 12.0),
        ];
        let rect = crop_rect(&CropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            dest_x: Some(80.0),
            dest_y: Some(700.0),
            runs: &runs,
        });
        assert!(
            rect[1] > 580.0,
            "crop should stop before next entry: {rect:?}"
        );
    }

    #[test]
    fn empty_page_uses_fixed_fallback_box() {
        let rect = crop_rect(&CropInput {
            page_w_pt: 600.0,
            page_h_pt: 800.0,
            dest_x: None,
            dest_y: Some(400.0),
            runs: &[],
        });
        assert_eq!(rect, [32.0, 336.0, 536.0, 98.0]);
    }

    #[test]
    fn missing_y_anchors_crop_at_page_top() {
        let rect = crop_rect(&CropInput {
            page_w_pt: 600.0,
            page_h_pt: 800.0,
            dest_x: None,
            dest_y: None,
            runs: &[],
        });
        assert_eq!(rect[1] + rect[3], 800.0);
    }

    #[test]
    fn vertical_limits_clamp_on_short_and_tall_pages() {
        let short = crop_rect(&CropInput {
            page_w_pt: 200.0,
            page_h_pt: 80.0,
            dest_x: None,
            dest_y: None,
            runs: &[],
        });
        assert!(short[3] <= 80.0);

        let runs: Vec<_> = (0..80)
            .map(|line| run("line", 80.0, 1_900.0 - line as f32 * 12.0, 440.0, 9.0))
            .collect();
        let tall = crop_rect(&CropInput {
            page_w_pt: 600.0,
            page_h_pt: 2_000.0,
            dest_x: Some(100.0),
            dest_y: Some(1_900.0),
            runs: &runs,
        });
        assert!(tall[3] <= 408.0);
    }

    #[test]
    fn tile_conversion_flips_y_and_uses_fixed_scale() {
        assert_eq!(
            rect_to_tile([10.0, 20.0, 100.0, 50.0], 200.0, PREVIEW_SCALE_MILLI),
            TileRect {
                x: 20,
                y: 260,
                w: 200,
                h: 100,
            }
        );
    }

    // Two-column fixtures modelled on real pages: an Elsevier `5p` page
    // (595 x 794, columns 37.6-288.6 and 306.6-557.7, 12pt leading). PDFium
    // breaks a line into runs at many word spaces, so lines are word runs.
    const PAGE_W: f32 = 595.0;
    const PAGE_H: f32 = 794.0;
    const LEFT: (f32, f32) = (37.6, 288.6);
    const RIGHT: (f32, f32) = (306.6, 557.7);

    /// One line of text from `x0` to `x1` on baseline `y`, as word runs.
    fn words(runs: &mut Vec<TextRun>, x0: f32, x1: f32, y: f32) {
        let mut x = x0;
        while x < x1 - 1.0 {
            let w = 26.0_f32.min(x1 - x);
            runs.push(run("word", x, y, w, 7.0));
            x += w + 3.5;
        }
    }

    /// `count` lines of a column from baseline `top` down.
    fn column(runs: &mut Vec<TextRun>, (x0, x1): (f32, f32), top: f32, count: usize) {
        for line in 0..count {
            words(runs, x0, x1, top - 12.0 * line as f32);
        }
    }

    fn two_columns(top: f32, count: usize) -> Vec<TextRun> {
        let mut runs = Vec::new();
        column(&mut runs, LEFT, top, count);
        column(&mut runs, RIGHT, top, count);
        runs
    }

    fn crop(runs: &[TextRun], x: Option<f32>, y: f32) -> [f32; 4] {
        crop_rect(&CropInput {
            page_w_pt: PAGE_W,
            page_h_pt: PAGE_H,
            dest_x: x,
            dest_y: Some(y),
            runs,
        })
    }

    fn in_left_column(rect: [f32; 4]) -> bool {
        rect[0] >= LEFT.0 - 6.0 && rect[0] + rect[2] <= LEFT.1 + 6.0
    }

    fn in_right_column(rect: [f32; 4]) -> bool {
        rect[0] >= RIGHT.0 - 6.0 && rect[0] + rect[2] <= RIGHT.1 + 6.0
    }

    fn full_width(rect: [f32; 4]) -> bool {
        rect[0] <= LEFT.0 && rect[0] + rect[2] >= RIGHT.1
    }

    /// The crop holds the line on baseline `y` from `x0` to `x1`.
    fn holds(rect: [f32; 4], x0: f32, x1: f32, y: f32) -> bool {
        rect[0] <= x0 && rect[0] + rect[2] >= x1 && rect[1] <= y && rect[1] + rect[3] >= y + 7.0
    }

    #[test]
    fn columns_are_read_from_word_runs() {
        let columns = page_columns(&two_columns(740.0, 50), PAGE_W);
        assert_eq!(columns.len(), 2, "{columns:?}");
        assert!((columns[0].start - LEFT.0).abs() < 1.0 && (columns[0].end - LEFT.1).abs() < 1.0);
        assert!((columns[1].start - RIGHT.0).abs() < 1.0 && (columns[1].end - RIGHT.1).abs() < 1.0);
    }

    #[test]
    fn a_reference_anchored_left_of_its_text_previews_its_own_column() {
        // Elsevier anchors a reference at x 37.0, a hair left of its label:
        // the clusters around the anchor held no x, and the fallback took
        // the right-hand column.
        let runs = two_columns(740.0, 50);
        let rect = crop(&runs, Some(37.0), 671.0);
        assert!(in_left_column(rect), "{rect:?}");
        assert!(holds(rect, LEFT.0, LEFT.1, 668.0), "{rect:?}");
    }

    #[test]
    fn an_equation_anchored_at_its_tag_previews_the_tags_column() {
        // Some generators anchor an equation at its number, at the column's
        // right edge beside the gutter.
        let runs = two_columns(740.0, 50);
        assert!(in_left_column(crop(&runs, Some(288.7), 415.0)));
        assert!(in_right_column(crop(&runs, Some(557.7), 305.0)));
    }

    #[test]
    fn a_full_width_abstract_above_does_not_merge_the_columns() {
        // Page one: title and abstract across the page, the columns below.
        // Every full-width line in the 400pt under an anchor used to bridge
        // the gutter and fuse the two columns into one crop.
        let mut runs = Vec::new();
        for line in 0..14 {
            words(&mut runs, LEFT.0, RIGHT.1, 740.0 - 12.0 * line as f32);
        }
        column(&mut runs, LEFT, 560.0, 36);
        column(&mut runs, RIGHT, 560.0, 36);
        let rect = crop(&runs, Some(RIGHT.0), 570.0);
        assert!(in_right_column(rect), "{rect:?}");
        assert!(holds(rect, RIGHT.0, RIGHT.1, 560.0), "{rect:?}");
        let rect = crop(&runs, Some(LEFT.0), 450.0);
        assert!(in_left_column(rect), "{rect:?}");
    }

    #[test]
    fn a_full_width_caption_previews_full_width() {
        // A `figure*` at the top: artwork (no text), then a two-line caption
        // across both columns, then the columns. The anchor sits just over
        // the caption.
        let mut runs = Vec::new();
        words(&mut runs, LEFT.0, RIGHT.1, 480.0);
        words(&mut runs, LEFT.0, 200.0, 468.0);
        column(&mut runs, LEFT, 440.0, 30);
        column(&mut runs, RIGHT, 440.0, 30);
        let rect = crop(&runs, Some(LEFT.0), 490.0);
        assert!(full_width(rect), "{rect:?}");
        assert!(holds(rect, LEFT.0, RIGHT.1, 480.0), "{rect:?}");
    }

    #[test]
    fn an_anchor_over_a_floats_artwork_reaches_its_caption() {
        // hypcap anchors a float at its top: under it, narrow axis labels in
        // both halves, and only then the full-width caption.
        let mut runs = Vec::new();
        for y in [700.0, 640.0, 580.0] {
            runs.push(run("0.5", 60.0, y, 12.0, 6.0));
            runs.push(run("0.5", 330.0, y, 12.0, 6.0));
        }
        runs.push(run("time (s)", 140.0, 540.0, 30.0, 6.0));
        runs.push(run("energy", 420.0, 540.0, 30.0, 6.0));
        words(&mut runs, LEFT.0, RIGHT.1, 520.0);
        column(&mut runs, LEFT, 490.0, 34);
        column(&mut runs, RIGHT, 490.0, 34);
        let rect = crop(&runs, Some(LEFT.0), 730.0);
        assert!(full_width(rect), "{rect:?}");
    }

    #[test]
    fn column_text_stops_short_of_a_full_width_float_below() {
        // The columns run down to a `table*` at the foot of the page; a
        // destination near the end of the left column must not take a
        // column-wide slice of the table.
        let mut runs = Vec::new();
        column(&mut runs, LEFT, 740.0, 30);
        column(&mut runs, RIGHT, 740.0, 30);
        words(&mut runs, LEFT.0, RIGHT.1, 368.0);
        for line in 0..6 {
            words(
                &mut runs,
                LEFT.0 + 20.0,
                RIGHT.1 - 20.0,
                352.0 - 12.0 * line as f32,
            );
        }
        let rect = crop(&runs, Some(LEFT.0), 430.0);
        assert!(in_left_column(rect), "{rect:?}");
        assert!(rect[1] > 368.0 + 7.0, "took a slice of the table: {rect:?}");
        assert!(holds(rect, LEFT.0, LEFT.1, 392.0), "{rect:?}");
    }

    #[test]
    fn an_overfull_display_is_not_joined_to_the_next_column() {
        // A display overrunning its column ends 4pt short of the right-hand
        // column's line on the same baseline; tall delimiters make its run
        // taller than the words beside it.
        let mut runs = two_columns(740.0, 50);
        runs.retain(|r| !(r.x < 300.0 && (r.y - 500.0).abs() < 0.5));
        runs.push(run("display", 70.0, 496.0, 232.6, 16.0));
        let rect = crop(&runs, Some(LEFT.0), 512.0);
        assert!(in_left_column(rect), "{rect:?}");
    }

    #[test]
    fn a_mark_in_the_gutter_widens_neither_column() {
        let mut runs = two_columns(740.0, 50);
        runs.push(run("2", 296.0, 600.0, 4.0, 5.0));
        let columns = page_columns(&runs, PAGE_W);
        assert_eq!(columns.len(), 2);
        assert!(
            columns[0].end < 290.0 && columns[1].start > 305.0,
            "{columns:?}"
        );
    }

    #[test]
    fn a_destination_without_x_takes_the_column_under_it() {
        // `/FitH`: a y only. The left column has ended higher up; the line
        // under the anchor is in the right-hand column.
        let mut runs = Vec::new();
        column(&mut runs, LEFT, 740.0, 20);
        column(&mut runs, RIGHT, 740.0, 50);
        let rect = crop(&runs, None, 300.0);
        assert!(in_right_column(rect), "{rect:?}");
    }

    #[test]
    fn margin_equation_numbers_are_not_a_column() {
        // A single-column page whose displays carry their numbers at the
        // right margin: a strip of aligned runs is no column.
        let mut runs = Vec::new();
        for line in 0..50 {
            let y = 740.0 - 12.0 * line as f32;
            if line % 6 == 3 {
                runs.push(run("a = b", 250.0, y, 80.0, 7.0));
                runs.push(run("(1)", 525.0, y, 15.0, 7.0));
            } else {
                words(&mut runs, 72.0, 540.0, y);
            }
        }
        assert_eq!(page_columns(&runs, 612.0).len(), 1);
    }

    #[test]
    fn a_table_of_aligned_cells_is_not_two_columns() {
        let mut runs = Vec::new();
        for line in 0..40 {
            let y = 740.0 - 12.0 * line as f32;
            for x in [80.0, 200.0, 330.0, 450.0] {
                runs.push(run("0.123", x, y, 30.0, 7.0));
            }
        }
        assert_eq!(page_columns(&runs, 612.0).len(), 1);
    }
}

//! A float's extent on its page: the artwork (or table body) and its whole
//! caption, and nothing else - not the end of the previous float's caption
//! above it, not the prose around it.
//!
//! A link to a figure lands either just over its caption (hyperref's
//! default) or at the top of the float (hypcap). The ordinary preview crop
//! anchors there and runs a fixed height down, which shows the wrong part
//! of a float either way: the end of whatever sits above, and the top of
//! the artwork cut off before its caption. Here the float is read from the
//! page text instead: captions are found by their "Figure 11." / "Table 3:"
//! heads, each grown over its continuation lines, and the artwork is the
//! space on the caption's artwork side up to the nearest prose line or
//! other caption.
//!
//! PDFium-free, like `preview` and `figures`.

use super::figures::parse_caption;
use super::preview::{column_bounds, page_columns, segments, Interval};
use super::types::TextRun;

/// A row at least this share of its column wide is prose, not part of an
/// artwork (axis labels, legends, sub-captions, table cells).
const PROSE_RATIO: f32 = 0.55;
/// Rows of a float's artwork stay narrower than this share of the column;
/// anything wider between a destination and a caption is text, and the
/// destination is not the float's top.
const ARTWORK_ROW_RATIO: f32 = 0.45;
/// Runs closer than this along a row are one stretch of the caption's line.
const JOIN_GAP_PT: f32 = 6.5;
const PADDING_PT: f32 = 4.0;
/// Least room on a side of a caption for it to hold the artwork.
const MIN_ARTWORK_PT: f32 = 15.0;

/// A caption: its first line and the lines continuing it.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionBlock {
    /// "Figure 11", "Table 3", as printed.
    pub label: String,
    /// A table, algorithm or listing: set under its caption by default.
    pub table_like: bool,
    /// Left edge of the caption's first line.
    pub x: f32,
    pub x0: f32,
    pub x1: f32,
    /// Top of the first line, bottom of the last; y-up.
    pub top: f32,
    pub bottom: f32,
    /// Height of the first line.
    pub line_h: f32,
}

impl CaptionBlock {
    fn overlaps(&self, row: &TextRun) -> bool {
        let vertical = (row.y + row.h).min(self.top) - row.y.max(self.bottom);
        let horizontal = (row.x + row.w).min(self.x1) - row.x.max(self.x0);
        vertical > 0.3 * row.h.min(self.line_h) && horizontal > 1.0
    }
}

fn same_row(a: &TextRun, b: &TextRun) -> bool {
    let overlap = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    overlap >= 0.5 * a.h.min(b.h)
}

/// The text of the line starting at `runs[first]`: it and the runs that
/// follow it along its row, as far as `limit` characters.
fn line_text(runs: &[TextRun], first: usize, limit: usize) -> String {
    let mut text = runs[first].text.trim().to_string();
    let mut end = runs[first].x + runs[first].w;
    while text.chars().count() < limit {
        let next = runs
            .iter()
            .filter(|r| {
                let gap = r.x - end;
                (-1.0..=JOIN_GAP_PT).contains(&gap)
                    && same_row(r, &runs[first])
                    && !r.text.trim().is_empty()
            })
            .min_by(|a, b| a.x.total_cmp(&b.x));
        let Some(next) = next else { break };
        text.push(' ');
        text.push_str(next.text.trim());
        end = next.x + next.w;
    }
    text
}

/// Every caption on a page, read from its runs.
pub fn caption_blocks(runs: &[TextRun]) -> Vec<CaptionBlock> {
    let rows = segments(runs);
    let columns = page_columns(runs, f32::MAX);
    let mut heads: Vec<(String, usize)> = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let lower = run.text.trim_start().to_lowercase();
        let opens = ["fig", "table", "tab.", "algorithm", "alg.", "listing"]
            .iter()
            .any(|w| lower.starts_with(w));
        if !opens || run.w <= 0.0 || run.h <= 0.0 {
            continue;
        }
        // The first thing on its line: nothing just before it on the row.
        let starts_line = !runs.iter().any(|r| {
            let gap = run.x - (r.x + r.w);
            (-1.0..=JOIN_GAP_PT).contains(&gap) && r.x < run.x && same_row(r, run)
        });
        if !starts_line {
            continue;
        }
        if let Some((label, _)) = parse_caption(&line_text(runs, i, 40)) {
            heads.push((label, i));
        }
    }
    let is_head = |row: &TextRun| {
        heads
            .iter()
            .any(|&(_, i)| same_row(row, &runs[i]) && (row.x - runs[i].x).abs() < 1.0)
    };
    let mut out = Vec::new();
    for (label, i) in &heads {
        let run = &runs[*i];
        // The caption's first line, whole: a justified caption's word
        // spaces can be wider than a row's joins, though never a gutter.
        let mut head = TextRun {
            text: String::new(),
            start: run.start,
            x: run.x,
            y: run.y,
            w: run.w,
            h: run.h,
            edges: Vec::new(),
            math: Vec::new(),
        };
        loop {
            let end = head.x + head.w;
            let reach = 2.0 * head.h.max(run.h);
            let next = runs
                .iter()
                .filter(|r| {
                    let gap = r.x - end;
                    (-1.0..=reach).contains(&gap)
                        && r.x + r.w > end
                        && same_row(r, run)
                        && !columns
                            .windows(2)
                            .any(|pair| end <= pair[0].end + 1.0 && r.x >= pair[1].start - 1.0)
                })
                .min_by(|a, b| a.x.total_cmp(&b.x));
            let Some(next) = next else { break };
            let y0 = head.y.min(next.y);
            let y1 = (head.y + head.h).max(next.y + next.h);
            head.w = next.x + next.w - head.x;
            head.y = y0;
            head.h = y1 - y0;
        }
        let head = &head;
        let h = head.h.max(1.0);
        let mut block = CaptionBlock {
            label: label.clone(),
            table_like: !label.to_lowercase().starts_with("fig"),
            x: run.x,
            x0: head.x,
            x1: head.x + head.w,
            top: head.y + head.h,
            bottom: head.y,
            line_h: h,
        };
        let span_end = head.x + head.w;
        let mut previous = head;
        for _ in 0..20 {
            let next = rows
                .iter()
                .filter(|row| {
                    let top = row.y + row.h;
                    top <= previous.y + 0.5 * h
                        && previous.y - top <= 1.1 * h
                        // Under the head, or centred under it and wider (IEEE
                        // sets "TABLE I" over a centred line of its title).
                        && (row.x >= run.x - 2.0 * h
                            || (row.x + row.w / 2.0 - (head.x + head.w / 2.0)).abs() <= 2.0 * h)
                        && row.x < span_end - 1.0
                        && (row.h - h).abs() <= 0.5 * h
                        && !is_head(row)
                        // A line of the caption runs most of its width, or
                        // starts at its left edge, or is centred under it;
                        // a table's first cells under its caption do none.
                        && (row.w >= 0.5 * head.w
                            || (row.x - run.x).abs() <= 2.0 * h
                            || (row.x + row.w / 2.0 - (head.x + head.w / 2.0)).abs() <= 2.0 * h)
                })
                .max_by(|a, b| (a.y + a.h).total_cmp(&(b.y + b.h)));
            let Some(next) = next else { break };
            block.x0 = block.x0.min(next.x);
            block.x1 = block.x1.max(next.x + next.w);
            block.bottom = block.bottom.min(next.y);
            previous = next;
        }
        out.push(block);
    }
    out
}

struct Page<'a> {
    runs: &'a [TextRun],
    rows: Vec<TextRun>,
    blocks: Vec<CaptionBlock>,
    page_w: f32,
    page_h: f32,
}

impl Page<'_> {
    fn new(runs: &[TextRun], page_w: f32, page_h: f32) -> Page<'_> {
        Page {
            runs,
            rows: segments(runs),
            blocks: caption_blocks(runs),
            page_w: page_w.max(1.0),
            page_h: page_h.max(1.0),
        }
    }

    /// The column a caption's float sits in: the caption's own, or all of
    /// the text width for a full-width float, and never narrower than the
    /// caption itself.
    fn area(&self, block: &CaptionBlock) -> Interval {
        // A single column is all of the page's text width; on a page of
        // columns, the caption's column, or all of them for a full-width
        // float.
        let columns = page_columns(self.runs, self.page_w);
        let column = match columns.as_slice() {
            [one] => *one,
            _ => column_bounds(self.runs, self.page_w, block.top + 0.5, Some(block.x)),
        };
        let mut area = Interval {
            start: column.start.min(block.x0),
            end: column.end.max(block.x1),
        };
        // Floats side by side (two minipages): each keeps its own half,
        // split midway between the captions.
        for other in &self.blocks {
            let level = other.top.min(block.top) - other.bottom.max(block.bottom) > 0.0;
            if other == block || !level {
                continue;
            }
            if other.x0 >= block.x1 {
                area.end = area.end.min((block.x1 + other.x0) / 2.0);
            } else if other.x1 <= block.x0 {
                area.start = area.start.max((other.x1 + block.x0) / 2.0);
            }
        }
        area
    }

    /// The words of a row, from the runs in it.
    fn row_text(&self, row: &TextRun) -> String {
        let text: Vec<&str> = self
            .runs
            .iter()
            .filter(|r| same_row(r, row) && r.x >= row.x - 0.5 && r.x + r.w <= row.x + row.w + 0.5)
            .map(|r| r.text.as_str())
            .collect();
        text.join(" ")
    }

    /// A numbered heading: "3 Method", "4.2. Results", "M. Encoder details".
    fn is_heading(&self, row: &TextRun) -> bool {
        let text = self.row_text(row);
        let mut parts = text.split_whitespace();
        let (Some(number), Some(word)) = (parts.next(), parts.next()) else {
            return false;
        };
        let number = number.trim_end_matches('.');
        let numbered = !number.is_empty()
            && number.split('.').all(|piece| {
                !piece.is_empty()
                    && (piece.chars().all(|c| c.is_ascii_digit())
                        || (piece.len() == 1 && piece.chars().all(|c| c.is_ascii_uppercase())))
            });
        numbered
            && word.starts_with(|c: char| c.is_uppercase())
            && word.chars().filter(|c| c.is_alphabetic()).count() >= 3
    }

    /// A line of running text, read from its words: at least `ratio` of
    /// the column wide, and `words` words or more rather than a table row's
    /// numbers or a diagram's labels.
    fn is_prose(&self, row: &TextRun, area: Interval, ratio: f32, words: usize) -> bool {
        if row.w < ratio * (area.end - area.start) {
            return false;
        }
        let text = self.row_text(row);
        let found = text
            .split_whitespace()
            .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 3)
            .count();
        let digits = text.chars().filter(char::is_ascii_digit).count();
        found >= words && digits * 5 < text.chars().count()
    }

    /// How far a table runs from its caption, `up` or down: row after row
    /// while the gaps stay a table's - a float is set apart from the text
    /// around it by more than any gap between its own rows - and until
    /// another caption. Its rows can be as wide as prose, so width says
    /// nothing. The far edge of the last row, if there is one.
    fn table_extent(&self, block: &CaptionBlock, area: Interval, up: bool) -> Option<f32> {
        let mut rows: Vec<&TextRun> = self
            .rows_in(area)
            .filter(|r| {
                !block.overlaps(r)
                    && if up {
                        r.y >= block.top - 0.5
                    } else {
                        r.y + r.h <= block.bottom + 0.5
                    }
            })
            .collect();
        if up {
            rows.sort_by(|a, b| a.y.total_cmp(&b.y));
        } else {
            rows.sort_by(|a, b| (b.y + b.h).total_cmp(&(a.y + a.h)));
        }
        let h = block.line_h;
        let mut edge = if up { block.top } else { block.bottom };
        let mut reached = None;
        for (i, row) in rows.into_iter().enumerate() {
            let gap = if up {
                row.y - edge
            } else {
                edge - (row.y + row.h)
            };
            // The caption's own skip and a rule can open the first gap wider.
            let allowed = if i == 0 {
                (3.5 * h).max(20.0)
            } else {
                (1.6 * h).max(10.0)
            };
            if gap > allowed
                || self.in_other_caption(block, row)
                || self.is_prose(row, area, 0.7, 8)
            {
                break;
            }
            edge = if up {
                edge.max(row.y + row.h)
            } else {
                edge.min(row.y)
            };
            reached = Some(edge);
        }
        reached
    }

    fn rows_in<'s>(&'s self, area: Interval) -> impl Iterator<Item = &'s TextRun> + 's {
        self.rows.iter().filter(move |r| {
            r.w > 0.0 && r.h > 0.0 && r.x + r.w > area.start + 1.0 && r.x < area.end - 1.0
        })
    }

    fn in_other_caption(&self, block: &CaptionBlock, row: &TextRun) -> bool {
        self.blocks
            .iter()
            .any(|other| other != block && other.overlaps(row))
    }

    /// Whether a row ends the float: prose, or another float's caption.
    fn boundary(&self, block: &CaptionBlock, area: Interval, row: &TextRun) -> bool {
        !block.overlaps(row)
            && (self.is_prose(row, area, PROSE_RATIO, 5) || self.in_other_caption(block, row))
    }

    /// The bottom of the nearest boundary row above the caption.
    fn ceiling(&self, block: &CaptionBlock, area: Interval) -> f32 {
        let limit = self.page_h.min(block.top + 0.75 * self.page_h);
        let boundary = self
            .rows_in(area)
            .filter(|r| r.y >= block.top - 0.5 && self.boundary(block, area, r))
            .map(|r| r.y)
            .reduce(f32::min);
        if let Some(boundary) = boundary {
            return boundary.min(limit);
        }
        // Nothing above but artwork, up to the page's top margin - where
        // the artwork ends in an image is unknowable from text, but the
        // running head in the margin is no part of it.
        (0.94 * self.page_h).max(block.top).min(limit)
    }

    /// The top of the nearest boundary row below the caption.
    fn floor(&self, block: &CaptionBlock, area: Interval) -> f32 {
        self.rows_in(area)
            .filter(|r| r.y + r.h <= block.bottom + 0.5 && self.boundary(block, area, r))
            .map(|r| r.y + r.h)
            .fold(0.0_f32.max(block.bottom - 0.75 * self.page_h), f32::max)
    }

    /// The float of `block`, artwork side chosen from the room either side
    /// of its caption, unless `top` already says where the float begins.
    fn float(&self, block: &CaptionBlock, top: Option<f32>) -> [f32; 4] {
        let area = self.area(block);
        let ceiling = self.ceiling(block, area);
        let floor = self.floor(block, area);
        let (y0, y1) = match top {
            Some(top) => (block.bottom, top.min(ceiling).max(block.top)),
            None if block.table_like => {
                match (
                    self.table_extent(block, area, false),
                    self.table_extent(block, area, true),
                ) {
                    (Some(bottom), _) if block.bottom - bottom >= MIN_ARTWORK_PT => {
                        (bottom, block.top)
                    }
                    (_, Some(top)) if top - block.top >= MIN_ARTWORK_PT => (block.bottom, top),
                    _ => (block.bottom, block.top),
                }
            }
            None => {
                let above = ceiling - block.top;
                let below = block.bottom - floor;
                let artwork_below = if block.table_like {
                    below >= MIN_ARTWORK_PT
                } else {
                    above < MIN_ARTWORK_PT && below >= MIN_ARTWORK_PT
                };
                if artwork_below {
                    (floor, block.top)
                } else {
                    (block.bottom, ceiling)
                }
            }
        };
        let x0 = (area.start - PADDING_PT).max(0.0);
        let x1 = (area.end + PADDING_PT).min(self.page_w);
        // Padding, but never onto a line outside the float.
        let under = self
            .rows_in(area)
            .filter(|r| r.y + r.h <= y0 + 0.5)
            .map(|r| r.y + r.h)
            .fold(floor, f32::max);
        let over = self
            .rows_in(area)
            .filter(|r| r.y >= y1 - 0.5)
            .map(|r| r.y)
            .fold(ceiling, f32::min);
        let y0 = (y0 - PADDING_PT).max(under + 0.5).min(y0).max(0.0);
        let y1 = (y1 + PADDING_PT).min(over - 0.5).max(y1).min(self.page_h);
        [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
    }

    /// Median height of the page's prose rows: a heading is set larger.
    fn body_h(&self) -> f32 {
        let mut heights: Vec<f32> = self
            .rows
            .iter()
            .filter(|r| r.w >= 0.3 * self.page_w)
            .map(|r| r.h)
            .collect();
        heights.sort_by(f32::total_cmp);
        heights.get(heights.len() / 2).copied().unwrap_or(10.0)
    }
}

/// The float a link destination at (`dest_x`, `anchor_y`) points at, when
/// it points at one: the anchor sits just over a caption, or over a float's
/// artwork with nothing but artwork between it and the caption.
pub fn float_for_anchor(
    runs: &[TextRun],
    page_w_pt: f32,
    page_h_pt: f32,
    dest_x: Option<f32>,
    anchor_y: f32,
) -> Option<[f32; 4]> {
    let page = Page::new(runs, page_w_pt, page_h_pt);
    if page.blocks.is_empty() {
        return None;
    }
    let column = column_bounds(runs, page.page_w, anchor_y, dest_x);
    // In the destination's column (or across it, for a full-width float).
    let under = |b: &CaptionBlock| (b.x1.min(column.end) - b.x0.max(column.start)) > 0.0;
    // hyperref's own anchor: a line's height over the caption.
    if let Some(block) = page
        .blocks
        .iter()
        .filter(|b| {
            // Over the caption, or (revtex) on its first line.
            let over = b.top >= anchor_y - 16.0 && b.top <= anchor_y + 3.0;
            let on = anchor_y <= b.top + 1.0 && anchor_y >= b.bottom;
            under(b) && (over || on)
        })
        .min_by(|a, b| {
            (a.top - anchor_y)
                .abs()
                .total_cmp(&(b.top - anchor_y).abs())
        })
    {
        return Some(page.float(block, None));
    }
    // hypcap's: the float's top, artwork and then the caption below it.
    // Floats side by side share a top; the one under the anchor's x.
    let below: Vec<&CaptionBlock> = page
        .blocks
        .iter()
        .filter(|b| under(b) && b.top < anchor_y && anchor_y - b.top <= 0.8 * page.page_h)
        .collect();
    let highest = below.iter().map(|b| b.top).reduce(f32::max)?;
    let off = |b: &CaptionBlock| dest_x.map_or(0.0, |x| (b.x0 - x).max(x - b.x1).max(0.0));
    let block = *below
        .iter()
        .filter(|b| b.top >= highest - 3.0)
        .min_by(|a, b| off(a).total_cmp(&off(b)))?;
    let area = page.area(block);
    let body_h = page.body_h();
    // Text between the anchor and the caption - prose, another caption, a
    // heading (larger than the text at the column's margin, or numbered
    // as one at the anchor) - says the anchor is not the float's top.
    let text_between = page
        .rows_in(area)
        .filter(|r| r.y + r.h <= anchor_y + 2.0 && r.y >= block.top)
        .any(|r| {
            let at_margin = (r.x - area.start).abs() < 3.0;
            let at_anchor = dest_x.is_some_and(|x| (r.x - x).abs() < 3.0);
            page.is_prose(r, area, ARTWORK_ROW_RATIO, 5)
                || page.in_other_caption(block, r)
                || (r.h >= 1.15 * body_h && at_margin)
                || (page.is_heading(r) && (at_margin || at_anchor))
        });
    // A table's caption usually heads it: the float's top is the caption,
    // and the table runs on below.
    let top = (!block.table_like).then_some(anchor_y + 2.0);
    (!text_between).then(|| page.float(block, top))
}

/// Every float on a page, each `[x, y, w, h]` y-up: its artwork (or table
/// body) and its caption. What lies inside one is no part of the running
/// text: an axis label set as `$\tau(s)$`, a table cell, an algorithm's
/// line.
pub fn float_regions(runs: &[TextRun], page_w_pt: f32, page_h_pt: f32) -> Vec<[f32; 4]> {
    let page = Page::new(runs, page_w_pt, page_h_pt);
    page.blocks.iter().map(|b| page.float(b, None)).collect()
}

/// The float of the caption whose first line starts at (`x`, `baseline`).
pub fn float_for_caption(
    runs: &[TextRun],
    page_w_pt: f32,
    page_h_pt: f32,
    x: f32,
    baseline: f32,
) -> Option<[f32; 4]> {
    let page = Page::new(runs, page_w_pt, page_h_pt);
    let block = page.blocks.iter().find(|b| {
        (b.x - x).abs() <= 3.0 && baseline >= b.top - b.line_h - 2.0 && baseline <= b.top + 1.0
    })?;
    Some(page.float(block, None))
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

    /// A line of prose from `x0` to `x1` on `y`, as word runs.
    fn prose(runs: &mut Vec<TextRun>, x0: f32, x1: f32, y: f32) {
        let mut x = x0;
        while x < x1 - 1.0 {
            let w = 26.0_f32.min(x1 - x);
            runs.push(run("word", x, y, w, 8.0));
            x += w + 3.5;
        }
    }

    /// A caption: "Figure N." then words, `lines` lines from `y` down.
    fn caption(runs: &mut Vec<TextRun>, label: &str, x0: f32, x1: f32, y: f32, lines: usize) {
        let mut parts = label.split(' ');
        let word = parts.next().unwrap_or("Figure");
        let number = parts.next().unwrap_or("1.");
        runs.push(run(word, x0, y, 23.0, 8.0));
        runs.push(run(number, x0 + 27.0, y, 10.0, 8.0));
        prose(runs, x0 + 41.0, x1, y);
        for line in 1..lines {
            prose(
                runs,
                x0,
                if line + 1 == lines { x0 + 200.0 } else { x1 },
                y - 11.0 * line as f32,
            );
        }
    }

    fn contains(rect: [f32; 4], y0: f32, y1: f32) -> bool {
        rect[1] <= y0 && rect[1] + rect[3] >= y1
    }

    /// The page the user met: a single-column page with the end of Figure
    /// 10's caption, then Figure 11's panels (labels only), its caption,
    /// then Table 3 and prose.
    fn figure_page() -> Vec<TextRun> {
        let mut runs = Vec::new();
        for y in [740.0, 728.0, 716.0, 704.0] {
            prose(&mut runs, 58.0, 553.0, y);
        }
        caption(&mut runs, "Figure 10.", 58.6, 553.0, 556.7, 4);
        runs.push(run("(a) teachers", 147.5, 501.2, 31.7, 6.0));
        runs.push(run("(b) DMA + DMB", 232.2, 501.2, 49.9, 6.0));
        runs.push(run("pA(x|c)", 132.1, 486.3, 23.1, 5.4));
        runs.push(run("3.5", 172.1, 410.2, 8.0, 4.9));
        runs.push(run("3.5", 266.2, 410.2, 8.0, 4.9));
        caption(&mut runs, "Figure 11.", 58.6, 553.0, 386.8, 2);
        caption(&mut runs, "Table 3.", 58.7, 294.0, 346.8, 3);
        runs.push(run("look", 166.5, 302.6, 12.3, 4.8));
        runs.push(run("teacher A", 61.9, 289.6, 26.7, 4.8));
        runs.push(run("0.00", 166.6, 289.6, 11.9, 4.8));
        runs.push(run("DMA", 61.9, 272.7, 15.9, 5.6));
        for line in 0..10 {
            prose(&mut runs, 58.7, 294.3, 200.9 - 11.0 * line as f32);
            prose(&mut runs, 317.0, 553.0, 346.0 - 11.0 * line as f32);
        }
        runs
    }

    #[test]
    fn reads_caption_blocks_with_their_continuation_lines() {
        let blocks = caption_blocks(&figure_page());
        let labels: Vec<&str> = blocks.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Figure 10", "Figure 11", "Table 3"]);
        assert!(
            (blocks[0].bottom - (556.7 - 33.0)).abs() < 1.0,
            "{:?}",
            blocks[0]
        );
        assert!(
            (blocks[1].bottom - (386.8 - 11.0)).abs() < 1.0,
            "{:?}",
            blocks[1]
        );
        assert!(blocks[2].table_like);
    }

    #[test]
    fn a_float_top_anchor_previews_the_artwork_and_its_caption_only() {
        // hypcap's anchor sits at y 517, just under Figure 10's caption.
        let rect =
            float_for_anchor(&figure_page(), 612.0, 792.0, Some(58.5), 517.0).expect("a float");
        assert!(
            contains(rect, 375.8, 507.2),
            "artwork and caption: {rect:?}"
        );
        assert!(
            rect[1] + rect[3] < 523.9 + 0.5,
            "the previous caption: {rect:?}"
        );
        assert!(rect[1] > 346.8 + 8.0, "the table below: {rect:?}");
    }

    #[test]
    fn a_caption_anchor_previews_the_artwork_above_it() {
        // hyperref without hypcap: the anchor is a line over the caption.
        let rect =
            float_for_anchor(&figure_page(), 612.0, 792.0, Some(58.5), 398.0).expect("a float");
        assert!(contains(rect, 375.8, 507.2), "{rect:?}");
        assert!(rect[1] + rect[3] < 523.9 + 0.5, "{rect:?}");
    }

    #[test]
    fn a_table_caption_previews_the_table_under_it() {
        let rect =
            float_for_anchor(&figure_page(), 612.0, 792.0, Some(58.7), 358.0).expect("a float");
        assert!(contains(rect, 272.7, 354.8), "caption and body: {rect:?}");
        assert!(rect[1] > 200.9 + 8.0 - 0.5, "the prose under it: {rect:?}");
        assert!(rect[0] + rect[2] < 300.0, "its column only: {rect:?}");
        assert!(rect[1] + rect[3] < 375.8, "Figure 11's caption: {rect:?}");
    }

    #[test]
    fn a_prose_destination_is_not_a_float() {
        assert_eq!(
            float_for_anchor(&figure_page(), 612.0, 792.0, Some(58.0), 750.0),
            None
        );
    }

    #[test]
    fn a_caption_found_from_its_line_previews_its_float() {
        let rect = float_for_caption(&figure_page(), 612.0, 792.0, 58.6, 386.8).expect("float");
        assert!(contains(rect, 375.8, 507.2), "{rect:?}");
        assert!(rect[1] + rect[3] < 523.9 + 0.5, "{rect:?}");
    }

    #[test]
    fn an_anchor_on_the_captions_first_line_previews_its_float() {
        // revtex sets the anchor at the caption's baseline.
        let rect =
            float_for_anchor(&figure_page(), 612.0, 792.0, Some(58.5), 387.0).expect("a float");
        assert!(contains(rect, 375.8, 507.2), "{rect:?}");
    }

    #[test]
    fn a_heading_over_a_float_is_the_headings_destination() {
        // A section opening straight onto a figure: its anchor is over the
        // heading, not the float.
        let mut runs = Vec::new();
        runs.push(run("M.", 58.5, 700.0, 9.0, 8.0));
        runs.push(run("Encoder details", 71.0, 700.0, 70.0, 8.0));
        runs.push(run("0.5", 140.0, 650.0, 12.0, 6.0));
        caption(&mut runs, "Figure 3.", 58.5, 553.0, 600.0, 2);
        for line in 0..10 {
            prose(&mut runs, 58.5, 553.0, 560.0 - 12.0 * line as f32);
        }
        assert_eq!(
            float_for_anchor(&runs, 612.0, 792.0, Some(58.5), 710.0),
            None
        );
    }

    #[test]
    fn floats_side_by_side_each_keep_their_half() {
        // Two minipages under one anchor; the anchor's x picks the float.
        let mut runs = Vec::new();
        for y in [740.0, 728.0] {
            prose(&mut runs, 72.0, 540.0, y);
        }
        runs.push(run("0.5", 120.0, 650.0, 12.0, 6.0));
        runs.push(run("0.5", 380.0, 650.0, 12.0, 6.0));
        caption(&mut runs, "Figure 6:", 72.0, 290.0, 600.0, 1);
        caption(&mut runs, "Figure 7:", 322.0, 540.0, 600.0, 1);
        for line in 0..10 {
            prose(&mut runs, 72.0, 540.0, 560.0 - 12.0 * line as f32);
        }
        let left = float_for_anchor(&runs, 612.0, 792.0, Some(80.0), 715.0).expect("float");
        assert!(left[0] + left[2] < 322.0, "{left:?}");
        let right = float_for_anchor(&runs, 612.0, 792.0, Some(400.0), 715.0).expect("float");
        assert!(right[0] > 290.0, "{right:?}");
        assert!(contains(right, 600.0, 656.0), "{right:?}");
    }
    /// A table's runs: rows of cells from `top` down, `rows` of them.
    fn table(runs: &mut Vec<TextRun>, x0: f32, y: f32, rows: usize) {
        for row in 0..rows {
            let y = y - 13.0 * row as f32;
            runs.push(run("SGD", x0, y, 22.0, 8.0));
            for (i, cell) in ["12.4", "8.1", "5.3"].iter().enumerate() {
                runs.push(run(cell, x0 + 90.0 + 50.0 * i as f32, y, 18.0, 7.0));
            }
        }
    }

    #[test]
    fn a_tables_cells_are_no_line_of_its_caption() {
        // The first cells sit under the caption, but as cells: indented
        // from it and narrow. Taking them as its lines once let the table
        // run on over the display set after it.
        let mut runs = Vec::new();
        caption(&mut runs, "Table 1:", 159.0, 452.0, 710.0, 1);
        table(&mut runs, 198.0, 692.0, 5);
        runs.push(run(
            "A long equation is broken with multline:",
            72.0,
            585.0,
            199.0,
            10.0,
        ));
        runs.push(run(
            "J(theta) = E log D(x) + E log(1 - D(G(z)))",
            82.0,
            556.0,
            251.0,
            13.0,
        ));
        runs.push(run(
            "+ lambda E (grad D - 1)^2 + mu theta^2.",
            338.0,
            532.0,
            177.0,
            13.0,
        ));
        for line in 0..8 {
            prose(&mut runs, 72.0, 540.0, 500.0 - 12.0 * line as f32);
        }
        let blocks = caption_blocks(&runs);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].bottom > 700.0, "{:?}", blocks[0]);
        let regions = float_regions(&runs, 612.0, 792.0);
        assert_eq!(regions.len(), 1);
        let [_, y, _, _] = regions[0];
        assert!(y > 569.0, "the display under the table: {:?}", regions[0]);
    }

    #[test]
    fn an_ieee_table_title_centred_under_its_head_is_its_caption() {
        // "TABLE I" over a centred, wider line of small capitals, then the
        // table: the float runs over the table's rows.
        let mut runs = Vec::new();
        for y in [760.0, 748.0] {
            prose(&mut runs, 49.0, 300.0, y);
        }
        runs.push(run("TABLE I", 159.0, 733.0, 30.0, 5.0));
        runs.push(run(
            "TEST ERROR (%) OF EACH METHOD ON THREE BENCHMARKS.",
            70.0,
            723.0,
            209.0,
            7.0,
        ));
        table(&mut runs, 88.0, 705.0, 4);
        for line in 0..10 {
            prose(&mut runs, 49.0, 300.0, 620.0 - 12.0 * line as f32);
            prose(&mut runs, 312.0, 563.0, 760.0 - 12.0 * line as f32);
        }
        let regions = float_regions(&runs, 612.0, 792.0);
        assert_eq!(regions.len(), 1, "{regions:?}");
        assert!(contains(regions[0], 666.0, 738.0), "{:?}", regions[0]);
        assert!(
            regions[0][1] > 628.0,
            "the prose under it: {:?}",
            regions[0]
        );
    }
}

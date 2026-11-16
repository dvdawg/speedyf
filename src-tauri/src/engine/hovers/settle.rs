//! What every preview crop keeps, whatever found it: it never cuts through
//! a display equation - a display it touches is in it whole, limits,
//! fractions and number included - and never through a line of text - a
//! line it touches is in it over its whole width, or left out cleanly.
//! The readers that find a target (a statement, a section's first lines, a
//! reference, a footnote, a display) decide where it starts and ends;
//! `settle` then makes the crop around it whole, so a reader's slip shows
//! as a little more or less of the page, never as half an equation.
//!
//! The page's displays come from the display detector
//! (`glyphs::page_equation_blocks`, through `Page::set_displays`); a page
//! read without them is settled by its lines alone.
//!
//! PDFium-free.

use super::page::{Line, Page};
use super::Rect;

/// How far a box reaches into a crop: its horizontal overlap in points,
/// and the share of its height inside.
fn reach(x0: f32, x1: f32, bottom: f32, top: f32, r: &Edges) -> (f32, f32) {
    let ox = x1.min(r.x1) - x0.max(r.x0);
    let oy = (top.min(r.y1) - bottom.max(r.y0)).max(0.0);
    (ox, oy / (top - bottom).max(0.5))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Edges {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Edges {
    fn of(rect: Rect) -> Edges {
        Edges {
            x0: rect[0],
            y0: rect[1],
            x1: rect[0] + rect[2],
            y1: rect[1] + rect[3],
        }
    }

    fn take(&mut self, x0: f32, x1: f32, bottom: f32, top: f32) {
        self.x0 = self.x0.min(x0);
        self.x1 = self.x1.max(x1);
        self.y0 = self.y0.min(bottom);
        self.y1 = self.y1.max(top);
    }

    fn holds(&self, x0: f32, x1: f32, bottom: f32, top: f32) -> bool {
        x0 >= self.x0 - 1.0
            && x1 <= self.x1 + 1.0
            && bottom >= self.y0 - 1.0
            && top <= self.y1 + 1.0
    }
}

/// A line counts: not a running head or foot, and not empty.
fn counts(l: &Line) -> bool {
    !l.flags.furniture && l.end > l.start
}

/// Whether `l` is the rest of `h`'s row: level with it (sharing most of
/// the lower one's height - a footnote's raised mark is level with its
/// text) and set on close beside it, not across a gutter.
fn same_row(h: &Line, l: &Line) -> bool {
    let shared = h.top.min(l.top) - h.bottom.max(l.bottom);
    let level = shared >= 0.5 * h.height().min(l.height());
    let gap = (l.x0 - h.x1).max(h.x0 - l.x1);
    level && gap < 0.8 * h.em().min(l.em())
}

/// Whether line `i` is a mark set on another line's letters - an accent,
/// a dot, a hat the page's text gives a line of its own - rather than a
/// line: a character or two, all combining marks or under three points
/// tall. It goes with the line it sits on; it never decides a crop alone.
pub fn is_mark(page: &Page, i: usize) -> bool {
    let l = &page.lines[i];
    let text = page.line_text(i);
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    !chars.is_empty()
        && chars.len() <= 2
        && (chars
            .iter()
            .all(|&c| unicode_normalization::char::is_combining_mark(c))
            || l.top - l.bottom < 3.0)
}

/// Whether mark `m` sits on line `h`: over its letters, at most a few
/// points above or below it.
fn sits_on(h: &Line, m: &Line) -> bool {
    m.x0 >= h.x0 - 1.0 && m.x1 <= h.x1 + 1.0 && m.bottom <= h.top + 3.0 && m.top >= h.bottom - 3.0
}

/// Whether a crop touches a line: over more than a sliver both ways.
fn touches_line(l: &Line, r: &Edges) -> bool {
    let (ox, share) = reach(l.x0, l.x1, l.bottom, l.top, r);
    counts(l) && ox > 3.0 && share > 0.15
}

/// The lines of display `d`: those whose middle is inside it.
pub fn members(page: &Page, d: usize) -> impl Iterator<Item = usize> + '_ {
    let b = page.displays[d];
    (0..page.lines.len()).filter(move |&i| {
        let l = &page.lines[i];
        let (cx, cy) = ((l.x0 + l.x1) / 2.0, (l.top + l.bottom) / 2.0);
        counts(l)
            && cx >= b[0] - 1.0
            && cx <= b[0] + b[2] + 1.0
            && cy >= b[1] - 1.0
            && cy <= b[1] + b[3] + 1.0
    })
}

/// The display line `i` is a piece of, if any.
pub fn display_of(page: &Page, i: usize) -> Option<usize> {
    (0..page.displays.len()).find(|&d| members(page, d).any(|m| m == i))
}

/// A display as one line of the page: its box, standing in for its pieces
/// when a reader takes it as a unit. Its text is that of its lowest line
/// (what a reader asks of the line it read last: does it end with a colon,
/// is it a fragment); it reads as mathematics.
pub fn display_line(page: &Page, d: usize) -> Option<Line> {
    let b = page.displays[d];
    let lowest =
        members(page, d).min_by(|&a, &c| page.lines[a].bottom.total_cmp(&page.lines[c].bottom))?;
    let mut line = page.lines[lowest].clone();
    let pieces: Vec<usize> = members(page, d).collect();
    line.x0 = pieces
        .iter()
        .map(|&i| page.lines[i].x0)
        .fold(b[0], f32::min);
    line.x1 = pieces
        .iter()
        .map(|&i| page.lines[i].x1)
        .fold(b[0] + b[2], f32::max);
    line.bottom = pieces
        .iter()
        .map(|&i| page.lines[i].bottom)
        .fold(b[1], f32::min);
    line.top = pieces
        .iter()
        .map(|&i| page.lines[i].top)
        .fold(b[1] + b[3], f32::max);
    line.math_share = 1.0;
    line.math_count = line.math_count.max(4);
    Some(line)
}

/// Whether a crop touches display `d`: through a line of it, or over a good
/// part of its height.
fn touches_display(page: &Page, d: usize, r: &Edges) -> bool {
    let b = page.displays[d];
    let (ox, share) = reach(b[0], b[0] + b[2], b[1], b[1] + b[3], r);
    if ox <= 3.0 || share <= 0.0 {
        return false;
    }
    (share > 0.25 && share * b[3] > 3.0)
        || members(page, d).any(|i| touches_line(&page.lines[i], r))
}

/// Whether pulling `r` in to `pulled` keeps what `r` held whole: every
/// display and every line (but `skip`, the one pulled away from) in it
/// still in it - a crop pulled off a line it grazes must not drop or cut
/// another.
fn keeps(page: &Page, mark: &[bool], r: &Edges, pulled: &Edges, skip: usize) -> bool {
    let displays = page.displays.iter().all(|b| {
        let (x0, x1, bottom, top) = (b[0], b[0] + b[2], b[1], b[1] + b[3]);
        !r.holds(x0, x1, bottom, top) || pulled.holds(x0, x1, bottom, top)
    });
    displays
        && page.lines.iter().enumerate().all(|(i, l)| {
            i == skip
                || mark[i]
                || !counts(l)
                || !r.holds(l.x0, l.x1, l.bottom, l.top)
                || pulled.holds(l.x0, l.x1, l.bottom, l.top)
        })
}

/// `rect` made whole: every display it touches in it entirely, every line
/// it touches in it over its full width - or, a line only grazed at its
/// top or bottom edge, or reached into from beside, left out by pulling
/// that edge back. Grows rather than cuts when the two disagree.
pub fn settle(page: &Page, rect: Rect) -> Rect {
    if !rect.iter().all(|v| v.is_finite()) {
        return rect;
    }
    let mut r = Edges::of(rect);
    let in_display: Vec<bool> = {
        let mut v = vec![false; page.lines.len()];
        for d in 0..page.displays.len() {
            for i in members(page, d) {
                v[i] = true;
            }
        }
        v
    };
    let mark: Vec<bool> = (0..page.lines.len()).map(|i| is_mark(page, i)).collect();
    let mut pulled_off = vec![false; page.lines.len()];
    for round in 0..16 {
        let before = r;
        // Last rounds only grow: a crop pulled back and grown again in turn
        // settles on the larger.
        let may_trim = round < 8;
        let mut pulled_in = false;
        for d in 0..page.displays.len() {
            let b = page.displays[d];
            if touches_display(page, d, &r) && !r.holds(b[0], b[0] + b[2], b[1], b[1] + b[3]) {
                r.take(b[0] - 1.0, b[0] + b[2] + 1.0, b[1] - 1.0, b[1] + b[3] + 1.0);
            }
        }
        for (i, l) in page.lines.iter().enumerate() {
            if mark[i] || !touches_line(l, &r) || r.holds(l.x0, l.x1, l.bottom, l.top) {
                continue;
            }
            let (ox, share) = reach(l.x0, l.x1, l.bottom, l.top, &r);
            let centre_x = (l.x0 + l.x1) / 2.0;
            // Reached into from beside: little of it in, and little of
            // the crop's width - a crop over the start of a long line of
            // its own column takes the line.
            let beside = (centre_x < r.x0 || centre_x > r.x1)
                && ox < 0.5 * (l.x1 - l.x0)
                && ox < 0.25 * (r.x1 - r.x0);
            // A line of the next column reached into: pull that side in. A
            // line grazed at its top or bottom: leave it out.
            let mut pulled = r;
            if beside {
                if centre_x > r.x1 {
                    pulled.x1 = (l.x0 - 0.5).max(r.x0 + 1.0);
                } else {
                    pulled.x0 = (l.x1 + 0.5).min(r.x1 - 1.0);
                }
            } else if share < 0.5 {
                let centre_y = (l.top + l.bottom) / 2.0;
                if centre_y > (r.y0 + r.y1) / 2.0 {
                    pulled.y1 = (l.bottom - 0.3).max(r.y0 + 1.0);
                } else {
                    pulled.y0 = (l.top + 0.3).min(r.y1 - 1.0);
                }
            }
            // A line pulled off once and reached again is taken: what
            // reached it is the crop's own.
            if may_trim
                && !pulled_off[i]
                && pulled != r
                && !in_display[i]
                && keeps(page, &mark, &r, &pulled, i)
            {
                pulled_off[i] = true;
                pulled_in = true;
                r = pulled;
                continue;
            }
            r.take(l.x0 - 0.5, l.x1 + 0.5, l.bottom - 0.5, l.top + 0.5);
        }
        // A line the page's text splits in pieces (a change of font, a
        // formula) is one line still: the rest of a row it holds, set on
        // close by, comes with it.
        for (i, l) in page.lines.iter().enumerate() {
            if !counts(l) || r.holds(l.x0, l.x1, l.bottom, l.top) {
                continue;
            }
            let joined = page.lines.iter().enumerate().any(|(k, h)| {
                k != i
                    && counts(h)
                    && !mark[k]
                    && r.holds(h.x0, h.x1, h.bottom, h.top)
                    && (same_row(h, l) || (mark[i] && sits_on(h, l)))
            });
            if joined {
                r.take(l.x0 - 0.5, l.x1 + 0.5, l.bottom - 0.5, l.top + 0.5);
            }
        }
        // Settled: nothing grown, nothing pulled in (a line pulled off and
        // grown back over in one round is looked at again).
        if r == before && !pulled_in {
            break;
        }
    }
    let x0 = r.x0.max(0.0);
    let y0 = r.y0.max(0.0);
    let x1 = r.x1.min(page.width.max(1.0));
    let y1 = r.y1.min(page.height.max(1.0));
    [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
}

/// Whether `rect` keeps the invariants `settle` makes: no display and no
/// line touched but not whole.
pub fn is_settled(page: &Page, rect: Rect) -> bool {
    let r = Edges::of(rect);
    let loose = |x0: f32, x1: f32, bottom: f32, top: f32| {
        x0 >= r.x0 - 2.0 && x1 <= r.x1 + 2.0 && bottom >= r.y0 - 1.5 && top <= r.y1 + 1.5
    };
    (0..page.displays.len()).all(|d| {
        let b = page.displays[d];
        !touches_display(page, d, &r) || loose(b[0], b[0] + b[2], b[1], b[1] + b[3])
    }) && page
        .lines
        .iter()
        .all(|l| !touches_line(l, &r) || loose(l.x0, l.x1, l.bottom, l.top))
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::*;

    /// Body lines, a display of pieces in odd order - a sum, its limits,
    /// the expression to its right - with its number at the margin, then
    /// body lines.
    fn page_with_display() -> Page {
        let mut b = Builder::new();
        for y in [600.0, 588.0, 576.0] {
            b.line(
                y,
                BODY,
                "Running text of the body set across the column here.",
            );
        }
        b.line_at(200.0, 552.0, MATH, "X");
        b.line_at(199.0, 564.0, SCRIPT, "m");
        b.line_at(197.0, 541.0, SCRIPT, "i=1");
        // The number first: the page's text has it as a line of its own.
        b.line_at(320.0, 553.0, BODY, "(4)");
        b.line_at(212.0, 553.0, MATH, "f(i) <= g(i) + h(i)");
        for y in [524.0, 512.0] {
            b.line(
                y,
                BODY,
                "Running text of the body set across the column here.",
            );
        }
        let mut page = b.page();
        page.set_displays(vec![([196.0, 540.0, 115.0, 30.0], Some("4".into()))]);
        page
    }

    fn holds(rect: Rect, b: [f32; 4]) -> bool {
        b[0] >= rect[0] - 1.0
            && b[2] <= rect[0] + rect[2] + 1.0
            && b[1] >= rect[1] - 1.0
            && b[3] <= rect[1] + rect[3] + 1.0
    }

    #[test]
    fn a_display_touched_is_taken_whole_with_its_number() {
        let page = page_with_display();
        // The number joined the display.
        assert!(
            page.displays[0][0] + page.displays[0][2] >= 334.0,
            "{:?}",
            page.displays
        );
        // A crop over the sum's top only: its upper limit and a little.
        let rect = settle(&page, [195.0, 556.0, 20.0, 15.0]);
        assert!(holds(rect, [196.0, 540.0, 335.0, 570.0]), "{rect:?}");
        assert!(is_settled(&page, rect), "{rect:?}");
        // Not the body around it.
        assert!(rect[1] + rect[3] < 576.0 && rect[1] > 531.5, "{rect:?}");
    }

    #[test]
    fn a_line_is_taken_whole_or_left_out() {
        let page = page_with_display();
        // Most of a body line's height, a third of its width: all of it.
        let rect = settle(&page, [72.0, 589.0, 100.0, 6.0]);
        assert!(holds(rect, [72.0, 588.0, 330.0, 595.5]), "{rect:?}");
        // A line only grazed at the crop's foot is left out.
        let rect = settle(&page, [72.0, 594.0, 280.0, 14.0]);
        assert!(rect[1] >= 595.0, "{rect:?}");
        assert!(holds(rect, [72.0, 600.0, 330.0, 607.5]), "{rect:?}");
        assert!(is_settled(&page, rect), "{rect:?}");
    }

    #[test]
    fn a_line_reached_into_from_beside_is_left_out() {
        let mut b = Builder::new();
        b.line_at(320.0, 600.0, BODY, "Right column text here.");
        b.line_at(72.0, 600.0, BODY, "Left column text here.");
        let page = b.page();
        let rect = settle(&page, [70.0, 598.0, 255.0, 12.0]);
        assert!(rect[0] + rect[2] < 320.0, "{rect:?}");
        assert!(is_settled(&page, rect), "{rect:?}");
    }

    #[test]
    fn the_rest_of_a_row_comes_with_it() {
        // A line the page's text splits at a change of font.
        let mut b = Builder::new();
        let x = 72.0 + 36.0 * 5.0;
        let end = b.line_at(x + 2.0, 600.0, MATH, "f:X->[0,1]");
        b.line_at(72.0, 600.0, ITALIC, "Theorem 2.3. Fix a class H and a map");
        b.line(588.0, ITALIC, "such that the bound holds.");
        let page = b.page();
        let rect = settle(&page, [70.0, 585.0, x - 69.0, 25.0]);
        assert!(rect[0] + rect[2] >= end - 1.0, "{rect:?}");
    }
}

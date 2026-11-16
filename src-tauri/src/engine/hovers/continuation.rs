//! A target that runs on past a column or page break: a theorem whose
//! statement fills the foot of one column and goes on at the top of the
//! next, a long footnote split between two column feet or two pages, a
//! bibliography entry broken by a column break. The preview shows every
//! piece, one under the other; this finds the pieces after the first.
//!
//! A piece runs on only when the first ends at its column's foot - nothing
//! of the body below it in its column - and what opens the next column (or
//! the next page's first column) reads as more of the same: no label of
//! its own, the statement's face, an entry's hanging indent.
//!
//! PDFium-free.

use super::page::{same_column, Line, Page};
use super::statement::column_left;
use super::targets::{
    clear_of_neighbours, labelled, line_rect, next_line, opens_another, pad, same_row,
    statement_lines, union, Flow,
};
use super::Rect;

/// One piece after the first: on the destination page, or on the page
/// after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub next_page: bool,
    pub rect: Rect,
}

/// At most this many pieces in all: a statement longer than two column
/// breaks is previewed in part.
const MAX_PIECES: usize = 3;

/// The pieces after `first`, in reading order.
pub fn continuation(flow: Flow, page: &Page, next: Option<&Page>, first: Rect) -> Vec<Piece> {
    if flow == Flow::Footnote {
        return super::footnotes::note_continuation(page, next, first)
            .map(|(next_page, rect)| {
                let there = if next_page {
                    next.unwrap_or(page)
                } else {
                    page
                };
                Piece {
                    next_page,
                    rect: super::settle::settle(there, rect),
                }
            })
            .into_iter()
            .collect();
    }
    let italic = match flow {
        Flow::Statement => statement_face(page, first),
        _ => None,
    };
    let hanging = flow == Flow::Reference && hangs(page, first);
    let mut out: Vec<Piece> = Vec::new();
    let mut here = page;
    let mut on_next = false;
    let mut rect = first;
    while out.len() + 1 < MAX_PIECES {
        let Some(last) = last_line(here, rect) else {
            break;
        };
        if !ends_column(here, last) {
            break;
        }
        let (there, start, to_next) = match next_column_start(here, last) {
            Some(start) => (here, start, on_next),
            None if !on_next => match next.and_then(|n| page_start(n).map(|s| (n, s))) {
                Some((n, start)) => (n, start, true),
                None => break,
            },
            None => break,
        };
        let ended = ends_sentence(&here.line_text(last));
        let piece = match flow {
            Flow::Statement => statement_piece(there, start, italic, ended),
            _ => reference_piece(there, start, hanging),
        };
        let Some(piece) = piece else {
            break;
        };
        let piece = super::settle::settle(there, piece);
        out.push(Piece {
            next_page: to_next,
            rect: piece,
        });
        here = there;
        on_next = to_next;
        rect = piece;
    }
    out
}

fn ends_sentence(text: &str) -> bool {
    matches!(
        text.trim_end().chars().last(),
        Some('.' | '!' | '?' | ':' | ';')
    )
}

fn body_size(page: &Page, l: &Line) -> bool {
    page.body_size <= 0.0 || l.size <= 0.0 || l.size >= 0.92 * page.body_size
}

/// The lines a region holds.
fn lines_in(page: &Page, rect: Rect) -> Vec<usize> {
    (0..page.lines.len())
        .filter(|&i| {
            let l = &page.lines[i];
            let centre = (l.top + l.bottom) / 2.0;
            let overlap = l.x1.min(rect[0] + rect[2]) - l.x0.max(rect[0]);
            !l.flags.furniture
                && centre >= rect[1]
                && centre <= rect[1] + rect[3]
                && overlap > 0.5 * (l.x1 - l.x0)
        })
        .collect()
}

/// The lowest line of a region.
fn last_line(page: &Page, rect: Rect) -> Option<usize> {
    lines_in(page, rect)
        .into_iter()
        .filter(|&i| !folio(page, i))
        .min_by(|&a, &b| page.lines[a].bottom.total_cmp(&page.lines[b].bottom))
}

/// A statement's face: whether the prose of its first piece is set apart
/// from the body's.
fn statement_face(page: &Page, rect: Rect) -> Option<bool> {
    let lines = lines_in(page, rect);
    let prose: Vec<&Line> = lines
        .iter()
        .skip(1)
        .map(|&i| &page.lines[i])
        .filter(|l| !l.is_display() && l.math_share < 0.2 && l.x1 - l.x0 > 40.0)
        .collect();
    if prose.is_empty() {
        // No line of prose past the head: the face its lines are set in,
        // the head's words with them - a short statement ending in a
        // display or a list.
        let own = lines
            .iter()
            .filter(|&&i| page.lines[i].font != page.body_font)
            .count();
        return (!lines.is_empty()).then(|| 2 * own > lines.len());
    }
    let own = prose.iter().filter(|l| l.font != page.body_font).count();
    Some(2 * own > prose.len())
}

/// Whether a reference list hangs: an entry's further lines indented past
/// its first.
fn hangs(page: &Page, rect: Rect) -> bool {
    let lines = lines_in(page, rect);
    let Some(&first) = lines.first() else {
        return false;
    };
    let x0 = page.lines[first].x0;
    lines
        .iter()
        .skip(1)
        .any(|&i| (3.0..=30.0).contains(&(page.lines[i].x0 - x0)))
}

/// Whether nothing of the body follows `line` in its column: it is the
/// column's last line, footnotes and running feet apart, low on the page.
fn ends_column(page: &Page, line: usize) -> bool {
    let last = &page.lines[line];
    if last.bottom > 0.5 * page.height {
        return false;
    }
    !page.lines.iter().enumerate().any(|(i, l)| {
        !l.flags.furniture
            && body_size(page, l)
            && same_column(l, last)
            && l.top < last.bottom - 0.5
            && !folio(page, i)
    })
}

/// A page number its page's model did not flag: digits or a roman
/// numeral alone on a line.
fn folio(page: &Page, i: usize) -> bool {
    let text = page.line_text(i);
    let t = text.trim();
    !t.is_empty()
        && t.len() <= 5
        && (t.chars().all(|c| c.is_ascii_digit())
            || t.chars().all(|c| matches!(c, 'i' | 'v' | 'x' | 'l')))
}

/// The regions of a page's figures and tables, read from their captions.
fn float_regions(page: &Page) -> Vec<Rect> {
    let runs = super::xref::runs_of(page);
    crate::engine::floats::caption_blocks(&runs)
        .into_iter()
        .filter_map(|block| {
            crate::engine::floats::float_for_caption(
                &runs,
                page.width,
                page.height,
                block.x,
                block.top - 1.0,
            )
        })
        .collect()
}

/// Lines that can open a column's text: body, not furniture, not in a
/// float.
fn text_lines(page: &Page) -> Vec<usize> {
    let floats = float_regions(page);
    (0..page.lines.len())
        .filter(|&i| {
            let l = &page.lines[i];
            let cx = (l.x0 + l.x1) / 2.0;
            let cy = (l.top + l.bottom) / 2.0;
            !l.flags.furniture
                && body_size(page, l)
                && !folio(page, i)
                && !floats
                    .iter()
                    .any(|f| cx >= f[0] && cx <= f[0] + f[2] && cy >= f[1] && cy <= f[1] + f[3])
        })
        .collect()
}

/// The top of the column right of `line`'s, on the same page.
fn next_column_start(page: &Page, line: usize) -> Option<usize> {
    let last = &page.lines[line];
    let lines = text_lines(page);
    // The column's right edge: its lines', not the last one's, which may
    // end short.
    let edge = lines
        .iter()
        .map(|&i| &page.lines[i])
        .filter(|l| same_column(l, last) && (l.top - last.top).abs() < 300.0)
        .map(|l| l.x1)
        .fold(last.x1, f32::max);
    let right: Vec<usize> = lines
        .into_iter()
        .filter(|&i| {
            let l = &page.lines[i];
            !same_column(l, last) && l.x0 > edge - 4.0
        })
        .collect();
    // The nearest column to the right, not one further on: a column, many
    // lines starting together, not a stray line or a margin's numbers.
    let column = right
        .iter()
        .copied()
        .min_by(|&a, &b| page.lines[a].x0.total_cmp(&page.lines[b].x0))?;
    let in_column: Vec<usize> = right
        .into_iter()
        .filter(|&i| same_column(&page.lines[i], &page.lines[column]))
        .collect();
    let wide = in_column
        .iter()
        .filter(|&&i| page.lines[i].x1 - page.lines[i].x0 > 0.6 * (last.x1 - last.x0).max(60.0))
        .count();
    if in_column.len() < 4 || wide < 3 {
        return None;
    }
    in_column
        .into_iter()
        .max_by(|&a, &b| page.lines[a].top.total_cmp(&page.lines[b].top))
}

/// The top of a page's first column.
fn page_start(page: &Page) -> Option<usize> {
    let lines = text_lines(page);
    // The first column: the one the leftmost line of text is in.
    let leftmost = lines
        .iter()
        .copied()
        .filter(|&i| page.lines[i].x1 - page.lines[i].x0 > 60.0)
        .min_by(|&a, &b| page.lines[a].x0.total_cmp(&page.lines[b].x0))?;
    lines
        .into_iter()
        .filter(|&i| same_column(&page.lines[i], &page.lines[leftmost]))
        .max_by(|&a, &b| page.lines[a].top.total_cmp(&page.lines[b].top))
}

/// A statement going on at `start`: no label of its own, and - for a
/// statement in a face of its own - that face, or a display or list item;
/// for an upright one, the sentence left open at the break.
fn statement_piece(page: &Page, start: usize, italic: Option<bool>, ended: bool) -> Option<Rect> {
    let l = &page.lines[start];
    let text = page.line_text(start);
    let text = text.trim_start();
    if text.starts_with("Proof") || text.starts_with("PROOF") || opens_another(page, start) {
        return None;
    }
    let display = l.is_display() || l.math_share >= 0.2;
    let item = super::statement::opens_item(text);
    let own_face = l.font != page.body_font;
    let goes_on = match italic {
        Some(true) => display || item || own_face,
        _ => !ended || ((display || item) && !text.starts_with(char::is_uppercase)),
    };
    if !goes_on {
        return None;
    }
    let mut face = italic;
    let region = statement_lines(page, start, italic, &mut face);
    Some(clear_of_neighbours(
        pad(union(line_rect(l), region), 3.0, page),
        page,
    ))
}

/// A bibliography entry going on at `start`: not a new entry - no label,
/// and in a hanging list, indented.
fn reference_piece(page: &Page, start: usize, hanging: bool) -> Option<Rect> {
    let l = &page.lines[start];
    let left = column_left(page, l);
    let new_entry =
        |i: usize| labelled(&page.line_text(i)) || (hanging && page.lines[i].x0 <= left + 1.5);
    if new_entry(start) {
        return None;
    }
    let reach = 1.6 * super::statement::leading_gap(page);
    let mut region = line_rect(l);
    let mut previous = start;
    let mut cursor = start;
    for _ in 0..20 {
        let Some(j) = next_line(page, cursor) else {
            break;
        };
        cursor = j;
        let (p, n) = (&page.lines[previous], &page.lines[j]);
        if same_row(p, n) {
            region = union(region, line_rect(n));
            continue;
        }
        let close = p.bottom - n.top <= (0.9 * p.em()).max(reach) && n.top < p.top;
        if new_entry(j) || !same_column(n, l) || !close {
            break;
        }
        region = union(region, line_rect(n));
        previous = j;
    }
    Some(clear_of_neighbours(pad(region, 3.0, page), page))
}

#[cfg(test)]
mod tests {
    use super::super::page::build::*;
    use super::*;

    const LEFT: f32 = 54.0;
    const RIGHT: f32 = 312.0;

    /// A two-column page: filler in both columns, and a statement at the
    /// foot of the left column going on at the top of the right.
    fn two_columns(italic_rest: bool, break_mid_sentence: bool) -> (Page, Rect) {
        let mut b = Builder::new();
        let filler = "Running text of the body set across the column.";
        let mut y = 700.0;
        while y > 140.0 {
            b.line_at(LEFT, y, BODY, filler);
            y -= 12.0;
        }
        let x = b.text(LEFT, 128.0, BOLD, "Theorem 3.");
        b.text(
            x + 4.0,
            128.0,
            ITALIC,
            "Every bounded sequence of reals has",
        );
        b.newline();
        b.line_at(
            LEFT,
            116.0,
            ITALIC,
            "a convergent subsequence, and every one",
        );
        let tail = if break_mid_sentence {
            "of its limits lies in the closed hull of"
        } else {
            "of its limits lies in the closed hull."
        };
        b.line_at(LEFT, 104.0, ITALIC, tail);
        let rest = if italic_rest { ITALIC } else { BODY };
        b.line_at(
            RIGHT,
            700.0,
            rest,
            "the sequence, which is compact in turn.",
        );
        b.line_at(
            RIGHT,
            682.0,
            BODY,
            "Proof. Take the bounded sequence and pass on.",
        );
        let mut y = 670.0;
        while y > 100.0 {
            b.line_at(RIGHT, y, BODY, filler);
            y -= 12.0;
        }
        let page = b.page();
        let first = [LEFT - 3.0, 101.0, 240.0, 37.0];
        (page, first)
    }

    #[test]
    fn a_statement_goes_on_at_the_top_of_the_next_column() {
        let (page, first) = two_columns(true, false);
        let pieces = continuation(Flow::Statement, &page, None, first);
        assert_eq!(pieces.len(), 1, "{pieces:?}");
        let rect = pieces[0].rect;
        assert!(!pieces[0].next_page);
        assert!(rect[0] > RIGHT - 5.0, "{rect:?}");
        assert!(rect[1] + rect[3] > 705.0, "{rect:?}");
        assert!(rect[1] > 690.0, "the proof stays out: {rect:?}");
    }

    #[test]
    fn the_body_at_the_next_column_is_no_continuation() {
        // The statement ended with its sentence; the right column opens
        // with the body face.
        let (page, first) = two_columns(false, false);
        assert!(continuation(Flow::Statement, &page, None, first).is_empty());
    }

    #[test]
    fn a_statement_ending_mid_column_does_not_go_on() {
        let (page, _) = two_columns(true, false);
        // The first lines of the left column, well clear of its foot.
        let first = [LEFT - 3.0, 685.0, 240.0, 20.0];
        assert!(continuation(Flow::Statement, &page, None, first).is_empty());
    }

    #[test]
    fn a_statement_goes_on_at_the_top_of_the_next_page() {
        let mut b = Builder::new();
        let mut y = 700.0;
        while y > 140.0 {
            b.line_at(
                LEFT,
                y,
                BODY,
                "Running text of the body set across the page width.",
            );
            y -= 12.0;
        }
        let x = b.text(LEFT, 128.0, BOLD, "Lemma 4.");
        b.text(
            x + 4.0,
            128.0,
            ITALIC,
            "Every bounded sequence of reals has a",
        );
        b.newline();
        b.line_at(
            LEFT,
            116.0,
            ITALIC,
            "convergent subsequence whose limit lies in",
        );
        let page = b.page();
        let mut n = Builder::new();
        n.line_at(
            LEFT,
            740.0,
            ITALIC,
            "the closed hull of the sequence itself.",
        );
        n.line_at(
            LEFT,
            722.0,
            BOLD,
            "Proof. Pass to a subsequence and conclude.",
        );
        let mut y = 710.0;
        while y > 100.0 {
            n.line_at(
                LEFT,
                y,
                BODY,
                "Running text of the body set across the page width.",
            );
            y -= 12.0;
        }
        let next = n.page();
        let pieces = continuation(
            Flow::Statement,
            &page,
            Some(&next),
            [LEFT - 3.0, 113.0, 300.0, 25.0],
        );
        assert_eq!(pieces.len(), 1, "{pieces:?}");
        assert!(pieces[0].next_page);
        let rect = pieces[0].rect;
        assert!(rect[1] > 728.0 && rect[1] + rect[3] > 745.0, "{rect:?}");
    }

    #[test]
    fn a_hanging_reference_goes_on_indented_in_the_next_column() {
        let mut b = Builder::new();
        let mut y = 700.0;
        while y > 128.0 {
            b.line_at(
                LEFT,
                y,
                BODY,
                "A. Author, A title of a paper, J. Things 1 (2001).",
            );
            b.line_at(
                LEFT + 12.0,
                y - 12.0,
                BODY,
                "pages 1-10, more of the entry here.",
            );
            y -= 24.0;
        }
        b.line_at(
            LEFT,
            116.0,
            BODY,
            "B. Writer, The entry that breaks across the",
        );
        b.line_at(
            LEFT + 12.0,
            104.0,
            BODY,
            "column, its title running on and on to",
        );
        b.line_at(
            RIGHT + 12.0,
            700.0,
            BODY,
            "the next column, J. Things 4 (2004) 1-9.",
        );
        b.line_at(
            RIGHT,
            688.0,
            BODY,
            "C. Next, Another entry starting at the margin.",
        );
        b.line_at(
            RIGHT + 12.0,
            676.0,
            BODY,
            "its second line, J. Things 5 (2005).",
        );
        b.line_at(
            RIGHT,
            664.0,
            BODY,
            "D. Last, One more entry starting at the margin.",
        );
        b.line_at(
            RIGHT + 12.0,
            652.0,
            BODY,
            "and its second line, J. Things 6 (2006).",
        );
        let page = b.page();
        let pieces = continuation(
            Flow::Reference,
            &page,
            None,
            [LEFT - 3.0, 101.0, 240.0, 25.0],
        );
        assert_eq!(pieces.len(), 1, "{pieces:?}");
        let rect = pieces[0].rect;
        assert!(rect[1] > 694.0 && rect[1] + rect[3] > 705.0, "{rect:?}");
    }
}

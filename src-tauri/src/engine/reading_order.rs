//! The order a page's text runs are read in.
//!
//! PDFium hands characters over in the order the content stream draws them,
//! and a stream is free to draw a page in any order at all: a line of the left
//! column then a line of the right, the running head last, a footnote before
//! the text that calls it. The text layer is one span per run in run order,
//! and a selection is a DOM range over those spans, so whatever the stream
//! order is, a drag selects: from the end of the left column to the top of
//! the right, it picked up half the page.
//!
//! So the runs are put in reading order before they reach the text layer:
//! the page's columns (the gutters `preview` finds), cut across by any
//! full-width element (a title block, a `figure*` caption, a wide table row)
//! into bands read top to bottom; within a band, a `table*` over the columns
//! first, then each column in turn, left to right; then any float drawn into
//! the middle of a column after the column's text; and last what belongs to
//! no column: running heads, page numbers, margin notes and stamps. Within a
//! column the stream's own order is kept, which is the order everything else
//! about a column - display equations, a superscript drawn after its line -
//! was built against, except where the stream goes back up the column: a
//! stretch drawn after text wholly below it (a column drawn after its
//! footnote, a figure's labels after the text under the figure) goes before
//! that text.
//!
//! A run's `start` is untouched: it still indexes the page's character
//! stream, which is what search hits and everything else that maps
//! characters to the page use.
//!
//! PDFium-free, so the ordering is an ordinary unit test.

use super::preview::{
    median_height, nearest_column, page_columns, rows_with_members, spans_gutter, Interval,
};
use super::types::TextRun;

/// A running head or a page number sits in this share of the page's height
/// at its top or bottom.
const MARGIN_BAND_SHARE: f32 = 0.1;
/// ...set apart from the text by at least this many of the body's line
/// heights...
const MARGIN_GAP_LINES: f32 = 1.5;
/// ...in type no larger than the body's (a title is not a running head).
const MARGIN_SIZE_RATIO: f32 = 1.3;
/// At most this many lines at each end of the page are running matter.
const MARGIN_MAX_BANDS: usize = 2;
/// A stretch of a column's runs that starts this many line heights above
/// where the stream last was has gone back up the column.
const BACKTRACK_LINES: f32 = 1.5;
/// ...as has one this many below it: past a figure, not a paragraph break.
const LEAP_LINES: f32 = 4.0;
/// A stretch drawn into the middle of a column's text from this many line
/// heights below it is a float drawn after the column.
const FLOAT_JUMP_LINES: f32 = 8.0;
/// Text this many line heights clear of where the columns' prose starts is
/// set apart from it.
const APART_LINES: f32 = 1.5;
/// Text this far clear of the text block's edge is beside it, not in it.
const SIDE_GAP_PT: f32 = 3.0;
/// Fewest prose lines that say where a page's text block is.
const MIN_PROSE_LINES: usize = 8;
/// A line starting within this many points of a column's left edge is
/// that column's (an indent included).
const FLUSH_PT: f32 = 15.0;
/// A stretch of text is at least this many line heights wide: text, not
/// a label.
const STRETCH_WIDTH_LINES: f32 = 10.0;
/// A column whose lines are at least this share of its width is prose.
const PROSE_SHARE: f32 = 0.6;

/// The page's runs in reading order (see the module comment).
pub fn in_reading_order(runs: Vec<TextRun>, page_w: f32, page_h: f32) -> Vec<TextRun> {
    let order = reading_order(&runs, page_w, page_h);
    let mut slots: Vec<Option<TextRun>> = runs.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

/// Where each run goes: the indices of `runs` in reading order.
pub fn reading_order(runs: &[TextRun], page_w: f32, page_h: f32) -> Vec<usize> {
    let n = runs.len();
    // The stream's own order, which is what each column keeps.
    let mut raw: Vec<usize> = (0..n).collect();
    raw.sort_by_key(|&i| runs[i].start);
    if n < 2 {
        return raw;
    }
    let columns = page_columns(runs, page_w);
    let (rows, member) = rows_with_members(runs, &columns, true);
    // The body's line height, from rows rather than runs: a run is often
    // one word, and a word without ascenders or descenders is short.
    let all: Vec<&TextRun> = rows.iter().collect();
    let line_h = median_height(&all);

    let mut margin = running_matter(runs, page_h, line_h);
    side_matter(runs, &columns, &rows, &member, &mut margin);

    // Each row: the column it is in, or None when it reaches across a gutter.
    let row_column: Vec<Option<usize>> = rows
        .iter()
        .map(|row| {
            let index = nearest_column(&columns, row.x + row.w.min(20.0) / 2.0);
            (!spans_gutter(&columns, index, row)).then_some(index)
        })
        .collect();
    // Rows with any text of the body (not a running head) in them.
    let mut in_body = vec![false; rows.len()];
    for (i, m) in member.iter().enumerate() {
        if let Some(row) = *m {
            in_body[row] |= margin[i] == Margin::Body;
        }
    }
    // A row across a gutter is a full-width element unless a column's own
    // line goes on beside it, starting at that column's edge: a display
    // overrunning its column. What else is beside it is the rest of its
    // own line, broken at a wide space (a sentence's end in a centred
    // abstract).
    let flush = |row: &TextRun| {
        columns
            .iter()
            .any(|c| row.x >= c.start - 2.0 && row.x <= c.start + FLUSH_PT)
    };
    let wide: Vec<usize> = (0..rows.len())
        .filter(|&i| row_column[i].is_none() && in_body[i])
        .filter(|&i| {
            let row = &rows[i];
            !rows.iter().enumerate().any(|(j, other)| {
                row_column[j].is_some() && in_body[j] && flush(other) && {
                    let overlap = (row.y + row.h).min(other.y + other.h) - row.y.max(other.y);
                    overlap > 0.5 * row.h.min(other.h)
                }
            })
        })
        .collect();
    // The wide rows cut the page into bands, top first: band k lies below
    // k of them.
    let mut cuts: Vec<f32> = wide.iter().map(|&i| centre(&rows[i])).collect();
    cuts.sort_by(|a, b| b.total_cmp(a));
    cuts.dedup();
    let band_of = |y: f32| cuts.iter().filter(|&&cut| cut > y).count();

    // Where each run is read: the band (2k for the text of band k, 2k+1 for
    // the wide row under it), and then the column within a band.
    // A run on a wide row's line (the rest of a line broken at a wide
    // space) is read with it.
    let slot_of_wide = |row: usize| {
        let cut = cuts
            .iter()
            .position(|&c| c == centre(&rows[row]))
            .unwrap_or(0);
        2 * cut + 1
    };
    let slot: Vec<usize> = (0..n)
        .map(|i| match member[i] {
            Some(row) if wide.contains(&row) => slot_of_wide(row),
            _ => {
                let y = centre(&runs[i]);
                wide.iter()
                    .find(|&&w| y >= rows[w].y && y <= top(&rows[w]))
                    .map_or(2 * band_of(y), |&w| slot_of_wide(w))
            }
        })
        .collect();
    let mut key: Vec<(usize, usize)> = slot.iter().map(|&s| (s, 0)).collect();
    let mut bands: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        if slot[i] % 2 == 0 && margin[i] == Margin::Body {
            bands.entry(slot[i]).or_default().push(i);
        }
    }
    for members in bands.values() {
        for (i, column) in band_columns(
            runs,
            members,
            &columns,
            &row_column,
            &member,
            page_w,
            line_h,
        ) {
            key[i].1 = column;
        }
    }

    let mut out = Vec::with_capacity(n);
    let mut cells: std::collections::BTreeMap<(usize, usize), Vec<usize>> = Default::default();
    for &i in &raw {
        if margin[i] == Margin::Body {
            cells.entry(key[i]).or_default().push(i);
        }
    }
    let mut floats = Vec::new();
    for cell in cells.values() {
        let (text, inserted) = down_the_column(runs, cell, line_h);
        out.extend(text);
        floats.extend(inserted);
    }
    out.extend(floats);
    for part in [Margin::Head, Margin::Foot, Margin::Side] {
        let cell: Vec<usize> = raw.iter().copied().filter(|&i| margin[i] == part).collect();
        let (text, inserted) = down_the_column(runs, &cell, line_h);
        out.extend(text);
        out.extend(inserted);
    }
    out
}

/// The column each run of one band (`members`, indices of `runs`) is read
/// in.
///
/// A band is set in the page's columns when its text fits them: none of
/// its rows reaches across their gutters, and its lines start at their
/// edges. Otherwise it may have columns of its own - a keyword box beside
/// an abstract, whose gutter is somewhere else - found from the band's own
/// text; and failing that it is one column, as is a band whose columns hold
/// no prose (a wide table, a `figure*`'s artwork).
fn band_columns(
    runs: &[TextRun],
    members: &[usize],
    page: &[Interval],
    page_row_column: &[Option<usize>],
    page_member: &[Option<usize>],
    page_w: f32,
    line_h: f32,
) -> Vec<(usize, usize)> {
    let subset: Vec<TextRun> = members.iter().map(|&i| runs[i].clone()).collect();
    let crossing = members
        .iter()
        .any(|&i| page_member[i].is_some_and(|row| page_row_column[row].is_none()));
    let columns = if page.len() >= 2 && !crossing && flush_columns(&subset, page) {
        page.to_vec()
    } else {
        let own = page_columns(&subset, page_w);
        if own.len() < 2 {
            return Vec::new();
        }
        own
    };
    let (rows, member) = rows_with_members(&subset, &columns, true);
    let column_of = |row: &TextRun| {
        let index = nearest_column(&columns, row.x + row.w.min(20.0) / 2.0);
        if spans_gutter(&columns, index, row) {
            nearest_column(&columns, row.x)
        } else {
            index
        }
    };
    // Where each column's prose starts.
    let mut first = vec![f32::MIN; columns.len()];
    for row in &rows {
        let index = nearest_column(&columns, row.x + row.w.min(20.0) / 2.0);
        if !spans_gutter(&columns, index, row) && row.w >= PROSE_SHARE * width_of(&columns, index) {
            first[index] = first[index].max(top(row));
        }
    }
    if first.iter().all(|&t| t == f32::MIN) {
        return Vec::new();
    }
    // Above where any column's prose starts, the band is not in columns: a
    // `table*` at the top of the page over the columns' text, its cells in
    // no column. It is read whole, before the columns. (Not so below where
    // they end: a column's last lines are too often not prose - a short
    // line, a display - to tell a float under the columns from them.)
    let above = first.iter().copied().fold(f32::MIN, f32::max) + APART_LINES * line_h;
    // Only what lies across the columns is apart from them: a figure in
    // one column, above where the next column's text starts, is that
    // column's.
    let mut touched = vec![false; columns.len()];
    for row in rows.iter().filter(|r| r.y > above) {
        for (k, column) in columns.iter().enumerate() {
            touched[k] |= row.x < column.end && row.x + row.w > column.start;
        }
    }
    let apart = touched.iter().filter(|&&t| t).count() >= 2;
    members
        .iter()
        .zip(&member)
        .map(|(&i, m)| {
            let Some(row) = *m else {
                return (i, 1);
            };
            let row = &rows[row];
            let place = if apart && row.y > above {
                0
            } else {
                1 + column_of(row)
            };
            (i, place)
        })
        .collect()
}

/// Whether `runs` are set in `columns`: the lines in each column after the
/// first mostly start at its left edge, as a column's do and the pieces of
/// a line broken at a wide space (in a centred block) do not.
fn flush_columns(runs: &[TextRun], columns: &[Interval]) -> bool {
    let (rows, _) = rows_with_members(runs, columns, true);
    columns.iter().skip(1).all(|column| {
        let inside: Vec<&TextRun> = rows
            .iter()
            .filter(|row| row.x >= column.start - 2.0 && row.x < column.end)
            .collect();
        let flush = inside
            .iter()
            .filter(|row| row.x <= column.start + FLUSH_PT)
            .count();
        !inside.is_empty() && 2 * flush >= inside.len()
    })
}

fn centre(run: &TextRun) -> f32 {
    run.y + run.h / 2.0
}

fn top(run: &TextRun) -> f32 {
    run.y + run.h
}

fn width_of(columns: &[Interval], index: usize) -> f32 {
    columns.get(index).map_or(f32::INFINITY, |c| c.width())
}

/// One column's runs, given in the stream's order, read down the column:
/// the stream's order, except that a stretch it drew after text wholly
/// below it goes before that text. And, apart, the stretches drawn
/// into the middle of the column's text after it.
fn down_the_column(runs: &[TextRun], cell: &[usize], line_h: f32) -> (Vec<usize>, Vec<usize>) {
    let mut stretches: Vec<Vec<usize>> = Vec::new();
    for &i in cell {
        // A step back up the column, or a leap down it past more than a
        // paragraph's spacing (from a figure's label to the text under the
        // figure), starts another stretch.
        let broken = stretches
            .last()
            .and_then(|s| s.last())
            .is_some_and(|&last| {
                let step = top(&runs[i]) - top(&runs[last]);
                step > BACKTRACK_LINES * line_h || step < -LEAP_LINES * line_h
            });
        match stretches.last_mut() {
            Some(stretch) if !broken => stretch.push(i),
            _ => stretches.push(vec![i]),
        }
    }
    // Narrow stretches one after another (a figure's tick labels, each a
    // step back up) are one piece of artwork, kept in the stream's order.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut artwork = false;
    for stretch in stretches {
        let left = stretch.iter().map(|&i| runs[i].x).fold(f32::MAX, f32::min);
        let right = stretch
            .iter()
            .map(|&i| runs[i].x + runs[i].w)
            .fold(f32::MIN, f32::max);
        let narrow = right - left < STRETCH_WIDTH_LINES * line_h;
        match groups.last_mut() {
            Some(group) if narrow && artwork => group.extend(stretch),
            _ => groups.push(stretch),
        }
        artwork = narrow;
    }
    let extent = |group: &[usize]| {
        let highest = group
            .iter()
            .map(|&i| top(&runs[i]))
            .fold(f32::MIN, f32::max);
        let lowest = group.iter().map(|&i| runs[i].y).fold(f32::MAX, f32::min);
        (lowest, highest)
    };
    // A group goes before those drawn ahead of it that lie wholly below
    // it: a column's text drawn after its footnote, a figure's labels after
    // the text under the figure. One that overlaps what came before (the
    // rest of a column after a display's numerator) stays after it.
    let mut kept: Vec<Vec<usize>> = Vec::new();
    // A group drawn after the text both above and below it - a float set
    // in the middle of a column, drawn once the column was done - has no
    // place in the column's text to go to: it is read after the page's
    // text, as the stream had it, rather than between two columns.
    let mut inserted = Vec::new();
    let mut drawn: Vec<usize> = Vec::new();
    for group in groups {
        let (lowest, highest) = extent(&group);
        let above = drawn
            .iter()
            .any(|&i| top(&runs[i]) >= highest + 0.5 * line_h);
        let below = drawn
            .iter()
            .any(|&i| top(&runs[i]) <= lowest - 0.5 * line_h);
        // ...and drawn from well below it: a float drawn once the column
        // was done, not the next column of a matrix in a display.
        let from_far = drawn
            .last()
            .is_some_and(|&i| highest - top(&runs[i]) > FLOAT_JUMP_LINES * line_h);
        drawn.extend(&group);
        if above && below && from_far {
            inserted.extend(group);
            continue;
        }
        let mut at = kept.len();
        while at > 0 && lowest >= extent(&kept[at - 1]).1 - 0.5 * line_h {
            at -= 1;
        }
        kept.insert(at, group);
    }
    (kept.concat(), inserted)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Margin {
    Body,
    /// A running head, at the top of the page.
    Head,
    /// A running foot or a page number.
    Foot,
    /// Beside the text block: a margin note, line numbers, an arXiv stamp.
    Side,
}

/// Marks as `Side` the runs wholly outside the text block across the page:
/// left of where its leftmost prose lines start, or right of where its
/// rightmost end.
fn side_matter(
    runs: &[TextRun],
    columns: &[Interval],
    rows: &[TextRun],
    member: &[Option<usize>],
    margin: &mut [Margin],
) {
    let prose = |row: &&TextRun| {
        let index = nearest_column(columns, row.x + row.w.min(20.0) / 2.0);
        row.w >= PROSE_SHARE * width_of(columns, index)
    };
    let mut lefts: Vec<f32> = rows.iter().filter(prose).map(|r| r.x).collect();
    let mut rights: Vec<f32> = rows.iter().filter(prose).map(|r| r.x + r.w).collect();
    if lefts.len() < MIN_PROSE_LINES {
        return;
    }
    lefts.sort_by(f32::total_cmp);
    rights.sort_by(f32::total_cmp);
    // Past a stray line or two: an overhanging display.
    let left = lefts[lefts.len() / 20];
    let right = rights[rights.len() - 1 - rights.len() / 20];
    for (i, run) in runs.iter().enumerate() {
        // By its row: a hanging label is part of its line.
        let row = member[i].map_or(run, |r| &rows[r]);
        if margin[i] == Margin::Body
            && (row.x + row.w < left - SIDE_GAP_PT || row.x > right + SIDE_GAP_PT)
        {
            margin[i] = Margin::Side;
        }
    }
}

/// Which runs are running heads and feet: a line or two at the very top or
/// bottom of the page, in type no larger than the body's, set apart from
/// everything else on it.
fn running_matter(runs: &[TextRun], page_h: f32, line_h: f32) -> Vec<Margin> {
    let mut out = vec![Margin::Body; runs.len()];
    // The page's lines as vertical bands, top first: runs whose extents
    // overlap are one band.
    let mut by_top: Vec<usize> = (0..runs.len()).collect();
    by_top.sort_by(|&a, &b| top(&runs[b]).total_cmp(&top(&runs[a])));
    let mut bands: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for i in by_top {
        let run = &runs[i];
        match bands.last_mut() {
            Some((bottom, _, members)) if top(run) > *bottom => {
                *bottom = bottom.min(run.y);
                members.push(i);
            }
            _ => bands.push((run.y, top(run), vec![i])),
        }
    }
    let gap = MARGIN_GAP_LINES * line_h;
    let small = |members: &[usize]| {
        members
            .iter()
            .all(|&i| runs[i].h <= MARGIN_SIZE_RATIO * line_h)
    };
    // From the top down.
    for k in 0..bands.len().min(MARGIN_MAX_BANDS) {
        let (bottom, _, members) = &bands[k];
        let below = bands.get(k + 1).map_or(f32::NEG_INFINITY, |b| b.1);
        if *bottom < (1.0 - MARGIN_BAND_SHARE) * page_h || bottom - below < gap || !small(members) {
            break;
        }
        for &i in members {
            out[i] = Margin::Head;
        }
    }
    // From the bottom up.
    for k in (0..bands.len()).rev().take(MARGIN_MAX_BANDS) {
        let (_, band_top, members) = &bands[k];
        let above = if k == 0 {
            f32::INFINITY
        } else {
            bands[k - 1].0
        };
        if out[members[0]] != Margin::Body
            || *band_top > MARGIN_BAND_SHARE * page_h
            || above - band_top < gap
            || !small(members)
        {
            break;
        }
        for &i in members {
            out[i] = Margin::Foot;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_W: f32 = 612.0;
    const PAGE_H: f32 = 792.0;
    const LINE: f32 = 12.0;
    const H: f32 = 8.0;

    fn run(text: &str, start: u32, x: f32, y: f32, w: f32) -> TextRun {
        TextRun {
            text: text.to_string(),
            start,
            x,
            y,
            w,
            h: H,
            edges: Vec::new(),
            math: Vec::new(),
        }
    }

    /// A page of two columns, 240pt wide from x=54 and x=306, `lines` lines
    /// each from y=700 down, drawn as the stream of `interleaved` pages
    /// does: a line of the left column, then the line beside it in the
    /// right. Texts are "L3", "R3" and so on.
    fn two_columns(lines: usize, interleaved: bool) -> Vec<TextRun> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut push = |out: &mut Vec<TextRun>, text: String, x: f32, y: f32| {
            out.push(run(&text, start, x, y, 240.0));
            start += 10;
        };
        if interleaved {
            for k in 0..lines {
                let y = 700.0 - LINE * k as f32;
                push(&mut out, format!("L{k}"), 54.0, y);
                push(&mut out, format!("R{k}"), 306.0, y);
            }
        } else {
            for (prefix, x) in [("L", 54.0), ("R", 306.0)] {
                for k in 0..lines {
                    push(&mut out, format!("{prefix}{k}"), x, 700.0 - LINE * k as f32);
                }
            }
        }
        out
    }

    fn texts(runs: &[TextRun]) -> Vec<String> {
        runs.iter().map(|r| r.text.clone()).collect()
    }

    fn read(runs: Vec<TextRun>) -> Vec<String> {
        texts(&in_reading_order(runs, PAGE_W, PAGE_H))
    }

    fn column(prefix: &str, lines: usize) -> Vec<String> {
        (0..lines).map(|k| format!("{prefix}{k}")).collect()
    }

    #[test]
    fn interleaved_columns_read_one_after_the_other() {
        let expected = [column("L", 20), column("R", 20)].concat();
        assert_eq!(read(two_columns(20, true)), expected);
        assert_eq!(read(two_columns(20, false)), expected);
    }

    #[test]
    fn the_stream_order_is_kept_where_it_is_reading_order() {
        let runs = two_columns(20, false);
        assert_eq!(
            reading_order(&runs, PAGE_W, PAGE_H),
            (0..runs.len()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_start_still_names_its_own_characters() {
        let runs = two_columns(10, true);
        let ordered = in_reading_order(runs.clone(), PAGE_W, PAGE_H);
        for run in &ordered {
            let original = runs.iter().find(|r| r.text == run.text).unwrap();
            assert_eq!(run.start, original.start);
        }
    }

    /// A `figure*` caption across both columns halfway down: the columns
    /// above it, then the caption, then the columns below.
    #[test]
    fn a_full_width_caption_cuts_the_page_into_bands() {
        let mut runs = Vec::new();
        let mut start = 0;
        let mut add = |text: String, x: f32, y: f32, w: f32| {
            runs.push(run(&text, start, x, y, w));
            start += 10;
        };
        // The stream: the caption first, then the columns line by line.
        add("CAP".into(), 54.0, 500.0, 492.0);
        for k in 0..12 {
            let y = 700.0 - LINE * k as f32;
            add(format!("LA{k}"), 54.0, y, 240.0);
            add(format!("RA{k}"), 306.0, y, 240.0);
        }
        for k in 0..12 {
            let y = 470.0 - LINE * k as f32;
            add(format!("LB{k}"), 54.0, y, 240.0);
            add(format!("RB{k}"), 306.0, y, 240.0);
        }
        let expected = [
            column("LA", 12),
            column("RA", 12),
            vec!["CAP".to_string()],
            column("LB", 12),
            column("RB", 12),
        ]
        .concat();
        assert_eq!(read(runs), expected);
    }

    /// A display overrunning the left column into the gutter, with the
    /// right column's lines going on beside it, is part of the left column.
    #[test]
    fn a_line_overrunning_into_the_next_column_stays_in_its_own() {
        let mut runs = two_columns(20, true);
        // L5 runs on 10pt into the next column, beside R5
        let at = runs.iter().position(|r| r.text == "L5").unwrap();
        runs[at].w = 262.0;
        let expected = [column("L", 20), column("R", 20)].concat();
        assert_eq!(read(runs), expected);
    }

    /// A centred abstract over the two columns, one of whose lines the
    /// stream broke at a sentence's end, its second piece right of the
    /// gutter: still one line of the abstract, read before the columns.
    #[test]
    fn a_full_width_line_broken_at_a_wide_space_stays_one_line() {
        let mut runs = Vec::new();
        let mut start = 0;
        let mut add = |text: &str, x: f32, y: f32, w: f32| {
            runs.push(run(text, start, x, y, w));
            start += 10;
        };
        for k in 0..4 {
            let y = 720.0 - LINE * k as f32;
            if k == 2 {
                add("A2a", 100.0, y, 250.0);
                add("A2b", 360.0, y, 150.0);
            } else {
                add(&format!("A{k}"), 100.0, y, 410.0);
            }
        }
        for k in 0..20 {
            let y = 660.0 - LINE * k as f32;
            add(&format!("L{k}"), 54.0, y, 240.0);
            add(&format!("R{k}"), 306.0, y, 240.0);
        }
        let expected = [
            vec!["A0", "A1", "A2a", "A2b", "A3"]
                .into_iter()
                .map(String::from)
                .collect(),
            column("L", 20),
            column("R", 20),
        ]
        .concat();
        assert_eq!(read(runs), expected);
    }

    /// The running head drawn after the left column and the page number
    /// before the right: both last, so a drag from one column to the next
    /// takes neither.
    #[test]
    fn running_heads_and_page_numbers_go_last() {
        let mut runs = two_columns(20, false);
        let left = 20;
        runs.insert(
            left,
            run("Journal of Things 12 (2026)", 15, 54.0, 750.0, 150.0),
        );
        runs.insert(left + 1, run("7", 16, 300.0, 40.0, 6.0));
        for (k, r) in runs.iter_mut().enumerate() {
            r.start = 10 * k as u32;
        }
        let expected = [
            column("L", 20),
            column("R", 20),
            vec!["Journal of Things 12 (2026)".to_string(), "7".to_string()],
        ]
        .concat();
        assert_eq!(read(runs), expected);
    }

    /// A title is set large: not a running head however close to the top.
    #[test]
    fn a_title_is_no_running_head() {
        let mut runs = vec![TextRun {
            h: 16.0,
            ..run("A Title", 0, 150.0, 740.0, 300.0)
        }];
        for (k, mut r) in two_columns(20, false).into_iter().enumerate() {
            r.start = 10 * (k as u32 + 1);
            runs.push(r);
        }
        assert_eq!(read(runs)[0], "A Title");
    }

    /// The stream drew a column's footnote before its text: read after it.
    #[test]
    fn a_stretch_drawn_before_the_text_above_it_follows_that_text() {
        let mut runs = vec![run("FOOT", 0, 54.0, 90.0, 252.0)];
        for (k, mut r) in two_columns(20, false).into_iter().enumerate() {
            r.start = 10 * (k as u32 + 1);
            runs.push(r);
        }
        let expected = [column("L", 20), vec!["FOOT".to_string()], column("R", 20)].concat();
        assert_eq!(read(runs), expected);
    }

    /// A superscript drawn after its line, a little above it, stays where
    /// the stream put it.
    #[test]
    fn a_script_drawn_after_its_line_stays_with_it() {
        let mut runs = two_columns(20, false);
        let mut mark = run("1", 0, 200.0, 700.0 - 5.0 * LINE + 4.0, 4.0);
        mark.h = 5.0;
        let at = runs.iter().position(|r| r.text == "L5").unwrap();
        runs.insert(at + 1, mark);
        for (k, r) in runs.iter_mut().enumerate() {
            r.start = 10 * k as u32;
        }
        let read = read(runs);
        let at = read.iter().position(|t| t == "L5").unwrap();
        assert_eq!(read[at + 1], "1");
    }

    /// An arXiv stamp up the left margin, a letter a run, drawn after the
    /// left column: beside the text, so read after all of it.
    #[test]
    fn a_stamp_in_the_margin_goes_last() {
        let mut runs = two_columns(20, false);
        let stamp: Vec<TextRun> = "arXiv:2404"
            .chars()
            .enumerate()
            .map(|(k, ch)| TextRun {
                w: 8.0,
                h: 6.0,
                ..run(&ch.to_string(), 0, 18.0, 300.0 + 9.0 * k as f32, 8.0)
            })
            .collect();
        runs.splice(20..20, stamp);
        for (k, r) in runs.iter_mut().enumerate() {
            r.start = 10 * k as u32;
        }
        let read = read(runs);
        assert_eq!(read[..40], [column("L", 20), column("R", 20)].concat()[..]);
        assert_eq!(read[40..].concat(), "arXiv:2404");
    }

    /// A figure at the top of the left column, its labels drawn after the
    /// column's text: read where the figure is, above the text.
    #[test]
    fn a_figure_drawn_late_is_read_where_it_stands() {
        let mut runs: Vec<TextRun> = two_columns(20, false)
            .into_iter()
            .map(|mut r| {
                r.y -= 120.0;
                r
            })
            .collect();
        let labels: Vec<TextRun> = (0..5)
            .map(|k| run(&format!("t{k}"), 0, 60.0, 600.0 + 20.0 * k as f32, 10.0))
            .collect();
        runs.splice(20..20, labels);
        for (k, r) in runs.iter_mut().enumerate() {
            r.start = 10 * k as u32;
        }
        let read = read(runs);
        assert_eq!(read[..5], ["t0", "t1", "t2", "t3", "t4"]);
        assert_eq!(read[5..], [column("L", 20), column("R", 20)].concat()[..]);
    }

    /// A table set halfway down the left column, drawn after both columns'
    /// text: after the page's text, not between the two columns, so a drag
    /// from the foot of one column to the head of the next skips it.
    #[test]
    fn a_float_drawn_into_a_column_after_it_is_read_after_the_text() {
        let mut runs = Vec::new();
        for k in 0..30 {
            // a gap for the table between lines 10 and 11
            let y = 700.0 - LINE * k as f32 - if k > 10 { 60.0 } else { 0.0 };
            runs.push(run(&format!("L{k}"), 0, 54.0, y, 240.0));
        }
        for k in 0..30 {
            runs.push(run(
                &format!("R{k}"),
                0,
                306.0,
                700.0 - LINE * k as f32,
                240.0,
            ));
        }
        for row in 0..3 {
            for (c, x) in [60.0, 140.0, 220.0].into_iter().enumerate() {
                let y = 560.0 - LINE * row as f32;
                runs.push(run(&format!("T{row}{c}"), 0, x, y, 60.0));
            }
        }
        for (k, r) in runs.iter_mut().enumerate() {
            r.start = 10 * k as u32;
        }
        let read = read(runs);
        assert_eq!(read[..60], [column("L", 30), column("R", 30)].concat()[..]);
        assert_eq!(read[60], "T00");
    }

    /// A `table*` across the top of the page, none of its cells crossing
    /// the gutter, drawn after the columns' text: read before the columns,
    /// not split between them.
    #[test]
    fn a_wide_table_over_the_columns_is_read_whole() {
        let mut runs = two_columns(20, true)
            .into_iter()
            .map(|mut r| {
                r.y -= 60.0;
                r
            })
            .collect::<Vec<_>>();
        for row in 0..3 {
            for (c, x) in [70.0, 190.0, 330.0, 450.0].into_iter().enumerate() {
                runs.push(run(
                    &format!("T{row}{c}"),
                    0,
                    x,
                    730.0 - LINE * row as f32,
                    50.0,
                ));
            }
        }
        for (k, r) in runs.iter_mut().enumerate() {
            r.start = 10 * k as u32;
        }
        let table: Vec<String> = (0..3)
            .flat_map(|row| (0..4).map(move |c| format!("T{row}{c}")))
            .collect();
        assert_eq!(
            read(runs),
            [table, column("L", 20), column("R", 20)].concat()
        );
    }

    /// A single column is read in the stream's order.
    #[test]
    fn a_single_column_keeps_the_stream_order() {
        let runs: Vec<TextRun> = (0..30)
            .map(|k| {
                run(
                    &format!("S{k}"),
                    k * 10,
                    72.0,
                    700.0 - LINE * k as f32,
                    468.0,
                )
            })
            .collect();
        assert_eq!(read(runs), column("S", 30));
    }
}

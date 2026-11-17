//! Where a theorem-like statement ends, read line by line: what the hover
//! index's statements, the link previews' fallback and a statement's
//! continuation over a column or page break all share.
//!
//! A statement runs from its label to the proof, the next labelled block
//! or heading, or a clear return to the body's prose: the next paragraph
//! back in the body face after an italic statement, or one set apart by
//! more than a line break. What it holds on the way is all of it - its
//! displays, and its enumerations, whose items ("(i)", "(a)", "1.") are
//! often set upright inside an italic statement and set apart by a list's
//! own skip, and whose further lines hang indented under the item's text.
//!
//! PDFium-free.

use super::page::{same_column, Line, Page};

/// Whether a line opens a list item: "(i)", "(ii)", "(a)", "(1)", "1.",
/// "1)", "a)", "iv.", or a bullet.
pub fn opens_item(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with([
        '\u{2022}', '\u{2023}', '\u{25e6}', '\u{2013}', '\u{2014}', '\u{00b7}', '*',
    ]) {
        return true;
    }
    let label: String = t.chars().take_while(|c| !c.is_whitespace()).collect();
    if label.is_empty() || label.chars().count() > 7 {
        return false;
    }
    let inner = if let Some(rest) = label.strip_prefix('(') {
        let Some(inner) = rest.strip_suffix(')') else {
            return false;
        };
        inner
    } else if let Some(inner) = label.strip_suffix(')').or_else(|| label.strip_suffix('.')) {
        inner
    } else {
        return false;
    };
    let roman = !inner.is_empty() && inner.chars().all(|c| matches!(c, 'i' | 'v' | 'x'));
    let letter = inner.chars().count() == 1 && inner.chars().all(|c| c.is_ascii_lowercase());
    let number = !inner.is_empty() && inner.len() <= 2 && inner.chars().all(|c| c.is_ascii_digit());
    roman || letter || number
}

/// The gap a page leaves between the lines of a paragraph: the commonest
/// one between lines that follow each other in a column.
pub fn leading_gap(page: &Page) -> f32 {
    let mut gaps: Vec<f32> = page
        .lines
        .windows(2)
        .filter(|w| same_column(&w[0], &w[1]) && w[0].bottom > w[1].top)
        .map(|w| w[0].bottom - w[1].top)
        .filter(|g| *g < 24.0)
        .collect();
    gaps.sort_by(f32::total_cmp);
    gaps.get(gaps.len() / 2).copied().unwrap_or(3.0)
}

/// The left margin of the column `line` is in: the leftmost start of the
/// column's lines near it.
pub fn column_left(page: &Page, line: &Line) -> f32 {
    page.lines
        .iter()
        .filter(|l| {
            !l.flags.furniture
                && same_column(l, line)
                && (l.top - line.top).abs() < 200.0
                && l.x1 - l.x0 > 0.5 * (line.x1 - line.x0).max(1.0)
        })
        .map(|l| l.x0)
        .fold(line.x0, f32::min)
}

/// The column's left and right edges around `line`: its lines' extent.
pub fn column_extent(page: &Page, line: &Line) -> (f32, f32) {
    let right = page
        .lines
        .iter()
        .filter(|l| {
            !l.flags.furniture
                && same_column(l, line)
                && (l.top - line.top).abs() < 200.0
                && l.x1 - l.x0 > 0.5 * (line.x1 - line.x0).max(1.0)
        })
        .map(|l| l.x1)
        .fold(line.x1, f32::max);
    (column_left(page, line), right)
}

/// The text column `line` is set in, read off the lines of that column
/// under it (what follows; above may be a single-column title block): the
/// edges most of them reach (one full-width caption or
/// display nearby does not make the column the page's width).
/// Where the lines of a statement opening there may lie: a display set out
/// across the column can lie wholly beside a short first line.
pub fn text_column(page: &Page, line: &Line) -> (f32, f32) {
    let near: Vec<&Line> = page
        .lines
        .iter()
        .filter(|l| {
            !l.flags.furniture
                && same_column(l, line)
                && l.top <= line.top + 2.0
                && line.top - l.top < 200.0
                && l.x1 - l.x0 > 0.5 * (line.x1 - line.x0).max(1.0)
        })
        .collect();
    // Most lines reach the column's edges; a paragraph's last line, a
    // display, an indent do not, and a lone caption reaches past them.
    let rank = |mut v: Vec<f32>, at: f32| {
        v.sort_by(f32::total_cmp);
        let i = ((v.len() as f32 - 1.0) * at).round().max(0.0) as usize;
        v.get(i).copied()
    };
    let left = rank(near.iter().map(|l| l.x0).collect(), 0.2).unwrap_or(line.x0);
    let right = rank(near.iter().map(|l| l.x1).collect(), 0.8).unwrap_or(line.x1);
    (left.min(line.x0), right.max(line.x1))
}

fn opens_proof(text: &str) -> bool {
    text.starts_with("Proof") || text.starts_with("PROOF")
}

fn prose(l: &Line) -> bool {
    !l.is_display() && l.math_share < 0.2
}

/// Reads a statement on, one line at a time.
pub struct Reader {
    /// Whether the statement is set in a face of its own (amsthm's italic
    /// `plain` style); None until its first line of prose says.
    pub italic: Option<bool>,
    /// The font the statement's own text is set in, once a line of it
    /// in a face of its own is read.
    face: Option<u16>,
    in_list: bool,
    left: f32,
    gap0: f32,
    body_font: u16,
}

impl Reader {
    /// A reader for a statement whose column starts at `left`, its face
    /// known or not.
    pub fn new(page: &Page, left: f32, italic: Option<bool>) -> Reader {
        Reader {
            italic,
            face: None,
            in_list: false,
            left,
            gap0: leading_gap(page),
            body_font: page.body_font,
        }
    }

    /// Whether line `j`, set under `previous` in the same column, is still
    /// the statement's. `another` says whether a line opens something of
    /// its own: a statement, a heading, a caption.
    pub fn takes(
        &mut self,
        page: &Page,
        previous: &Line,
        j: usize,
        another: &dyn Fn(usize) -> bool,
    ) -> bool {
        self.takes_line(page, previous, &page.lines[j], j, another)
    }

    /// `takes` for a line that may stand for several: a display taken
    /// whole (`settle::display_line`), `j` its first line.
    pub fn takes_line(
        &mut self,
        page: &Page,
        previous: &Line,
        l: &Line,
        j: usize,
        another: &dyn Fn(usize) -> bool,
    ) -> bool {
        let text = page.line_text(j);
        let text = text.trim_start();
        if opens_proof(text) {
            return false;
        }
        // The rest of a row the proof opens, read before its start (a hat
        // over a letter splits the row): the proof's too.
        let proof_row = page.lines.iter().enumerate().any(|(k, m)| {
            k != j
                && l.x0 - m.x1 < 2.0 * l.em()
                && (m.baseline - l.baseline).abs() < 0.3 * l.em()
                && m.x0 < l.x0
                && opens_proof(page.line_text(k).trim_start())
        });
        if proof_row {
            return false;
        }
        let item = opens_item(text) && l.x0 <= self.left + 40.0;
        // "1. Given ..." in a statement is its list, not a heading: a
        // heading is short and stands out by its size; an item follows the
        // statement's colon, or other items, or runs long.
        let introduced: String = page.text[previous.start..previous.end].iter().collect();
        let item_first = item
            && (page.body_size <= 0.0 || l.size <= 1.05 * page.body_size)
            && (self.in_list
                || introduced.trim_end().ends_with(':')
                || text.split_whitespace().count() > 6);
        if !item_first && another(j) {
            return false;
        }
        // A display, or a lone script or symbol of one set as a line.
        let fragment = |m: &Line| m.end.saturating_sub(m.start) < 4;
        let display = !prose(l) || !prose(previous) || fragment(l) || fragment(previous);
        let gap = previous.bottom - l.top;
        let em = l.em().max(previous.em());
        if gap > 3.0 * em && !display {
            return false;
        }
        let indented = l.x0 > self.left + 3.0;
        let own_face = l.font != self.body_font;
        if item {
            self.in_list = true;
            return true;
        }
        // A line of the statement's text, from the column's margin (or a
        // paragraph's indent) - not a piece of a display set out across the
        // column, which may be in the body face: "[|err(".
        let text_line = prose(l) && !fragment(l) && l.x0 <= self.left + 25.0;
        if text_line {
            match self.italic {
                None => self.italic = Some(own_face),
                Some(true) if !own_face => {
                    // Back in the body face: an item's further lines,
                    // hanging under its text, or the paragraph after.
                    return self.in_list && indented;
                }
                _ => {}
            }
        }
        let apart = gap > self.gap0 + 3.5;
        // Its own face, read on a line of its text: a statement in that
        // face goes on in it, a paragraph break within it too.
        let in_face = text_line && self.italic == Some(true) && own_face;
        if in_face && self.face.is_none() && !apart {
            self.face = Some(l.font);
        }
        if !apart || !text_line {
            if !indented && text_line {
                self.in_list = false;
            }
            return true;
        }
        if in_face && self.face == Some(l.font) {
            return true;
        }
        if !prose(previous) || fragment(previous) {
            // After a display, set apart by its spacing: the statement goes
            // on in its own face ("where the probability is taken ...");
            // an upright one goes on unindented or mid-sentence, not with a
            // new paragraph.
            if self.italic == Some(true) {
                return true;
            }
            return !indented || text.starts_with(|c: char| c.is_lowercase());
        }
        // Set apart, in prose: a list's end and the statement going on, or
        // the paragraph after the statement.
        if self.in_list && indented {
            return true;
        }
        if self.in_list {
            self.in_list = false;
            // Its own face again, or - an upright statement - at the
            // margin, not indented as a new paragraph is.
            return self.italic == Some(true) || !indented;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_items_are_read_as_items() {
        for t in [
            "(i) Every",
            "(iv) Then",
            "(a) For",
            "(12) x",
            "1. The",
            "b) It",
            "\u{2022} A",
        ] {
            assert!(opens_item(t), "{t}");
        }
        for t in [
            "(Theorem 3)",
            "The map",
            "1.2 Results",
            "(29)x",
            "A. Author",
        ] {
            assert!(!opens_item(t), "{t}");
        }
    }
}

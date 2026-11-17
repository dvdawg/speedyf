//! A page as the hover detectors read it: characters with their geometry and
//! font facts, grouped into visual lines, and one normalised text stream over
//! the whole page that every pattern is matched against.
//!
//! The normalised text exists because what a reader sees and what PDFium
//! hands back differ in small ways that defeat plain matching: a non-breaking
//! or thin space between "Fig." and "3", a word hyphenated across a line
//! ("Fig-" / "ure 3"), ligatures, a reference split over a line break. Each
//! character of the normalised text remembers the PDFium character it came
//! from, so a match can be turned back into rectangles on the page.
//!
//! PDFium-free: tests build pages out of synthetic glyphs.

use unicode_normalization::UnicodeNormalization;

/// A normalised character that stands for no PDFium character: a space put
/// in for a gap between words or between lines.
pub const NO_RAW: usize = usize::MAX;

/// One PDFium character.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glyph {
    pub ch: char,
    /// `[x, y, w, h]`, display space, y-up from the bottom-left corner. A
    /// zero width or height means PDFium gave the character no box: a space,
    /// a line break.
    pub rect: [f32; 4],
    /// Nominal font size in points; 0 when unknown.
    pub size: f32,
    /// Page-local font id. Two characters share an id exactly when they share
    /// a font; ids mean nothing across pages.
    pub font: u16,
    pub bold: bool,
    /// Set in a font reserved for mathematics.
    pub math: bool,
}

impl Glyph {
    pub fn has_box(&self) -> bool {
        self.rect[2] > 0.0 && self.rect[3] > 0.0
    }
}

/// What extraction produces for one page.
#[derive(Clone, Debug, Default)]
pub struct PageInput {
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<Glyph>,
    /// Font names by id: `fonts[id - 1]` is the name of `Glyph::font ==
    /// id`. May be empty (synthetic pages); ids then stand for themselves.
    pub fonts: Vec<String>,
}

/// Line-level facts the detectors share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineFlags {
    /// A running head or foot: page numbers, a journal name.
    pub furniture: bool,
    /// Inside a bibliography.
    pub bibliography: bool,
    /// A table-of-contents entry, or any line on a contents page.
    pub contents: bool,
}

/// One visual line, in the order the page's content stream draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Range of the line in `Page::text`.
    pub start: usize,
    pub end: usize,
    pub x0: f32,
    pub x1: f32,
    pub bottom: f32,
    pub top: f32,
    /// Bottom of the line's full-size glyphs.
    pub baseline: f32,
    /// Most common font size on the line.
    pub size: f32,
    /// Most common font on the line.
    pub font: u16,
    /// Share of the line's visible characters that are mathematics.
    pub math_share: f32,
    pub math_count: u32,
    pub flags: LineFlags,
}

impl Line {
    pub fn height(&self) -> f32 {
        (self.top - self.bottom).max(1.0)
    }

    /// The line's nominal size, or its height when the size is unknown.
    pub fn em(&self) -> f32 {
        if self.size > 0.0 {
            self.size
        } else {
            self.height()
        }
    }

    /// A line reading as mathematics rather than prose.
    pub fn is_display(&self) -> bool {
        (self.math_count >= 2 && self.math_share >= 0.35)
            || (self.math_count >= 4 && self.math_share >= 0.25)
    }

    pub fn excluded(&self) -> bool {
        self.flags.furniture || self.flags.bibliography || self.flags.contents
    }
}

/// Where a normalised character sits relative to its line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Script {
    #[default]
    Normal,
    Super,
    Sub,
}

/// A page ready for the detectors.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<Glyph>,
    /// The normalised text of every line, lines joined by a space (or
    /// nothing, where a hyphenated word was closed up).
    pub text: Vec<char>,
    /// For each character of `text`, the PDFium character it came from.
    pub raw_of: Vec<usize>,
    pub script: Vec<Script>,
    /// For each character of `text`, its line.
    pub line_of: Vec<u32>,
    pub lines: Vec<Line>,
    /// The page's running-text size and font: the most common among its
    /// non-mathematical characters.
    pub body_size: f32,
    pub body_font: u16,
    /// The page's display equations (`glyphs::page_equation_blocks`),
    /// `[x, bottom, w, h]` in the lines' y-up space, each with its number
    /// when one is set level with it: a unit no preview cuts through and a
    /// reader takes whole. Empty until `set_displays`.
    pub displays: Vec<[f32; 4]>,
    /// Font names by id, as `PageInput::fonts`.
    pub font_names: Vec<String>,
}

/// What one PDFium character becomes in the normalised text.
enum Norm {
    Space,
    /// Dropped, unless it ends a line, where it was a hyphen.
    Control,
    Chars(Vec<char>, Script),
}

fn superscript_digit(ch: char) -> Option<char> {
    Some(match ch {
        '\u{2070}' => '0',
        '\u{00B9}' => '1',
        '\u{00B2}' => '2',
        '\u{00B3}' => '3',
        '\u{2074}' => '4',
        '\u{2075}' => '5',
        '\u{2076}' => '6',
        '\u{2077}' => '7',
        '\u{2078}' => '8',
        '\u{2079}' => '9',
        _ => return None,
    })
}

fn normalize(ch: char) -> Norm {
    match ch {
        '\t' | '\n' | '\r' | ' ' => Norm::Space,
        // no-break, en/em/thin/hair spaces, narrow no-break, ideographic
        '\u{00A0}' | '\u{2000}'..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => Norm::Space,
        // soft hyphen, hyphens, dashes, minus: one dash for ranges and breaks
        '\u{00AD}' | '\u{2010}'..='\u{2015}' | '\u{2212}' => Norm::Chars(vec!['-'], Script::Normal),
        '\u{2018}' | '\u{2019}' | '\u{2032}' => Norm::Chars(vec!['\''], Script::Normal),
        '\u{201C}' | '\u{201D}' => Norm::Chars(vec!['"'], Script::Normal),
        // footnote and section marks are kept as they are
        '\u{00A7}' | '\u{00B6}' | '\u{2020}' | '\u{2021}' | '\u{2016}' | '\u{2217}' => {
            Norm::Chars(vec![ch], Script::Normal)
        }
        c if c.is_control() => Norm::Control,
        c if (c as u32) < 0x80 => Norm::Chars(vec![c], Script::Normal),
        c => {
            if let Some(digit) = superscript_digit(c) {
                return Norm::Chars(vec![digit], Script::Super);
            }
            let expanded: Vec<char> = std::iter::once(c).nfkc().collect();
            if expanded.is_empty() || expanded.iter().all(|e| e.is_whitespace()) {
                Norm::Space
            } else {
                Norm::Chars(expanded, Script::Normal)
            }
        }
    }
}

/// A character that marks a line as mathematics wherever it is set.
pub fn is_math_symbol(ch: char) -> bool {
    matches!(ch,
        '=' | '<' | '>' | '+' | '^' | '_' | '|'
        | '\u{00B1}' | '\u{00D7}' | '\u{00F7}' | '\u{00B7}'
        | '\u{0391}'..='\u{03A9}' | '\u{03B1}'..='\u{03C9}' | '\u{03D1}' | '\u{03D5}' | '\u{03F5}'
        | '\u{2190}'..='\u{21FF}' | '\u{2200}'..='\u{22FF}' | '\u{27E8}' | '\u{27E9}'
        | '\u{1D400}'..='\u{1D7FF}')
}

/// Most common value among `values`, compared at 0.1 resolution.
fn mode_f32(values: impl Iterator<Item = f32>) -> f32 {
    let mut counts: std::collections::HashMap<i32, u32> = std::collections::HashMap::new();
    for v in values {
        if v > 0.0 {
            *counts.entry((v * 10.0).round() as i32).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(k, _)| k as f32 / 10.0)
        .unwrap_or(0.0)
}

fn mode_u16(values: impl Iterator<Item = u16>) -> u16 {
    let mut counts: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(k, _)| k)
        .unwrap_or(0)
}

/// Group a page's characters into visual lines, in stream order.
///
/// A character joins the current line while it sits level with the line's
/// core and does not jump back to the left. Level is judged against the
/// line's largest glyph so far, so a superscript - smaller, raised - stays on
/// its line while the next line, a whole line height down, does not.
/// PDFium's own newlines are not trusted: it emits one at every text-object
/// boundary, including either side of a superscript.
fn group_lines(glyphs: &[Glyph]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    // centre and height of the line's largest glyph, and its right edge
    let mut core: Option<(f32, f32)> = None;
    let mut last_right = f32::MIN;
    // The previous glyph's box: a ligature's characters ("ff", "ffi") all
    // come with the ligature's one box, the later ones a jump back to its
    // start that is no new line.
    let mut last_box: Option<[f32; 4]> = None;
    for (index, glyph) in glyphs.iter().enumerate() {
        if !glyph.has_box() {
            if core.is_some() {
                current.push(index);
            }
            continue;
        }
        let [x, y, w, h] = glyph.rect;
        let centre = y + h / 2.0;
        let ligature = last_box.is_some_and(|b| {
            (b[0] - x).abs() < 0.5 && (b[1] - y).abs() < 0.5 && (b[3] - h).abs() < 0.5
        });
        let joins = core.is_some_and(|(core_centre, core_h)| {
            let span = core_h.max(h);
            (centre - core_centre).abs() <= 0.6 * span && (x >= last_right - span || ligature)
        });
        last_box = Some(glyph.rect);
        if !joins && core.is_some() {
            out.push(std::mem::take(&mut current));
            core = None;
        }
        current.push(index);
        match core {
            Some((_, core_h)) if h <= core_h => {}
            _ => core = Some((centre, h)),
        }
        last_right = x + w;
    }
    if core.is_some() {
        out.push(current);
    }
    out
}

impl Page {
    pub fn new(input: PageInput) -> Page {
        let PageInput {
            width,
            height,
            glyphs,
            fonts,
        } = input;
        let groups = group_lines(&glyphs);

        let body_size = mode_f32(
            glyphs
                .iter()
                .filter(|g| g.has_box() && !g.math && g.ch.is_alphanumeric())
                .map(|g| g.size),
        );
        let body_font = mode_u16(
            glyphs
                .iter()
                .filter(|g| {
                    g.has_box()
                        && !g.math
                        && g.ch.is_alphanumeric()
                        && (body_size <= 0.0 || (g.size - body_size).abs() < 0.3)
                })
                .map(|g| g.font),
        );

        let mut page = Page {
            width,
            height,
            body_size,
            body_font,
            font_names: fonts,
            ..Page::default()
        };
        for group in groups {
            page.push_line(&glyphs, &group);
        }
        page.glyphs = glyphs;
        page
    }

    fn push_line(&mut self, glyphs: &[Glyph], indices: &[usize]) {
        let boxed: Vec<&Glyph> = indices
            .iter()
            .map(|&i| &glyphs[i])
            .filter(|g| g.has_box())
            .collect();
        if boxed.is_empty() {
            return;
        }
        let size = mode_f32(
            boxed
                .iter()
                .filter(|g| !g.ch.is_whitespace())
                .map(|g| g.size),
        );
        let font = mode_u16(
            boxed
                .iter()
                .filter(|g| g.ch.is_alphanumeric() && (size <= 0.0 || (g.size - size).abs() < 0.3))
                .map(|g| g.font),
        );
        let max_h = boxed.iter().map(|g| g.rect[3]).fold(0.0f32, f32::max);
        let mut full: Vec<f32> = boxed
            .iter()
            .filter(|g| {
                if size > 0.0 && g.size > 0.0 {
                    g.size >= 0.9 * size
                } else {
                    g.rect[3] >= 0.8 * max_h
                }
            })
            .map(|g| g.rect[1])
            .collect();
        full.sort_by(f32::total_cmp);
        let baseline = full
            .get(full.len() / 2)
            .copied()
            .unwrap_or(boxed[0].rect[1]);
        let line_h = if size > 0.0 { size } else { max_h.max(1.0) };

        let mut x0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut bottom = f32::MAX;
        let mut top = f32::MIN;
        let mut math_count = 0u32;
        let mut visible = 0u32;
        for g in &boxed {
            x0 = x0.min(g.rect[0]);
            x1 = x1.max(g.rect[0] + g.rect[2]);
            bottom = bottom.min(g.rect[1]);
            top = top.max(g.rect[1] + g.rect[3]);
            if !g.ch.is_whitespace() {
                visible += 1;
                if g.math || is_math_symbol(g.ch) {
                    math_count += 1;
                }
            }
        }

        let line_index = self.lines.len() as u32;
        // Join to the previous line: a hyphenated word closes up, anything
        // else is a space.
        if !self.text.is_empty() {
            let first_letter = boxed.first().map(|g| g.ch);
            let ends_hyphen = self.text.last() == Some(&'-')
                && self
                    .text
                    .len()
                    .checked_sub(2)
                    .and_then(|i| self.text.get(i))
                    .is_some_and(|c| c.is_alphabetic());
            match first_letter {
                Some(next) if ends_hyphen && next.is_alphabetic() => {
                    if next.is_lowercase() {
                        self.pop_char();
                    }
                }
                _ => self.push_char(' ', NO_RAW, Script::Normal, line_index.saturating_sub(1)),
            }
        }
        let start = self.text.len();

        let mut pending_space = false;
        let mut previous_right: Option<f32> = None;
        let mut last_was_control = false;
        for &index in indices {
            let glyph = &glyphs[index];
            let norm = normalize(glyph.ch);
            if !glyph.has_box() {
                // PDFium reports a line break at every text-object boundary,
                // either side of a superscript included; only a real space
                // separates words. A gap on the page does too, below.
                if matches!(norm, Norm::Space) && glyph.ch != '\n' && glyph.ch != '\r' {
                    pending_space = true;
                }
                continue;
            }
            if let Some(right) = previous_right {
                if glyph.rect[0] - right > 0.25 * line_h {
                    pending_space = true;
                }
            }
            previous_right = Some(glyph.rect[0] + glyph.rect[2]);
            let script = {
                let smaller = if size > 0.0 && glyph.size > 0.0 {
                    glyph.size <= 0.9 * size
                } else {
                    glyph.rect[3] <= 0.8 * max_h
                };
                let offset = glyph.rect[1] - baseline;
                if smaller && offset >= 0.15 * line_h {
                    Script::Super
                } else if smaller && offset <= -0.15 * line_h {
                    Script::Sub
                } else {
                    Script::Normal
                }
            };
            match norm {
                Norm::Space => pending_space = true,
                Norm::Control => last_was_control = true,
                Norm::Chars(chars, own) => {
                    last_was_control = false;
                    if pending_space && self.text.len() > start && self.text.last() != Some(&' ') {
                        self.push_char(' ', NO_RAW, Script::Normal, line_index);
                    }
                    pending_space = false;
                    let script = if own == Script::Super {
                        Script::Super
                    } else {
                        script
                    };
                    for ch in chars {
                        self.push_char(ch, index, script, line_index);
                    }
                }
            }
        }
        // An unmappable character at the end of a line is the hyphen a word
        // was broken with.
        if last_was_control && self.text.last().is_some_and(|c| c.is_alphabetic()) {
            let raw = *indices.last().unwrap_or(&NO_RAW);
            self.push_char('-', raw, Script::Normal, line_index);
        }
        let end = self.text.len();
        if end == start {
            return;
        }
        self.lines.push(Line {
            start,
            end,
            x0,
            x1,
            bottom,
            top,
            baseline,
            size,
            font,
            math_share: if visible == 0 {
                0.0
            } else {
                math_count as f32 / visible as f32
            },
            math_count,
            flags: LineFlags::default(),
        });
    }

    fn push_char(&mut self, ch: char, raw: usize, script: Script, line: u32) {
        self.text.push(ch);
        self.raw_of.push(raw);
        self.script.push(script);
        self.line_of.push(line);
    }

    fn pop_char(&mut self) {
        self.text.pop();
        self.raw_of.pop();
        self.script.pop();
        self.line_of.pop();
    }

    /// Take the page's display equations: `[x, bottom, w, h]` y-up, with
    /// the number the detector read, which it leaves outside the box - the
    /// line carrying "(n)" level with the display, to its right, joins it.
    pub fn set_displays(&mut self, blocks: Vec<([f32; 4], Option<String>)>) {
        self.displays = blocks
            .into_iter()
            .filter(|(b, _)| b.iter().all(|v| v.is_finite()) && b[2] > 0.0 && b[3] > 0.0)
            .map(|(mut b, number)| {
                let Some(number) = number else { return b };
                let tag = format!("({number})");
                let (bottom, top) = (b[1], b[1] + b[3]);
                for (i, l) in self.lines.iter().enumerate() {
                    let centre = (l.top + l.bottom) / 2.0;
                    // A line of its own at the margin, or the display's
                    // row ending in it.
                    if centre >= bottom - 2.0
                        && centre <= top + 2.0
                        && l.x1 >= b[0] + b[2]
                        && l.x0 <= b[0] + b[2] + 0.5 * self.width
                        && (l.x0 >= b[0] + 0.5 * b[2] || l.x0 <= b[0] + b[2])
                        && self.line_text(i).trim_end().ends_with(&tag)
                    {
                        let x1 = l.x1.max(b[0] + b[2]);
                        let y0 = l.bottom.min(b[1]);
                        let y1 = l.top.max(b[1] + b[3]);
                        b = [b[0], y0, x1 - b[0], y1 - y0];
                        break;
                    }
                }
                b
            })
            .map(|b| self.clear_of_prose(b))
            .collect();
    }

    /// A display's box kept off the lines above and below it that are no
    /// part of it: the detector's box can reach a point or two into the
    /// line over it, which would tie that line to the display. Never less
    /// than the display's own lines.
    fn clear_of_prose(&self, b: [f32; 4]) -> [f32; 4] {
        let (x0, x1) = (b[0], b[0] + b[2]);
        let (b0, t0) = (b[1], b[1] + b[3]);
        let (mut bottom, mut top) = (b0, t0);
        let member = |l: &Line| {
            let (cx, cy) = ((l.x0 + l.x1) / 2.0, (l.top + l.bottom) / 2.0);
            cx >= x0 - 1.0 && cx <= x1 + 1.0 && cy >= b0 - 1.0 && cy <= t0 + 1.0
        };
        let own: Vec<&Line> = self
            .lines
            .iter()
            .filter(|l| !l.flags.furniture && member(l))
            .collect();
        let own_top = own.iter().map(|l| l.top).fold(f32::MIN, f32::max);
        let own_bottom = own.iter().map(|l| l.bottom).fold(f32::MAX, f32::min);
        for l in &self.lines {
            if l.flags.furniture || member(l) || l.x1.min(x1) - l.x0.max(x0) <= 3.0 {
                continue;
            }
            let centre = (l.top + l.bottom) / 2.0;
            if centre > top && l.bottom < top {
                top = l.bottom.max(own_top).min(top);
            } else if centre < bottom && l.top > bottom {
                bottom = l.top.min(own_bottom).max(bottom);
            }
        }
        if top <= bottom {
            return b;
        }
        [b[0], bottom, b[2], top - bottom]
    }

    /// The name of font `id`, or None when the page carries no names.
    pub fn font_name(&self, id: u16) -> Option<&str> {
        (id as usize)
            .checked_sub(1)
            .and_then(|i| self.font_names.get(i))
            .map(String::as_str)
    }

    /// The text of one line.
    pub fn line_text(&self, line: usize) -> String {
        let l = &self.lines[line];
        self.text[l.start..l.end].iter().collect()
    }

    /// The glyph behind a normalised character, if there is one.
    pub fn glyph_at(&self, at: usize) -> Option<&Glyph> {
        self.raw_of
            .get(at)
            .filter(|&&raw| raw != NO_RAW)
            .and_then(|&raw| self.glyphs.get(raw))
    }

    /// Rectangles covering `text[start..end]`, one per line it touches.
    pub fn rects(&self, start: usize, end: usize) -> Vec<[f32; 4]> {
        let mut out: Vec<(u32, [f32; 4])> = Vec::new();
        for at in start..end.min(self.text.len()) {
            let Some(glyph) = self.glyph_at(at) else {
                continue;
            };
            if !glyph.has_box() {
                continue;
            }
            let line = self.line_of[at];
            let [x, y, w, h] = glyph.rect;
            match out.iter_mut().find(|(l, _)| *l == line) {
                Some((_, r)) => {
                    let right = (r[0] + r[2]).max(x + w);
                    let top = (r[1] + r[3]).max(y + h);
                    r[0] = r[0].min(x);
                    r[1] = r[1].min(y);
                    r[2] = right - r[0];
                    r[3] = top - r[1];
                }
                None => out.push((line, [x, y, w, h])),
            }
        }
        out.into_iter().map(|(_, r)| r).collect()
    }

    /// Whether any character of `text[start..end]` is set in a math font.
    pub fn any_math(&self, start: usize, end: usize) -> bool {
        (start..end.min(self.text.len())).any(|at| self.glyph_at(at).is_some_and(|g| g.math))
    }

    /// The one font every letter and digit of `text[start..end]` is set in,
    /// or None when they are set in several.
    pub fn font_of(&self, start: usize, end: usize) -> Option<u16> {
        let mut fonts = (start..end.min(self.text.len()))
            .filter(|&at| self.text[at].is_alphanumeric())
            .filter_map(|at| self.glyph_at(at).map(|g| g.font));
        let first = fonts.next()?;
        fonts.all(|f| f == first).then_some(first)
    }

    /// Whether every letter and digit of `text[start..end]` is bold.
    pub fn all_bold(&self, start: usize, end: usize) -> bool {
        let mut any = false;
        for at in start..end.min(self.text.len()) {
            if !self.text[at].is_alphanumeric() {
                continue;
            }
            match self.glyph_at(at) {
                Some(g) if g.bold => any = true,
                _ => return false,
            }
        }
        any
    }

    /// Size of the glyphs in `text[start..end]` (most common), or 0.
    pub fn size_of(&self, start: usize, end: usize) -> f32 {
        mode_f32(
            (start..end.min(self.text.len()))
                .filter(|&at| self.text[at].is_alphanumeric())
                .filter_map(|at| self.glyph_at(at).map(|g| g.size)),
        )
    }

    /// The line a normalised character sits on.
    pub fn line_at(&self, at: usize) -> Option<&Line> {
        self.line_of
            .get(at)
            .and_then(|&l| self.lines.get(l as usize))
    }

    /// Whether the line before `line` (in stream order) runs on into it: same
    /// column, set close above, in the same size, not ending a sentence. A
    /// label at the start of such a line is mid-sentence, not a heading.
    pub fn continues_previous(&self, line: usize) -> bool {
        let Some(previous) = line.checked_sub(1).and_then(|p| self.lines.get(p)) else {
            return false;
        };
        let current = &self.lines[line];
        if !same_column(previous, current) {
            return false;
        }
        let em = previous.em().max(current.em());
        let pitch = previous.baseline - current.baseline;
        if pitch <= 0.0 || pitch > 1.6 * em {
            return false;
        }
        if previous.size > 0.0 && current.size > 0.0 {
            let ratio = previous.size / current.size;
            if !(0.92..=1.09).contains(&ratio) {
                return false;
            }
        }
        let text = self.line_text(line - 1);
        let last = text.trim_end().chars().last();
        !matches!(last, Some('.' | ':' | '!' | '?'))
    }
}

/// Whether two lines share a column: they overlap horizontally by a good
/// part of the narrower one.
pub fn same_column(a: &Line, b: &Line) -> bool {
    let overlap = a.x1.min(b.x1) - a.x0.max(b.x0);
    let narrower = (a.x1 - a.x0).min(b.x1 - b.x0).max(1.0);
    overlap >= 0.3 * narrower
}

/// The normalised text as a string, for tests and diagnostics.
#[cfg(test)]
pub fn text_of(page: &Page) -> String {
    page.text.iter().collect()
}

#[cfg(test)]
pub mod build {
    //! Synthetic pages for tests: lines of text set in a font of a given
    //! size at a given position, one glyph per character with a fixed
    //! advance.
    use super::*;

    pub struct Builder {
        pub width: f32,
        pub height: f32,
        pub glyphs: Vec<Glyph>,
    }

    #[derive(Clone, Copy)]
    pub struct Style {
        pub size: f32,
        pub font: u16,
        pub bold: bool,
        pub math: bool,
    }

    pub const BODY: Style = Style {
        size: 10.0,
        font: 1,
        bold: false,
        math: false,
    };
    pub const BOLD: Style = Style {
        size: 10.0,
        font: 2,
        bold: true,
        math: false,
    };
    pub const SMALL: Style = Style {
        size: 8.0,
        font: 3,
        bold: false,
        math: false,
    };
    pub const MATH: Style = Style {
        size: 10.0,
        font: 4,
        bold: false,
        math: true,
    };
    pub const SCRIPT: Style = Style {
        size: 7.0,
        font: 1,
        bold: false,
        math: false,
    };
    pub const ITALIC: Style = Style {
        size: 10.0,
        font: 6,
        bold: false,
        math: false,
    };

    impl Builder {
        pub fn new() -> Self {
            Builder {
                width: 612.0,
                height: 792.0,
                glyphs: Vec::new(),
            }
        }

        /// Characters of `text` from `x` on the baseline `y`, each `0.5 *
        /// size` wide; returns the x after the last one. Spaces are emitted
        /// as box-less characters, the way PDFium reports most of them.
        pub fn text(&mut self, x: f32, y: f32, style: Style, text: &str) -> f32 {
            let advance = 0.5 * style.size;
            let mut at = x;
            for ch in text.chars() {
                if ch == ' ' {
                    self.glyphs.push(Glyph {
                        ch,
                        size: style.size,
                        font: style.font,
                        ..Glyph::default()
                    });
                } else {
                    self.glyphs.push(Glyph {
                        ch,
                        rect: [at, y, advance * 0.95, 0.75 * style.size],
                        size: style.size,
                        font: style.font,
                        bold: style.bold,
                        math: style.math,
                    });
                }
                at += advance;
            }
            at
        }

        /// A raised script after the text ending at `x` on baseline `y`.
        pub fn sup(&mut self, x: f32, y: f32, text: &str) -> f32 {
            self.text(x, y + 3.5, SCRIPT, text)
        }

        /// A line break, as PDFium reports one.
        pub fn newline(&mut self) {
            self.glyphs.push(Glyph {
                ch: '\n',
                ..Glyph::default()
            });
        }

        /// One line of `style` text at the left margin, then a break.
        pub fn line(&mut self, y: f32, style: Style, text: &str) -> f32 {
            self.line_at(72.0, y, style, text)
        }

        /// One line of `style` text from `x`, then a break.
        pub fn line_at(&mut self, x: f32, y: f32, style: Style, text: &str) -> f32 {
            let end = self.text(x, y, style, text);
            self.newline();
            end
        }

        pub fn page(self) -> Page {
            Page::new(PageInput {
                width: self.width,
                height: self.height,
                glyphs: self.glyphs,
                fonts: Vec::new(),
            })
        }
    }

    impl Default for Builder {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn lines_are_grouped_and_joined_with_spaces() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "First line of text");
        b.line(688.0, BODY, "second line.");
        let page = b.page();
        assert_eq!(page.lines.len(), 2);
        assert_eq!(text_of(&page), "First line of text second line.");
    }

    #[test]
    fn a_word_hyphenated_across_lines_is_closed_up() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "as shown in Fig-");
        b.line(688.0, BODY, "ure 3 the curve");
        let page = b.page();
        assert_eq!(text_of(&page), "as shown in Figure 3 the curve");
    }

    #[test]
    fn a_hyphen_before_a_capital_is_kept() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "a non-");
        b.line(688.0, BODY, "FDT regime");
        assert_eq!(text_of(&b.page()), "a non-FDT regime");
    }

    #[test]
    fn non_breaking_and_thin_spaces_are_spaces() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "Fig.\u{00A0}3 and Eq.\u{2009}(2)");
        assert_eq!(text_of(&b.page()), "Fig. 3 and Eq. (2)");
    }

    #[test]
    fn a_raised_small_glyph_stays_on_its_line_as_a_superscript() {
        let mut b = Builder::new();
        let end = b.text(72.0, 700.0, BODY, "word");
        b.newline();
        b.sup(end, 700.0, "1");
        b.newline();
        b.text(end + 4.0, 700.0, BODY, " more");
        b.newline();
        b.line(688.0, BODY, "next");
        let page = b.page();
        assert_eq!(page.lines.len(), 2);
        let at = page.text.iter().position(|c| *c == '1').unwrap();
        assert_eq!(page.script[at], Script::Super);
        assert_eq!(page.script[0], Script::Normal);
    }

    #[test]
    fn a_ligatures_characters_sharing_its_box_stay_on_their_line() {
        // "efficiency": PDFium gives each character of the "ffi" ligature
        // the ligature's one box, a jump back to its start.
        let mut b = Builder::new();
        let x = b.text(72.0, 700.0, BODY, "e");
        for ch in ['f', 'f', 'i'] {
            b.glyphs.push(Glyph {
                ch,
                rect: [x, 700.0, 12.0, 7.5],
                size: 10.0,
                font: 1,
                ..Glyph::default()
            });
        }
        b.text(x + 12.5, 700.0, BODY, "ciency of it");
        b.newline();
        b.line(688.0, BODY, "next line");
        let page = b.page();
        assert_eq!(page.lines.len(), 2, "{:?}", text_of(&page));
        assert!(
            page.line_text(0).starts_with("efficiency"),
            "{:?}",
            page.line_text(0)
        );
    }

    #[test]
    fn rects_split_per_line() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "see Eq.");
        b.line(688.0, BODY, "(12) here");
        let page = b.page();
        let text = text_of(&page);
        let start = text.find("Eq.").unwrap();
        let end = text.find("(12)").unwrap() + 4;
        let rects = page.rects(start, end);
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn body_size_is_the_most_common() {
        let mut b = Builder::new();
        for i in 0..5 {
            b.line(700.0 - 12.0 * i as f32, BODY, "body text here");
        }
        b.line(600.0, SMALL, "a note");
        let page = b.page();
        assert_eq!(page.body_size, 10.0);
        assert_eq!(page.body_font, 1);
    }

    #[test]
    fn a_line_continuing_a_sentence_is_detected() {
        let mut b = Builder::new();
        b.line(700.0, BODY, "the result is shown in");
        b.line(688.0, BODY, "Fig. 3. Then more.");
        b.line(660.0, BODY, "Fig. 4. New paragraph");
        let page = b.page();
        assert!(page.continues_previous(1));
        assert!(!page.continues_previous(2));
    }
}

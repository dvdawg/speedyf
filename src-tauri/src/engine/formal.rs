/// Recovering formal environments — theorems, lemmas, definitions, figures —
/// from a LaTeX-compiled PDF.
///
/// hyperref anchors every stepped counter with a named destination, so a paper
/// carries `theorem.3.1`, `figure.2`, `definition.1.4` and friends whether or
/// not the author ever wrote a `\label`. That gives a complete, precisely
/// positioned candidate list.
///
/// What it does NOT give is the name to show. A destination carries the LaTeX
/// *counter*, and the near-universal `\newtheorem{lemma}[theorem]{Lemma}`
/// idiom shares one counter across lemmas, corollaries and propositions — so
/// all three anchor as `theorem.N`. Trusting the destination name would label
/// every corollary in the document "Theorem".
///
/// The printed text is what actually knows. So the destination supplies the
/// anchor and the number, the text at that anchor supplies the real name, and
/// the number appearing in *both* is what lets us reject a bad pairing rather
/// than emit a mislabelled entry.
use crate::engine::tagged::MathSpan;
use crate::engine::types::FormalEntryDto;
use pdfium_render::prelude::{PdfiumLibraryBindings, FPDF_DOCUMENT};

/// Destinations hyperref emits that are never formal environments. Most junk
/// would fail the text check anyway; skipping these up front avoids doing the
/// text work for them at all.
const STRUCTURAL_PREFIXES: &[&str] = &[
    "page.",
    "subsubsection.",
    "part.",
    "paragraph.",
    "subparagraph.",
    "cite.",
    "Item.",
    // thmtools anchors every theorem-like environment on a dummy counter as
    // well as its own, so half a paper's destinations are these
    "thmt@dummyctr",
    // unnumbered headings: nothing to verify a name against
    "section*.",
    "chapter*.",
    "Hfootnote.",
    "footnote.",
    // Anchor text for an equation is the equation itself, so it can never pass
    // the name check — and a list of every numbered equation is noise anyway.
    "equation.",
];

/// Headings group the list rather than populate it. The depth is the nesting
/// the panel and the breadcrumb show: section, then subsection, then the
/// environments themselves.
pub fn heading_depth(name: &str) -> Option<u8> {
    if name.starts_with("section.") || name.starts_with("chapter.") || name.starts_with("appendix.")
    {
        Some(0)
    } else if name.starts_with("subsection.") {
        Some(1)
    } else {
        None
    }
}

pub fn is_section_destination(name: &str) -> bool {
    heading_depth(name).is_some()
}

pub fn is_structural_destination(name: &str) -> bool {
    name == "Doc-Start"
        || name == "Navigation"
        || STRUCTURAL_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The counter value encoded in a destination name: `theorem.3.1` -> `3.1`.
/// Returns None when the name has no dotted numeric tail.
pub fn destination_number(name: &str) -> Option<&str> {
    let (_, tail) = name.split_once('.')?;
    is_counter_value(tail).then_some(tail)
}

/// A printed counter: dotted components that are each a number, or a short
/// letter like the `A` of an appendix. The letter length is what keeps whole
/// words out, so `table.of.contents` is not mistaken for a counter.
fn is_counter_value(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && (part.chars().all(|c| c.is_ascii_digit())
                    || (part.len() <= 3 && part.chars().all(|c| c.is_ascii_alphabetic())))
        })
}

#[derive(Debug, PartialEq, Eq)]
pub struct Heading {
    /// the word as printed: "Theorem", "Lemma", "Figure"
    pub kind: String,
    pub number: String,
}

/// Parse the head of the text sitting at an anchor.
///
/// Handles the shapes LaTeX actually produces, including the missing space in
/// `Theorem 1.1(Name).` that PDF text extraction hands back:
///   `Theorem 1.1.`            -> kind/number, no title
///   `Theorem 1.1 (Main).`     -> title "Main"
///   `Lemma 2.`                -> shared-counter environments keep their name
///   `Figure 3: A caption.`    -> title "A caption"
pub fn parse_heading(text: &str) -> Option<Heading> {
    let text = text.trim_start();
    let mut chars = text.char_indices().peekable();

    // A leading capitalized word.
    let start = chars.peek()?.0;
    if !text[start..].chars().next()?.is_uppercase() {
        return None;
    }
    let mut end = start;
    while let Some(&(i, c)) = chars.peek() {
        if c.is_alphabetic() {
            end = i + c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    let kind = &text[start..end];
    if kind.len() < 3 {
        return None;
    }

    // Whitespace, then the counter.
    let rest = text[end..].trim_start();
    let bytes = rest.as_bytes();
    let mut n = 0usize;

    // The first component is either a number ("4") or an appendix letter
    // ("B"). Without the letter case every environment in an appendix is
    // invisible: "Lemma B.1" is anchored on `theorem.B.1`, and a digits-only
    // counter cannot read either side of that.
    if bytes.first().is_some_and(u8::is_ascii_uppercase) {
        while n < bytes.len() && n < 3 && bytes[n].is_ascii_uppercase() {
            n += 1;
        }
    } else {
        while n < bytes.len() && bytes[n].is_ascii_digit() {
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }

    // Dotted numeric components. A '.' belongs to the counter only when a
    // digit follows it, so the sentence-ending period in "Theorem 1.1." is
    // left alone.
    while n + 1 < bytes.len() && bytes[n] == b'.' && bytes[n + 1].is_ascii_digit() {
        n += 1;
        while n < bytes.len() && bytes[n].is_ascii_digit() {
            n += 1;
        }
    }

    // A counter ends at a boundary. Without this "Lemma Suppose that..."
    // reads as the counter "S".
    if bytes.get(n).is_some_and(u8::is_ascii_alphanumeric) {
        return None;
    }
    let number = &rest[..n];

    Some(Heading {
        kind: kind.to_string(),
        number: number.to_string(),
    })
}

/// Pair a destination with the text found at it. Returns None unless the two
/// agree on the number — that agreement is the whole verification story.
pub fn reconcile(dest_name: &str, anchor_text: &str) -> Option<Heading> {
    if is_structural_destination(dest_name) {
        return None;
    }
    find_heading(anchor_text, destination_number(dest_name)?)
}

/// The heading somewhere in the text after an anchor, if its number is the one
/// the destination names.
///
/// hyperref fires an anchor as the counter steps — before the environment's
/// own vertical space and heading — so it routinely lands on the last line of
/// the *preceding* paragraph, leaving the heading a line or two further down:
///
///   "as follows.  Definition 1.2 (Gaussian Mixtures). The ..."
///
/// Demanding the heading start the text dropped most of a real paper. Scanning
/// forward is safe precisely because the number still has to match: another
/// environment's heading caught in the window simply does not. */
fn find_heading(text: &str, dest_number: &str) -> Option<Heading> {
    let mut at_boundary = true;
    for (i, ch) in text.char_indices() {
        if i > HEADING_SEARCH_CHARS {
            break;
        }
        if at_boundary && ch.is_uppercase() {
            if let Some(heading) = parse_heading(&text[i..]) {
                if heading.number == dest_number {
                    return Some(heading);
                }
            }
        }
        at_boundary = ch.is_whitespace();
    }
    None
}

/// A destination's position in the document.
pub struct Anchor {
    pub name: String,
    pub page: u32,
    /// x in page space, when the destination specifies one
    pub x: Option<f32>,
    pub y: f32,
}

/// Backstop against a pathological document; real papers have hundreds.
const MAX_NAMED_DESTS: u64 = 200_000;
const MAX_DEST_NAME_BYTES: i64 = 64 * 1024;

/// Every named destination that could plausibly be a formal environment or a
/// section heading, as `(name, page index, y in page space)`.
///
/// PDFium's two-call idiom: pass a null buffer to learn the length, then call
/// again to fill it. The name comes back as UTF-16, NUL-terminated.
pub fn enumerate_anchors(
    b: &dyn PdfiumLibraryBindings,
    raw: FPDF_DOCUMENT,
    page_count: u32,
) -> Vec<Anchor> {
    // FPDF_DWORD is c_ulong: 64-bit on Unix, 32-bit on Windows. Clippy only
    // ever sees the target it runs on, so on Unix it reads as a no-op cast —
    // but dropping it fails to compile on Windows, where it is a real widening.
    #[allow(clippy::unnecessary_cast)]
    let total = (b.FPDF_CountNamedDests(raw) as u64).min(MAX_NAMED_DESTS);
    let mut anchors = Vec::new();
    for index in 0..total {
        let mut buflen: std::os::raw::c_long = 0;
        let dest = b.FPDF_GetNamedDest(raw, index as i32, std::ptr::null_mut(), &mut buflen);
        if dest.is_null() || buflen <= 0 || buflen as i64 > MAX_DEST_NAME_BYTES {
            continue;
        }
        let mut units = vec![0u16; (buflen as usize).div_ceil(2)];
        let dest = b.FPDF_GetNamedDest(
            raw,
            index as i32,
            units.as_mut_ptr() as *mut std::os::raw::c_void,
            &mut buflen,
        );
        if dest.is_null() || buflen <= 0 {
            continue;
        }
        units.truncate((buflen as usize).div_ceil(2).min(units.len()));
        let name = String::from_utf16_lossy(&units);
        let name = name.trim_end_matches('\0');
        if name.is_empty() || is_structural_destination(name) || destination_number(name).is_none()
        {
            continue;
        }
        let page_index = b.FPDFDest_GetDestPageIndex(raw, dest);
        if page_index < 0 || page_index as u32 >= page_count {
            continue;
        }
        let (mut has_x, mut has_y, mut has_zoom) = (0, 0, 0);
        let (mut x, mut y, mut zoom) = (0.0f32, 0.0f32, 0.0f32);
        b.FPDFDest_GetLocationInPage(
            dest,
            &mut has_x,
            &mut has_y,
            &mut has_zoom,
            &mut x,
            &mut y,
            &mut zoom,
        );
        if has_y == 0 {
            continue;
        }
        anchors.push(Anchor {
            name: name.to_string(),
            page: page_index as u32,
            x: (has_x != 0).then_some(x),
            y,
        });
    }
    anchors
}

/// How far below a hyperref anchor a line's top may sit and still be its line.
const ANCHOR_TOLERANCE_PT: f32 = 6.0;
/// Runs whose distance from the anchor differs by less than this are treated
/// as the same line of text.
const LINE_GROUPING_PT: f32 = 5.0;
/// How far left of a destination's own x a character may start and still be
/// counted as part of the text it points at.
const ANCHOR_X_TOLERANCE_PT: f32 = 4.0;
const ANCHOR_TEXT_CHARS: usize = 400;
/// How far into that text a heading may start and still belong to the anchor.
const HEADING_SEARCH_CHARS: usize = 320;

/// Index of the first character an anchor points at, in the page's character
/// stream. Search matches carry indices into that same stream, which is what
/// lets a hit be attributed to the environment containing it without going
/// anywhere near geometry.
pub fn anchor_start(boxes: &[[f32; 4]], anchor_x: Option<f32>, anchor_y: f32) -> Option<usize> {
    anchored_line(boxes, anchor_x, anchor_y).map(|(start, _)| start)
}

/// Text starting at a destination anchor, in reading order, capped at enough
/// characters to cover a name or caption.
///
/// The anchor sits at the line's top while a run's `y` is the bottom of its
/// box (display space is y-up), so we compare against each run's top edge.
///
/// The subtlety is that PDFium hands back one run per *word*, and a word's box
/// top varies by a point or so with its ascenders — so the run closest to the
/// anchor is whichever word happens to sit lowest, not the first one. Taking it
/// would start the text mid-line: for `Theorem 1.1 (Main).` the nearest run is
/// `(Main).`, which loses the very name we came for. So: find the nearest line,
/// then take the first run *on* that line.
/// Index of the first character on the line an anchor points at, plus that
/// line's distance below the anchor.
///
/// The anchor sits at the line's top while a box's `y` is its bottom (display
/// space is y-up), so we compare against each box's top edge. Characters are
/// matched rather than runs because a run's box top varies by a point or so
/// with its ascenders: the *nearest* thing to the anchor is whichever glyph
/// happens to sit lowest, not the first one. For `Theorem 1.1 (Main).` that is
/// the paren — starting there would lose the very name we came for. So: find
/// the nearest line, then take the first character on it.
fn anchored_line(boxes: &[[f32; 4]], anchor_x: Option<f32>, anchor_y: f32) -> Option<(usize, f32)> {
    // Characters left of the destination's own x are not its text. This is
    // what keeps the arXiv stamp out: it runs rotated down the margin at
    // x≈13 while the text block starts at x≈134, its glyphs span the whole
    // page height so one of them sits at almost any anchor's y, and being
    // background content it is drawn *first* — so it holds the lowest
    // character indices and won every "first match on this line" search.
    // It cost the panel a theorem per collision and, when the stamp happened
    // to start with the right digit, produced a section named after the paper.
    let in_column = |b: &[f32; 4]| anchor_x.is_none_or(|x| b[0] >= x - ANCHOR_X_TOLERANCE_PT);
    let top_of = |b: &[f32; 4]| (b[2] > 0.0 && b[3] > 0.0 && in_column(b)).then(|| b[1] + b[3]);
    let nearest = boxes
        .iter()
        .filter_map(top_of)
        .map(|top| anchor_y - top)
        .filter(|gap| *gap >= -ANCHOR_TOLERANCE_PT)
        .fold(f32::INFINITY, f32::min);
    if !nearest.is_finite() {
        return None;
    }
    let start = boxes.iter().position(|b| {
        top_of(b).is_some_and(|top| {
            let gap = anchor_y - top;
            gap >= -ANCHOR_TOLERANCE_PT && gap <= nearest + LINE_GROUPING_PT
        })
    })?;
    Some((start, nearest))
}

fn take_chars(raw: &str, start: usize, limit: usize) -> String {
    raw.chars().skip(start).take(limit).collect()
}

/// Text starting at a destination anchor, in reading order, capped at enough
/// characters to cover a name or caption.
///
/// Reads the raw character stream rather than the text runs. The run builder
/// strips inter-word spaces — a caption arrives as `Aplaceholderfigure.` —
/// while the stream PDFium hands back still has them, and its indices line up
/// with the character boxes one-for-one.
pub fn anchor_text(
    raw: &str,
    boxes: &[[f32; 4]],
    anchor_x: Option<f32>,
    anchor_y: f32,
) -> Option<String> {
    let (start, _) = anchored_line(boxes, anchor_x, anchor_y)?;
    let text = take_chars(raw, start, ANCHOR_TEXT_CHARS);
    (!text.trim().is_empty()).then_some(text)
}

/// The text of just the line a destination anchors, stopping where that line
/// does.
///
/// `anchor_text` deliberately reads on past the line end, because a theorem's
/// name can spill onto the next one. A section heading is the opposite case:
/// reading on swallows the paragraph beneath it, so a table of contents built
/// that way prints the opening sentence of every section.
///
/// The line is tracked by glyph *bottoms*, not tops. Tops vary enormously
/// within one line — in "4. Canonicality" the period's box is 2pt tall
/// against the digit's 8.2pt, so its top sits 6pt lower and a top-based
/// comparison ends the heading at the period, leaving a table of contents
/// that reads "4", "5", "6". Bottoms sit on the shared baseline, varying only
/// by a descender, while the next line is a whole line-height away.
pub fn anchor_line_text(
    raw: &str,
    boxes: &[[f32; 4]],
    meta: GlyphMeta<'_>,
    anchor_x: Option<f32>,
    anchor_y: f32,
) -> Option<String> {
    let (start, _) = anchored_line(boxes, anchor_x, anchor_y)?;
    let chars: Vec<char> = raw.chars().collect();
    let (text, _, _, _, _) = line_at(&chars, boxes, meta, start);
    (!text.is_empty()).then_some(text)
}

/// Baseline spread tolerated within one heading line, and the *only* thing
/// that ends one.
///
/// Not the newline PDFium emits, which does not mean what it looks like: a
/// superscript is a separate text object, so "4.1. L2 Optimality" arrives as
/// `4.1. L` `\r\n` `2 Optimality`, and breaking there truncates the title to
/// "4.1. L". Baselines are unambiguous — that superscript sits 3.6pt above its
/// line while the next line is 18.6pt below it. Wider than `LINE_GROUPING_PT`
/// to clear scripts, still well inside normal leading.
const HEADING_LINE_BAND_PT: f32 = 8.0;

/// Where a heading actually is, as opposed to where its destination pointed.
///
/// The position matters as much as the text: it is what document order sorts
/// on and what clicking the entry scrolls to, and the anchor is wrong about
/// both often enough to matter.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadingHit {
    pub title: String,
    /// baseline of the heading line, display space y-up
    pub y: f32,
    /// left edge of the heading line
    pub x: f32,
}

/// How far a heading's left edge may sit from its destination's x and still be
/// the same column. A heading starts at its column's margin, which is where
/// hyperref's anchor x points; anything further is another column.
const HEADING_X_TOLERANCE_PT: f32 = 12.0;

/// Whether `line` opens with the counter `counter` and then a title.
///
/// The trailing check for a letter is what separates "4. Canonicality" from a
/// line of arithmetic that happens to open with the same digit, and stops the
/// counter "4" from claiming the subsection line "4.1. ...".
fn opens_with_counter(line: &str, counter: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix(counter) else {
        return false;
    };
    let rest = rest.trim_start_matches(['.', ':', ')', ' ']);
    rest.chars().next().is_some_and(char::is_alphabetic)
}

/// Horizontal gap, relative to line height, that reads as a missing space
/// rather than ordinary letter spacing.
const WORD_GAP_RATIO: f32 = 0.35;

/// A glyph must be set at most this fraction of its line's body size before it
/// can be read as a super- or subscript. TeX sets scripts at 70% of the text
/// size and scriptscripts at 50%, so this clears both with room to spare.
const SCRIPT_SIZE_RATIO: f32 = 0.85;

/// Fallback when PDFium reports no font size: the glyph box height, which is
/// the shape-dependent measure this replaced.
const SCRIPT_HEIGHT_RATIO: f32 = 0.8;
/// ...and it must sit at least this fraction of the line height off the
/// baseline. This is what keeps descenders out: a "y" drops below the
/// baseline but stays full height, so it fails the ratio above anyway.
const SCRIPT_OFFSET_RATIO: f32 = 0.15;

/// Per-character font facts from the extractor, passed alongside the boxes,
/// plus any mathematics the document tagged for us.
#[derive(Clone, Copy)]
pub struct GlyphMeta<'a> {
    pub sizes: &'a [f32],
    pub math: &'a [bool],
    /// Math recovered from a structure tree, in character order. Where one of
    /// these covers a stretch of a line it replaces the reconstruction, being
    /// the author's own markup rather than an inference from glyph positions.
    pub spans: &'a [MathSpan],
}

impl<'a> GlyphMeta<'a> {
    /// No font facts available — script detection falls back to glyph boxes.
    pub const NONE: GlyphMeta<'static> = GlyphMeta {
        sizes: &[],
        math: &[],
        spans: &[],
    };

    pub fn new(sizes: &'a [f32], math: &'a [bool]) -> Self {
        GlyphMeta {
            sizes,
            math,
            spans: &[],
        }
    }

    /// Attach mathematics read from the document's structure tree.
    pub fn with_spans(self, spans: &'a [MathSpan]) -> Self {
        GlyphMeta { spans, ..self }
    }

    /// The tagged span starting exactly at `index`, if there is one.
    fn span_at(&self, index: usize) -> Option<&'a MathSpan> {
        self.spans.iter().find(|span| span.start as usize == index)
    }

    fn size(&self, index: usize) -> f32 {
        self.sizes.get(index).copied().unwrap_or(0.0)
    }

    fn is_math(&self, index: usize) -> bool {
        self.math.get(index).copied().unwrap_or(false)
    }
}

/// The font size most of a line is set in, ignoring glyphs with none.
fn dominant_size(glyphs: &[(char, [f32; 4], f32, bool, bool)]) -> f32 {
    let mut sizes: Vec<f32> = glyphs
        .iter()
        .filter(|(ch, _, size, _, _)| *size > 0.0 && !ch.is_whitespace())
        .map(|(_, _, size, _, _)| *size)
        .collect();
    if sizes.is_empty() {
        return 0.0;
    }
    // The mode, not the mean: a line of body text with two superscripts should
    // report the body size, and averaging would drag it down.
    sizes.sort_by(f32::total_cmp);
    let mut best = (sizes[0], 0usize);
    let mut run = (sizes[0], 0usize);
    for size in &sizes {
        if (*size - run.0).abs() < 0.05 {
            run.1 += 1;
        } else {
            run = (*size, 1);
        }
        if run.1 > best.1 {
            best = run;
        }
    }
    best.0
}

/// Emit one run of same-script glyphs as the LaTeX that produced it.
///
/// A PDF has no `^`: "L^2 Optimality" is drawn as an "L" and a smaller "2"
/// sitting 3.6pt higher, and a plain concatenation flattens the two into "L2".
/// The notation is what the author wrote and what a reader of a maths paper
/// expects to see, so it is what gets rebuilt — braced when the script runs to
/// more than one character, exactly as LaTeX requires.
fn push_script(run: &str, kind: Script, out: &mut String) {
    if run.is_empty() {
        return;
    }
    let marker = match kind {
        Script::Normal => {
            out.push_str(run);
            return;
        }
        Script::Super => '^',
        Script::Sub => '_',
    };
    out.push(marker);
    if run.chars().count() == 1 {
        out.push_str(run);
    } else {
        out.push('{');
        out.push_str(run);
        out.push('}');
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Normal,
    Super,
    Sub,
}

/// One visual line starting at `index`: its text, left edge, baseline, and
/// where the next line begins.
///
/// Lines are cut on baseline changes rather than on the newlines PDFium
/// emits — see `HEADING_LINE_BAND_PT` for why those cannot be trusted. The
/// same baselines that delimit the line also reveal its super- and subscripts,
/// which are restored rather than flattened.
fn line_at(
    chars: &[char],
    boxes: &[[f32; 4]],
    meta: GlyphMeta<'_>,
    index: usize,
) -> (String, f32, f32, f32, usize) {
    let baseline = boxes
        .iter()
        .skip(index)
        .find(|b| b[2] > 0.0 && b[3] > 0.0)
        .map(|b| b[1]);

    // Each glyph, plus its font size, whether it is math, and whether a
    // text-object boundary was crossed to reach it.
    let mut glyphs: Vec<(char, [f32; 4], f32, bool, bool)> = Vec::new();
    let mut min_x = f32::MAX;
    let mut cursor = index;
    let mut crossed_boundary = false;
    while cursor < chars.len() {
        // A tagged document already said what this is. Take its markup
        // verbatim and step over the glyphs it covers, rather than deriving
        // the same thing again — worse — from their positions. The pieces go
        // in with no geometry of their own, so they read as ordinary text and
        // never look like scripts to the code below.
        if let Some(span) = meta.span_at(cursor) {
            if span.end as usize > cursor {
                for (offset, latex_char) in span.latex.chars().enumerate() {
                    glyphs.push((
                        latex_char,
                        [0.0; 4],
                        0.0,
                        false,
                        crossed_boundary && offset == 0,
                    ));
                }
                cursor = span.end as usize;
                crossed_boundary = false;
                continue;
            }
        }
        let ch = chars[cursor];
        if ch == '\n' || ch == '\r' {
            crossed_boundary = true;
            cursor += 1;
            continue;
        }

        let Some(b) = boxes.get(cursor) else { break };
        if b[2] > 0.0 && b[3] > 0.0 {
            if baseline.is_some_and(|line| (b[1] - line).abs() > HEADING_LINE_BAND_PT) {
                break;
            }
            min_x = min_x.min(b[0]);
        }
        glyphs.push((
            ch,
            *b,
            meta.size(cursor),
            meta.is_math(cursor),
            crossed_boundary,
        ));
        crossed_boundary = false;
        cursor += 1;
    }

    let line_baseline = baseline.unwrap_or(0.0);
    // Full-size glyphs sitting on the line define what "full size" means.
    let line_height = glyphs
        .iter()
        .filter(|(_, b, _, _, _)| b[3] > 0.0 && (b[1] - line_baseline).abs() < 1.0)
        .map(|(_, b, _, _, _)| b[3])
        .fold(0.0f32, f32::max)
        .max(1.0);
    // The line's body size: the most common font size among its glyphs. A
    // script is set smaller than this, which is a fact about the font rather
    // than an inference from the shape of a glyph.
    let line_size = dominant_size(&glyphs);

    let mut text = String::new();
    let mut run = String::new();
    let mut run_kind = Script::Normal;
    let mut previous_right: Option<f32> = None;
    for (ch, b, size, is_math, after_boundary) in &glyphs {
        // A script is a glyph set at a smaller font size and shifted off the
        // baseline. Measuring the *size* rather than the glyph box is what
        // keeps a period or a hyphen — short at any size — from reading as a
        // raised script. Math fonts are allowed non-alphanumeric scripts,
        // since "x^{-1}" and "x^+" are real; text fonts are not, because a
        // hyphen there is punctuation.
        let smaller = if *size > 0.0 && line_size > 0.0 {
            *size <= SCRIPT_SIZE_RATIO * line_size
        } else {
            b[3] > 0.0 && b[3] <= SCRIPT_HEIGHT_RATIO * line_height
        };
        let can_be_script = ch.is_alphanumeric() || (*is_math && !ch.is_whitespace());
        let kind = if can_be_script && smaller && b[2] > 0.0 && b[3] > 0.0 {
            let offset = b[1] - line_baseline;
            if offset >= SCRIPT_OFFSET_RATIO * line_height {
                Script::Super
            } else if offset <= -SCRIPT_OFFSET_RATIO * line_height {
                Script::Sub
            } else {
                Script::Normal
            }
        } else {
            Script::Normal
        };

        // Ordinary spaces are in the stream already. The one place a space
        // goes missing is a text-object boundary, where PDFium emits a newline
        // instead: "S^2 under" arrives as "S", a raised "2", then "under".
        // Only there is the gap on the page worth consulting — a word space
        // measures about 0.4 of the line height, letter spacing under 0.15.
        if *after_boundary && b[2] > 0.0 && !ch.is_whitespace() {
            let gapped =
                previous_right.is_some_and(|right| b[0] - right > WORD_GAP_RATIO * line_height);
            if gapped && !run.ends_with(' ') && !text.ends_with(' ') {
                push_script(&run, run_kind, &mut text);
                run.clear();
                run_kind = Script::Normal;
                text.push(' ');
            }
        }
        if b[2] > 0.0 {
            previous_right = Some(b[0] + b[2]);
        }

        if kind != run_kind {
            push_script(&run, run_kind, &mut text);
            run.clear();
            run_kind = kind;
        }
        run.push(*ch);
    }
    push_script(&run, run_kind, &mut text);

    // PDFium reports a glyph it cannot map to Unicode as a control character,
    // and in these papers that is nearly always the hyphen a word was broken
    // across lines with — "corrup-" then "tion". Rendered as-is it is a hollow
    // box mid-title, so it is dropped; at the line end it becomes the hyphen
    // it was, which is the only thing that lets a caller rejoin the word.
    let mut cleaned = String::with_capacity(text.len());
    let control_positions: Vec<usize> = text
        .char_indices()
        .filter(|(_, c)| c.is_control())
        .map(|(i, _)| i)
        .collect();
    let last_control = control_positions.last().copied();
    let trailing = text.trim_end();
    for (i, ch) in text.char_indices() {
        if !ch.is_control() {
            cleaned.push(ch);
        } else if Some(i) == last_control && trailing.len() <= i + ch.len_utf8() {
            cleaned.push('-');
        }
    }
    let text = cleaned;

    (
        text.trim().to_string(),
        min_x,
        line_baseline,
        line_height,
        cursor.max(index + 1),
    )
}

/// One visual line of a page: its text with LaTeX scripts restored, and where
/// it sits.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLine {
    pub text: String,
    /// left edge of the line
    pub x: f32,
    /// baseline, display space y-up
    pub y: f32,
    /// height of the line's full-size glyphs, for judging the gap to the next
    pub height: f32,
}

/// Every visual line on a page, in the extractor's reading order.
///
/// Reading order, not geometric order, is what keeps a two-column page
/// readable: the stream emits one column at a time, so consecutive characters
/// belong to the same column and a change of baseline is a real line break.
/// Sorting by height instead splices the columns into each other.
///
/// Shared with the figure panel, which needs captions read exactly the way
/// headings are — same line grouping, same script handling.
pub fn page_lines(raw: &str, boxes: &[[f32; 4]], meta: GlyphMeta<'_>) -> Vec<PageLine> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        let (text, x, y, height, next) = line_at(&chars, boxes, meta, index);
        index = next;
        if !text.is_empty() {
            out.push(PageLine { text, x, y, height });
        }
    }
    out
}

/// The heading a section destination names, verified against the counter in
/// the destination's own name.
///
/// hyperref does not reliably anchor a section *on* its heading. The anchor
/// for "1. Introduction" in a two-column paper landed between the abstract's
/// last line and the heading, close enough to the line above that reading the
/// anchored line gave ", we work under the ambient Gaussian corrup" — and the
/// anchored line is not even guaranteed to be in the right column, because the
/// column filter only excludes what lies to the *left* of the anchor.
///
/// So the page is searched instead of the anchor trusted. The destination name
/// carries the counter ("section.4" -> "4") and the printed heading opens with
/// it; among the lines that do, and that start at the anchor's own column, the
/// one nearest the anchor wins. Nothing matching means nothing is returned —
/// a dropped entry beats a sentence fragment posing as a section title.
pub fn heading_line(
    raw: &str,
    boxes: &[[f32; 4]],
    meta: GlyphMeta<'_>,
    anchor_x: Option<f32>,
    anchor_y: f32,
    counter: &str,
) -> Option<HeadingHit> {
    let mut best: Option<(f32, HeadingHit)> = None;
    for line in page_lines(raw, boxes, meta) {
        if !opens_with_counter(&line.text, counter) {
            continue;
        }
        if anchor_x.is_some_and(|x| (line.x - x).abs() > HEADING_X_TOLERANCE_PT) {
            continue;
        }
        let distance = (line.y - anchor_y).abs();
        if best.as_ref().is_none_or(|(closest, _)| distance < *closest) {
            best = Some((
                distance,
                HeadingHit {
                    title: line.text,
                    y: line.y,
                    x: line.x,
                },
            ));
        }
    }
    best.map(|(_, hit)| hit)
}

/// The line stating the environment a destination names, found by searching
/// the page rather than trusting the anchor.
///
/// The same anchor drift that misplaces section headings hits environments
/// too: one destination lands inside a display formula, another on the page
/// number, and reading from there finds nothing to reconcile. The counter in
/// the destination name is checkable, so the page is searched for a line that
/// *opens* with a matching environment — opening matters, because a passing
/// reference to "Theorem 3.1" appears mid-sentence while its statement starts
/// a line. Nearest to the anchor wins.
pub fn environment_line(
    raw: &str,
    boxes: &[[f32; 4]],
    meta: GlyphMeta<'_>,
    anchor_y: f32,
    counter: &str,
) -> Option<(Heading, f32)> {
    let mut best: Option<(f32, Heading, f32)> = None;
    for line in page_lines(raw, boxes, meta) {
        let Some(heading) = parse_heading(&line.text) else {
            continue;
        };
        if heading.number != counter {
            continue;
        }
        let distance = (line.y - anchor_y).abs();
        if best
            .as_ref()
            .is_none_or(|(closest, _, _)| distance < *closest)
        {
            best = Some((distance, heading, line.y));
        }
    }
    best.map(|(_, heading, y)| (heading, y))
}

/// Interleaves headings and environments into the list the panel renders.
///
/// A heading is emitted only once something lands under it — a section with no
/// environments is noise in a list that exists to find environments — and an
/// environment indents to however many heading levels are currently standing.
/// Both inputs must be in document order.
pub fn merge_structure(
    headings: Vec<FormalEntryDto>,
    environments: Vec<FormalEntryDto>,
) -> Vec<FormalEntryDto> {
    let mut out: Vec<FormalEntryDto> = Vec::new();
    let mut pending: [Option<FormalEntryDto>; 2] = [None, None];
    let mut shown: u8 = 0;
    let mut heads = headings.into_iter().peekable();

    for mut env in environments {
        while heads
            .peek()
            .is_some_and(|h| h.page < env.page || (h.page == env.page && h.y >= env.y))
        {
            let head = heads.next().expect("peeked");
            let depth = (head.depth as usize).min(1);
            pending[depth] = Some(head);
            if depth == 0 {
                pending[1] = None; // a new section closes the subsection under it
                shown = 0;
            } else {
                shown = shown.min(1);
            }
        }
        for (depth, slot) in pending.iter_mut().enumerate() {
            if let Some(entry) = slot.take() {
                out.push(entry);
                shown = depth as u8 + 1;
            }
        }
        env.depth = shown;
        out.push(env);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_the_structural_destinations_hyperref_also_emits() {
        for name in [
            "Doc-Start",
            "page.7",
            "cite.Knuth1984",
            "Item.12",
            "equation.4",
        ] {
            assert!(is_structural_destination(name), "{name} should be skipped");
        }
        assert!(!is_structural_destination("theorem.1.1"));
        assert!(!is_structural_destination("figure.3"));
        // headings are kept, but to group the list rather than populate it
        assert!(!is_structural_destination("section.2"));
        assert_eq!(heading_depth("section.2"), Some(0));
        assert_eq!(heading_depth("subsection.2.1"), Some(1));
        assert_eq!(heading_depth("appendix.A"), Some(0));
        assert_eq!(heading_depth("theorem.1.1"), None);
    }

    #[test]
    fn reads_the_counter_value_out_of_a_destination_name() {
        assert_eq!(destination_number("theorem.3.1"), Some("3.1"));
        assert_eq!(destination_number("figure.2"), Some("2"));
        assert_eq!(destination_number("definition.1.4"), Some("1.4"));
        assert_eq!(destination_number("Doc-Start"), None);
        assert_eq!(destination_number("nodots"), None);
    }

    #[test]
    fn parses_a_bare_numbered_environment() {
        let h = parse_heading("Theorem 1.1. The mistake bound is finite.").unwrap();
        assert_eq!(h.kind, "Theorem");
        assert_eq!(h.number, "1.1");
    }

    #[test]
    fn parses_an_appendix_counter() {
        // Every environment in an appendix is numbered this way, and a
        // digits-only counter made all of them invisible.
        let lemma = parse_heading("Lemma B.1 (Chord expansion). Uniformly in z").expect("lemma");
        assert_eq!(
            (lemma.kind.as_str(), lemma.number.as_str()),
            ("Lemma", "B.1")
        );
        let prop = parse_heading("Proposition F.10 (Uniform charts).").expect("prop");
        assert_eq!(prop.number, "F.10");
        let remark = parse_heading("Remark C.1 (Restriction).").expect("remark");
        assert_eq!(remark.number, "C.1");
    }

    #[test]
    fn a_capitalized_word_is_not_a_counter() {
        // "Lemma Suppose that ..." must not read as counter "S".
        assert!(parse_heading("Lemma Suppose that x is small").is_none());
        assert!(parse_heading("Theorem Under these assumptions").is_none());
    }

    #[test]
    fn parses_a_named_environment_even_without_the_space_pdf_drops() {
        // this is the exact shape PDFium hands back for `\begin{theorem}[Main]`
        let h = parse_heading("Theorem 1.1(Main).No label here.").unwrap();
        // the bracketed name is not shown, but must not derail kind/number
        assert_eq!(h.kind, "Theorem");
        assert_eq!(h.number, "1.1");
    }

    #[test]
    fn keeps_the_printed_name_for_shared_counter_environments() {
        // the whole point: these anchor as theorem.1.2 / theorem.1.3
        assert_eq!(
            parse_heading("Lemma 1.2.An unlabeled lemma.").unwrap().kind,
            "Lemma"
        );
        assert_eq!(
            parse_heading("Corollary 1.3.An unlabeled corollary.")
                .unwrap()
                .kind,
            "Corollary"
        );
    }

    /// Lay out `lines` as (text, top edge) the way the engine hands them over:
    /// a raw character stream plus one box per character, with spaces carrying
    /// no geometry.
    /// Uniform-size fixture: every glyph is body text at 9pt.
    fn sizes_for(raw: &str, size: f32) -> Vec<f32> {
        raw.chars()
            .map(|ch| if ch.is_whitespace() { 0.0 } else { size })
            .collect()
    }

    fn page(lines: &[(&str, f32)]) -> (String, Vec<[f32; 4]>) {
        let mut raw = String::new();
        let mut boxes = Vec::new();
        for (text, top) in lines {
            let mut x = 0.0f32;
            for ch in text.chars() {
                raw.push(ch);
                boxes.push(if ch == ' ' {
                    [0.0; 4]
                } else {
                    [x, top - 8.0, 5.0, 8.0]
                });
                x += 6.0;
            }
        }
        (raw, boxes)
    }

    #[test]
    fn keeps_the_inter_word_spaces_the_runs_drop() {
        // the run builder gives "Aplaceholderfigure."; the stream has spaces
        let (raw, boxes) = page(&[("Figure 1: A placeholder figure.", 500.0)]);
        assert_eq!(
            anchor_text(&raw, &boxes, None, 505.0).unwrap(),
            "Figure 1: A placeholder figure."
        );
    }

    #[test]
    fn ignores_marginal_text_outside_the_anchors_column() {
        // An arXiv stamp runs rotated down the margin: it is drawn first, so
        // it holds the lowest character indices, and its glyphs span the page
        // height so one sits at almost any anchor's y. Only its x gives it
        // away — the text block starts far to its right.
        let mut raw = String::new();
        let mut boxes: Vec<[f32; 4]> = Vec::new();
        for ch in "2211.11320v3 [stat.ML]".chars() {
            raw.push(ch);
            boxes.push([13.5, 434.0, 6.0, 8.0]); // margin column, at our anchor's y
        }
        let mut x = 134.0f32;
        for ch in "Theorem 1.1. First result.".chars() {
            raw.push(ch);
            boxes.push([x, 434.0, 5.0, 8.0]);
            x += 6.0;
        }

        // without the column filter the stamp wins, because it comes first
        assert!(anchor_text(&raw, &boxes, None, 443.0)
            .unwrap()
            .starts_with("2211"));
        // with it, the anchor finds its own text
        assert!(anchor_text(&raw, &boxes, Some(133.8), 443.0)
            .unwrap()
            .starts_with("Theorem 1.1."));
    }

    #[test]
    fn starts_at_the_first_character_on_the_anchored_line_not_the_nearest() {
        let (raw, boxes) = page(&[
            ("1 Setup", 667.0),
            ("Theorem 1.1 (Unlabeled).", 642.3),
            ("No label here at all.", 630.0),
        ]);
        let text = anchor_text(&raw, &boxes, None, 647.4).unwrap();
        assert!(text.starts_with("Theorem 1.1 (Unlabeled)."), "got {text:?}");
    }

    #[test]
    fn ignores_text_above_the_anchor() {
        let (raw, boxes) = page(&[("Header", 700.0)]);
        assert!(anchor_text(&raw, &boxes, None, 400.0).is_none());
        assert!(anchor_text(&raw, &boxes, None, 705.0).is_some());
        assert!(anchor_text("", &[], None, 700.0).is_none());
    }

    #[test]
    fn a_heading_line_stops_where_the_line_does() {
        // Two lines: the heading, then the paragraph under it. A table of
        // contents wants only the first.
        let (raw, boxes) = page(&[
            ("4 Extrinsic Gaussian smoothing", 700.0),
            ("We use the second fundamental form", 686.0),
        ]);
        let sizes = sizes_for(&raw, 9.0);
        assert_eq!(
            anchor_line_text(&raw, &boxes, GlyphMeta::new(&sizes, &[]), None, 710.0).as_deref(),
            Some("4 Extrinsic Gaussian smoothing")
        );
        // anchor_text, by contrast, deliberately reads on past the line.
        let spilled = anchor_text(&raw, &boxes, None, 710.0).unwrap();
        assert!(
            spilled.contains("We use"),
            "anchor_text should still read on: {spilled:?}"
        );
    }

    #[test]
    fn short_glyphs_do_not_end_a_heading_early() {
        // Real geometry from an arXiv paper: "4. Canonicality of r". Every box
        // shares a baseline at 578.8, but the period is 2pt tall against the
        // digit's 8.2, so its *top* sits 6pt below the line's. Reading tops
        // ended the title at the period and produced a contents list of bare
        // numbers.
        let raw = "4. Canonical".to_string();
        // Every glyph here is 9pt — including the period, whose *box* is tiny.
        let sizes = sizes_for(&raw, 9.0);
        let mut boxes = Vec::new();
        let mut x = 307.0f32;
        for ch in raw.chars() {
            let h = match ch {
                '.' => 2.01,
                ' ' => 0.0,
                c if c.is_uppercase() || c.is_ascii_digit() || "lit".contains(c) => 8.3,
                _ => 5.8,
            };
            boxes.push(if h == 0.0 {
                [0.0; 4]
            } else {
                [x, 578.8, 5.0, h]
            });
            x += 6.0;
        }
        assert_eq!(
            anchor_line_text(
                &raw,
                &boxes,
                GlyphMeta::new(&sizes, &[]),
                Some(307.0),
                596.2
            )
            .as_deref(),
            Some("4. Canonical")
        );
    }

    #[test]
    fn a_superscript_does_not_end_a_heading() {
        // Real shape from an arXiv paper: the superscript of "4.1. L2
        // Optimality" is its own text object, so PDFium emits a newline
        // before it. Treating that newline as a line end truncated the title
        // to "4.1. L". Only the baseline says where the line really ends.
        let heading = "4.1. L\r\n2 Optimality";
        let body = "\r\nThe risk";
        let raw = format!("{heading}{body}");
        // TeX sets a superscript at 70% of the body size.
        let sizes: Vec<f32> = raw
            .chars()
            .enumerate()
            .map(|(i, ch)| match ch {
                ' ' | '\r' | '\n' => 0.0,
                '2' if i < heading.chars().count() => 6.3,
                _ => 9.0,
            })
            .collect();
        let mut boxes = Vec::new();
        let mut x = 307.0f32;
        for (index, ch) in raw.chars().enumerate() {
            let on_next_line = index >= heading.chars().count();
            let (bottom, h) = match ch {
                ' ' | '\r' | '\n' => (0.0, 0.0),
                // the next line, 18.6pt below
                _ if on_next_line => (408.0, 6.8),
                // superscript: 3.6pt above its own line
                '2' => (430.3, 4.6),
                _ => (426.7, 6.8),
            };
            if h == 0.0 {
                // Newlines and spaces occupy no width on the page, and a
                // real PDF does not advance the pen past them.
                boxes.push([0.0; 4]);
                continue;
            }
            boxes.push([x, bottom, 5.0, h]);
            x += 5.5;
        }
        // The superscript is both kept on the line *and* restored as one.
        assert_eq!(
            anchor_line_text(
                &raw,
                &boxes,
                GlyphMeta::new(&sizes, &[]),
                Some(307.0),
                434.0
            )
            .as_deref(),
            Some("4.1. L^2 Optimality")
        );
    }

    /// A page of lines, each `(text, baseline, left_x)`, with per-glyph boxes.
    fn laid_out(lines: &[(&str, f32, f32)]) -> (String, Vec<[f32; 4]>, Vec<f32>) {
        let mut raw = String::new();
        let mut boxes = Vec::new();
        for (text, baseline, left) in lines {
            let mut x = *left;
            for ch in text.chars() {
                raw.push(ch);
                boxes.push(if ch == ' ' {
                    [0.0; 4]
                } else {
                    [x, *baseline, 5.0, 7.0]
                });
                x += 6.0;
            }
            raw.push('\r');
            boxes.push([0.0; 4]);
            raw.push('\n');
            boxes.push([0.0; 4]);
        }
        let sizes = sizes_for(&raw, 9.0);
        (raw, boxes, sizes)
    }

    #[test]
    fn a_counter_opens_a_heading_but_not_a_calculation() {
        assert!(opens_with_counter("4. Canonicality", "4"));
        assert!(opens_with_counter("4.1. L2 Optimality", "4.1"));
        assert!(opens_with_counter("A. Some Useful Results", "A"));
        // arithmetic, not a title
        assert!(!opens_with_counter("1 + 2 = 3", "1"));
        // a section counter must not claim its own subsection's line
        assert!(!opens_with_counter("4.1. L2 Optimality", "4"));
        assert!(!opens_with_counter("5. Something", "4"));
    }

    #[test]
    fn a_heading_is_found_even_when_its_anchor_points_elsewhere() {
        // The real failure: hyperref anchored "1. Introduction" between the
        // abstract's last line and the heading, near enough to the line above
        // that reading the anchored line gave a sentence fragment.
        let (raw, boxes, sizes) = laid_out(&[
            ("we work under the ambient corruption", 328.0, 56.0),
            ("1. Introduction", 306.0, 56.0),
            ("The problem we consider is", 288.0, 56.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(55.4),
            324.4,
            "1",
        )
        .expect("heading");
        assert_eq!(hit.title, "1. Introduction");
        assert_eq!(hit.y, 306.0);
    }

    #[test]
    fn a_heading_never_comes_from_the_other_column() {
        // A numbered list item in the right column opens with the same
        // counter; only the left-column heading is this destination's.
        let (raw, boxes, sizes) = laid_out(&[
            ("1. We identify the tangent target", 500.0, 316.0),
            ("1. Introduction", 306.0, 56.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(55.4),
            324.4,
            "1",
        )
        .expect("heading");
        assert_eq!(hit.title, "1. Introduction");
    }

    #[test]
    fn no_matching_heading_yields_nothing_rather_than_a_guess() {
        let (raw, boxes, sizes) = laid_out(&[("nothing here opens with a counter", 300.0, 56.0)]);
        assert_eq!(
            heading_line(
                &raw,
                &boxes,
                GlyphMeta::new(&sizes, &[]),
                Some(55.4),
                324.4,
                "1"
            ),
            None
        );
    }

    /// A line whose glyphs carry explicit heights and baselines, so scripts
    /// can be described the way a PDF actually stores them.
    /// Glyphs as `(char, baseline, box height, font size)`. The font size is
    /// the thing script detection reads; the box height is only the shape.
    fn scripted(glyphs: &[(char, f32, f32, f32)]) -> (String, Vec<[f32; 4]>, Vec<f32>) {
        let mut raw = String::new();
        let mut boxes = Vec::new();
        let mut sizes = Vec::new();
        let mut x = 56.0f32;
        for (ch, baseline, h, size) in glyphs {
            raw.push(*ch);
            if *ch == ' ' {
                boxes.push([0.0; 4]);
                sizes.push(0.0);
                continue;
            }
            boxes.push([x, *baseline, 5.0, *h]);
            sizes.push(*size);
            x += 6.0;
        }
        (raw, boxes, sizes)
    }

    #[test]
    fn a_superscript_is_restored_rather_than_flattened() {
        // "4.1. L2 Optimality" on the page is an L and a smaller 2 sitting
        // 3.6pt higher — the LaTeX was L^2.
        let (raw, boxes, sizes) = scripted(&[
            ('4', 426.7, 6.8, 9.0),
            ('.', 426.6, 1.7, 9.0),
            ('1', 426.7, 6.8, 9.0),
            ('.', 426.6, 1.7, 9.0),
            (' ', 0.0, 0.0, 0.0),
            ('L', 426.7, 6.8, 9.0),
            ('2', 430.3, 4.6, 6.3),
            (' ', 0.0, 0.0, 0.0),
            ('O', 426.5, 7.1, 9.0),
            ('p', 424.7, 6.7, 9.0),
            ('t', 426.6, 6.4, 9.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(56.0),
            430.0,
            "4.1",
        )
        .expect("heading");
        assert_eq!(hit.title, "4.1. L^2 Opt");
    }

    #[test]
    fn a_hyphen_is_not_mistaken_for_a_superscript() {
        // A hyphen is a short bar drawn at mid-height, so by glyph geometry
        // alone it looks exactly like a raised script. Every "Leading-Order"
        // and "Second-Order" heading in a paper depends on this.
        let (raw, boxes, sizes) = scripted(&[
            ('4', 400.0, 6.8, 9.0),
            ('.', 400.0, 1.7, 9.0),
            (' ', 0.0, 0.0, 0.0),
            ('L', 400.0, 6.8, 9.0),
            ('e', 400.0, 4.7, 9.0),
            // the hyphen: a short bar, but set at the body size like its
            // neighbours — which is exactly why size settles it and the box
            // never could
            ('-', 402.6, 0.7, 9.0),
            ('O', 400.0, 7.1, 9.0),
            ('r', 400.0, 4.6, 9.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(56.0),
            402.0,
            "4",
        )
        .expect("heading");
        assert_eq!(hit.title, "4. Le-Or");
    }

    #[test]
    fn a_descender_is_not_mistaken_for_a_subscript() {
        // "p" and "y" drop below the baseline but stay full height; only a
        // shrunken glyph off the baseline is a script.
        let (raw, boxes, sizes) = scripted(&[
            ('1', 400.0, 6.8, 9.0),
            ('.', 400.0, 1.7, 9.0),
            (' ', 0.0, 0.0, 0.0),
            ('T', 400.0, 6.8, 9.0),
            ('y', 398.0, 6.6, 9.0),
            ('p', 398.0, 6.7, 9.0),
            ('e', 400.0, 4.7, 9.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(56.0),
            402.0,
            "1",
        )
        .expect("heading");
        assert_eq!(hit.title, "1. Type");
    }

    #[test]
    fn a_subscript_is_restored_too() {
        let (raw, boxes, sizes) = scripted(&[
            ('2', 400.0, 6.8, 9.0),
            ('.', 400.0, 1.7, 9.0),
            (' ', 0.0, 0.0, 0.0),
            ('R', 400.0, 6.8, 9.0),
            ('n', 396.4, 4.6, 6.3),
            (' ', 0.0, 0.0, 0.0),
            ('B', 400.0, 6.8, 9.0),
            ('a', 400.0, 4.7, 9.0),
            ('r', 400.0, 4.6, 9.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(56.0),
            402.0,
            "2",
        )
        .expect("heading");
        assert_eq!(hit.title, "2. R_n Bar");
    }

    #[test]
    fn a_multi_character_script_is_braced_the_way_latex_needs() {
        // "L^{10}" must not come back as "L^1^0".
        let (raw, boxes, sizes) = scripted(&[
            ('3', 400.0, 6.8, 9.0),
            ('.', 400.0, 1.7, 9.0),
            (' ', 0.0, 0.0, 0.0),
            ('L', 400.0, 6.8, 9.0),
            ('1', 403.5, 4.6, 6.3),
            ('0', 403.5, 4.6, 6.3),
            (' ', 0.0, 0.0, 0.0),
            ('N', 400.0, 6.8, 9.0),
            ('o', 400.0, 4.7, 9.0),
            ('w', 400.0, 4.6, 9.0),
        ]);
        let hit = heading_line(
            &raw,
            &boxes,
            GlyphMeta::new(&sizes, &[]),
            Some(56.0),
            402.0,
            "3",
        )
        .expect("heading");
        assert_eq!(hit.title, "3. L^{10} Now");
    }

    fn head(depth: u8, label: &str, page: u32, y: f32) -> FormalEntryDto {
        FormalEntryDto {
            heading: true,
            depth,
            label: label.into(),
            page,
            y,
            char_index: 0,
            x: None,
        }
    }
    fn env(label: &str, page: u32, y: f32) -> FormalEntryDto {
        FormalEntryDto {
            heading: false,
            depth: 0,
            label: label.into(),
            page,
            y,
            char_index: 0,
            x: None,
        }
    }
    fn shape(entries: &[FormalEntryDto]) -> Vec<String> {
        entries
            .iter()
            .map(|e| format!("{}{}", "  ".repeat(e.depth as usize), e.label))
            .collect()
    }

    #[test]
    fn nests_environments_under_their_section_and_subsection() {
        let merged = merge_structure(
            vec![
                head(0, "1 Introduction", 0, 700.0),
                head(1, "1.2 Preliminaries", 1, 600.0),
                head(0, "2 Results", 3, 700.0),
            ],
            vec![
                env("Theorem 1.1", 0, 500.0),
                env("Definition 1.2", 1, 400.0),
                env("Lemma 2.1", 3, 500.0),
            ],
        );
        assert_eq!(
            shape(&merged),
            vec![
                "1 Introduction",
                "  Theorem 1.1",
                "  1.2 Preliminaries",
                "    Definition 1.2",
                "2 Results",
                "  Lemma 2.1",
            ]
        );
    }

    #[test]
    fn a_new_section_closes_the_subsection_under_it() {
        let merged = merge_structure(
            vec![
                head(0, "1 One", 0, 700.0),
                head(1, "1.1 Sub", 0, 600.0),
                head(0, "2 Two", 1, 700.0),
            ],
            vec![env("Lemma 1.1", 0, 500.0), env("Lemma 2.1", 1, 500.0)],
        );
        // the second lemma sits directly under section 2, not under 1.1
        assert_eq!(
            shape(&merged),
            vec![
                "1 One",
                "  1.1 Sub",
                "    Lemma 1.1",
                "2 Two",
                "  Lemma 2.1"
            ]
        );
    }

    #[test]
    fn drops_headings_that_contain_nothing() {
        let merged = merge_structure(
            vec![head(0, "1 Empty", 0, 700.0), head(0, "2 Full", 1, 700.0)],
            vec![env("Theorem 2.1", 1, 500.0)],
        );
        assert_eq!(shape(&merged), vec!["2 Full", "  Theorem 2.1"]);
    }

    #[test]
    fn leaves_environments_flat_when_a_document_has_no_headings() {
        let merged = merge_structure(vec![], vec![env("Theorem 1", 0, 500.0)]);
        assert_eq!(shape(&merged), vec!["Theorem 1"]);
    }

    #[test]
    fn only_top_level_structure_groups_the_list() {
        assert!(is_section_destination("section.2"));
        assert!(is_section_destination("chapter.4"));
        assert!(is_section_destination("subsection.2.1"));
        assert!(!is_section_destination("theorem.1.1"));
    }

    #[test]
    fn rejects_text_that_is_not_a_numbered_environment() {
        assert!(parse_heading("1 Setup").is_none()); // a section heading
        assert!(parse_heading("The theorem states that").is_none()); // prose
        assert!(parse_heading("").is_none());
        assert!(parse_heading("Proof. Immediate.").is_none()); // unnumbered
    }

    #[test]
    fn reconciles_only_when_the_numbers_agree() {
        // a lemma sharing the theorem counter: names differ, numbers agree
        let ok = reconcile("theorem.1.2", "Lemma 1.2.An unlabeled lemma.").unwrap();
        assert_eq!(ok.kind, "Lemma");
        assert_eq!(ok.number, "1.2");

        // the text at the anchor belongs to something else entirely
        assert!(reconcile("theorem.1.2", "Theorem 9.9. Elsewhere.").is_none());
        assert!(reconcile("section.1", "Theorem 1.1.").is_none());
    }
}

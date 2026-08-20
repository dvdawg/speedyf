//! Finding figures: their captions, and the artwork each caption belongs to.
//!
//! Captions are read out of the page text, not from hyperref anchors. That was
//! the first design and it was wrong: hyperref only emits a destination for a
//! float carrying a `\label`, so a paper can have anchors for every section
//! and theorem and none at all for its figures — which is what real arXiv
//! papers turn out to look like. Page text is always there.
//!
//! The artwork itself is unlabelled in any PDF; it is just whatever occupies
//! the page between the caption and the prose above it, so the crop is
//! derived from where the *text* is not.
//!
//! PDFium-free, like `preview`, so the heuristics are ordinary unit tests.

use super::formal::PageLine;
use super::preview::column_bounds;
use super::types::TextRun;

/// Scale for panel thumbnails. Small on purpose: these are browsing aids, and
/// a figure list can hold dozens of them.
pub const FIGURE_THUMB_SCALE_MILLI: u32 = 500;

/// A run at least this wide, relative to its column, is prose rather than
/// something inside the artwork (an axis label, a legend, a sub-caption).
const BODY_WIDTH_RATIO: f32 = 0.55;
/// Never claim more of the page than this for one figure.
const MAX_HEIGHT_RATIO: f32 = 0.75;
/// Never claim less: a sliver is worse than a slightly generous crop.
const MIN_HEIGHT_PT: f32 = 60.0;
/// Breathing room so the crop does not shave the artwork's own edges.
const PADDING_PT: f32 = 4.0;

pub struct FigureCropInput<'a> {
    pub page_w_pt: f32,
    pub page_h_pt: f32,
    /// Caption anchor in display space, y-up — where hyperref points.
    pub caption_y: f32,
    /// Caption anchor x, when the destination carried one. Decides which
    /// column the figure sits in on a two-column page.
    pub caption_x: Option<f32>,
    pub runs: &'a [TextRun],
}

/// Crop covering the artwork above a caption, as `[x, y, w, h]` in display
/// space (y-up). The caption itself is excluded — the panel prints its label
/// alongside, so spending thumbnail pixels on it would only shrink the figure.
pub fn figure_crop_rect(input: &FigureCropInput<'_>) -> [f32; 4] {
    let page_w = input.page_w_pt.max(1.0);
    let page_h = input.page_h_pt.max(1.0);
    let caption_y = input.caption_y.clamp(0.0, page_h);
    let column = column_bounds(input.runs, page_w, caption_y, input.caption_x);
    let column_w = (column.end - column.start).max(1.0);

    // Top edge of the caption block: everything the caption occupies at or
    // just above its anchor line. Starting from the anchor alone would leave
    // the caption's own first line inside the crop.
    let caption_top = input
        .runs
        .iter()
        .filter(|run| {
            run.h > 0.0
                && run.x + run.w > column.start
                && run.x < column.end
                && run.y >= caption_y - 1.0
                && run.y <= caption_y + 1.0
        })
        .map(|run| run.y + run.h)
        .fold(caption_y, f32::max);

    // Walk upward from the caption. Narrow runs are treated as part of the
    // artwork and stepped over; the first run wide enough to be prose is the
    // ceiling, and the figure ends at that run's baseline.
    let mut ceiling = page_h;
    let mut above: Vec<&TextRun> = input
        .runs
        .iter()
        .filter(|run| {
            run.w > 0.0
                && run.h > 0.0
                && run.y > caption_top
                && run.x + run.w > column.start
                && run.x < column.end
        })
        .collect();
    above.sort_by(|left, right| left.y.total_cmp(&right.y));
    for run in above {
        if run.w >= BODY_WIDTH_RATIO * column_w {
            ceiling = run.y;
            break;
        }
    }

    let max_h = (MAX_HEIGHT_RATIO * page_h).min(page_h);
    let min_h = MIN_HEIGHT_PT.min(page_h);
    let mut height = (ceiling - caption_top).clamp(min_h, max_h);
    // A caption near the top of its column leaves no room above it; pin the
    // crop inside the page rather than letting it run off the edge.
    if caption_top + height > page_h {
        height = (page_h - caption_top).max(min_h.min(page_h));
    }

    let left = (column.start - PADDING_PT).max(0.0);
    let right = (column.end + PADDING_PT).min(page_w);
    let bottom = (caption_top - PADDING_PT).max(0.0);
    let top = (bottom + height + PADDING_PT).min(page_h);
    [
        left,
        bottom,
        (right - left).max(1.0),
        (top - bottom).max(1.0),
    ]
}

/// Words that open a caption, longest first so "figure" wins over "fig".
const CAPTION_WORDS: &[&str] = &["figure", "fig.", "table", "algorithm", "alg.", "listing"];

/// Longest caption text kept for a panel row. A caption can run for a
/// paragraph; the row only needs enough to tell one figure from another.
const MAX_TITLE_CHARS: usize = 140;

/// A caption found in the page text.
#[derive(Clone, Debug, PartialEq)]
pub struct Caption {
    /// normalized to how it is printed: "Figure 2", "Table 1"
    pub label: String,
    /// the caption itself, with LaTeX scripts restored
    pub title: String,
    /// baseline of the caption's first line, display space y-up
    pub y: f32,
    /// left edge of that line — decides which column the figure sits in
    pub x: f32,
}

/// Split a caption line into its label and the text that follows.
///
/// What follows the number is the discriminator that matters. A caption reads
/// "Figure 2." or "Figure 2:", while prose reads "Figure 2 computes the ...".
/// Without this rule every sentence that opens by naming a figure becomes a
/// row in the panel.
pub fn parse_caption(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    let lower = trimmed.to_lowercase();
    let word = CAPTION_WORDS
        .iter()
        .find(|candidate| lower.starts_with(*candidate))?;
    let rest = trimmed[word.len()..].trim_start();

    // The number: digits, with an interior dot only when a digit follows, so
    // the "." ending "Figure 2." reads as a terminator and not part of it.
    let bytes = rest.as_bytes();
    let mut n = 0usize;
    while n < bytes.len() {
        let c = bytes[n];
        let part_of_number = c.is_ascii_digit()
            || (c == b'.' && n + 1 < bytes.len() && bytes[n + 1].is_ascii_digit());
        if !part_of_number {
            break;
        }
        n += 1;
    }
    if n == 0 {
        return None;
    }
    let number = &rest[..n];

    let after = rest[n..].trim_start();
    if !(after.is_empty() || after.starts_with([':', '.', ')', '|', '\u{2013}', '\u{2014}'])) {
        return None;
    }

    let printed = &trimmed[..word.len()];
    let title = after
        .trim_start_matches([':', '.', ')', '|', '\u{2013}', '\u{2014}', ' '])
        .trim_end()
        .to_string();
    Some((format!("{printed} {number}"), title))
}

/// A continuation line must sit within this multiple of the line height below
/// its predecessor, or it belongs to whatever comes after the caption.
const CAPTION_LINE_GAP_RATIO: f32 = 2.2;
/// ...and start within this multiple of the line height of the caption's own
/// left edge, so the next column never gets spliced onto the end.
const CAPTION_INDENT_RATIO: f32 = 2.0;

/// Every caption on a page, in reading order.
///
/// Takes the page's lines rather than its runs so captions are read exactly
/// the way headings are: same reading-order grouping, and the same restoration
/// of the super- and subscripts a caption like "Figure 3. Error of r_σ" is
/// full of.
///
/// A caption is usually several lines. Stopping at the first one cuts it
/// mid-sentence, so continuation lines are gathered while they stay in the
/// caption's own column and close beneath it.
pub fn find_captions(lines: &[PageLine]) -> Vec<Caption> {
    let mut captions = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some((label, first)) = parse_caption(&line.text) else {
            continue;
        };
        let mut title = first;
        let mut previous = line;
        for next in &lines[index + 1..] {
            if title.chars().count() >= MAX_TITLE_CHARS {
                break;
            }
            let height = previous.height.max(1.0);
            let descending = next.y < previous.y;
            let close = previous.y - next.y <= CAPTION_LINE_GAP_RATIO * height;
            let aligned = (next.x - line.x).abs() <= CAPTION_INDENT_RATIO * height;
            if !descending || !close || !aligned || parse_caption(&next.text).is_some() {
                break;
            }
            // A word broken across lines rejoins without its hyphen; anything
            // else takes the space the line break stood for.
            let wrapped = title.trim_end().ends_with(['-', '\u{2010}'])
                && next
                    .text
                    .trim_start()
                    .starts_with(|c: char| c.is_lowercase());
            if wrapped {
                let keep = title.trim_end().len() - 1;
                title.truncate(keep);
                title.push_str(next.text.trim_start());
            } else {
                if !title.is_empty() && !title.ends_with(' ') {
                    title.push(' ');
                }
                title.push_str(&next.text);
            }
            previous = next;
        }
        captions.push(Caption {
            label,
            title: truncate_words(&title, MAX_TITLE_CHARS),
            y: line.y,
            x: line.x,
        });
    }
    captions
}

/// Trim to `limit` characters on a word boundary, marking the cut.
fn truncate_words(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(limit).collect();
    let cut = clipped.rfind(' ').unwrap_or(clipped.len());
    format!("{}…", clipped[..cut].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(x: f32, y: f32, w: f32, h: f32) -> TextRun {
        TextRun {
            text: "x".into(),
            start: 0,
            x,
            y,
            w,
            h,
        }
    }

    /// Body lines filling a column downward from `top`.
    fn body(x: f32, top: f32, lines: usize, w: f32) -> Vec<TextRun> {
        (0..lines)
            .map(|i| run(x, top - i as f32 * 14.0, w, 10.0))
            .collect()
    }

    #[test]
    fn crop_covers_the_gap_between_caption_and_the_prose_above() {
        // Prose down to y=600, blank artwork band, caption at y=400.
        let mut runs = body(60.0, 700.0, 8.0 as usize, 480.0);
        runs.push(run(60.0, 400.0, 480.0, 10.0)); // the caption line
        let rect = figure_crop_rect(&FigureCropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            caption_y: 400.0,
            caption_x: Some(60.0),
            runs: &runs,
        });
        let [_, bottom, _, height] = rect;
        assert!(
            bottom >= 400.0,
            "crop must start above the caption: {rect:?}"
        );
        assert!(
            bottom + height <= 600.0 + 14.0,
            "crop must stop at the prose above: {rect:?}"
        );
        assert!(height > 100.0, "artwork band should be claimed: {rect:?}");
    }

    #[test]
    fn narrow_runs_inside_the_artwork_do_not_end_the_crop() {
        // Axis labels sit in the middle of the figure; they are not prose and
        // must not be mistaken for the paragraph above it.
        let mut runs = body(60.0, 700.0, 4, 480.0);
        runs.push(run(200.0, 520.0, 40.0, 8.0)); // axis label
        runs.push(run(260.0, 500.0, 30.0, 8.0)); // legend
        runs.push(run(60.0, 400.0, 480.0, 10.0)); // caption
        let rect = figure_crop_rect(&FigureCropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            caption_y: 400.0,
            caption_x: Some(60.0),
            runs: &runs,
        });
        assert!(
            rect[1] + rect[3] > 520.0,
            "crop stopped at an axis label: {rect:?}"
        );
    }

    #[test]
    fn a_two_column_caption_crops_only_its_own_column() {
        let mut runs = Vec::new();
        for i in 0..10 {
            let y = 700.0 - i as f32 * 14.0;
            runs.push(run(42.0, y, 235.0, 10.0));
            runs.push(run(330.0, y, 235.0, 10.0));
        }
        runs.push(run(330.0, 400.0, 235.0, 10.0)); // caption, right column
        let rect = figure_crop_rect(&FigureCropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            caption_y: 400.0,
            caption_x: Some(330.0),
            runs: &runs,
        });
        assert!(
            rect[0] > 300.0,
            "crop leaked into the left column: {rect:?}"
        );
        assert!(rect[0] + rect[2] <= 575.0, "crop too wide: {rect:?}");
    }

    #[test]
    fn a_figure_with_nothing_above_it_still_gets_a_usable_crop() {
        let runs = vec![run(60.0, 700.0, 480.0, 10.0)];
        let rect = figure_crop_rect(&FigureCropInput {
            page_w_pt: 612.0,
            page_h_pt: 792.0,
            caption_y: 700.0,
            caption_x: Some(60.0),
            runs: &runs,
        });
        assert!(rect[3] >= MIN_HEIGHT_PT.min(792.0), "{rect:?}");
        assert!(rect[1] >= 0.0 && rect[1] + rect[3] <= 792.0, "{rect:?}");
    }

    #[test]
    fn crop_never_leaves_the_page() {
        for caption_y in [0.0, 5.0, 400.0, 780.0, 792.0] {
            let rect = figure_crop_rect(&FigureCropInput {
                page_w_pt: 612.0,
                page_h_pt: 792.0,
                caption_y,
                caption_x: None,
                runs: &[],
            });
            assert!(rect[0] >= 0.0, "{caption_y}: {rect:?}");
            assert!(rect[1] >= 0.0, "{caption_y}: {rect:?}");
            assert!(rect[0] + rect[2] <= 612.0, "{caption_y}: {rect:?}");
            assert!(rect[1] + rect[3] <= 792.0, "{caption_y}: {rect:?}");
        }
    }

    /// A caption line, as `parse_caption` receives it from `page_lines`.
    fn label_of(line: &str) -> Option<String> {
        parse_caption(line).map(|(label, _)| label)
    }

    #[test]
    fn reads_a_caption_opening_and_its_text() {
        assert_eq!(
            parse_caption("Figure 2: A caption"),
            Some(("Figure 2".into(), "A caption".into()))
        );
        assert_eq!(
            parse_caption("Table 1. Results for the three models"),
            Some(("Table 1".into(), "Results for the three models".into()))
        );
        assert_eq!(label_of("Algorithm 3)").as_deref(), Some("Algorithm 3"));
        assert_eq!(label_of("Figure 12").as_deref(), Some("Figure 12"));
    }

    #[test]
    fn a_caption_keeps_the_latex_the_line_reader_restored() {
        // page_lines hands over "Error of r_σ at L^2", not a flattened form.
        assert_eq!(
            parse_caption("Figure 3. Error of r_σ at L^2"),
            Some(("Figure 3".into(), "Error of r_σ at L^2".into()))
        );
    }

    #[test]
    fn a_sentence_beginning_with_a_figure_reference_is_not_a_caption() {
        // The whole reason the terminator rule exists: prose that opens by
        // naming a figure would otherwise fill the panel with duplicates.
        assert_eq!(parse_caption("Figure 2 computes r(z) numerically"), None);
        assert_eq!(parse_caption("Figures 2 and 3 show the same"), None);
    }

    #[test]
    fn ignores_lines_that_are_not_captions_at_all() {
        assert_eq!(parse_caption(""), None);
        assert_eq!(parse_caption("Theorem 3.1"), None);
        assert_eq!(parse_caption("Section 4: Results"), None);
        assert_eq!(parse_caption("Figure"), None);
    }

    #[test]
    fn a_long_caption_is_trimmed_on_a_word_boundary() {
        // Truncation lives in find_captions now, since a caption is gathered
        // across lines before there is anything to trim.
        let lines = [PageLine {
            text: format!("Figure 4: {}", "words and more words ".repeat(20)),
            x: 60.0,
            y: 400.0,
            height: 9.0,
        }];
        let captions = find_captions(&lines);
        let title = &captions[0].title;
        assert!(title.chars().count() <= MAX_TITLE_CHARS + 1, "{title}");
        assert!(title.ends_with('…'), "{title}");
        assert!(!title.contains("  "), "{title}");
    }

    #[test]
    fn a_caption_rejoins_a_word_broken_across_its_lines() {
        // The hyphen arrives as a control character PDFium could not map, and
        // the line reader hands it over as a trailing "-".
        let lines = [
            PageLine {
                text: "Figure 1. Variance collapse. Sec-".into(),
                x: 60.0,
                y: 400.0,
                height: 9.0,
            },
            PageLine {
                text: "ond moment of the raw target".into(),
                x: 60.0,
                y: 388.0,
                height: 9.0,
            },
        ];
        let captions = find_captions(&lines);
        assert_eq!(
            captions[0].title,
            "Variance collapse. Second moment of the raw target"
        );
    }

    #[test]
    fn finds_captions_among_a_pages_lines() {
        let lines = [
            PageLine {
                text: "Figure 2 computes the thing discussed above".into(),
                x: 60.0,
                y: 700.0,
                height: 9.0,
            },
            PageLine {
                text: "Figure 2. The thing itself".into(),
                x: 60.0,
                y: 400.0,
                height: 9.0,
            },
            PageLine {
                text: "Table 1: Numbers".into(),
                x: 307.0,
                y: 200.0,
                height: 9.0,
            },
        ];
        let captions = find_captions(&lines);
        assert_eq!(
            captions
                .iter()
                .map(|c| (c.label.as_str(), c.x))
                .collect::<Vec<_>>(),
            vec![("Figure 2", 60.0), ("Table 1", 307.0)]
        );
        assert_eq!(captions[0].title, "The thing itself");
    }
}

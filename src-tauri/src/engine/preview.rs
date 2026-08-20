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

#[derive(Clone, Copy)]
pub struct Interval {
    pub start: f32,
    pub end: f32,
}

fn median_height(runs: &[&TextRun]) -> f32 {
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

/// Horizontal extent of the text column holding `anchor_y` (and `dest_x`, when
/// the destination named one). Runs are clustered by their x extents, so a
/// two-column page yields two clusters and a destination picks its own.
///
/// Shared with the figure crop, which needs the same answer for a caption.
pub fn column_bounds(
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

    // A two-column gutter is about 18pt on a letter page, while gaps *within*
    // a line rarely reach 10pt. 4% of the page width (24pt) sat above the
    // gutter, so real two-column papers merged into one column-spanning
    // cluster and every crop taken from them was twice as wide as the text it
    // meant to show.
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
    let widest = |clusters: &[Interval]| {
        clusters
            .iter()
            .copied()
            .max_by(|left, right| (left.end - left.start).total_cmp(&(right.end - right.start)))
            .unwrap_or(fallback)
    };
    if clusters.is_empty() {
        return fallback;
    }
    match dest_x {
        Some(x) => clusters
            .iter()
            .copied()
            .find(|cluster| x >= cluster.start && x <= cluster.end)
            .unwrap_or_else(|| widest(&clusters)),
        None => widest(&clusters),
    }
}

pub fn crop_rect(input: &CropInput<'_>) -> [f32; 4] {
    let page_w = input.page_w_pt.max(1.0);
    let page_h = input.page_h_pt.max(1.0);
    let anchor_y = input.dest_y.unwrap_or(page_h).clamp(0.0, page_h);

    let column = column_bounds(input.runs, page_w, anchor_y, input.dest_x);

    // Leave headroom above the linked line itself, not just a hairline sliver,
    // so the preview shows a bit of what precedes the target (context: the
    // end of the prior sentence/heading), not just the destination pinned to
    // the very top edge.
    let y_top = (anchor_y + 30.0).min(page_h);
    let mut vertical: Vec<&TextRun> = input
        .runs
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
}

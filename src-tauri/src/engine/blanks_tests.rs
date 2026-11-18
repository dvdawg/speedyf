//! Synthetic pages for `blanks::detect`: each draws one kind of blank (or
//! one thing that only looks like one) and checks what comes back.

use super::*;

/// A US Letter page with nothing on it yet.
fn page() -> PageInput {
    PageInput {
        width: 612.0,
        height: 792.0,
        ..PageInput::default()
    }
}

fn hl(p: &mut PageInput, x0: f32, x1: f32, y: f32) {
    p.hlines.push(HLine { x0, x1, y, t: 0.5 });
}

fn vl(p: &mut PageInput, x: f32, y0: f32, y1: f32) {
    p.vlines.push(VLine { x, y0, y1, t: 0.5 });
}

/// The four edges of a stroked rectangle, as a path walk produces them.
fn rect(p: &mut PageInput, x: f32, y: f32, w: f32, h: f32) {
    hl(p, x, x + w, y);
    hl(p, x, x + w, y + h);
    vl(p, x, y, y + h);
    vl(p, x + w, y, y + h);
}

/// Text on a baseline: glyphs half an em wide, cap-height tall. Returns the
/// x where the text ends.
fn word(p: &mut PageInput, text: &str, x: f32, baseline: f32, size: f32) -> f32 {
    let mut at = x;
    for ch in text.chars() {
        let w = 0.5 * size;
        if !ch.is_whitespace() {
            p.glyphs.push(Glyph {
                ch,
                b: Bounds {
                    x0: at,
                    y0: baseline,
                    x1: at + w,
                    y1: baseline + 0.7 * size,
                },
                size,
            });
        }
        at += w;
    }
    at
}

fn kinds(found: &[BlankDto]) -> Vec<&str> {
    found.iter().map(|b| b.kind.as_str()).collect()
}

#[test]
fn a_rule_beside_a_label_is_a_line_blank() {
    let mut p = page();
    let end = word(&mut p, "Name:", 72.0, 700.0, 10.0);
    hl(&mut p, end + 4.0, 320.0, 699.5);
    let found = detect(&p);
    assert_eq!(kinds(&found), vec!["line"]);
    let r = found[0].rect;
    assert!((r.x - (end + 4.0)).abs() < 0.5, "starts at the rule: {r:?}");
    assert!(
        (r.y - 699.5).abs() < 0.01,
        "bottom is the rule itself: {r:?}"
    );
    assert!(r.h >= 10.0, "room to write: {r:?}");
    assert_eq!(found[0].font_size_pt, 10.0, "matches the label");
}

#[test]
fn a_run_of_underscores_is_a_line_blank() {
    let mut p = page();
    let end = word(&mut p, "Date", 72.0, 600.0, 11.0);
    for i in 0..12 {
        let x = end + 3.0 + i as f32 * 5.5;
        p.glyphs.push(Glyph {
            ch: '_',
            b: Bounds {
                x0: x,
                y0: 598.8,
                x1: x + 5.5,
                y1: 599.4,
            },
            size: 11.0,
        });
    }
    let found = detect(&p);
    assert_eq!(kinds(&found), vec!["line"]);
    assert!(found[0].rect.w > 60.0);
    assert_eq!(found[0].font_size_pt, 11.0);
}

#[test]
fn empty_boxes_are_boxes_and_small_squares_are_checkboxes() {
    let mut p = page();
    word(&mut p, "Comments", 72.0, 520.0, 9.0);
    rect(&mut p, 72.0, 470.0, 200.0, 40.0);
    word(&mut p, "Yes", 92.0, 440.0, 9.0);
    rect(&mut p, 72.0, 438.0, 10.0, 10.0);
    let found = detect(&p);
    let mut k = kinds(&found);
    k.sort();
    assert_eq!(k, vec!["box", "check"]);
    let check = found.iter().find(|b| b.kind == "check").unwrap();
    assert!((check.rect.x - 72.0).abs() < 0.01 && (check.rect.w - 10.0).abs() < 0.01);
}

#[test]
fn a_box_with_text_inside_is_not_empty() {
    let mut p = page();
    rect(&mut p, 72.0, 470.0, 120.0, 16.0);
    word(&mut p, "Already printed here", 74.0, 474.0, 9.0);
    assert!(detect(&p).is_empty());
}

#[test]
fn a_box_around_a_drawing_is_not_empty() {
    let mut p = page();
    rect(&mut p, 72.0, 400.0, 200.0, 150.0);
    p.ink.push(Bounds {
        x0: 100.0,
        y0: 420.0,
        x1: 240.0,
        y1: 520.0,
    });
    assert!(detect(&p).is_empty());
}

/// The IRS layout: a full-width rule above and below, a label in the top
/// left of the cell, and room to write underneath it.
#[test]
fn a_labelled_cell_offers_the_room_under_its_label() {
    let mut p = page();
    word(
        &mut p,
        "Form W-9 Request for Taxpayer Identification",
        36.0,
        704.0,
        12.0,
    );
    hl(&mut p, 36.0, 576.0, 700.0);
    hl(&mut p, 36.0, 576.0, 676.0);
    word(&mut p, "1 Name of entity", 40.0, 692.0, 7.0);
    let found = detect(&p);
    assert_eq!(kinds(&found), vec!["cell"]);
    let r = found[0].rect;
    assert!(r.y >= 676.0 && r.y + r.h <= 692.0, "under the label: {r:?}");
    assert!(r.w > 500.0);
    assert_eq!(found[0].font_size_pt, 8.0, "clamped up from the 7 pt label");
}

#[test]
fn empty_table_cells_are_found_and_the_header_row_is_not() {
    let mut p = page();
    let (x0, cols, w) = (72.0, 3, 120.0);
    let (top, rows, h) = (600.0, 4, 18.0);
    for r in 0..=rows {
        hl(&mut p, x0, x0 + cols as f32 * w, top - r as f32 * h);
    }
    for c in 0..=cols {
        vl(&mut p, x0 + c as f32 * w, top - rows as f32 * h, top);
    }
    for (c, name) in ["Item", "Qty", "Price"].iter().enumerate() {
        word(&mut p, name, x0 + c as f32 * w + 4.0, top - 13.0, 9.0);
    }
    let found = detect(&p);
    assert_eq!(found.len(), 9, "{:?}", kinds(&found));
    assert!(found.iter().all(|b| b.kind == "box"));
    // Reading order: rows top to bottom, left to right within each.
    let xs: Vec<f32> = found.iter().take(3).map(|b| b.rect.x).collect();
    assert_eq!(xs, vec![72.0, 192.0, 312.0]);
    assert!(found[0].rect.y > found[3].rect.y);
}

#[test]
fn a_row_of_equal_boxes_is_one_comb() {
    let mut p = page();
    word(&mut p, "SSN", 72.0, 503.0, 9.0);
    for i in 0..9 {
        rect(&mut p, 100.0 + i as f32 * 14.0, 500.0, 14.0, 16.0);
    }
    let found = detect(&p);
    assert_eq!(kinds(&found), vec!["comb"]);
    assert_eq!(found[0].cells, 9);
    assert!((found[0].rect.w - 126.0).abs() < 0.01);
}

#[test]
fn a_rule_divided_by_ticks_is_a_comb() {
    let mut p = page();
    word(&mut p, "Account", 72.0, 402.0, 9.0);
    hl(&mut p, 120.0, 240.0, 400.0);
    for i in 0..=6 {
        vl(&mut p, 120.0 + i as f32 * 20.0, 400.0, 406.0);
    }
    let found = detect(&p);
    assert_eq!(kinds(&found), vec!["comb"]);
    assert_eq!(found[0].cells, 6);
    assert!((found[0].rect.x - 120.0).abs() < 0.01);
}

#[test]
fn a_ballot_box_glyph_is_a_checkbox() {
    let mut p = page();
    p.glyphs.push(Glyph {
        ch: '\u{2610}',
        b: Bounds {
            x0: 72.0,
            y0: 300.0,
            x1: 81.0,
            y1: 309.0,
        },
        size: 10.0,
    });
    word(&mut p, "Single", 84.0, 301.0, 10.0);
    assert_eq!(kinds(&detect(&p)), vec!["check"]);
}

#[test]
fn a_drawn_ring_is_a_checkbox() {
    let mut p = page();
    p.rings.push(Bounds {
        x0: 72.0,
        y0: 300.0,
        x1: 80.0,
        y1: 308.0,
    });
    assert_eq!(kinds(&detect(&p)), vec!["check"]);
}

// ---------- negatives ----------

/// A booktabs table: rules full width with rows of entries between them.
#[test]
fn table_rules_with_text_are_not_blanks() {
    let mut p = page();
    word(
        &mut p,
        "Table 1: Results on the benchmark",
        72.0,
        712.0,
        10.0,
    );
    hl(&mut p, 72.0, 540.0, 704.0);
    for (i, h) in ["Method", "Accuracy", "Time"].iter().enumerate() {
        word(&mut p, h, 80.0 + i as f32 * 160.0, 692.0, 10.0);
    }
    hl(&mut p, 72.0, 540.0, 686.0);
    for r in 0..3 {
        let y = 672.0 - r as f32 * 14.0;
        for (i, v) in ["Ours", "91.2", "3.1"].iter().enumerate() {
            word(&mut p, v, 80.0 + i as f32 * 160.0, y, 10.0);
        }
    }
    hl(&mut p, 72.0, 540.0, 640.0);
    let found = detect(&p);
    assert!(found.is_empty(), "{found:?}");
}

/// A gridded data table: every cell holds an entry.
#[test]
fn a_full_grid_table_is_not_blanks() {
    let mut p = page();
    let (x0, w, top, h) = (72.0, 140.0, 600.0, 16.0);
    for r in 0..=4 {
        hl(&mut p, x0, x0 + 3.0 * w, top - r as f32 * h);
    }
    for c in 0..=3 {
        vl(&mut p, x0 + c as f32 * w, top - 4.0 * h, top);
    }
    for r in 0..4 {
        for c in 0..3 {
            word(
                &mut p,
                "entry",
                x0 + c as f32 * w + 4.0,
                top - r as f32 * h - 12.0,
                9.0,
            );
        }
    }
    let found = detect(&p);
    assert!(found.is_empty(), "{found:?}");
}

/// Plot axes: an L with ticks pointing out, numbers under them, a curve.
#[test]
fn figure_axes_are_not_blanks() {
    let mut p = page();
    vl(&mut p, 100.0, 400.0, 600.0);
    hl(&mut p, 100.0, 400.0, 400.0);
    for i in 0..=5 {
        let x = 100.0 + i as f32 * 60.0;
        vl(&mut p, x, 396.0, 400.0);
        word(&mut p, &format!("{}", i * 10), x - 4.0, 386.0, 8.0);
    }
    p.ink.push(Bounds {
        x0: 101.0,
        y0: 402.0,
        x1: 398.0,
        y1: 590.0,
    });
    let found = detect(&p);
    assert!(found.is_empty(), "{found:?}");
}

/// Inward ticks, the other way plots draw them, with a framed plot area.
#[test]
fn a_framed_plot_with_inward_ticks_is_not_blanks() {
    let mut p = page();
    rect(&mut p, 100.0, 400.0, 300.0, 200.0);
    for i in 0..=5 {
        let x = 100.0 + i as f32 * 60.0;
        vl(&mut p, x, 400.0, 404.0);
        word(&mut p, &format!("{}", i * 10), x - 4.0, 388.0, 8.0);
    }
    p.ink.push(Bounds {
        x0: 101.0,
        y0: 410.0,
        x1: 398.0,
        y1: 590.0,
    });
    let found = detect(&p);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_rule_under_a_heading_is_not_a_blank() {
    let mut p = page();
    word(&mut p, "Part II Certification", 72.0, 704.0, 12.0);
    hl(&mut p, 72.0, 540.0, 700.0);
    // and one a little further below its heading
    word(&mut p, "Section 3", 72.0, 612.0, 14.0);
    hl(&mut p, 72.0, 540.0, 604.0);
    let found = detect(&p);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn underlined_text_is_not_a_blank() {
    let mut p = page();
    let end = word(&mut p, "Important notice", 72.0, 500.0, 10.0);
    hl(&mut p, 72.0, end, 499.0);
    assert!(detect(&p).is_empty());
}

#[test]
fn a_caption_under_a_signature_line_keeps_it() {
    let mut p = page();
    hl(&mut p, 72.0, 260.0, 200.0);
    word(&mut p, "Signature of applicant", 72.0, 190.0, 7.0);
    assert_eq!(kinds(&detect(&p)), vec!["line"]);
}

#[test]
fn widgets_hide_the_blanks_they_cover() {
    let mut p = page();
    let end = word(&mut p, "Name:", 72.0, 700.0, 10.0);
    hl(&mut p, end + 4.0, 320.0, 699.5);
    let found = detect(&p);
    let widget = RectDto {
        x: end + 4.0,
        y: 699.0,
        w: 200.0,
        h: 14.0,
    };
    assert!(drop_covered(found.clone(), &[widget]).is_empty());
    let elsewhere = RectDto {
        x: 72.0,
        y: 100.0,
        w: 100.0,
        h: 14.0,
    };
    assert_eq!(drop_covered(found, &[elsewhere]).len(), 1);
}

#[test]
fn pieces_of_one_rule_merge() {
    let merged = merge_h(vec![
        HLine {
            x0: 10.0,
            x1: 50.0,
            y: 100.0,
            t: 0.5,
        },
        HLine {
            x0: 50.5,
            x1: 90.0,
            y: 100.2,
            t: 0.5,
        },
        HLine {
            x0: 120.0,
            x1: 150.0,
            y: 100.0,
            t: 0.5,
        },
    ]);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].x1, 90.0);
}

#[test]
fn axis_rects_are_recognised() {
    assert!(is_axis_rect(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 5.0),
        (0.0, 5.0),
        (0.0, 0.0)
    ]));
    assert!(!is_axis_rect(&[
        (0.0, 0.0),
        (10.0, 3.0),
        (10.0, 5.0),
        (0.0, 5.0)
    ]));
}

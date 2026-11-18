//! Fill & Sign marks: a check, a cross, a dot, a strike line or a circle
//! placed on a printed form.
//!
//! Each is written as a Stamp annotation whose appearance is one vector path,
//! with `/T` naming it as SpeedyF's and `/Contents` naming the mark, so it
//! reopens as the same editable mark (`annots.rs`) and looks the same in
//! every viewer.

use super::save::{parse_color, PageSpace};
use super::signature::{path_object, Placement, Seg};
use super::types::PlanAnnot;
use crate::errors::AppResult;
use pdfium_render::prelude::*;

/// `/T` of every mark stamp SpeedyF writes.
pub(crate) const MARK_CREATOR: &str = "SpeedyF Mark";
/// The marks there are, by the name kept in `/Contents`.
pub(crate) const MARKS: [&str; 5] = ["check", "cross", "dot", "line", "circle"];

/// Bezier handle length for a quarter circle of radius 1.
const KAPPA: f32 = 0.552_284_8;

/// An ellipse centred on (`cx`, `cy`) as four cubic arcs.
fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Vec<Seg> {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    vec![
        Seg::Move(cx + rx, cy),
        Seg::Cubic(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry),
        Seg::Cubic(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy),
        Seg::Cubic(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry),
        Seg::Cubic(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy),
        Seg::Close,
    ]
}

/// The path of a mark in a `w` x `h` box (points, y down), and whether it is
/// filled (a dot) rather than stroked.
pub(crate) fn mark_segs(mark: &str, w: f32, h: f32, pen: f32) -> (Vec<Seg>, bool) {
    match mark {
        "check" => (
            vec![
                Seg::Move(0.14 * w, 0.55 * h),
                Seg::Line(0.4 * w, 0.82 * h),
                Seg::Line(0.88 * w, 0.18 * h),
            ],
            false,
        ),
        "cross" => (
            vec![
                Seg::Move(0.2 * w, 0.2 * h),
                Seg::Line(0.8 * w, 0.8 * h),
                Seg::Move(0.8 * w, 0.2 * h),
                Seg::Line(0.2 * w, 0.8 * h),
            ],
            false,
        ),
        "dot" => {
            let r = 0.3 * w.min(h);
            (ellipse(w / 2.0, h / 2.0, r, r), true)
        }
        "line" => (vec![Seg::Move(0.0, h / 2.0), Seg::Line(w, h / 2.0)], false),
        // "circle", and anything unknown: an outline inside the box.
        _ => (
            ellipse(
                w / 2.0,
                h / 2.0,
                (w / 2.0 - pen / 2.0).max(0.5),
                (h / 2.0 - pen / 2.0).max(0.5),
            ),
            false,
        ),
    }
}

/// The pen a mark is drawn with when the plan does not say.
pub(crate) fn default_pen(w: f32, h: f32) -> f32 {
    (0.1 * w.min(h)).clamp(1.0, 2.5)
}

/// Write one mark from the edit plan as a Stamp annotation.
pub(crate) fn write<'a>(
    doc: &PdfDocument<'a>,
    page: &mut PdfPage<'a>,
    space: &PageSpace,
    annot: &PlanAnnot,
) -> AppResult<()> {
    let mark = annot
        .mark
        .as_deref()
        .filter(|m| MARKS.contains(m))
        .unwrap_or("check");
    let (w, h) = (annot.rect.w, annot.rect.h);
    if w <= 0.0 || h <= 0.0 {
        return Ok(());
    }
    let pen = annot.stroke_width.unwrap_or_else(|| default_pen(w, h));
    let color = parse_color(&annot.color, annot.opacity);
    let (segs, filled) = mark_segs(mark, w, h, pen);
    let at = Placement::new(space, annot.rect, w, h);
    let path = if filled {
        path_object(doc, &segs, &at, None, Some(color))?
    } else {
        path_object(doc, &segs, &at, Some((color, pen)), None)?
    };
    let Some(path) = path else {
        return Ok(());
    };
    let mut stamp = page.annotations_mut().create_stamp_annotation()?;
    // Bounds before objects: PDFium sizes the appearance /BBox from the rect
    // at the moment an object is appended (see save.rs, "bubble").
    stamp.set_bounds(space.rect_to_user(&annot.rect))?;
    stamp.set_contents(mark)?;
    stamp.set_creator(MARK_CREATOR)?;
    stamp.set_is_printed(true)?;
    stamp.objects_mut().add_path_object(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::annots::read_annotations;
    use crate::engine::pdfium_init;
    use crate::engine::save::build_output;
    use crate::engine::types::{EditPlan, PlanPage, PlanText, RectDto, TextStyle};
    use std::path::Path;

    const PAGE_W: f32 = 300.0;
    const PAGE_H: f32 = 420.0;

    fn test_pdfium() -> Pdfium {
        Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
    }

    fn blank_fixture(pdfium: &Pdfium, path: &Path) {
        let mut document = pdfium.create_new_pdf().expect("create fixture");
        document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(PAGE_W),
                PdfPoints::new(PAGE_H),
            ))
            .expect("page");
        document.save_to_file(path).expect("save fixture");
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> RectDto {
        RectDto { x, y, w, h }
    }

    fn mark(name: &str, r: RectDto) -> PlanAnnot {
        PlanAnnot {
            kind: "mark".into(),
            rect: r,
            color: "#1a1a1a".into(),
            opacity: 1.0,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
            signature: None,
            flatten: false,
            mark: Some(name.into()),
        }
    }

    /// Inked pixels (darker than light grey: a thin rule antialiases) inside
    /// a display rect of the page rendered at 1 px/pt.
    /// Statistics only; nothing is looked at.
    fn ink_in(page: &PdfPage<'_>, r: RectDto) -> usize {
        let (w, h) = (page.width().value as i32, page.height().value as i32);
        let bitmap = page.render(w, h, None).expect("render");
        let rgba = bitmap.as_rgba_bytes();
        let mut dark = 0;
        let y_top = (h as f32 - r.y - r.h).max(0.0) as i32;
        let y_bottom = (h as f32 - r.y).min(h as f32) as i32;
        for y in y_top..y_bottom {
            for x in (r.x as i32).max(0)..((r.x + r.w) as i32).min(w) {
                let i = ((y * w + x) * 4) as usize;
                if rgba[i] < 200 && rgba[i + 1] < 200 && rgba[i + 2] < 200 {
                    dark += 1;
                }
            }
        }
        dark
    }

    #[test]
    fn every_mark_has_a_path() {
        for name in MARKS {
            let (segs, filled) = mark_segs(name, 12.0, 12.0, 1.2);
            assert!(segs.len() >= 2, "{name}");
            assert_eq!(filled, name == "dot", "{name}");
        }
    }

    #[test]
    fn marks_and_comb_text_save_with_appearances_and_reopen_editable() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let (source, output) = (dir.path().join("in.pdf"), dir.path().join("out.pdf"));
        blank_fixture(&pdfium, &source);

        let boxes: Vec<(&str, RectDto)> = vec![
            ("check", rect(20.0, 300.0, 14.0, 14.0)),
            ("cross", rect(60.0, 300.0, 14.0, 14.0)),
            ("dot", rect(100.0, 300.0, 14.0, 14.0)),
            ("line", rect(140.0, 300.0, 40.0, 6.0)),
            ("circle", rect(200.0, 300.0, 30.0, 16.0)),
        ];
        let comb = rect(20.0, 200.0, 120.0, 16.0);
        let plan = EditPlan {
            pages: vec![PlanPage {
                src_index: Some(0),
                width_pt: PAGE_W,
                height_pt: PAGE_H,
                rotation: 0,
                annots: boxes.iter().map(|(n, r)| mark(n, *r)).collect(),
                texts: vec![PlanText {
                    rect: comb,
                    text: "123456".into(),
                    font_size_pt: 12.0,
                    color: "#000000".into(),
                    opacity: 1.0,
                    style: TextStyle::default(),
                    comb_cells: Some(6),
                }],
                images: Vec::new(),
                drop_src_annots: Vec::new(),
            }],
            form: Vec::new(),
            form_buttons: Vec::new(),
            form_choices: Vec::new(),
            form_display: Vec::new(),
            form_design: None,
        };
        build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output).expect("save");

        let bytes = std::fs::read(&output).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("SpeedyF Mark"));
        assert!(text.contains("SpeedyFComb"));

        let document = pdfium.load_pdf_from_file(&output, None).unwrap();
        let page = document.pages().get(0).unwrap();
        // Every mark has an appearance that puts ink inside its own box.
        for (name, r) in &boxes {
            assert!(ink_in(&page, *r) > 3, "no ink for {name} in {r:?}");
        }
        // The comb's characters are spread across all six cells.
        let pitch = comb.w / 6.0;
        for i in 0..6 {
            let cell = rect(comb.x + pitch * i as f32, comb.y, pitch, comb.h);
            assert!(ink_in(&page, cell) > 3, "cell {i} is empty");
        }
        drop(page);
        drop(document);

        let pages = read_annotations(&pdfium, output.to_str().unwrap(), None).unwrap();
        let annots = &pages[0].annots;
        let marks: Vec<&str> = annots
            .iter()
            .filter(|a| a.kind == "mark")
            .filter_map(|a| a.mark.as_deref())
            .collect();
        assert_eq!(marks, vec!["check", "cross", "dot", "line", "circle"]);
        for a in annots.iter().filter(|a| a.kind == "mark") {
            assert_eq!(a.color, "#1a1a1a");
        }
        let text_box = annots.iter().find(|a| a.kind == "textbox").unwrap();
        assert_eq!(text_box.text.as_deref(), Some("123456"));
        assert_eq!(text_box.comb_cells, Some(6));
        assert!((text_box.rect.x - comb.x).abs() < 0.5);
        assert!((text_box.rect.w - comb.w).abs() < 0.5);
    }
}

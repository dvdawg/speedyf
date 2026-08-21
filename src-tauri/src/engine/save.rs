//! EditPlan materialization + safe atomic save.
//!
//! Uses the high-level pdfium-render wrapper (editing happens once per save,
//! so wrapper ergonomics beat raw FFI here):
//!   new doc → import kept/reordered/duplicated pages (FPDF_ImportPages under
//!   the hood) → blank pages → true PDF annotations (Highlight/Square/Text
//!   note) → ink flattened to page path objects → added text as real page
//!   text objects → images as page image objects → AcroForm text values →
//!   write to a temp sibling → reopen-verify → atomic rename.
//!
//! Annotation coordinates arrive in display-normalized page space (y-up,
//! origin at the displayed crop-box bottom-left); `disp_to_user` maps them
//! back into raw PDF user space honoring crop box + /Rotate.

use crate::engine::types::{EditPlan, RectDto};
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::*;
use std::path::Path;

/// Write a replacement to a temporary sibling, verify it, perform any
/// last-moment handle cleanup, then atomically persist it over `dest`.
///
/// `TempPath` removes the sibling on every early-return path, so a writer or
/// verifier failure cannot modify the destination or leave save debris.
pub fn verified_atomic_replace(
    dest: &Path,
    write: impl FnOnce(&Path) -> AppResult<()>,
    verify: impl FnOnce(&Path) -> AppResult<()>,
    before_replace: impl FnOnce() -> AppResult<()>,
) -> AppResult<u64> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| AppError::Io("destination has no parent directory".into()))?;
    let temp = tempfile::Builder::new()
        .prefix(".speedyf-save-")
        .suffix(".pdf")
        .tempfile_in(dir)
        .map_err(|e| AppError::Io(format!("cannot create temp file: {e}")))?;
    let temp_path = temp.into_temp_path();

    write(&temp_path)?;
    verify(&temp_path)?;
    before_replace()?;

    let bytes = std::fs::metadata(&temp_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    temp_path
        .persist(dest)
        .map_err(|e| AppError::Io(format!("cannot replace destination: {e}")))?;
    Ok(bytes)
}

fn parse_color(hex: &str, opacity: f32) -> PdfColor {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    let (r, g, b) = if h.len() >= 6 {
        (
            ((v >> 16) & 0xff) as u8,
            ((v >> 8) & 0xff) as u8,
            (v & 0xff) as u8,
        )
    } else {
        (0, 0, 0)
    };
    PdfColor::new(r, g, b, (opacity.clamp(0.0, 1.0) * 255.0) as u8)
}

/// The mapping between the display-normalized space annotations arrive in
/// (y-up, origin at the displayed crop-box bottom-left) and raw PDF user space.
///
/// Both directions live here deliberately. Reading annotations back out of a
/// file needs the exact inverse of what writing them applies, and a second
/// implementation elsewhere would drift from this one the first time a crop box
/// or `/Rotate` case was fixed in only one of them.
pub(crate) struct PageSpace {
    crop_l: f32,
    crop_b: f32,
    crop_r: f32,
    crop_t: f32,
    rot: u16,
}

impl PageSpace {
    pub(crate) fn of(page: &PdfPage<'_>) -> PageSpace {
        let crop = page
            .boundaries()
            .crop()
            .map(|b| b.bounds)
            .or_else(|_| page.boundaries().media().map(|b| b.bounds))
            .unwrap_or(PdfRect::new(
                PdfPoints::new(0.0),
                PdfPoints::new(0.0),
                PdfPoints::new(page.height().value),
                PdfPoints::new(page.width().value),
            ));
        let rot = match page.rotation() {
            Ok(PdfPageRenderRotation::Degrees90) => 90,
            Ok(PdfPageRenderRotation::Degrees180) => 180,
            Ok(PdfPageRenderRotation::Degrees270) => 270,
            _ => 0,
        };
        PageSpace {
            crop_l: crop.left().value,
            crop_b: crop.bottom().value,
            crop_r: crop.right().value,
            crop_t: crop.top().value,
            rot,
        }
    }

    /// display-normalized (y-up) point → raw PDF user-space point
    pub(crate) fn disp_to_user(&self, dx: f32, dy: f32) -> (f32, f32) {
        let (l, b, r, t) = (self.crop_l, self.crop_b, self.crop_r, self.crop_t);
        match self.rot {
            90 => (l + (r - l) - dy, b + dx),
            180 => (l + (r - l) - dx, b + (t - b) - dy),
            270 => (l + dy, b + (t - b) - dx),
            _ => (l + dx, b + dy),
        }
    }

    /// raw PDF user-space point → display-normalized (y-up) point.
    ///
    /// The exact algebraic inverse of `disp_to_user`; the round-trip test below
    /// is what keeps it that way.
    pub(crate) fn user_to_disp(&self, ux: f32, uy: f32) -> (f32, f32) {
        let (l, b, r, t) = (self.crop_l, self.crop_b, self.crop_r, self.crop_t);
        match self.rot {
            90 => (uy - b, r - ux),
            180 => (r - ux, t - uy),
            270 => (t - uy, ux - l),
            _ => (ux - l, uy - b),
        }
    }

    pub(crate) fn rect_to_user(&self, rc: &RectDto) -> PdfRect {
        let (x1, y1) = self.disp_to_user(rc.x, rc.y);
        let (x2, y2) = self.disp_to_user(rc.x + rc.w, rc.y + rc.h);
        PdfRect::new(
            PdfPoints::new(y1.min(y2)),
            PdfPoints::new(x1.min(x2)),
            PdfPoints::new(y1.max(y2)),
            PdfPoints::new(x1.max(x2)),
        )
    }

    /// A user-space rectangle as a display-space one. Both corners are mapped
    /// and then re-normalized, because a rotated page swaps which corner is
    /// which — taking the mapped corners as-is would give negative extents.
    pub(crate) fn rect_from_user(&self, rect: &PdfRect) -> RectDto {
        let (x1, y1) = self.user_to_disp(rect.left().value, rect.bottom().value);
        let (x2, y2) = self.user_to_disp(rect.right().value, rect.top().value);
        RectDto {
            x: x1.min(x2),
            y: y1.min(y2),
            w: (x2 - x1).abs(),
            h: (y2 - y1).abs(),
        }
    }
}

fn add_annotations<'a>(
    doc: &PdfDocument<'a>,
    page: &mut PdfPage<'a>,
    plan_page: &crate::engine::types::PlanPage,
    helv: PdfFontToken,
) -> AppResult<()> {
    let space = PageSpace::of(page);

    for annot in &plan_page.annots {
        match annot.kind.as_str() {
            "highlight" => {
                let mut a = page.annotations_mut().create_highlight_annotation()?;
                a.set_fill_color(parse_color(&annot.color, annot.opacity))
                    .ok();
                let mut union: Option<PdfRect> = None;
                if let Some(quads) = &annot.quads {
                    for q in quads {
                        let pts = [
                            space.disp_to_user(q.p1.x, q.p1.y),
                            space.disp_to_user(q.p2.x, q.p2.y),
                            space.disp_to_user(q.p3.x, q.p3.y),
                            space.disp_to_user(q.p4.x, q.p4.y),
                        ];
                        let xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
                        let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();
                        let rect = PdfRect::new(
                            PdfPoints::new(ys.iter().cloned().fold(f32::MAX, f32::min)),
                            PdfPoints::new(xs.iter().cloned().fold(f32::MAX, f32::min)),
                            PdfPoints::new(ys.iter().cloned().fold(f32::MIN, f32::max)),
                            PdfPoints::new(xs.iter().cloned().fold(f32::MIN, f32::max)),
                        );
                        a.attachment_points_mut()
                            .create_attachment_point_at_end(PdfQuadPoints::from_rect(&rect))?;
                        union = Some(match union {
                            None => rect,
                            Some(u) => PdfRect::new(
                                PdfPoints::new(u.bottom().value.min(rect.bottom().value)),
                                PdfPoints::new(u.left().value.min(rect.left().value)),
                                PdfPoints::new(u.top().value.max(rect.top().value)),
                                PdfPoints::new(u.right().value.max(rect.right().value)),
                            ),
                        });
                    }
                }
                a.set_bounds(union.unwrap_or_else(|| space.rect_to_user(&annot.rect)))?;
            }
            "rect" => {
                let mut a = page.annotations_mut().create_square_annotation()?;
                a.set_bounds(space.rect_to_user(&annot.rect))?;
                a.set_stroke_color(parse_color(&annot.color, annot.opacity))
                    .ok();
                a.set_width(PdfPoints::new(annot.stroke_width.unwrap_or(2.0)))
                    .ok();
            }
            "note" => {
                let text = annot.text.clone().unwrap_or_default();
                let mut a = page.annotations_mut().create_text_annotation(&text)?;
                a.set_bounds(space.rect_to_user(&annot.rect))?;
                a.set_fill_color(parse_color(&annot.color, annot.opacity))
                    .ok();
            }
            "ink" => {
                // A real Ink annotation, holding its strokes as path objects
                // *inside itself*. This used to flatten them onto the page,
                // which rendered everywhere but made every stroke permanent:
                // once saved it was page content, indistinguishable from the
                // paper's own figures and impossible to move or delete again.
                //
                // The objects live in the annotation, so it carries its own
                // appearance rather than relying on a viewer to synthesize one.
                //
                // Do NOT set a stroke colour on the annotation itself. PDFium
                // refuses `FPDFAnnot_SetColor` for any annotation carrying an
                // `/AP` entry, and `pdfium-render` answers that refusal by
                // retrying through `FPDFPageObj_SetStrokeColor` with the
                // `FPDF_ANNOTATION` handle cast to `FPDF_PAGEOBJECT` — so
                // PDFium writes through an annotation as though it were a page
                // object. macOS survives the type-confused access; Linux and
                // Windows fault. `annots.rs` avoids the reading half of the
                // same trap. The colour and width belong to the path objects
                // anyway, which is where the reader takes them from.
                let Some(strokes) = &annot.strokes else {
                    continue;
                };
                let color = parse_color(&annot.color, annot.opacity);
                let width = PdfPoints::new(annot.stroke_width.unwrap_or(2.0));
                let usable: Vec<_> = strokes.iter().filter(|s| s.len() >= 2).collect();
                if usable.is_empty() {
                    continue;
                }
                let mut a = page.annotations_mut().create_ink_annotation()?;
                for stroke in usable {
                    let (x0, y0) = space.disp_to_user(stroke[0].x, stroke[0].y);
                    let mut path = PdfPagePathObject::new(
                        doc,
                        PdfPoints::new(x0),
                        PdfPoints::new(y0),
                        Some(color),
                        Some(width),
                        None,
                    )?;
                    for p in &stroke[1..] {
                        let (x, y) = space.disp_to_user(p.x, p.y);
                        path.line_to(PdfPoints::new(x), PdfPoints::new(y))?;
                    }
                    a.objects_mut().add_path_object(path)?;
                }
                a.set_bounds(space.rect_to_user(&annot.rect))?;
            }
            other => {
                log::warn!("unknown annotation kind ignored at save: {other}");
            }
        }
    }

    // Typed text becomes a FreeText annotation holding its own text objects,
    // rather than text objects written straight onto the page. Same reason as
    // ink: flattened text is indistinguishable from the document's own words
    // the moment it is saved, and cannot be edited or removed afterwards.
    for text in &plan_page.texts {
        let user_rect = space.rect_to_user(&text.rect);
        let size = text.font_size_pt.max(4.0);
        let color = parse_color(&text.color, text.opacity);
        let mut a = page
            .annotations_mut()
            .create_free_text_annotation(&text.text)?;
        a.set_bounds(user_rect)?;
        a.set_fill_color(color).ok();
        let _ = (doc, helv, size);
    }

    // Images become Stamp annotations carrying the image object.
    for img in &plan_page.images {
        let dyn_img = image::open(&img.source_path)
            .map_err(|e| AppError::Io(format!("cannot load image {}: {e}", img.source_path)))?;
        let user_rect = space.rect_to_user(&img.rect);
        let mut obj = PdfPageImageObject::new(doc, &dyn_img)?;
        let w = user_rect.right().value - user_rect.left().value;
        let h = user_rect.top().value - user_rect.bottom().value;
        obj.scale(w, h)?;
        obj.translate(
            PdfPoints::new(user_rect.left().value),
            PdfPoints::new(user_rect.bottom().value),
        )?;
        let mut a = page.annotations_mut().create_stamp_annotation()?;
        a.objects_mut().add_image_object(obj)?;
        a.set_bounds(user_rect)?;
    }

    Ok(())
}

fn apply_rotation(page: &mut PdfPage<'_>, extra_deg: u16) {
    if extra_deg % 360 == 0 {
        return;
    }
    let cur = match page.rotation() {
        Ok(PdfPageRenderRotation::Degrees90) => 90u16,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        _ => 0,
    };
    let total = (cur + extra_deg) % 360;
    page.set_rotation(match total {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    });
}

fn page_requires_mutation(page: &crate::engine::types::PlanPage) -> bool {
    page.rotation % 360 != 0
        || !page.annots.is_empty()
        || !page.texts.is_empty()
        || !page.images.is_empty()
        || !page.drop_src_annots.is_empty()
}

/// Remove the annotations the plan says were edited or deleted, before the
/// plan's own versions are written in their place.
///
/// Importing a page brings its annotations with it, so an annotation the user
/// loaded, edited and re-saved would otherwise appear twice: once as imported,
/// once as written. Only the indices named here are dropped — an annotation
/// nobody touched stays exactly as it came in, keeping the author, dates, popup
/// and appearance stream that this model has no place to put.
///
/// Indices refer to the source page's own enumeration, which import preserves.
/// They are removed high-to-low so that removing one cannot shift the position
/// of another still to be removed.
fn drop_replaced_annotations(page: &mut PdfPage<'_>, drop: &[u32]) {
    if drop.is_empty() {
        return;
    }
    let mut indices: Vec<u32> = drop.to_vec();
    indices.sort_unstable();
    indices.dedup();

    for index in indices.into_iter().rev() {
        let Ok(annot) = page.annotations().get(index as usize) else {
            continue;
        };
        // Refuse to delete anything the reader would not have surfaced: the
        // frontend can only have meant an annotation it was actually given.
        if !crate::engine::annots::is_owned(&annot) {
            log::warn!("refusing to drop annotation {index}: not a kind SpeedyF owns");
            continue;
        }
        if let Err(error) = page.annotations_mut().delete_annotation(annot) {
            log::warn!("cannot drop annotation {index}: {error:?}");
        }
    }
}

fn fill_form_fields(doc: &mut PdfDocument<'_>, form: &[(String, String)]) {
    if form.is_empty() || doc.form().is_none() {
        return;
    }
    let page_count = doc.pages().len();
    for pi in 0..page_count {
        let Ok(page) = doc.pages().get(pi) else {
            continue;
        };
        let annotation_count = page.annotations().len();
        for ai in 0..annotation_count {
            let Ok(mut annotation) = page.annotations().get(ai) else {
                continue;
            };
            let Some(field) = annotation.as_form_field_mut() else {
                continue;
            };
            let name = field.name().unwrap_or_default();
            let Some((_, value)) = form.iter().find(|(n, _)| *n == name) else {
                continue;
            };
            if let Some(text_field) = field.as_text_field_mut() {
                if let Err(e) = text_field.set_value(value) {
                    log::warn!("form fill failed for '{name}': {e:?}");
                }
            }
        }
    }
}

/// Build the output document from `plan` and write it to `out_path`.
pub fn build_output(
    pdfium: &Pdfium,
    src_path: &str,
    password: Option<&str>,
    plan: &EditPlan,
    out_path: &Path,
) -> AppResult<()> {
    let src = pdfium
        .load_pdf_from_file(src_path, password)
        .map_err(|e| AppError::Malformed(format!("cannot reopen source: {e:?}")))?;
    let mut out = pdfium.create_new_pdf()?;
    let helv = out.fonts_mut().helvetica();

    // 1) assemble pages: coalesce consecutive source pages into single
    //    FPDF_ImportPages calls (1-based index list), create blanks inline.
    let mut dest: u16 = 0;
    let mut i = 0;
    while i < plan.pages.len() {
        if plan.pages[i].src_index.is_some() {
            let mut list = String::new();
            let mut j = i;
            while j < plan.pages.len() {
                if let Some(s) = plan.pages[j].src_index {
                    if !list.is_empty() {
                        list.push(',');
                    }
                    list.push_str(&(s + 1).to_string());
                    j += 1;
                } else {
                    break;
                }
            }
            out.pages_mut()
                .copy_pages_from_document(&src, &list, dest)
                .map_err(|e| AppError::Internal(format!("page import failed: {e:?}")))?;
            dest += (j - i) as u16;
            i = j;
        } else {
            let pp = &plan.pages[i];
            out.pages_mut()
                .create_page_at_index(
                    PdfPagePaperSize::Custom(
                        PdfPoints::new(pp.width_pt),
                        PdfPoints::new(pp.height_pt),
                    ),
                    dest,
                )
                .map_err(|e| AppError::Internal(format!("blank page failed: {e:?}")))?;
            dest += 1;
            i += 1;
        }
    }

    // 2) per-page: annotations/texts/images in ORIGINAL display space, then rotation delta.
    for (idx, pp) in plan.pages.iter().enumerate() {
        if !page_requires_mutation(pp) {
            continue;
        }
        let mut page = out.pages().get(idx as u16)?;
        drop_replaced_annotations(&mut page, &pp.drop_src_annots);
        add_annotations(&out, &mut page, pp, helv)?;
        apply_rotation(&mut page, pp.rotation % 360);
    }

    // 3) AcroForm text values (best effort — unsupported fields are logged).
    fill_form_fields(&mut out, &plan.form);

    // 4) write
    let mut file = std::fs::File::create(out_path)?;
    out.save_to_writer(&mut file)
        .map_err(|e| AppError::Io(format!("write failed: {e:?}")))?;
    use std::io::Write;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pdfium_init;
    use crate::engine::types::{PlanAnnot, PlanPage, PlanText, PointDto, QuadDto};

    fn test_pdfium() -> Pdfium {
        Pdfium::new(pdfium_init::init_bindings(&[]).expect("load bundled PDFium"))
    }

    fn create_source_fixture(pdfium: &Pdfium, path: &Path) {
        let mut document = pdfium.create_new_pdf().expect("create fixture");
        let font = document.fonts_mut().helvetica();
        let specs = [
            ("SOURCE-A", PdfPageRenderRotation::None),
            ("SOURCE-B", PdfPageRenderRotation::Degrees90),
            ("SOURCE-C", PdfPageRenderRotation::Degrees270),
        ];
        for (index, (label, rotation)) in specs.into_iter().enumerate() {
            let mut page = document
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::Custom(
                    PdfPoints::new(300.0 + index as f32 * 20.0),
                    PdfPoints::new(420.0 + index as f32 * 20.0),
                ))
                .expect("create fixture page");
            let mut text = PdfPageTextObject::new(&document, label, font, PdfPoints::new(22.0))
                .expect("create fixture text");
            text.translate(PdfPoints::new(36.0), PdfPoints::new(300.0))
                .expect("position fixture text");
            page.objects_mut()
                .add_text_object(text)
                .expect("add fixture text");
            page.set_rotation(rotation);
        }
        document.save_to_file(path).expect("save fixture");
    }

    fn plan_page(src_index: Option<u32>, rotation: u16) -> PlanPage {
        PlanPage {
            src_index,
            width_pt: 360.0,
            height_pt: 480.0,
            rotation,
            annots: Vec::new(),
            texts: Vec::new(),
            images: Vec::new(),
            drop_src_annots: Vec::new(),
        }
    }

    fn rotation_degrees(page: &PdfPage<'_>) -> u16 {
        match page.rotation().expect("read page rotation") {
            PdfPageRenderRotation::Degrees90 => 90,
            PdfPageRenderRotation::Degrees180 => 180,
            PdfPageRenderRotation::Degrees270 => 270,
            PdfPageRenderRotation::None => 0,
        }
    }

    #[test]
    fn unchanged_imported_pages_do_not_require_a_mutation_pass() {
        let mut page = plan_page(Some(0), 0);
        assert!(!page_requires_mutation(&page));

        page.rotation = 90;
        assert!(page_requires_mutation(&page));
        page.rotation = 0;
        page.texts.push(PlanText {
            rect: RectDto {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            text: "changed".into(),
            font_size_pt: 10.0,
            color: "#000000".into(),
            opacity: 1.0,
        });
        assert!(page_requires_mutation(&page));
    }

    #[test]
    fn materializes_page_order_duplicates_blanks_and_rotation_deltas() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_source_fixture(&pdfium, &source);

        let plan = EditPlan {
            pages: vec![
                plan_page(Some(2), 90),
                plan_page(Some(0), 180),
                plan_page(Some(1), 90),
                plan_page(Some(2), 0),
                plan_page(None, 270),
            ],
            form: Vec::new(),
        };
        build_output(
            &pdfium,
            source.to_str().expect("source path"),
            None,
            &plan,
            &output,
        )
        .expect("build edited output");

        let saved = pdfium
            .load_pdf_from_file(&output, None)
            .expect("reopen edited output");
        assert_eq!(saved.pages().len(), 5);

        let mut text_order = Vec::new();
        let mut rotations = Vec::new();
        for index in 0..saved.pages().len() {
            let page = saved.pages().get(index).expect("saved page");
            text_order.push(page.text().expect("page text").all());
            rotations.push(rotation_degrees(&page));
        }
        assert!(text_order[0].contains("SOURCE-C"));
        assert!(text_order[1].contains("SOURCE-A"));
        assert!(text_order[2].contains("SOURCE-B"));
        assert!(text_order[3].contains("SOURCE-C"));
        assert!(text_order[4].trim().is_empty());
        assert_eq!(rotations, vec![0, 180, 180, 270, 270]);
    }

    #[test]
    fn every_annotation_kind_survives_a_save_and_reads_back() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_source_fixture(&pdfium, &source);

        let rect = RectDto {
            x: 40.0,
            y: 50.0,
            w: 100.0,
            h: 40.0,
        };
        let point = |x, y| PointDto { x, y };
        let mut page = plan_page(Some(0), 0);
        page.annots = vec![
            PlanAnnot {
                kind: "highlight".into(),
                rect,
                color: "#ffd54a".into(),
                opacity: 0.5,
                stroke_width: None,
                quads: Some(vec![QuadDto {
                    p1: point(40.0, 50.0),
                    p2: point(140.0, 50.0),
                    p3: point(140.0, 70.0),
                    p4: point(40.0, 70.0),
                }]),
                strokes: None,
                text: None,
            },
            PlanAnnot {
                kind: "rect".into(),
                rect: RectDto {
                    x: 30.0,
                    y: 100.0,
                    w: 120.0,
                    h: 60.0,
                },
                color: "#1e88e5".into(),
                opacity: 1.0,
                stroke_width: Some(3.0),
                quads: None,
                strokes: None,
                text: None,
            },
            PlanAnnot {
                kind: "note".into(),
                rect: RectDto {
                    x: 180.0,
                    y: 120.0,
                    w: 18.0,
                    h: 18.0,
                },
                color: "#ffeb3b".into(),
                opacity: 1.0,
                stroke_width: None,
                quads: None,
                strokes: None,
                text: Some("fixture note".into()),
            },
            PlanAnnot {
                kind: "ink".into(),
                rect: RectDto {
                    x: 25.0,
                    y: 190.0,
                    w: 100.0,
                    h: 40.0,
                },
                color: "#d81b60".into(),
                opacity: 0.8,
                stroke_width: Some(2.5),
                quads: None,
                strokes: Some(vec![vec![
                    point(25.0, 190.0),
                    point(70.0, 225.0),
                    point(125.0, 200.0),
                ]]),
                text: None,
            },
        ];
        page.texts.push(PlanText {
            rect: RectDto {
                x: 36.0,
                y: 250.0,
                w: 180.0,
                h: 40.0,
            },
            text: "ADDED-TEXT".into(),
            font_size_pt: 16.0,
            color: "#111111".into(),
            opacity: 1.0,
        });
        let plan = EditPlan {
            pages: vec![page],
            form: Vec::new(),
        };
        build_output(
            &pdfium,
            source.to_str().expect("source path"),
            None,
            &plan,
            &output,
        )
        .expect("build annotated output");

        // Everything written is a real annotation now — ink and typed text
        // included. They used to be flattened into page content, which meant
        // that saving made them permanent.
        let saved = pdfium
            .load_pdf_from_file(&output, None)
            .expect("reopen annotated output");
        let page = saved.pages().get(0).expect("saved page");
        assert_eq!(page.annotations().len(), 5);
        assert!(
            !page
                .objects()
                .iter()
                .any(|object| object.as_path_object().is_some()),
            "ink must no longer be flattened onto the page"
        );

        // ...and every one of them reads back through the annotation reader.
        let read = crate::engine::annots::read_annotations(
            &pdfium,
            output.to_str().expect("output path"),
            None,
        )
        .expect("read annotations back");
        let annots = &read.iter().find(|p| p.src == 0).expect("page 0").annots;
        let kinds: Vec<&str> = annots.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["highlight", "rect", "note", "ink", "textbox"],
            "every kind must survive the round-trip"
        );

        let note = annots.iter().find(|a| a.kind == "note").expect("note");
        assert_eq!(note.text.as_deref(), Some("fixture note"));
        let textbox = annots
            .iter()
            .find(|a| a.kind == "textbox")
            .expect("textbox");
        assert_eq!(textbox.text.as_deref(), Some("ADDED-TEXT"));

        // Geometry survives: the highlight comes back where it was put, in the
        // same display space it was written from.
        let highlight = annots
            .iter()
            .find(|a| a.kind == "highlight")
            .expect("highlight");
        assert!(
            (highlight.rect.x - 40.0).abs() < 1.0 && (highlight.rect.y - 50.0).abs() < 1.0,
            "highlight came back at {:?}",
            highlight.rect
        );
        assert_eq!(highlight.quads.as_ref().map(Vec::len), Some(1));

        // Ink keeps its colour and its shape, both of which live on the path
        // objects rather than on the annotation.
        let ink = annots.iter().find(|a| a.kind == "ink").expect("ink");
        assert_eq!(ink.color, "#d81b60");
        let strokes = ink.strokes.as_ref().expect("ink strokes");
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].len(), 3, "all three points of the stroke");
        assert!(
            (strokes[0][1].x - 70.0).abs() < 1.0 && (strokes[0][1].y - 225.0).abs() < 1.0,
            "middle point came back at {:?}",
            strokes[0][1]
        );
    }

    /// A source carrying annotations SpeedyF did not write: one it models
    /// (a highlight, as another tool would leave it) and one it does not (a
    /// link).
    fn create_annotated_fixture(pdfium: &Pdfium, path: &Path) {
        let mut document = pdfium.create_new_pdf().expect("create fixture");
        let mut page = document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(
                PdfPoints::new(300.0),
                PdfPoints::new(400.0),
            ))
            .expect("fixture page");
        for top in [80.0f32, 160.0] {
            let mut a = page
                .annotations_mut()
                .create_highlight_annotation()
                .expect("fixture highlight");
            a.set_bounds(PdfRect::new(
                PdfPoints::new(top - 20.0),
                PdfPoints::new(20.0),
                PdfPoints::new(top),
                PdfPoints::new(120.0),
            ))
            .expect("fixture bounds");
        }
        page.annotations_mut()
            .create_link_annotation("https://example.com")
            .expect("fixture link");
        drop(page);
        let mut file = std::fs::File::create(path).expect("create fixture file");
        document.save_to_writer(&mut file).expect("write fixture");
    }

    fn kinds_at(pdfium: &Pdfium, path: &Path) -> Vec<PdfPageAnnotationType> {
        let doc = pdfium.load_pdf_from_file(path, None).expect("reopen");
        let page = doc.pages().get(0).expect("page");
        let kinds = page
            .annotations()
            .iter()
            .map(|a| a.annotation_type())
            .collect();
        kinds
    }

    #[test]
    fn annotations_speedyf_does_not_model_survive_a_save_untouched() {
        // A link is not something this editor can represent, so it must come
        // through a save exactly as it went in — never adopted, never dropped.
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_annotated_fixture(&pdfium, &source);

        let plan = EditPlan {
            pages: vec![plan_page(Some(0), 90)],
            form: Vec::new(),
        };
        build_output(
            &pdfium,
            source.to_str().expect("path"),
            None,
            &plan,
            &output,
        )
        .expect("save");

        let kinds = kinds_at(&pdfium, &output);
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == PdfPageAnnotationType::Link)
                .count(),
            1,
            "the link must still be there"
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == PdfPageAnnotationType::Highlight)
                .count(),
            2,
            "untouched highlights must not be disturbed"
        );
    }

    #[test]
    fn only_the_annotations_named_for_replacement_are_dropped() {
        // Editing one highlight must not rewrite the one beside it: a
        // third-party annotation carries an author, dates and an appearance
        // stream this model has nowhere to put, so an untouched one is left
        // exactly as imported.
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_annotated_fixture(&pdfium, &source);

        let mut page = plan_page(Some(0), 0);
        // The user moved highlight 0 and left highlight 1 alone.
        page.drop_src_annots = vec![0];
        page.annots.push(PlanAnnot {
            kind: "highlight".into(),
            rect: RectDto {
                x: 200.0,
                y: 200.0,
                w: 60.0,
                h: 20.0,
            },
            color: "#00e676".into(),
            opacity: 0.5,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
        });
        let plan = EditPlan {
            pages: vec![page],
            form: Vec::new(),
        };
        build_output(
            &pdfium,
            source.to_str().expect("path"),
            None,
            &plan,
            &output,
        )
        .expect("save");

        let kinds = kinds_at(&pdfium, &output);
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == PdfPageAnnotationType::Highlight)
                .count(),
            2,
            "one dropped, one kept, one added"
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == PdfPageAnnotationType::Link)
                .count(),
            1,
            "the link is still not ours to touch"
        );

        // The moved copy is the one that came back, at its new home.
        let read =
            crate::engine::annots::read_annotations(&pdfium, output.to_str().expect("path"), None)
                .expect("read back");
        let annots = &read[0].annots;
        assert!(
            annots.iter().any(|a| (a.rect.x - 200.0).abs() < 1.0),
            "the edited highlight must be at its new position"
        );
    }

    #[test]
    fn injected_verify_failure_preserves_destination_and_cleans_temp_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let dest = dir.path().join("document.pdf");
        std::fs::write(&dest, b"original bytes").expect("seed destination");

        let result = verified_atomic_replace(
            &dest,
            |temp| {
                std::fs::write(temp, b"replacement bytes")?;
                Ok(())
            },
            |_temp| Err(AppError::Internal("injected verify failure".into())),
            || Ok(()),
        );

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&dest).expect("read destination"),
            b"original bytes"
        );
        let siblings: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read temp dir")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect();
        assert_eq!(siblings, vec![dest.file_name().unwrap()]);
    }
}

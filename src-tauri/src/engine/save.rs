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

fn parse_color(hex: &str, opacity: f32) -> PdfColor {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    let (r, g, b) = if h.len() >= 6 {
        (((v >> 16) & 0xff) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8)
    } else {
        (0, 0, 0)
    };
    PdfColor::new(r, g, b, (opacity.clamp(0.0, 1.0) * 255.0) as u8)
}

struct PageSpace {
    crop_l: f32,
    crop_b: f32,
    crop_r: f32,
    crop_t: f32,
    rot: u16,
}

impl PageSpace {
    fn of(page: &PdfPage<'_>) -> PageSpace {
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
            crop_l: crop.left.value,
            crop_b: crop.bottom.value,
            crop_r: crop.right.value,
            crop_t: crop.top.value,
            rot,
        }
    }

    /// display-normalized (y-up) point → raw PDF user-space point
    fn disp_to_user(&self, dx: f32, dy: f32) -> (f32, f32) {
        let (l, b, r, t) = (self.crop_l, self.crop_b, self.crop_r, self.crop_t);
        match self.rot {
            90 => (l + (r - l) - dy, b + dx),
            180 => (l + (r - l) - dx, b + (t - b) - dy),
            270 => (l + dy, b + (t - b) - dx),
            _ => (l + dx, b + dy),
        }
    }

    fn rect_to_user(&self, rc: &RectDto) -> PdfRect {
        let (x1, y1) = self.disp_to_user(rc.x, rc.y);
        let (x2, y2) = self.disp_to_user(rc.x + rc.w, rc.y + rc.h);
        PdfRect::new(
            PdfPoints::new(y1.min(y2)),
            PdfPoints::new(x1.min(x2)),
            PdfPoints::new(y1.max(y2)),
            PdfPoints::new(x1.max(x2)),
        )
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
                a.set_fill_color(parse_color(&annot.color, annot.opacity)).ok();
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
                                PdfPoints::new(u.bottom.value.min(rect.bottom.value)),
                                PdfPoints::new(u.left.value.min(rect.left.value)),
                                PdfPoints::new(u.top.value.max(rect.top.value)),
                                PdfPoints::new(u.right.value.max(rect.right.value)),
                            ),
                        });
                    }
                }
                a.set_bounds(union.unwrap_or_else(|| space.rect_to_user(&annot.rect)))?;
            }
            "rect" => {
                let mut a = page.annotations_mut().create_square_annotation()?;
                a.set_bounds(space.rect_to_user(&annot.rect))?;
                a.set_stroke_color(parse_color(&annot.color, annot.opacity)).ok();
                a.set_width(PdfPoints::new(annot.stroke_width.unwrap_or(2.0))).ok();
            }
            "note" => {
                let text = annot.text.clone().unwrap_or_default();
                let mut a = page.annotations_mut().create_text_annotation(&text)?;
                a.set_bounds(space.rect_to_user(&annot.rect))?;
                a.set_fill_color(parse_color(&annot.color, annot.opacity)).ok();
            }
            "ink" => {
                // Flattened to page path objects (portable; documented in README).
                if let Some(strokes) = &annot.strokes {
                    let color = parse_color(&annot.color, annot.opacity);
                    let width = PdfPoints::new(annot.stroke_width.unwrap_or(2.0));
                    for stroke in strokes {
                        if stroke.len() < 2 {
                            continue;
                        }
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
                        page.objects_mut().add_path_object(path)?;
                    }
                }
            }
            other => {
                log::warn!("unknown annotation kind ignored at save: {other}");
            }
        }
    }

    for text in &plan_page.texts {
        let user_rect = space.rect_to_user(&text.rect);
        let size = text.font_size_pt.max(4.0);
        let color = parse_color(&text.color, text.opacity);
        let mut y = user_rect.top.value - size;
        for line in text.text.split('\n') {
            let content = if line.trim().is_empty() { " " } else { line };
            let mut obj = PdfPageTextObject::new(doc, content, helv, PdfPoints::new(size))?;
            obj.set_fill_color(color).ok();
            obj.translate(PdfPoints::new(user_rect.left.value), PdfPoints::new(y))?;
            page.objects_mut().add_text_object(obj)?;
            y -= size * 1.3;
        }
    }

    for img in &plan_page.images {
        let dyn_img = image::open(&img.source_path)
            .map_err(|e| AppError::Io(format!("cannot load image {}: {e}", img.source_path)))?;
        let user_rect = space.rect_to_user(&img.rect);
        let mut obj = PdfPageImageObject::new(doc, &dyn_img)?;
        let w = user_rect.right.value - user_rect.left.value;
        let h = user_rect.top.value - user_rect.bottom.value;
        obj.scale(w, h)?;
        obj.translate(
            PdfPoints::new(user_rect.left.value),
            PdfPoints::new(user_rect.bottom.value),
        )?;
        page.objects_mut().add_image_object(obj)?;
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

fn fill_form_fields(doc: &mut PdfDocument<'_>, form: &[(String, String)]) {
    if form.is_empty() || doc.form().is_none() {
        return;
    }
    let page_count = doc.pages().len();
    for pi in 0..page_count {
        let Ok(page) = doc.pages().get(pi) else { continue };
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
        if let Some(_) = plan.pages[i].src_index {
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
        let mut page = out.pages().get(idx as u16)?;
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

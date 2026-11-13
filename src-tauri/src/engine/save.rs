//! EditPlan materialization + safe atomic save.
//!
//! Uses the high-level pdfium-render wrapper (editing happens once per save,
//! so wrapper ergonomics beat raw FFI here):
//!   new doc → import kept/reordered/duplicated pages (FPDF_ImportPages under
//!   the hood) → blank pages → true PDF annotations (Highlight/Square/Text
//!   note/ink/stamp) with explicit text/image appearances → AcroForm edits →
//!   write to a temp sibling → reopen-verify → atomic rename.
//!
//! Annotation coordinates arrive in display-normalized page space (y-up,
//! origin at the displayed crop-box bottom-left); `disp_to_user` maps them
//! back into raw PDF user space honoring crop box + /Rotate.

use crate::engine::forms;
use crate::engine::types::{EditPlan, RectDto, TextFontFamily};
use crate::errors::{AppError, AppResult};
use pdfium_render::prelude::*;
use std::collections::HashMap;
use std::path::Path;

type TextFonts = HashMap<(TextFontFamily, bool, bool), PdfFontToken>;

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
    fonts: &TextFonts,
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
            // A shaded bubble: a Stamp carrying one filled circle object.
            //
            // Stamp rather than Square, because the control being filled is
            // round and a square annotation can only ever draw a box. Stamp
            // rather than the PDF's own /Circle subtype, because PDFium
            // exposes no constructor for one — and because a Stamp carries
            // its appearance in its own objects, which is what keeps the mark
            // looking identical in every viewer instead of depending on one
            // to synthesize it.
            //
            // The colour lives on the path object, never on the annotation.
            // `FPDFAnnot_SetColor` faults on an annotation that has an /AP,
            // exactly as the ink branch above documents at length.
            "bubble" => {
                let user_rect = space.rect_to_user(&annot.rect);
                let cx = (user_rect.left().value + user_rect.right().value) / 2.0;
                let cy = (user_rect.bottom().value + user_rect.top().value) / 2.0;
                let w = user_rect.right().value - user_rect.left().value;
                let h = user_rect.top().value - user_rect.bottom().value;
                let radius = w.min(h) / 2.0;
                if radius <= 0.0 {
                    continue;
                }
                let color = parse_color(&annot.color, annot.opacity);
                let path = PdfPagePathObject::new_circle_at(
                    doc,
                    PdfPoints::new(cx),
                    PdfPoints::new(cy),
                    PdfPoints::new(radius),
                    None,
                    None,
                    Some(color),
                )?;
                let mut a = page.annotations_mut().create_stamp_annotation()?;
                // Bounds BEFORE the object, not after. PDFium builds the
                // appearance stream when an object is appended, and sizes its
                // /BBox from the annotation's rect *at that moment* — so
                // appending first bakes in a zero-sized box that clips the
                // whole mark away. The annotation still holds a correct path
                // and a correct /Rect, which is why this reads as working
                // right up until someone looks at the page.
                a.set_bounds(user_rect)?;
                a.objects_mut().add_path_object(path)?;
            }
            other => {
                log::warn!("unknown annotation kind ignored at save: {other}");
            }
        }
    }

    // PDFium only supports appending appearance objects to Ink and Stamp,
    // not FreeText. A bare FreeText with /Contents has no portable appearance
    // (and /IC is not its text color). Text stamps carry their own font and
    // drawing commands while remaining editable through our annotation reader.
    for text in &plan_page.texts {
        let font = fonts[&(text.style.font_family, text.style.bold, text.style.italic)];
        let user_rect = space.rect_to_user(&text.rect);
        let size = text.font_size_pt.max(4.0);
        let color = parse_color(&text.color, text.opacity);
        let mut a = page.annotations_mut().create_stamp_annotation()?;
        a.set_bounds(user_rect)?;
        a.set_contents(&text.text)?;
        a.set_creator(crate::engine::annots::TEXT_STAMP_CREATOR)?;
        a.set_is_printed(true)?;
        let mut y = text.rect.y + text.rect.h - size;
        for line in wrapped_text_lines(doc, font, size, &text.text, text.rect.w)? {
            if y + size < text.rect.y {
                break;
            }
            if !line.trim().is_empty() {
                let mut object =
                    PdfPageTextObject::new(doc, line.trim_end(), font, PdfPoints::new(size))?;
                object.set_fill_color(color)?;
                let line_width = object.width()?.value;
                // Map the baseline and both axes, not just the rectangle:
                // text added to an already-rotated page must stay upright.
                let (x0, y0) = space.disp_to_user(text.rect.x, y);
                let (xx, xy) = space.disp_to_user(text.rect.x + 1.0, y);
                let (yx, yy) = space.disp_to_user(text.rect.x, y + 1.0);
                object.transform(xx - x0, xy - y0, yx - x0, yy - y0, x0, y0)?;
                a.objects_mut().add_text_object(object)?;
                if text.style.underline {
                    let (x1, y1) = space.disp_to_user(text.rect.x, y - size * 0.13);
                    let (x2, y2) = space.disp_to_user(text.rect.x + line_width, y - size * 0.13);
                    let line = PdfPagePathObject::new_line(
                        doc,
                        PdfPoints::new(x1),
                        PdfPoints::new(y1),
                        PdfPoints::new(x2),
                        PdfPoints::new(y2),
                        color,
                        PdfPoints::new((size / 14.0).max(0.5)),
                    )?;
                    a.objects_mut().add_path_object(line)?;
                }
            }
            y -= size * 1.3;
        }
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
        // Bounds first — see the note on the "bubble" branch. Appending to a
        // still-empty annotation gives its appearance stream a zero /BBox,
        // which clips the image away completely while leaving a correct-looking
        // /Rect behind.
        a.set_bounds(user_rect)?;
        a.objects_mut().add_image_object(obj)?;
    }

    Ok(())
}

/// Preserve explicit line breaks and wrap at whitespace, like the text overlay.
/// Measure using the same PDF font used to draw the saved text.
fn wrapped_text_lines(
    doc: &PdfDocument<'_>,
    font: PdfFontToken,
    size: f32,
    text: &str,
    width: f32,
) -> AppResult<Vec<String>> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = Vec::new();
    let mut measure = PdfPageTextObject::new(doc, "", font, PdfPoints::new(size))?;
    for paragraph in normalized.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_inclusive(char::is_whitespace) {
            let candidate = format!("{line}{word}");
            measure.set_text(candidate.trim_end())?;
            if !line.is_empty() && measure.width()?.value > width {
                lines.push(std::mem::take(&mut line));
            }
            // A compact box must also wrap an unbroken word, as the overlay
            // does, rather than silently clipping its ending in the PDF.
            for c in word.chars() {
                measure.set_text(format!("{line}{c}"))?;
                if !line.is_empty() && !c.is_whitespace() && measure.width()?.value > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    Ok(lines)
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

/// Whether `plan` hands back the source's pages exactly as it found them:
/// same count, same order, nothing inserted, nothing dropped.
///
/// Per-page edits — rotation, annotations, form values — do not count as
/// restructuring; only the shape of the page list does.
fn preserves_source_pages(plan: &EditPlan, src_page_count: u16) -> bool {
    plan.pages.len() == src_page_count as usize
        && plan
            .pages
            .iter()
            .enumerate()
            .all(|(index, page)| page.src_index == Some(index as u32))
}

/// Build the output document from `plan` and write it to `out_path`.
///
/// Two ways to produce the output, and which one is used comes down to
/// whether the document is a form.
///
/// The default is to assemble a fresh document and import the pages the plan
/// asks for, which is what makes reordering, deletion and inserted blanks
/// possible at all. It also loses anything that lives on the *document*
/// rather than on a page — and an AcroForm is exactly that. `FPDF_ImportPages`
/// copies each widget annotation faithfully and leaves the `/AcroForm`
/// dictionary that gives them meaning behind, so a filled-in form came out the
/// other side as a page of inert boxes: the fields unreachable, the radio
/// groups' parents orphaned, and filling text fields silently doing
/// nothing because the output no longer had a form to fill.
///
/// So when the document has a form and the plan is not restructuring pages —
/// open a form, tick boxes, save, which is what filling one in *is* — the
/// source document is edited in place and saved instead. Every later step
/// works the same either way; they only care about pages.
///
/// Restructuring a form's pages still goes through the import path and still
/// loses the form. That is a real limitation rather than an oversight: pdfium
/// exposes no way to carry a catalog entry across documents, and dropping
/// pages out of a form raises questions about the fields on them that this
/// has no answer for.
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
    let has_form = src.form().is_some();
    let edit_in_place = has_form && preserves_source_pages(plan, src.pages().len());
    if has_form && !edit_in_place {
        log::warn!(
            "{src_path} has an AcroForm, but the save reorders, adds or removes \
             pages; the form cannot be carried across and will be dropped"
        );
    }

    let mut out = if edit_in_place {
        src
    } else {
        let mut fresh = pdfium.create_new_pdf()?;

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
                fresh
                    .pages_mut()
                    .copy_pages_from_document(&src, &list, dest)
                    .map_err(|e| AppError::Internal(format!("page import failed: {e:?}")))?;
                dest += (j - i) as u16;
                i = j;
            } else {
                let pp = &plan.pages[i];
                fresh
                    .pages_mut()
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
        fresh
    };
    // Load only font variants used by this edit plan, before borrowing pages.
    let mut fonts = TextFonts::new();
    for text in plan.pages.iter().flat_map(|p| &p.texts) {
        fonts
            .entry((text.style.font_family, text.style.bold, text.style.italic))
            .or_insert_with(|| {
                let f = out.fonts_mut();
                match (text.style.font_family, text.style.bold, text.style.italic) {
                    (TextFontFamily::Helvetica, false, false) => f.helvetica(),
                    (TextFontFamily::Helvetica, true, false) => f.helvetica_bold(),
                    (TextFontFamily::Helvetica, false, true) => f.helvetica_oblique(),
                    (TextFontFamily::Helvetica, true, true) => f.helvetica_bold_oblique(),
                    (TextFontFamily::TimesRoman, false, false) => f.times_roman(),
                    (TextFontFamily::TimesRoman, true, false) => f.times_bold(),
                    (TextFontFamily::TimesRoman, false, true) => f.times_italic(),
                    (TextFontFamily::TimesRoman, true, true) => f.times_bold_italic(),
                    (TextFontFamily::Courier, false, false) => f.courier(),
                    (TextFontFamily::Courier, true, false) => f.courier_bold(),
                    (TextFontFamily::Courier, false, true) => f.courier_oblique(),
                    (TextFontFamily::Courier, true, true) => f.courier_bold_oblique(),
                }
            });
    }

    // 2) per-page: annotations/texts/images in ORIGINAL display space, then rotation delta.
    for (idx, pp) in plan.pages.iter().enumerate() {
        if !page_requires_mutation(pp) {
            continue;
        }
        let mut page = out.pages().get(idx as u16)?;
        drop_replaced_annotations(&mut page, &pp.drop_src_annots);
        add_annotations(&out, &mut page, pp, &fonts)?;
        apply_rotation(&mut page, pp.rotation % 360);
    }

    // 3) write
    let mut file = std::fs::File::create(out_path)?;
    out.save_to_writer(&mut file)
        .map_err(|e| AppError::Io(format!("write failed: {e:?}")))?;
    use std::io::Write;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    // 4) Commit form values AND their appearances through PDFium's form-fill
    //    environment; setting /V alone leaves stale or missing text on disk.
    forms::apply_edits(pdfium.bindings(), out_path, &plan.form, &plan.form_buttons)?;
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
            style: Default::default(),
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
            form_buttons: Vec::new(),
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
            style: Default::default(),
            color: "#111111".into(),
            opacity: 1.0,
        });
        let plan = EditPlan {
            pages: vec![page],
            form: Vec::new(),
            form_buttons: Vec::new(),
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

    /// Count pixels dark enough to be ink inside a display-space rect.
    ///
    /// The page is rendered at one pixel per point, so a display rect (y-up,
    /// origin bottom-left) maps to pixels by flipping y against page height.
    fn ink_pixels_in(page: &PdfPage<'_>, rect: RectDto, page_h: f32) -> usize {
        let w = page.width().value.round() as u16;
        let h = page.height().value.round() as u16;
        let bitmap = page.render(w as i32, h as i32, None).expect("render page");
        let rgba = bitmap.as_rgba_bytes();
        let x0 = rect.x.floor().max(0.0) as usize;
        let x1 = ((rect.x + rect.w).ceil() as usize).min(w as usize);
        let y0 = ((page_h - rect.y - rect.h).floor().max(0.0)) as usize;
        let y1 = (((page_h - rect.y).ceil()) as usize).min(h as usize);
        let mut dark = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * w as usize + x) * 4;
                if let Some(px) = rgba.get(i..i + 3) {
                    if px[0] < 128 && px[1] < 128 && px[2] < 128 {
                        dark += 1;
                    }
                }
            }
        }
        dark
    }

    #[test]
    fn text_formatting_uses_the_selected_pdf_font_and_underlines() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("formatted.pdf");
        create_source_fixture(&pdfium, &source);
        let mut cases = Vec::new();
        let mut pages = Vec::new();
        for (family, names) in [
            (
                "Helvetica",
                [
                    "Helvetica",
                    "Helvetica-Bold",
                    "Helvetica-Oblique",
                    "Helvetica-BoldOblique",
                ],
            ),
            (
                "Times-Roman",
                [
                    "Times-Roman",
                    "Times-Bold",
                    "Times-Italic",
                    "Times-BoldItalic",
                ],
            ),
            (
                "Courier",
                [
                    "Courier",
                    "Courier-Bold",
                    "Courier-Oblique",
                    "Courier-BoldOblique",
                ],
            ),
        ] {
            for (variant, name) in names.iter().enumerate() {
                let bold = variant % 2 == 1;
                let italic = variant >= 2;
                let underline = variant != 0;
                cases.push((family, *name, bold, italic, underline));
                let mut page = plan_page(None, 0);
                page.texts.push(
                    serde_json::from_value(serde_json::json!({
                        "rect": {"x": 30, "y": 200, "w": 220, "h": 50},
                        "text": "Styled text", "fontSizePt": 20, "color": "#2255aa", "opacity": 1,
                        "fontFamily": family, "bold": bold, "italic": italic, "underline": underline
                    }))
                    .unwrap(),
                );
                pages.push(page);
            }
        }
        let plan = EditPlan {
            pages,
            form: vec![],
            form_buttons: vec![],
        };
        build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output).unwrap();
        let doc = pdfium.load_pdf_from_file(&output, None).unwrap();
        let read = crate::engine::annots::read_annotations(&pdfium, output.to_str().unwrap(), None)
            .unwrap();
        for (index, (family, name, bold, italic, underline)) in cases.iter().enumerate() {
            let page = doc.pages().get(index as u16).unwrap();
            let annot = page.annotations().get(0).unwrap();
            let object = annot.objects().get(0).unwrap();
            assert_eq!(object.as_text_object().unwrap().font().name(), *name);
            assert_eq!(
                annot.objects().iter().any(|o| o.as_path_object().is_some()),
                *underline
            );
            let saved = serde_json::to_value(&read[index].annots[0]).unwrap();
            assert_eq!(saved["fontFamily"], *family);
            assert_eq!(saved["bold"], *bold);
            assert_eq!(saved["italic"], *italic);
            assert_eq!(saved["underline"], *underline);
            assert_eq!(saved["color"], "#2255aa");
        }
    }

    #[test]
    fn added_text_has_a_visible_saved_appearance() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_source_fixture(&pdfium, &source);
        let rect = RectDto {
            x: 36.0,
            y: 200.0,
            w: 200.0,
            h: 80.0,
        };
        let mut page = plan_page(None, 0);
        page.width_pt = 300.0;
        page.height_pt = 420.0;
        page.texts.push(PlanText {
            rect,
            text: "Added text\nSecond line".into(),
            font_size_pt: 20.0,
            style: Default::default(),
            color: "#111111".into(),
            opacity: 1.0,
        });
        build_output(
            &pdfium,
            source.to_str().unwrap(),
            None,
            &EditPlan {
                pages: vec![page],
                form: Vec::new(),
                form_buttons: Vec::new(),
            },
            &output,
        )
        .expect("save added text");

        let doc = pdfium.load_pdf_from_file(&output, None).expect("reopen");
        let page = doc.pages().get(0).expect("page");
        let annot = page.annotations().get(0).expect("text annotation");
        assert!(annot.is_printed(), "added text must also print");
        assert_eq!(
            annot.objects().iter().filter(|o| o.as_text_object().is_some()).count(),
            2,
            "both lines must already have text objects before rendering can synthesize an appearance"
        );
        assert!(
            ink_pixels_in(&page, rect, 420.0) > 100,
            "saved text must draw on the page"
        );
        let read = crate::engine::annots::read_annotations(&pdfium, output.to_str().unwrap(), None)
            .expect("read added text");
        let text = &read[0].annots[0];
        assert_eq!(text.kind, "textbox");
        assert_eq!(text.text.as_deref(), Some("Added text\nSecond line"));
        assert_eq!(text.font_size_pt, Some(20.0));
        assert_eq!(text.color, "#111111");
    }

    #[test]
    fn added_text_stays_upright_on_cropped_rotated_pages() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        let mut document = pdfium.create_new_pdf().expect("new document");
        for rotation in [0, 90, 180, 270] {
            let mut page = document
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::Custom(
                    PdfPoints::new(340.0),
                    PdfPoints::new(480.0),
                ))
                .unwrap();
            page.boundaries_mut()
                .set_crop(PdfRect::new(
                    PdfPoints::new(30.0),
                    PdfPoints::new(20.0),
                    PdfPoints::new(450.0),
                    PdfPoints::new(320.0),
                ))
                .unwrap();
            apply_rotation(&mut page, rotation);
        }
        document.save_to_file(&source).unwrap();
        drop(document);
        let rect = RectDto {
            x: 36.0,
            y: 80.0,
            w: 160.0,
            h: 80.0,
        };
        let plan = EditPlan {
            pages: (0..4)
                .map(|index| {
                    let mut page = plan_page(Some(index), 0);
                    page.texts.push(PlanText {
                        rect,
                        text: "First line\nSecond line".into(),
                        font_size_pt: 20.0,
                        style: crate::engine::types::TextStyle {
                            font_family: TextFontFamily::TimesRoman,
                            bold: true,
                            italic: true,
                            underline: true,
                        },
                        color: "#111111".into(),
                        opacity: 0.8,
                    });
                    page
                })
                .collect(),
            form: Vec::new(),
            form_buttons: Vec::new(),
        };
        build_output(&pdfium, source.to_str().unwrap(), None, &plan, &output).unwrap();
        let doc = pdfium.load_pdf_from_file(&output, None).unwrap();
        for index in 0..4 {
            let page = doc.pages().get(index).unwrap();
            let height = page.height().value;
            // Each line must draw in its own horizontal band in display space.
            for y in [rect.y + rect.h - 20.0, rect.y + rect.h - 46.0] {
                assert!(
                    ink_pixels_in(
                        &page,
                        RectDto {
                            x: rect.x,
                            y,
                            w: rect.w,
                            h: 20.0
                        },
                        height
                    ) > 80,
                    "text missing or sideways on page {index}"
                );
            }
        }
        let read = crate::engine::annots::read_annotations(&pdfium, output.to_str().unwrap(), None)
            .unwrap();
        for page in read {
            let text = &page.annots[0];
            assert!((text.rect.x - rect.x).abs() < 0.01);
            assert!((text.rect.y - rect.y).abs() < 0.01);
            assert_eq!(text.font_size_pt, Some(20.0));
            assert!((text.opacity - 0.8).abs() < 0.01);
            assert_eq!(text.text_style.font_family, TextFontFamily::TimesRoman);
            assert!(text.text_style.bold && text.text_style.italic && text.text_style.underline);
        }
    }

    /// Everything a Stamp annotation draws lives in an appearance stream with
    /// its own /BBox, and a zero one clips the whole mark away while leaving
    /// the annotation, its path objects and its /Rect all reading as correct.
    /// So counting annotations proves nothing here, and neither does checking
    /// bounds — both were right the entire time a shaded bubble and an
    /// inserted image were writing stamps that no viewer would draw. The only
    /// assertion that catches it is whether pixels actually land on the page.
    #[test]
    fn stamp_appearance_streams_actually_put_ink_on_the_page() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_source_fixture(&pdfium, &source);

        // The fixture's first page, so imported geometry matches the plan.
        let (page_w, page_h) = (300.0f32, 420.0f32);
        let shaded = RectDto {
            x: 40.0,
            y: 60.0,
            w: 14.0,
            h: 14.0,
        };
        let mut page = plan_page(Some(0), 0);
        page.width_pt = page_w;
        page.height_pt = page_h;
        page.annots.push(PlanAnnot {
            kind: "bubble".into(),
            rect: shaded,
            color: "#000000".into(),
            opacity: 1.0,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
        });
        build_output(
            &pdfium,
            source.to_str().expect("source path"),
            None,
            &EditPlan {
                pages: vec![page],
                form: Vec::new(),
                form_buttons: Vec::new(),
            },
            &output,
        )
        .expect("build output");

        let doc = pdfium.load_pdf_from_file(&output, None).expect("reopen");
        let page = doc.pages().get(0).expect("page");
        let inked = ink_pixels_in(&page, shaded, page_h);
        // A filled circle inscribed in a 14pt box covers ~pi/4 of it; well
        // over half of that is a floor no anti-aliasing can drop below.
        assert!(
            inked > 60,
            "the shaded bubble drew {inked} dark pixels — the appearance \
             stream is being clipped away"
        );

        // A patch of the same page the plan never touched must stay blank,
        // so the count above cannot be the fixture's own content.
        let untouched = RectDto {
            x: 200.0,
            y: 60.0,
            w: 14.0,
            h: 14.0,
        };
        assert_eq!(
            ink_pixels_in(&page, untouched, page_h),
            0,
            "control patch should be blank"
        );
    }

    /// A shaded bubble is written as a Stamp, and so is a placed image — so
    /// reading one back has to tell them apart. It did not: every Stamp came
    /// back as an `image`, and an image read from a file carries no source
    /// path, so the save path drops it and writes nothing in its place.
    /// Shading a quiz, saving, reopening and nudging one mark would have
    /// erased it.
    #[test]
    fn a_shaded_bubble_reopens_as_a_bubble_not_an_image() {
        let _guard = pdfium_init::test_guard();
        let pdfium = test_pdfium();
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("source.pdf");
        let output = dir.path().join("output.pdf");
        create_source_fixture(&pdfium, &source);

        let mut page = plan_page(Some(0), 0);
        page.width_pt = 300.0;
        page.height_pt = 420.0;
        page.annots.push(PlanAnnot {
            kind: "bubble".into(),
            rect: RectDto {
                x: 40.0,
                y: 60.0,
                w: 14.0,
                h: 14.0,
            },
            color: "#1a1a1a".into(),
            opacity: 1.0,
            stroke_width: None,
            quads: None,
            strokes: None,
            text: None,
        });
        build_output(
            &pdfium,
            source.to_str().expect("source path"),
            None,
            &EditPlan {
                pages: vec![page],
                form: Vec::new(),
                form_buttons: Vec::new(),
            },
            &output,
        )
        .expect("build output");

        let pages = crate::engine::annots::read_annotations(
            &pdfium,
            output.to_str().expect("output path"),
            None,
        )
        .expect("read annotations");
        let kinds: Vec<&str> = pages
            .iter()
            .flat_map(|p| p.annots.iter())
            .map(|a| a.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["bubble"], "the shading came back as {kinds:?}");
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
            form_buttons: Vec::new(),
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
            form_buttons: Vec::new(),
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

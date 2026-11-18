//! Redaction on real PDFs, verified with three independent text extractors.
//!
//! `SPEEDYF_REDACT_EVAL_DIR=<dir with PDFs> cargo test --lib redact::eval --
//! --ignored --nocapture`. For each file: words that occur exactly once in the
//! document (up to three per page on its first text pages) and the middle of
//! the first image are marked, the marks applied, and the clean rewrite
//! written next to the inputs as `<name>.redacted.pdf`. Then:
//! - PDFium, pdftotext and pypdf must not find any marked word;
//! - the decompressed bytes are grepped for the words;
//! - other words on the marked pages must survive (collateral loss);
//! - the rendered pixel at the centre of the image mark must be the fill;
//! - the output must hold one revision and the same page count.

use super::*;
use crate::engine::pdfium_init;
use pdfium_render::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn external_text(tool: &str, path: &Path) -> Option<String> {
    let out = match tool {
        "pdftotext" => Command::new("pdftotext").arg(path).arg("-").output().ok()?,
        _ => Command::new("python3")
            .arg("-I")
            .arg("-c")
            .arg("import sys,pypdf; r=pypdf.PdfReader(sys.argv[1]); print('\\n'.join((p.extract_text() or '') for p in r.pages))")
            .arg(path)
            .output()
            .ok()?,
    };
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn decompressed(bytes: &[u8]) -> Vec<u8> {
    let Ok(mut doc) = Document::load_mem(bytes) else {
        return Vec::new();
    };
    doc.decompress();
    let mut out = Vec::new();
    let _ = doc.save_to(&mut out);
    out
}

fn count(hay: &[u8], needle: &str) -> usize {
    hay.windows(needle.len())
        .filter(|w| *w == needle.as_bytes())
        .count()
}

struct Target {
    page: usize,
    word: String,
    rect: Rect,
}

#[test]
#[ignore]
fn redact_eval() {
    let Ok(dir) = std::env::var("SPEEDYF_REDACT_EVAL_DIR") else {
        eprintln!("set SPEEDYF_REDACT_EVAL_DIR");
        return;
    };
    let pdfium = Pdfium::new(pdfium_init::init_bindings(&[]).expect("PDFium"));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| e == "pdf") && !p.to_string_lossy().contains(".redacted.")
        })
        .collect();
    files.sort();
    println!(
        "| file | pages | words | PDFium | pdftotext | pypdf | raw bytes | collateral kept | image | revisions | ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let started = std::time::Instant::now();

        // Targets, from PDFium's reading of the original.
        let src = match pdfium.load_pdf_from_byte_slice(&bytes, None) {
            Ok(d) => d,
            Err(e) => {
                println!("| {name} | PDFium cannot open: {e:?} |");
                continue;
            }
        };
        let page_count = src.pages().len() as usize;
        let mut page_texts = Vec::new();
        for page in src.pages().iter() {
            page_texts.push(page.text().map(|t| t.all()).unwrap_or_default());
        }
        let mut freq: HashMap<String, usize> = HashMap::new();
        for t in &page_texts {
            for w in words_of(t) {
                *freq.entry(w).or_default() += 1;
            }
        }
        let mut targets: Vec<Target> = Vec::new();
        let mut image_mark: Option<(usize, Rect)> = None;
        for (index, page) in src.pages().iter().enumerate() {
            if targets.len() >= 6 && image_mark.is_some() {
                break;
            }
            if image_mark.is_none() {
                for object in page.objects().iter() {
                    if object.object_type() == PdfPageObjectType::Image {
                        if let Ok(b) = object.bounds() {
                            let (l, r) = (b.left().value, b.right().value);
                            let (bo, t) = (b.bottom().value, b.top().value);
                            if r - l > 40.0 && t - bo > 40.0 {
                                let (cx, cy) = ((l + r) / 2.0, (bo + t) / 2.0);
                                let (hw, hh) = ((r - l) * 0.15, (t - bo) * 0.15);
                                image_mark =
                                    Some((index, Rect::new(cx - hw, cy - hh, cx + hw, cy + hh)));
                                break;
                            }
                        }
                    }
                }
            }
            if targets.len() >= 6 {
                continue;
            }
            let Ok(text) = page.text() else { continue };
            let all = text.chars();
            let chars: Vec<_> = all.iter().collect();
            let mut on_page = 0;
            let mut i = 0;
            while i < chars.len() && on_page < 3 && targets.len() < 6 {
                let c = chars[i].unicode_char().unwrap_or(' ');
                if !c.is_alphanumeric() {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < chars.len() && chars[i].unicode_char().is_some_and(char::is_alphanumeric)
                {
                    i += 1;
                }
                let word: String = chars[start..i]
                    .iter()
                    .filter_map(|c| c.unicode_char())
                    .collect();
                if word.len() < 6
                    || freq.get(&word) != Some(&1)
                    || !word.chars().all(|c| c.is_ascii_alphabetic())
                {
                    continue;
                }
                let boxes: Vec<Rect> = chars[start..i]
                    .iter()
                    .filter_map(|c| c.loose_bounds().ok())
                    .map(|b| {
                        Rect::new(
                            b.left().value,
                            b.bottom().value,
                            b.right().value,
                            b.top().value,
                        )
                    })
                    .collect();
                if boxes.len() != i - start || boxes.iter().any(|b| b.area() <= 0.0) {
                    continue;
                }
                let rect = boxes[1..].iter().fold(boxes[0], |a, b| a.union(b));
                // Skip rotated or oddly tall words; one line only.
                if rect.height() > 40.0 || rect.width() > 300.0 {
                    continue;
                }
                targets.push(Target {
                    page: index,
                    word,
                    rect,
                });
                on_page += 1;
                i += 1;
            }
        }
        drop(src);

        // Apply.
        let mut doc = match Document::load_mem(&bytes) {
            Ok(d) => d,
            Err(e) => {
                println!("| {name} | lopdf cannot read: {e} |");
                continue;
            }
        };
        if doc.is_encrypted() {
            println!("| {name} | encrypted, skipped |");
            continue;
        }
        let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
        let add = |doc: &mut Document, page: usize, r: Rect| {
            let annot = doc.add_object(dictionary! {
                "Type" => "Annot", "Subtype" => "Redact",
                "Rect" => vec![Object::Real(r.x0), Object::Real(r.y0), Object::Real(r.x1), Object::Real(r.y1)],
            });
            let pid = pages[page];
            let existing = doc
                .get_dictionary(pid)
                .unwrap()
                .get(b"Annots")
                .ok()
                .cloned();
            let mut list = match existing {
                Some(Object::Reference(id)) => {
                    doc.get_object(id).unwrap().as_array().unwrap().clone()
                }
                Some(Object::Array(a)) => a,
                _ => Vec::new(),
            };
            list.push(Object::Reference(annot));
            doc.get_dictionary_mut(pid)
                .unwrap()
                .set("Annots", Object::Array(list));
        };
        for t in &targets {
            add(&mut doc, t.page, t.rect);
        }
        if let Some((page, r)) = image_mark {
            add(&mut doc, page, r);
        }
        let decode = |d: &Document, s: &lopdf::Stream| decode::decode_with_pdfium(&pdfium, d, s);
        let report = match apply_document_with(&mut doc, Some(&decode)) {
            Ok(r) => r,
            Err(e) => {
                println!("| {name} | apply failed: {e} |");
                continue;
            }
        };
        let out = clean_rewrite(&mut doc).unwrap();
        let out_path = path.with_extension("redacted.pdf");
        std::fs::write(&out_path, &out).unwrap();
        let ms = started.elapsed().as_millis();

        // Verify.
        let words: Vec<&str> = targets.iter().map(|t| t.word.as_str()).collect();
        let after = pdfium.load_pdf_from_byte_slice(&out, None).unwrap();
        let after_pages = after.pages().len() as usize;
        let after_texts: Vec<String> = after
            .pages()
            .iter()
            .map(|p| p.text().map(|t| t.all()).unwrap_or_default())
            .collect();
        let found_in = |text: &str| -> usize {
            let set: HashSet<String> = words_of(text).into_iter().collect();
            words.iter().filter(|w| set.contains(**w)).count()
        };
        let pdfium_left = found_in(&after_texts.join("\n"));
        let pdftotext_left = external_text("pdftotext", &out_path).map(|t| found_in(&t));
        let pypdf_left = external_text("pypdf", &out_path).map(|t| found_in(&t));
        let raw_before = decompressed(&bytes);
        let raw_after = decompressed(&out);
        let raw_before_hits: usize = words.iter().map(|w| count(&raw_before, w)).sum();
        let raw_after_hits: usize = words.iter().map(|w| count(&raw_after, w)).sum();

        // Collateral: other words (4+ letters) of the marked pages kept.
        let marked: HashSet<usize> = targets.iter().map(|t| t.page).collect();
        let (mut kept, mut total) = (0usize, 0usize);
        for &p in &marked {
            let before: HashSet<String> = words_of(&page_texts[p])
                .into_iter()
                .filter(|w| w.len() >= 4 && !words.contains(&w.as_str()))
                .collect();
            let after_set: HashSet<String> = words_of(&after_texts[p]).into_iter().collect();
            total += before.len();
            kept += before.iter().filter(|w| after_set.contains(*w)).count();
        }

        let image = match image_mark {
            None => "none".to_string(),
            Some((page, r)) => {
                let p = after.pages().get(page as u16).unwrap();
                let w = p.width().value;
                let h = p.height().value;
                let bitmap = p
                    .render_with_config(&PdfRenderConfig::new().set_target_width(w as i32))
                    .unwrap();
                let img = bitmap.as_image().to_rgb8();
                // Centre of the mark in device pixels (no rotation assumed
                // for this probe; a rotated page reports n/a).
                let rot = p.rotation().map(|r| r.as_degrees()).unwrap_or(0.0);
                let crop = p
                    .boundaries()
                    .crop()
                    .or_else(|_| p.boundaries().media())
                    .map(|b| b.bounds);
                if let (0.0, Ok(c)) = (rot, crop) {
                    let x = ((r.x0 + r.x1) / 2.0 - c.left().value) / w * img.width() as f32;
                    let y = (c.top().value - (r.y0 + r.y1) / 2.0) / h * img.height() as f32;
                    let px = img.get_pixel(
                        (x as u32).min(img.width() - 1),
                        (y as u32).min(img.height() - 1),
                    );
                    format!(
                        "pixel {:?}; blanked {} removed {}",
                        px.0, report.images_blanked, report.images_removed
                    )
                } else {
                    format!(
                        "n/a (rot {rot}); blanked {} removed {}",
                        report.images_blanked, report.images_removed
                    )
                }
            }
        };
        let fmt = |v: Option<usize>| v.map_or("n/a".to_string(), |n| format!("{n} left"));
        println!(
            "| {name} | {page_count}->{after_pages} | {} | {pdfium_left} left | {} | {} | {raw_before_hits}->{raw_after_hits} | {kept}/{total} | {image} | {} | {ms} |",
            words.len(),
            fmt(pdftotext_left),
            fmt(pypdf_left),
            crate::engine::sanitize::revision_count(&out),
        );
        if !report.warnings.is_empty() {
            println!("|  | warnings: {:?} |", report.warnings);
        }
        eprintln!("{name}: {words:?} {report:?}");
    }
}

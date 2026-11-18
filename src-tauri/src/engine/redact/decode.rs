//! Decoding an image lopdf cannot (CCITT fax, JBIG2, JPEG 2000, CMYK JPEG)
//! through PDFium, so a redaction can blank its pixels instead of dropping
//! the whole image.
//!
//! The image stream is copied, with whatever it references (an ICC profile, a
//! palette, JBIG2 globals), into a one-page PDF that draws only it; PDFium
//! opens that and hands back the image's bitmap at its own resolution.

use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use pdfium_render::prelude::*;
use std::collections::HashMap;

/// Copy `o` from `src` into `dst`, following references.
fn copy(
    src: &Document,
    dst: &mut Document,
    o: &Object,
    map: &mut HashMap<ObjectId, ObjectId>,
    depth: u32,
) -> Object {
    if depth > 16 {
        return Object::Null;
    }
    match o {
        Object::Reference(id) => {
            if let Some(new) = map.get(id) {
                return Object::Reference(*new);
            }
            let new = dst.new_object_id();
            map.insert(*id, new);
            let copied = match src.get_object(*id) {
                Ok(inner) => copy(src, dst, inner, map, depth + 1),
                Err(_) => Object::Null,
            };
            dst.objects.insert(new, copied);
            Object::Reference(new)
        }
        Object::Array(a) => Object::Array(
            a.iter()
                .map(|x| copy(src, dst, x, map, depth + 1))
                .collect(),
        ),
        Object::Dictionary(d) => Object::Dictionary(copy_dict(src, dst, d, map, depth)),
        Object::Stream(s) => {
            let dict = copy_dict(src, dst, &s.dict, map, depth);
            Object::Stream(Stream::new(dict, s.content.clone()).with_compression(false))
        }
        other => other.clone(),
    }
}

fn copy_dict(
    src: &Document,
    dst: &mut Document,
    d: &Dictionary,
    map: &mut HashMap<ObjectId, ObjectId>,
    depth: u32,
) -> Dictionary {
    let mut out = Dictionary::new();
    for (k, v) in d.iter() {
        out.set(k.clone(), copy(src, dst, v, map, depth + 1));
    }
    out
}

/// The image's samples as 8-bit gray (`1`) or RGB (`3`), decoded by PDFium.
pub fn decode_with_pdfium(
    pdfium: &Pdfium,
    doc: &Document,
    image: &Stream,
) -> Option<(Vec<u8>, usize)> {
    let w = image.dict.get(b"Width").ok()?.as_i64().ok()?;
    let h = image.dict.get(b"Height").ok()?.as_i64().ok()?;
    if w <= 0 || h <= 0 || w * h > 100_000_000 {
        return None;
    }
    let mut mini = Document::with_version("1.7");
    let mut map = HashMap::new();
    let mut dict = image.dict.clone();
    dict.remove(b"SMask");
    dict.remove(b"Mask");
    dict.remove(b"Length");
    // A stencil mask is decoded as a 1-bit gray image with the same /Decode,
    // so dark comes out exactly where the mask paints.
    if dict.get(b"ImageMask").ok().and_then(|o| o.as_bool().ok()) == Some(true) {
        dict.remove(b"ImageMask");
        dict.set("ColorSpace", Object::Name(b"DeviceGray".to_vec()));
        dict.set("BitsPerComponent", 1);
    }
    let dict = copy_dict(doc, &mut mini, &dict, &mut map, 0);
    let gray = matches!(dict.get(b"ColorSpace"), Ok(Object::Name(n)) if n == b"DeviceGray")
        || dict
            .get(b"BitsPerComponent")
            .ok()
            .and_then(|o| o.as_i64().ok())
            == Some(1);
    let img = mini.add_object(Stream::new(dict, image.content.clone()).with_compression(false));
    let content = mini.add_object(Stream::new(
        dictionary! {},
        format!("q {w} 0 0 {h} 0 0 cm /Im Do Q").into_bytes(),
    ));
    let pages = mini.new_object_id();
    let page = mini.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages),
        "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()],
        "Contents" => Object::Reference(content),
        "Resources" => dictionary! { "XObject" => dictionary! { "Im" => Object::Reference(img) } },
    });
    mini.objects.insert(
        pages,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }),
    );
    let catalog =
        mini.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(pages) });
    mini.trailer.set("Root", Object::Reference(catalog));
    let mut bytes = Vec::new();
    mini.save_to(&mut bytes).ok()?;

    let pdf = pdfium.load_pdf_from_byte_vec(bytes, None).ok()?;
    let page = pdf.pages().get(0).ok()?;
    let objects = page.objects();
    let object = objects
        .iter()
        .find(|o| o.object_type() == PdfPageObjectType::Image)?;
    let raw = object.as_image_object()?.get_raw_image().ok()?;
    if raw.width() != w as u32 || raw.height() != h as u32 {
        return None;
    }
    Some(if gray {
        (raw.into_luma8().into_raw(), 1)
    } else {
        (raw.into_rgb8().into_raw(), 3)
    })
}

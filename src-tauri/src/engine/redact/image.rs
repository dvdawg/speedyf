//! Blanking the pixels of an image a redaction mark covers.
//!
//! The image is decoded, the covered samples overwritten with the fill
//! colour, and the result re-encoded with Flate - so the original pixels are
//! gone from the file rather than hidden under a box. An image whose encoding
//! cannot be decoded here (JPEG 2000, JBIG2, CCITT) is removed from that
//! placement entirely instead: over-removal, never under.

use super::geom::{Matrix, Rect};
use lopdf::{Document, Object, Stream};

pub enum ImageOutcome {
    Unchanged,
    /// The image with the covered pixels blanked.
    Replaced(Stream),
    /// The image could not be edited; drop this placement of it.
    Remove,
}

/// Images larger than this many samples are not decoded; they are removed
/// where marked instead.
const MAX_SAMPLES: u64 = 120_000_000;

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

/// Number of colour components of a colour space, if known.
fn components(doc: &Document, cs: &Object) -> Option<usize> {
    match deref(doc, cs) {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"CalGray" | b"G" | b"Indexed" | b"I" => Some(1),
            b"DeviceRGB" | b"CalRGB" | b"RGB" | b"Lab" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            _ => None,
        },
        Object::Array(a) => {
            let family = a.first().and_then(|o| deref(doc, o).as_name().ok())?;
            match family {
                b"ICCBased" => {
                    let stream = deref(doc, a.get(1)?).as_stream().ok()?;
                    let n = stream.dict.get(b"N").ok()?.as_i64().ok()?;
                    (1..=4).contains(&n).then_some(n as usize)
                }
                b"Indexed" | b"I" | b"Separation" => Some(1),
                b"DeviceN" => deref(doc, a.get(1)?).as_array().ok().map(Vec::len),
                b"CalGray" => Some(1),
                b"CalRGB" | b"Lab" => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Whether a colour space is one whose samples are the fill colour's own
/// components (so the blanked pixels show the fill rather than a palette
/// entry or a spot ink).
fn family(doc: &Document, cs: &Object) -> &'static str {
    let name = match deref(doc, cs) {
        Object::Name(n) => n.clone(),
        Object::Array(a) => a
            .first()
            .and_then(|o| deref(doc, o).as_name().ok())
            .map(<[u8]>::to_vec)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    match name.as_slice() {
        b"DeviceGray" | b"CalGray" | b"G" => "gray",
        b"DeviceRGB" | b"CalRGB" | b"RGB" => "rgb",
        b"DeviceCMYK" | b"CMYK" => "cmyk",
        b"ICCBased" => "icc",
        _ => "other",
    }
}

/// The pixel rectangles (`x0, y0, x1, y1`, exclusive ends, row 0 at the top)
/// of a `w` x `h` image placed by `ctm` that the marks cover.
///
/// Each mark is mapped back into image space and its bounding box taken, so a
/// rotated or skewed placement blanks a slightly larger area than the mark:
/// conservative.
pub fn covered_pixels(ctm: &Matrix, w: u32, h: u32, marks: &[Rect]) -> Vec<[u32; 4]> {
    let Some(inv) = ctm.invert() else {
        return Vec::new();
    };
    let placed = ctm.map_box(0.0, 0.0, 1.0, 1.0);
    let mut out = Vec::new();
    for mark in marks {
        let Some(cover) = placed.intersection(mark) else {
            continue;
        };
        let unit = inv.map_box(cover.x0, cover.y0, cover.x1, cover.y1);
        // A pixel counts when the mark covers more than a hair of it; the
        // tolerance absorbs float noise at exact pixel edges.
        let lo = |v: f32| (v + 1e-3).floor().max(0.0) as u32;
        let hi = |v: f32| (v - 1e-3).ceil().max(0.0) as u32;
        let px0 = lo(unit.x0.clamp(0.0, 1.0) * w as f32);
        let px1 = hi(unit.x1.clamp(0.0, 1.0) * w as f32);
        // Image row 0 is the top (v = 1).
        let py0 = lo((1.0 - unit.y1.clamp(0.0, 1.0)) * h as f32);
        let py1 = hi((1.0 - unit.y0.clamp(0.0, 1.0)) * h as f32);
        if px1 > px0 && py1 > py0 {
            out.push([px0, py0, px1.min(w), py1.min(h)]);
        }
    }
    out
}

/// Overwrite the given pixel rectangles of packed sample data.
fn blank(data: &mut [u8], w: u32, n: usize, bpc: u32, rects: &[[u32; 4]], value: &[u32]) {
    let stride = (w as usize * n * bpc as usize).div_ceil(8);
    for r in rects {
        for y in r[1]..r[3] {
            let row = &mut data[y as usize * stride..(y as usize + 1) * stride];
            for x in r[0]..r[2] {
                for c in 0..n {
                    let v = value.get(c).copied().unwrap_or(0);
                    let sample = x as usize * n + c;
                    match bpc {
                        8 => row[sample] = v as u8,
                        16 => {
                            row[2 * sample] = (v >> 8) as u8;
                            row[2 * sample + 1] = v as u8;
                        }
                        1 | 2 | 4 => {
                            let bit = sample * bpc as usize;
                            let byte = bit / 8;
                            let shift = 8 - bpc as usize - bit % 8;
                            let mask = (((1u16 << bpc) - 1) as u8) << shift;
                            row[byte] = (row[byte] & !mask) | (((v as u8) << shift) & mask);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// The sample values that paint `fill` (RGB, 0..1) in a colour family.
fn fill_samples(family: &str, n: usize, bpc: u32, fill: [f32; 3]) -> Vec<u32> {
    let max = ((1u32 << bpc.min(16)) - 1) as f32;
    let q = |v: f32| (v.clamp(0.0, 1.0) * max).round() as u32;
    match (family, n) {
        ("gray" | "icc", 1) => {
            vec![q(0.299 * fill[0] + 0.587 * fill[1] + 0.114 * fill[2])]
        }
        ("rgb" | "icc", 3) => fill.iter().map(|&v| q(v)).collect(),
        ("cmyk" | "icc", 4) => {
            let k = 1.0 - fill.iter().cloned().fold(0.0, f32::max);
            let d = (1.0 - k).max(1e-6);
            vec![
                q((1.0 - fill[0] - k) / d),
                q((1.0 - fill[1] - k) / d),
                q((1.0 - fill[2] - k) / d),
                q(k),
            ]
        }
        // Palette indices, spot inks: zero. The fill box paints over it.
        _ => vec![0; n],
    }
}

/// A fallback image decoder: samples as 8-bit gray (1) or RGB (3).
pub type Decoder<'a> = &'a dyn Fn(&Document, &Stream) -> Option<(Vec<u8>, usize)>;

type Decoded = (Vec<u8>, usize, u32, &'static str);

/// Decode with what lopdf and the image crate handle: the standard filters,
/// and plain JPEGs.
fn decode_native(
    doc: &Document,
    stream: &Stream,
    filters: &[Vec<u8>],
    image_mask: bool,
    w: u32,
    h: u32,
    out_dict: &mut lopdf::Dictionary,
) -> Option<Decoded> {
    let dict = &stream.dict;
    if filters.last().map(Vec::as_slice) == Some(b"DCTDecode") {
        // CMYK JPEGs (often Adobe-inverted) and remapped samples would not
        // survive a decode to RGB with their colours intact.
        let cmyk = dict
            .get(b"ColorSpace")
            .ok()
            .and_then(|cs| components(doc, cs))
            == Some(4);
        if filters.len() != 1 || cmyk || dict.has(b"Decode") || image_mask {
            return None;
        }
        let decoded =
            image::load_from_memory_with_format(&stream.content, image::ImageFormat::Jpeg).ok()?;
        let (bytes, n, cs) = match decoded.color().channel_count() {
            1 => (decoded.into_luma8().into_raw(), 1, "DeviceGray"),
            _ => (decoded.into_rgb8().into_raw(), 3, "DeviceRGB"),
        };
        out_dict.set("ColorSpace", Object::Name(cs.as_bytes().to_vec()));
        out_dict.set("BitsPerComponent", 8);
        out_dict.remove(b"Decode");
        return Some((bytes, n, 8, if n == 1 { "gray" } else { "rgb" }));
    }
    let decodable = filters.iter().all(|f| {
        matches!(
            f.as_slice(),
            b"FlateDecode"
                | b"Fl"
                | b"LZWDecode"
                | b"LZW"
                | b"ASCII85Decode"
                | b"A85"
                | b"ASCIIHexDecode"
                | b"AHx"
                | b"RunLengthDecode"
                | b"RL"
        )
    });
    if !decodable {
        return None;
    }
    let (n, bpc, family) = if image_mask {
        (1, 1, "mask")
    } else {
        let cs = dict.get(b"ColorSpace").ok()?;
        let n = components(doc, cs)?;
        let bpc = dict
            .get(b"BitsPerComponent")
            .ok()
            .map(|o| deref(doc, o))
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(8) as u32;
        (n, bpc, family(doc, cs))
    };
    if ![1, 2, 4, 8, 16].contains(&bpc) || u64::from(w) * u64::from(h) * n as u64 > MAX_SAMPLES {
        return None;
    }
    let bytes = stream
        .decompressed_content_with_limit(MAX_SAMPLES as usize * 2 + 4096)
        .ok()?;
    Some((bytes, n, bpc, family))
}

/// Blank what `marks` cover of the image `stream` placed by `ctm`.
pub fn redact_image(
    doc: &Document,
    stream: &Stream,
    ctm: &Matrix,
    marks: &[Rect],
    fill: [f32; 3],
    decoder: Option<Decoder<'_>>,
) -> ImageOutcome {
    let dict = &stream.dict;
    let int = |k: &[u8]| {
        dict.get(k)
            .ok()
            .map(|o| deref(doc, o))
            .and_then(|o| o.as_i64().ok())
    };
    let (Some(w), Some(h)) = (int(b"Width"), int(b"Height")) else {
        return ImageOutcome::Remove;
    };
    if w <= 0 || h <= 0 || w > 1 << 20 || h > 1 << 20 {
        return ImageOutcome::Remove;
    }
    let (w, h) = (w as u32, h as u32);
    let rects = covered_pixels(ctm, w, h, marks);
    if rects.is_empty() {
        return ImageOutcome::Unchanged;
    }

    let image_mask = dict
        .get(b"ImageMask")
        .ok()
        .and_then(|o| o.as_bool().ok())
        .unwrap_or(false);
    let filters: Vec<Vec<u8>> = stream
        .filters()
        .map(|f| f.into_iter().map(<[u8]>::to_vec).collect())
        .unwrap_or_default();
    let mut out_dict = dict.clone();

    let native = decode_native(doc, stream, &filters, image_mask, w, h, &mut out_dict);
    let decoded = native.or_else(|| {
        // Anything else - CCITT, JBIG2, JPEG 2000, CMYK JPEG - through PDFium,
        // as 8-bit gray or RGB. A stencil mask comes back as gray, dark
        // where it paints, and is packed again as a 1-bit mask.
        let (bytes, n) = decoder?(doc, stream)?;
        let mut d = dict.clone();
        if image_mask {
            if n != 1 {
                return None;
            }
            let stride = (w as usize).div_ceil(8);
            let mut packed = vec![0u8; stride * h as usize];
            for (i, v) in bytes.iter().enumerate().take(w as usize * h as usize) {
                if *v >= 128 {
                    let (row, col) = (i / w as usize, i % w as usize);
                    packed[row * stride + col / 8] |= 0x80 >> (col % 8);
                }
            }
            d.remove(b"Decode");
            d.remove(b"ColorSpace");
            d.set("BitsPerComponent", 1);
            out_dict = d;
            return Some((packed, 1, 1u32, "mask"));
        }
        d.set(
            "ColorSpace",
            Object::Name(if n == 1 {
                b"DeviceGray".to_vec()
            } else {
                b"DeviceRGB".to_vec()
            }),
        );
        d.set("BitsPerComponent", 8);
        d.remove(b"Decode");
        out_dict = d;
        Some((bytes, n, 8u32, if n == 1 { "gray" } else { "rgb" }))
    });
    let Some((mut data, n, bpc, family)) = decoded else {
        return ImageOutcome::Remove;
    };

    let stride = (w as usize * n * bpc as usize).div_ceil(8);
    if data.len() < stride * h as usize {
        return ImageOutcome::Remove;
    }
    let value = if family == "mask" {
        // A stencil paints where samples are 0 (default /Decode): 1 paints
        // nothing, and the fill box covers the spot.
        vec![1]
    } else {
        fill_samples(family, n, bpc, fill)
    };
    blank(&mut data, w, n, bpc, &rects, &value);

    out_dict.remove(b"Filter");
    out_dict.remove(b"DecodeParms");
    out_dict.remove(b"Length");
    let mut out = Stream::new(out_dict, data);
    let _ = out.compress();
    ImageOutcome::Replaced(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covered_pixels_maps_rows_from_top() {
        // 10x10 image placed over [0,100]^2
        let ctm = Matrix([100.0, 0.0, 0.0, 100.0, 0.0, 0.0]);
        let rects = covered_pixels(&ctm, 10, 10, &[Rect::new(0.0, 80.0, 30.0, 100.0)]);
        assert_eq!(rects, vec![[0, 0, 3, 2]]);
    }

    #[test]
    fn blank_sub_byte_samples() {
        let mut data = vec![0u8; 2]; // 2 rows of 8 one-bit pixels
        blank(&mut data, 8, 1, 1, &[[2, 1, 4, 2]], &[1]);
        assert_eq!(data, vec![0, 0b0011_0000]);
    }
}

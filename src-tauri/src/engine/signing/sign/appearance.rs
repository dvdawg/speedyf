//! The visible face of a certificate signature: the signer's name, the date
//! and the reason in Helvetica, with the user's visual signature beside them
//! when they asked for it.
//!
//! Written as one form XObject in the widget's own upright space; a page
//! displayed rotated gets a `/Matrix` that turns the form with it, so the
//! text reads upright on screen.

use super::ArtChoice;
use crate::engine::signature::{stroke_segs, typed_outline, Seg};
use crate::engine::types::SignatureArt;
use crate::errors::{AppError, AppResult};
use base64::Engine as _;
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::fmt::Write as _;

/// Helvetica advance widths for 32..=126, in thousandths of an em.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // 32-47
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 48-63
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // 64-79
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 80-95
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // 96-111
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // 112-126
];

/// Text in WinAnsiEncoding: ASCII and Latin-1 pass through, anything else
/// becomes a question mark (Helvetica has no glyph for it anyway).
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c as u32 {
            0x20..=0x7E | 0xA0..=0xFF => c as u32 as u8,
            _ => b'?',
        })
        .collect()
}

fn width_em(bytes: &[u8]) -> f32 {
    bytes
        .iter()
        .map(|&b| match b {
            32..=126 => HELVETICA[(b - 32) as usize] as f32,
            _ => 556.0,
        })
        .sum::<f32>()
        / 1000.0
}

fn literal(bytes: &[u8]) -> String {
    let mut out = String::from("(");
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            32..=126 => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push(')');
    out
}

fn num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

fn rgb(hex: &str) -> (f32, f32, f32) {
    let h = hex.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0);
    if h.len() == 6 {
        (
            ((v >> 16) & 0xff) as f32 / 255.0,
            ((v >> 8) & 0xff) as f32 / 255.0,
            (v & 0xff) as f32 / 255.0,
        )
    } else {
        (0.0, 0.0, 0.0)
    }
}

/// Maps the artwork's own box (y down) into a rectangle of the form,
/// keeping its proportions and centring it.
struct Fit {
    x: f32,
    y: f32,
    s: f32,
    h: f32,
}

impl Fit {
    fn new(art_w: f32, art_h: f32, bx: f32, by: f32, bw: f32, bh: f32) -> Fit {
        let s = (bw / art_w.max(0.01)).min(bh / art_h.max(0.01));
        Fit {
            x: bx + (bw - art_w * s) / 2.0,
            y: by + (bh - art_h * s) / 2.0,
            s,
            h: art_h,
        }
    }

    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.s, self.y + (self.h - y) * self.s)
    }
}

fn path_ops(segs: &[Seg], fit: &Fit, out: &mut String) {
    for seg in segs {
        match *seg {
            Seg::Move(x, y) => {
                let (x, y) = fit.at(x, y);
                let _ = writeln!(out, "{} {} m", num(x), num(y));
            }
            Seg::Line(x, y) => {
                let (x, y) = fit.at(x, y);
                let _ = writeln!(out, "{} {} l", num(x), num(y));
            }
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                let (ax, ay) = fit.at(ax, ay);
                let (bx, by) = fit.at(bx, by);
                let (x, y) = fit.at(x, y);
                let _ = writeln!(
                    out,
                    "{} {} {} {} {} {} c",
                    num(ax),
                    num(ay),
                    num(bx),
                    num(by),
                    num(x),
                    num(y)
                );
            }
            Seg::Close => out.push_str("h\n"),
        }
    }
}

fn zlib(data: &[u8]) -> AppResult<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data)?;
    Ok(enc.finish()?)
}

/// An image XObject with its transparency as a soft mask.
fn image_xobject(doc: &mut Document, png_b64: &str) -> AppResult<ObjectId> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png_b64)
        .map_err(|e| AppError::Malformed(format!("signature image: {e}")))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|e| AppError::Malformed(format!("signature image: {e}")))?
        .to_rgba8();
    let (w, h) = image.dimensions();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    for px in image.pixels() {
        rgb.extend_from_slice(&px.0[..3]);
        alpha.push(px.0[3]);
    }
    let mask = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceGray",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        zlib(&alpha)?,
    ));
    Ok(doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
            "SMask" => Object::Reference(mask),
        },
        zlib(&rgb)?,
    )))
}

/// Draw the artwork into the box, returning an image XObject to register
/// when it is a picture.
fn art_ops(
    doc: &mut Document,
    choice: &ArtChoice,
    bx: f32,
    by: f32,
    bw: f32,
    bh: f32,
    out: &mut String,
) -> AppResult<Option<ObjectId>> {
    let (r, g, b) = rgb(&choice.color);
    match &choice.art {
        SignatureArt::Drawn {
            w,
            h,
            strokes,
            width,
        } => {
            let fit = Fit::new(*w, *h, bx, by, bw, bh);
            let _ = writeln!(
                out,
                "q {} {} {} RG {} w 1 J 1 j",
                num(r),
                num(g),
                num(b),
                num((width * fit.s).max(0.3))
            );
            for stroke in strokes {
                path_ops(&stroke_segs(stroke), &fit, out);
                out.push_str("S\n");
            }
            out.push_str("Q\n");
            Ok(None)
        }
        SignatureArt::Typed { text, style, .. } => {
            let outline = typed_outline(text, style)?;
            let fit = Fit::new(outline.w, outline.h, bx, by, bw, bh);
            let _ = writeln!(out, "q {} {} {} rg", num(r), num(g), num(b));
            path_ops(&outline.segs, &fit, out);
            out.push_str("f\nQ\n");
            Ok(None)
        }
        SignatureArt::Image { w, h, png } => {
            let id = image_xobject(doc, png)?;
            let fit = Fit::new(*w, *h, bx, by, bw, bh);
            let _ = writeln!(
                out,
                "q {} 0 0 {} {} {} cm /Img1 Do Q",
                num(w * fit.s),
                num(h * fit.s),
                num(fit.x),
                num(fit.y)
            );
            Ok(Some(id))
        }
    }
}

/// Write the appearance form for a `w` x `h` (upright) box and return it.
pub(super) fn write(
    doc: &mut Document,
    w: f32,
    h: f32,
    rot: u16,
    lines: &[String],
    art: Option<&ArtChoice>,
) -> AppResult<ObjectId> {
    let pad = (h.min(w) * 0.06).clamp(1.0, 4.0);
    let mut ops = String::new();
    let mut image = None;

    // With artwork, it takes the left part (or the top, in a tall box).
    let (tx, ty, tw, th) = match art {
        Some(choice) => {
            if w >= h * 1.4 {
                let aw = w * 0.45;
                image = art_ops(
                    doc,
                    choice,
                    pad,
                    pad,
                    aw - 2.0 * pad,
                    h - 2.0 * pad,
                    &mut ops,
                )?;
                (aw, 0.0, w - aw, h)
            } else {
                let ah = h * 0.5;
                image = art_ops(
                    doc,
                    choice,
                    pad,
                    h - ah + pad,
                    w - 2.0 * pad,
                    ah - 2.0 * pad,
                    &mut ops,
                )?;
                (0.0, 0.0, w, h - ah)
            }
        }
        None => (0.0, 0.0, w, h),
    };

    let encoded: Vec<Vec<u8>> = lines.iter().map(|l| win_ansi(l)).collect();
    let widest = encoded
        .iter()
        .map(|l| width_em(l))
        .fold(0.0f32, f32::max)
        .max(0.01);
    let n = encoded.len().max(1) as f32;
    let leading = 1.18;
    let size = ((th - 2.0 * pad) / (n * leading))
        .min((tw - 2.0 * pad) / widest)
        .clamp(2.0, 11.0);
    let block = n * size * leading;
    let top = ty + th - ((th - block) / 2.0).max(pad);
    ops.push_str("q 0.1 0.1 0.1 rg BT\n");
    for (i, line) in encoded.iter().enumerate() {
        let y = top - size * (0.9 + i as f32 * leading);
        let _ = writeln!(
            ops,
            "/Helv {} Tf 1 0 0 1 {} {} Tm {} Tj",
            num(size),
            num(tx + pad),
            num(y),
            literal(line)
        );
    }
    ops.push_str("ET Q\n");

    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut resources = dictionary! {
        "Font" => dictionary! { "Helv" => Object::Reference(font) },
    };
    if let Some(id) = image {
        resources.set("XObject", dictionary! { "Img1" => Object::Reference(id) });
    }
    // Turn the upright form with the page's display rotation.
    let matrix: [i64; 4] = match rot {
        90 => [0, 1, -1, 0],
        180 => [-1, 0, 0, -1],
        270 => [0, -1, 1, 0],
        _ => [1, 0, 0, 1],
    };
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![Object::Real(0.0), Object::Real(0.0), Object::Real(w), Object::Real(h)],
        "Matrix" => vec![
            matrix[0].into(), matrix[1].into(), matrix[2].into(), matrix[3].into(),
            0.into(), 0.into(),
        ],
        "Resources" => resources,
    };
    Ok(doc.add_object(Stream::new(dict, ops.into_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_encoded_and_escaped() {
        assert_eq!(
            win_ansi("Ana (n) \u{e9}\u{4e2d}"),
            b"Ana (n) \xe9?".to_vec()
        );
        assert_eq!(literal(b"a(b)\\\xe9"), "(a\\(b\\)\\\\\\351)");
        assert!((width_em(b"ii") - 0.444).abs() < 1e-3);
        assert_eq!(num(2.5000), "2.5");
        assert_eq!(num(-0.0001), "0");
    }
}

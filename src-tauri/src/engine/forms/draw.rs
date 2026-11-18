//! Appearance streams for the fields Prepare form mode writes.
//!
//! A widget is drawn by its /AP: without one, some readers show nothing at
//! all. PDFium can regenerate an appearance when a value is typed, but not
//! create a field, so the fields made here carry appearances drawn here:
//! text in Helvetica (the AcroForm's /Helv), and check marks and radio dots
//! in ZapfDingbats (/ZaDb), as Acrobat draws its own.
//!
//! Everything is in the widget's own upright box (`w` x `h`, origin at the
//! bottom left as it is displayed); `matrix` turns that box to match a
//! rotated page, the way /MK /R asks.

/// Helvetica advance widths (1/1000 em) for ASCII 32..=126, from the
/// standard AFM. Anything else is measured as a digit.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // 32-47
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 48-63
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // 64-79
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 80-95
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // 96-111
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // 112-126
];

/// Helvetica's ascent and descent, in em.
const ASCENT: f32 = 0.718;
const DESCENT: f32 = 0.207;

/// Padding inside a widget's box, in points, as Acrobat leaves.
const PAD: f32 = 2.0;

/// The size an auto-sized text field is drawn at, at most.
const AUTO_MAX: f32 = 12.0;
const AUTO_MIN: f32 = 4.0;

/// Width of `text` in Helvetica at 1 pt.
pub fn text_width(text: &str) -> f32 {
    text.chars()
        .map(|c| {
            let code = c as u32;
            if (32..=126).contains(&code) {
                HELVETICA[(code - 32) as usize] as f32
            } else {
                556.0
            }
        })
        .sum::<f32>()
        / 1000.0
}

/// `text` as WinAnsi bytes for a literal string, escaped. Characters the
/// encoding lacks become '?'.
pub fn win_ansi_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        let byte: u8 = match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                c as u8
            }
            ' '..='~' => c as u8,
            '\u{20ac}' => 0x80,
            '\u{2018}' => 0x91,
            '\u{2019}' => 0x92,
            '\u{201c}' => 0x93,
            '\u{201d}' => 0x94,
            '\u{2022}' => 0x95,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            _ => b'?',
        };
        if byte < 0x80 {
            out.push(byte as char);
        } else {
            // Octal escapes keep the stream ASCII.
            out.push_str(&format!("\\{byte:03o}"));
        }
    }
    out
}

/// A number for a content stream: short, no exponent, no "-0".
pub fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let rounded = (v * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        return "0".into();
    }
    let s = format!("{rounded:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// /Matrix for an appearance drawn upright in a `w` x `h` box on a page
/// turned by `rotate` degrees.
pub fn matrix(rotate: u16, w: f32, h: f32) -> [f32; 6] {
    match rotate {
        90 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, w, h],
        270 => [0.0, -1.0, 1.0, 0.0, 0.0, w],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

/// How a widget's box is styled.
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub w: f32,
    pub h: f32,
    pub border: bool,
}

impl Frame {
    fn border_ops(&self) -> String {
        if !self.border || self.w < 1.0 || self.h < 1.0 {
            return String::new();
        }
        format!(
            "q 0 G 1 w 0.5 0.5 {} {} re S Q\n",
            num(self.w - 1.0),
            num(self.h - 1.0)
        )
    }
}

/// The size text is drawn at: the field's own, or for auto (0) what fits
/// the box's height, and its width when `fit` is given.
pub fn auto_size(size: f32, h: f32, fit: Option<(&str, f32)>) -> f32 {
    if size > 0.0 {
        return size;
    }
    let mut s = ((h - 2.0 * PAD) / (ASCENT + DESCENT) * 0.9).clamp(AUTO_MIN, AUTO_MAX);
    if let Some((text, w)) = fit {
        let tw = text_width(text);
        if tw > 0.0 {
            s = s.min((w - 2.0 * PAD) / tw).max(AUTO_MIN);
        }
    }
    s
}

/// One line of text in a box, vertically centred, aligned by `align`
/// (0 left, 1 centre, 2 right).
fn line_ops(text: &str, size: f32, frame: &Frame, align: u8, y: f32) -> String {
    let tw = text_width(text) * size;
    let x = match align {
        1 => (frame.w - tw) / 2.0,
        2 => frame.w - PAD - tw,
        _ => PAD,
    };
    format!(
        "BT /Helv {} Tf 0 g {} {} Td ({}) Tj ET\n",
        num(size),
        num(x),
        num(y),
        win_ansi_literal(text)
    )
}

fn centred_baseline(size: f32, h: f32) -> f32 {
    (h - (ASCENT + DESCENT) * size) / 2.0 + DESCENT * size
}

/// Greedy word wrap at `width` points for Helvetica at `size`.
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && text_width(&candidate) * size > width {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// The normal appearance of a text field (or a combo box showing `value`).
pub fn text_appearance(
    value: &str,
    size: f32,
    frame: &Frame,
    align: u8,
    multiline: bool,
    comb: u32,
) -> String {
    let mut ops = frame.border_ops();
    ops.push_str("/Tx BMC\nq\n");
    ops.push_str(&format!(
        "{} {} {} {} re W n\n",
        num(1.0),
        num(1.0),
        num((frame.w - 2.0).max(0.0)),
        num((frame.h - 2.0).max(0.0))
    ));
    if !value.is_empty() {
        if comb > 0 {
            let cell = frame.w / comb as f32;
            let s = auto_size(size, frame.h, None).min(cell * 1.2);
            let y = centred_baseline(s, frame.h);
            for (i, c) in value.chars().take(comb as usize).enumerate() {
                let one = c.to_string();
                let x = cell * i as f32 + (cell - text_width(&one) * s) / 2.0;
                ops.push_str(&format!(
                    "BT /Helv {} Tf 0 g {} {} Td ({}) Tj ET\n",
                    num(s),
                    num(x),
                    num(y),
                    win_ansi_literal(&one)
                ));
            }
        } else if multiline {
            let s = if size > 0.0 { size } else { 10.0 };
            let lead = s * 1.15;
            let mut y = frame.h - PAD - ASCENT * s;
            for line in wrap(value, s, frame.w - 2.0 * PAD) {
                if y < -lead {
                    break;
                }
                ops.push_str(&line_ops(&line, s, frame, align, y));
                y -= lead;
            }
        } else {
            let s = auto_size(size, frame.h, Some((value, frame.w)));
            ops.push_str(&line_ops(
                value,
                s,
                frame,
                align,
                centred_baseline(s, frame.h),
            ));
        }
    }
    ops.push_str("Q\nEMC\n");
    ops
}

/// A list box: its options top-down, the selected ones shaded.
pub fn list_appearance(options: &[String], selected: &[usize], size: f32, frame: &Frame) -> String {
    let s = if size > 0.0 { size } else { 10.0 };
    let lead = s * 1.15;
    let mut ops = frame.border_ops();
    ops.push_str("/Tx BMC\nq\n");
    ops.push_str(&format!(
        "1 1 {} {} re W n\n",
        num((frame.w - 2.0).max(0.0)),
        num((frame.h - 2.0).max(0.0))
    ));
    let mut top = frame.h - 1.0;
    for (i, label) in options.iter().enumerate() {
        if top < 0.0 {
            break;
        }
        if selected.contains(&i) {
            ops.push_str(&format!(
                "0.6 0.75 0.85 rg 1 {} {} {} re f\n",
                num(top - lead),
                num((frame.w - 2.0).max(0.0)),
                num(lead)
            ));
        }
        let y = top - lead + (lead - (ASCENT + DESCENT) * s) / 2.0 + DESCENT * s;
        ops.push_str(&line_ops(label, s, frame, 0, y));
        top -= lead;
    }
    ops.push_str("Q\nEMC\n");
    ops
}

/// ZapfDingbats glyphs a button shows when on: a check (4) or a dot (l).
pub const CHECK_GLYPH: char = '4';
pub const DOT_GLYPH: char = 'l';

/// The "on" appearance of a checkbox or radio button: its glyph centred
/// and sized to the box. The "off" one is `off_appearance`.
pub fn button_on_appearance(glyph: char, size: f32, frame: &Frame) -> String {
    // ZapfDingbats widths for the two glyphs used, and their ink height.
    let (width, height): (f32, f32) = match glyph {
        DOT_GLYPH => (0.791, 0.705),
        _ => (0.846, 0.705),
    };
    let side = frame.w.min(frame.h);
    let s = if size > 0.0 {
        size
    } else {
        (side * 0.8 / height.max(width)).max(1.0)
    };
    let x = (frame.w - width * s) / 2.0;
    let y = (frame.h - height * s) / 2.0;
    let mut ops = frame.border_ops();
    ops.push_str(&format!(
        "q 0 g BT /ZaDb {} Tf {} {} Td ({}) Tj ET Q\n",
        num(s),
        num(x),
        num(y),
        glyph
    ));
    ops
}

pub fn off_appearance(frame: &Frame) -> String {
    frame.border_ops()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(12.3456), "12.346");
    }

    #[test]
    fn literals_escape_and_encode() {
        assert_eq!(win_ansi_literal("a(b)\\"), "a\\(b\\)\\\\");
        assert_eq!(win_ansi_literal("\u{e9}"), "\\351");
        assert_eq!(win_ansi_literal("\u{4e2d}"), "?");
    }

    #[test]
    fn auto_size_fits_height_and_width() {
        assert_eq!(auto_size(9.0, 50.0, None), 9.0);
        assert!(auto_size(0.0, 14.0, None) < 12.0);
        let wide = auto_size(0.0, 30.0, Some(("WWWWWWWW", 60.0)));
        assert!(text_width("WWWWWWWW") * wide <= 60.0);
    }

    #[test]
    fn wraps_at_width() {
        let lines = wrap("one two three four five", 10.0, 60.0);
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(text_width(l) * 10.0 <= 60.0 || !l.contains(' '));
        }
    }

    #[test]
    fn comb_draws_one_glyph_per_cell() {
        let ops = text_appearance(
            "123",
            0.0,
            &Frame {
                w: 60.0,
                h: 14.0,
                border: false,
            },
            0,
            false,
            5,
        );
        assert_eq!(ops.matches(" Tj").count(), 3);
    }
}

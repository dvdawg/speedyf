//! Turning a rendered page into a dark one, without turning it into a negative.
//!
//! The naive `255 - c` is what most viewers ship and it is wrong in two ways
//! that matter for papers. It destroys hue — a blue hyperlink comes back
//! orange, a red warning comes back cyan — and it renders photographs as film
//! negatives. Neither is what "dark mode" is asking for; the request is that
//! the *paper* go dark and the *ink* go light, with everything else recognisably
//! itself.
//!
//! So the transform here works in Oklab: lightness is inverted along a
//! compressed range (white lands on paper-dark, black on ink-light, and the
//! endpoints stop short of pure black/white to avoid halation), while hue and
//! chroma are carried through untouched. Colours that leave sRGB at their new
//! lightness are pulled back by reducing chroma alone, so the hue that survives
//! is the hue the author chose.
//!
//! Photographs are exempted rather than transformed. `render` finds the image
//! objects on the page and asks [`looks_photographic`] about each one; the
//! continuous-tone ones keep their own pixels while the page around them
//! inverts. Line art, plots and screenshots — the images that are mostly white
//! paper themselves — invert with everything else, which is the whole point of
//! doing this for a figure-heavy PDF.
//!
//! The pass sits between PDFium and the encoder, so it has to be cheap. Two
//! lookup tables carry it: an exact one for neutrals, which is nearly the whole
//! of a text page, and a lightness-spaced 3-D one read with trilinear
//! interpolation for everything else. On a 8.4 Mpx raster that is ~16 ms for a
//! page of text and ~103 ms for one that is continuous colour edge to edge,
//! against ~870 ms for the same page computed exactly per pixel.
//!
//! PDFium-free, like `preview` and `figures`, so all of it is ordinary unit
//! tests.

use std::sync::OnceLock;

/// Oklab lightness that pure white is mapped to: the "paper" of a dark page.
/// Kept in step with `--page-bg` under `[data-invert-pages='on']` in the
/// frontend stylesheet by `paper_matches_stylesheet` below.
const PAPER_L: f32 = 0.231;
/// Oklab lightness that pure black is mapped to: the "ink". Short of 1.0 on
/// purpose — full-white text on a near-black page glares.
const INK_L: f32 = 0.943;
/// Chroma is carried across intact apart from this slight pull, which keeps
/// mid-lightness colours from reading as more saturated after the flip than
/// they did before it.
const CHROMA_SCALE: f32 = 0.92;
/// Bisection steps used to pull an out-of-gamut colour back by chroma alone.
/// Six steps resolve the chroma scale to ~1.5%, well under a visible step.
const GAMUT_STEPS: u32 = 6;
/// Spread across a pixel's channels at or under which it is handled as a
/// neutral. Two codes of chroma is invisible, and treating it as grey is what
/// keeps re-encoded white paper from drifting off the neutral axis.
const NEUTRAL_TOLERANCE: u8 = 2;

/// A rectangle in bitmap pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PixelRect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl PixelRect {
    fn right(&self) -> usize {
        self.x + self.w
    }

    fn bottom(&self) -> usize {
        self.y + self.h
    }

    /// Clip to a bitmap, dropping rectangles that fall outside it entirely.
    pub fn clip(&self, w: usize, h: usize) -> Option<PixelRect> {
        let x0 = self.x.min(w);
        let y0 = self.y.min(h);
        let x1 = self.right().min(w);
        let y1 = self.bottom().min(h);
        (x1 > x0 && y1 > y0).then_some(PixelRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }
}

// ---------- colour space ----------

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear sRGB → Oklab (Ottosson's coefficients).
fn linear_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    )
}

/// Oklab → linear sRGB. May land outside `[0, 1]`; the caller decides.
fn oklab_to_linear(ok_l: f32, ok_a: f32, ok_b: f32) -> (f32, f32, f32) {
    let l = ok_l + 0.396_337_78 * ok_a + 0.215_803_76 * ok_b;
    let m = ok_l - 0.105_561_346 * ok_a - 0.063_854_17 * ok_b;
    let s = ok_l - 0.089_484_18 * ok_a - 1.291_485_5 * ok_b;
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    (
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    )
}

fn in_gamut(r: f32, g: f32, b: f32) -> bool {
    const SLACK: f32 = 0.000_5;
    let lo = -SLACK;
    let hi = 1.0 + SLACK;
    r >= lo && r <= hi && g >= lo && g <= hi && b >= lo && b <= hi
}

fn to_u8(c: f32) -> u8 {
    (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// The lightness curve. Decreasing and affine in Oklab L, which is already
/// perceptually spaced — so midtones land where the eye expects rather than
/// bunching up the way a gamma-space flip does.
fn invert_lightness(ok_l: f32) -> f32 {
    INK_L + ok_l.clamp(0.0, 1.0) * (PAPER_L - INK_L)
}

/// Invert one sRGB colour, preserving hue. The exact transform: everything
/// else in this module is an approximation of it, checked against it.
pub fn invert_color(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    if r == g && g == b {
        let out = gray_lut()[r as usize];
        return (out, out, out);
    }
    let (r, g, b) = invert_unit(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    (to_u8(r), to_u8(g), to_u8(b))
}

/// The transform proper, over sRGB components in `0..=1`, answering in linear
/// light. Split out so the lookup table can be built on exact grid positions
/// rather than on whatever 8-bit value happens to sit nearest one.
fn invert_unit(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (lr, lg, lb) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let (ok_l, ok_a, ok_b) = linear_to_oklab(lr, lg, lb);
    let out_l = invert_lightness(ok_l);
    let (target_a, target_b) = (ok_a * CHROMA_SCALE, ok_b * CHROMA_SCALE);

    let (mut cr, mut cg, mut cb) = oklab_to_linear(out_l, target_a, target_b);
    if !in_gamut(cr, cg, cb) {
        // Walk the chroma down, never the hue: the pair (a, b) is scaled
        // together so the angle between them — the hue — is fixed, and only
        // its length changes. `lo` is always in gamut because chroma 0 on a
        // valid lightness is a plain grey.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..GAMUT_STEPS {
            let mid = 0.5 * (lo + hi);
            let (r, g, b) = oklab_to_linear(out_l, target_a * mid, target_b * mid);
            if in_gamut(r, g, b) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (r, g, b) = oklab_to_linear(out_l, target_a * lo, target_b * lo);
        cr = r;
        cg = g;
        cb = b;
    }
    (cr, cg, cb)
}

/// Nodes per axis in the colour lookup table.
const LUT_NODES: usize = 33;

/// Interpolated lookup for non-neutral colours.
///
/// The exact transform costs three cube roots, six powers and, off in the
/// corners of the gamut, a bisection on top of them — about a hundred
/// nanoseconds a pixel, which is a fifth of a second on one page-sized raster
/// of continuous colour. That is the wrong order of magnitude for something
/// standing between PDFium and the screen, so colours come from a table built
/// once and read with trilinear interpolation.
///
/// The nodes are spaced by *lightness*, not by code value. Evenly spaced codes
/// put almost no nodes where the transform bends hardest — the shadows, where
/// a step of one code is a large step in lightness and where every dark ink on
/// the page lives — and interpolating across that gap drifts by ten codes or
/// more. Spacing on the cube root of linear light is the same spacing Oklab's
/// own L uses, which is the axis the transform is a straight line along.
///
/// Neutrals deliberately do NOT come through here: the eight cube corners
/// bracketing a grey are themselves coloured, so interpolating between them
/// tints it. Greys have their own exact 1-D table, and `invert_rgb` sends
/// anything close enough to neutral there instead.
fn color_lut() -> &'static [[u8; 3]] {
    static LUT: OnceLock<Vec<[u8; 3]>> = OnceLock::new();
    LUT.get_or_init(|| {
        let node: Vec<f32> = (0..LUT_NODES)
            .map(|k| {
                let lightness = k as f32 / (LUT_NODES - 1) as f32;
                linear_to_srgb(lightness * lightness * lightness)
            })
            .collect();
        let mut lut = Vec::with_capacity(LUT_NODES * LUT_NODES * LUT_NODES);
        for &r in &node {
            for &g in &node {
                for &b in &node {
                    let (r, g, b) = invert_unit(r, g, b);
                    lut.push([to_u8(r), to_u8(g), to_u8(b)]);
                }
            }
        }
        lut
    })
}

/// Where each 8-bit code sits on the table's lightness-spaced axis. A table
/// itself, so placing a pixel on the grid costs a load rather than the cube
/// root that defines the spacing.
fn axis_lut() -> &'static [f32; 256] {
    static AXIS: OnceLock<[f32; 256]> = OnceLock::new();
    AXIS.get_or_init(|| {
        let mut axis = [0f32; 256];
        for (code, slot) in axis.iter_mut().enumerate() {
            let lightness = srgb_to_linear(code as f32 / 255.0).cbrt();
            *slot = lightness * (LUT_NODES - 1) as f32;
        }
        axis
    })
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Trilinear read of [`color_lut`].
fn invert_color_fast(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let lut = color_lut();
    let axis = axis_lut();
    let place = |c: u8| {
        let pos = axis[c as usize];
        let base = (pos as usize).min(LUT_NODES - 2);
        (base, pos - base as f32)
    };
    let (i, fi) = place(r);
    let (j, fj) = place(g);
    let (k, fk) = place(b);
    let at = |di: usize, dj: usize, dk: usize| {
        lut[((i + di) * LUT_NODES + (j + dj)) * LUT_NODES + k + dk]
    };
    let mut out = [0u8; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        let c = |di, dj, dk| at(di, dj, dk)[channel] as f32;
        let y0 = lerp(
            lerp(c(0, 0, 0), c(0, 0, 1), fk),
            lerp(c(0, 1, 0), c(0, 1, 1), fk),
            fj,
        );
        let y1 = lerp(
            lerp(c(1, 0, 0), c(1, 0, 1), fk),
            lerp(c(1, 1, 0), c(1, 1, 1), fk),
            fj,
        );
        *slot = (lerp(y0, y1, fi) + 0.5).clamp(0.0, 255.0) as u8;
    }
    (out[0], out[1], out[2])
}

/// Neutral colours collapse to one dimension, and a rendered page is almost
/// entirely neutral — white paper, black text, and the greys of antialiasing
/// between them. Answering those 256 cases from a table is what keeps the pass
/// over a multi-megapixel page cheap enough to sit in the render path.
fn gray_lut() -> &'static [u8; 256] {
    static LUT: OnceLock<[u8; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0u8; 256];
        for (value, slot) in lut.iter_mut().enumerate() {
            let linear = srgb_to_linear(value as f32 / 255.0);
            // For a neutral, Oklab L is the cube root of linear luminance and
            // both chroma axes are zero, so the round trip is this short.
            let out_l = invert_lightness(linear.cbrt());
            *slot = to_u8(out_l * out_l * out_l);
        }
        lut
    })
}

// ---------- bitmap pass ----------

/// Spans of `[0, width)` on row `y` that no protected rectangle covers.
///
/// `rects` must be sorted by `x`. Returned spans are in ascending order and do
/// not touch, so a caller can walk them as the row's writable segments.
pub fn unprotected_spans(
    rects: &[PixelRect],
    y: usize,
    width: usize,
    out: &mut Vec<(usize, usize)>,
) {
    out.clear();
    let mut cursor = 0usize;
    for rect in rects {
        if y < rect.y || y >= rect.bottom() {
            continue;
        }
        let (start, end) = (rect.x.min(width), rect.right().min(width));
        if end <= cursor {
            continue;
        }
        if start > cursor {
            out.push((cursor, start));
        }
        cursor = end;
    }
    if cursor < width {
        out.push((cursor, width));
    }
}

/// Invert an RGB8 bitmap in place, leaving `protect` untouched.
pub fn invert_rgb(rgb: &mut [u8], width: usize, height: usize, protect: &[PixelRect]) {
    if width == 0 || height == 0 {
        return;
    }
    let mut rects: Vec<PixelRect> = protect
        .iter()
        .filter_map(|rect| rect.clip(width, height))
        .collect();
    rects.sort_by_key(|rect| rect.x);

    let mut spans: Vec<(usize, usize)> = Vec::new();
    for y in 0..height {
        if rects.is_empty() {
            spans.clear();
            spans.push((0, width));
        } else {
            unprotected_spans(&rects, y, width, &mut spans);
        }
        let row = &mut rgb[y * width * 3..(y + 1) * width * 3];
        for &(start, end) in spans.iter() {
            for px in row[start * 3..end * 3].chunks_exact_mut(3) {
                let (r, g, b) = (px[0], px[1], px[2]);
                let low = r.min(g).min(b);
                if b.max(g).max(r) - low <= NEUTRAL_TOLERANCE {
                    // Exactly neutral, or as good as: a chroma of a code or two
                    // is beneath noticing, and it is what JPEG leaves behind on
                    // the white ground of a scanned or re-encoded figure. Taking
                    // it as grey keeps that ground off the interpolated path,
                    // where it would come back faintly tinted.
                    let out = gray_lut()[low as usize];
                    px[0] = out;
                    px[1] = out;
                    px[2] = out;
                    continue;
                }
                let (or, og, ob) = invert_color_fast(r, g, b);
                px[0] = or;
                px[1] = og;
                px[2] = ob;
            }
        }
    }
}

// ---------- photograph detection ----------

/// Pixels sampled from an image region before classifying it. Enough to be
/// stable on a photograph, few enough that a page full of figures costs
/// nothing measurable.
const SAMPLE_TARGET: usize = 4096;
/// Channel value at or above which a pixel counts as paper rather than content.
const NEAR_WHITE: u8 = 0xF0;
/// A region with at least this much paper in it is line art on a white ground —
/// a plot, a diagram, a screenshot of a document — and wants inverting with the
/// rest of the page.
const PAPER_FRACTION: f32 = 0.35;
/// Continuous tone spreads across the histogram. Fewer occupied buckets than
/// this means flat fills and hard edges, which is drawn artwork, not a photo.
const MIN_TONE_BUCKETS: usize = 14;
/// Buckets the 0..=255 luma range is quantised into for that count.
const TONE_BUCKETS: usize = 32;

/// Whether an image region is continuous-tone artwork that inverting would
/// wreck — a photograph, a micrograph, a rendered 3D scene.
///
/// The two signals are deliberately cheap and independent: how much of the
/// region is white paper, and how many distinct tones it spans. A photograph
/// has little paper and many tones; a chart has a lot of paper; a flat logo or
/// line drawing has few tones. Only the region that is short on paper *and*
/// rich in tone is protected, so the common case for a paper — a matplotlib
/// figure on a white ground — still goes dark.
pub fn looks_photographic(rgb: &[u8], width: usize, height: usize, rect: PixelRect) -> bool {
    let Some(rect) = rect.clip(width, height) else {
        return false;
    };
    // Both axes get their own stride so a wide, short banner samples across its
    // width rather than reading one column many times.
    let step_x = (rect.w as f32 / (SAMPLE_TARGET as f32).sqrt())
        .ceil()
        .max(1.0) as usize;
    let step_y = (rect.h as f32 / (SAMPLE_TARGET as f32).sqrt())
        .ceil()
        .max(1.0) as usize;

    let mut buckets = [false; TONE_BUCKETS];
    let mut occupied = 0usize;
    let mut paper = 0usize;
    let mut total = 0usize;
    let mut y = rect.y;
    while y < rect.bottom() {
        let row = &rgb[y * width * 3..(y + 1) * width * 3];
        let mut x = rect.x;
        while x < rect.right() {
            let (r, g, b) = (row[x * 3], row[x * 3 + 1], row[x * 3 + 2]);
            if r >= NEAR_WHITE && g >= NEAR_WHITE && b >= NEAR_WHITE {
                paper += 1;
            }
            // Rec. 601 luma in integer arithmetic; exactness is irrelevant to a
            // bucket count and this stays off the float path.
            let luma = (77 * r as usize + 150 * g as usize + 29 * b as usize) >> 8;
            let bucket = (luma * TONE_BUCKETS / 256).min(TONE_BUCKETS - 1);
            if !buckets[bucket] {
                buckets[bucket] = true;
                occupied += 1;
            }
            total += 1;
            x += step_x;
        }
        y += step_y;
    }
    if total == 0 {
        return false;
    }
    (paper as f32 / total as f32) < PAPER_FRACTION && occupied >= MIN_TONE_BUCKETS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oklab_l_of(r: u8, g: u8, b: u8) -> f32 {
        let (lr, lg, lb) = (
            srgb_to_linear(r as f32 / 255.0),
            srgb_to_linear(g as f32 / 255.0),
            srgb_to_linear(b as f32 / 255.0),
        );
        linear_to_oklab(lr, lg, lb).0
    }

    fn hue_of(r: u8, g: u8, b: u8) -> f32 {
        let (lr, lg, lb) = (
            srgb_to_linear(r as f32 / 255.0),
            srgb_to_linear(g as f32 / 255.0),
            srgb_to_linear(b as f32 / 255.0),
        );
        let (_, a, b) = linear_to_oklab(lr, lg, lb);
        b.atan2(a)
    }

    fn solid(width: usize, height: usize, color: [u8; 3]) -> Vec<u8> {
        color
            .iter()
            .copied()
            .cycle()
            .take(width * height * 3)
            .collect()
    }

    #[test]
    fn white_becomes_paper_dark_and_black_becomes_ink_light() {
        let (r, g, b) = invert_color(255, 255, 255);
        assert_eq!((r, g, b), (r, r, r), "paper stays neutral");
        assert!((28..=32).contains(&r), "white → near #1d1d1d, got {r}");

        let (r, g, b) = invert_color(0, 0, 0);
        assert_eq!((r, g, b), (r, r, r), "ink stays neutral");
        assert!((234..=238).contains(&r), "black → near #ececec, got {r}");
    }

    /// The stylesheet paints `.page` behind the raster, and any mismatch shows
    /// as a rim of the wrong grey around every page. `--page-bg-inverted` is
    /// #1d1d1d, so that is what white has to land on.
    #[test]
    fn paper_matches_stylesheet() {
        assert_eq!(invert_color(255, 255, 255), (0x1d, 0x1d, 0x1d));
    }

    #[test]
    fn hue_survives_the_flip() {
        // A link blue, a warning red, a chart green: all dark-on-white inks.
        for &color in &[[0x1a, 0x3f, 0xd0], [0xc6, 0x28, 0x28], [0x1b, 0x7f, 0x3a]] {
            let (r, g, b) = invert_color(color[0], color[1], color[2]);
            let before = hue_of(color[0], color[1], color[2]);
            let after = hue_of(r, g, b);
            let drift = (after - before).abs();
            let drift = drift.min(std::f32::consts::TAU - drift);
            assert!(
                drift < 0.09,
                "{color:?} → {:?} drifted {drift} rad",
                (r, g, b)
            );
        }
    }

    /// The failure mode this whole module exists to avoid: `255 - c` sends a
    /// blue to an orange, which is a hue rotation of about half a turn.
    #[test]
    fn beats_naive_inversion_on_hue() {
        let color = [0x1a, 0x3f, 0xd0];
        let naive = hue_of(255 - color[0], 255 - color[1], 255 - color[2]);
        let before = hue_of(color[0], color[1], color[2]);
        let naive_drift = (naive - before)
            .abs()
            .min(std::f32::consts::TAU - (naive - before).abs());
        assert!(naive_drift > 1.5, "the baseline really is this bad");
    }

    #[test]
    fn lightness_is_monotonically_reversed() {
        let mut previous = 255i32;
        for value in 0..=255u8 {
            let (out, _, _) = invert_color(value, value, value);
            assert!(
                (out as i32) <= previous,
                "grey {value} broke the ordering: {out} after {previous}"
            );
            previous = out as i32;
        }
    }

    #[test]
    fn dark_ink_ends_up_lighter_than_the_paper_it_sat_on() {
        // Whatever the hue, text has to come out clearly above the background.
        for &color in &[[0x1a, 0x3f, 0xd0], [0x33, 0x33, 0x33], [0x80, 0x20, 0x20]] {
            let (r, g, b) = invert_color(color[0], color[1], color[2]);
            let paper = invert_color(255, 255, 255);
            assert!(
                oklab_l_of(r, g, b) > oklab_l_of(paper.0, paper.1, paper.2) + 0.25,
                "{color:?} → {:?} is not readable on the inverted page",
                (r, g, b)
            );
        }
    }

    #[test]
    fn every_colour_lands_inside_srgb() {
        // A saturated ramp at both ends of the lightness range is where naive
        // chroma preservation blows out.
        for &r in &[0u8, 40, 128, 200, 255] {
            for &g in &[0u8, 40, 128, 200, 255] {
                for &b in &[0u8, 40, 128, 200, 255] {
                    let (or, og, ob) = invert_color(r, g, b);
                    // Round-tripping the answer must not move it: an out-of-gamut
                    // intermediate would have been clipped, and clipping shows up
                    // as a hue that no longer matches the requested one.
                    let want = hue_of(r, g, b);
                    let got = hue_of(or, og, ob);
                    let drift = (got - want).abs();
                    let drift = drift.min(std::f32::consts::TAU - drift);
                    let chroma_free = r == g && g == b;
                    assert!(
                        chroma_free || drift < 0.35,
                        "({r},{g},{b}) → ({or},{og},{ob}) drifted {drift} rad"
                    );
                }
            }
        }
    }

    #[test]
    fn spans_cover_the_row_when_nothing_is_protected() {
        let mut spans = Vec::new();
        unprotected_spans(&[], 3, 100, &mut spans);
        assert_eq!(spans, vec![(0, 100)]);
    }

    #[test]
    fn spans_skip_a_protected_rect_on_the_rows_it_covers() {
        let rects = [PixelRect {
            x: 20,
            y: 2,
            w: 30,
            h: 4,
        }];
        let mut spans = Vec::new();

        unprotected_spans(&rects, 3, 100, &mut spans);
        assert_eq!(spans, vec![(0, 20), (50, 100)]);

        unprotected_spans(&rects, 6, 100, &mut spans);
        assert_eq!(spans, vec![(0, 100)], "row below the rect is untouched");
    }

    #[test]
    fn overlapping_rects_do_not_produce_backwards_spans() {
        let rects = [
            PixelRect {
                x: 10,
                y: 0,
                w: 40,
                h: 10,
            },
            PixelRect {
                x: 20,
                y: 0,
                w: 10,
                h: 10,
            },
            PixelRect {
                x: 45,
                y: 0,
                w: 20,
                h: 10,
            },
        ];
        let mut spans = Vec::new();
        unprotected_spans(&rects, 5, 100, &mut spans);
        assert_eq!(spans, vec![(0, 10), (65, 100)]);
    }

    #[test]
    fn a_rect_running_off_the_edge_is_clipped() {
        let rects = [PixelRect {
            x: 90,
            y: 0,
            w: 40,
            h: 10,
        }];
        let mut spans = Vec::new();
        unprotected_spans(&rects, 0, 100, &mut spans);
        assert_eq!(spans, vec![(0, 90)]);
    }

    #[test]
    fn protected_pixels_keep_their_original_values() {
        let (w, h) = (8usize, 8usize);
        let mut rgb = solid(w, h, [255, 255, 255]);
        let rect = PixelRect {
            x: 2,
            y: 2,
            w: 3,
            h: 3,
        };
        invert_rgb(&mut rgb, w, h, &[rect]);

        let at = |x: usize, y: usize| {
            let i = (y * w + x) * 3;
            (rgb[i], rgb[i + 1], rgb[i + 2])
        };
        assert_eq!(at(3, 3), (255, 255, 255), "inside the protected rect");
        assert_eq!(at(0, 0), invert_color(255, 255, 255), "outside it");
        assert_eq!(at(5, 3), invert_color(255, 255, 255), "just past its edge");
        assert_eq!(at(2, 1), invert_color(255, 255, 255), "just above it");
    }

    #[test]
    fn inverting_a_bitmap_matches_inverting_its_pixels() {
        let (w, h) = (4usize, 3usize);
        let mut rgb = Vec::new();
        let colors = [
            [255u8, 255, 255],
            [0, 0, 0],
            [0x1a, 0x3f, 0xd0],
            [0x80, 0x80, 0x80],
        ];
        for i in 0..w * h {
            rgb.extend_from_slice(&colors[i % colors.len()]);
        }
        let expected: Vec<u8> = rgb
            .chunks_exact(3)
            .flat_map(|px| {
                let (r, g, b) = invert_color(px[0], px[1], px[2]);
                [r, g, b]
            })
            .collect();
        invert_rgb(&mut rgb, w, h, &[]);
        for (got, want) in rgb.iter().zip(expected.iter()) {
            assert!(
                got.abs_diff(*want) <= LUT_TOLERANCE,
                "bitmap pass gave {got} where the exact transform gives {want}"
            );
        }
    }

    /// Neutrals must come back neutral. A tint on the greys is the one error
    /// the eye picks up instantly, because it lands on the body text.
    #[test]
    fn greys_stay_neutral_through_the_bitmap_pass() {
        let values: Vec<u8> = (0..=255u8).collect();
        let mut rgb: Vec<u8> = values.iter().flat_map(|&v| [v, v, v]).collect();
        invert_rgb(&mut rgb, values.len(), 1, &[]);
        for (i, px) in rgb.chunks_exact(3).enumerate() {
            assert_eq!(
                (px[0], px[1], px[2]),
                (px[0], px[0], px[0]),
                "grey {i} came back tinted"
            );
            assert_eq!(px[0], invert_color(i as u8, i as u8, i as u8).0);
        }
    }

    /// A code or two of chroma — what JPEG leaves on white paper — is taken as
    /// neutral rather than dragged through the interpolated path.
    #[test]
    fn near_neutrals_are_treated_as_grey() {
        let mut rgb = vec![254u8, 255, 253, 200, 201, 199];
        invert_rgb(&mut rgb, 2, 1, &[]);
        assert_eq!(&rgb[0..3], &[gray_lut()[253]; 3]);
        assert_eq!(&rgb[3..6], &[gray_lut()[199]; 3]);
    }

    /// How far the table is allowed to sit from the exact transform.
    ///
    /// The residual is not a resolution problem — quadrupling the nodes only
    /// takes it from 12 to 7. It is the gamut mapping's own kink: colours that
    /// need their chroma cut and colours that do not meet along a crease, and
    /// interpolating across a crease rounds it off. It shows up only on inks
    /// already at the edge of what sRGB can hold, where a dozen codes of
    /// saturation is not a difference anyone reads off a page.
    const LUT_TOLERANCE: u8 = 12;

    #[test]
    fn lut_tracks_the_exact_transform() {
        let mut worst = 0u8;
        let mut worst_at = (0u8, 0u8, 0u8);
        // A sweep dense enough to land between nodes, not just on them.
        for r in (0..=255u16).step_by(5) {
            for g in (0..=255u16).step_by(5) {
                for b in (0..=255u16).step_by(5) {
                    let (r, g, b) = (r as u8, g as u8, b as u8);
                    if b.max(g).max(r) - r.min(g).min(b) <= NEUTRAL_TOLERANCE {
                        continue; // handled by the exact grey table
                    }
                    let exact = invert_color(r, g, b);
                    let fast = invert_color_fast(r, g, b);
                    for (a, e) in [(fast.0, exact.0), (fast.1, exact.1), (fast.2, exact.2)] {
                        if a.abs_diff(e) > worst {
                            worst = a.abs_diff(e);
                            worst_at = (r, g, b);
                        }
                    }
                }
            }
        }
        assert!(
            worst <= LUT_TOLERANCE,
            "table drifts {worst} codes from the exact transform at {worst_at:?}"
        );
    }

    #[test]
    fn a_chart_on_white_is_not_treated_as_a_photograph() {
        // Mostly paper, with coloured strokes across it.
        let (w, h) = (64usize, 64usize);
        let mut rgb = solid(w, h, [255, 255, 255]);
        for y in 0..h {
            for x in 0..w {
                if x == y || y == h / 2 {
                    let i = (y * w + x) * 3;
                    rgb[i..i + 3].copy_from_slice(&[0x1f, 0x77, 0xb4]);
                }
            }
        }
        assert!(!looks_photographic(
            &rgb,
            w,
            h,
            PixelRect { x: 0, y: 0, w, h }
        ));
    }

    #[test]
    fn a_flat_logo_is_not_treated_as_a_photograph() {
        // Little paper, but only a handful of tones: drawn artwork, and
        // inverting it is what the user asked for.
        let (w, h) = (64usize, 64usize);
        let mut rgb = solid(w, h, [0x20, 0x30, 0x90]);
        for y in 0..h / 2 {
            for x in 0..w {
                let i = (y * w + x) * 3;
                rgb[i..i + 3].copy_from_slice(&[0xd0, 0x40, 0x30]);
            }
        }
        assert!(!looks_photographic(
            &rgb,
            w,
            h,
            PixelRect { x: 0, y: 0, w, h }
        ));
    }

    #[test]
    fn a_continuous_tone_image_is_protected() {
        // A smooth gradient with noise: little paper, tones everywhere.
        let (w, h) = (64usize, 64usize);
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let base = (x * 3 + y * 2) as u8;
                rgb[i] = base.wrapping_add(20);
                rgb[i + 1] = base / 2 + 30;
                rgb[i + 2] = 200u8.saturating_sub(base);
            }
        }
        assert!(looks_photographic(
            &rgb,
            w,
            h,
            PixelRect { x: 0, y: 0, w, h }
        ));
    }

    #[test]
    fn only_the_named_region_is_classified() {
        // A photograph in the corner of an otherwise blank page: asking about
        // the page must not inherit the photograph's answer, and vice versa.
        let (w, h) = (64usize, 64usize);
        let mut rgb = solid(w, h, [255, 255, 255]);
        for y in 0..16 {
            for x in 0..16 {
                let i = (y * w + x) * 3;
                let base = (x * 9 + y * 5) as u8;
                rgb[i..i + 3].copy_from_slice(&[base, base / 2 + 40, 180 - base / 2]);
            }
        }
        assert!(looks_photographic(
            &rgb,
            w,
            h,
            PixelRect {
                x: 0,
                y: 0,
                w: 16,
                h: 16
            }
        ));
        assert!(!looks_photographic(
            &rgb,
            w,
            h,
            PixelRect { x: 0, y: 0, w, h }
        ));
    }

    #[test]
    fn an_empty_or_offscreen_region_is_never_protected() {
        let (w, h) = (8usize, 8usize);
        let rgb = solid(w, h, [10, 20, 30]);
        assert!(!looks_photographic(
            &rgb,
            w,
            h,
            PixelRect {
                x: 0,
                y: 0,
                w: 0,
                h: 4
            }
        ));
        assert!(!looks_photographic(
            &rgb,
            w,
            h,
            PixelRect {
                x: 50,
                y: 50,
                w: 4,
                h: 4
            }
        ));
    }
}

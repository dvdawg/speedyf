/** The engine's page inversion, for a single colour.
 *
 * Overlays drawn by the shell are DOM siblings of the raster, so nothing the
 * engine does to a page reaches them (see the note above `[data-invert-pages]`
 * in the stylesheet). Most overlays want it that way — they are the reader's
 * marks and are already theme-aware. The exception is an overlay that is
 * pretending to be *ink on the page*: a shaded bubble is near-black because
 * that is what a pen leaves, and near-black on an inverted page is invisible.
 *
 * Rather than swap in some other colour that happens to show up, such an
 * overlay is put through the same transform the raster went through, so it
 * lands exactly where the printed ring beside it landed. This is a port of
 * `invert_color` in `src-tauri/src/engine/invert.rs` — the exact transform,
 * not the lookup tables the per-pixel path uses. Keep the two in step; the
 * constants and the checks in `invertColor.test.ts` mirror that module.
 */

/** Oklab lightness pure white maps to: the "paper" of a dark page. */
const PAPER_L = 0.231;
/** Oklab lightness pure black maps to: the "ink", short of 1.0 so it doesn't glare. */
const INK_L = 0.943;
/** Slight pull on chroma, so colours don't read as more saturated after the flip. */
const CHROMA_SCALE = 0.92;
/** Bisection steps used to pull an out-of-gamut colour back by chroma alone. */
const GAMUT_STEPS = 6;

function srgbToLinear(c: number): number {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function linearToSrgb(c: number): number {
  return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
}

/** Linear sRGB → Oklab (Ottosson's coefficients). */
function linearToOklab(r: number, g: number, b: number): [number, number, number] {
  const l = Math.cbrt(0.41222147 * r + 0.53633254 * g + 0.051445995 * b);
  const m = Math.cbrt(0.2119035 * r + 0.6806995 * g + 0.10739696 * b);
  const s = Math.cbrt(0.08830246 * r + 0.28171885 * g + 0.6299787 * b);
  return [
    0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
    1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
    0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
  ];
}

/** Oklab → linear sRGB. May land outside `[0, 1]`; the caller decides. */
function oklabToLinear(okL: number, okA: number, okB: number): [number, number, number] {
  const l = (okL + 0.39633778 * okA + 0.21580376 * okB) ** 3;
  const m = (okL - 0.105561346 * okA - 0.06385417 * okB) ** 3;
  const s = (okL - 0.08948418 * okA - 1.2914855 * okB) ** 3;
  return [
    4.0767417 * l - 3.3077116 * m + 0.23096994 * s,
    -1.268438 * l + 2.6097574 * m - 0.34131938 * s,
    -0.0041960863 * l - 0.7034186 * m + 1.7076147 * s,
  ];
}

function inGamut(r: number, g: number, b: number): boolean {
  const slack = 0.0005;
  const lo = -slack;
  const hi = 1 + slack;
  return r >= lo && r <= hi && g >= lo && g <= hi && b >= lo && b <= hi;
}

function toByte(c: number): number {
  return Math.min(255, Math.max(0, Math.round(linearToSrgb(Math.min(1, Math.max(0, c))) * 255)));
}

/** The lightness curve: decreasing and affine in Oklab L. */
function invertLightness(okL: number): number {
  return INK_L + Math.min(1, Math.max(0, okL)) * (PAPER_L - INK_L);
}

/** The transform proper, over sRGB components in `0..=1`, answering in linear light. */
function invertUnit(r: number, g: number, b: number): [number, number, number] {
  const [okL, okA, okB] = linearToOklab(srgbToLinear(r), srgbToLinear(g), srgbToLinear(b));
  const outL = invertLightness(okL);
  const targetA = okA * CHROMA_SCALE;
  const targetB = okB * CHROMA_SCALE;

  const direct = oklabToLinear(outL, targetA, targetB);
  if (inGamut(direct[0], direct[1], direct[2])) return direct;

  // Walk the chroma down, never the hue: the pair (a, b) is scaled together so
  // the angle between them — the hue — is fixed, and only its length changes.
  // `lo` is always in gamut because chroma 0 on a valid lightness is a grey.
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < GAMUT_STEPS; i++) {
    const mid = 0.5 * (lo + hi);
    const [r2, g2, b2] = oklabToLinear(outL, targetA * mid, targetB * mid);
    if (inGamut(r2, g2, b2)) lo = mid;
    else hi = mid;
  }
  return oklabToLinear(outL, targetA * lo, targetB * lo);
}

/** Invert one colour, preserving hue: `#1a1a1a` comes back as the near-white
 * the page's own black text became, `#ffffff` as the page background.
 *
 * Takes and returns a CSS `#rgb`/`#rrggbb` string. Anything else is returned
 * unchanged — an annotation colour is user data, and a colour this cannot
 * parse is better shown as authored than dropped. */
export function invertInk(color: string): string {
  const rgb = parseHex(color);
  if (!rgb) return color;
  const [r, g, b] = rgb;
  if (r === g && g === b) {
    // For a neutral, Oklab L is the cube root of linear luminance and both
    // chroma axes are zero, so the round trip is this short — and staying off
    // the general path is what keeps a grey from picking up a tint.
    const outL = invertLightness(Math.cbrt(srgbToLinear(r / 255)));
    return hex([outL ** 3, outL ** 3, outL ** 3]);
  }
  return hex(invertUnit(r / 255, g / 255, b / 255));
}

function hex(linear: readonly number[]): string {
  return `#${linear.map((c) => toByte(c).toString(16).padStart(2, '0')).join('')}`;
}

function parseHex(color: string): [number, number, number] | null {
  const body = color.trim().replace(/^#/, '');
  const digits = /^[0-9a-f]{3}$/i.test(body) ? body.replace(/./g, '$&$&') : body;
  if (!/^[0-9a-f]{6}$/i.test(digits)) return null;
  return [
    parseInt(digits.slice(0, 2), 16),
    parseInt(digits.slice(2, 4), 16),
    parseInt(digits.slice(4, 6), 16),
  ];
}

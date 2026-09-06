/**
 * The colour port, checked against the engine it mirrors.
 *
 * Every expected value here was produced by `engine::invert::invert_color`
 * itself, not by this module — the point of the test is that an overlay
 * pretending to be ink lands exactly where the raster's ink landed. If the
 * two ever drift, a shaded bubble stops matching the ring printed around it.
 */
import { describe, expect, it } from 'vitest';
import { invertInk } from './invertColor';

/** Hue in Oklab's (a, b) plane, for checking the flip does not rotate it. */
function hueOf(hex: string): number {
  const lin = (i: number) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };
  const [lr, lg, lb] = [lin(1), lin(3), lin(5)];
  const l = Math.cbrt(0.41222147 * lr + 0.53633254 * lg + 0.051445995 * lb);
  const m = Math.cbrt(0.2119035 * lr + 0.6806995 * lg + 0.10739696 * lb);
  const s = Math.cbrt(0.08830246 * lr + 0.28171885 * lg + 0.6299787 * lb);
  return Math.atan2(
    0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
    1.9779985 * l - 2.4285922 * m + 0.4505937 * s
  );
}

describe('invertInk', () => {
  // The same anchor `paper_matches_stylesheet` asserts in invert.rs, and the
  // value of --page-bg under [data-invert-pages='on'].
  it('sends white to the inverted page background', () => {
    expect(invertInk('#ffffff')).toBe('#1d1d1d');
  });

  // The interesting case: the near-black a shaded bubble is drawn in. It must
  // come back as the same grey the ring printed around it comes back as, so
  // the two still read as one filled control.
  it('sends the shading ink where the page sends its own black', () => {
    expect(invertInk('#1a1a1a')).toBe('#bababa');
    expect(invertInk('#000000')).toBe('#ececec');
  });

  it('agrees with the engine across the neutral axis', () => {
    // Sampled from the engine's grey table; greys are nearly the whole of a
    // text page, and a tint here would show on every antialiased edge.
    const engineGreys: Record<string, string> = {
      '#010101': '#dcdcdc',
      '#020202': '#d8d8d8',
      '#404040': '#989898',
      '#808080': '#686868',
      '#c0c0c0': '#3f3f3f',
      '#fefefe': '#1e1e1e',
    };
    for (const [before, after] of Object.entries(engineGreys)) {
      expect(invertInk(before)).toBe(after);
    }
  });

  it('agrees with the engine on colours, including out-of-gamut ones', () => {
    // A link blue, a warning red, a chart green, a highlighter yellow, and a
    // pure blue — the last needs the chroma bisection to come back in gamut.
    const engineColors: Record<string, string> = {
      '#1a3fd0': '#4a7bfe',
      '#c62828': '#c73b36',
      '#1b7f3a': '#378b4b',
      '#ffd54f': '#3d2f00',
      '#0000ff': '#457cfe',
    };
    for (const [before, after] of Object.entries(engineColors)) {
      expect(invertInk(before)).toBe(after);
    }
  });

  // The property the engine exists for: a blue shading comes back a lighter
  // blue, never the orange `255 - c` would make of it.
  it('keeps the hue a shading was drawn in', () => {
    for (const color of ['#1a3fd0', '#c62828', '#1b7f3a']) {
      const drift = Math.abs(hueOf(invertInk(color)) - hueOf(color));
      expect(Math.min(drift, 2 * Math.PI - drift)).toBeLessThan(0.09);
    }
  });

  it('reads short hex the way CSS does', () => {
    expect(invertInk('#fff')).toBe(invertInk('#ffffff'));
    expect(invertInk('#1A3FD0')).toBe(invertInk('#1a3fd0'));
  });

  // An annotation's colour is user data. One this cannot parse is shown as
  // authored rather than dropped or turned into black.
  it('passes through anything it cannot parse', () => {
    for (const color of ['rgb(0 0 0)', 'red', '', '#12345']) {
      expect(invertInk(color)).toBe(color);
    }
  });
});

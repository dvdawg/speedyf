import { describe, expect, it } from 'vitest';
import { anchorBox, cropSpec } from './cropGeometry';
import type { Anchor, Quad } from './types';

const page = { widthPt: 612, heightPt: 792 };

/** A quad for a line from x0 to x1 with its bottom at y, 10pt tall. */
const line = (x0: number, x1: number, y: number): Quad => ({
  x1: x0,
  y1: y + 10,
  x2: x1,
  y2: y + 10,
  x3: x0,
  y3: y,
  x4: x1,
  y4: y,
});

const quoted = (quads: Quad[]): Anchor => ({ page: 0, quads, rect: null, quote: 'q' });

describe('anchorBox', () => {
  it('is the lines together, padded', () => {
    expect(anchorBox(quoted([line(100, 300, 500), line(72, 250, 488)]), page)).toEqual({
      x: 69,
      y: 485,
      w: 234,
      h: 28,
    });
  });

  it('stays on the page and keeps the top of a long passage', () => {
    const box = anchorBox(quoted([line(0, 612, 780), line(0, 300, 20)]), page)!;
    expect(box.x).toBe(0);
    expect(box.x + box.w).toBe(612);
    expect(box.y + box.h).toBe(792);
    expect(box.h).toBe(360);
  });

  it('is a region comment its box', () => {
    const region: Anchor = {
      page: 0,
      quads: null,
      rect: { x: 100, y: 100, w: 50, h: 40 },
      quote: '',
    };
    expect(anchorBox(region, page)).toEqual({ x: 97, y: 97, w: 56, h: 46 });
  });

  it('is nothing for an anchor with no area', () => {
    expect(anchorBox(quoted([]), page)).toBeNull();
  });
});

describe('cropSpec', () => {
  it('fits the card and renders sharp for the screen', () => {
    const spec = cropSpec(quoted([line(100, 300, 500), line(72, 250, 488)]), page, 280, 2)!;
    // 234pt into 280px: 1.2 CSS px a point, rendered at the 3x bucket
    expect(spec.width).toBeCloseTo(280);
    expect(spec.scaleMilli).toBe(3000);
    // from the page's top-left, in pixels at that scale
    expect(spec.tile).toEqual({ x: 207, y: 837, w: 702, h: 84 });
  });

  it('never blows a short quote up past a readable size', () => {
    const spec = cropSpec(quoted([line(100, 130, 500)]), page, 280, 1)!;
    expect(spec.width).toBeCloseTo(36 * 1.6);
  });
});

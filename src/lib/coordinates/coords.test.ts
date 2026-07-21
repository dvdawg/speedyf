import { describe, expect, it } from 'vitest';
import {
  anchorScrollTop,
  bucketForScale,
  cssToPdf,
  fitPageZoom,
  fitWidthZoom,
  pageCssDims,
  pageNormToPdfUser,
  pdfRectToCssRect,
  pdfToPageCss,
  pdfUserToPageNorm,
  RENDER_SCALE_BUCKETS,
  tileGrid,
} from './coords';
import { layoutPages } from './layout';
import type { Rotation } from '../../types/model';

const page = (w = 600, h = 800, rotation: Rotation = 0) => ({
  widthPt: w,
  heightPt: h,
  rotation,
});

describe('pageCssDims', () => {
  it('keeps dims at 0/180 and swaps at 90/270, scaled by zoom', () => {
    expect(pageCssDims(page(600, 800, 0), 1)).toEqual({ w: 600, h: 800 });
    expect(pageCssDims(page(600, 800, 180), 2)).toEqual({ w: 1200, h: 1600 });
    expect(pageCssDims(page(600, 800, 90), 1)).toEqual({ w: 800, h: 600 });
    expect(pageCssDims(page(600, 800, 270), 0.5)).toEqual({ w: 400, h: 300 });
  });
});

describe('pdfToPageCss / cssToPdf', () => {
  it('maps PDF bottom-left origin to CSS top-left at rotation 0', () => {
    expect(pdfToPageCss({ x: 100, y: 100 }, page(), 1)).toEqual({ x: 100, y: 700 });
    expect(pdfToPageCss({ x: 100, y: 100 }, page(), 2)).toEqual({ x: 200, y: 1400 });
    expect(cssToPdf({ x: 100, y: 700 }, page(), 1)).toEqual({ x: 100, y: 100 });
  });

  it('maps corners correctly at each rotation', () => {
    // PDF origin (bottom-left of the page)…
    expect(pdfToPageCss({ x: 0, y: 0 }, page(600, 800, 0), 1)).toEqual({ x: 0, y: 800 });
    // …appears at display top-left when rotated 90° CW
    expect(pdfToPageCss({ x: 0, y: 0 }, page(600, 800, 90), 1)).toEqual({ x: 0, y: 0 });
    // …at display top-right when rotated 180°
    expect(pdfToPageCss({ x: 0, y: 0 }, page(600, 800, 180), 1)).toEqual({ x: 600, y: 0 });
    // …at display bottom-right when rotated 270°
    expect(pdfToPageCss({ x: 0, y: 0 }, page(600, 800, 270), 1)).toEqual({ x: 800, y: 600 });
  });

  it.each([0, 90, 180, 270] as Rotation[])('round-trips at rotation %d and zoom 2.5', (rot) => {
    const p = page(612, 792, rot);
    const pdf = { x: 123.5, y: 456.25 };
    const css = pdfToPageCss(pdf, p, 2.5);
    const back = cssToPdf(css, p, 2.5);
    expect(back.x).toBeCloseTo(pdf.x, 6);
    expect(back.y).toBeCloseTo(pdf.y, 6);
  });

  it.each([0.5, 1, 2.5])('round-trips at zoom %f with rotation 90', (zoom) => {
    const p = page(600, 800, 90);
    const css = { x: 33.25, y: 71.5 };
    const pdf = cssToPdf(css, p, zoom);
    const back = pdfToPageCss(pdf, p, zoom);
    expect(back.x).toBeCloseTo(css.x, 6);
    expect(back.y).toBeCloseTo(css.y, 6);
  });
});

describe('crop-box origin conversions', () => {
  it('offsets between normalized page space and raw PDF user space', () => {
    const geom = { cropX: 10, cropY: 20 };
    expect(pageNormToPdfUser({ x: 0, y: 0 }, geom)).toEqual({ x: 10, y: 20 });
    expect(pdfUserToPageNorm({ x: 10, y: 20 }, geom)).toEqual({ x: 0, y: 0 });
    expect(pdfUserToPageNorm(pageNormToPdfUser({ x: 5, y: 7 }, geom), geom)).toEqual({
      x: 5,
      y: 7,
    });
  });
});

describe('pdfRectToCssRect', () => {
  it('converts and flips y at rotation 0', () => {
    expect(pdfRectToCssRect({ x: 50, y: 50, w: 100, h: 200 }, page(), 1)).toEqual({
      x: 50,
      y: 550,
      w: 100,
      h: 200,
    });
  });

  it('swaps extent at rotation 90', () => {
    const r = pdfRectToCssRect({ x: 50, y: 50, w: 100, h: 200 }, page(600, 800, 90), 1);
    expect(r).toEqual({ x: 50, y: 50, w: 200, h: 100 });
  });

  it('scales with zoom', () => {
    const r = pdfRectToCssRect({ x: 10, y: 10, w: 20, h: 30 }, page(), 2);
    expect(r).toEqual({ x: 20, y: 1520, w: 40, h: 60 });
  });
});

describe('bucketForScale', () => {
  it('snaps up to the nearest bucket and clamps at the extremes', () => {
    expect(RENDER_SCALE_BUCKETS[0]).toBe(0.25);
    expect(bucketForScale(0.2)).toBe(0.25);
    expect(bucketForScale(0.3)).toBe(0.5);
    expect(bucketForScale(1)).toBe(1);
    expect(bucketForScale(1.01)).toBe(1.25);
    expect(bucketForScale(5)).toBe(4);
  });
});

describe('tileGrid', () => {
  it('covers the full device area with clipped edge tiles', () => {
    const tiles = tileGrid(2500, 1800, 1024);
    expect(tiles).toHaveLength(6);
    expect(tiles[0]).toEqual({ x: 0, y: 0, w: 1024, h: 1024 });
    expect(tiles[tiles.length - 1]).toEqual({ x: 2048, y: 1024, w: 452, h: 776 });
    const area = tiles.reduce((s, t) => s + t.w * t.h, 0);
    expect(area).toBe(2500 * 1800);
  });

  it('returns one tile when the page fits', () => {
    expect(tileGrid(800, 600, 1024)).toEqual([{ x: 0, y: 0, w: 800, h: 600 }]);
  });
});

describe('fit zoom calculations', () => {
  it('fit-page uses the limiting axis of the rotated page', () => {
    expect(fitPageZoom(page(600, 800, 0), 1200, 900, 24)).toBeCloseTo(852 / 800, 6);
    expect(fitPageZoom(page(600, 800, 90), 1200, 900, 24)).toBeCloseTo(852 / 600, 6);
  });

  it('fit-width uses the rotated width', () => {
    expect(fitWidthZoom(page(600, 800, 0), 1200, 24)).toBeCloseTo(1152 / 600, 6);
    expect(fitWidthZoom(page(600, 800, 90), 1200, 24)).toBeCloseTo(1152 / 800, 6);
  });
});

describe('anchorScrollTop', () => {
  it('keeps the content under the anchor stable across a zoom change', () => {
    const pages = [page(), page(), page()];
    const oldLayout = layoutPages(pages, 1, { gap: 16, padding: 16, containerW: 1000 });
    const newLayout = layoutPages(pages, 2, { gap: 16, padding: 16, containerW: 1000 });
    // content point: oldScrollTop 300 + anchor 100 = 400 → page 0, fraction (400-16)/800
    const next = anchorScrollTop(oldLayout, newLayout, 300, 100);
    // new page0: top 16, cssH 1600 → content y = 16 + 0.48*1600 = 784 → scrollTop 684
    expect(next).toBeCloseTo(684, 6);
  });

  it('clamps into the nearest page when the anchor sits in a gap', () => {
    const pages = [page(), page()];
    const oldLayout = layoutPages(pages, 1, { gap: 16, padding: 16, containerW: 1000 });
    const newLayout = layoutPages(pages, 2, { gap: 16, padding: 16, containerW: 1000 });
    // anchor lands in the gap between pages (y = 820)
    const next = anchorScrollTop(oldLayout, newLayout, 800, 20);
    expect(Number.isFinite(next)).toBe(true);
    expect(next).toBeGreaterThanOrEqual(0);
  });
});

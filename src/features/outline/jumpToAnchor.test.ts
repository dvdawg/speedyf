import { describe, expect, it } from 'vitest';
import { figureTopY } from './jumpToAnchor';

/** Mirrors engine/preview.rs `rect_to_tile`, so the test proves the inverse
 * rather than restating the formula it is meant to undo. */
function rectToTileY(rectY: number, rectH: number, pageHeightPt: number, scaleMilli: number) {
  return Math.round((pageHeightPt - rectY - rectH) * (scaleMilli / 1000));
}

describe('figureTopY', () => {
  it('recovers the top edge the crop was built from', () => {
    // Artwork sitting from y=500 to y=700 on an 800pt page.
    const tileY = rectToTileY(500, 200, 800, 2000);
    expect(figureTopY(tileY, 2000, 800)).toBeCloseTo(700, 1);
  });

  it('lands above the caption, which is the whole point', () => {
    // Caption anchor at y=480; artwork above it, top at 700.
    const captionY = 480;
    const tileY = rectToTileY(500, 200, 800, 2000);
    expect(figureTopY(tileY, 2000, 800)!).toBeGreaterThan(captionY);
  });

  it('returns the page top for artwork flush with it', () => {
    expect(figureTopY(0, 2000, 800)).toBe(800);
  });

  it('refuses a crop that falls outside the page', () => {
    // Scrolling somewhere arbitrary is worse than staying on the caption.
    expect(figureTopY(999_999, 2000, 800)).toBeNull();
  });

  it('refuses nonsense inputs rather than dividing by zero', () => {
    expect(figureTopY(100, 0, 800)).toBeNull();
    expect(figureTopY(100, 2000, 0)).toBeNull();
  });

  it('is independent of the scale the crop was rendered at', () => {
    const atLow = figureTopY(rectToTileY(500, 200, 800, 1000), 1000, 800)!;
    const atHigh = figureTopY(rectToTileY(500, 200, 800, 4000), 4000, 800)!;
    expect(atLow).toBeCloseTo(atHigh, 1);
  });
});

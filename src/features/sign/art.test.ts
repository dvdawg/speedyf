import { describe, expect, it } from 'vitest';
import type { SignatureArt } from '../../types/model';
import {
  drawnArt,
  extendStroke,
  finishStroke,
  fitIntoField,
  placementRect,
  resizeKeepingAspect,
  strokeD,
} from './art';

const p = (x: number, y: number) => ({ x, y });
const typed = (w: number, h: number): SignatureArt => ({
  type: 'typed',
  w,
  h,
  text: 'Ada',
  style: 'flowing',
  d: 'M0 0Z',
});

describe('strokeD', () => {
  // The engine (stroke_segs) draws exactly this curve; the two must agree on
  // where each piece starts and ends.
  it('runs quadratics through midpoints and ends on the last point', () => {
    expect(strokeD([p(0, 0), p(10, 0), p(10, 10), p(20, 10)])).toBe('M0 0Q10 0 10 5Q10 10 20 10');
  });

  it('draws a dot and a straight two-point stroke', () => {
    expect(strokeD([p(1, 1)])).toBe('M1 1L1.01 1');
    expect(strokeD([p(1, 1), p(5, 5)])).toBe('M1 1L5 5');
    expect(strokeD([])).toBe('');
  });
});

describe('smoothing a stroke as it is drawn', () => {
  it('follows the pointer part of the way and drops jitter', () => {
    const stroke = [p(0, 0)];
    expect(extendStroke(stroke, p(10, 0))).toBe(true);
    expect(stroke[1]).toEqual(p(5, 0));
    // Barely moved: not worth a point.
    expect(extendStroke(stroke, p(5.5, 0))).toBe(false);
    expect(stroke).toHaveLength(2);
  });

  it('ends exactly where the pointer was lifted', () => {
    const stroke = [p(0, 0), p(5, 0)];
    finishStroke(stroke, p(10, 0));
    expect(stroke.at(-1)).toEqual(p(10, 0));
  });
});

describe('drawnArt', () => {
  it('crops the strokes to their ink with room for the pen', () => {
    const art = drawnArt([[p(100, 50), p(200, 80)], [p(150, 60)]], 2);
    expect(art).toEqual({
      type: 'drawn',
      w: 104,
      h: 34,
      width: 2,
      strokes: [[p(2, 2), p(102, 32)], [p(52, 12)]],
    });
    expect(drawnArt([], 2)).toBeNull();
  });
});

describe('placing', () => {
  const page = { widthPt: 612, heightPt: 792 };

  it('centres a signature on the click at a signature size, proportions kept', () => {
    const r = placementRect(typed(300, 60), 'signature', p(300, 400), page);
    expect(r.w).toBe(150);
    expect(r.h).toBe(30);
    expect(r.x + r.w / 2).toBe(300);
    expect(r.y + r.h / 2).toBe(400);
  });

  it('limits a tall mark by height and initials to a small box', () => {
    const tall = placementRect(typed(100, 100), 'signature', p(300, 400), page);
    expect(tall.h).toBe(42);
    expect(tall.w).toBe(42);
    const initials = placementRect(typed(200, 100), 'initials', p(300, 400), page);
    expect(initials.w).toBe(60);
    expect(initials.h).toBe(30);
  });

  it('keeps a signature placed near an edge on the page', () => {
    const r = placementRect(typed(300, 60), 'signature', p(5, 790), page);
    expect(r.x).toBe(0);
    expect(r.y + r.h).toBe(792);
  });

  it('fits a signature inside a field, centred and unstretched', () => {
    const field = { x: 100, y: 100, w: 200, h: 30 };
    const r = fitIntoField(typed(300, 60), field);
    expect(r.w / r.h).toBeCloseTo(5);
    expect(r.h).toBeCloseTo(24);
    expect(r.x).toBeGreaterThanOrEqual(field.x);
    expect(r.x + r.w).toBeLessThanOrEqual(field.x + field.w);
    expect(r.y + r.h / 2).toBeCloseTo(field.y + field.h / 2);
  });
});

describe('resizeKeepingAspect', () => {
  const r = { x: 100, y: 100, w: 100, h: 25 };

  it('keeps the opposite corner and the proportions', () => {
    // Drag the south-east corner right and up: width wins.
    const se = resizeKeepingAspect(r, 'se', p(300, 120), 4);
    expect(se).toEqual({ x: 100, y: 75, w: 200, h: 50 });
    // North-west, dragged mostly downward-left: the top-right stays.
    const nw = resizeKeepingAspect(r, 'nw', p(150, 200), 4);
    expect(nw.x + nw.w).toBe(200);
    expect(nw.y).toBe(100);
    expect(nw.w / nw.h).toBeCloseTo(4);
    expect(nw.h).toBe(100);
  });

  it('never shrinks below a usable size', () => {
    const tiny = resizeKeepingAspect(r, 'ne', p(90, 90), 4);
    expect(tiny.w).toBe(12);
    expect(tiny.h).toBe(3);
  });
});

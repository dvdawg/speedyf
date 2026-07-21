import { describe, expect, it } from 'vitest';
import { layoutPages, pageIndexAt, visibleRange } from './layout';
import type { Rotation } from '../../types/model';

const P = (w: number, h: number, rotation: Rotation = 0) => ({ widthPt: w, heightPt: h, rotation });

const opts = { gap: 16, padding: 16, containerW: 1000 };

describe('layoutPages', () => {
  it('stacks pages vertically with padding and gaps', () => {
    const l = layoutPages([P(600, 800), P(595, 842), P(800, 600, 90)], 1, opts);
    expect(l.tops).toEqual([16, 832, 1690]);
    expect(l.heights).toEqual([800, 842, 800]);
    expect(l.widths).toEqual([600, 595, 600]);
    expect(l.totalH).toBe(2506);
  });

  it('centers pages horizontally', () => {
    const l = layoutPages([P(600, 800), P(595, 842)], 1, opts);
    expect(l.lefts[0]).toBe(200);
    expect(l.lefts[1]).toBeCloseTo(202.5);
  });

  it('never centers into negative space at high zoom', () => {
    const l = layoutPages([P(600, 800)], 4, opts);
    expect(l.lefts[0]).toBe(16);
    expect(l.totalW).toBe(2432);
  });

  it('scales with zoom', () => {
    const l = layoutPages([P(600, 800)], 2, opts);
    expect(l.tops).toEqual([16]);
    expect(l.heights).toEqual([1600]);
    expect(l.totalH).toBe(1632);
  });

  it('handles an empty document', () => {
    const l = layoutPages([], 1, opts);
    expect(l.totalH).toBe(32);
    expect(l.tops).toEqual([]);
  });
});

describe('visibleRange', () => {
  const layout = layoutPages([P(600, 800), P(595, 842), P(800, 600, 90)], 1, opts);
  // page spans: [16..816], [832..1674], [1690..2490]

  it('returns the pages intersecting the viewport plus overscan', () => {
    expect(visibleRange(layout, 800, 600, 200)).toEqual([0, 1]);
    expect(visibleRange(layout, 800, 600, 800)).toEqual([0, 2]);
    expect(visibleRange(layout, 0, 100, 0)).toEqual([0, 0]);
  });

  it('clamps beyond the end of content', () => {
    expect(visibleRange(layout, 99999, 600, 100)).toEqual([2, 2]);
  });

  it('returns an empty range for an empty layout', () => {
    const empty = layoutPages([], 1, opts);
    expect(visibleRange(empty, 0, 600, 100)).toEqual([0, -1]);
  });
});

describe('pageIndexAt', () => {
  const layout = layoutPages([P(600, 800), P(595, 842), P(800, 600, 90)], 1, opts);

  it('finds the page containing a content-space y', () => {
    expect(pageIndexAt(layout, 20)).toBe(0);
    expect(pageIndexAt(layout, 1000)).toBe(1);
    expect(pageIndexAt(layout, 2489)).toBe(2);
  });

  it('snaps gap positions to the nearest following page', () => {
    expect(pageIndexAt(layout, 820)).toBe(1);
  });

  it('clamps out-of-range positions', () => {
    expect(pageIndexAt(layout, -5)).toBe(0);
    expect(pageIndexAt(layout, 99999)).toBe(2);
  });
});

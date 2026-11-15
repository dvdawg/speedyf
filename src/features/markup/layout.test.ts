import { describe, expect, it } from 'vitest';
import { paginate, placeNotes } from './layout';

const band = { top: 10, bottom: 500, gap: 4 };

function ends(tops: number[], heights: number[]) {
  return tops.map((t, i) => t + heights[i]!);
}

function noOverlap(tops: number[], heights: number[], gap: number) {
  return tops.every((t, i) => i === 0 || t >= tops[i - 1]! + heights[i - 1]! + gap - 1e-6);
}

describe('placeNotes', () => {
  it('puts each note level with its anchor when there is room', () => {
    expect(placeNotes([100, 300], [30, 30], band)).toEqual([100, 300]);
  });

  it('splits a collision between the notes, one up and one down', () => {
    const tops = placeNotes([100, 110], [30, 30], band);
    // Each moves 12 pt: the least squared displacement.
    expect(tops).toEqual([88, 122]);
    expect(noOverlap(tops, [30, 30], band.gap)).toBe(true);
  });

  it('keeps reading order and moves a crowd as little as it can', () => {
    const heights = [20, 20, 20, 20];
    const tops = placeNotes([200, 200, 200, 400], heights, band);
    expect(noOverlap(tops, heights, band.gap)).toBe(true);
    // The three that collide straddle 200; the fourth stays put.
    expect(tops[0]!).toBeLessThan(200);
    expect(tops[2]!).toBeGreaterThan(200);
    expect(tops[3]).toBe(400);
  });

  it('stays inside the band at both ends', () => {
    const heights = [40, 40, 40];
    const low = placeNotes([480, 490, 495], heights, band);
    expect(Math.max(...ends(low, heights))).toBeLessThanOrEqual(band.bottom);
    expect(noOverlap(low, heights, band.gap)).toBe(true);
    const high = placeNotes([0, 2, 4], heights, band);
    expect(high[0]).toBe(band.top);
    expect(noOverlap(high, heights, band.gap)).toBe(true);
  });

  it('stacks from the top when the notes cannot fit at all', () => {
    const heights = [300, 300];
    const tops = placeNotes([100, 200], heights, band);
    expect(tops).toEqual([10, 314]);
  });
});

describe('paginate', () => {
  it('fills pages in order', () => {
    const pages = paginate([{ height: 40 }, { height: 40 }, { height: 40 }], 100);
    expect(pages.map((p) => p.blocks)).toEqual([[0, 1], [2]]);
  });

  it('never leaves a heading at the foot of a page', () => {
    const pages = paginate(
      [{ height: 60 }, { height: 20, keepWithNext: true }, { height: 40 }],
      100
    );
    expect(pages.map((p) => p.blocks)).toEqual([[0], [1, 2]]);
  });

  it('repeats a table head over rows that run onto a new page', () => {
    const head = { height: 10, keepWithNext: true };
    const row = { height: 30, headerHeight: 10 };
    const pages = paginate([{ height: 30 }, head, row, row, row, row], 100);
    expect(pages.map((p) => [p.blocks, p.repeatHeader])).toEqual([
      [[0, 1, 2, 3], false],
      [[4, 5], true],
    ]);
  });

  it('gives a block taller than a page a page of its own', () => {
    const pages = paginate([{ height: 30 }, { height: 300 }, { height: 30 }], 100);
    expect(pages.map((p) => p.blocks)).toEqual([[0], [1], [2]]);
  });
});

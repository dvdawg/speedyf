import { describe, expect, it } from 'vitest';
import { layoutMargin, MARGIN, paginate, stackMargin, type MarginNote } from './layout';

const note = (id: string, desiredTop: number, full: number, short = 20, tiny = 10): MarginNote => ({
  id,
  desiredTop,
  full,
  short,
  tiny,
});

function overlaps(placed: { top: number; height: number }[], gap: number): boolean {
  const sorted = [...placed].sort((a, b) => a.top - b.top);
  return sorted.some(
    (p, i) => i > 0 && p.top < sorted[i - 1]!.top + sorted[i - 1]!.height + gap - 1e-6
  );
}

describe('stackMargin', () => {
  const box = { top: 10, bottom: 500, gap: 4 };

  it('puts each note level with its anchor when there is room', () => {
    const placed = stackMargin([note('a', 100, 30), note('b', 300, 30)], box);
    expect(placed.map((p) => [p.id, p.top, p.form])).toEqual([
      ['a', 100, 'full'],
      ['b', 300, 'full'],
    ]);
  });

  it('pushes a note down below the one before, never overlapping', () => {
    const placed = stackMargin([note('b', 105, 30), note('a', 100, 30), note('c', 110, 30)], box);
    expect(placed.map((p) => p.id)).toEqual(['a', 'b', 'c']);
    expect(placed.map((p) => p.top)).toEqual([100, 134, 168]);
    expect(overlaps(placed, box.gap)).toBe(false);
  });

  it('pulls notes back up so the last one ends on the page', () => {
    const placed = stackMargin([note('a', 400, 60), note('b', 480, 60)], box);
    expect(placed.at(-1)!.top + placed.at(-1)!.height).toBeLessThanOrEqual(box.bottom);
    expect(overlaps(placed, box.gap)).toBe(false);
    expect(placed.every((p) => p.form === 'full')).toBe(true);
  });

  it('shortens the longest notes first when they cannot all fit', () => {
    const placed = stackMargin([note('a', 20, 100), note('b', 140, 300), note('c', 450, 120)], box);
    const form = Object.fromEntries(placed.map((p) => [p.id, p.form]));
    expect(form).toEqual({ a: 'full', b: 'short', c: 'full' });
    expect(overlaps(placed, box.gap)).toBe(false);
    expect(placed.at(-1)!.top + placed.at(-1)!.height).toBeLessThanOrEqual(box.bottom);
  });

  it('goes down to the smallest form when shortening is not enough', () => {
    const notes = Array.from({ length: 30 }, (_, i) => note(`n${i}`, i * 10, 40, 22, 10));
    const placed = stackMargin(notes, box);
    expect(placed.some((p) => p.form === 'tiny')).toBe(true);
    expect(overlaps(placed, box.gap)).toBe(false);
    expect(placed.at(-1)!.top + placed.at(-1)!.height).toBeLessThanOrEqual(box.bottom);
  });

  it('keeps the margin frame of a page', () => {
    const placed = layoutMargin([note('a', 0, 20)], 792);
    expect(placed[0]!.top).toBe(MARGIN.top);
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

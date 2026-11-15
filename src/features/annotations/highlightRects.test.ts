import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import { highlightRects } from './highlightRects';

const run = (text: string, x: number, y: number, w: number, h = 10): TextRunDto => ({
  text,
  start: 0,
  x,
  y,
  w,
  h,
});

const runs = [
  run('we have', 60, 700, 200),
  run('a + b', 140, 680, 30),
  run('c', 152, 668, 6),
  run('(3)', 480, 674, 14),
];
const display = { x: 138, y: 667, w: 34, h: 24, number: '3' };

describe('highlightRects', () => {
  it('marks the crossed part of each run', () => {
    expect(highlightRects(runs, { x: 100, y: 695, w: 50, h: 20 })).toEqual([
      { x: 100, y: 700, w: 50, h: 10 },
    ]);
  });

  it('marks a display equation it touches as one box', () => {
    // over the numerator only
    const rects = highlightRects(runs, { x: 130, y: 678, w: 50, h: 14 }, [display]);
    expect(rects).toEqual([{ x: 138, y: 667, w: 356, h: 24 }]);
  });

  it('marks the runs one by one without the equation boxes', () => {
    expect(highlightRects(runs, { x: 130, y: 660, w: 50, h: 34 })).toHaveLength(2);
  });
});

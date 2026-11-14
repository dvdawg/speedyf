import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import { paragraphRunRange } from './paragraph';

/** One run, its bottom at `y` (page points, y up). */
function run(text: string, x: number, y: number, w = 200, h = 10): TextRunDto {
  return { text, start: 0, x, y, w, h };
}

/** Body lines 12pt apart, each line two runs (a word gap between them). */
function body(top: number, count: number, x = 60, width = 240, indentFirst = false): TextRunDto[] {
  const out: TextRunDto[] = [];
  for (let i = 0; i < count; i += 1) {
    const left = indentFirst && i === 0 ? x + 10 : x;
    const y = top - i * 12;
    out.push(run(`line ${i} a`, left, y, 100));
    out.push(run(`line ${i} b`, left + 104, y, width - 104 - (left - x)));
  }
  return out;
}

describe('paragraphRunRange', () => {
  it('selects the lines of one paragraph and stops at a paragraph gap', () => {
    const first = body(700, 3);
    const second = body(700 - 3 * 12 - 12, 4);
    const runs = [...first, ...second];
    expect(paragraphRunRange(runs, 2)).toEqual([0, 5]);
    expect(paragraphRunRange(runs, 9)).toEqual([6, 13]);
  });

  it('stops at an indented first line with no gap above it', () => {
    const first = body(700, 3);
    const second = body(700 - 3 * 12, 3, 60, 240, true);
    const runs = [...first, ...second];
    expect(paragraphRunRange(runs, 0)).toEqual([0, 5]);
    expect(paragraphRunRange(runs, 11)).toEqual([6, 11]);
  });

  it('keeps to one column of a two-column page', () => {
    const left = body(700, 4, 50, 240);
    const right = body(700, 4, 320, 240);
    const runs = [...left, ...right];
    expect(paragraphRunRange(runs, 7)).toEqual([0, 7]);
    expect(paragraphRunRange(runs, 8)).toEqual([8, 15]);
  });

  it('splits two columns read line by line on one baseline', () => {
    // Runs interleaved across the gutter, as some producers write them.
    const runs = [
      run('left 0', 50, 700, 240),
      run('right 0', 320, 700, 240),
      run('left 1', 50, 688, 240),
      run('right 1', 320, 688, 240),
    ];
    expect(paragraphRunRange(runs, 0)).toEqual([0, 0]);
    expect(paragraphRunRange(runs, 1)).toEqual([1, 1]);
  });

  it('does not run a heading into the paragraph under it', () => {
    const heading = run('2 Method', 60, 720, 120, 14);
    const paragraph = body(704, 3);
    const runs = [heading, ...paragraph];
    expect(paragraphRunRange(runs, 0)).toEqual([0, 0]);
    expect(paragraphRunRange(runs, 3)).toEqual([1, 6]);
  });

  it('selects a single-line caption alone', () => {
    const above = body(700, 3);
    const caption = run('Figure 1: A caption.', 60, 600, 180, 9);
    const below = body(580, 3);
    const runs = [...above, caption, ...below];
    expect(paragraphRunRange(runs, 6)).toEqual([6, 6]);
    expect(paragraphRunRange(runs, 8)).toEqual([7, 12]);
  });

  it('answers null outside the runs', () => {
    expect(paragraphRunRange([], 0)).toBeNull();
    expect(paragraphRunRange(body(700, 1), 5)).toBeNull();
  });
});

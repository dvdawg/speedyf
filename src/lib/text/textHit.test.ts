import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import {
  buildTextModel,
  displayEdge,
  displaysOf,
  hitTest,
  selectionRects,
  wordAt,
  type DisplayBox,
} from './textHit';

/** One run, its bottom at `y` (page points, y up). */
function run(text: string, x: number, y: number, w: number, h = 10): TextRunDto {
  return { text, start: 0, x, y, w, h };
}

/** A two-column page: each column three lines 12pt apart, the left column's
 * last line short. Left column x 60..260, right 300..500. */
function twoColumns(): TextRunDto[] {
  return [
    run('left one', 60, 700, 200),
    run('left two', 60, 688, 200),
    run('left end', 60, 676, 80),
    run('right one', 300, 700, 200),
    run('right two', 300, 688, 200),
    run('right three', 300, 676, 200),
  ];
}

describe('hitTest', () => {
  it('finds the character boundary nearest the pointer', () => {
    // ten characters over 100pt: 10pt each
    const model = buildTextModel([run('abcdefghij', 0, 100, 100)]);
    expect(hitTest(model, { x: 34, y: 105 })).toEqual({ run: 0, offset: 3 });
    expect(hitTest(model, { x: 36, y: 105 })).toEqual({ run: 0, offset: 4 });
  });

  it('places character boundaries by measured width', () => {
    // "W" is three times as wide as "i": 30pt then 10pt
    const model = buildTextModel([run('Wi', 0, 100, 40)], (t) => (t === 'W' ? 3 : 1));
    expect(model.edges(0)).toEqual([0, 30, 40]);
    expect(hitTest(model, { x: 29, y: 105 })).toEqual({ run: 0, offset: 1 });
  });

  it('takes the line nearest a pointer between lines, not the page end', () => {
    const model = buildTextModel(twoColumns());
    // Between "left one" (700..710) and "left two" (688..698), nearer the
    // upper: its end, when right of it.
    expect(hitTest(model, { x: 250, y: 699.5 })).toEqual({ run: 0, offset: 8 });
    // Well below the text, the last line of the column.
    expect(hitTest(model, { x: 100, y: 200 })?.run).toBe(2);
  });

  it('gives the margin of a short line to that line', () => {
    const model = buildTextModel(twoColumns());
    // Right of "left end" (ends at 140) but inside the column: its end, not
    // the line above.
    expect(hitTest(model, { x: 230, y: 681 })).toEqual({ run: 2, offset: 8 });
  });

  it('keeps a drag in the gutter with the column it started in', () => {
    const model = buildTextModel(twoColumns());
    const left = { minX: 60, maxX: 260 };
    const right = { minX: 300, maxX: 500 };
    // 285 is nearer the right column (15pt) than the left (25pt); 275 the
    // left (15pt against 25pt).
    expect(hitTest(model, { x: 285, y: 693 })?.run).toBe(4);
    expect(hitTest(model, { x: 285, y: 693 }, left)).toEqual({ run: 1, offset: 8 });
    expect(hitTest(model, { x: 275, y: 693 }, right)).toEqual({ run: 4, offset: 0 });
    // Well inside the other column, it is that column.
    expect(hitTest(model, { x: 400, y: 693 }, left)?.run).toBe(4);
  });

  it('takes the gap between two runs as the start of the next', () => {
    const model = buildTextModel([run('one', 0, 100, 30), run('two', 40, 100, 30)]);
    expect(hitTest(model, { x: 31, y: 105 })).toEqual({ run: 1, offset: 0 });
    expect(hitTest(model, { x: 38, y: 105 })).toEqual({ run: 1, offset: 0 });
  });

  it('never splits a surrogate pair', () => {
    // a math italic x (two UTF-16 units) then "y"
    const model = buildTextModel([run('\u{1D465}y', 0, 100, 20)]);
    const at = hitTest(model, { x: 9, y: 105 })!;
    expect([0, 2]).toContain(at.offset);
  });

  it('answers nothing for a page without text', () => {
    expect(hitTest(buildTextModel([]), { x: 0, y: 0 })).toBeNull();
  });
});

describe('wordAt', () => {
  it('selects the word under the offset', () => {
    expect(wordAt('the quick fox', 5)).toEqual([4, 9]);
    expect(wordAt('the quick fox', 9)).toEqual([4, 9]);
    expect(wordAt("don't", 1)).toEqual([0, 5]);
  });

  it('selects one character off a word', () => {
    expect(wordAt('a (b)', 2)).toEqual([2, 3]);
  });
});

describe('selectionRects', () => {
  it('paints one band per line across the word gaps', () => {
    const runs = [
      run('alpha', 60, 700, 50, 7),
      run('Beta', 114, 699, 40, 10),
      run('gamma', 60, 688, 50, 9),
    ];
    const model = buildTextModel(runs);
    const rects = selectionRects(model, [
      { run: 0, from: 0, to: 5 },
      { run: 1, from: 0, to: 4 },
      { run: 2, from: 0, to: 2 },
    ]);
    expect(rects).toHaveLength(2);
    const first = rects[0]!;
    const second = rects[1]!;
    // the first line spans both runs, gap included, at one height
    expect(first[0]).toBe(60);
    expect(first[2]).toBe(94);
    // half way to the next line's ink at most, so bands do not overlap
    expect(first[1]).toBeGreaterThanOrEqual(second[1]! + second[3]!);
    // the second covers two of five characters
    expect(second[2]).toBeCloseTo(20);
  });

  it('starts a new band where a selection skips a run', () => {
    const model = buildTextModel([
      run('a', 0, 100, 10),
      run('b', 12, 100, 10),
      run('c', 24, 100, 10),
    ]);
    expect(
      selectionRects(model, [
        { run: 0, from: 0, to: 1 },
        { run: 2, from: 0, to: 1 },
      ])
    ).toHaveLength(2);
  });
});

/** Prose, a two-row display (a fraction) numbered (3), more prose. */
function withDisplay(): { runs: TextRunDto[]; box: DisplayBox } {
  return {
    runs: [
      run('we have', 60, 700, 200),
      run('a + b', 140, 680, 30),
      run('c', 152, 668, 6),
      run('(3)', 480, 674, 14),
      run('and so on', 60, 650, 200),
    ],
    box: { x: 138, y: 667, w: 34, h: 24, number: '3' },
  };
}

describe('display equations', () => {
  it('takes the runs inside the box and the number beside it', () => {
    const { runs, box } = withDisplay();
    const [display] = displaysOf(runs, [box]);
    expect(display).toMatchObject({ first: 1, last: 3 });
    // one box around the rows and the number
    expect(display!.box[0]).toBe(138);
    expect(display!.box[0] + display!.box[2]).toBe(494);
  });

  it('leaves a display whose runs are interleaved with prose', () => {
    const { runs, box } = withDisplay();
    const mixed = [runs[1]!, runs[0]!, runs[2]!];
    expect(displaysOf(mixed, [box])).toEqual([]);
  });

  it('moves a selection end in a display out to its edge', () => {
    const { runs, box } = withDisplay();
    const model = buildTextModel(runs, undefined, [box]);
    expect(displayEdge(model, { run: 2, offset: 0 }, true)).toEqual({ run: 3, offset: 3 });
    expect(displayEdge(model, { run: 2, offset: 1 }, false)).toEqual({ run: 1, offset: 0 });
    expect(displayEdge(model, { run: 0, offset: 2 }, true)).toEqual({ run: 0, offset: 2 });
  });

  it('paints a display as one box', () => {
    const { runs, box } = withDisplay();
    const model = buildTextModel(runs, undefined, [box]);
    const rects = selectionRects(model, [
      { run: 0, from: 3, to: 7 },
      { run: 1, from: 0, to: 5 },
      { run: 2, from: 0, to: 1 },
      { run: 3, from: 0, to: 3 },
      { run: 4, from: 0, to: 3 },
    ]);
    expect(rects).toHaveLength(3);
    expect(rects[1]).toEqual([136, 665, 360, 28]);
  });
});

/** "Wi\ufb01 in" in a serif face, as the engine places it: glyph advances of
 * different widths ("W" wide, "i" narrow), a ligature as one character, an
 * inkless word space, and a run whose ink starts a little right of its
 * first advance. Edges from `x` (60). */
function serifRun(): TextRunDto {
  return {
    text: 'Wi\ufb01 in',
    start: 0,
    x: 60,
    y: 700,
    w: 38,
    h: 9,
    // W 0..9.4, i 9.4..12.2, fi 12.2..18.3, space 18.3..20.8, i 20.8..23.6, n 23.6..28.6
    edges: [0, 9.4, 12.2, 18.3, 20.8, 23.6, 28.6],
  };
}

describe('hitTest on the engine glyph edges', () => {
  it('includes a glyph pressed at its left edge', () => {
    const model = buildTextModel([serifRun()]);
    // the left edge of "W", and just inside the ligature's left half
    expect(hitTest(model, { x: 60.2, y: 704 })).toEqual({ run: 0, offset: 0 });
    expect(hitTest(model, { x: 72.5, y: 704 })).toEqual({ run: 0, offset: 2 });
  });

  it('takes the boundary after a glyph pressed in its right half', () => {
    const model = buildTextModel([serifRun()]);
    // the right half of the narrow "i" (69.4..72.2)
    expect(hitTest(model, { x: 71.5, y: 704 })).toEqual({ run: 0, offset: 2 });
    // the right half of the ligature: after it, never inside it
    expect(hitTest(model, { x: 77.5, y: 704 })).toEqual({ run: 0, offset: 3 });
  });

  it('starts at the next word from anywhere in the space before it', () => {
    const model = buildTextModel([serifRun()]);
    // the space is 78.3..80.8: both halves begin the word "in"
    expect(hitTest(model, { x: 78.5, y: 704 })).toEqual({ run: 0, offset: 4 });
    expect(hitTest(model, { x: 80.6, y: 704 })).toEqual({ run: 0, offset: 4 });
  });

  it('paints exactly the characters a press selects', () => {
    const model = buildTextModel([serifRun()]);
    const [band] = selectionRects(model, [{ run: 0, from: 4, to: 6 }]);
    expect(band![0]).toBeCloseTo(80.8);
    expect(band![2]).toBeCloseTo(7.8);
  });

  it('starts at the first letter after a leading space', () => {
    const model = buildTextModel([
      { text: ' word', start: 0, x: 100, y: 700, w: 30, h: 9, edges: [0, 3, 9, 15, 20, 26] },
    ]);
    expect(hitTest(model, { x: 101, y: 704 })).toEqual({ run: 0, offset: 1 });
    expect(hitTest(model, { x: 103.5, y: 704 })).toEqual({ run: 0, offset: 1 });
  });

  it('falls back to an estimate when the edges do not fit the text', () => {
    const model = buildTextModel([{ ...serifRun(), edges: [0, 1] }]);
    expect(model.edges(0)).toHaveLength(7);
    expect(model.edges(0)[0]).toBe(60);
  });
});

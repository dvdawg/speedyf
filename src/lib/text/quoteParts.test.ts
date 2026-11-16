import { describe, expect, it } from 'vitest';
import type { QuotePiece } from './copySelection';
import { fillQuote, mathChunks, oneFormula, planQuote, prosy } from './quoteParts';

/** The stretches mathChunks finds, as text. */
const chunks = (text: string, math: [number, number][]) =>
  mathChunks(text, math).map(([a, b]) => text.slice(a, b));

/** Where `words` occur in `text`, as math stretches. */
const mathAt = (text: string, ...words: string[]): [number, number][] =>
  words.map((w) => {
    const at = text.indexOf(w);
    return [at, at + w.length];
  });

describe('mathChunks', () => {
  it('takes a formula with the operators and numbers that bind it', () => {
    const text = 'where x = 0 for all n, and so';
    expect(chunks(text, mathAt(text, 'x', 'n'))).toEqual(['x = 0', 'n']);
  });

  it('leaves numbers beside a formula, and the sentence punctuation, to the prose', () => {
    const text = 'see (3), x is small.';
    expect(chunks(text, mathAt(text, 'x'))).toEqual(['x']);
    const end = 'and so on for y.';
    expect(chunks(end, mathAt(end, 'y'))).toEqual(['y']);
  });

  it('closes a bracket the formula opens', () => {
    const text = 'every d in (0, 1), every';
    // "in" is a word: the formula is "d" alone
    expect(chunks(text, mathAt(text, 'd'))).toEqual(['d']);
    const symbols = 'every \u03b4 \u2208 (0, 1), every';
    expect(chunks(symbols, [])).toEqual(['\u03b4 \u2208 (0, 1)']);
  });

  it('reads function names among symbols as the formula', () => {
    const text = 'Choose q = \u2308log(1/\u03b4)\u2309. Then';
    expect(chunks(text, mathAt(text, 'q'))).toEqual(['q = \u2308log(1/\u03b4)\u2309']);
  });

  it('never takes a word', () => {
    const text = 'the x-axis and log of it';
    expect(chunks(text, mathAt(text, 'x'))).toEqual([]);
  });
});

/** One run of a line, its `math` stretches in a math font. */
function piece(
  text: string,
  x: number,
  y: number,
  math: [number, number][] = [],
  w = text.length * 5,
  h = 10
): QuotePiece {
  return {
    run: { text, start: x, x, y, w, h, page: 0, math },
    from: 0,
    to: text.length,
    display: null,
  };
}

describe('planQuote', () => {
  it('takes a formula the extractor split into two runs whole', () => {
    const plan = planQuote([
      piece('every \u03b4 \u2208 (0,', 60, 700),
      piece('1), every P', 130, 700, [[10, 11]]),
    ]);
    expect(plan.inline.map((p) => p.text)).toEqual(['\u03b4 \u2208 (0, 1)', 'P']);
    expect(plan.inline[0]!.rects).toHaveLength(2);
    expect(plan.plain).toBe('every \u03b4 \u2208 (0, 1), every P');
  });

  it('stands a display in by its box and number', () => {
    const display = { box: [100, 650, 200, 20], block: [100, 650, 180, 20], number: '3' } as const;
    const plan = planQuote([
      piece('we have', 60, 700),
      { ...piece('a + b = c', 120, 655), display },
      { ...piece('(3)', 290, 655), display },
      piece('so it holds', 60, 620),
    ]);
    expect(plan.displays).toMatchObject([{ text: 'a + b = c (3)', number: '3' }]);
    expect(plan.displays[0]!.block).toEqual([100, 650, 180, 20]);
  });
});

describe('fillQuote', () => {
  const plan = planQuote([
    piece('as x and y_1 hold', 60, 700, [
      [3, 4],
      [9, 12],
    ]),
  ]);
  const [x, y] = plan.inline;

  it('typesets what was read and sets the rest as its text', () => {
    const tex = new Map([
      [x!.key, 'x'],
      [y!.key, null],
    ]);
    expect(fillQuote(plan, tex)).toBe('as $x$ and *y\\_1* hold');
  });

  it('drops a display number its tag already says', () => {
    const display = { box: [100, 650, 200, 20], block: [100, 650, 180, 20], number: '3' } as const;
    const withDisplay = planQuote([
      { ...piece('a + b', 120, 655), display },
      piece('(3) so', 300, 655),
    ]);
    const key = withDisplay.displays[0]!.key;
    expect(fillQuote(withDisplay, new Map([[key, 'a + b \\tag{3}']]))).toBe(
      '$$ a + b \\tag{3} $$ so'
    );
  });

  it('writes a raised index on the formula before it', () => {
    const scripted = planQuote([
      piece('Here h', 60, 700, [[5, 6]]),
      piece('(n)', 92, 709, [[1, 2]], 10, 6),
    ]);
    const [h, n] = scripted.inline;
    const tex = new Map([
      [h!.key, 'h'],
      [n!.key, '(n)'],
    ]);
    expect(fillQuote(scripted, tex)).toBe('Here $h^{(n)}$');
  });
});

describe('oneFormula', () => {
  it('joins a reading whose pieces only punctuation parts', () => {
    expect(oneFormula('$\\delta \\in (0$, 1)')).toBe('\\delta \\in (0, 1)');
    expect(oneFormula('$$ a $$')).toBe('a');
    expect(oneFormula('$x$ is')).toBeNull();
    expect(oneFormula('plain')).toBeNull();
  });
});

describe('prosy', () => {
  it('tells a table or a caption from a formula', () => {
    expect(prosy('P errP Maj(f1, f2, f3) > e exp(-q)')).toBe(false);
    expect(prosy('Experiment Opt. Carnot Geodesic Hybrid')).toBe(true);
    expect(prosy('m 0.545 pg 80.9 mg 7.51 T0 300')).toBe(true);
  });
});

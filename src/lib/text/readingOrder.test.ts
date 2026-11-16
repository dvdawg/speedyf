/** A drag across two columns, as the text layer resolves it: the caret at
 * each end from `hitTest`, the runs between them in layer order, the text
 * from `smartCopyText`. The layer's order is the engine's reading order
 * (`reading_order.rs`); in the order an interleaving PDF draws its runs,
 * the same drag takes half the page. */
import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import { paragraphRunRange } from './paragraph';
import { smartCopyText } from './smartCopy';
import { buildTextModel, hitTest, type PagePoint } from './textHit';

const LINE = 12;

function run(text: string, start: number, x: number, y: number): TextRunDto {
  return { text, start, x, y, w: 240, h: 9 };
}

/** Two columns of `lines` lines, x=54 and x=306, from y=700 down, drawn
 * a line of each in turn, as some PDFs draw them. */
function interleaved(lines: number): TextRunDto[] {
  const out: TextRunDto[] = [];
  for (let k = 0; k < lines; k += 1) {
    const y = 700 - LINE * k;
    out.push(run(`left${k}`, 20 * k, 54, y), run(`right${k}`, 20 * k + 10, 306, y));
  }
  return out;
}

/** The same runs as the engine sends them: a column, then the next. */
const readingOrder = (runs: TextRunDto[]) => [
  ...runs.filter((r) => r.x < 300),
  ...runs.filter((r) => r.x >= 300),
];

function drag(runs: TextRunDto[], from: PagePoint, to: PagePoint): string {
  const model = buildTextModel(runs);
  const a = hitTest(model, from)!;
  const b = hitTest(model, to)!;
  const [s, e] = a.run <= b.run ? [a, b] : [b, a];
  const pieces = [];
  for (let r = s.run; r <= e.run; r += 1) {
    const text = runs[r]!.text.slice(
      r === s.run ? s.offset : 0,
      r === e.run ? e.offset : undefined
    );
    if (text) pieces.push({ ...runs[r]!, text, page: 0, order: r });
  }
  return smartCopyText(pieces);
}

describe('a drag from the foot of one column to the head of the next', () => {
  // from the start of left17 to the end of right1
  const from = { x: 54, y: 700 - LINE * 17 + 4 };
  const to = { x: 560, y: 700 - LINE + 4 };

  it('takes the left column tail, then the right column head', () => {
    expect(drag(readingOrder(interleaved(20)), from, to)).toBe(
      'left17 left18 left19 right0 right1'
    );
  });

  it('took the lines between in the order the PDF drew them', () => {
    // the regression this guards: everything drawn between the two ends
    expect(drag(interleaved(20), from, to).split(' ').length).toBeGreaterThan(5);
  });
});

describe('a triple-click', () => {
  it('selects the paragraph in its own column', () => {
    const runs = readingOrder(interleaved(6));
    const at = runs.findIndex((r) => r.text === 'right2');
    const range = paragraphRunRange(runs, at)!;
    const texts = runs.slice(range[0], range[1] + 1).map((r) => r.text);
    expect(texts).toEqual(['right0', 'right1', 'right2', 'right3', 'right4', 'right5']);
  });
});

/** Which statement a selection quotes, cut out of what surrounds it, and
 * the cross-references in it. */
import { describe, expect, it } from 'vitest';
import type { Statement } from './paperDocs';
import { splitStatementRects, splitStatementText, statementAt, xrefsIn } from './selectionContext';
import type { HoverSpan } from '../hovers/hoverTypes';

/** An 800pt page; Proposition 2.2's label on the line at top-left y 210,
 * the statement from 207 to 300. */
const PROP: Statement = {
  kind: 'proposition',
  word: 'Proposition',
  number: '2.2',
  title: 'Moments of two block overlaps',
  page: 0,
  labelRects: [[100, 580, 90, 10]],
  region: [97, 500, 400, 93],
  text: 'Proposition 2.2 (Moments of two block overlaps). Let P be a distribution on the cube. Then the moments agree.',
};
const H = 800;
const line = (top: number, x = 100, w = 380): [number, number, number, number] => [x, top, w, 10];

describe('the statement a selection quotes', () => {
  it('is the one whose label it covers', () => {
    expect(statementAt({ rects: [line(180), line(195), line(210)] }, [PROP], H)?.statement).toBe(
      PROP
    );
  });

  it('is the one whose label line it starts on, though it missed the label', () => {
    const hit = statementAt({ rects: [line(210, 300, 180), line(225)] }, [PROP], H);
    expect(hit?.box).toEqual([97, 207, 400, 93]);
    expect(hit?.labelBox).toEqual([100, 210, 90, 10]);
  });

  it('is the one it starts inside and runs to the end of', () => {
    expect(statementAt({ rects: [line(240), line(255), line(285)] }, [PROP], H)).not.toBeNull();
    // A sentence from its middle is a plain quote.
    expect(statementAt({ rects: [line(240), line(255)] }, [PROP], H)).toBeNull();
    // An equation is never a statement.
    expect(statementAt({ rects: [line(210)] }, [{ ...PROP, kind: 'equation' }], H)).toBeNull();
  });
});

describe('cutting a selection around its statement', () => {
  it('finds the statement in the text, label or not, and what follows it', () => {
    const text =
      'as follows. Proposition 2.2 (Moments of two block overlaps). Let P be a distribution on the cube. Then the moments agree.\n\nBy Theorem 2.1 it holds.';
    expect(splitStatementText(text, PROP.text)).toEqual({
      before: 'as follows.',
      statement:
        'Proposition 2.2 (Moments of two block overlaps). Let P be a distribution on the cube. Then the moments agree.',
      after: 'By Theorem 2.1 it holds.',
    });
    const missed = 'Let P be a distribution on the cube. Then the moments agree. Next.';
    expect(splitStatementText(missed, PROP.text)).toEqual({
      before: '',
      statement: 'Let P be a distribution on the cube. Then the moments agree.',
      after: 'Next.',
    });
  });

  it('splits the rectangles by the statement extent', () => {
    const parts = splitStatementRects(
      [line(190), line(210), line(285), line(320)],
      [97, 207, 400, 93]
    );
    expect(parts.before).toEqual([line(190)]);
    expect(parts.statement).toEqual([line(210), line(285)]);
    expect(parts.after).toEqual([line(320)]);
  });
});

describe('cross-references in a selection', () => {
  it('lists the resolved spans it covers, with their targets top-left', () => {
    const span = (label: string, kind: HoverSpan['kind'], y: number): HoverSpan => ({
      kind,
      rects: [[120, y, 50, 10]],
      label,
      target: { page: 2, x: 72, y: 700, region: [72, 600, 400, 100], continues: false, terms: [] },
    });
    const spans = [
      span('Theorem 2.1', 'theorem', 580),
      span('Footnote 3', 'footnote', 580),
      span('theta_t', 'symbol', 580),
      span('Equation 12', 'equation', 100),
    ];
    expect(xrefsIn({ rects: [line(210)] }, spans, H, () => 800)).toEqual([
      { label: 'Theorem 2.1', kind: 'theorem', number: '2.1', page: 2, box: [72, 100, 400, 100] },
    ]);
  });
});

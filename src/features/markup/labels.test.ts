import { describe, expect, it } from 'vitest';
import { quadsFromRects } from './geometry';
import {
  equationAt,
  labelFor,
  labelText,
  proofStarts,
  sectionAt,
  shortSection,
  statementAt,
  type Heading,
  type LabelContext,
  type StatementLike,
} from './labels';
import type { Anchor } from './types';

const H = 800;

/** Text at `top` points down page `page`. */
function at(page: number, top: number, x = 100): Anchor {
  return { page, quads: quadsFromRects([[x, top, 50, 10]], H), rect: null, quote: 'x' };
}

const headings: Heading[] = [
  { depth: 0, label: '1 Introduction', page: 0, y: 700 },
  { depth: 0, label: '2 Results', page: 1, y: 600 },
  { depth: 1, label: '2.1 Setup', page: 1, y: 400 },
  { depth: 0, label: 'Appendix A: Proofs', page: 4, y: Number.POSITIVE_INFINITY },
];

const lemma: StatementLike = {
  kind: 'lemma',
  word: 'Lemma',
  number: '2.3',
  page: 2,
  // y-up: from 500 to 600 (top 200..300 from the page top)
  region: [72, 500, 460, 100],
};
const theorem: StatementLike = {
  kind: 'theorem',
  word: 'Theorem',
  number: '3.1',
  page: 3,
  region: [72, 650, 460, 60],
};
const equation: StatementLike = {
  kind: 'equation',
  word: 'Equation',
  number: '12',
  page: 2,
  region: [150, 300, 300, 30],
};

const ctx: LabelContext = {
  headings,
  statements: [lemma, equation, theorem],
  // "Proof." right under Theorem 3.1, at 630 on page 3.
  proofs: [{ page: 3, y: 630 }],
};

describe('sections', () => {
  it('shortens numbered headings and keeps the rest as titles', () => {
    expect(shortSection('2.1 Setup')).toBe('Sec. 2.1');
    expect(shortSection('3. Main result')).toBe('Sec. 3');
    expect(shortSection('A.2 Technical lemmas')).toBe('Sec. A.2');
    expect(shortSection('A note on notation')).toBe('A note on notation');
    expect(shortSection('Introduction')).toBe('Introduction');
    expect(shortSection('An unreasonably long heading that goes on')).toBe(
      'An unreasonably long...'
    );
  });

  it('finds the deepest heading above a point', () => {
    expect(sectionAt(headings, 0, 800)).toBeNull();
    expect(sectionAt(headings, 0, 500)?.title).toBe('1 Introduction');
    expect(sectionAt(headings, 1, 500)?.title).toBe('2 Results');
    expect(sectionAt(headings, 1, 300)?.short).toBe('Sec. 2.1');
    expect(sectionAt(headings, 3, 300)?.title).toBe('2.1 Setup');
    // A new section closes the subsection before it.
    expect(sectionAt(headings, 4, 10)?.title).toBe('Appendix A: Proofs');
  });
});

describe('statements', () => {
  it('names the statement a point lies in', () => {
    expect(statementAt(ctx, 2, 200, 550)).toBe('Lemma 2.3');
    expect(statementAt(ctx, 2, 200, 450)).toBeNull();
  });

  it('names the proof a point lies in, until something else begins', () => {
    expect(statementAt(ctx, 3, 200, 500)).toBe('proof of Theorem 3.1');
    // Still the proof on the next page.
    expect(statementAt(ctx, 4, 200, 10)).toBeNull(); // the appendix heading ends it
    const longer: LabelContext = { ...ctx, headings: headings.slice(0, 3) };
    expect(statementAt(longer, 4, 200, 700)).toBe('proof of Theorem 3.1');
    // Before the proof starts is not in it.
    expect(statementAt(ctx, 3, 200, 640)).toBeNull();
  });

  it('labels the numbered equation an anchor sits in', () => {
    expect(equationAt(ctx, at(2, 485, 200))).toBe('eq. (12)');
    expect(equationAt(ctx, at(2, 300, 200))).toBeNull();
  });
});

describe('labelFor', () => {
  it('puts page, section, statement and equation together', () => {
    const label = labelFor(at(2, 250, 200), ctx);
    expect(label.page).toBe(3);
    expect(label.section?.short).toBe('Sec. 2.1');
    expect(label.statement).toBe('Lemma 2.3');
    expect(labelText(label)).toBe('p. 3 - Sec. 2.1 - Lemma 2.3');
    expect(labelText(label, false)).toBe('p. 3 - Lemma 2.3');
  });
});

describe('proofStarts', () => {
  it('finds lines opening with Proof, and not the word inside a line', () => {
    const runs = [
      { text: 'Proof.', x: 72, y: 600, w: 30, h: 10 },
      { text: 'Let x be given.', x: 105, y: 600, w: 100, h: 10 },
      { text: 'We now give the', x: 72, y: 500, w: 80, h: 10 },
      { text: 'Proof of the claim', x: 160, y: 500, w: 80, h: 10 },
      { text: 'PROOF OF THEOREM 2.', x: 72, y: 300, w: 120, h: 10 },
      { text: 'Proofs are hard', x: 72, y: 200, w: 120, h: 10 },
    ];
    expect(proofStarts(5, runs)).toEqual([
      { page: 5, y: 610 },
      { page: 5, y: 310 },
    ]);
  });
});

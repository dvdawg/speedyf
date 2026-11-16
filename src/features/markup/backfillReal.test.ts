/** Comments from a real review, kept before quotes were read part by part,
 * re-read from the page under their quads: a draft with margin line numbers
 * (lineno) and inline math in a pdfTeX font set, as the engine dumps it.
 * Each stored quote is what the old selection made of it (numbers, glyph
 * order, a stray letter); the rebuilt one must read as printed. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { marginNumbers } from '../../lib/text/marginNumbers';
import { piecesUnderQuads } from '../../lib/text/piecesUnder';
import { quadBox } from './quoteBackfill';
import { quotePieces, type Dump } from './quoteHarness';
import type { MarkupComment } from './types';

interface Case {
  dump: Dump;
  comment: MarkupComment;
  plain: string;
  after: string;
}

const cases = JSON.parse(
  readFileSync(join(__dirname, 'fixtures/backfill-real.json'), 'utf8')
) as Case[];

async function reread(c: Case) {
  const H = c.dump.size[1];
  const pieces = piecesUnderQuads(
    c.dump.runs,
    c.comment.anchor.quads!.map(quadBox),
    c.dump.displays.map(({ bbox: [x, y, w, h], number }) => ({ x, y: H - y - h, w, h, number })),
    0,
    c.comment.anchor.quote
  );
  return quotePieces(c.dump, pieces);
}

const byId = (id: string) => cases.find((c) => c.comment.id === id)!;

describe('re-reading kept quotes of a numbered draft', () => {
  it('reads each as it did when the fixture was made', async () => {
    for (const c of cases) {
      const q = await reread(c);
      expect(q.plain).toBe(c.plain);
      expect(q.after).toBe(c.after);
    }
  });

  it('finds the margin line numbers', () => {
    const { runs } = byId('c_mwihix7m2sx').dump;
    const flagged = runs.filter((_, i) => marginNumbers(runs)[i]).map((r) => r.text);
    expect(flagged).toEqual(expect.arrayContaining(['492', '493', '494', '495', '496', '497']));
  });

  it('drops the line numbers the old quote had', async () => {
    const c = byId('c_mwihix7m2sx');
    expect(c.comment.anchor.quote).toContain('poorly 492 tuned');
    const q = await reread(c);
    expect(q.plain).toContain('a poorly tuned one leaves');
    expect(q.plain).not.toMatch(/\b4\d\d\b/);
  });

  it('typesets the math the old quote had as glyph soup', async () => {
    const q = await reread(byId('c_mwihmj563x3'));
    expect(q.after).toMatch(
      /^Equation \(5\)\. \$\\sum_i a_i\\operatorname\{KL\}\(q\\\|p_i\) - \\sum_j/
    );
    expect(q.after).toContain('$A\\log q^\\star + A\\log Z$');
    expect(q.after).toContain('with $\\bar{p} = \\sum_i(a_i/A)p_i$, whose minimiser');
    expect(q.after).not.toMatch(/\b7\d\d\b/);
    expect(q.after).not.toContain('*');
  });

  it('rejoins a word broken at a line end, around its line number', async () => {
    const q = await reread(byId('c_mwihnd0r4mu'));
    expect(q.plain).toContain('differentiating,');
    expect(q.after).toContain('\\int fpe^{\\beta\\mathcal{R}}/Z(\\beta)$; differentiating, $');
  });
});

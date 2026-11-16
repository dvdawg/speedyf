import { describe, expect, it } from 'vitest';
import { plausibleQuoteTex, texParses } from './quoteTexCheck';

describe('plausibleQuoteTex', () => {
  const plain =
    'Under the forward KL the same weights give sum_i a_i KL(p_i||q) = A E_p[log q] + const with p = sum (a_i/A) p_i whose minimiser is the';

  it('keeps a faithful reading', () => {
    expect(
      plausibleQuoteTex(
        'Under the forward KL the same weights give $\\sum_i a_i \\mathrm{KL}(p_i \\| q) = A\\,\\mathbb{E}_p[\\log q] + \\mathrm{const}$ with $p = \\sum_i (a_i/A) p_i$ whose minimiser is the',
        plain
      )
    ).toBe(true);
  });

  it('drops a reading whose words were taken apart', () => {
    // what a converter made of a passage of text and math in a review
    const garbage =
      'AEqEu_q atloiogn$q$ (5). Eq\u2211 \u2211i aiiKaiLlo($q$g\u0131ppii)+ \u2211\u2211j \u03b2jEq\\[Rj = 746 bracket equals A log q . + A log Z. j \u03b2jRj] and the 774478 \u2211^U nder the forward KL the same weights give 749 ;a_i KL(p_i||q) = AE_p[log q] +const with 750 p = \u2211(ai/A)pi whose minimiser is the';
    expect(plausibleQuoteTex(garbage, plain)).toBe(false);
  });

  it('drops margin line numbers the page text does not have', () => {
    expect(
      plausibleQuoteTex('the bound $x^2$ holds 746 for all', 'the bound x2 holds for all')
    ).toBe(false);
    expect(plausibleQuoteTex('in 2024 the bound $x^2$', 'in 2024 the bound x2')).toBe(true);
  });

  it('drops a formula that swallowed prose', () => {
    expect(
      plausibleQuoteTex('$$ \\beta f = f p e^{\\beta R}/Z(\\beta); differentiat- ing $$', '')
    ).toBe(false);
    expect(plausibleQuoteTex('$$ \\frac{\\partial}{\\partial\\beta} \\log Z(\\beta) $$', '')).toBe(
      true
    );
  });

  it('drops mojibake and text with no math at all', () => {
    expect(plausibleQuoteTex('caf\u00c3\u00a9 $x$', 'cafe x')).toBe(false);
    expect(plausibleQuoteTex('plain words only', 'plain words only')).toBe(false);
  });
});

describe('texParses', () => {
  it('asks KaTeX about every formula', async () => {
    expect(await texParses('a $\\frac{1}{2}$ and $$ x^2 $$')).toBe(true);
    expect(await texParses('a $\\frac{1}{2$ b')).toBe(false);
    expect(await texParses('$\\notacommand{x}$')).toBe(false);
  });
});

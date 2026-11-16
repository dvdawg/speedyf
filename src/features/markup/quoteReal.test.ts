/** Comment quotes from real papers: selections mixing prose, inline math
 * and a numbered display, cut from pages of arXiv papers (pdfTeX), as the
 * engine dumps them (`quote_eval_dump`). Each is quoted through Math to
 * LaTeX's own code, part by part (`planQuote`), and must read as the page
 * does: prose as printed, every formula typeset, each display whole with
 * its number as its tag. `before` in the fixture is what quoting the whole
 * selection at once gave, kept for comparison. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { quote, type Dump } from './quoteHarness';

interface Case {
  dump: Dump;
  before: string | null;
  after: string;
}

const cases = JSON.parse(
  readFileSync(join(__dirname, 'fixtures/quotes-real.json'), 'utf8')
) as Case[];

const at = (name: string, page: number) =>
  cases.find((c) => c.dump.path === name && c.dump.page === page)!;

async function quoted(c: Case): Promise<string> {
  return (await quote(c.dump, 0, c.dump.runs.length - 1)).after;
}

describe('quotes from real papers', () => {
  it('reads each as it did when the fixture was made', async () => {
    for (const c of cases) expect(await quoted(c)).toBe(c.after);
  });

  it('keeps a formula whole across the runs it was set in', async () => {
    const text = await quoted(at('mot.pdf', 1));
    // was "$\delta \in (0$, 1)" read as one selection
    expect(text).toContain('For every $\\delta \\in (0, 1)$, every distribution $P$');
    expect(text).toContain(
      '\\leq 10^7\\frac{d + \\log(1/\\delta)}{n}\\right) \\geq 1 - \\delta. $$'
    );
    expect(text).toContain('lower bound [EHKV89] up to');
  });

  it('reads a display whole, with its number as its tag', async () => {
    expect(await quoted(at('2107.05673v3.pdf', 1))).toContain('$$ A = U - W \\geq 0, \\tag{2} $$');
    const volterra = await quoted(at('2512.24540v3.pdf', 3));
    // was one $$ ... $$ for each piece of the display
    expect(volterra.match(/\$\$/g)).toHaveLength(2);
    expect(volterra).toContain('\\prod_{j=1}^n\\mathrm{d}t_j. \\tag{17} $$');
    // a symbol and its raised index, two runs on the page, one formula
    expect(volterra).toContain('Here $h^{(n)}$');
  });

  it('reads function names inside a formula as the formula', async () => {
    const text = await quoted(at('mot.pdf', 2));
    expect(text).toContain(
      'Choose $q = \\lceil\\log(1/\\delta)\\rceil$. Then $q \\leq \\log(e/\\delta)$'
    );
    expect(text).not.toContain('*');
  });

  it('reads a two-line display as one, and the prose around it as printed', async () => {
    const text = await quoted(at('2512.24540v3.pdf', 4));
    expect(text).toMatch(/^laxation function \(as defined by Eq\. \(24\)\)/);
    expect(text).toContain('\\begin{gathered}');
    expect(text).toContain('\\tag{25} $$');
    expect(text).toMatch(/From this expression, we can derive thermodynamic$/);
  });
});

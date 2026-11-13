/** Text an extension shows that came from a document: ASCII, with math as
 * `$...$` TeX (or, in Math to LaTeX's output, as LaTeX commands), and
 * accented letters in names kept as written.
 *
 * Each worker is one self-contained file, so each has its own copy of the
 * helper: Notation's `asciiText`, References' `displayText`, Math to LaTeX's
 * `asciiLatex`. They are reached the way the other worker tests reach theirs,
 * by returning them from the same function body the host's blob would run. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

function load<T>(id: string, names: string[]): T {
  const source = readFileSync(join(__dirname, `../../../extensions/${id}/worker.js`), 'utf8');
  return new Function('self', `${source}\nreturn { ${names.join(', ')} };`)({}) as T;
}

const { asciiText } = load<{ asciiText(text: string): string }>('dev.speedyf.notation', [
  'asciiText',
]);
const { displayText } = load<{ displayText(text: string): string }>('dev.speedyf.references', [
  'displayText',
]);
const { asciiLatex } = load<{ asciiLatex(latex: string): string }>('dev.speedyf.mathlatex', [
  'asciiLatex',
]);

describe.each([
  ['Notation asciiText', asciiText],
  ['References displayText', displayText],
])('%s', (_name, sanitize) => {
  it('straightens curly quotes', () => {
    expect(sanitize('\u201cDeep learning\u201d and \u2018it\u2019s\u2019')).toBe(
      "\"Deep learning\" and 'it's'"
    );
  });

  it('writes dashes and the ellipsis in ASCII', () => {
    expect(sanitize('pages 3\u20135 \u2014 a survey\u2026')).toBe('pages 3-5 - a survey...');
  });

  it('spells ligatures out', () => {
    expect(sanitize('the \ufb01rst e\ufb03cient \ufb02ow')).toBe('the first efficient flow');
  });

  it('writes Greek and math symbols as TeX', () => {
    expect(sanitize('\u03b1-stable')).toBe('$\\alpha$-stable');
    expect(sanitize('x \u2264 \u221e')).toBe('x $\\leq$ $\\infty$');
    expect(sanitize('f \u2192 g')).toBe('f $\\to$ g');
  });

  it('raises and lowers super- and subscripts', () => {
    expect(sanitize('x\u00b2 + y')).toBe('$x^{2}$ + y');
    expect(sanitize('E\u2080 at 10\u207b\u00b3')).toBe('$E_{0}$ at $10^{-3}$');
  });

  it('keeps accented letters in names', () => {
    expect(sanitize('Paul Erd\u0151s and L\u00e1szl\u00f3 Gy\u00f6rfi')).toBe(
      'Paul Erd\u0151s and L\u00e1szl\u00f3 Gy\u00f6rfi'
    );
    // A letter and its combining accent, as some PDFs give it, are one letter.
    expect(sanitize('Go\u0308del')).toBe('G\u00f6del');
  });

  it('escapes a dollar sign so it is not read as math', () => {
    expect(sanitize('costs $5')).toBe('costs \\$5');
  });

  it('leaves no non-ASCII character but letters', () => {
    const out = sanitize(
      '\u00ab\u03bb\u2208\u211d\u00bb \u2022 \u00a9 2024\u00a0\u2013 Erd\u0151s'
    );
    expect(out.replace(/\p{L}/gu, (c) => (c < '\u0080' ? c : ''))).toMatch(/^[\x20-\x7e]*$/);
  });
});

describe('Math to LaTeX asciiLatex', () => {
  it('writes Unicode math as LaTeX commands', () => {
    expect(asciiLatex('\u03b1 + \u03b2 \u2264 \u221e')).toBe('\\alpha + \\beta \\leq \\infty');
    expect(asciiLatex('\u2211_{i} \u03b1x')).toBe('\\sum_{i} \\alpha x');
  });

  it('raises and lowers super- and subscripts', () => {
    expect(asciiLatex('x\u00b2 + y\u2080')).toBe('x^{2} + y_{0}');
  });

  it('puts a combining accent on its letter and spells styled letters plainly', () => {
    expect(asciiLatex('x\u0302 + \u{1d465}')).toBe('\\hat{x} + x');
  });

  it('straightens punctuation and ligatures, and keeps accented letters', () => {
    expect(asciiLatex('\\text{Erd\u0151s\u2019s \u201cbound\u201d \u2014 \ufb01ne}')).toBe(
      "\\text{Erd\u0151s's ''bound'' - fine}"
    );
  });

  it('leaves ASCII LaTeX as it is', () => {
    expect(asciiLatex('\\frac{a}{b} + \\alpha_{i}^{2}')).toBe('\\frac{a}{b} + \\alpha_{i}^{2}');
  });
});

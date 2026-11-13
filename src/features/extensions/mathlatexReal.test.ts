/** The Math to LaTeX extension against glyphs read from real PDFs.
 *
 * `fixtures/mathlatex-real.json` holds the glyphs PDFium reports for display
 * equations (and one text selection) in arXiv papers (pdfTeX, dvips +
 * Ghostscript, a page re-saved as one form XObject), an Elsevier article set
 * in STIX Two Math, and a MathType/InDesign book, each with the LaTeX the
 * converter gives, checked against the PDF and, for arXiv, its source. They
 * are what the hand-made fixtures in mathlatex.test.ts could not be: loose
 * boxes, ToUnicode text with the raw codes gone, extension-font glyphs that
 * hang below their origin, Mac Roman read into a Symbol font.
 *
 * The rest are the rules those PDFs taught, each in isolation.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.mathlatex/worker.js'),
  'utf8'
);

interface Glyph {
  text: string;
  code: number;
  font: string;
  size: number;
  x: number;
  y: number;
  w: number;
  h: number;
  baseline: number;
  bold: boolean;
  italic: boolean;
  ink: [number, number, number, number] | null;
}
interface Atom {
  tex: string;
  kind: string;
  ok: boolean;
  tall?: boolean;
  delimSize?: number;
  y: number;
  h: number;
}
interface Snippet {
  tex: string;
  env: string;
  display: boolean;
  numbers: string[];
}
interface Ext {
  decodeGlyph(g: Glyph): Atom;
  reconstruct(glyphs: Glyph[]): { latex: string; confidence: string; rows: number };
  dropNumber(glyphs: Glyph[], number: string): Glyph[];
  parseSource(files: { name: string; text: string }[]): Snippet[];
  collectMacros(text: string): Map<string, string>;
  expandMacros(tex: string, macros: Map<string, string>): string;
  texTokens(tex: string): { t: string }[];
  matchSnippets(
    snippets: Snippet[],
    queries: string[][],
    opts?: { number?: string | null; display?: boolean }
  ): { tex: string; exact: boolean } | null;
}

const ext = new Function(
  'self',
  `${SOURCE}\nreturn { decodeGlyph, reconstruct, dropNumber, parseSource, collectMacros, expandMacros, texTokens, matchSnippets };`
)({}) as Ext;

interface Case {
  source: string;
  what: string;
  number: string | null;
  latex: string;
  glyphs: string[];
}

const FIXTURES = JSON.parse(
  readFileSync(join(__dirname, 'fixtures/mathlatex-real.json'), 'utf8')
) as { cases: Case[] };

/** `font|code|size|x|y|w|h|baseline|italic|ink|text` → a glyph. */
function unpack(line: string): Glyph {
  const f = line.split('|');
  const n = (i: number) => Number(f[i]);
  return {
    font: f[0]!,
    code: n(1),
    size: n(2),
    x: n(3),
    y: n(4),
    w: n(5),
    h: n(6),
    baseline: n(7),
    bold: false,
    italic: f[8] === '1',
    ink: f[9] === '-' ? null : (f[9]!.split(',').map(Number) as Glyph['ink']),
    text: f.slice(10).join('|'),
  };
}

describe('real equations', () => {
  it('has the cases it was built from', () => {
    expect(FIXTURES.cases.length).toBeGreaterThanOrEqual(15);
  });

  for (const c of FIXTURES.cases) {
    it(`${c.source}: ${c.what}`, () => {
      let glyphs = c.glyphs.map(unpack);
      // As the equation action does: the region may clip the number.
      if (c.number) glyphs = ext.dropNumber(glyphs, c.number);
      expect(ext.reconstruct(glyphs).latex).toBe(c.latex);
    });
  }
});

/** A glyph the way PDFium reports one from a font with a ToUnicode map:
 * the text, the Unicode scalar as its code, the font's ascent-to-descent
 * box (here 0.75 above the baseline and 0.25 below), no raw code. */
function mapped(font: string, text: string, x: number, baseline: number, size = 10): Glyph {
  return {
    text,
    code: text.codePointAt(0) ?? 0,
    font,
    size,
    x,
    y: baseline - 0.75 * size,
    w: 0.5 * size,
    h: size,
    baseline,
    bold: false,
    italic: false,
    ink: null,
  };
}

describe('glyphs with Unicode text', () => {
  it('reads the text, not the TeX encoding, when there is one', () => {
    // In OML, code 46 is \triangleright and 44 \leftharpoondown; mapped, they
    // are the period and comma they print as.
    expect(ext.decodeGlyph(mapped('CMMI10', '.', 0, 10)).tex).toBe('.');
    expect(ext.decodeGlyph(mapped('CMMI10', ',', 0, 10)).tex).toBe(',');
    expect(ext.decodeGlyph(mapped('CMMI10', '/', 0, 10)).tex).toBe('/');
    expect(ext.decodeGlyph(mapped('CMSY10', '|', 0, 10)).tex).toBe('|');
    expect(ext.decodeGlyph(mapped('CMMI10', 'λ', 0, 10)).tex).toBe('\\lambda');
    expect(ext.decodeGlyph(mapped('CMR10', '∆', 0, 10)).tex).toBe('\\Delta');
    expect(ext.decodeGlyph(mapped('CMR10', 'Ω', 0, 10)).tex).toBe('\\Omega');
    // TeX's \| maps to U+2225; it is a norm far more often than \parallel.
    expect(ext.decodeGlyph(mapped('CMSY10', '∥', 0, 10)).tex).toBe('\\|');
  });

  it('keeps the font for style: calligraphic, blackboard, fraktur, script', () => {
    expect(ext.decodeGlyph(mapped('CMSY10', 'L', 0, 10)).tex).toBe('\\mathcal{L}');
    expect(ext.decodeGlyph(mapped('MSBM10', 'R', 0, 10)).tex).toBe('\\mathbb{R}');
    expect(ext.decodeGlyph(mapped('EUFM10', 'B', 0, 10)).tex).toBe('\\mathfrak{B}');
    expect(ext.decodeGlyph(mapped('RSFS10', 'F', 0, 10)).tex).toBe('\\mathscr{F}');
    // An italic letter from the math font stays a plain letter.
    expect(ext.decodeGlyph(mapped('CMMI10', 'x', 0, 10))).toMatchObject({ tex: 'x' });
  });

  it('reads an unmapped glyph by its code, U+0000 included', () => {
    // PDFium reports cmex's code 0, the \big left parenthesis, as U+0000.
    const big = { ...mapped('CMEX10', '\u0000', 0, 10), code: 0 };
    expect(ext.decodeGlyph(big)).toMatchObject({ tex: '(', kind: 'open', delimSize: 1 });
  });

  it('places an extension-font glyph below its origin, where it is drawn', () => {
    // Display \int (code 0x5A) hangs 2.22 em from its origin.
    const int = { ...mapped('CMEX10', '', 100, 50), code: 0x5a };
    const atom = ext.decodeGlyph(int);
    expect(atom.y).toBeCloseTo(50, 0);
    expect(atom.y + atom.h).toBeCloseTo(72.2, 0);
    // With an ink box, that is used instead.
    const inked = { ...int, ink: [100, 49, 8, 20] as Glyph['ink'] };
    expect(ext.decodeGlyph(inked)).toMatchObject({ y: 49, h: 20 });
  });

  it('sizes a Unicode delimiter by its ink', () => {
    const tall = {
      ...mapped('STIXTwoMath-Regular', '(', 0, 10),
      ink: [0, -14, 4, 30] as Glyph['ink'],
    };
    expect(ext.decodeGlyph(tall)).toMatchObject({ kind: 'open', tall: true, delimSize: 4 });
    const small = {
      ...mapped('STIXTwoMath-Regular', '(', 0, 10),
      ink: [0, 2, 4, 10] as Glyph['ink'],
    };
    expect(ext.decodeGlyph(small).tall).toBeFalsy();
  });

  it('reads \\not as a negation', () => {
    const r = ext.reconstruct([
      mapped('CMMI10', 'a', 100, 50),
      mapped('CMSY10', '̸', 108, 50),
      mapped('CMR10', '=', 108, 50),
      mapped('CMMI10', 'b', 118, 50),
    ]);
    expect(r.latex).toBe('a \\neq b');
  });

  it('reads a Symbol font whose text came through Mac Roman', () => {
    // Codes E5 (∑), F2 (∫) and CE (∈) of the Symbol encoding, read as Mac
    // Roman: Â, Ú, Œ.
    const r = ext.reconstruct([
      mapped('MTSymbol', 'Ú', 100, 50, 14),
      mapped('Times-Italic', 'x', 112, 50),
      mapped('MTSymbol', 'Œ', 120, 50),
      mapped('Times-Italic', 'A', 130, 50),
    ]);
    expect(r.latex).toBe('\\int x \\in A');
  });

  it('keeps a word set in the text font upright, and an operator name named', () => {
    const word = (w: string, x: number) =>
      Array.from(w).map((c, i) => mapped('CMR10', c, x + 5 * i, 50));
    const r = ext.reconstruct([
      ...word('diam', 100),
      mapped('CMR10', '(', 120, 50),
      mapped('CMMI10', 'T', 125, 50),
      mapped('CMR10', ')', 131, 50),
    ]);
    expect(r.latex).toBe('\\operatorname{diam}(T)');
  });

  it('drops the equation number the region clipped', () => {
    const glyphs = [
      mapped('CMMI10', 'x', 100, 50),
      mapped('CMR10', '(', 300, 50),
      mapped('CMR10', '1', 305, 50),
    ];
    expect(ext.dropNumber(glyphs, '12').map((g) => g.text)).toEqual(['x']);
    // A parenthesis that is not the number stays.
    expect(ext.dropNumber(glyphs, '3').map((g) => g.text)).toEqual(['x', '(', '1']);
  });
});

describe('reading arXiv source', () => {
  const MAIN = String.raw`\documentclass{revtex4-1}
\newcommand{\be}{\begin{equation}}
\newcommand{\ee}{\end{equation}}
\newcommand{\err}[2]{\mathrm{err}_{#2}(#1)}
\newcommand{\bl}[1][\lambda]{\boldsymbol{#1}}
\begin{document}
\be \pi(x|\bl) = e^{-x} \ee
\begin{subequations}
\begin{align}
  a &= b \\
  c &= d
\end{align}
\end{subequations}
\be \err{f}{P} \le \bl[\mu] \ee
\end{document}
`;

  it('opens displays written with shorthand macros', () => {
    const snippets = ext.parseSource([{ name: 'main.tex', text: MAIN }]);
    const pi = snippets.find((s) => s.tex.startsWith('\\pi'));
    expect(pi?.numbers).toEqual(['1']);
  });

  it('numbers subequations 2a, 2b, then goes on at 3', () => {
    const snippets = ext.parseSource([{ name: 'main.tex', text: MAIN }]);
    expect(snippets.find((s) => s.env === 'align')?.numbers).toEqual(['2a', '2b']);
    expect(snippets.find((s) => s.tex.includes('err'))?.numbers).toEqual(['3']);
  });

  it('expands macros with arguments, and an optional first one', () => {
    const macros = ext.collectMacros(MAIN);
    expect(ext.expandMacros('\\err{f}{P}', macros)).toBe('{\\mathrm{err}_{P}(f)}');
    expect(ext.expandMacros('\\bl + \\bl[\\mu]', macros)).toBe(
      '{\\boldsymbol{\\lambda}} + {\\boldsymbol{\\mu}}'
    );
    // An argument-free macro is still all a Map lookup sees.
    expect(macros.has('err')).toBe(false);
  });

  it('matches a reconstruction to its numbered source', () => {
    const snippets = ext.parseSource([{ name: 'main.tex', text: MAIN }]);
    const query = ext
      .texTokens('\\operatorname{err}_P(f) \\leq \\boldsymbol{\\mu}')
      .map((t) => t.t);
    const found = ext.matchSnippets(snippets, [query], { number: '3', display: true });
    expect(found?.tex).toBe('{\\mathrm{err}_{P}(f)} \\le {\\boldsymbol{\\mu}}');
  });
});

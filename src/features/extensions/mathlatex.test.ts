/** The pure parts of the built-in Math to LaTeX extension: the font tables,
 * the Unicode table, reading structure back out of glyph geometry, the
 * arXiv source reader (gzip, tar, TeX) and its matcher, and the MathML
 * converter used for tagged PDFs and JATS.
 *
 * Reached the way the References tests reach their parser: the shipped
 * worker is run as a function body that returns its own functions. Nothing
 * in the shipped file exports them.
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
}
interface Atom {
  tex: string;
  kind: string;
  ok: boolean;
  delimSize?: number;
}
interface Snippet {
  tex: string;
  env: string;
  display: boolean;
  numbers: string[];
}
interface Ext {
  decodeGlyph(g: Glyph): Atom;
  fromUnicode(text: string): Atom | null;
  reconstruct(glyphs: Glyph[]): { latex: string; confidence: string; rows: number };
  gunzip(bytes: Uint8Array): Promise<Uint8Array>;
  untar(bytes: Uint8Array): { name: string; data: Uint8Array }[];
  readEprint(bytes: Uint8Array): Promise<{ name: string; text: string }[] | null>;
  parseSource(files: { name: string; text: string }[]): Snippet[];
  collectMacros(text: string): Map<string, string>;
  expandMacros(tex: string, macros: Map<string, string>): string;
  texTokens(tex: string): { t: string }[];
  textTokens(text: string): string[];
  matchSnippets(
    snippets: Snippet[],
    queries: string[][],
    opts?: { number?: string | null; display?: boolean }
  ): { tex: string; exact: boolean; snippet: Snippet } | null;
  mathmlToLatex(xml: string): string;
  jatsFormulas(xml: string): Snippet[];
  OML: unknown[];
  OMS: unknown[];
  OMX: unknown[];
  OT1: unknown[];
  MSAM: unknown[];
  MSBM: unknown[];
}

const ext = new Function(
  'self',
  `${SOURCE}\nreturn { decodeGlyph, fromUnicode, reconstruct, gunzip, untar, readEprint, parseSource, collectMacros, expandMacros, texTokens, textTokens, matchSnippets, mathmlToLatex, jatsFormulas, OML, OMS, OMX, OT1, MSAM, MSBM };`
)({}) as Ext;

// ---------- fixtures ----------

/** A glyph set on `baseline` at `x`, the way the host reports one: a box from
 * ascender to descender, the raw code, the font's base name. */
function g(font: string, code: number, x: number, baseline: number, size = 10, w?: number): Glyph {
  return {
    text: '',
    code,
    font,
    size,
    x,
    y: baseline - 0.7 * size,
    w: w ?? 0.55 * size,
    h: 0.9 * size,
    baseline,
    bold: false,
    italic: false,
  };
}
/** Math italic letter (cmmi). */
const mi = (ch: string, x: number, baseline: number, size = 10) =>
  g(size < 9 ? 'CMMI7' : 'CMMI10', ch.charCodeAt(0), x, baseline, size);
/** Roman (cmr): digits, +, =, upright letters. */
const rm = (ch: string, x: number, baseline: number, size = 10, w?: number) =>
  g(size < 9 ? 'CMR7' : 'CMR10', ch.charCodeAt(0), x, baseline, size, w);
const rule = (x: number, y: number, w: number): Glyph => ({
  text: '',
  code: 0,
  font: '__rule',
  size: 0,
  x,
  y,
  w,
  h: 0.4,
  baseline: y + 0.4,
  bold: false,
  italic: false,
});

// ---------- tables ----------

describe('TeX font tables', () => {
  it('has all 128 slots of every encoding', () => {
    for (const t of [ext.OML, ext.OMS, ext.OMX, ext.OT1, ext.MSAM, ext.MSBM]) {
      expect(t).toHaveLength(128);
    }
  });

  it('reads math italic (OML) by code', () => {
    expect(ext.decodeGlyph(g('CMMI10', 11, 0, 10)).tex).toBe('\\alpha');
    expect(ext.decodeGlyph(g('CMMI10', 34, 0, 10)).tex).toBe('\\varepsilon');
    expect(ext.decodeGlyph(g('CMMI10', 120, 0, 10)).tex).toBe('x');
    expect(ext.decodeGlyph(g('CMMI10', 64, 0, 10)).tex).toBe('\\partial');
    expect(ext.decodeGlyph(g('CMMI10', 96, 0, 10)).tex).toBe('\\ell');
    // The subset prefix is not part of the name.
    expect(ext.decodeGlyph(g('ABCDEF+CMMI7', 12, 0, 10)).tex).toBe('\\beta');
    // Latin Modern's equivalent reads the same.
    expect(ext.decodeGlyph(g('LMMathItalic10-Regular', 18, 0, 10)).tex).toBe('\\theta');
  });

  it('reads math symbols (OMS) by code', () => {
    expect(ext.decodeGlyph(g('CMSY10', 20, 0, 10))).toMatchObject({ tex: '\\leq', kind: 'rel' });
    expect(ext.decodeGlyph(g('CMSY10', 50, 0, 10))).toMatchObject({ tex: '\\in', kind: 'rel' });
    expect(ext.decodeGlyph(g('CMSY10', 49, 0, 10)).tex).toBe('\\infty');
    expect(ext.decodeGlyph(g('CMSY10', 0, 0, 10))).toMatchObject({ tex: '-', kind: 'bin' });
    expect(ext.decodeGlyph(g('CMSY10', 70, 0, 10)).tex).toBe('\\mathcal{F}');
    expect(ext.decodeGlyph(g('CMSY7', 48, 0, 7)).kind).toBe('prime');
  });

  it('reads the extension font (OMX): big operators and sized delimiters', () => {
    expect(ext.decodeGlyph(g('CMEX10', 0x58, 0, 10))).toMatchObject({
      tex: '\\sum',
      kind: 'bigop',
    });
    expect(ext.decodeGlyph(g('CMEX10', 0x50, 0, 10))).toMatchObject({
      tex: '\\sum',
      kind: 'bigop',
    });
    expect(ext.decodeGlyph(g('CMEX10', 0x5a, 0, 10))).toMatchObject({
      tex: '\\int',
      kind: 'bigop',
    });
    expect(ext.decodeGlyph(g('CMEX10', 0x59, 0, 10)).tex).toBe('\\prod');
    expect(ext.decodeGlyph(g('CMEX10', 0x00, 0, 10))).toMatchObject({
      tex: '(',
      kind: 'open',
      delimSize: 1,
    });
    expect(ext.decodeGlyph(g('CMEX10', 0x21, 0, 10))).toMatchObject({
      tex: ')',
      kind: 'close',
      delimSize: 4,
    });
    expect(ext.decodeGlyph(g('CMEX10', 0x08, 0, 10)).tex).toBe('\\{');
    expect(ext.decodeGlyph(g('CMEX10', 0x30, 0, 10)).kind).toBe('piece');
    expect(ext.decodeGlyph(g('CMEX10', 0x71, 0, 10)).kind).toBe('radical');
  });

  it('reads the AMS fonts', () => {
    expect(ext.decodeGlyph(g('MSBM10', 0x52, 0, 10)).tex).toBe('\\mathbb{R}');
    expect(ext.decodeGlyph(g('MSBM10', 0x4e, 0, 10)).tex).toBe('\\mathbb{N}');
    expect(ext.decodeGlyph(g('MSBM10', 0x3f, 0, 10)).tex).toBe('\\varnothing');
    expect(ext.decodeGlyph(g('MSAM10', 0x36, 0, 10)).tex).toBe('\\leqslant');
    expect(ext.decodeGlyph(g('MSAM10', 0x03, 0, 10)).tex).toBe('\\square');
    expect(ext.decodeGlyph(g('EUFM10', 0x67, 0, 10)).tex).toBe('\\mathfrak{g}');
  });

  it('reads text fonts (OT1): upright Greek, digits, accents', () => {
    expect(ext.decodeGlyph(g('CMR10', 0, 0, 10)).tex).toBe('\\Gamma');
    expect(ext.decodeGlyph(g('CMR10', 50, 0, 10)).tex).toBe('2');
    expect(ext.decodeGlyph(g('CMR10', 94, 0, 10))).toMatchObject({ tex: '\\hat', kind: 'accent' });
  });

  it('falls back to the Unicode text for fonts it does not know', () => {
    const cambria = { ...g('CambriaMath', 0, 0, 10), text: '≤' };
    expect(ext.decodeGlyph(cambria).tex).toBe('\\leq');
    const unknown = { ...g('Mystery', 0, 0, 10), text: '' };
    expect(ext.decodeGlyph(unknown).ok).toBe(false);
  });
});

describe('the Unicode table', () => {
  it('maps Greek, operators, relations, arrows and sets', () => {
    expect(ext.fromUnicode('α')?.tex).toBe('\\alpha');
    expect(ext.fromUnicode('φ')?.tex).toBe('\\varphi');
    expect(ext.fromUnicode('ϕ')?.tex).toBe('\\phi');
    expect(ext.fromUnicode('Ω')?.tex).toBe('\\Omega');
    expect(ext.fromUnicode('≤')).toMatchObject({ tex: '\\leq', kind: 'rel' });
    expect(ext.fromUnicode('⇒')?.tex).toBe('\\Rightarrow');
    expect(ext.fromUnicode('∑')).toMatchObject({ tex: '\\sum', kind: 'bigop' });
    expect(ext.fromUnicode('∩')).toMatchObject({ tex: '\\cap', kind: 'bin' });
    expect(ext.fromUnicode('∅')?.tex).toBe('\\emptyset');
    expect(ext.fromUnicode('−')?.tex).toBe('-');
    expect(ext.fromUnicode('ℝ')?.tex).toBe('\\mathbb{R}');
    expect(ext.fromUnicode('⁢')?.tex).toBe('');
  });

  it('maps the mathematical alphanumerics to a style and the base letter', () => {
    expect(ext.fromUnicode('𝐱')?.tex).toBe('\\mathbf{x}');
    expect(ext.fromUnicode('𝑥')?.tex).toBe('x');
    expect(ext.fromUnicode('𝔤')?.tex).toBe('\\mathfrak{g}');
    expect(ext.fromUnicode('𝕜')?.tex).toBe('\\mathbb{k}');
    expect(ext.fromUnicode('𝒜')?.tex).toBe('\\mathcal{A}');
    expect(ext.fromUnicode('𝛼')?.tex).toBe('\\alpha');
    expect(ext.fromUnicode('𝟙')?.tex).toBe('\\mathbb{1}');
  });
});

// ---------- geometry ----------

describe('structure from glyph geometry', () => {
  it('reads a subscript and a superscript on one base: x_i^2', () => {
    const r = ext.reconstruct([
      mi('x', 100, 200),
      mi('i', 105.7, 201.5, 7),
      rm('2', 105.7, 196.4, 7),
    ]);
    expect(r.latex).toBe('x_i^2');
    expect(r.confidence).toBe('high');
  });

  it('reads a fraction: \\frac{a+b}{c}', () => {
    const r = ext.reconstruct([
      mi('a', 101, 191),
      rm('+', 107, 191, 10, 7.8),
      mi('b', 116, 191),
      rule(100, 197.3, 22),
      mi('c', 108.5, 207),
    ]);
    expect(r.latex).toBe('\\frac{a + b}{c}');
  });

  it('reads a square root: \\sqrt{x}', () => {
    const radical = { ...g('CMSY10', 112, 100, 200), y: 192.5, h: 10, w: 8.3 };
    const r = ext.reconstruct([radical, rule(108.3, 192.5, 6.5), mi('x', 108.6, 200)]);
    expect(r.latex).toBe('\\sqrt{x}');
  });

  it('reads a big operator with limits: \\sum_{i=1}^{n} a_i', () => {
    const sum = { ...g('CMEX10', 0x58, 100, 188), y: 188, h: 20, w: 14.4 };
    const r = ext.reconstruct([
      sum,
      mi('n', 105, 185, 7),
      mi('i', 101.5, 214, 7),
      rm('=', 104.3, 214, 7, 5.5),
      rm('1', 109.8, 214, 7, 3.5),
      mi('a', 118, 200),
      mi('i', 123.3, 201.5, 7),
    ]);
    expect(r.latex).toBe('\\sum_{i=1}^n a_i');
  });

  it('reads a superscript with an operator in it: e^{-\\beta E}', () => {
    const r = ext.reconstruct([
      mi('e', 100, 200),
      g('CMSY7', 0, 104.7, 196.4, 7),
      g('CMMI7', 12, 110.9, 196.4, 7),
      mi('E', 115, 196.4, 7),
    ]);
    expect(r.latex).toBe('e^{-\\beta E}');
  });

  it('reads a matrix inside tall parentheses as pmatrix', () => {
    const open = { ...g('CMEX10', 0x20, 100, 212), y: 188, h: 24, w: 6 };
    const close = { ...g('CMEX10', 0x21, 132, 212), y: 188, h: 24, w: 6 };
    const r = ext.reconstruct([
      open,
      mi('a', 108, 196),
      mi('b', 122, 196),
      mi('c', 108, 208),
      mi('d', 122, 208),
      close,
    ]);
    expect(r.latex).toBe('\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}');
  });

  it('reads two lines whose relations line up as aligned', () => {
    const r = ext.reconstruct([
      mi('x', 100, 200),
      rm('=', 108.5, 200, 10, 7.8),
      mi('a', 119, 200),
      rm('+', 127, 200, 10, 7.8),
      mi('b', 137, 200),
      rm('=', 108.5, 215, 10, 7.8),
      mi('c', 119, 215),
    ]);
    expect(r.latex).toBe('\\begin{aligned} x &= a + b \\\\ &= c \\end{aligned}');
    expect(r.rows).toBe(2);
  });

  it('reads an accent over a symbol: \\hat{x}', () => {
    const hat = { ...rm('^', 100.6, 197), y: 190, h: 4, w: 5 };
    expect(ext.reconstruct([mi('x', 100, 200), hat]).latex).toBe('\\hat{x}');
  });

  it('reads an upright function name: \\sin\\theta', () => {
    const r = ext.reconstruct([
      rm('s', 100, 200, 10, 3.9),
      rm('i', 103.9, 200, 10, 2.8),
      rm('n', 106.7, 200, 10, 5.6),
      g('CMMI10', 18, 114, 200),
    ]);
    expect(r.latex).toBe('\\sin\\theta');
  });

  it('marks undecodable glyphs as less certain', () => {
    const r = ext.reconstruct([mi('x', 100, 200), { ...g('Mystery', 0, 106, 200), text: '' }]);
    expect(r.confidence).not.toBe('high');
  });
});

// ---------- arXiv source ----------

/** A ustar archive of `files`, built byte by byte. */
function tar(files: Record<string, string>): Uint8Array {
  const enc = new TextEncoder();
  const blocks: Uint8Array[] = [];
  for (const [name, text] of Object.entries(files)) {
    const data = enc.encode(text);
    const h = new Uint8Array(512);
    const put = (s: string, at: number) => h.set(enc.encode(s), at);
    put(name, 0);
    put('0000644\0', 100);
    put('0000000\0', 108);
    put('0000000\0', 116);
    put(`${data.length.toString(8).padStart(11, '0')}\0`, 124);
    put('00000000000\0', 136);
    put('        ', 148);
    put('0', 156);
    put('ustar\0', 257);
    put('00', 263);
    let sum = 0;
    for (const b of h) sum += b;
    put(`${sum.toString(8).padStart(6, '0')}\0 `, 148);
    blocks.push(h);
    const body = new Uint8Array(Math.ceil(data.length / 512) * 512);
    body.set(data);
    blocks.push(body);
  }
  blocks.push(new Uint8Array(1024));
  const out = new Uint8Array(blocks.reduce((n, b) => n + b.length, 0));
  let at = 0;
  for (const b of blocks) {
    out.set(b, at);
    at += b.length;
  }
  return out;
}

async function gzip(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new CompressionStream('gzip'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

const MAIN = String.raw`\documentclass{article}
\usepackage{amsmath}
\newcommand{\R}{\mathbb{R}}
\def\eps{\varepsilon}
\newcommand{\norm}[1]{\lVert #1 \rVert}
\begin{document}
Let $f : \R \to \R$ and fix $\eps > 0$. % a comment with $x$ in it
\input{sections/intro}
\begin{equation}
  E = mc^2 \label{eq:einstein}
\end{equation}
\begin{align}
  a &= b + c \\
  d &= e \nonumber
\end{align}
\begin{equation}
  \int_0^1 x^2 \, dx = \frac{1}{3}
\end{equation}
\end{document}
`;
const INTRO = String.raw`\section{Intro}
We have \[ \sum_{i=1}^{n} a_i = S. \]
`;

describe('the arXiv e-print reader', () => {
  it('reads a gzipped tar back to its files', async () => {
    const packed = await gzip(tar({ 'main.tex': MAIN, 'sections/intro.tex': INTRO }));
    const unpacked = await ext.gunzip(packed);
    const files = ext.untar(unpacked);
    expect(files.map((f) => f.name)).toEqual(['main.tex', 'sections/intro.tex']);
    expect(new TextDecoder().decode(files[1]!.data)).toBe(INTRO);
  });

  it('takes a single gzipped .tex as the whole source, and refuses a PDF', async () => {
    const single = await ext.readEprint(await gzip(new TextEncoder().encode(MAIN)));
    expect(single?.[0]?.text).toBe(MAIN);
    const pdf = await ext.readEprint(new TextEncoder().encode('%PDF-1.5\n...'));
    expect(pdf).toBeNull();
  });

  it('resolves \\input, drops comments, numbers equations and expands simple macros', async () => {
    const files = await ext.readEprint(
      await gzip(tar({ 'main.tex': MAIN, 'sections/intro.tex': INTRO }))
    );
    const snippets = ext.parseSource(files!);
    const texts = snippets.map((s) => s.tex);
    expect(texts).toContain('f : \\mathbb{R} \\to \\mathbb{R}');
    expect(texts).toContain('\\varepsilon > 0');
    expect(texts).toContain('\\sum_{i=1}^{n} a_i = S.');
    expect(texts.some((t) => t.includes('$x$') || t === 'x')).toBe(false);
    const einstein = snippets.find((s) => s.tex === 'E = mc^2');
    expect(einstein?.numbers).toEqual(['1']);
    const align = snippets.find((s) => s.env === 'align');
    expect(align?.tex).toBe('\\begin{aligned} a &= b + c \\\\ d &= e \\end{aligned}');
    expect(align?.numbers).toEqual(['2']);
    expect(snippets.find((s) => s.tex.startsWith('\\int'))?.numbers).toEqual(['3']);
  });
});

describe('macros', () => {
  it('collects argument-free definitions and expands them', () => {
    const macros = ext.collectMacros(
      String.raw`\newcommand{\R}{\mathbb{R}} \newcommand\vx{\mathbf{x}} \def\eps{\varepsilon}
      \DeclareMathOperator{\Tr}{Tr} \newcommand{\abs}[1]{|#1|} \renewcommand{\B}{\R^n}`
    );
    expect(macros.get('R')).toBe('\\mathbb{R}');
    expect(macros.has('abs')).toBe(false);
    expect(ext.expandMacros('\\vx \\in \\R, \\Tr A, \\eps_0, \\B, \\Rx', macros)).toBe(
      '\\mathbf{x} \\in \\mathbb{R}, \\operatorname{Tr} A, \\varepsilon_0, \\mathbb{R}^n, \\Rx'
    );
  });
});

describe('matching a selection against source', () => {
  const snippets: Snippet[] = [
    { tex: 'E = mc^2', env: 'equation', display: true, numbers: ['1'] },
    { tex: 'x_i^2 + y_i^2 = r^2', env: 'equation', display: true, numbers: ['2'] },
    { tex: '\\frac{a+b}{c} \\leq \\alpha', env: 'equation', display: true, numbers: ['3'] },
    { tex: 'x_i^2 + y_i^2 = r^2', env: 'equation', display: true, numbers: ['4'] },
    { tex: '\\mathbb{R}^n', env: 'inline', display: false, numbers: [] },
  ];
  const q = (tex: string) => ext.texTokens(tex).map((t) => t.t);

  it('picks the equation the reconstructed glyphs contain', () => {
    const found = ext.matchSnippets(snippets, [q('\\frac{a + b}{c}\\leq\\alpha')]);
    expect(found?.tex).toBe('\\frac{a+b}{c} \\leq \\alpha');
    expect(found?.exact).toBe(true);
  });

  it('matches the selected text too: ℝ is \\mathbb{R}, α is \\alpha', () => {
    expect(ext.matchSnippets(snippets, [ext.textTokens('a+b\nc ≤ α')])?.tex).toBe(
      '\\frac{a+b}{c} \\leq \\alpha'
    );
    expect(ext.matchSnippets(snippets, [ext.textTokens('ℝ n')])).toBeNull();
    expect(ext.matchSnippets(snippets, [ext.textTokens('x ∈ ℝn')])).toBeNull();
  });

  it('cuts out the selected part of a longer equation', () => {
    expect(ext.matchSnippets(snippets, [q('y_i^2 = r^2')])?.tex).toBe('y_i^2 = r^2');
  });

  it('honours the printed equation number between identical equations', () => {
    const query = [q('x_i^2+y_i^2=r^2')];
    expect(
      ext.matchSnippets(snippets, query, { number: '(4)', display: true })?.snippet.numbers
    ).toEqual(['4']);
    expect(
      ext.matchSnippets(snippets, query, { number: '2', display: true })?.snippet.numbers
    ).toEqual(['2']);
  });

  it('accepts a close but inexact reconstruction, and nothing unrelated', () => {
    const fuzzy = ext.matchSnippets(snippets, [q('x_i^2 + y_j^2 = r^2')]);
    expect(fuzzy?.snippet.tex).toBe('x_i^2 + y_i^2 = r^2');
    expect(fuzzy?.exact).toBe(false);
    expect(ext.matchSnippets(snippets, [q('\\gamma \\otimes \\delta \\to \\omega')])).toBeNull();
  });
});

// ---------- MathML ----------

describe('MathML → LaTeX', () => {
  const m = (body: string) =>
    ext.mathmlToLatex(`<math xmlns="http://www.w3.org/1998/Math/MathML">${body}</math>`);

  it('converts mfrac, msqrt and mroot', () => {
    expect(m('<mfrac><mrow><mi>a</mi><mo>+</mo><mi>b</mi></mrow><mi>c</mi></mfrac>')).toBe(
      '\\frac{a + b}{c}'
    );
    expect(m('<msqrt><mi>x</mi></msqrt>')).toBe('\\sqrt{x}');
    expect(m('<mroot><mi>x</mi><mn>3</mn></mroot>')).toBe('\\sqrt[3]{x}');
  });

  it('converts msubsup and primes', () => {
    expect(m('<msubsup><mi>x</mi><mi>i</mi><mn>2</mn></msubsup>')).toBe('x_i^2');
    expect(m('<msup><mi>f</mi><mo>′</mo></msup>')).toBe("f'");
    expect(m('<msup><mi>e</mi><mrow><mo>−</mo><mi>β</mi><mi>E</mi></mrow></msup>')).toBe(
      'e^{-\\beta E}'
    );
  });

  it('converts munderover on big operators, and accents', () => {
    expect(
      m(
        '<munderover><mo>∑</mo><mrow><mi>i</mi><mo>=</mo><mn>1</mn></mrow><mi>n</mi></munderover><msub><mi>a</mi><mi>i</mi></msub>'
      )
    ).toBe('\\sum_{i=1}^n a_i');
    expect(m('<munder><mo>lim</mo><mrow><mi>x</mi><mo>→</mo><mn>0</mn></mrow></munder>')).toBe(
      '\\lim_{x\\to 0}'
    );
    expect(m('<mover accent="true"><mi>x</mi><mo>^</mo></mover>')).toBe('\\hat{x}');
    expect(m('<mover><mrow><mi>a</mi><mi>b</mi></mrow><mo>¯</mo></mover>')).toBe('\\overline{ab}');
  });

  it('converts mtable inside fences to a matrix, and right/left columns to aligned', () => {
    expect(
      m(
        '<mrow><mo>(</mo><mtable><mtr><mtd><mi>a</mi></mtd><mtd><mi>b</mi></mtd></mtr><mtr><mtd><mi>c</mi></mtd><mtd><mi>d</mi></mtd></mtr></mtable><mo>)</mo></mrow>'
      )
    ).toBe('\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}');
    expect(
      m(
        '<mtable columnalign="right left"><mtr><mtd><mi>x</mi></mtd><mtd><mo>=</mo><mn>1</mn></mtd></mtr><mtr><mtd><mi>y</mi></mtd><mtd><mo>=</mo><mn>2</mn></mtd></mtr></mtable>'
      )
    ).toBe('\\begin{aligned} x & = 1 \\\\ y & = 2 \\end{aligned}');
  });

  it('converts mathvariant to font commands', () => {
    expect(m('<mi mathvariant="double-struck">R</mi>')).toBe('\\mathbb{R}');
    expect(m('<mi mathvariant="script">L</mi>')).toBe('\\mathcal{L}');
    expect(m('<mi mathvariant="bold">x</mi>')).toBe('\\mathbf{x}');
    expect(m('<mi mathvariant="fraktur">g</mi>')).toBe('\\mathfrak{g}');
    expect(m('<mi mathvariant="normal">d</mi><mi>x</mi>')).toBe('\\mathrm{d}x');
    expect(m('<mi>sin</mi><mo>&#x2061;</mo><mi>θ</mi>')).toBe('\\sin\\theta');
  });

  it('reads namespaced JATS formulas, preferring tex-math, with their labels', () => {
    const xml = `<article><body>
      <disp-formula id="e1"><label>(1)</label><mml:math><mml:mi>E</mml:mi><mml:mo>=</mml:mo><mml:mi>m</mml:mi><mml:msup><mml:mi>c</mml:mi><mml:mn>2</mml:mn></mml:msup></mml:math></disp-formula>
      <inline-formula><alternatives><tex-math><![CDATA[\\documentclass{minimal}\\begin{document}$\\alpha_0$\\end{document}]]></tex-math><mml:math><mml:msub><mml:mi>α</mml:mi><mml:mn>0</mml:mn></mml:msub></mml:math></alternatives></inline-formula>
    </body></article>`;
    const formulas = ext.jatsFormulas(xml);
    expect(formulas).toEqual([
      { tex: 'E = mc^2', env: 'disp-formula', display: true, numbers: ['1'] },
      { tex: '\\alpha_0', env: 'inline-formula', display: false, numbers: [] },
    ]);
  });
});

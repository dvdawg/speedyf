/** Math to LaTeX reading a selection of prose and math as Markdown, and
 * finding a quoted passage in a paper's TeX source.
 *
 * The selections are built glyph by glyph the way the host reports them
 * (pdfTeX fonts, scripts raised or lowered and smaller), so what is tested
 * is the reading of geometry and fonts: what is math, what is prose, where
 * a display is, and how lines and paragraphs join. */
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
interface Doc {
  body: string;
  macros: Map<string, string>;
}
interface Ext {
  selectionMarkdown(
    glyphs: Glyph[],
    opts?: { text?: string }
  ): { markdown: string; confidence: string; math: number } | null;
  texDocument(files: { name: string; text: string }[]): Doc;
  exactQuote(doc: Doc, quote: string): { markdown: string; exact: boolean } | null;
}

const ext = new Function(
  'self',
  `${SOURCE}\nreturn { selectionMarkdown, texDocument, exactQuote };`
)({}) as Ext;

/** Glyphs typeset one at a time: `at` moves the pen to a line, `put` sets
 * text in a font (a space is a word space), `size` and `dy` make a
 * script. */
function page() {
  const glyphs: Glyph[] = [];
  let x = 0;
  let base = 0;
  return {
    glyphs,
    at(nx: number, nb: number) {
      x = nx;
      base = nb;
    },
    put(text: string, font: string, opts: { size?: number; dy?: number; italic?: boolean } = {}) {
      const size = opts.size ?? 10;
      const dy = opts.dy ?? 0;
      for (const ch of Array.from(text)) {
        if (ch === ' ') {
          x += 0.33 * size;
          continue;
        }
        const w = 0.5 * size;
        glyphs.push({
          text: ch,
          code: ch.codePointAt(0)!,
          font,
          size,
          x,
          y: base + dy - 0.75 * size,
          w,
          h: size,
          baseline: base + dy,
          bold: false,
          italic: Boolean(opts.italic) || /CMMI|CMTI/.test(font),
        });
        x += w;
      }
    },
    space() {
      x += 3.3;
    },
    text() {
      return glyphs.map((g) => g.text).join('');
    },
  };
}

const sup = { size: 7, dy: -3.6 };
const sub = { size: 7, dy: 1.5 };

/** The paragraph from the bug report: inline math with scripts, a norm, set
 * symbols and relations, a displayed inequality with its number, prose
 * with an italic word, a ligature and a word hyphenated at a line end. */
function reportParagraph() {
  const p = page();
  p.at(72, 100);
  p.put('Let VC(', 'CMR10');
  p.put('C', 'CMMI10');
  p.put(')', 'CMR10');
  p.space();
  p.put('=', 'CMR10');
  p.space();
  p.put('d', 'CMMI10');
  p.space();
  p.put('\u2265', 'CMSY10');
  p.space();
  p.put('1', 'CMR10');
  p.space();
  p.put('and ', 'CMR10');
  p.put('A', 'CMMI10');
  p.put('(', 'CMR10');
  p.put('S', 'CMMI10');
  p.put(')', 'CMR10');
  p.space();
  p.put('\u2208', 'CMSY10');
  p.space();
  p.put('C', 'CMMI10');
  p.space();
  p.put('for every sample ', 'CMR10');
  p.put('S', 'CMMI10');
  p.space();
  p.put('\u2208', 'CMSY10');
  p.space();
  p.put('X', 'CMMI10');
  p.put('m', 'CMMI7', sup);
  p.put('.', 'CMR10');
  p.space();
  p.put('If', 'CMR10');
  p.at(72, 112);
  p.put('S', 'CMMI10');
  p.put(',', 'CMMI10');
  p.space();
  p.put('T', 'CMMI10');
  p.space();
  p.put('\u223C', 'CMSY10');
  p.space();
  p.put('P', 'CMMI10');
  p.put('m', 'CMMI7', sup);
  p.space();
  p.put('are inde-', 'CMR10');
  p.at(72, 124);
  p.put('pendent, the \uFB01rst bound holds with the norm ', 'CMR10');
  p.put('\u2016', 'CMSY10');
  p.put('P', 'CMMI10');
  p.put('\u2016', 'CMSY10');
  p.put('q', 'CMMI7', sub);
  p.put(':', 'CMR10');
  // The display, centred, numbered at the right margin.
  p.at(150, 142);
  p.put('\u2016', 'CMSY10');
  p.put('P', 'CMMI10');
  p.put('(', 'CMR10');
  p.put('A', 'CMMI10');
  p.put('(', 'CMR10');
  p.put('S', 'CMMI10');
  p.put(')', 'CMR10');
  p.space();
  p.put('\u2229', 'CMSY10');
  p.space();
  p.put('A', 'CMMI10');
  p.put('(', 'CMR10');
  p.put('T', 'CMMI10');
  p.put('))', 'CMR10');
  p.put('\u2016', 'CMSY10');
  p.put('L', 'CMMI7', sub);
  p.put('q', 'CMMI5', { size: 5, dy: 2.5 });
  p.space();
  p.put('\u2264', 'CMSY10');
  p.space();
  p.put('62400', 'CMR10');
  p.put('d', 'CMMI10');
  p.space();
  p.put('+', 'CMR10');
  p.space();
  p.put('qm', 'CMMI10');
  p.put('.', 'CMMI10');
  p.at(480, 142);
  p.put('(3)', 'CMR10');
  p.at(72, 160);
  p.put('where ', 'CMR10');
  p.put('every', 'CMTI10');
  p.put(' word stays prose.', 'CMR10');
  return p;
}

const REPORT_MARKDOWN = [
  'Let $\\operatorname{VC}(C) = d \\geq 1$ and $A(S) \\in C$ for every sample $S \\in X^m$. ' +
    'If $S, T \\sim P^m$ are independent, the first bound holds with the norm $\\|P\\|_q$:',
  '$$ \\|P(A(S) \\cap A(T))\\|_{L_q} \\leq 62400d + qm. \\tag{3} $$',
  'where every word stays prose.',
].join('\n\n');

describe('Math to LaTeX: a selection as Markdown', () => {
  it('writes inline math, scripts, norms and a numbered display, prose as prose', () => {
    const p = reportParagraph();
    const read = ext.selectionMarkdown(p.glyphs, { text: p.text() });
    expect(read).not.toBeNull();
    expect(read!.markdown).toBe(REPORT_MARKDOWN);
    expect(read!.math).toBe(6);
    expect(read!.confidence).toBe('high');
  });

  it('never wraps prose in math: italic words, emphasis, a lone "a"', () => {
    const p = page();
    p.at(72, 100);
    p.put('We call it ', 'CMR10');
    p.put('the main result', 'CMTI10');
    p.put(', a fact; see Section 3 (and [12]).', 'CMR10');
    p.at(72, 112);
    p.put('A word processor sets ', 'TimesNewRomanPSMT');
    p.put('this phrase', 'TimesNewRomanPS-ItalicMT', { italic: true });
    p.put(' and ', 'TimesNewRomanPSMT');
    p.put('a', 'TimesNewRomanPS-ItalicMT', { italic: true });
    p.put(' word in italics.', 'TimesNewRomanPSMT');
    expect(ext.selectionMarkdown(p.glyphs, { text: p.text() })).toBeNull();
  });

  it('reads a lone italic letter in a text font as a variable, among upright words', () => {
    const p = page();
    p.at(72, 100);
    p.put('Each ', 'TimesNewRomanPSMT');
    p.put('x', 'TimesNewRomanPS-ItalicMT', { italic: true });
    p.put(' \u2208 ', 'TimesNewRomanPSMT');
    p.put('X', 'TimesNewRomanPS-ItalicMT', { italic: true });
    p.put(' is a point, and so is the next.', 'TimesNewRomanPSMT');
    expect(ext.selectionMarkdown(p.glyphs, { text: p.text() })!.markdown).toBe(
      'Each $x \\in X$ is a point, and so is the next.'
    );
  });

  it('spells out ligatures, rejoins hyphenated words and keeps paragraphs', () => {
    const p = page();
    p.at(72, 100);
    p.put('The \uFB01eld ', 'CMR10');
    p.put('F', 'CMMI10');
    p.put(' is ef-', 'CMR10');
    p.at(72, 112);
    p.put('\uFB01cient and *starred*.', 'CMR10');
    // A new paragraph: indented.
    p.at(84, 124);
    p.put('Next one.', 'CMR10');
    expect(ext.selectionMarkdown(p.glyphs, { text: p.text() })!.markdown).toBe(
      'The field $F$ is efficient and \\*starred\\*.\n\nNext one.'
    );
  });

  it('answers nothing for glyphs that do not cover the selected text', () => {
    const p = reportParagraph();
    const text = `${p.text()} and a second page of words that the glyphs here do not hold at all`;
    expect(ext.selectionMarkdown(p.glyphs.slice(0, 40), { text })).toBeNull();
  });
});

const PAPER = String.raw`\documentclass{article}
\newcommand{\VC}{\operatorname{VC}}
\newcommand{\cC}{\mathcal{C}}
\newcommand{\norm}[1]{\left\|#1\right\|}
\begin{document}
\section{Introduction}
Earlier text about learning~\cite{vapnik}. % a comment the page never shows
Unrelated words fill this paragraph, so that nothing matches here.

Let $\VC(\cC) = d \geq 1$ and $A(S) \in \cC$ for every sample $S \in X^m$. % note
If $S, T \sim P^m$ are inde\-pendent, the \emph{first} bound holds with the norm $\norm{P}_q$:
\begin{equation}\label{eq:main}
  \|P(A(S)\cap A(T))\|_{L_q} \le 62400\, d + qm.
\end{equation}
where every word stays prose, as in~\cite{smith}.
\end{document}
`;

describe('Math to LaTeX: a quote found in the TeX source', () => {
  const doc = ext.texDocument([{ name: 'paper.tex', text: PAPER }]);

  it('finds the passage and writes it from the source, math as the author wrote it', () => {
    const quote = REPORT_MARKDOWN.replace('prose.', 'prose, as in \\[12\\].');
    const found = ext.exactQuote(doc, quote);
    expect(found).not.toBeNull();
    expect(found!.markdown).toBe(
      [
        'Let $\\operatorname{VC}(\\mathcal{C}) = d \\geq 1$ and $A(S) \\in \\mathcal{C}$ for every ' +
          'sample $S \\in X^m$. If $S, T \\sim P^m$ are independent, the _first_ bound holds ' +
          'with the norm ${\\left\\|P\\right\\|}_q$:',
        // The number the page printed; the label and comments gone.
        '$$ \\|P(A(S)\\cap A(T))\\|_{L_q} \\le 62400\\, d + qm. \\tag{3} $$',
        // A citation reads as the page printed it.
        'where every word stays prose, as in \\[12\\].',
      ].join('\n\n')
    );
    expect(found!.exact).toBe(true);
  });

  it('writes a citation the page did not show as [ref]', () => {
    const found = ext.exactQuote(
      doc,
      'Earlier text about learning. Unrelated words fill this paragraph, so that nothing matches here.'
    );
    expect(found!.markdown).toBe(
      'Earlier text about learning \\[ref\\]. Unrelated words fill this paragraph, so that nothing matches here.'
    );
  });

  it('finds nothing for a passage the paper does not have', () => {
    expect(
      ext.exactQuote(
        doc,
        'A passage about something else entirely, which this paper does not contain anywhere.'
      )
    ).toBeNull();
    // Too few of its words, even in order.
    expect(
      ext.exactQuote(doc, 'Let the bound hold for every sample of words we never wrote here.')
    ).toBeNull();
  });
});

/** A fake arXiv source: theorem-like environments with and without a
 * name, a `\label`, a starred environment, and a `.bbl` that says what each
 * key prints. */
const STATEMENTS = String.raw`\documentclass{article}
\newtheorem{theorem}{Theorem}[section]
\newtheorem{proposition}[theorem]{Proposition}
\begin{document}
Some earlier words in the paper that do not matter at all for this.
\begin{proposition}[Moments of two block overlaps]\label{prop:moments}
Let $P$ be a distribution on the cube, as in~\cite{simon}. Then every pair
of block overlaps has bounded moments~\cite{han,larsen}.
\end{proposition}
\begin{theorem*}\label{thm:bw}
Every bounded sequence of real numbers has, by~\cite{simon}, a convergent subsequence.
\end{theorem*}
\bibliography{refs}
\end{document}
`;
const BBL = String.raw`\begin{thebibliography}{Lar23}
\bibitem[Sim15]{simon} B.~Simon, \emph{A comprehensive course}, 2015.
\bibitem[Han16]{han} H.~Han, Overlaps, 2016.
\bibitem[Lar23]{larsen} L.~Larsen, Blocks, 2023.
\end{thebibliography}
`;

describe('Math to LaTeX: a statement found in the TeX source', () => {
  const doc = ext.texDocument([
    { name: 'paper.tex', text: STATEMENTS },
    { name: 'paper.bbl', text: BBL },
  ]);

  it('writes a named statement header, never leaking the "]" of its name', () => {
    const found = ext.exactQuote(
      doc,
      'Proposition 2.2 (Moments of two block overlaps). Let $P$ be a distribution on the cube, ' +
        'as in \\[Sim15\\]. Then every pair of block overlaps has bounded moments \\[Han16, Lar23\\].'
    );
    expect(found!.markdown).toBe(
      '**Proposition 2.2** (Moments of two block overlaps). Let $P$ be a distribution on the ' +
        'cube, as in \\[Sim15\\]. Then every pair of block overlaps has bounded moments ' +
        '\\[Han16, Lar23\\].'
    );
    expect(found!.markdown).not.toContain(']]');
  });

  it('takes the header in when the quote starts inside the name or after the label', () => {
    const inside = ext.exactQuote(
      doc,
      'two block overlaps). Let $P$ be a distribution on the cube, as in \\[Sim15\\]. Then every pair'
    );
    expect(
      inside!.markdown.startsWith('**Proposition** (Moments of two block overlaps). Let')
    ).toBe(true);
    const after = ext.exactQuote(
      doc,
      'Let $P$ be a distribution on the cube, as in \\[Sim15\\]. Then every pair of block overlaps'
    );
    expect(after!.markdown.startsWith('**Proposition** (Moments of two block overlaps). Let')).toBe(
      true
    );
  });

  it('reads a starred environment and its label, and cites by the bibliography', () => {
    const found = ext.exactQuote(
      doc,
      'Theorem. Every bounded sequence of real numbers has, by, a convergent subsequence.'
    );
    // A citation the page's text did not pair: the .bbl says what it prints.
    expect(found!.markdown).toBe(
      '**Theorem**. Every bounded sequence of real numbers has, by \\[Sim15\\], a convergent subsequence.'
    );
    expect(found!.markdown).not.toContain('thm:bw');
  });
});

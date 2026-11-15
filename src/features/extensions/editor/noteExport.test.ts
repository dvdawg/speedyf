/** A note exported as a Markdown file of its own, and as LaTeX. */
import { describe, expect, it } from 'vitest';
import {
  escapeLatex,
  exportAssetName,
  exportFolderName,
  exportMarkdown,
  noteToLatex,
} from './noteExport';

const NOTE = [
  '# Fluctuation theorems',
  '',
  'The *Jarzynski* equality @jarzynski1997 holds for **any** protocol; see @a; @b and [@c; @d].',
  '',
  '## Results & costs',
  '',
  '- 50% of runs, cost $5 with `code_x`',
  '  - nested_item #1',
  '- second ~ item',
  '',
  '1. first',
  '2. second',
  '',
  '> The bound $\\langle e^{-\\beta W} \\rangle = e^{-\\beta \\Delta F}$ is tight.',
  '>',
  '> $$ W \\geq \\Delta F $$',
  '> As shown [@crooks1999, p. 12]{box=1,2,3,4}.',
  '',
  '$$',
  '\\int_0^1 f(x)\\,dx',
  '$$',
  '',
  '### Figure',
  '',
  '![Region, p. 3](assets/region-1.png)',
  '',
  'Pages [@zd24, pp. 3-5] and \\_escaped\\_ text \\ back.',
].join('\n');

describe('export as Markdown', () => {
  it('names the picture folder after the file', () => {
    expect(exportFolderName('/Users/me/Thesis notes.md')).toBe('Thesis notes-assets');
    expect(exportFolderName('C:\\notes\\Thesis.markdown')).toBe('Thesis-assets');
    expect(exportAssetName('assets/region-1.png')).toBe('region-1.png');
    expect(exportAssetName('figs/a.png')).toBe('figs/a.png');
  });

  it('links pictures into the folder beside it and drops the page boxes', () => {
    const note =
      'See [@crooks1999, p. 12]{box=1,2,3,4} and @a.\n' +
      '![Region](assets/region-1.png) ![Again](assets/region-1.png)\n' +
      '![Web](https://example.com/x.png) `[@k]{box=1,2,3,4}` and ![x](figs/b.png)\n' +
      '```\n![code](assets/c.png) [@k]{box=1,2,3,4}\n```\n' +
      '$$ [@k]{box=1} $$';
    const out = exportMarkdown(note, 'My Thesis-assets');
    expect(out.text).toBe(
      'See [@crooks1999, p. 12] and @a.\n' +
        '![Region](My%20Thesis-assets/region-1.png) ![Again](My%20Thesis-assets/region-1.png)\n' +
        '![Web](https://example.com/x.png) `[@k]{box=1,2,3,4}` and ![x](My%20Thesis-assets/figs/b.png)\n' +
        '```\n![code](assets/c.png) [@k]{box=1,2,3,4}\n```\n' +
        '$$ [@k]{box=1} $$'
    );
    expect(out.assets).toEqual(['assets/region-1.png', 'figs/b.png']);
  });

  it('leaves a note with nothing to change as it is', () => {
    const plain = '# Notes\n\nJust @a, with $x_1$.';
    expect(exportMarkdown(plain, 'n-assets')).toEqual({ text: plain, assets: [] });
  });
});

describe('copy as LaTeX', () => {
  it('converts a representative note', () => {
    expect(noteToLatex(NOTE, { key: (k) => (k === 'a' ? 'alpha2020' : k) })).toBe(
      [
        '\\section{Fluctuation theorems}',
        '',
        'The \\emph{Jarzynski} equality \\cite{jarzynski1997} holds for \\textbf{any} protocol; see \\cite{alpha2020,b} and \\cite{c,d}.',
        '',
        '\\subsection{Results \\& costs}',
        '',
        '\\begin{itemize}',
        '  \\item 50\\% of runs, cost \\$5 with \\texttt{code\\_x}',
        '  \\begin{itemize}',
        '    \\item nested\\_item \\#1',
        '  \\end{itemize}',
        '  \\item second \\textasciitilde{} item',
        '\\end{itemize}',
        '',
        '\\begin{enumerate}',
        '  \\item first',
        '  \\item second',
        '\\end{enumerate}',
        '',
        '\\begin{quote}',
        'The bound $\\langle e^{-\\beta W} \\rangle = e^{-\\beta \\Delta F}$ is tight.',
        '',
        '\\[ W \\geq \\Delta F \\]',
        'As shown \\cite[p.~12]{crooks1999}.',
        '\\end{quote}',
        '',
        '\\[',
        '\\int_0^1 f(x)\\,dx',
        '\\]',
        '',
        '\\subsubsection{Figure}',
        '',
        '% figure: assets/region-1.png (Region, p. 3)',
        '',
        'Pages \\cite[pp.~3--5]{zd24} and \\_escaped\\_ text \\textbackslash{} back.',
        '',
      ].join('\n')
    );
  });

  it('escapes every special character in prose, and nothing in math', () => {
    expect(escapeLatex('\\ { } $ & # _ % ^ ~')).toBe(
      '\\textbackslash{} \\{ \\} \\$ \\& \\# \\_ \\% \\textasciicircum{} \\textasciitilde{}'
    );
    expect(noteToLatex('100% of $a_1 \\% b$ and $$ x_{%} $$')).toBe(
      '100\\% of $a_1 \\% b$ and \\$\\$ x\\_\\{\\%\\} \\$\\$\n'
    );
    expect(noteToLatex('$$ a_1 & b^2 $$')).toBe('\\[\na_1 & b^2\n\\]\n');
  });

  it('keeps a picture inside a line on a comment line of its own', () => {
    expect(noteToLatex('Before ![a](assets/x.png) after')).toBe(
      'Before\n% figure: assets/x.png (a)\nafter\n'
    );
  });
});

describe('statements, cross-references and quoted references on export', () => {
  const QUOTED = [
    '> **Proposition 2.2** (Moments). Let $P$ be as in [^Sim15] and [@han2016].',
    '>',
    '> [@rz26, p. 2]{box=72,100,468,80}',
    '',
    'By [Theorem 2.1](@rz26#p1){box=72,300,400,60}, and [@rz26, Prop. 2.2]{box=72,100,468,80 page=2}.',
    '',
    '[^Sim15]: [Sim15] B. Simon, Spectral_theory (2015).',
  ].join('\n');

  it('keeps Pandoc in Markdown: citations and footnotes, cross-references as words', () => {
    expect(exportMarkdown(QUOTED, 'n-assets').text).toBe(
      [
        '> **Proposition 2.2** (Moments). Let $P$ be as in [^Sim15] and [@han2016].',
        '>',
        '> [@rz26, p. 2]',
        '',
        'By Theorem 2.1, and [@rz26, Prop. 2.2].',
        '',
        '[^Sim15]: [Sim15] B. Simon, Spectral_theory (2015).',
      ].join('\n')
    );
  });

  it('writes LaTeX: the statement set as amsthm does, its citation with a locator', () => {
    expect(noteToLatex(QUOTED)).toBe(
      [
        '\\begin{quote}',
        '\\textbf{Proposition 2.2} (Moments). {\\itshape Let $P$ be as in [Sim15]\\footnote{B. Simon, Spectral\\_theory (2015).} and \\cite{han2016}.}',
        '',
        '\\cite[p.~2]{rz26}',
        '\\end{quote}',
        '',
        'By Theorem 2.1, and \\cite[Prop.~2.2]{rz26}.',
        '',
      ].join('\n')
    );
  });
});

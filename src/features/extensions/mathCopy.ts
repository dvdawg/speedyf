/** The forms a formula is copied in from the math popover (`ui.showMath`).
 *
 * Pure, so each format is pinned by a test: a copy button that quietly
 * produced slightly wrong LaTeX would be worse than none.
 */
import type { MathConfidence, MathSource } from './api';

/** Environments whose body is already align-shaped: rows split by `\\`,
 * columns by `&`. `gathered` has no columns, which `align` accepts. */
const ALIGNISH =
  /^\\begin\{(aligned|alignedat|split|gathered|align\*?|eqnarray\*?)\}(\{\d+\})?([\s\S]*)\\end\{\1\}$/;

/** Whether the LaTeX breaks lines, which is what makes an `align` copy worth
 * offering. */
export function hasLineBreaks(latex: string): boolean {
  return latex.includes('\\\\');
}

/** As raw LaTeX: exactly what the extension produced, trimmed. */
export function latexCopy(latex: string): string {
  return latex.trim();
}

/** As Markdown math: `$...$` inline, `$$...$$` displayed. */
export function markdownCopy(latex: string, display: boolean): string {
  const body = latex.trim();
  return display ? `$$${body}$$` : `$${body}$`;
}

/** As an `align` environment. A formula that is already one aligned block
 * (`aligned`, `split`, `gathered`, or `align` itself) has its rows moved
 * across rather than nested; anything else becomes the body as is. A trailing
 * `\\` would print an empty numbered row, so it is dropped. */
export function alignCopy(latex: string): string {
  let body = latex.trim();
  const match = ALIGNISH.exec(body);
  if (match) body = (match[3] ?? '').trim();
  body = body.replace(/\\\\\s*$/, '').trim();
  return `\\begin{align}\n${body}\n\\end{align}`;
}

/** What KaTeX is asked to typeset. Bare rows (`a \\ b` with no environment)
 * are wrapped in `aligned` so they stack as they did on the page, and an
 * `align` the extension produced is shown as the `aligned` KaTeX supports. */
export function renderSource(latex: string): string {
  const body = latex.trim();
  const match = ALIGNISH.exec(body);
  if (match && match[1] !== 'aligned' && match[1] !== 'gathered' && match[1] !== 'alignedat') {
    return `\\begin{aligned}${match[3] ?? ''}\\end{aligned}`;
  }
  if (hasLineBreaks(body) && !body.includes('\\begin{')) {
    return `\\begin{aligned}${body}\\end{aligned}`;
  }
  return body;
}

const SOURCE_WORDS: Record<MathSource, string> = {
  tags: 'PDF tags',
  arxiv: 'arXiv source',
  jats: 'journal version',
  fonts: 'fonts',
};

/** The dim note under the formula: where it came from and how sure. */
export function provenanceNote(source: MathSource, confidence: MathConfidence): string {
  return `from ${SOURCE_WORDS[source]}, ${confidence} confidence`;
}

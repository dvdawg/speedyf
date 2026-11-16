/** Whether a quote's LaTeX (`quoteTex`) can be trusted to say what the
 * page says. A converter reading glyphs can misread a passage badly - words
 * taken apart and their letters spread through the formulas, margin line
 * numbers, symbols from the wrong font - and a quote that renders as
 * nonsense is worse than the page's own text. So a reading is kept only
 * when it passes every check here, and KaTeX parses each of its formulas
 * (`texParses`); otherwise the comment shows the page itself and quotes its
 * plain text.
 *
 * `plausibleQuoteTex` is pure and synchronous, so a review read from disk is
 * checked again as it loads. */
import { splitMath } from '../../components/mathSplit';

const DISPLAY = /\$\$([^$]+?)\$\$/g;

/** Characters no faithful reading contains: the replacement character,
 * private-use code points a font left unmapped, a dotless i (what a TeX
 * font's glyph names often decode to), and the marks of text decoded in
 * the wrong encoding. */
const MOJIBAKE = /[\uFFFD\uE000-\uF8FF\u0131]|\u00C3[\u0080-\u00BF]|\u00C2[\u0080-\u00BF]/;

/** Big operators and relations: in a quote's prose they are math the
 * reading failed to set as math. */
const OPERATOR_IN_PROSE = /[\u2200-\u22FF\u2A00-\u2AFF]/;

/** Commands whose argument is words, not math. */
const TEXT_COMMANDS =
  /\\(?:text|textrm|textit|textbf|mathrm|operatorname|mbox|mathsf|mathtt|mathit|mathbf|begin|end)\s*\{[^{}]*\}/g;

/** Of a quote's prose words (three letters or more), the share that must be
 * words of the page's own text. */
const WORDS_FROM_PAGE = 0.8;

function words(text: string): string[] {
  return (text.toLowerCase().match(/[a-z]{3,}/g) ?? []).filter(Boolean);
}

/** The formulas and the prose of a quote. */
function parts(tex: string): { math: string[]; prose: string[]; displays: Set<number> } {
  const math: string[] = [];
  /** which of `math` are displays */
  const displays = new Set<number>();
  const prose: string[] = [];
  let at = 0;
  const flat = tex.replace(/\s+/g, ' ');
  const inline = (text: string) => {
    for (const segment of splitMath(text)) {
      if (segment.kind === 'text') prose.push(segment.text);
      else math.push(segment.src);
    }
  };
  for (const m of flat.matchAll(DISPLAY)) {
    inline(flat.slice(at, m.index));
    displays.add(math.length);
    math.push(m[1]!);
    at = m.index + m[0].length;
  }
  inline(flat.slice(at));
  return { math, prose, displays };
}

/** A formula that reads as one: balanced braces, scripts on something,
 * and no run of four or more letters outside a command or a text argument
 * - variables are letters one at a time, and a word inside a formula is
 * prose the reading swallowed. */
export function plausibleFormula(src: string): boolean {
  let depth = 0;
  for (const ch of src) {
    if (ch === '{') depth += 1;
    if (ch === '}') depth -= 1;
    if (depth < 0) return false;
  }
  if (depth !== 0) return false;
  // A script on nothing ("(_{a}", "e^{-^{b}"), or a block opening on an
  // empty row: a layout the reading did not understand.
  if (/(^|[({[,=&]|\\left.|\\\\)\s*[_^]/.test(src) || /[_^]\{?\s*[-+]?\s*[_^]/.test(src)) {
    return false;
  }
  if (/\\begin\{[a-z]+\*?\}\s*\\\\/.test(src)) return false;
  const bare = src.replace(TEXT_COMMANDS, ' ').replace(/\\[A-Za-z]+/g, ' ');
  return !/[A-Za-z]{4,}/.test(bare);
}

/** Whether `tex` is a faithful reading of a quote whose plain text (as the
 * page gave it, for a selection; empty for an equation on its own) is
 * `plain`. */
export function plausibleQuoteTex(tex: string, plain: string): boolean {
  const text = tex.trim();
  if (!text || MOJIBAKE.test(text)) return false;
  const { math, prose } = parts(text);
  if (math.length === 0) return false;
  if (!math.every(plausibleFormula)) return false;
  // A formula that could not be read stands in the quote as its plain
  // text in italics (`fillQuote`): the page's words, not a reading's, so
  // the checks of a reading's prose do not apply to it.
  const proseText = prose.join(' ').replace(/(^|[^\\])\*(?:[^*\\]|\\.)*\*/g, '$1 ');
  if (OPERATOR_IN_PROSE.test(proseText)) return false;
  // A number of three digits or more in the prose that the page's text
  // does not have: a margin line number the reading picked up.
  const plainNumbers = new Set(plain.match(/\d{3,}/g) ?? []);
  for (const n of proseText.match(/\d{3,}/g) ?? []) {
    if (!plainNumbers.has(n)) return false;
  }
  const said = words(proseText);
  if (said.length > 0) {
    const page = new Set(words(plain));
    if (page.size === 0) return false;
    const found = said.filter((w) => page.has(w)).length;
    if (found < WORDS_FROM_PAGE * said.length) return false;
  }
  return true;
}

type Katex = { renderToString(src: string, options: object): string };
let katex: Promise<Katex | null> | null = null;

/** Whether KaTeX parses every formula in `tex`. False when KaTeX cannot be
 * loaded: an unchecked reading is not kept. */
export async function texParses(tex: string): Promise<boolean> {
  katex ??= import('katex')
    .then((mod) => ((mod as { default?: Katex }).default ?? mod) as Katex)
    .catch(() => null);
  const k = await katex;
  if (!k) return false;
  const { math, displays } = parts(tex);
  return math.every((src, i) => {
    try {
      k.renderToString(src, {
        displayMode: displays.has(i),
        throwOnError: true,
        strict: 'ignore',
        trust: false,
      });
      return true;
    } catch {
      return false;
    }
  });
}

/** A converter's formula tidied: no variation selectors (a font's hint
 * that came through as text), no empty rows at the ends of a block, and a
 * block of one row as that row. */
export function tidyTex(tex: string): string {
  let t = '';
  for (const ch of tex) {
    const cp = ch.codePointAt(0)!;
    if (cp < 0xfe00 || cp > 0xfe0f) t += ch;
  }
  t = t.replace(/\s+/g, ' ').trim();
  const block = /^\\begin\{(gathered|aligned)\}([\s\S]*)\\end\{\1\}$/.exec(t);
  if (block) {
    const rows = block[2]!.split('\\\\').map((r) => r.trim());
    while (rows.length > 0 && rows[0] === '') rows.shift();
    while (rows.length > 0 && rows.at(-1) === '') rows.pop();
    t =
      rows.length === 1 && block[1] === 'gathered'
        ? rows[0]!
        : `\\begin{${block[1]}} ${rows.join(' \\\\ ')} \\end{${block[1]}}`;
  }
  return t;
}

/** A formula as a converter read it, tidied, when it can be trusted; null
 * otherwise. */
export async function trustedFormula(
  tex: string | null | undefined,
  display: boolean
): Promise<string | null> {
  const t = tex ? tidyTex(tex) : '';
  if (!t || !plausibleFormula(t)) return null;
  return (await texParses(display ? `$$ ${t} $$` : `$${t}$`)) ? t : null;
}

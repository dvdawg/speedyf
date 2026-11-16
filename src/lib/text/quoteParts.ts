/** A selection taken apart for quoting: its prose, the formulas set inline
 * in it, and the display equations it covers, so each can be read the way
 * it reads best. The prose is the page's own text (cleaned as a copy is:
 * margin numbers gone, lines joined, words broken at a line end rejoined);
 * each inline formula and each display is converted on its own, small and
 * with its own geometry, and the quote is assembled from the answers -
 * where one fails, its plain text stands in for it alone.
 *
 * Which characters are math: those the engine saw set in a math font (a
 * run's `math` stretches), and mathematical symbols in any font. A formula
 * is a stretch of words that are math, with the operators, numbers and
 * brackets that join them ("x_i = 0" whole, not "x_i" and then "= 0"),
 * and never the prose words around it.
 *
 * Pure. Runs in page points, y up. */
import type { QuotePiece, SpanDisplay } from './copySelection';
import { smartCopyText, visualOrder, type CopyRun } from './smartCopy';
import { splitMath } from '../../components/mathSplit';

export interface InlinePart {
  key: string;
  /** what the page says there */
  text: string;
  page: number;
  /** where it is, `[x, y, w, h]` per run it spans, page points y up */
  rects: [number, number, number, number][];
}

export interface DisplayPart {
  key: string;
  text: string;
  page: number;
  /** the engine's box for the equation, `[x, y, w, h]` y up */
  block: [number, number, number, number];
  number: string | null;
}

export interface QuotePlan {
  /** the quote as plain text */
  plain: string;
  /** the quote with each part a placeholder character (`key`) */
  template: string;
  inline: InlinePart[];
  displays: DisplayPart[];
}

/** Placeholders: characters from the private use area, which no page text
 * reaching a quote contains. */
const FIRST_KEY = 0xe000;

/** A code point that is a mathematical symbol in any font: Greek letters,
 * letterlike symbols, arrows, operators, delimiters, mathematical letters. */
export function isMathSymbol(cp: number): boolean {
  return (
    (cp >= 0x391 && cp <= 0x3a9) ||
    (cp >= 0x3b1 && cp <= 0x3c9) ||
    cp === 0x3d1 ||
    cp === 0x3d5 ||
    cp === 0x3f0 ||
    cp === 0x3f1 ||
    cp === 0x3f5 ||
    (cp >= 0x2100 && cp <= 0x214f) ||
    (cp >= 0x2190 && cp <= 0x21ff) ||
    (cp >= 0x2200 && cp <= 0x22ff) ||
    (cp >= 0x2308 && cp <= 0x230b) ||
    (cp >= 0x27c0 && cp <= 0x27ef) ||
    (cp >= 0x2980 && cp <= 0x2aff) ||
    (cp >= 0x1d400 && cp <= 0x1d7ff)
  );
}

/** Function names set upright in a formula ("log", "exp"): words, but the
 * formula's, when they stand among its symbols. */
const FUNCTIONS = new Set(
  'log ln exp sin cos tan sec csc cot sinh cosh tanh arcsin arccos arctan lim liminf limsup max min sup inf argmax argmin det dim ker deg gcd arg Pr tr diag sgn sign var cov poly'.split(
    ' '
  )
);

/** Characters that join the pieces of a formula without being words. */
const GLUE = /^[0-9()[\]{}|.,;:'!/*^_~=<>+-]$/;
/** Operators: a token of these binds the formula across it. */
const OPERATOR = /[=<>+*/]|^-$/;
const MINUS = 0x2212;

type TokenKind = 'math' | 'op' | 'glue' | 'prose';

interface Token {
  from: number;
  to: number;
  kind: TokenKind;
}

/** Whether a code point is an operator or a relation: what binds the
 * numbers beside it into a formula. */
function isOperator(cp: number): boolean {
  return (
    '=<>+*/'.includes(String.fromCodePoint(cp)) ||
    cp === MINUS ||
    (cp >= 0x2190 && cp <= 0x22ff) ||
    (cp >= 0x2a00 && cp <= 0x2aff)
  );
}

function inRanges(k: number, ranges: readonly (readonly [number, number])[]): boolean {
  return ranges.some(([a, b]) => k >= a && k < b);
}

/** The stretches of `text[from, to)` that are inline formulas. */
export function mathChunks(
  text: string,
  ranges: readonly (readonly [number, number])[],
  from = 0,
  to = text.length
): [number, number][] {
  // words, by spaces
  const tokens: Token[] = [];
  let k = from;
  while (k < to) {
    while (k < to && /\s/.test(text[k]!)) k += 1;
    if (k >= to) break;
    const start = k;
    let math = false;
    let prose = false;
    let glueOnly = true;
    let bracket = false;
    let word = '';
    const words: string[] = [];
    while (k < to && !/\s/.test(text[k]!)) {
      const cp = text.codePointAt(k)!;
      const units = cp > 0xffff ? 2 : 1;
      const ch = text.slice(k, k + units);
      if (inRanges(k, ranges) || isMathSymbol(cp)) {
        math = true;
        glueOnly = false;
      } else if (/\p{L}/u.test(ch)) {
        prose = true;
        glueOnly = false;
        word += ch;
        k += units;
        continue;
      } else if (!GLUE.test(ch) && cp !== MINUS) {
        glueOnly = false;
      }
      if (/[()[\]{}|]/.test(ch)) bracket = true;
      if (word) words.push(word);
      word = '';
      k += units;
    }
    if (word) words.push(word);
    // "log(1/x)", "exp(-q)": its words are function names, among symbols
    if (prose && (math || bracket) && words.every((w) => FUNCTIONS.has(w))) {
      prose = false;
      math = true;
    }
    const token = text.slice(start, k);
    let kind: TokenKind;
    if (prose) kind = 'prose';
    else if (math) kind = 'math';
    else if (glueOnly && (OPERATOR.test(token) || token.includes(String.fromCharCode(MINUS))))
      kind = 'op';
    else if (glueOnly) kind = 'glue';
    else kind = 'prose';
    tokens.push({ from: start, to: k, kind });
  }

  const out: [number, number][] = [];
  let i = 0;
  while (i < tokens.length) {
    if (tokens[i]!.kind === 'prose') {
      i += 1;
      continue;
    }
    let j = i;
    while (j + 1 < tokens.length && tokens[j + 1]!.kind !== 'prose') j += 1;
    let a = i;
    let b = j;
    // Numbers and punctuation at the ends belong to the formula only when
    // an operator binds them to it ("x = 0"), not beside it ("(3), x").
    const starts = (t: Token) => isOperator(text.codePointAt(t.from)!);
    const ends = (t: Token) => {
      let at = t.to - 1;
      while (at > t.from && /[.,;:]/.test(text[at]!)) at -= 1;
      const low = text.charCodeAt(at);
      return isOperator(low >= 0xdc00 && low <= 0xdfff ? text.codePointAt(at - 1)! : low);
    };
    for (;;) {
      const first = tokens[a]!;
      const last = tokens[b]!;
      if (a < b && first.kind === 'glue' && !starts(tokens[a + 1]!)) a += 1;
      else if (a < b && last.kind === 'glue' && !ends(tokens[b - 1]!)) b -= 1;
      else break;
    }
    // A bracket the formula opens, it closes: "(0, 1)" read across a gap.
    const open = (from: number, to: number) => {
      let depth = 0;
      for (let c = from; c < to; c += 1) {
        if ('([{'.includes(text[c]!)) depth += 1;
        if (')]}'.includes(text[c]!)) depth -= 1;
      }
      return depth;
    };
    while (b < j && open(tokens[a]!.from, tokens[b]!.to) > 0) b += 1;
    const slice = tokens.slice(a, b + 1);
    if (slice.some((t) => t.kind === 'math')) {
      let end = tokens[b]!.to;
      // a sentence's punctuation after the formula is the sentence's
      while (end > tokens[a]!.from && /[.,;:]/.test(text[end - 1]!) && !inRanges(end - 1, ranges))
        end -= 1;
      if (end > tokens[a]!.from) out.push([tokens[a]!.from, end]);
    }
    i = j + 1;
  }
  return out;
}

/** Markdown's special characters in page text, escaped. */
export function escapeMarkdown(text: string): string {
  return text.replace(/[\\`*_$]/g, (c) => `\\${c}`);
}

function median(values: number[]): number {
  if (values.length === 0) return 10;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[sorted.length >> 1]!;
}

/** Take a selection apart into prose, inline formulas and displays. */
export function planQuote(pieces: readonly QuotePiece[]): QuotePlan {
  const inline: InlinePart[] = [];
  const displays: DisplayPart[] = [];
  const keyFor = new Map<string, InlinePart | DisplayPart>();
  let next = FIRST_KEY;
  const newKey = () => String.fromCharCode(next++);

  const sub: CopyRun[] = [];
  const byDisplay = new Map<SpanDisplay, { part: DisplayPart; run: CopyRun }>();

  // Pieces of one line are read together, so a formula the extractor
  // broke into two runs ("(0," and "1)") is found whole.
  type Piece = { run: CopyRun; from: number; to: number };
  let lineGroup: Piece[] = [];
  const edgeOf = (run: CopyRun) => {
    const n = run.text.length;
    return (k: number) =>
      run.edges && run.edges.length === n + 1 ? run.x + run.edges[k]! : run.x + (run.w * k) / n;
  };
  const centreIn = (a: CopyRun, b: CopyRun) => a.y + a.h / 2 >= b.y && a.y + a.h / 2 <= b.y + b.h;
  // overlapping by half, or a script whose centre is within the line
  const sameLine = (a: Piece, b: Piece) =>
    a.run.page === b.run.page &&
    (Math.min(a.run.y + a.run.h, b.run.y + b.run.h) - Math.max(a.run.y, b.run.y) >
      0.5 * Math.min(a.run.h, b.run.h) ||
      centreIn(a.run, b.run) ||
      centreIn(b.run, a.run));
  const flushLine = () => {
    const group = [...lineGroup].sort((p, q) => edgeOf(p.run)(p.from) - edgeOf(q.run)(q.from));
    lineGroup = [];
    if (group.length === 0) return;
    // the line's text, with where each character came from
    let text = '';
    const origin: ({ piece: number; k: number } | null)[] = [];
    const ranges: [number, number][] = [];
    group.forEach((p, i) => {
      const edge = edgeOf(p.run);
      if (i > 0) {
        const prev = group[i - 1]!;
        const gap = edge(p.from) - edgeOf(prev.run)(prev.to);
        if (gap > 0.15 * Math.min(p.run.h, prev.run.h) && !/\s$/.test(text)) {
          text += ' ';
          origin.push(null);
        }
      }
      const base = text.length;
      for (const [ma, mb] of p.run.math ?? []) {
        const lo = Math.max(ma, p.from);
        const hi = Math.min(mb, p.to);
        if (hi > lo) ranges.push([base + lo - p.from, base + hi - p.from]);
      }
      for (let k = p.from; k < p.to; k += 1) {
        text += p.run.text[k];
        origin.push({ piece: i, k });
      }
    });
    const inFormula = group.map((p) => new Array<boolean>(p.run.text.length).fill(false));
    for (const [a, b] of mathChunks(text, ranges)) {
      const spans = new Map<number, [number, number]>();
      for (let c = a; c < b; c += 1) {
        const o = origin[c];
        if (!o) continue;
        inFormula[o.piece]![o.k] = true;
        const span = spans.get(o.piece);
        if (span) span[1] = o.k + 1;
        else spans.set(o.piece, [o.k, o.k + 1]);
      }
      if (spans.size === 0) continue;
      const rects: [number, number, number, number][] = [];
      for (const [i, [ka, kb]] of spans) {
        const run = group[i]!.run;
        const edge = edgeOf(run);
        rects.push([edge(ka), run.y, Math.max(0, edge(kb) - edge(ka)), run.h]);
      }
      const key = newKey();
      const part: InlinePart = { key, text: text.slice(a, b), page: group[0]!.run.page, rects };
      inline.push(part);
      keyFor.set(key, part);
      const [firstPiece, [firstK]] = [...spans][0]!;
      const left = Math.min(...rects.map((r) => r[0]));
      const right = Math.max(...rects.map((r) => r[0] + r[2]));
      const bottom = Math.min(...rects.map((r) => r[1]));
      const top = Math.max(...rects.map((r) => r[1] + r[3]));
      sub.push({
        text: key,
        start: group[firstPiece]!.run.start + firstK / 1000,
        x: left,
        y: bottom,
        w: right - left,
        h: top - bottom,
        page: part.page,
      });
    }
    // the prose around the formulas, piece by piece
    group.forEach((p, i) => {
      const edge = edgeOf(p.run);
      let k = p.from;
      while (k < p.to) {
        if (inFormula[i]![k]) {
          k += 1;
          continue;
        }
        const a = k;
        while (k < p.to && !inFormula[i]![k]) k += 1;
        sub.push({
          text: p.run.text.slice(a, k),
          start: p.run.start + a / 1000,
          x: edge(a),
          y: p.run.y,
          w: Math.max(0, edge(k) - edge(a)),
          h: p.run.h,
          page: p.run.page,
        });
      }
    });
  };

  for (const { run, from, to, display } of pieces) {
    const last = lineGroup.at(-1);
    if (display || (last && !sameLine(last, { run, from, to }))) flushLine();
    if (display) {
      const seen = byDisplay.get(display);
      const words = run.text.slice(from, to);
      if (seen) {
        seen.part.text += ` ${words}`;
        continue;
      }
      const key = newKey();
      const part: DisplayPart = {
        key,
        text: words,
        page: run.page,
        block: [...display.block],
        number: display.number,
      };
      const [x, y, w, h] = display.box;
      const stand: CopyRun = { text: key, start: run.start, x, y, w, h, page: run.page };
      byDisplay.set(display, { part, run: stand });
      displays.push(part);
      keyFor.set(key, part);
      sub.push({ ...stand, display: display.box });
      continue;
    }
    lineGroup.push({ run, from, to });
  }
  flushLine();

  // One formula split over two runs (a gap the extractor broke the line
  // at) is one formula: neighbours on a line with only space between.
  const medianH = median(sub.map((r) => r.h));
  const ordered = visualOrder(
    [...sub].sort((a, b) => a.page - b.page || a.start - b.start),
    medianH
  );
  const merged: CopyRun[] = [];
  for (const run of ordered) {
    const last = merged.at(-1);
    const lastPart = last ? keyFor.get(last.text) : undefined;
    const part = keyFor.get(run.text);
    if (
      last &&
      lastPart &&
      part &&
      'rects' in lastPart &&
      'rects' in part &&
      last.page === run.page &&
      Math.min(last.y + last.h, run.y + run.h) - Math.max(last.y, run.y) >
        0.5 * Math.min(last.h, run.h) &&
      run.x - (last.x + last.w) < 1.5 * medianH
    ) {
      lastPart.text += run.x - (last.x + last.w) > 0.15 * medianH ? ` ${part.text}` : part.text;
      lastPart.rects.push(...part.rects);
      inline.splice(inline.indexOf(part), 1);
      keyFor.delete(run.text);
      const right = Math.max(last.x + last.w, run.x + run.w);
      last.w = right - last.x;
      continue;
    }
    merged.push({ ...run });
  }
  // `smartCopyText` orders by `start`; keep the order settled here.
  const template = smartCopyText(merged.map((run, i) => ({ ...run, start: i })));
  let plain = template;
  for (const [key, part] of keyFor) plain = plain.split(key).join(part.text);
  return { plain, template, inline, displays };
}

/** The formula a converter read a stretch as, when it read it as one: a
 * Markdown answer made of math, with nothing between its pieces but what
 * a formula has anyway (numbers, brackets, punctuation) - "$\\delta \\in
 * (0$, 1)" is the formula "\\delta \\in (0, 1)". Null when it holds words. */
export function oneFormula(markdown: string | null | undefined): string | null {
  const text = (markdown ?? '').trim();
  if (!text) return null;
  const display = /^\$\$([\s\S]+)\$\$$/.exec(text);
  if (display && !display[1]!.includes('$')) return display[1]!.replace(/\s+/g, ' ').trim();
  let out = '';
  let math = false;
  for (const segment of splitMath(text)) {
    if (segment.kind === 'math') {
      out += segment.src;
      math = true;
      continue;
    }
    const glue = segment.text.replace(/\\(.)/g, '$1');
    if (!/^[\s0-9()[\]{}|.,;:'!/=<>+-]*$/.test(glue)) return null;
    out += glue;
  }
  const formula = out.replace(/\s+/g, ' ').trim();
  return math && formula ? formula : null;
}

/** Whether a display's plain text reads as prose (a table, a caption the
 * equation finder took for a formula) rather than as a formula: not worth
 * converting, and quoted as it is. */
export function prosy(text: string): boolean {
  return (
    // words, not the names of functions and operators
    (text.match(/[A-Za-z]{5,}/g) ?? []).length >= 3 ||
    // a table of measurements
    (text.match(/\d+\.\d+/g) ?? []).length >= 3
  );
}

/** Assemble the quote's Markdown: its prose escaped, each formula as
 * `$...$` and each display as `$$ ... $$` where it was read (`tex`), else
 * its plain text, set in italics when it is a formula. Formulas the page
 * sets side by side (a symbol and its raised index, split into two runs)
 * are one. */
export function fillQuote(plan: QuotePlan, tex: ReadonlyMap<string, string | null>): string {
  let out = '';
  let math = '';
  let mathPart: InlinePart | null = null;
  const flush = () => {
    if (math) out += `$${math}$`;
    math = '';
    mathPart = null;
  };
  /** Where a part sits: the middle of its box, and its height. */
  const place = (part: InlinePart) => {
    const bottom = Math.min(...part.rects.map((r) => r[1]));
    const top = Math.max(...part.rects.map((r) => r[1] + r[3]));
    return { centre: (bottom + top) / 2, h: top - bottom };
  };
  /** A display's number, printed after it, which its \\tag says already. */
  let printed: string | null = null;
  const template = [...plan.template];
  for (let i = 0; i < template.length; i += 1) {
    const ch = template[i]!;
    if (printed !== null) {
      const label = `(${printed})`;
      const ahead = template.slice(i).join('');
      const at = ahead.search(/\S/);
      if (at >= 0 && ahead.startsWith(label, at)) {
        i += at + label.length - 1;
        printed = null;
        continue;
      }
      printed = null;
    }
    const inlinePart = plan.inline.find((p) => p.key === ch);
    const displayPart = plan.displays.find((p) => p.key === ch);
    const part = inlinePart ?? displayPart;
    if (!part) {
      flush();
      out += escapeMarkdown(ch);
      continue;
    }
    const read = tex.get(part.key)?.trim();
    const plain = escapeMarkdown(part.text.replace(/\s+/g, ' ').trim());
    if (inlinePart && read) {
      if (mathPart) {
        // set raised or lowered against the formula before it: its index
        const a = place(mathPart);
        const b = place(inlinePart);
        if (b.h < a.h && b.centre > a.centre + 0.25 * a.h) math += `^{${read}}`;
        else if (b.h < a.h && b.centre < a.centre - 0.25 * a.h) math += `_{${read}}`;
        else math += ` ${read}`;
      } else {
        math = read;
      }
      mathPart = inlinePart;
      continue;
    }
    flush();
    if (read && displayPart?.number && read.includes('\\tag')) printed = displayPart.number;
    if (read) out += ` $$ ${read} $$ `;
    else if (displayPart && prosy(part.text)) out += ` ${plain} `;
    else out += `*${plain}*`;
  }
  flush();
  return out.replace(/[ \t]+/g, ' ').trim();
}

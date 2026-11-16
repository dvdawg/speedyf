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
  /** where it is, `[x, y, w, h]` per run it spans, page points y up, at
   * the height of its line: what a converter reads it from (a minus sign's
   * own box is a sliver no glyph centre falls in) */
  rects: [number, number, number, number][];
  /** the box of its own ink: where it sits against a formula beside it */
  ink: [number, number, number, number];
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
  'log ln exp sin cos tan sec csc cot sinh cosh tanh arcsin arccos arctan lim liminf limsup max min sup inf argmax argmin det dim ker deg gcd arg Pr tr diag sgn sign var cov poly const'.split(
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
  // not a big operator (a sum, an integral): one of those opens a formula
  // rather than joining a number to one
  const big =
    (cp >= 0x220f && cp <= 0x2211) ||
    (cp >= 0x222b && cp <= 0x2233) ||
    (cp >= 0x22c0 && cp <= 0x22c3) ||
    (cp >= 0x2a00 && cp <= 0x2a0c);
  return (
    !big &&
    ('=<>+*/'.includes(String.fromCodePoint(cp)) ||
      cp === MINUS ||
      (cp >= 0x2190 && cp <= 0x22ff) ||
      (cp >= 0x2a00 && cp <= 0x2aff))
  );
}

/** Variation selectors: a font's hint about which form of the glyph
 * before it to draw ("\u2211\ufe01"), nothing a reader sees. */
export function isIgnorable(cp: number): boolean {
  return cp >= 0xfe00 && cp <= 0xfe0f;
}

/** Accents a formula puts over a letter ("p" and its bar), drawn as
 * characters of their own. */
function isAccent(cp: number): boolean {
  return (
    cp === 0xaf ||
    cp === 0xa8 ||
    cp === 0xb4 ||
    cp === 0x2d9 ||
    cp === 0x2dc ||
    (cp >= 0x2c6 && cp <= 0x2c7) ||
    (cp >= 0x300 && cp <= 0x36f)
  );
}

/** Text without its variation selectors. */
export function withoutIgnorable(text: string): string {
  let out = '';
  for (const ch of text) if (!isIgnorable(ch.codePointAt(0)!)) out += ch;
  return out;
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
      if (isIgnorable(cp)) {
        k += units;
        continue;
      }
      if (inRanges(k, ranges) || isMathSymbol(cp) || isAccent(cp)) {
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
    // "log(1/x)", "exp(-q)", "KL(q||p)": its words are function or
    // operator names, among symbols
    const named = (w: string) => FUNCTIONS.has(w) || (math && /^[A-Z]{1,3}$/.test(w));
    if (prose && (math || bracket) && words.every(named)) {
      prose = false;
      math = true;
    }
    const token = text.slice(start, k);
    let kind: TokenKind;
    // "log" on its own, between symbols: the formula's, at its ends not
    if (prose && words.length === 1 && FUNCTIONS.has(words[0]!) && token === words[0]) {
      kind = 'glue';
    } else if (prose) kind = 'prose';
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
  let turn: (run: CopyRun) => number = () => 0;
  const centreIn = (a: CopyRun, b: CopyRun) => a.y + a.h / 2 >= b.y && a.y + a.h / 2 <= b.y + b.h;
  // an accent (a bar, a hat) drawn as a mark of its own, just over or
  // under a letter of the line
  const accentOf = (a: CopyRun, b: CopyRun) =>
    a.h < 0.35 * b.h && a.y + a.h / 2 >= b.y - 0.6 * b.h && a.y + a.h / 2 <= b.y + 1.6 * b.h;
  // overlapping by half, or a script whose centre is within the line
  const sameLine = (a: Piece, b: Piece) =>
    a.run.page === b.run.page &&
    (Math.min(a.run.y + a.run.h, b.run.y + b.run.h) - Math.max(a.run.y, b.run.y) >
      0.5 * Math.min(a.run.h, b.run.h) ||
      centreIn(a.run, b.run) ||
      centreIn(b.run, a.run) ||
      accentOf(a.run, b.run) ||
      accentOf(b.run, a.run));
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
    // the line's band, from its ordinary runs (not a tall sum sign)
    const heights = group.map((p) => p.run.h).sort((x, y) => x - y);
    const typical = heights[heights.length >> 1]!;
    const body = group.filter((p) => p.run.h <= 1.5 * typical && p.run.h >= 0.5 * typical);
    const banded = body.length > 0 ? body : group;
    const bandBottom = Math.min(...banded.map((p) => p.run.y)) - 0.25 * typical;
    const bandTop = Math.max(...banded.map((p) => p.run.y + p.run.h)) + 0.25 * typical;
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
      const own: [number, number, number, number][] = [];
      for (const [i, [ka, kb]] of spans) {
        const run = group[i]!.run;
        const edge = edgeOf(run);
        const w = Math.max(0, edge(kb) - edge(ka));
        rects.push([edge(ka), bandBottom, w, bandTop - bandBottom]);
        own.push([edge(ka), run.y, w, run.h]);
      }
      const inkLeft = Math.min(...own.map((r) => r[0]));
      const inkBottom = Math.min(...own.map((r) => r[1]));
      const inkRight = Math.max(...own.map((r) => r[0] + r[2]));
      const inkTop = Math.max(...own.map((r) => r[1] + r[3]));
      const key = newKey();
      const part: InlinePart = {
        key,
        text: text.slice(a, b),
        page: group[0]!.run.page,
        rects,
        ink: [inkLeft, inkBottom, inkRight - inkLeft, inkTop - inkBottom],
      };
      inline.push(part);
      keyFor.set(key, part);
      const [firstPiece, [firstK]] = [...spans][0]!;
      sub.push({
        text: key,
        start: turn(group[firstPiece]!.run) + firstK / 10000,
        x: inkLeft,
        y: inkBottom,
        w: inkRight - inkLeft,
        h: inkTop - inkBottom,
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
          start: turn(p.run) + a / 10000,
          x: edge(a),
          y: p.run.y,
          w: Math.max(0, edge(k) - edge(a)),
          h: p.run.h,
          page: p.run.page,
        });
      }
    });
  };

  // A run drawn out of its line's turn - a tall sum sign of the next line
  // emitted among the words of this one - goes to the line it sits on.
  const inTurn: QuotePiece[] = [...pieces];
  for (let i = 1; i + 1 < inTurn.length; i += 1) {
    const [before, it, after] = [inTurn[i - 1]!, inTurn[i]!, inTurn[i + 1]!];
    if (it.display || sameLine(before, it) || sameLine(it, after) || !sameLine(before, after)) {
      continue;
    }
    const home = inTurn.findIndex((p, j) => j > i + 1 && sameLine(p, it));
    if (home < 0) continue;
    inTurn.splice(i, 1);
    inTurn.splice(home - 1, 0, it);
  }

  // Pieces keep the order settled here, not the order the PDF drew them.
  const turns = new Map(inTurn.map((p, i) => [p.run, i]));
  turn = (run) => turns.get(run) ?? 0;

  for (const { run, from, to, display } of inTurn) {
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
      const stand: CopyRun = { text: key, start: turn(run), x, y, w, h, page: run.page };
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
      const l = Math.min(lastPart.ink[0], part.ink[0]);
      const bo = Math.min(lastPart.ink[1], part.ink[1]);
      const r = Math.max(lastPart.ink[0] + lastPart.ink[2], part.ink[0] + part.ink[2]);
      const t = Math.max(lastPart.ink[1] + lastPart.ink[3], part.ink[1] + part.ink[3]);
      lastPart.ink = [l, bo, r - l, t - bo];
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
  for (const part of [...inline, ...displays]) part.text = withoutIgnorable(part.text);
  let plain = template;
  for (const [key, part] of keyFor) plain = plain.split(key).join(part.text);
  return { plain: withoutIgnorable(plain), template: withoutIgnorable(template), inline, displays };
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
    let glue = segment.text.replace(/\\(.)/g, '$1');
    // an accent the reading left beside its letter: "$p$" then a bar
    const accent = ACCENTS.get(glue.charAt(0));
    if (accent && /[A-Za-z]$/.test(out)) {
      out = `${out.slice(0, -1)}${accent}{${out.slice(-1)}}`;
      glue = glue.slice(1);
    }
    // a function's or a constant's name ("+const") is the formula's
    glue = glue.replace(/[A-Za-z]+/g, (w) => (FUNCTIONS.has(w) ? `\\mathrm{${w}}` : w));
    if (!/^[\s0-9()[\]{}|.,;:'!/=<>+-]*$/.test(glue.replace(/\\mathrm\{[a-zA-Z]+\}/g, ''))) {
      return null;
    }
    out += glue;
  }
  const formula = out.replace(/\s+/g, ' ').trim();
  return math && formula ? formula : null;
}

/** Accents a reading can leave as a character beside their letter. */
const ACCENTS = new Map([
  [String.fromCharCode(0xaf), '\\bar'],
  [String.fromCharCode(0x2c6), '\\hat'],
  [String.fromCharCode(0x2dc), '\\tilde'],
  [String.fromCharCode(0x2d9), '\\dot'],
  [String.fromCharCode(0xa8), '\\ddot'],
]);

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

/** Whether text ends in an operator or a relation, which a formula
 * goes on past. */
function endsWithOperator(text: string): boolean {
  const last = [...text.trim()].at(-1);
  return last !== undefined && isOperator(last.codePointAt(0)!);
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
  let joinNext = false;
  const flush = () => {
    if (math) out += `$${math}$`;
    math = '';
    mathPart = null;
  };
  /** Where a part sits: the middle of its box, and its height. */
  const place = (part: InlinePart) => ({ centre: part.ink[1] + part.ink[3] / 2, h: part.ink[3] });
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
      // A formula broken at a line end after an operator ("... =") goes on
      // after the break: the space is the formula's if a formula follows.
      if (/\s/.test(ch) && mathPart && endsWithOperator(mathPart.text)) {
        const next = template.slice(i).join('').search(/\S/);
        const after = next < 0 ? undefined : template[i + next];
        const following = plan.inline.find((p) => p.key === after);
        if (following && tex.get(following.key)?.trim()) {
          i += next - 1;
          joinNext = true;
          continue;
        }
      }
      flush();
      out += escapeMarkdown(ch);
      continue;
    }
    const read = tex.get(part.key)?.trim();
    const plain = escapeMarkdown(part.text.replace(/\s+/g, ' ').trim());
    if (inlinePart && read) {
      if (mathPart && joinNext) {
        math += ` ${read}`;
      } else if (mathPart) {
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
      joinNext = false;
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

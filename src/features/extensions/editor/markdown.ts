/** The Markdown dialect the note editor draws, read without a DOM so it is
 * tested on its own.
 *
 * Deliberately small: the blocks and spans a research note uses, read the way
 * CommonMark and Pandoc read them where they agree, plus Pandoc's citations.
 * Anything it does not recognise is shown as the text it is. */

export type BlockKind = 'line' | 'math' | 'code';

/** A run of the note's text that is edited and drawn as one: a line, or a
 * `$$ ... $$` / fenced block spanning several. Joining every block's `raw`
 * with newlines gives the note back exactly. */
export interface BlockSpan {
  kind: BlockKind;
  raw: string;
  /** Offset of the block's first character in the note. */
  start: number;
}

const FENCE = /^\s{0,3}(`{3,}|~{3,})/;

/** Cut a note into blocks. Linear in its length, and the only pass over the
 * whole text an edit costs. */
export function splitBlocks(text: string): BlockSpan[] {
  const lines = text.split('\n');
  const out: BlockSpan[] = [];
  let offset = 0;
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    const trimmed = line.trim();
    let end = i;
    let kind: BlockKind = 'line';
    if (trimmed.startsWith('$$')) {
      kind = 'math';
      // `$$ x $$` on one line closes itself; otherwise run to the line
      // ending in `$$`, or to the end of the note when none does.
      const closesHere = trimmed.length >= 4 && trimmed.endsWith('$$');
      if (!closesHere) {
        end = i + 1;
        while (end < lines.length && !lines[end]!.trim().endsWith('$$')) end++;
        if (end >= lines.length) end = lines.length - 1;
      }
    } else {
      const fence = FENCE.exec(line);
      if (fence) {
        kind = 'code';
        const mark = fence[1]!;
        end = i + 1;
        while (end < lines.length && !lines[end]!.trim().startsWith(mark)) end++;
        if (end >= lines.length) end = lines.length - 1;
      }
    }
    const raw = lines.slice(i, end + 1).join('\n');
    out.push({ kind, raw, start: offset });
    offset += raw.length + 1;
    i = end + 1;
  }
  return out;
}

/** The TeX of a math block: what lies between its `$$`s. */
export function mathSource(raw: string): string {
  let s = raw.trim();
  if (s.startsWith('$$')) s = s.slice(2);
  if (s.endsWith('$$')) s = s.slice(0, -2);
  return s.trim();
}

/** The code of a fenced block, without its fences. */
export function codeSource(raw: string): string {
  const lines = raw.split('\n');
  const body = lines.slice(1);
  if (body.length > 0 && FENCE.test(body[body.length - 1]!)) body.pop();
  return body.join('\n');
}

export type LineShape =
  | { kind: 'blank' }
  | { kind: 'heading'; level: number; body: string }
  | { kind: 'bullet'; indent: number; body: string; marker: string }
  | { kind: 'ordered'; indent: number; body: string; marker: string }
  | { kind: 'quote'; depth: number; body: string }
  | { kind: 'rule' }
  | { kind: 'para'; body: string };

/** How a single line is drawn. */
export function lineShape(raw: string): LineShape {
  if (raw.trim() === '') return { kind: 'blank' };
  let m = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(raw);
  if (m) return { kind: 'heading', level: m[1]!.length, body: m[2]! };
  if (/^\s{0,3}([-*_])(\s*\1){2,}\s*$/.test(raw)) return { kind: 'rule' };
  m = /^(\s*)(>\s?)+/.exec(raw);
  if (m) {
    const depth = (m[0].match(/>/g) ?? []).length;
    return { kind: 'quote', depth, body: raw.slice(m[0].length) };
  }
  m = /^(\s*)([-*+])\s+(.*)$/.exec(raw);
  if (m) return { kind: 'bullet', indent: m[1]!.length, marker: m[2]!, body: m[3]! };
  m = /^(\s*)(\d{1,9}[.)])\s+(.*)$/.exec(raw);
  if (m) return { kind: 'ordered', indent: m[1]!.length, marker: m[2]!, body: m[3]! };
  return { kind: 'para', body: raw };
}

/** A citation as written: `@key`, or `[@key, p. 12]{box=x,y,w,h}`. */
export interface Citation {
  key: string;
  /** The locator after the key ("p. 12"), when bracketed with one. */
  locator: string | null;
  /** Zero-based page, read from a "p." locator. */
  page: number | null;
  /** Where on the page, top-left page points: from `{box=...}`. */
  box: [number, number, number, number] | null;
}

export type Span =
  | { kind: 'text'; text: string }
  | { kind: 'em'; children: Span[] }
  | { kind: 'strong'; children: Span[] }
  | { kind: 'code'; text: string }
  | { kind: 'math'; tex: string }
  | { kind: 'link'; href: string; children: Span[] }
  | { kind: 'image'; src: string; alt: string }
  | { kind: 'cite'; cite: Citation; raw: string };

/** Characters a cite key is made of: Pandoc's, less what ends a sentence. */
const KEY = String.raw`[A-Za-z0-9_][A-Za-z0-9_:.#$%&+?<>~/-]*[A-Za-z0-9_]|[A-Za-z0-9_]`;
const BRACKET_CITE = new RegExp(
  String.raw`^\[@(${KEY})(?:\s*,\s*([^\]]*))?\](?:\{box=(-?[\d.]+),(-?[\d.]+),(-?[\d.]+),(-?[\d.]+)\})?`
);
const BARE_CITE = new RegExp(String.raw`^@(${KEY})`);

/** Read "p. 12", "pp. 12-14", "page 3" into a zero-based page. */
export function locatorPage(locator: string | null): number | null {
  if (!locator) return null;
  const m = /\b(?:pp?\.|pages?)\s*(\d{1,6})/i.exec(locator);
  if (!m) return null;
  const page = Number(m[1]) - 1;
  return page >= 0 ? page : null;
}

/** Closing `$` of inline math opened at `open`, by Pandoc's rules (see
 * `components/mathSplit.ts`), or -1. */
function closeDollar(text: string, open: number): number {
  const first = text[open + 1];
  if (first === undefined || first === '$' || /\s/.test(first)) return -1;
  for (let j = open + 1; j < text.length; j++) {
    const c = text[j];
    if (c === '\\') {
      j += 1;
      continue;
    }
    if (c !== '$') continue;
    if (/\s/.test(text[j - 1]!)) return -1;
    const after = text[j + 1];
    if (after !== undefined && /[0-9]/.test(after)) return -1;
    return j;
  }
  return -1;
}

/** Where an emphasis run opened with `mark` at `at` closes, or -1. */
function closeEmphasis(text: string, at: number, mark: string): number {
  const inner = at + mark.length;
  if (inner >= text.length || /\s/.test(text[inner]!)) return -1;
  for (let j = inner + 1; j <= text.length - mark.length; j++) {
    if (text[j] === '\\') {
      j++;
      continue;
    }
    if (text.startsWith(mark, j) && !/\s/.test(text[j - 1]!)) {
      // `**` is not two `*` closings.
      if (mark.length === 1 && text[j + 1] === mark) {
        j++;
        continue;
      }
      return j;
    }
  }
  return -1;
}

/** Inline spans of one line's text. */
export function inlineSpans(text: string): Span[] {
  const out: Span[] = [];
  let plain = '';
  const flush = () => {
    if (plain) out.push({ kind: 'text', text: plain });
    plain = '';
  };
  let i = 0;
  while (i < text.length) {
    const c = text[i]!;
    const rest = text.slice(i);
    if (c === '\\' && i + 1 < text.length && /[!-/:-@[-`{-~]/.test(text[i + 1]!)) {
      plain += text[i + 1];
      i += 2;
      continue;
    }
    if (c === '`') {
      const close = text.indexOf('`', i + 1);
      if (close > i + 1) {
        flush();
        out.push({ kind: 'code', text: text.slice(i + 1, close) });
        i = close + 1;
        continue;
      }
    }
    if (c === '$') {
      const close = closeDollar(text, i);
      if (close > 0) {
        flush();
        out.push({ kind: 'math', tex: text.slice(i + 1, close) });
        i = close + 1;
        continue;
      }
    }
    if (c === '[' && text[i + 1] === '@') {
      const m = BRACKET_CITE.exec(rest);
      if (m) {
        flush();
        const locator = m[2]?.trim() || null;
        const box =
          m[3] !== undefined
            ? ([Number(m[3]), Number(m[4]), Number(m[5]), Number(m[6])] as [
                number,
                number,
                number,
                number,
              ])
            : null;
        out.push({
          kind: 'cite',
          raw: m[0],
          cite: { key: m[1]!, locator, page: locatorPage(locator), box },
        });
        i += m[0].length;
        continue;
      }
    }
    if (c === '@' && (i === 0 || /[\s([{;,"'-]/.test(text[i - 1]!))) {
      const m = BARE_CITE.exec(rest);
      if (m) {
        flush();
        out.push({
          kind: 'cite',
          raw: m[0],
          cite: { key: m[1]!, locator: null, page: null, box: null },
        });
        i += m[0].length;
        continue;
      }
    }
    if (c === '!' && text[i + 1] === '[') {
      const m = /^!\[([^\]]*)\]\(([^)\s]+)\)/.exec(rest);
      if (m) {
        flush();
        out.push({ kind: 'image', alt: m[1]!, src: m[2]! });
        i += m[0].length;
        continue;
      }
    }
    if (c === '[') {
      const m = /^\[([^\]]+)\]\(([^)\s]+)\)/.exec(rest);
      if (m) {
        flush();
        out.push({ kind: 'link', href: m[2]!, children: inlineSpans(m[1]!) });
        i += m[0].length;
        continue;
      }
    }
    if (c === '*' || c === '_') {
      const strong = text[i + 1] === c ? c + c : null;
      if (strong) {
        const close = closeEmphasis(text, i, strong);
        if (close > 0) {
          flush();
          out.push({ kind: 'strong', children: inlineSpans(text.slice(i + 2, close)) });
          i = close + 2;
          continue;
        }
      }
      // `_` inside a word is not emphasis (snake_case).
      const intraword = c === '_' && i > 0 && /\w/.test(text[i - 1]!);
      if (!intraword) {
        const close = closeEmphasis(text, i, c);
        if (close > 0) {
          flush();
          out.push({ kind: 'em', children: inlineSpans(text.slice(i + 1, close)) });
          i = close + 1;
          continue;
        }
      }
    }
    plain += c;
    i++;
  }
  flush();
  return out;
}

/** Every citation in a note, with its offset: what backlinks count and the
 * first mention is found by. */
export function citationsIn(text: string): { cite: Citation; offset: number }[] {
  const out: { cite: Citation; offset: number }[] = [];
  for (const block of splitBlocks(text)) {
    if (block.kind !== 'line') continue;
    let searchFrom = 0;
    const walk = (spans: Span[]) => {
      for (const span of spans) {
        if (span.kind === 'cite') {
          const at = block.raw.indexOf(span.raw, searchFrom);
          if (at >= 0) searchFrom = at + span.raw.length;
          out.push({ cite: span.cite, offset: block.start + Math.max(0, at) });
        } else if (span.kind === 'em' || span.kind === 'strong' || span.kind === 'link') {
          walk(span.children);
        }
      }
    };
    walk(inlineSpans(block.raw));
  }
  return out;
}

/** Map a click at `offset` in a block's drawn text to an offset in its
 * source: walk both, skipping the source's markup. Close enough to put the
 * caret where the click was for prose, headings and emphasis. */
export function sourceOffset(raw: string, drawn: string, offset: number): number {
  let j = 0;
  for (let i = 0; i < raw.length; i++) {
    if (j >= offset) return i;
    if (raw[i] === drawn[j]) j++;
  }
  return raw.length;
}

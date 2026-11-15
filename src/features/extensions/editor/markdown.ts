/** The Markdown dialect the note editor draws, read without a DOM so it is
 * tested on its own.
 *
 * Deliberately small: the blocks and spans a research note uses, read the way
 * CommonMark and Pandoc read them where they agree, plus Pandoc's citations.
 * Anything it does not recognise is shown as the text it is. */

export type BlockKind = 'line' | 'math' | 'code' | 'quote';

/** A run of the note's text that is edited and drawn as one: a line, a
 * `$$ ... $$` / fenced block spanning several, or a run of consecutive `>`
 * lines, which reads as one block quote. Joining every block's `raw`
 * with newlines gives the note back exactly. */
export interface BlockSpan {
  kind: BlockKind;
  raw: string;
  /** Offset of the block's first character in the note. */
  start: number;
}

const FENCE = /^\s{0,3}(`{3,}|~{3,})/;
const QUOTE = /^\s*>/;

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
      } else if (QUOTE.test(line)) {
        // A quote of several paragraphs is one block, drawn with one border:
        // `>` lines run on, a bare `>` between paragraphs included.
        kind = 'quote';
        while (end + 1 < lines.length && QUOTE.test(lines[end + 1]!)) end++;
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
  | { kind: 'footnote'; id: string; body: string }
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
  m = /^\[\^([A-Za-z0-9_.:+-]{1,80})\]:\s*(.*)$/.exec(raw);
  if (m) return { kind: 'footnote', id: m[1]!, body: m[2]! };
  m = /^(\s*)([-*+])\s+(.*)$/.exec(raw);
  if (m) return { kind: 'bullet', indent: m[1]!.length, marker: m[2]!, body: m[3]! };
  m = /^(\s*)(\d{1,9}[.)])\s+(.*)$/.exec(raw);
  if (m) return { kind: 'ordered', indent: m[1]!.length, marker: m[2]!, body: m[3]! };
  return { kind: 'para', body: raw };
}

/** A citation as written: `@key`, `[@key, p. 12]{box=x,y,w,h}`, or a
 * statement `[@key, Prop. 2.2]{box=x,y,w,h page=3}`. */
export interface Citation {
  key: string;
  /** The locator after the key ("p. 12", "Prop. 2.2"), when bracketed
   * with one. */
  locator: string | null;
  /** Zero-based page, read from a "p." locator, else from `page=`. */
  page: number | null;
  /** Where on the page, top-left page points: from `{box=...}`. */
  box: [number, number, number, number] | null;
}

type Box = [number, number, number, number];

export type Span =
  | { kind: 'text'; text: string }
  | { kind: 'em'; children: Span[] }
  | { kind: 'strong'; children: Span[] }
  | { kind: 'code'; text: string }
  | { kind: 'math'; tex: string }
  | { kind: 'link'; href: string; children: Span[] }
  | { kind: 'image'; src: string; alt: string }
  | { kind: 'cite'; cite: Citation; raw: string }
  /** A cross-reference into a paper, `[Theorem 2.1](@key#p3){box=...}`:
   * the words as printed, the paper, the zero-based page and the target's
   * box (top-left page points). */
  | { kind: 'xref'; text: string; key: string; page: number | null; box: Box | null; raw: string }
  /** A reference a quoted paper printed that no item is: `[^id]`, its
   * entry in the note's `[^id]: ...` line, optionally followed by
   * `{label=Sim15 at=1}`: the label the paper printed (else the one its
   * entry opens with) and, in a citation group, its place among the
   * group's citations. */
  | { kind: 'noteref'; id: string; label: string | null; at: number | null; raw: string }
  /** Citations printed as one bracket, "[Han16, Lar23, AAHLZ24]": a
   * Pandoc group `[@a; @b]` and the reference markers right after it, or
   * citations and markers written side by side, in the printed order. */
  | { kind: 'group'; items: GroupItem[]; raw: string };

/** One citation of a group: an item's, or a reference no item is. */
export type GroupItem =
  | { kind: 'cite'; cite: Citation; raw: string }
  | { kind: 'noteref'; id: string; label: string | null; at: number | null; raw: string };

/** Characters a cite key is made of: Pandoc's, less what ends a sentence. */
const KEY = String.raw`[A-Za-z0-9_][A-Za-z0-9_:.#$%&+?<>~/-]*[A-Za-z0-9_]|[A-Za-z0-9_]`;
const NUM = String.raw`(-?[\d.]+)`;
const BOX = String.raw`\{box=${NUM},${NUM},${NUM},${NUM}(?:\s+page=(\d{1,6}))?\}`;
const BRACKET_CITE = new RegExp(String.raw`^\[@(${KEY})(?:\s*,\s*([^\]]*))?\](?:${BOX})?`);
const BARE_CITE = new RegExp(String.raw`^@(${KEY})`);
const XREF = new RegExp(String.raw`^\[([^\]\n]+)\]\(@([^)\s]+?)(?:#p(\d{1,6}))?\)(?:${BOX})?`);
const NOTEREF =
  /^\[\^([A-Za-z0-9_.:+-]{1,80})\](?!:)(?:\{((?:\s*(?:label=(?:"[^"\n]{0,60}"|[^\s}"]{1,60})|at=\d{1,3}))+)\s*\})?/;
/** `[@a; @b, p. 3; @c]`: a Pandoc citation of several works. */
const GROUP_CITE = new RegExp(String.raw`^\[(@(?:${KEY})[^\];\n]*(?:;\s*@(?:${KEY})[^\];\n]*)+)\]`);

function boxOf(m: RegExpExecArray, at: number): Box | null {
  return m[at] !== undefined
    ? [Number(m[at]), Number(m[at + 1]), Number(m[at + 2]), Number(m[at + 3])]
    : null;
}

/** The words a statement block opens with, as amsthm prints them. */
export const STATEMENT_WORDS = [
  'Theorem',
  'Lemma',
  'Proposition',
  'Corollary',
  'Claim',
  'Conjecture',
  'Definition',
  'Remark',
  'Example',
  'Assumption',
  'Hypothesis',
  'Fact',
  'Observation',
  'Problem',
  'Question',
  'Notation',
  'Exercise',
];

/** Kinds set in italic, as amsthm's `plain` style sets them; the rest
 * (definition, remark, example...) are upright. */
const ITALIC_KINDS = new Set([
  'theorem',
  'lemma',
  'proposition',
  'corollary',
  'claim',
  'conjecture',
  'hypothesis',
  'fact',
  'observation',
]);

/** A statement's header: `**Proposition 2.2** (Title).` */
const STATEMENT_HEAD = new RegExp(
  String.raw`^\*\*(${STATEMENT_WORDS.join('|')})(?:\s+([^*\s]{1,20}))?\*\*(?:\s*\(((?:[^()]|\([^()]*\))*)\))?\.?\s*`
);

/** A statement block as written: the header of its first line, and that
 * line's rest. */
export interface StatementHead {
  /** "proposition" */
  kind: string;
  /** "Proposition" */
  word: string;
  number: string | null;
  title: string | null;
  /** Set in italic (theorem-like) or upright (definition-like). */
  italic: boolean;
  /** What follows the header on its line. */
  rest: string;
}

/** The statement a quote's first line opens: a block quote whose first
 * line starts `**Proposition 2.2** (Title).` is a statement (Notepad's
 * statement blocks; any Markdown reader shows it as the bold-labelled
 * quote it is). Null for any other line. */
export function statementHead(body: string): StatementHead | null {
  const m = STATEMENT_HEAD.exec(body);
  if (!m) return null;
  const kind = m[1]!.toLowerCase();
  return {
    kind,
    word: m[1]!,
    number: m[2] ?? null,
    title: m[3]?.trim() || null,
    italic: ITALIC_KINDS.has(kind),
    rest: body.slice(m[0].length),
  };
}

/** A quote line that is nothing but a page link (a statement's last). */
export function onlyPageLink(body: string): boolean {
  const m = BRACKET_CITE.exec(body.trim());
  return !!m && m[0].length === body.trim().length;
}

/** The note's reference entries, `[^id]: text`, by id (the first of each). */
export function footnotesIn(text: string): Map<string, string> {
  const out = new Map<string, string>();
  const re = /^\[\^([A-Za-z0-9_.:+-]{1,80})\]:\s*(.*)$/gm;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) if (!out.has(m[1]!)) out.set(m[1]!, m[2]!);
  return out;
}

/** How a `[^id]` reads: the printed label its entry opens with
 * ("[Sim15] B. Simon, ..."), else the id in brackets. */
export function noterefLabel(id: string, entry: string | undefined, label?: string | null): string {
  if (label) return `[${label}]`;
  const m = entry ? /^\[([^\]\n]{1,40})\]/.exec(entry.trim()) : null;
  return m ? `[${m[1]}]` : `[${id}]`;
}

/** An entry's text without the printed label it opens with. */
export function noterefText(entry: string): string {
  return entry.trim().replace(/^\[[^\]\n]{1,40}\]\s*/, '');
}

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

/** A bracketed citation at the start of `rest`: `[@key, locator]{box=...}`,
 * a Pandoc group `[@a; @b]`, or a reference marker `[^id]{label=...}`. */
function citationAt(rest: string): { items: GroupItem[]; group: boolean; raw: string } | null {
  if (rest[1] === '@') {
    const m = BRACKET_CITE.exec(rest);
    if (m) {
      const locator = m[2]?.trim() || null;
      const attrPage = m[7] !== undefined ? Number(m[7]) - 1 : null;
      const cite: Citation = {
        key: m[1]!,
        locator,
        page: locatorPage(locator) ?? (attrPage !== null && attrPage >= 0 ? attrPage : null),
        box: boxOf(m, 3),
      };
      return { items: [{ kind: 'cite', cite, raw: m[0] }], group: false, raw: m[0] };
    }
    const g = GROUP_CITE.exec(rest);
    if (!g) return null;
    const items: GroupItem[] = [];
    for (const part of g[1]!.split(';')) {
      const one = new RegExp(String.raw`^\s*@(${KEY})(?:\s*,\s*(.*))?$`).exec(part);
      if (!one) return null;
      const locator = one[2]?.trim() || null;
      items.push({
        kind: 'cite',
        raw: part.trim(),
        cite: { key: one[1]!, locator, page: locatorPage(locator), box: null },
      });
    }
    return { items, group: true, raw: g[0] };
  }
  const m = NOTEREF.exec(rest);
  if (!m) return null;
  const attrs = m[2] ?? '';
  const label = /label=(?:"([^"\n]*)"|([^\s}"]+))/.exec(attrs);
  const at = /at=(\d{1,3})/.exec(attrs);
  return {
    items: [
      {
        kind: 'noteref',
        id: m[1]!,
        label: label ? (label[1] ?? label[2] ?? '').trim() || null : null,
        at: at ? Number(at[1]) : null,
        raw: m[0],
      },
    ],
    group: false,
    raw: m[0],
  };
}

/** The citations written side by side from `i`, as one span: a lone
 * citation or marker as itself, several as a group in the printed order
 * (a marker's `at` its place among the cited items before it). */
function citationRun(text: string, i: number): { span: Span; length: number } | null {
  const first = citationAt(text.slice(i));
  if (!first) return null;
  const parts = [first];
  let end = i + first.raw.length;
  for (;;) {
    const next = text[end] === '[' ? citationAt(text.slice(end)) : null;
    if (!next) break;
    parts.push(next);
    end += next.raw.length;
  }
  if (parts.length === 1 && !first.group) {
    const only = first.items[0]!;
    return { span: only, length: first.raw.length };
  }
  const items: GroupItem[] = [];
  const placed: { item: GroupItem; at: number }[] = [];
  for (const part of parts) {
    for (const item of part.items) {
      if (item.kind === 'noteref' && item.at !== null) placed.push({ item, at: item.at });
      else items.push(item);
    }
  }
  for (const { item, at } of placed.sort((a, b) => a.at - b.at)) {
    items.splice(Math.min(at, items.length), 0, item);
  }
  return { span: { kind: 'group', items, raw: text.slice(i, end) }, length: end - i };
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
    if (c === '[' && (text[i + 1] === '@' || text[i + 1] === '^')) {
      const run = citationRun(text, i);
      if (run) {
        flush();
        out.push(run.span);
        i += run.length;
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
      const x = XREF.exec(rest);
      if (x) {
        flush();
        const page = x[3] !== undefined ? Number(x[3]) - 1 : null;
        out.push({
          kind: 'xref',
          text: x[1]!,
          key: x[2]!,
          page: page !== null && page >= 0 ? page : null,
          box: boxOf(x, 4),
          raw: x[0],
        });
        i += x[0].length;
        continue;
      }
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
    if (block.kind === 'math' || block.kind === 'code') continue;
    let searchFrom = 0;
    const walk = (spans: Span[]) => {
      for (const span of spans) {
        if (span.kind === 'cite') {
          const at = block.raw.indexOf(span.raw, searchFrom);
          if (at >= 0) searchFrom = at + span.raw.length;
          out.push({ cite: span.cite, offset: block.start + Math.max(0, at) });
        } else if (span.kind === 'group') {
          const at = block.raw.indexOf(span.raw, searchFrom);
          if (at >= 0) searchFrom = at + span.raw.length;
          for (const item of span.items) {
            if (item.kind !== 'cite') continue;
            const inner = at >= 0 ? span.raw.indexOf(`@${item.cite.key}`) : -1;
            out.push({
              cite: item.cite,
              offset: block.start + Math.max(0, at) + Math.max(0, inner),
            });
          }
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

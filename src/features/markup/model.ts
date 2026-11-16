/** The comment list as a review reads it: read from storage, numbered,
 * grouped by section, repeats folded together, counted, and written out as
 * a plain-text letter. Pure. */
import { numberComments } from './geometry';
import { plausibleQuoteTex } from './quoteTexCheck';
import { labelText, type CommentLabel } from './labels';
import { quickmarkById } from './quickmarks';
import {
  COMMENT_KINDS,
  KIND_LABEL,
  MARKUP_VERSION,
  emptyMarkup,
  type Anchor,
  type CommentKind,
  type MarkupComment,
  type MarkupDoc,
  type Quad,
  type Rect,
} from './types';

/* ---- storage ---- */

let seq = 0;

/** A fresh comment id: "c_" and something no other comment has. */
export function newCommentId(now = Date.now()): string {
  seq = (seq + 1) % 1_000_000;
  return `c_${now.toString(36)}${seq.toString(36)}${Math.floor(Math.random() * 1296).toString(36)}`;
}

const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown): string => (typeof v === 'string' ? v : '');

function readQuad(v: unknown): Quad | null {
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  const keys = ['x1', 'y1', 'x2', 'y2', 'x3', 'y3', 'x4', 'y4'] as const;
  const out: Partial<Quad> = {};
  for (const k of keys) {
    const n = num(o[k]);
    if (n === null) return null;
    out[k] = n;
  }
  return out as Quad;
}

function readRect(v: unknown): Rect | null {
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  const x = num(o.x);
  const y = num(o.y);
  const w = num(o.w);
  const h = num(o.h);
  return x === null || y === null || w === null || h === null ? null : { x, y, w, h };
}

/** What a saved reading is held to: the quote, or nothing for a comment on
 * an equation on its own (quoted "Equation (3)"). */
function quoteBasis(o: Record<string, unknown>): string {
  const quote = str(o.quote);
  return /^Equation( \([^)]*\))?$/.test(quote) ? '' : quote;
}

function readAnchor(v: unknown): Anchor | null {
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  const page = num(o.page);
  if (page === null || page < 0) return null;
  const quads = Array.isArray(o.quads)
    ? o.quads.map(readQuad).filter((q): q is Quad => q !== null)
    : null;
  const rect = readRect(o.rect);
  if ((!quads || quads.length === 0) && !rect) return null;
  return {
    page: Math.floor(page),
    quads: quads && quads.length > 0 ? quads : null,
    rect,
    quote: str(o.quote),
    // A reading saved before readings were checked is checked now: one that
    // fails is dropped, and the page's own text stands.
    ...(typeof o.quoteTex === 'string' && plausibleQuoteTex(o.quoteTex, quoteBasis(o))
      ? { quoteTex: o.quoteTex }
      : {}),
  };
}

function readComment(v: unknown): MarkupComment | null {
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  const anchor = readAnchor(o.anchor);
  if (!anchor) return null;
  // Reviews from before the three kinds said proof, exposition or
  // literature: all of those are plain comments now.
  const kind = COMMENT_KINDS.includes(o.kind as CommentKind) ? (o.kind as CommentKind) : null;
  return {
    id: str(o.id) || newCommentId(),
    kind: kind ?? 'comment',
    anchor,
    body: str(o.body),
    replacement: typeof o.replacement === 'string' ? o.replacement : null,
    quickmark: typeof o.quickmark === 'string' ? o.quickmark : null,
    created: num(o.created) ?? 0,
  };
}

/** The stored JSON read back, whatever state it is in: anything unreadable
 * is dropped rather than failing the whole review. A file from a newer
 * version is still read as far as it goes. */
export function parseMarkup(json: string | null | undefined): MarkupDoc {
  if (!json) return emptyMarkup();
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return emptyMarkup();
  }
  if (!raw || typeof raw !== 'object') return emptyMarkup();
  const o = raw as Record<string, unknown>;
  const comments = Array.isArray(o.comments)
    ? o.comments.map(readComment).filter((c): c is MarkupComment => c !== null)
    : [];
  return { version: MARKUP_VERSION, summary: str(o.summary), comments };
}

export function serializeMarkup(doc: MarkupDoc): string {
  return JSON.stringify({ version: MARKUP_VERSION, summary: doc.summary, comments: doc.comments });
}

/* ---- entries and groups ---- */

export interface Entry {
  comment: MarkupComment;
  number: number;
  label: CommentLabel;
}

/** The comments numbered in reading order, each with its label. */
export function buildEntries(
  comments: readonly MarkupComment[],
  labelOf: (anchor: Anchor) => CommentLabel
): Entry[] {
  return numberComments(comments).map(({ comment, number }) => ({
    comment,
    number,
    label: labelOf(comment.anchor),
  }));
}

export interface Group {
  key: string;
  /** The heading as printed, or "Front matter" before the first one. */
  title: string;
  entries: Entry[];
}

export const FRONT_MATTER = 'Front matter';

/** Entries grouped by their section, in paper order. Entries are already in
 * reading order, so a section's are consecutive. */
export function groupBySection(entries: readonly Entry[]): Group[] {
  const groups: Group[] = [];
  for (const entry of entries) {
    const s = entry.label.section;
    const key = s ? `${s.page}:${s.y}:${s.title}` : 'front';
    const last = groups.at(-1);
    if (last && last.key === key) last.entries.push(entry);
    else groups.push({ key, title: s ? s.title : FRONT_MATTER, entries: [entry] });
  }
  return groups;
}

/** How many comments of each kind. */
export function kindCounts(comments: readonly MarkupComment[]): Record<CommentKind, number> {
  const out: Record<CommentKind, number> = {
    comment: 0,
    question: 0,
    typo: 0,
  };
  for (const c of comments) out[c.kind] += 1;
  return out;
}

/** A typo fix (or a bare typo note): listed in the table, not the sections. */
export function isTypoEntry(comment: MarkupComment): boolean {
  return comment.kind === 'typo';
}

/* ---- repeats ---- */

const norm = (text: string) => text.replace(/\s+/g, ' ').trim().toLowerCase();

export interface Merged {
  entry: Entry;
  /** The same remark on the same words elsewhere, in reading order. */
  also: Entry[];
}

/** Fold repeats into their first occurrence: the same QuickMark, saying the
 * same thing (nothing typed after it that differs) about the same words.
 * Anything else stays its own entry. */
export function mergeRepeats(entries: readonly Entry[]): Merged[] {
  const out: Merged[] = [];
  const byKey = new Map<string, Merged>();
  for (const entry of entries) {
    const c = entry.comment;
    const quote = norm(c.anchor.quote);
    const key = c.quickmark && quote ? `${c.quickmark}\u0000${quote}\u0000${norm(c.body)}` : null;
    const first = key ? byKey.get(key) : undefined;
    if (first) {
      first.also.push(entry);
      continue;
    }
    const merged: Merged = { entry, also: [] };
    out.push(merged);
    if (key) byKey.set(key, merged);
  }
  return out;
}

/** "also p. 7, p. 9" for a folded entry; empty when there is nothing. */
export function alsoText(merged: Merged): string {
  if (merged.also.length === 0) return '';
  return `also ${merged.also.map((e) => `p. ${e.label.page}`).join(', ')}`;
}

/* ---- text ---- */

/** A quote for a card or the letter: one line, cut at a word near `max`. */
export function shortQuote(text: string, max = 140): string {
  const t = text.replace(/\s+/g, ' ').trim();
  if (t.length <= max) return t;
  const cut = t.slice(0, max);
  const space = cut.lastIndexOf(' ');
  return `${(space > max * 0.6 ? cut.slice(0, space) : cut).replace(/[\s,;:.]+$/, '')}...`;
}

/** A quote cut to about `max` characters, never inside a `$...$` or
 * `$$...$$`: half a formula is not a formula. */
export function shortMathQuote(text: string, max = 140): string {
  const t = text.replace(/\s+/g, ' ').trim();
  if (t.length <= max) return t;
  // Where a cut may go: a space outside math.
  let math = false;
  let best = -1;
  for (let i = 0; i < t.length && i <= max; i += 1) {
    if (t[i] === '\\') {
      i += 1;
      continue;
    }
    if (t[i] === '$') {
      if (t[i + 1] === '$') i += 1;
      math = !math;
      continue;
    }
    if (!math && t[i] === ' ') best = i;
  }
  if (best <= 0) {
    // The first formula runs past the limit: it is kept whole.
    const end = /^\$\$.*?\$\$|^\$.*?\$/.exec(t)?.[0];
    return end ? `${end}...` : shortQuote(t, max);
  }
  return `${t.slice(0, best).replace(/[\s,;:.]+$/, '')}...`;
}

/** What a comment quotes, for the letter: its words with their math as
 * LaTeX when that was read, else as the page gave them. */
export function anchorQuote(anchor: Anchor, max = 140): string {
  return anchor.quoteTex && plausibleQuoteTex(anchor.quoteTex, quoteBasis({ quote: anchor.quote }))
    ? shortMathQuote(anchor.quoteTex, max)
    : shortQuote(anchor.quote, max);
}

/** Every `[@key]` citation in a body, in order of first appearance. */
export function citedKeys(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/\[([^\]\n]*@[^\]\n]*)\]/g)) {
    for (const part of m[1]!.split(';')) {
      const key = /@([A-Za-z0-9_:.+-]+)/.exec(part)?.[1]?.replace(/[.:]+$/, '');
      if (key && !out.includes(key)) out.push(key);
    }
  }
  return out;
}

/** `[@key]` and `[@a; @b, p. 3]` written as their labels: "[Jar97]",
 * "[Jar97; Smi20, p. 3]". `label` answers null for a key no item has,
 * which then stays as typed. */
export function citationsAsText(text: string, label: (key: string) => string | null): string {
  return text.replace(/\[([^\]\n]*@[^\]\n]*)\]/g, (whole, inner: string) => {
    const parts = inner.split(';').map((part) => {
      const m = /^\s*@([A-Za-z0-9_:.+-]+?)([.:]*)(\s*,\s*(.*))?\s*$/.exec(part);
      if (!m) return null;
      const name = label(m[1]!);
      if (!name) return null;
      return m[4] ? `${name}, ${m[4].trim()}` : name;
    });
    return parts.every((p) => p !== null) ? `[${parts.join('; ')}]` : whole;
  });
}

/** The "Overall" box's starting point: the reviewer writes the paragraph,
 * the counts are filled in. */
export function summarySkeleton(comments: readonly MarkupComment[]): string {
  const counts = kindCounts(comments);
  const parts: string[] = [];
  const say = (n: number, one: string, many: string) => {
    if (n > 0) parts.push(`${n} ${n === 1 ? one : many}`);
  };
  say(counts.comment, 'comment', 'comments');
  say(counts.question, 'question', 'questions');
  say(counts.typo, 'typo', 'typos');
  const total = comments.length;
  const tally =
    total === 0 ? '' : `${total} ${total === 1 ? 'note' : 'notes'}: ${joinList(parts)}.`;
  return ['Summary: ', '', 'Main concerns:', '- ', '', tally].join('\n').trimEnd() + '\n';
}

function joinList(parts: string[]): string {
  if (parts.length <= 1) return parts.join('');
  return `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}`;
}

/** What a typo entry says should replace the words. */
export function typoSuggestion(comment: MarkupComment): string {
  if (comment.replacement !== null) return comment.replacement;
  const stock = quickmarkById(comment.quickmark)?.text;
  const body = comment.body.trim();
  return stock && body.replace(/\.$/, '') === stock ? '' : body;
}

export interface LetterInput {
  title: string;
  date: string;
  summary: string;
  entries: readonly Entry[];
  /** Cite key -> its label without brackets ("Jar97"). */
  label: (key: string) => string | null;
  /** Cite key -> its reference line, for the References at the end. */
  reference: (key: string) => string | null;
}

const indent = (text: string, pad: string) =>
  text
    .split('\n')
    .map((line) => (line.trim() ? pad + line : ''))
    .join('\n');

/** The review as a plain-text letter: grouped by section, typos in a list
 * of their own, the library items cited at the end. ASCII throughout. */
export function plainLetter(input: LetterInput): string {
  const out: string[] = [`Comments on ${input.title}`, input.date, ''];
  const summary = input.summary.trim();
  if (summary) out.push('Overall', '', citationsAsText(summary, input.label), '');

  const merged = mergeRepeats(input.entries.filter((e) => !isTypoEntry(e.comment)));
  const mergedEntries = merged.map((m) => m.entry);
  for (const group of groupBySection(mergedEntries)) {
    out.push(group.title, '');
    for (const entry of group.entries) {
      const m = merged.find((x) => x.entry === entry)!;
      const c = entry.comment;
      const head = [
        `${entry.number}.`,
        labelText(entry.label, false),
        `[${KIND_LABEL[c.kind]}]`,
        alsoText(m) ? `(${alsoText(m)})` : '',
      ]
        .filter(Boolean)
        .join(' ');
      out.push(`  ${head}`);
      if (c.anchor.quote.trim()) out.push(`     "${anchorQuote(c.anchor)}"`);
      else if (c.anchor.rect) out.push('     (marked region)');
      const body = citationsAsText(c.body.trim(), input.label);
      if (body) out.push(indent(body, '     '));
      out.push('');
    }
  }

  const typos = input.entries.filter((e) => isTypoEntry(e.comment));
  if (typos.length > 0) {
    out.push('Typos and wording', '');
    for (const e of typos) {
      const fix = typoSuggestion(e.comment);
      const original = shortQuote(e.comment.anchor.quote, 80);
      out.push(
        `  ${e.number}. p. ${e.label.page}: "${original}"${fix ? ` -> "${fix}"` : ''}` +
          (e.comment.replacement !== null && e.comment.body.trim()
            ? ` (${citationsAsText(e.comment.body.trim(), input.label)})`
            : '')
      );
    }
    out.push('');
  }

  const keys = [
    ...new Set([summary, ...input.entries.map((e) => e.comment.body)].flatMap(citedKeys)),
  ];
  const refs = keys
    .map((key) => {
      const label = input.label(key);
      const line = input.reference(key);
      return label && line ? `  [${label}] ${line}` : null;
    })
    .filter((line): line is string => line !== null);
  if (refs.length > 0) out.push('References', '', ...refs, '');

  return (
    out
      .join('\n')
      .replace(/\n{3,}/g, '\n\n')
      .trimEnd() + '\n'
  );
}

/** "November 15, 2026". */
export function longDate(when: Date): string {
  const months = [
    'January',
    'February',
    'March',
    'April',
    'May',
    'June',
    'July',
    'August',
    'September',
    'October',
    'November',
    'December',
  ];
  return `${months[when.getMonth()]} ${when.getDate()}, ${when.getFullYear()}`;
}

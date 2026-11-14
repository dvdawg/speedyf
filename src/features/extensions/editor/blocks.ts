/** The editor's model of a note, without a DOM: blocks kept across edits,
 * and the small decisions keys make (what Enter continues, what the `@`
 * picker offers). */
import { matchScore } from '../../palette/fuzzyScore';
import type { CitationEntry } from '../editorHost';
import { lineShape, splitBlocks, type BlockKind } from './markdown';

export interface Block {
  id: number;
  kind: BlockKind;
  raw: string;
}

let nextBlockId = 1;

/** Cut `text` into blocks, reusing `previous` blocks whose kind and text are
 * unchanged, in order, so a list keyed by block draws only what changed. */
export function reuseBlocks(previous: readonly Block[], text: string): Block[] {
  const pool = new Map<string, Block[]>();
  for (const block of previous) {
    const id = `${block.kind}\u0000${block.raw}`;
    const list = pool.get(id);
    if (list) list.push(block);
    else pool.set(id, [block]);
  }
  return splitBlocks(text).map((span) => {
    const id = `${span.kind}\u0000${span.raw}`;
    const reused = pool.get(id)?.shift();
    return reused ?? { id: nextBlockId++, kind: span.kind, raw: span.raw };
  });
}

/** Where each block starts in the note. */
export function startsOf(blocks: readonly Block[]): number[] {
  const out = new Array<number>(blocks.length);
  let at = 0;
  for (let i = 0; i < blocks.length; i++) {
    out[i] = at;
    at += blocks[i]!.raw.length + 1;
  }
  return out;
}

/** The text that continues a list item when Enter is pressed at the end of
 * `raw`: the next marker, or null when it is not a list item. An empty item
 * ends the list instead (`''`). */
export function continueList(raw: string): string | null {
  const shape = lineShape(raw);
  if (shape.kind !== 'bullet' && shape.kind !== 'ordered' && shape.kind !== 'quote') return null;
  if (shape.body.trim() === '') return '';
  const indent = ' '.repeat(shape.kind === 'quote' ? 0 : shape.indent);
  if (shape.kind === 'bullet') return `${indent}${shape.marker} `;
  if (shape.kind === 'quote') return '> ';
  const n = Number.parseInt(shape.marker, 10);
  return `${indent}${n + 1}${shape.marker.slice(-1)} `;
}

/** An `@` being typed just before `caret` in `text`: where it starts and
 * what follows it so far. */
export function citeQuery(text: string, caret: number): { at: number; query: string } | null {
  const before = text.slice(Math.max(0, caret - 80), caret);
  const m = /(^|[\s([{;,"'])@([A-Za-z0-9_:.-]*)$/.exec(before);
  if (!m) return null;
  return { at: caret - m[2]!.length - 1, query: m[2]! };
}

/** The picker's rows for `query`: the note's project's items, best first. */
export function pickerMatches(entries: readonly CitationEntry[], query: string): CitationEntry[] {
  const picks = entries.filter((e) => e.pick);
  if (!query.trim()) return picks.slice(0, 8);
  const scored: { entry: CitationEntry; score: number }[] = [];
  for (const entry of picks) {
    const score = matchScore(entry.title || entry.label, [entry.key, entry.detail], query);
    if (score !== null) scored.push({ entry, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, 8).map((s) => s.entry);
}

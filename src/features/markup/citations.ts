/** Citing library items in a comment: `@` opens a picker of the library,
 * a pick writes `[@citekey]`, and the citation reads "[Jar97]" - the same
 * alpha.bst-style label the Notepad and the graph show. The letter lists
 * the cited items under References. Pure over the rows the store loads. */
import { matchScore } from '../palette/fuzzyScore';
import type { PaperRow } from '../../types/engine';

export interface LibraryEntry {
  id: string;
  key: string;
  /** "Jar97", without brackets; unique among the library. */
  label: string;
  title: string;
  /** "Attias, Smith - 2022": the picker's second line. */
  detail: string;
  /** The reference line the letter prints. */
  reference: string;
  aliases: string[];
}

export interface CiteIndex {
  entries: LibraryEntry[];
  byKey: Map<string, LibraryEntry>;
}

export const EMPTY_INDEX: CiteIndex = { entries: [], byKey: new Map() };

function baseLabel(row: PaperRow): string {
  const alpha = (row.alphaLabel || '').trim();
  if (alpha) return alpha;
  const who = row.firstAuthor || (row.authors || '').split(/[,&]| et al\./)[0]!.trim();
  if (who) return row.year ? `${who}${String(row.year).slice(-2)}` : who;
  return row.citeKey;
}

/** 0 -> "a", 25 -> "z", 26 -> "aa". */
function suffix(n: number): string {
  let out = '';
  let k = n;
  for (;;) {
    out = String.fromCharCode(97 + (k % 26)) + out;
    if (k < 26) return out;
    k = Math.floor(k / 26) - 1;
  }
}

/** Labels unique among `rows` the way alpha.bst makes them: a repeated
 * base gets a, b, c in order of year (none last), then title, then id. */
export function alphaLabels(rows: readonly PaperRow[]): Map<string, string> {
  const groups = new Map<string, PaperRow[]>();
  for (const row of rows) {
    const base = baseLabel(row);
    const list = groups.get(base);
    if (list) list.push(row);
    else groups.set(base, [row]);
  }
  const out = new Map<string, string>();
  const cmp = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
  for (const [base, group] of groups) {
    if (group.length === 1) {
      out.set(group[0]!.id, base);
      continue;
    }
    [...group]
      .sort((a, b) => {
        if (a.year !== b.year) {
          if (a.year === null) return 1;
          if (b.year === null) return -1;
          return a.year - b.year;
        }
        return cmp(a.title || '', b.title || '') || cmp(a.id, b.id);
      })
      .forEach((row, i) => out.set(row.id, `${base}${suffix(i)}`));
  }
  return out;
}

/** "Attias, Smith. Title. Venue, 2022." */
export function referenceLine(row: PaperRow): string {
  const parts: string[] = [];
  const authors = (row.authors || '').trim();
  if (authors) parts.push(authors.replace(/\.$/, ''));
  if (row.title.trim()) parts.push(row.title.trim().replace(/\.$/, ''));
  const tail = [row.venue?.trim(), row.year ? String(row.year) : ''].filter(Boolean).join(', ');
  if (tail) parts.push(tail);
  return parts.length > 0 ? `${parts.join('. ')}.` : row.citeKey;
}

export function buildCiteIndex(rows: readonly PaperRow[]): CiteIndex {
  const labels = alphaLabels(rows);
  const entries = rows.map((row) => ({
    id: row.id,
    key: row.citeKey,
    label: labels.get(row.id) ?? row.citeKey,
    title: row.title,
    detail: [row.authors, row.year].filter(Boolean).join(' - '),
    reference: referenceLine(row),
    aliases: row.keyHistory ?? [],
  }));
  const byKey = new Map<string, LibraryEntry>();
  for (const entry of entries) byKey.set(entry.key, entry);
  for (const entry of entries) {
    for (const alias of entry.aliases) if (!byKey.has(alias)) byKey.set(alias, entry);
  }
  return { entries, byKey };
}

/** An `@` being typed just before `caret`: where it starts and the query. */
export function atQuery(text: string, caret: number): { at: number; query: string } | null {
  const before = text.slice(Math.max(0, caret - 80), caret);
  const m = /(^|[\s([{;,"'])@([A-Za-z0-9_:.-]*)$/.exec(before);
  if (!m) return null;
  return { at: caret - m[2]!.length - 1, query: m[2]! };
}

/** The picker's rows for a query, best first. */
export function pickCandidates(index: CiteIndex, query: string, limit = 8): LibraryEntry[] {
  if (!query.trim()) return index.entries.slice(0, limit);
  const scored: { entry: LibraryEntry; score: number }[] = [];
  for (const entry of index.entries) {
    const score = matchScore(
      entry.title || entry.label,
      [entry.key, entry.detail, entry.label],
      query
    );
    if (score !== null) scored.push({ entry, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.slice(0, limit).map((s) => s.entry);
}

/** `text` with the `@query` at `at` (to `caret`) replaced by `[@key]`. */
export function insertCitation(
  text: string,
  at: number,
  caret: number,
  key: string
): { text: string; caret: number } {
  const insert = `[@${key}]`;
  return { text: text.slice(0, at) + insert + text.slice(caret), caret: at + insert.length };
}

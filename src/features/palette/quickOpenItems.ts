/** Matching and ranking for the quick-open switcher.
 *
 * Pure, like `paletteItems.ts`: the component fetches the library rows once
 * per opening and this decides what survives and in what order. A few
 * thousand rows are matched on every keystroke, so each row's searchable text
 * is folded once, when the list arrives, not once per key.
 */
import type { PaperRow } from '../../types/engine';
import { yearText } from '../projects/paperFields';
import { subsequenceScore } from './fuzzyScore';

/** Rows shown at most. Anything further down is found by typing more. */
export const MAX_RESULTS = 50;
/** Rows in the untyped list. */
export const DEFAULT_COUNT = 20;

/** Case- and diacritic-insensitive form: "erdos" finds the accented name. */
export function fold(text: string): string {
  return text.normalize('NFKD').replace(/\p{M}/gu, '').toLowerCase();
}

export interface Prepared {
  row: PaperRow;
  title: string;
  authors: string;
  year: string;
  venue: string;
  citeKey: string;
  alpha: string;
}

export function prepare(rows: readonly PaperRow[]): Prepared[] {
  return rows.map((row) => ({
    row,
    title: fold(row.title),
    authors: fold(row.authors),
    year: row.year === null ? '' : String(row.year),
    venue: fold(row.venue ?? ''),
    citeKey: fold(row.citeKey),
    alpha: fold(row.alphaLabel),
  }));
}

function isWordStart(text: string, at: number): boolean {
  if (at === 0) return true;
  return !/[a-z0-9]/.test(text[at - 1]!);
}

/** Whether `term` begins a word somewhere in `text`. */
function startsWord(text: string, term: string): boolean {
  let at = text.indexOf(term);
  while (at >= 0) {
    if (isWordStart(text, at)) return true;
    at = text.indexOf(term, at + 1);
  }
  return false;
}

interface Scored {
  item: Prepared;
  /** Lexicographic: compared field by field, larger first. */
  rank: [number, number, number, number, number];
}

/** Score one row, or null when some term matches nowhere.
 *
 * A term matches when it occurs in any field, or as a subsequence of the
 * title or cite key (the palette's scorer, which already refuses a match
 * smeared across a long title). */
function score(item: Prepared, whole: string, terms: readonly string[]): Scored | null {
  let wordStarts = 0;
  let fuzzy = 0;
  for (const term of terms) {
    const titleScore = subsequenceScore(item.title, term);
    const keyScore = subsequenceScore(item.citeKey, term);
    const literal =
      item.title.includes(term) ||
      item.authors.includes(term) ||
      item.year.includes(term) ||
      item.venue.includes(term) ||
      item.citeKey.includes(term) ||
      item.alpha.includes(term);
    if (!literal && titleScore === null && keyScore === null) return null;
    if (startsWord(item.title, term) || item.citeKey.startsWith(term)) wordStarts += 1;
    fuzzy += Math.max(titleScore ?? 0, (keyScore ?? 0) * 0.6);
  }
  const exactKey = item.citeKey === whole || item.alpha === whole ? 1 : 0;
  const titlePrefix = item.title.startsWith(whole) ? 1 : 0;
  return {
    item,
    rank: [exactKey, titlePrefix, wordStarts, fuzzy, item.row.year ?? 0],
  };
}

function compare(a: Scored, b: Scored): number {
  for (let i = 0; i < a.rank.length; i += 1) {
    const diff = b.rank[i]! - a.rank[i]!;
    if (diff !== 0) return diff;
  }
  return a.item.title.localeCompare(b.item.title);
}

/** The typed list: every term must match, then in order an exact cite key
 * or alpha label, a title that starts with the query, terms that start a
 * title word (or the cite key), the palette's subsequence score, and newest
 * first. */
export function rankRows(
  items: readonly Prepared[],
  query: string,
  limit: number = MAX_RESULTS
): PaperRow[] {
  const whole = fold(query).trim().replace(/\s+/g, ' ');
  if (whole.length === 0) return [];
  const terms = whole.split(' ');
  const scored: Scored[] = [];
  for (const item of items) {
    const entry = score(item, whole, terms);
    if (entry) scored.push(entry);
  }
  scored.sort(compare);
  return scored.slice(0, limit).map((entry) => entry.item.row);
}

/** The untyped list: the most recently opened items, newest first, when there
 * are any; otherwise the head of the library's own order. */
export function defaultRows(
  rows: readonly PaperRow[],
  recentIds: readonly string[],
  limit: number = DEFAULT_COUNT
): PaperRow[] {
  const byId = new Map(rows.map((row) => [row.id, row]));
  const recent: PaperRow[] = [];
  const seen = new Set<string>();
  for (const id of recentIds) {
    const row = byId.get(id);
    if (!row || seen.has(id)) continue;
    seen.add(id);
    recent.push(row);
    if (recent.length >= limit) break;
  }
  return recent.length > 0 ? recent : rows.slice(0, limit);
}

/** "Authors, Year, Venue", leaving out whatever is unknown. */
export function detailLine(row: PaperRow): string {
  return [row.authors.trim(), yearText(row), (row.venue ?? '').trim()]
    .filter((part) => part.length > 0)
    .join(', ');
}

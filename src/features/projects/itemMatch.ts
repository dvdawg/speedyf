/** What a typed query matches, the way the engine's list search does
 * (papers.rs: `fold`, `query_terms`, `matches`): lower case with the
 * diacritics folded away, split on whitespace, and every word must occur
 * somewhere in the item's text. The engine filters the scope's rows; this
 * is the same rule for what is already on screen: the graph's find, and a
 * person's items, which arrive whole. */
import type { PaperRow } from '../../types/engine';

/** Lower-cased with combining marks dropped: "gartner" finds the name
 * spelled with an umlaut. */
export function fold(text: string): string {
  return text.normalize('NFKD').toLowerCase().normalize('NFKD').replace(/\p{M}/gu, '');
}

/** The query's words, folded; none for a blank query. */
export function queryTerms(query: string): string[] {
  return query
    .split(/\s+/)
    .map(fold)
    .filter((term) => term.length > 0);
}

/** One item's searchable text, folded: what the engine searches (title,
 * authors, year, venue, cite key) and its alpha label, the graph's
 * suffixed one ("ZD24a") when given. */
export function rowSearchText(
  row: Pick<PaperRow, 'title' | 'authors' | 'year' | 'venue' | 'citeKey' | 'alphaLabel'>,
  label?: string
): string {
  return fold(
    [
      row.title,
      row.authors,
      row.year === null ? '' : String(row.year),
      row.venue ?? '',
      row.citeKey,
      label ?? row.alphaLabel,
    ].join(' ')
  );
}

/** Every term occurs in the folded text. */
export function matchesTerms(text: string, terms: readonly string[]): boolean {
  return terms.every((term) => text.includes(term));
}

/** The rows the query matches, in their order; all of them for a blank
 * query. */
export function filterRows<T extends PaperRow>(rows: readonly T[], query: string): T[] {
  const terms = queryTerms(query);
  if (terms.length === 0) return rows.slice();
  return rows.filter((row) => matchesTerms(rowSearchText(row), terms));
}

/** How the middle column's list is sorted: one choice for every scope,
 * kept across sessions under its own localStorage key, read and written
 * the way graphViewPrefs.ts does (storage may be missing or broken). */
import type { PaperRow } from '../../types/engine';

export type ListSort = 'default' | 'year' | 'title' | 'cited';

export const LIST_SORTS: readonly ListSort[] = ['default', 'year', 'title', 'cited'];

export const SORT_LABEL: Record<ListSort, string> = {
  default: 'Default',
  year: 'Year',
  title: 'Title',
  cited: 'Cited here',
};

export const SORT_KEY = 'speedyf-list-sort-v1';

function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

export function loadSort(): ListSort {
  try {
    const value = storage()?.getItem(SORT_KEY);
    return LIST_SORTS.includes(value as ListSort) ? (value as ListSort) : 'default';
  } catch {
    return 'default';
  }
}

export function saveSort(sort: ListSort): void {
  try {
    storage()?.setItem(SORT_KEY, sort);
  } catch {
    // Full or refused: the choice lasts for this session.
  }
}

export function nextSort(sort: ListSort): ListSort {
  return LIST_SORTS[(LIST_SORTS.indexOf(sort) + 1) % LIST_SORTS.length]!;
}

const sortYear = (row: PaperRow) => row.year ?? row.yearFloor ?? null;

/** Newest first, undated last; then title. */
function byYear(a: PaperRow, b: PaperRow): number {
  const ya = sortYear(a);
  const yb = sortYear(b);
  if (ya !== yb) {
    if (ya === null) return 1;
    if (yb === null) return -1;
    return yb - ya;
  }
  return a.title.localeCompare(b.title);
}

/** The rows in `sort` order; the engine's own order for 'default'. A
 * stable sort, so equal rows keep that order. */
export function sortRows(
  rows: readonly PaperRow[],
  sort: ListSort,
  citedHere: Readonly<Record<string, number>>
): PaperRow[] {
  const out = rows.slice();
  switch (sort) {
    case 'default':
      return out;
    case 'year':
      return out.sort(byYear);
    case 'title':
      return out.sort((a, b) => a.title.localeCompare(b.title));
    case 'cited':
      return out.sort((a, b) => (citedHere[b.id] ?? 0) - (citedHere[a.id] ?? 0) || byYear(a, b));
  }
}

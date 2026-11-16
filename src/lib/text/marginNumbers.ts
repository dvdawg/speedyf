/** Line numbers printed in a page's margin (the `lineno` package, a
 * journal's review copy): text the PDF really has, but not part of the
 * prose. A selection over three lines of a numbered paper would otherwise
 * read "the bound 746 holds for every 747 n", in a copy, a quote, a
 * comment.
 *
 * Told apart from numbers that belong to the page (a table column, a page
 * number, a footnote mark) by how line numbers are set: a column of bare
 * integers on one aligned edge, counting up down the page, each one beside
 * its line rather than on it - no other text on that line crosses it.
 *
 * Pure: runs in page points, y up, as the engine sends them.
 */
import type { TextRunDto } from '../../types/engine';

/** A bare integer, as a line number prints. */
const NUMBER = /^\d{1,4}$/;
/** Edges this close (points) are one aligned column. */
const ALIGN_PT = 1.5;
/** Fewer than this many in a column is not a numbered margin. */
const MIN_COLUMN = 3;
/** Line numbers count up down the page, by one, or by a few when only
 * every fifth line is numbered. */
const MAX_STEP = 10;
/** Share of a column's steps that must count up for it to be numbering. */
const COUNTING_SHARE = 0.8;

/** Which runs are margin line numbers: true at their indices. */
export function marginNumbers(runs: readonly TextRunDto[]): boolean[] {
  const out = runs.map(() => false);
  const candidates: number[] = [];
  runs.forEach((run, i) => {
    if (NUMBER.test(run.text.trim())) candidates.push(i);
  });
  if (candidates.length < MIN_COLUMN) return out;

  // Columns by right edge (lineno sets its numbers flush right) and by
  // left edge (flush left, in a right margin).
  const columns: number[][] = [];
  for (const edge of [(r: TextRunDto) => r.x + r.w, (r: TextRunDto) => r.x]) {
    const sorted = [...candidates].sort((a, b) => edge(runs[a]!) - edge(runs[b]!));
    let column: number[] = [];
    for (const i of sorted) {
      const last = column.at(-1);
      if (last !== undefined && edge(runs[i]!) - edge(runs[last]!) > ALIGN_PT) {
        if (column.length >= MIN_COLUMN) columns.push(column);
        column = [];
      }
      column.push(i);
    }
    if (column.length >= MIN_COLUMN) columns.push(column);
  }

  for (const column of columns) {
    // top of the page first
    const down = [...column].sort((a, b) => runs[b]!.y - runs[a]!.y);
    const counts = (k: number) => {
      const step = Number(runs[down[k]!]!.text) - Number(runs[down[k - 1]!]!.text);
      return step >= 1 && step <= MAX_STEP;
    };
    let counting = 0;
    for (let k = 1; k < down.length; k += 1) if (counts(k)) counting += 1;
    if (counting < COUNTING_SHARE * (down.length - 1)) continue;
    // Each one in the count: a stray number on the same edge (a page
    // number under the gutter) is not.
    down.forEach((i, k) => {
      const inCount = (k > 0 && counts(k)) || (k + 1 < down.length && counts(k + 1));
      if (inCount && besideItsLine(runs, i)) out[i] = true;
    });
  }
  return out;
}

/** No other text on the number's line crosses it: it stands in a margin or
 * a gutter, not among the words. */
function besideItsLine(runs: readonly TextRunDto[], index: number): boolean {
  const n = runs[index]!;
  const centre = n.y + n.h / 2;
  return runs.every((run, i) => {
    if (i === index || NUMBER.test(run.text.trim())) return true;
    const onLine = centre >= run.y - n.h / 2 && centre <= run.y + run.h + n.h / 2;
    const crosses = run.x < n.x + n.w + 1 && run.x + run.w > n.x - 1;
    return !(onLine && crosses);
  });
}

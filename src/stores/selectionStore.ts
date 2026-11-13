/** The text currently selected in a document, reconstructed.
 *
 * Kept here rather than read on demand because the two things that want it —
 * the command palette and, through it, extension selection actions — both run
 * at a moment when the DOM selection may already be gone. Opening the palette
 * moves focus to its input, and reading `document.getSelection()` after that
 * answers about the input, not the page.
 *
 * The text is the *smart-copied* reconstruction, not `selection.toString()`. A
 * raw PDF selection comes back as one fragment per positioned run, with broken
 * lines, ligature glyphs and words split at line ends; handing an extension
 * that instead of what a copy would produce would make every extension deal
 * with the same mess.
 */
import { createSignal } from 'solid-js';

const [selectedText, setSelectedTextSignal] = createSignal<string>('');

/** Where the selection is: the page it lies on (the one holding most of it,
 * when it spans two) and its rectangles there, in page points from the
 * displayed page's top-left corner. What `selection.glyphs` reads, and where
 * a popover about the selection is anchored. */
export interface SelectionGeometry {
  docId: number;
  page: number;
  rects: [number, number, number, number][];
}

const [selectedGeometry, setSelectedGeometry] = createSignal<SelectionGeometry | null>(null);

export { selectedText, selectedGeometry };

/** True when there is something an action could be performed on. */
export function hasSelection(): boolean {
  return selectedText().length > 0;
}

/** Settle time before a selection is reconstructed.
 *
 * `selectionchange` fires continuously while dragging, and rebuilding the text
 * walks every span in the scroller — cheap once, wasteful sixty times a second.
 * Waiting for the drag to stop costs nothing: nothing reads this until the user
 * reaches for a menu.
 */
const SETTLE_MS = 150;

let timer: ReturnType<typeof setTimeout> | undefined;

/** Called by the viewer as the selection changes. `compute` is deferred, so a
 * selection that is still being dragged is never reconstructed. */
export function noteSelectionChanged(
  compute: () => string | null,
  geometry?: () => SelectionGeometry | null
): void {
  clearTimeout(timer);
  timer = setTimeout(() => {
    // `compute` reads the *live* Selection, which may have collapsed during the
    // settle window — a click lands, focus moves, the drag ends somewhere
    // empty. An empty answer means "there is nothing to read now", never
    // "what you highlighted a moment ago is void", so the last real capture
    // stands. Only a genuine new selection replaces it.
    const text = compute();
    if (text && text.length > 0) {
      setSelectedTextSignal(text);
      setSelectedGeometry(geometry?.() ?? null);
    }
  }, SETTLE_MS);
}

/** Forget the selection entirely.
 *
 * Deliberately NOT called when the selection merely collapses. Moving focus —
 * opening the command palette, clicking a button — collapses it, and clearing
 * on that defeats the entire purpose of this store: the text would be gone by
 * the moment anything wanted to act on it. What is held here is "the last thing
 * you highlighted", which stays true until you highlight something else.
 *
 * Called when that stops being meaningful: a different document becomes
 * active, or the one it came from goes away.
 */
export function clearSelection(): void {
  clearTimeout(timer);
  setSelectedTextSignal('');
  setSelectedGeometry(null);
}

/** The geometry of selected runs, grouped onto the page holding most of them
 * and turned top-left. Pure, so the choice is testable without a DOM. */
export function selectionGeometryFrom(
  docId: number,
  runs: readonly { page: number; rect: readonly [number, number, number, number] }[],
  pageHeight: (page: number) => number | undefined
): SelectionGeometry | null {
  const counts = new Map<number, number>();
  for (const run of runs) counts.set(run.page, (counts.get(run.page) ?? 0) + 1);
  let page = -1;
  let best = 0;
  for (const [candidate, count] of counts) {
    if (count > best || (count === best && candidate < page)) {
      page = candidate;
      best = count;
    }
  }
  const height = pageHeight(page);
  if (page < 0 || height === undefined) return null;
  const rects = runs
    .filter((run) => run.page === page)
    .map(
      ({ rect: [x, y, w, h] }) => [x, height - (y + h), w, h] as [number, number, number, number]
    );
  return { docId, page, rects: mergeRows(rects) };
}

/** The engine takes at most this many rectangles in one glyph request. */
export const MAX_SELECTION_RECTS = 64;

type Rect = [number, number, number, number];

/** A long selection is one run per word group, which can be hundreds of
 * rectangles. Runs on one line (vertical overlap over half the shorter) are
 * merged first; past the cap, everything becomes one rectangle. */
function mergeRows(rects: Rect[]): Rect[] {
  if (rects.length <= MAX_SELECTION_RECTS) return rects;
  const rows: Rect[] = [];
  for (const rect of [...rects].sort((a, b) => a[1] - b[1])) {
    const last = rows.at(-1);
    const overlap = last
      ? Math.min(last[1] + last[3], rect[1] + rect[3]) - Math.max(last[1], rect[1])
      : 0;
    if (last && overlap > 0.5 * Math.min(last[3], rect[3])) {
      rows[rows.length - 1] = union(last, rect);
    } else {
      rows.push(rect);
    }
  }
  return rows.length <= MAX_SELECTION_RECTS ? rows : [rows.reduce(union)];
}

function union(a: Rect, b: Rect): Rect {
  const left = Math.min(a[0], b[0]);
  const top = Math.min(a[1], b[1]);
  const right = Math.max(a[0] + a[2], b[0] + b[2]);
  const bottom = Math.max(a[1] + a[3], b[1] + b[3]);
  return [left, top, right - left, bottom - top];
}

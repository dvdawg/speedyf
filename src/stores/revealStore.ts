/** A place in the document to point at, briefly.
 *
 * Navigating somewhere answers "which page", and that is not the same as
 * answering "where". A reference cited three times on page 6 puts you on page
 * 6 three times, and you have to find the `[12]` yourself each time — on a
 * dense page, that is most of the work the panel was supposed to save.
 *
 * So a jump can say what it was aiming at, and the viewer marks it for a
 * couple of seconds. Deliberately not a selection and not a highlight you have
 * to dismiss: it is a gesture, and a mark that outstayed the moment would
 * accumulate one per jump until the page was covered in them.
 */
import { createSignal } from 'solid-js';

/** Display-normalized page space, y-up: `[x, y_bottom, w, h]` — the same shape
 * the engine reports link and text rects in, so nothing has to convert. */
export type RevealRect = [number, number, number, number];

export interface Reveal {
  docId: number;
  /** Zero-based, as the reader sees it. */
  page: number;
  /** One box per piece of the thing being pointed at. A citation is often
   * several links — an author and a year, or two halves either side of a line
   * break — and marking one of them draws a box round part of a name. */
  rects: RevealRect[];
  /** Distinguishes one reveal from the next at the same place, so asking for
   * the same spot twice marks it twice rather than silently doing nothing. */
  token: number;
}

/** How long the mark stays. Long enough to look up from the sidebar and find
 * it, short enough not to become part of the page. */
const LINGER_MS = 2400;

const [reveal, setReveal] = createSignal<Reveal | null>(null);
let nextToken = 1;
let timer: ReturnType<typeof setTimeout> | undefined;

export { reveal };

/** Mark a place, replacing whatever was marked before.
 *
 * No way to clear it early, and none needed: it belongs to one document and
 * one page, so switching tabs cannot show it against the wrong paper, and it
 * takes itself down either way. */
export function revealPlace(docId: number, page: number, rects: readonly RevealRect[]): void {
  clearTimeout(timer);
  if (rects.length === 0) return;
  setReveal({ docId, page, rects: [...rects], token: nextToken++ });
  timer = setTimeout(() => setReveal(null), LINGER_MS);
}

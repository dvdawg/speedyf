/** The Fill tool: typing onto printed forms that have no fields.
 *
 * Holds what the tool places next (text, a mark or today's date), the blanks
 * found per page, and where Tab is headed when the next blank is on a page
 * that is not mounted yet. Blanks are only asked for while the tool is held,
 * and only then outlined: the rest of the time a page looks like its paper. */
import { createStore, produce } from 'solid-js/store';
import { createSignal } from 'solid-js';
import type { Blank } from '../../types/engine';
import type { MarkKind, PageId } from '../../types/model';
import { engine } from '../../lib/transport/engine';
import { setActiveTool, toolState } from '../annotations/toolStore';

export type FillMode = 'text' | 'date' | MarkKind;

interface FillState {
  mode: FillMode;
  /** Blanks per `${docId}:${src}`, once found. */
  blanks: Record<string, Blank[]>;
}

const [fillState, setFillState] = createStore<FillState>({ mode: 'text', blanks: {} });
export { fillState };

const pending = new Map<string, Promise<Blank[]>>();

const key = (docId: number, src: number) => `${docId}:${src}`;

/** The blanks on a page, fetched once per document and page. */
export function loadBlanks(docId: number, src: number): Promise<Blank[]> {
  const k = key(docId, src);
  const known = fillState.blanks[k];
  if (known) return Promise.resolve(known);
  let request = pending.get(k);
  if (!request) {
    request = engine
      .getPageBlanks(docId, src)
      .catch(() => [] as Blank[])
      .then((found) => {
        setFillState('blanks', k, found);
        pending.delete(k);
        return found;
      });
    pending.set(k, request);
  }
  return request;
}

export function blanksOf(docId: number, src: number): Blank[] | undefined {
  return fillState.blanks[key(docId, src)];
}

export function setFillMode(mode: FillMode) {
  setFillState('mode', mode);
}

export const fillActive = () => toolState.active === 'fill';

/** Pick up the Fill tool (in text mode), or put it down. */
export function toggleFill(mode: FillMode = 'text') {
  if (fillActive() && fillState.mode === mode) {
    setActiveTool('select');
    return;
  }
  setFillState('mode', mode);
  setActiveTool('fill');
}

// ---------- which blank a text box was typed into ----------

/** Text boxes the tool made, by annotation id: the page and blank they
 * fill, so Tab knows where it is leaving from. Session only. */
const filled = new Map<string, { pageId: PageId; blank: number }>();

export function linkBlank(annotId: string, pageId: PageId, blank: number) {
  filled.set(annotId, { pageId, blank });
}

export function blankOfAnnot(annotId: string): { pageId: PageId; blank: number } | undefined {
  return filled.get(annotId);
}

// ---------- Tab onto a page that is not mounted yet ----------

const [arrival, setArrival] = createSignal<{ pageId: PageId; blank: number } | null>(null);
export { arrival };

/** Ask the page `pageId` to open blank `blank` for typing as soon as it is
 * shown. */
export function requestArrival(pageId: PageId, blank: number) {
  setArrival({ pageId, blank });
}

export function takeArrival(pageId: PageId): number | null {
  const a = arrival();
  if (!a || a.pageId !== pageId) return null;
  setArrival(null);
  return a.blank;
}

/** Test hook. */
export function resetFill() {
  setFillState(
    produce((s) => {
      s.mode = 'text';
      s.blanks = {};
    })
  );
  pending.clear();
  filled.clear();
  setArrival(null);
}

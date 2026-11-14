/** Opening a library item at a place: what a notepad link does, and
 * `navigate.openItem` for any extension holding `projects:read`.
 *
 * Through the reader's own plumbing: the item's file opens in a tab (or its
 * tab comes forward), the jump is recorded for Back and Forward, and the
 * place is marked for a moment, as a References row marks a citation. */
import { engine } from '../../lib/transport/engine';
import { pdfRectToCssRect } from '../../lib/coordinates/coords';
import { revealPlace } from '../../stores/revealStore';
import { findTabByPath, type TabRecord } from '../../stores/tabsStore';
import { openInNewTabOrFocus } from '../document/tabsController';
import { pathFor } from '../projects/openPaper';
import { jumpToPage } from '../viewer/navHistory';
import { noteOrigin } from './readerContext';

/** A top-left `[x, y, w, h]` box on the page as displayed, in points, as
 * the engine's y-up `[x, y_bottom, w, h]`. Null on a rotated page, where
 * the two spaces are not one flip apart. */
export function toYUp(
  box: readonly [number, number, number, number],
  geom: { heightPt: number; rotation: number }
): [number, number, number, number] | null {
  if (geom.rotation !== 0) return null;
  const [x, y, w, h] = box;
  return [x, geom.heightPt - (y + h), w, h];
}

/** Jump to `page`, and to `box` on it (top-left page points) when given:
 * framed a third of the way down and marked briefly. */
export function goToPlace(
  tab: TabRecord,
  page: number,
  box: readonly [number, number, number, number] | null
): void {
  const pages = tab.documentStore.state.pages.length;
  if (pages === 0) return;
  const at = Math.min(Math.max(0, page), pages - 1);
  const geom = tab.zoom.pagesGeom()[at];
  const yUp = box && geom ? toYUp(box, geom) : null;
  if (!geom || !yUp) {
    jumpToPage(tab, at);
    return;
  }
  revealPlace(tab.documentStore.state.docId, at, [yUp]);
  const css = pdfRectToCssRect(
    { x: yUp[0], y: yUp[1], w: yUp[2], h: yUp[3] },
    geom,
    tab.viewport.state.zoom
  );
  jumpToPage(tab, at, Math.max(0, css.y - tab.viewport.state.containerH * 0.35));
}

/** Open item `itemId`, at `page` (zero-based) and `box` when given. The
 * project it is opened from is remembered, so reader actions on it write
 * to that project's notepad. Returns whether it opened here. */
export async function openItemAt(
  itemId: string,
  opts: {
    projectId?: string | null;
    page?: number | null;
    box?: readonly [number, number, number, number] | null;
  } = {}
): Promise<boolean> {
  const paper = await engine.libraryPaper(itemId);
  const path = pathFor(paper);
  if (!path) throw new Error('this item has no file to open');
  if (opts.projectId && paper.projectIds.includes(opts.projectId)) {
    noteOrigin(path, opts.projectId);
  }
  if (!(await openInNewTabOrFocus(path))) return false;
  if (opts.page === undefined || opts.page === null) return true;
  const tab = findTabByPath(path);
  // Open in another window: that window has it, at its own place.
  if (!tab) return true;
  await tab.ready;
  goToPlace(tab, opts.page, opts.box ?? null);
  return true;
}

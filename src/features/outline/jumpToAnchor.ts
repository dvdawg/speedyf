/** Shared navigation for the outline, formal-environment and breadcrumb
 * surfaces.
 *
 * A PDF destination gives a page and, usually, a y within it. Landing on the
 * page alone drops you at its top edge, which for a heading halfway down means
 * hunting for it; using the y puts the target where you are already looking. */
import type { ViewportStore } from '../../stores/viewportStore';
import type { DocState } from '../document/documentStore';

/** Position of a source page in the tab, which is not the source index itself
 * once pages have been reordered, duplicated or deleted in this session. */
export function layoutIndexOf(doc: DocState, srcPage: number): number | null {
  const index = doc.pages.findIndex((page) => page.srcIndex === srcPage);
  return index >= 0 ? index : null;
}

/** Converts a destination y (page space, points, y-up) into the offset the
 * viewer wants: CSS px down from the page's top edge. The viewer already
 * subtracts its own padding, which is what leaves the target a comfortable
 * distance below the top rather than flush against it.
 *
 * `srcPage` is a source page index — what every PDF destination refers to. */
export function jumpToAnchor(
  vp: ViewportStore,
  doc: DocState,
  srcPage: number,
  y: number | null
): void {
  const page = layoutIndexOf(doc, srcPage);
  if (page === null) return; // the page was deleted in this session
  const geom = doc.pages[page];
  if (y === null || !geom) {
    vp.requestScrollToPage(page);
    return;
  }
  const rotation = (geom.baseRotation + geom.userRotation + vp.state.viewRotation) % 360;
  const heightPt = rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
  vp.requestScrollToPage(page, Math.max(0, (heightPt - y) * vp.state.zoom));
}

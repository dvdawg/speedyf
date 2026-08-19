/** Shared navigation for the outline and formal-environment panels.
 *
 * A PDF destination gives a page and, usually, a y within it. Landing on the
 * page alone drops you at its top edge, which for a heading halfway down means
 * hunting for it; using the y puts the target where you are already looking. */
import type { ViewportStore } from '../../stores/viewportStore';
import type { DocState } from '../document/documentStore';

/** Converts a destination y (page space, points, y-up) into the offset the
 * viewer wants: CSS px down from the page's top edge. The viewer already
 * subtracts its own padding, which is what leaves the target a comfortable
 * distance below the top rather than flush against it. */
export function jumpToAnchor(
  vp: ViewportStore,
  doc: DocState,
  page: number,
  y: number | null
): void {
  const geom = doc.pages[page];
  if (y === null || !geom) {
    vp.requestScrollToPage(page);
    return;
  }
  const rotation = (geom.baseRotation + geom.userRotation + vp.state.viewRotation) % 360;
  const heightPt = rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
  vp.requestScrollToPage(page, Math.max(0, (heightPt - y) * vp.state.zoom));
}

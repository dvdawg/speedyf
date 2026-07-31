/** Zoom/fit orchestration shared by Toolbar, shortcuts, and the Viewer.
 * All geometry goes through lib/coordinates; the Viewer registers its scroll
 * element so zoom changes can re-anchor the scroll position. */
import { documentStore } from '../document/documentStore';
import {
  clampZoom,
  PAGE_GAP,
  setViewport,
  VIEW_PADDING,
  viewport,
} from '../../stores/viewportStore';
import type { PageGeom } from '../../lib/coordinates/coords';
import { anchorScrollTop, fitPageZoom, fitWidthZoom } from '../../lib/coordinates/coords';
import type { Layout } from '../../lib/coordinates/layout';
import { layoutPages } from '../../lib/coordinates/layout';
import type { FitMode, Rotation } from '../../types/model';

let scroller: HTMLElement | null = null;

export function registerScroller(el: HTMLElement | null) {
  scroller = el;
}

export function pagesGeom(): PageGeom[] {
  return documentStore.state.pages.map((p) => ({
    widthPt: p.widthPt,
    heightPt: p.heightPt,
    rotation: ((p.baseRotation + p.userRotation + viewport.viewRotation) % 360) as Rotation,
  }));
}

export function layoutFor(zoom: number, geoms: PageGeom[] = pagesGeom()): Layout {
  return layoutPages(geoms, zoom, {
    gap: PAGE_GAP,
    padding: VIEW_PADDING,
    containerW: viewport.containerW,
  });
}

/** Set zoom keeping the content under `anchorY` (viewport px) stable. */
export function setZoomAnchored(zoom: number, anchorY?: number, fitMode: FitMode = 'custom') {
  const target = clampZoom(zoom);
  if (target === viewport.zoom) {
    setViewport('fitMode', fitMode);
    return;
  }
  const anchor = anchorY ?? viewport.containerH / 2;
  const geoms = pagesGeom();
  const oldLayout = layoutFor(viewport.zoom, geoms);
  const newLayout = layoutFor(target, geoms);
  const newScroll = anchorScrollTop(oldLayout, newLayout, viewport.scrollTop, anchor);
  setViewport({ zoom: target, fitMode });
  if (scroller) {
    // layout heights update synchronously with the store; re-anchor next frame
    requestAnimationFrame(() => {
      if (scroller) scroller.scrollTop = newScroll;
    });
  }
}

export function zoomStep(direction: 1 | -1) {
  const steps = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5, 6];
  const z = viewport.zoom;
  const next =
    direction > 0
      ? (steps.find((s) => s > z + 1e-6) ?? steps[steps.length - 1]!)
      : ([...steps].reverse().find((s) => s < z - 1e-6) ?? steps[0]!);
  setZoomAnchored(next);
}

/** Fit based on the current page's rotated geometry. */
export function applyFit(mode: 'fit-page' | 'fit-width') {
  const geoms = pagesGeom();
  const geom = geoms[Math.min(viewport.currentPage, geoms.length - 1)];
  if (!geom) return;
  const zoom =
    mode === 'fit-page'
      ? fitPageZoom(geom, viewport.containerW, viewport.containerH, VIEW_PADDING)
      : fitWidthZoom(geom, viewport.containerW, VIEW_PADDING);
  setZoomAnchored(zoom, undefined, mode);
}

/** Re-apply the active fit mode (window resized / rotation changed). */
export function refreshFit() {
  if (viewport.fitMode === 'fit-page' || viewport.fitMode === 'fit-width') {
    applyFit(viewport.fitMode);
  }
}

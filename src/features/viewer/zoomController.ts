/** Zoom/fit orchestration shared by Toolbar, shortcuts, and the Viewer.
 * All geometry goes through lib/coordinates; the Viewer registers its scroll
 * element so zoom changes can re-anchor the scroll position.
 * One instance per open tab (see tabsStore.ts); `createZoomController()` is
 * the factory, with a temporary module-level singleton kept below until the
 * tab-registry wiring lands. */
import type { DocumentStore } from '../document/documentStore';
import { clampZoom, PAGE_GAP, VIEW_PADDING, type ViewportStore } from '../../stores/viewportStore';
import type { AnchorPoint, PageGeom, ScrollPos } from '../../lib/coordinates/coords';
import { anchorScroll, fitPageZoom, fitWidthZoom } from '../../lib/coordinates/coords';
import type { Layout } from '../../lib/coordinates/layout';
import { layoutPages } from '../../lib/coordinates/layout';
import type { FitMode, Rotation } from '../../types/model';

export interface ZoomController {
  registerScroller(el: HTMLElement | null): void;
  pagesGeom(): PageGeom[];
  layoutFor(zoom: number, geoms?: PageGeom[]): Layout;
  setZoomAnchored(zoom: number, anchor?: AnchorPoint, fitMode?: FitMode): void;
  zoomStep(direction: 1 | -1): void;
  applyFit(mode: 'fit-page' | 'fit-width'): void;
  refreshFit(): void;
}

export function createZoomController(doc: DocumentStore, vp: ViewportStore): ZoomController {
  let scroller: HTMLElement | null = null;

  function pagesGeom(): PageGeom[] {
    return doc.state.pages.map((p) => ({
      widthPt: p.widthPt,
      heightPt: p.heightPt,
      rotation: ((p.baseRotation + p.userRotation + vp.state.viewRotation) % 360) as Rotation,
    }));
  }

  function layoutFor(zoom: number, geoms: PageGeom[] = pagesGeom()): Layout {
    return layoutPages(geoms, zoom, {
      gap: PAGE_GAP,
      padding: VIEW_PADDING,
      containerW: vp.state.containerW,
    });
  }

  function setZoomAnchored(zoom: number, anchor?: AnchorPoint, fitMode: FitMode = 'custom') {
    const target = clampZoom(zoom);
    if (target === vp.state.zoom) {
      vp.setState('fitMode', fitMode);
      return;
    }
    // The scroll element is the source of truth for the current position, not
    // vp.state: the store is written from a rAF-throttled scroll handler and
    // so lags the DOM by up to a frame. Anchoring off the stale value made
    // every step of a continuous gesture compound the same offset, which is
    // what walked the page out from under the cursor mid-pinch.
    const from: ScrollPos = scroller
      ? { top: scroller.scrollTop, left: scroller.scrollLeft }
      : { top: vp.state.scrollTop, left: vp.state.scrollLeft };
    const point = anchor ?? { x: vp.state.containerW / 2, y: vp.state.containerH / 2 };
    const geoms = pagesGeom();
    const oldLayout = layoutFor(vp.state.zoom, geoms);
    const newLayout = layoutFor(target, geoms);
    const next = anchorScroll(oldLayout, newLayout, from, point);
    vp.setState({ zoom: target, fitMode });
    if (scroller) {
      // Zoom and scroll must land in the SAME frame. Handing the scroll off to
      // rAF (as this used to) let the browser paint one frame at the new zoom
      // with the old offset — a visible jump on every step of a gesture.
      //
      // Assigning here is safe because the setState above has already flushed
      // the canvas's new size to the DOM, so the browser clamps against the
      // right bounds. That holds for every caller today — rAF callbacks, the
      // ResizeObserver, and Solid's delegated event handlers all run outside
      // any batch. It would NOT hold inside an explicit batch(), where render
      // effects are deferred to the end and this would clamp to a stale
      // scrollHeight; don't wrap a zoom call in one.
      scroller.scrollTop = next.top;
      scroller.scrollLeft = next.left;
      // Publish what the browser actually settled on, so the next step of the
      // gesture and the tile-visibility math both read a current value.
      vp.setState({ scrollTop: scroller.scrollTop, scrollLeft: scroller.scrollLeft });
    }
  }

  function zoomStep(direction: 1 | -1) {
    const steps = [0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5, 6];
    const z = vp.state.zoom;
    const next =
      direction > 0
        ? (steps.find((s) => s > z + 1e-6) ?? steps[steps.length - 1]!)
        : ([...steps].reverse().find((s) => s < z - 1e-6) ?? steps[0]!);
    setZoomAnchored(next);
  }

  function applyFit(mode: 'fit-page' | 'fit-width') {
    const geoms = pagesGeom();
    const geom = geoms[Math.min(vp.state.currentPage, geoms.length - 1)];
    if (!geom) return;
    const zoom =
      mode === 'fit-page'
        ? fitPageZoom(geom, vp.state.containerW, vp.state.containerH, VIEW_PADDING)
        : fitWidthZoom(geom, vp.state.containerW, VIEW_PADDING);
    setZoomAnchored(zoom, undefined, mode);
  }

  function refreshFit() {
    if (vp.state.fitMode === 'fit-page' || vp.state.fitMode === 'fit-width') {
      applyFit(vp.state.fitMode);
    }
  }

  return {
    registerScroller(el: HTMLElement | null) {
      scroller = el;
    },
    pagesGeom,
    layoutFor,
    setZoomAnchored,
    zoomStep,
    applyFit,
    refreshFit,
  };
}

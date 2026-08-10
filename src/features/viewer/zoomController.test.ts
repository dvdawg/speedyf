import { describe, expect, it } from 'vitest';
import { createDocumentStore } from '../document/documentStore';
import { createViewportStore } from '../../stores/viewportStore';
import { createZoomController } from './zoomController';
import { pageIndexAt } from '../../lib/coordinates/layout';
import type { DocMeta } from '../../types/model';

const meta: DocMeta = {
  docId: 1,
  path: '/tmp/a.pdf',
  name: 'a.pdf',
  pageCount: 5,
  sizes: Array.from({ length: 5 }, () => [612, 792, 0, 0, 0] as [number, number, number, number, number]),
  estimatedSize: [612, 792],
};

const VIEW_W = 900;
const VIEW_H = 700;

/** Stands in for the scroll container. Clamps assignments the way a real one
 * does, against the content size at the *current* zoom — which is the whole
 * point: the controller must have applied the new layout before it scrolls. */
function harness() {
  const documentStore = createDocumentStore();
  const viewport = createViewportStore();
  const zoom = createZoomController(documentStore, viewport);
  documentStore.initFromMeta(meta);
  viewport.setState({ containerW: VIEW_W, containerH: VIEW_H });

  let top = 0;
  let left = 0;
  const content = () => {
    const l = zoom.layoutFor(viewport.state.zoom);
    return { w: l.totalW, h: l.totalH };
  };
  const scroller = {
    get scrollTop() {
      return top;
    },
    set scrollTop(v: number) {
      top = Math.max(0, Math.min(v, Math.max(0, content().h - VIEW_H)));
    },
    get scrollLeft() {
      return left;
    },
    set scrollLeft(v: number) {
      left = Math.max(0, Math.min(v, Math.max(0, content().w - VIEW_W)));
    },
  } as HTMLElement;

  zoom.registerScroller(scroller);
  return { documentStore, viewport, zoom, scroller };
}

/** Which page, and where within it, currently sits under the anchor. */
function contentUnderAnchor(
  zoom: ReturnType<typeof createZoomController>,
  viewport: ReturnType<typeof createViewportStore>,
  scroller: HTMLElement,
  anchor: { x: number; y: number }
) {
  const l = zoom.layoutFor(viewport.state.zoom);
  const cy = scroller.scrollTop + anchor.y;
  const i = pageIndexAt(l, cy);
  return {
    page: i,
    fracY: (cy - l.tops[i]!) / l.heights[i]!,
    fracX: (scroller.scrollLeft + anchor.x - l.lefts[i]!) / l.widths[i]!,
  };
}

describe('setZoomAnchored', () => {
  it('applies the scroll correction in the same tick as the zoom', () => {
    // Deferring this to rAF let the browser paint one frame at the new zoom
    // with the old scroll offset — the jump seen on every gesture step.
    const { viewport, zoom, scroller } = harness();
    scroller.scrollTop = 1200;
    const before = scroller.scrollTop;

    zoom.setZoomAnchored(2, { x: 450, y: 350 });

    expect(viewport.state.zoom).toBe(2);
    expect(scroller.scrollTop).not.toBe(before);
  });

  it('publishes the settled scroll position to the store', () => {
    // The store is otherwise only written by a rAF-throttled scroll handler,
    // so without this the next gesture step anchors off a stale offset.
    const { viewport, zoom, scroller } = harness();
    scroller.scrollTop = 1200;

    zoom.setZoomAnchored(2, { x: 450, y: 350 });

    expect(viewport.state.scrollTop).toBe(scroller.scrollTop);
    expect(viewport.state.scrollLeft).toBe(scroller.scrollLeft);
  });

  it('holds the content under the cursor across a continuous pinch', () => {
    const { viewport, zoom, scroller } = harness();
    const anchor = { x: 450, y: 240 };
    scroller.scrollTop = 1500;
    const start = contentUnderAnchor(zoom, viewport, scroller, anchor);

    // ~50 frames of a pinch-in, each anchored off the result of the last.
    for (let i = 0; i < 50; i++) {
      zoom.setZoomAnchored(viewport.state.zoom * 1.03, anchor);
    }

    expect(viewport.state.zoom).toBeGreaterThan(4);
    const end = contentUnderAnchor(zoom, viewport, scroller, anchor);
    expect(end.page).toBe(start.page);
    expect(end.fracY).toBeCloseTo(start.fracY, 4);
  });

  it('holds the anchor horizontally once the page overflows the viewport', () => {
    const { viewport, zoom, scroller } = harness();
    // Zoom in far enough that the page is wider than the viewport and can
    // actually scroll horizontally, then pinch from an off-centre point.
    zoom.setZoomAnchored(2);
    scroller.scrollLeft = 150;
    const anchor = { x: 200, y: 300 };
    const start = contentUnderAnchor(zoom, viewport, scroller, anchor);
    expect(scroller.scrollLeft).toBeGreaterThan(0);

    for (let i = 0; i < 20; i++) {
      zoom.setZoomAnchored(viewport.state.zoom * 1.03, anchor);
    }

    const end = contentUnderAnchor(zoom, viewport, scroller, anchor);
    expect(end.fracX).toBeCloseTo(start.fracX, 4);
    expect(end.fracY).toBeCloseTo(start.fracY, 4);
  });

  it('is reversible: zooming out along the same path returns to the start', () => {
    const { viewport, zoom, scroller } = harness();
    const anchor = { x: 450, y: 240 };
    scroller.scrollTop = 1500;
    const startTop = scroller.scrollTop;

    for (let i = 0; i < 25; i++) zoom.setZoomAnchored(viewport.state.zoom * 1.05, anchor);
    for (let i = 0; i < 25; i++) zoom.setZoomAnchored(viewport.state.zoom / 1.05, anchor);

    expect(viewport.state.zoom).toBeCloseTo(1, 6);
    expect(scroller.scrollTop).toBeCloseTo(startTop, 4);
  });
});

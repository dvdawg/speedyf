import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createDocumentStore } from '../../document/documentStore';
import { createViewportStore } from '../../../stores/viewportStore';
import { createZoomController } from '../zoomController';
import { clearColumnCache, createColumnMode, type ColumnMode } from './columnMode';
import { GESTURE_GAP_MS, PUSH_PX, MARGIN_X } from './columnFlow';
import type { ColumnLayout, ReadingSegment } from '../../../types/engine';
import type { DocMeta } from '../../../types/model';

const VIEW_W = 900;
const VIEW_H = 700;

const col = (x: number, y: number, w: number, h: number, column = 0): ReadingSegment => ({
  kind: 'column',
  rect: [x, y, w, h],
  band: 0,
  column,
});
const block = (x: number, y: number, w: number, h: number): ReadingSegment => ({
  kind: 'block',
  rect: [x, y, w, h],
  band: 0,
  column: 0,
});
const reading = (kind: ColumnLayout['kind'], segments: ReadingSegment[]): ColumnLayout => ({
  src: 0,
  kind,
  columns: kind === 'columns' ? 2 : kind === 'single' ? 1 : 0,
  bands: kind === 'columns' ? 1 : 0,
  segments,
  source: 'text',
  reason: kind === 'none' ? 'slides' : '',
});

/** Page 0: title, two columns. Page 1: two columns. Page 2: one text block.
 * Page 3: no reading (a slide). Page 4: landscape, two columns. */
const LAYOUTS: ColumnLayout[] = [
  reading('columns', [block(54, 690, 504, 50), col(54, 72, 240, 600), col(318, 72, 240, 600, 1)]),
  reading('columns', [col(54, 72, 240, 648), col(318, 72, 240, 648, 1)]),
  reading('single', [block(90, 72, 432, 648)]),
  reading('none', []),
  reading('columns', [col(54, 54, 330, 504), col(408, 54, 330, 504, 1)]),
];

const meta: DocMeta = {
  docId: 7,
  path: '/tmp/paper.pdf',
  name: 'paper.pdf',
  pageCount: 5,
  sizes: [
    [612, 792, 0, 0, 0],
    [612, 792, 0, 0, 0],
    [612, 792, 0, 0, 0],
    [612, 792, 0, 0, 0],
    [792, 612, 0, 0, 0],
  ],
  estimatedSize: [612, 792],
};

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function harness() {
  const documentStore = createDocumentStore();
  const viewport = createViewportStore();
  const zoom = createZoomController(documentStore, viewport);
  documentStore.initFromMeta(meta);
  viewport.setState({ containerW: VIEW_W, containerH: VIEW_H });
  zoom.applyFit('fit-width');

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
  };
  zoom.registerScroller(scroller as unknown as HTMLElement);

  let now = 0;
  const fetched: number[] = [];
  let mode!: ColumnMode;
  const dispose = createRoot((d) => {
    mode = createColumnMode({ documentStore, viewport, zoom }, scroller, {
      fetchLayout: (_docId, src) => {
        fetched.push(src);
        return Promise.resolve({ ...LAYOUTS[src]!, src });
      },
      now: () => now,
      reducedMotion: () => true,
      setTimer: (fn) => {
        fn();
        return () => {};
      },
    });
    return d;
  });

  /** A point on a page, as the viewport sees it now. */
  const pointOn = (page: number, x: number, yUp: number) => {
    const l = zoom.layoutFor(viewport.state.zoom);
    const z = viewport.state.zoom;
    const h = meta.sizes[page]![1];
    return {
      x: l.lefts[page]! + x * z - scroller.scrollLeft,
      y: l.tops[page]! + (h - yUp) * z - scroller.scrollTop,
    };
  };
  /** A fresh wheel gesture of `steps` events of `dy` each. */
  const gesture = (dy: number, steps = 1) => {
    now += GESTURE_GAP_MS + 100;
    for (let i = 0; i < steps; i++) {
      now += 16;
      mode.wheel({ deltaY: dy, deltaMode: 0 });
    }
  };
  /** Scroll to the end of the current segment, then push past it. */
  const readOn = async (dir: 1 | -1) => {
    gesture(dir * 4000, 3);
    gesture(dir * (PUSH_PX + 10));
    await flush();
  };
  return {
    documentStore,
    viewport,
    zoom,
    scroller,
    mode,
    pointOn,
    gesture,
    readOn,
    fetched,
    dispose,
  };
}

afterEach(() => clearColumnCache());

describe('entering', () => {
  it('fits the column under the pointer and holds the view on it', async () => {
    const h = harness();
    h.scroller.scrollTop = 0;
    await h.mode.toggle(h.pointOn(0, 400, 400));
    expect(h.mode.active()).toBe(true);
    expect(h.viewport.state.fitMode).toBe('column');
    expect(h.mode.current()).toEqual({ page: 0, index: 2 });
    expect(240 * h.viewport.state.zoom).toBeCloseTo(VIEW_W - 2 * MARGIN_X);
    // The column is centred: its left edge sits one margin in.
    const l = h.zoom.layoutFor(h.viewport.state.zoom);
    expect(l.lefts[0]! + 318 * h.viewport.state.zoom - h.scroller.scrollLeft).toBeCloseTo(MARGIN_X);
    h.dispose();
  });

  it('fits a single-column page to its text block, margins trimmed', async () => {
    const h = harness();
    h.viewport.requestScrollToPage(2);
    h.scroller.scrollTop = h.zoom.layoutFor(h.viewport.state.zoom).tops[2]!;
    await h.mode.toggle(null);
    expect(h.mode.current()).toEqual({ page: 2, index: 0 });
    expect(432 * h.viewport.state.zoom).toBeCloseTo(VIEW_W - 2 * MARGIN_X);
    h.dispose();
  });

  it('falls back to the page width on a page with no reading, without the mode', async () => {
    const h = harness();
    h.zoom.setZoomAnchored(2);
    h.scroller.scrollTop = h.zoom.layoutFor(h.viewport.state.zoom).tops[3]! + 10;
    await h.mode.toggle(null);
    expect(h.mode.active()).toBe(false);
    expect(h.viewport.state.fitMode).toBe('fit-width');
    h.dispose();
  });

  it('enters on a double-click in a column, but not on a title or a single-column page', async () => {
    const h = harness();
    expect(await h.mode.doubleClick(h.pointOn(0, 300, 710))).toBe(false);
    expect(h.mode.active()).toBe(false);
    h.scroller.scrollTop = h.zoom.layoutFor(h.viewport.state.zoom).tops[2]!;
    expect(await h.mode.doubleClick(h.pointOn(2, 300, 400))).toBe(false);
    h.scroller.scrollTop = 0;
    expect(await h.mode.doubleClick(h.pointOn(0, 100, 400))).toBe(true);
    expect(h.mode.current()).toEqual({ page: 0, index: 1 });
    // A second one leaves.
    expect(await h.mode.doubleClick(h.pointOn(0, 100, 400))).toBe(true);
    expect(h.mode.active()).toBe(false);
    h.dispose();
  });
});

describe('reading on', () => {
  it('goes down the left column, up to the right one, then to the next page', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    expect(h.mode.current()).toEqual({ page: 0, index: 1 });
    await h.readOn(1);
    expect(h.mode.current()).toEqual({ page: 0, index: 2 });
    // At the right column's top.
    const z = h.viewport.state.zoom;
    const l = h.zoom.layoutFor(z);
    const colTop = l.tops[0]! + (792 - 672) * z;
    expect(h.scroller.scrollTop).toBeLessThanOrEqual(colTop);
    expect(h.scroller.scrollTop).toBeGreaterThan(colTop - 40);
    await h.readOn(1);
    expect(h.mode.current()).toEqual({ page: 1, index: 0 });
    h.dispose();
  });

  it('does not skip a column on one long swipe and its momentum', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    // One gesture: events 16ms apart, far more than the column.
    h.gesture(400, 80);
    await flush();
    expect(h.mode.current()).toEqual({ page: 0, index: 1 });
    h.dispose();
  });

  it('goes back up symmetrically, landing at the bottom of the previous column', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 400, 600));
    expect(h.mode.current()).toEqual({ page: 0, index: 2 });
    h.gesture(200);
    await h.readOn(-1);
    expect(h.mode.current()).toEqual({ page: 0, index: 1 });
    await h.readOn(-1);
    expect(h.mode.current()).toEqual({ page: 0, index: 0 });
    h.dispose();
  });

  it('reads a page with no reading whole, and moves through it', async () => {
    const h = harness();
    h.scroller.scrollTop = h.zoom.layoutFor(h.viewport.state.zoom).tops[2]!;
    await h.mode.toggle(null);
    await h.readOn(1);
    expect(h.mode.current()).toEqual({ page: 3, index: 0 });
    // The whole slide's width.
    expect(612 * h.viewport.state.zoom).toBeCloseTo(VIEW_W - 2 * MARGIN_X);
    await h.readOn(1);
    // A landscape page of a different size: its own column fit.
    expect(h.mode.current()).toEqual({ page: 4, index: 0 });
    expect(330 * h.viewport.state.zoom).toBeCloseTo(VIEW_W - 2 * MARGIN_X);
    h.dispose();
  });

  it('moves on with arrows and Space at the end, but not on held-key repeats', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    // The column is about three screens tall: a few presses reach its end,
    // the next one moves on, and the last scrolls the right column.
    for (let i = 0; i < 5; i++) h.mode.key({ key: ' ', shiftKey: false, repeat: false });
    await flush();
    expect(h.mode.current()).toEqual({ page: 0, index: 2 });
    const before = h.mode.current();
    h.mode.key({ key: 'ArrowUp', shiftKey: false, repeat: true });
    expect(h.mode.current()).toEqual(before);
    h.dispose();
  });
});

describe('leaving', () => {
  it('returns to the zoom and place it left, when nothing was read', async () => {
    const h = harness();
    const zoom = h.viewport.state.zoom;
    await h.mode.toggle(h.pointOn(0, 100, 600));
    expect(h.mode.exit()).toBe(true);
    expect(h.viewport.state.zoom).toBeCloseTo(zoom);
    expect(h.viewport.state.fitMode).toBe('fit-width');
    expect(h.viewport.takeScrollRequest()).toMatchObject({ kind: 'page', page: 0 });
    h.dispose();
  });

  it('keeps the reading place at the old zoom after reading on', async () => {
    const h = harness();
    const zoom = h.viewport.state.zoom;
    await h.mode.toggle(h.pointOn(0, 100, 600));
    await h.readOn(1);
    await h.readOn(1);
    h.mode.exit();
    expect(h.viewport.state.zoom).toBeCloseTo(zoom);
    expect(h.viewport.takeScrollRequest()).toBeNull();
    const l = h.zoom.layoutFor(zoom);
    expect(h.scroller.scrollTop).toBeGreaterThan(l.tops[1]! - VIEW_H);
    h.dispose();
  });

  it('leaves at the chosen zoom when a zoom command takes over', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    h.zoom.zoomStep(-1);
    await flush();
    expect(h.mode.active()).toBe(false);
    expect(h.viewport.state.fitMode).toBe('custom');
    h.dispose();
  });

  it('leaves when the view is turned', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    h.viewport.rotateView();
    await flush();
    expect(h.mode.active()).toBe(false);
    h.dispose();
  });
});

describe('jumps', () => {
  it('lands in the column the jump aims at', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    const z = h.viewport.state.zoom;
    // A search hit in page 1's right column, framed a third down the view.
    const offset = (792 - 400) * z - VIEW_H * 0.3;
    const l = h.zoom.layoutFor(z);
    h.scroller.scrollTop = l.tops[1]! - 24 + offset;
    h.mode.afterJump({ kind: 'page', page: 1, offsetCss: offset, focusX: 400, seq: 1 });
    await flush();
    expect(h.mode.current()).toEqual({ page: 1, index: 1 });
    h.dispose();
  });

  it('goes to the first segment of a page for next/previous page', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 400, 300));
    const l = h.zoom.layoutFor(h.viewport.state.zoom);
    h.scroller.scrollTop = l.tops[1]! - 24;
    h.mode.afterJump({ kind: 'page', page: 1, seq: 2 });
    await flush();
    expect(h.mode.current()).toEqual({ page: 1, index: 0 });
    h.dispose();
  });

  it('leaves when the jump lands on a page with no columns to read', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    const l = h.zoom.layoutFor(h.viewport.state.zoom);
    h.scroller.scrollTop = l.tops[3]! - 24;
    h.mode.afterJump({ kind: 'page', page: 3, seq: 3 });
    await flush();
    expect(h.mode.active()).toBe(false);
    h.dispose();
  });
});

describe('resizing', () => {
  it('refits the column to the new width and keeps the place in it', async () => {
    const h = harness();
    await h.mode.toggle(h.pointOn(0, 100, 600));
    h.gesture(300, 1);
    h.viewport.setState({ containerW: 600 });
    h.mode.refit();
    expect(240 * h.viewport.state.zoom).toBeCloseTo(600 - 2 * MARGIN_X);
    expect(h.mode.current()).toEqual({ page: 0, index: 1 });
    h.dispose();
  });
});

/** Column zoom: one reading segment (a column, or a full-width block) fits
 * the view's width at a time, and reading flows segment to segment - down
 * the left column, up to the top of the right one, on to the next page.
 *
 * One controller per mounted Viewer. It holds no layout of its own: the
 * engine reads each page once (`get_column_layout`), the answer is kept per
 * page, and everything else is derived from the zoom controller's layout.
 *
 * The mode is the viewport's `fitMode === 'column'`. Every other zoom path
 * (zoom in/out, fit, actual size, pinch, the zoom field) sets a different fit
 * mode, and that alone ends column zoom - so an explicit zoom command always
 * wins without each one knowing about this file. Nothing is remembered
 * across sessions: column zoom is a way of reading, not a document setting.
 */
import { createEffect, createSignal, on, onCleanup } from 'solid-js';
import type { ColumnLayout } from '../../../types/engine';
import type { DocumentStore } from '../../document/documentStore';
import type { ScrollRequest, ViewportStore } from '../../../stores/viewportStore';
import { VIEW_PADDING } from '../../../stores/viewportStore';
import type { ZoomController } from '../zoomController';
import { pageIndexAt } from '../../../lib/coordinates/layout';
import { cssToPdf, pdfToPageCss } from '../../../lib/coordinates/coords';
import type { FitMode } from '../../../types/model';
import {
  centredLeft,
  columnAt,
  edgeState,
  fitZoom,
  keyStep,
  pageFlow,
  scrollRange,
  segmentAt,
  segmentBox,
  wheelPixels,
  wheelStep,
  type PageFlow,
} from './columnFlow';

/** What the controller needs of a tab. */
export interface ColumnHost {
  documentStore: DocumentStore;
  viewport: ViewportStore;
  zoom: ZoomController;
}

/** The scroll container, or a stand-in for tests. */
export interface ColumnScroller {
  scrollTop: number;
  scrollLeft: number;
  scrollTo?(options: ScrollToOptions): void;
}

export interface ColumnDeps {
  fetchLayout(docId: number, src: number): Promise<ColumnLayout>;
  now(): number;
  reducedMotion(): boolean;
  /** Delay for settling after a scroll the controller did not make. */
  setTimer(fn: () => void, ms: number): () => void;
}

/** A point in the scroll container's own viewport, CSS px. */
export interface ViewPoint {
  x: number;
  y: number;
}

export interface ColumnMode {
  active(): boolean;
  /** The segment being read, for tests and diagnostics. */
  current(): { page: number; index: number } | null;
  /** Enter at a point (or the reading position), or leave. */
  toggle(at: ViewPoint | null): Promise<void>;
  /** The Zoom to Column command: toggles at the pointer when it is over the
   * pages, else at the reading position. */
  command(): Promise<void>;
  /** Where the pointer is over the view, or null when it is elsewhere. */
  setPointer(at: ViewPoint | null): void;
  /** A double-click on blank space at a point: zooms the column there when
   * the page has columns, leaves when already zoomed. Resolves whether it
   * acted, so the caller can tidy up the selection the click made. */
  doubleClick(at: ViewPoint): Promise<boolean>;
  /** Leaves; `restore` goes back to the zoom from before. Returns whether it
   * was active. */
  exit(restore?: boolean): boolean;
  /** A wheel event while active: scrolls within the segment and moves on
   * past its end. Returns whether it took the event. */
  wheel(e: { deltaY: number; deltaMode: number }): boolean;
  /** Arrow keys and Space while active. Returns whether it took the key. */
  key(e: { key: string; shiftKey: boolean; repeat: boolean }): boolean;
  /** A programmatic scroll (a jump, page navigation, Back) just landed. */
  afterJump(req: ScrollRequest): void;
  /** The view changed size. */
  refit(): void;
  /** The scroll position changed. */
  scrolled(): void;
  setPointerDown(down: boolean): void;
  dispose(): void;
}

interface Saved {
  zoom: number;
  fitMode: FitMode;
  page: number;
  fraction: number;
  scrollLeft: number;
}

type Placement = 'start' | 'end' | { y: number; at: number } | { fraction: number };

const layoutCache = new Map<string, ColumnLayout>();
const inflight = new Map<string, Promise<ColumnLayout | null>>();
const CACHE_LIMIT = 4000;

/** Forget cached readings, for tests. */
export function clearColumnCache(): void {
  layoutCache.clear();
  inflight.clear();
}

export function createColumnMode(
  host: ColumnHost,
  scroller: ColumnScroller,
  deps: ColumnDeps
): ColumnMode {
  const vp = host.viewport;
  const doc = host.documentStore;
  const [active, setActive] = createSignal(false);
  let current: { page: number; index: number } | null = null;
  let saved: Saved | null = null;
  let entered: { page: number; index: number; top: number } | null = null;
  let edge = edgeState();
  /** Bumped by every operation, so an older one that resumes after an await
   * gives way to whatever the reader did since. */
  let seq = 0;
  /** Where the controller last put the view; anything else is a scroll it
   * did not make (the scrollbar, Home/End, a selection drag). */
  let expected = { top: -1, left: -1 };
  let animatingUntil = 0;
  let pointerDown = false;
  let settleCancel: (() => void) | null = null;
  let settleAfterPointer = false;
  let pointer: ViewPoint | null = null;

  const cacheKey = (src: number) => `${doc.state.docId}:${src}`;

  const pages = () => doc.state.pages;
  const geoms = () => host.zoom.pagesGeom();
  const layout = () => host.zoom.layoutFor(vp.state.zoom);

  function flowFrom(page: number, l: ColumnLayout | null): PageFlow | null {
    const p = pages()[page];
    const g = geoms()[page];
    if (!p || !g) return null;
    return pageFlow(l, g, p.widthPt, p.heightPt);
  }

  /** The page's flow if its reading is already known. */
  function flowNow(page: number): PageFlow | null {
    const p = pages()[page];
    if (!p) return null;
    if (p.srcIndex === null) return flowFrom(page, null);
    const l = layoutCache.get(cacheKey(p.srcIndex));
    return l ? flowFrom(page, l) : null;
  }

  async function ensure(page: number): Promise<PageFlow | null> {
    const p = pages()[page];
    if (!p) return null;
    if (p.srcIndex === null) return flowFrom(page, null);
    const k = cacheKey(p.srcIndex);
    const cached = layoutCache.get(k);
    if (cached) return flowFrom(page, cached);
    let pending = inflight.get(k);
    if (!pending) {
      pending = deps
        .fetchLayout(doc.state.docId, p.srcIndex)
        .then((l) => {
          if (layoutCache.size >= CACHE_LIMIT) layoutCache.clear();
          layoutCache.set(k, l);
          return l;
        })
        .catch(() => null)
        .finally(() => inflight.delete(k));
      inflight.set(k, pending);
    }
    return flowFrom(page, await pending);
  }

  function prefetch(page: number) {
    if (page >= 0 && page < pages().length) void ensure(page);
  }

  /** A viewport point as a page and a point on it (display points, y-up). */
  function locate(at: ViewPoint): { page: number; x: number; y: number; inside: boolean } | null {
    const l = layout();
    if (l.tops.length === 0) return null;
    const cx = scroller.scrollLeft + at.x;
    const cy = scroller.scrollTop + at.y;
    const page = pageIndexAt(l, cy);
    const g = geoms()[page];
    if (!g) return null;
    const css = { x: cx - (l.lefts[page] ?? 0), y: cy - (l.tops[page] ?? 0) };
    const inside =
      css.x >= 0 && css.y >= 0 && css.x <= (l.widths[page] ?? 0) && css.y <= (l.heights[page] ?? 0);
    const pt = cssToPdf(css, g, vp.state.zoom);
    return { page, x: pt.x, y: pt.y, inside };
  }

  function setScroll(top: number, left: number, animate: boolean) {
    if (animate && scroller.scrollTo && !deps.reducedMotion()) {
      scroller.scrollTo({ top, left, behavior: 'smooth' });
      animatingUntil = deps.now() + 700;
      expected = { top, left };
      return;
    }
    animatingUntil = 0;
    scroller.scrollTop = top;
    scroller.scrollLeft = left;
    expected = { top: scroller.scrollTop, left: scroller.scrollLeft };
    vp.setState({ scrollTop: scroller.scrollTop, scrollLeft: scroller.scrollLeft });
  }

  /** Fit a segment and put it in view. */
  function place(page: number, index: number, how: Placement, animate = false): boolean {
    const flow = flowNow(page);
    const seg = flow?.segments[index];
    const g = geoms()[page];
    if (!flow || !seg || !g) return false;
    const z = fitZoom(seg, g, vp.state.containerW, active() ? vp.state.zoom : null);
    const sameZoom = Math.abs(z - vp.state.zoom) < 1e-9;
    // The mode flag first, so the fit-mode watcher sees column zoom
    // throughout; then the zoom, which also re-lays the content out.
    host.zoom.setZoomAnchored(z, undefined, 'column');
    const l = layout();
    const box = segmentBox(seg, page, g, l, vp.state.zoom);
    const { lo, hi } = scrollRange(box, vp.state.containerH);
    const left = centredLeft(box, vp.state.containerW, l.totalW);
    let top: number;
    if (how === 'start') top = lo;
    else if (how === 'end') top = hi;
    else if ('fraction' in how) top = lo + how.fraction * (hi - lo);
    else {
      const y =
        (l.tops[page] ?? 0) + pdfToPageCss({ x: seg.rect[0], y: how.y }, g, vp.state.zoom).y;
      top = y - how.at;
    }
    top = Math.min(hi, Math.max(lo, top));
    current = { page, index };
    setScroll(top, left, animate && sameZoom);
    prefetch(page - 1);
    prefetch(page + 1);
    return true;
  }

  /** The current segment's box and scroll range at the current zoom. */
  function range(): { lo: number; hi: number; left: number } | null {
    if (!current) return null;
    const flow = flowNow(current.page);
    const seg = flow?.segments[current.index];
    const g = geoms()[current.page];
    if (!seg || !g) return null;
    const l = layout();
    const box = segmentBox(seg, current.page, g, l, vp.state.zoom);
    return {
      ...scrollRange(box, vp.state.containerH),
      left: centredLeft(box, vp.state.containerW, l.totalW),
    };
  }

  async function advance(dir: -1 | 1) {
    if (!current) return;
    const mine = ++seq;
    const { page, index } = current;
    const flow = flowNow(page) ?? (await ensure(page));
    if (mine !== seq || !active() || !flow) return;
    const next = index + dir;
    if (next >= 0 && next < flow.segments.length) {
      place(page, next, dir > 0 ? 'start' : 'end', true);
      return;
    }
    const to = page + dir;
    if (to < 0 || to >= pages().length) return;
    const nextFlow = await ensure(to);
    if (mine !== seq || !active() || !nextFlow) return;
    place(to, dir > 0 ? 0 : nextFlow.segments.length - 1, dir > 0 ? 'start' : 'end', true);
  }

  function capture(): Saved {
    const l = layout();
    const top = scroller.scrollTop;
    const page = Math.min(pageIndexAt(l, top + VIEW_PADDING), Math.max(0, pages().length - 1));
    const height = l.heights[page] || 1;
    return {
      zoom: vp.state.zoom,
      fitMode: vp.state.fitMode,
      page,
      fraction: (top - (l.tops[page] ?? 0) + VIEW_PADDING) / height,
      scrollLeft: scroller.scrollLeft,
    };
  }

  /** Enter on a page's segment, keeping the point at viewport height `at`. */
  function enter(page: number, index: number, y: number, at: number): boolean {
    saved = capture();
    edge = edgeState();
    setActive(true);
    if (!place(page, index, { y, at })) {
      setActive(false);
      saved = null;
      return false;
    }
    entered = { page, index, top: scroller.scrollTop };
    return true;
  }

  function exit(restore = true): boolean {
    if (!active()) return false;
    seq++;
    setActive(false);
    const moved =
      !entered ||
      !current ||
      current.page !== entered.page ||
      current.index !== entered.index ||
      Math.abs(scroller.scrollTop - entered.top) > 4;
    current = null;
    entered = null;
    settleCancel?.();
    settleCancel = null;
    const back = saved;
    saved = null;
    if (!restore || !back) return true;
    const fitMode: FitMode = back.fitMode === 'column' ? 'custom' : back.fitMode;
    // Back to the old zoom, anchored on what is being read now; or, when
    // nothing was read in between, exactly where the view was.
    const anchor = { x: vp.state.containerW / 2, y: vp.state.containerH * 0.3 };
    if (fitMode === 'fit-width' || fitMode === 'fit-page') host.zoom.applyFit(fitMode);
    else host.zoom.setZoomAnchored(back.zoom, moved ? anchor : undefined, fitMode);
    if (!moved) vp.requestRestoreView(back.page, back.fraction, back.scrollLeft);
    if (vp.state.fitMode === 'column') vp.setState('fitMode', fitMode);
    return true;
  }

  async function toggle(at: ViewPoint | null) {
    if (exit(true)) return;
    const mine = ++seq;
    const point = at ?? { x: vp.state.containerW / 2, y: vp.state.containerH * 0.3 };
    const where = locate(point);
    if (!where) return;
    const flow = await ensure(where.page);
    if (mine !== seq || !flow) return;
    if (flow.kind === 'none') {
      // Slides, a poster, a page with no text: no columns to read, so the
      // best the command can do is the page's width.
      host.zoom.applyFit('fit-width');
      return;
    }
    const index = segmentAt(flow.segments, at && where.inside ? where.x : null, where.y);
    if (index >= 0) enter(where.page, index, where.y, point.y);
  }

  async function doubleClick(at: ViewPoint): Promise<boolean> {
    if (exit(true)) return true;
    const mine = ++seq;
    const where = locate(at);
    if (!where || !where.inside) return false;
    const flow = await ensure(where.page);
    if (mine !== seq || !flow || flow.kind !== 'columns') return false;
    const index = columnAt(flow.segments, where.x, where.y);
    if (index < 0) return false;
    return enter(where.page, index, where.y, at.y);
  }

  function wheel(e: { deltaY: number; deltaMode: number }): boolean {
    if (!active()) return false;
    const r = range();
    if (!r) return true;
    const dy = wheelPixels(e.deltaY, e.deltaMode, vp.state.containerH);
    const top = scroller.scrollTop;
    const step = wheelStep(edge, deps.now(), dy, top, r.lo, r.hi);
    if (step.advance !== 0) void advance(step.advance);
    else if (step.top !== top || scroller.scrollLeft !== r.left) setScroll(step.top, r.left, false);
    return true;
  }

  function key(e: { key: string; shiftKey: boolean; repeat: boolean }): boolean {
    if (!active()) return false;
    let amount: number;
    if (e.key === 'ArrowDown') amount = 40;
    else if (e.key === 'ArrowUp') amount = -40;
    else if (e.key === ' ') amount = (e.shiftKey ? -1 : 1) * vp.state.containerH * 0.85;
    else return false;
    const r = range();
    if (!r) return true;
    const step = keyStep(amount, e.repeat, scroller.scrollTop, r.lo, r.hi);
    if (step.advance !== 0) void advance(step.advance);
    else setScroll(step.top, r.left, false);
    return true;
  }

  function afterJump(req: ScrollRequest) {
    if (!active()) return;
    // The scroll that just landed is the jump's, not the reader's.
    expected = { top: scroller.scrollTop, left: scroller.scrollLeft };
    const mine = ++seq;
    const l = layout();
    const at = vp.state.containerH * 0.3;
    const focusY = scroller.scrollTop + at;
    const page =
      req.kind === 'page' ? Math.min(req.page, pages().length - 1) : pageIndexAt(l, focusY);
    const toPage = req.kind === 'page' && req.offsetCss === undefined && req.fraction === undefined;
    const focusX = req.kind === 'page' ? (req.focusX ?? null) : null;
    // Imperative, like an event handler: the answer is read once, when it comes.
    // eslint-disable-next-line solid/reactivity
    void ensure(page).then((flow) => {
      if (mine !== seq || !active()) return;
      if (!flow || flow.kind === 'none') {
        // Nothing to read in columns where the jump landed.
        exit(true);
        return;
      }
      const g = geoms()[page];
      const lNow = layout();
      if (!g) return;
      const css = {
        x: 0,
        y: Math.min(Math.max(0, focusY - (lNow.tops[page] ?? 0)), lNow.heights[page] ?? 0),
      };
      const y = cssToPdf(css, g, vp.state.zoom).y;
      if (toPage && focusX === null) {
        place(page, 0, 'start');
        return;
      }
      place(page, Math.max(0, segmentAt(flow.segments, focusX, y)), { y, at });
    });
  }

  function refit() {
    if (!active() || !current) return;
    const r = range();
    const fraction = r && r.hi > r.lo ? (scroller.scrollTop - r.lo) / (r.hi - r.lo) : 0;
    place(current.page, current.index, { fraction: Math.min(1, Math.max(0, fraction)) });
  }

  /** After a scroll the controller did not make: find the segment now in
   * the middle of the view and settle into it. */
  function settle() {
    settleCancel = null;
    if (!active()) return;
    if (pointerDown) {
      settleAfterPointer = true;
      return;
    }
    const mine = ++seq;
    const at = { x: vp.state.containerW / 2, y: vp.state.containerH / 2 };
    const where = locate(at);
    if (!where) return;
    // eslint-disable-next-line solid/reactivity -- read once, when it comes
    void ensure(where.page).then((flow) => {
      if (mine !== seq || !active() || !flow) return;
      const index = Math.max(0, segmentAt(flow.segments, where.x, where.y));
      if (current && current.page === where.page && current.index === index) {
        const r = range();
        if (!r) return;
        const top = Math.min(r.hi, Math.max(r.lo, scroller.scrollTop));
        if (top !== scroller.scrollTop || r.left !== scroller.scrollLeft)
          setScroll(top, r.left, false);
        return;
      }
      place(where.page, index, { y: where.y, at: at.y });
    });
  }

  function scrolled() {
    if (!active()) return;
    if (deps.now() < animatingUntil) return;
    if (
      Math.abs(scroller.scrollTop - expected.top) < 1.5 &&
      Math.abs(scroller.scrollLeft - expected.left) < 1.5
    ) {
      return;
    }
    settleCancel?.();
    settleCancel = deps.setTimer(settle, 150);
  }

  function setPointerDown(down: boolean) {
    pointerDown = down;
    if (!down && settleAfterPointer) {
      settleAfterPointer = false;
      settle();
    }
  }

  // Any other zoom (a zoom command, a fit, a pinch) has set another fit
  // mode: that is the reader leaving column zoom, at the zoom they chose.
  createEffect(
    on(
      () => vp.state.fitMode,
      (mode) => {
        if (mode !== 'column' && active()) exit(false);
        // A tab handed over from another window arrives with the mode's
        // name but none of its state; it reads at the page width instead.
        if (mode === 'column' && !active()) host.zoom.applyFit('fit-width');
      }
    )
  );
  // Turning the view turns the columns sideways.
  createEffect(
    on(
      () => vp.state.viewRotation,
      () => exit(true),
      { defer: true }
    )
  );
  onCleanup(() => settleCancel?.());

  return {
    active,
    current: () => current,
    toggle,
    command: () => toggle(pointer),
    setPointer(at) {
      pointer = at;
    },
    doubleClick,
    exit,
    wheel,
    key,
    afterJump,
    refit,
    scrolled,
    setPointerDown,
    dispose() {
      seq++;
      settleCancel?.();
    },
  };
}

// ---------- registry ----------

const modes = new Map<string, ColumnMode>();

/** The column zoom of a tab's mounted viewer, for commands and Escape. */
export function columnModeFor(tabId: string | undefined | null): ColumnMode | null {
  return tabId ? (modes.get(tabId) ?? null) : null;
}

export function registerColumnMode(tabId: string, mode: ColumnMode): () => void {
  modes.set(tabId, mode);
  return () => {
    if (modes.get(tabId) === mode) modes.delete(tabId);
  };
}

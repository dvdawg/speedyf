/** Continuous-scroll virtualized viewer. Mounts PageView components only for
 * the visible range ± one viewport of overscan; everything else is empty
 * scroll space (the canvas div carries the full layout height).
 *
 * The page-rendering content (ViewerContent) only mounts once the document
 * is loaded — mirroring Sidebar's pattern — so its geoms/layout memos are
 * created fresh against real data instead of having to reactively transition
 * from an empty pages array to a populated one. */
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  useContext,
} from 'solid-js';
import { VIEW_PADDING } from '../../stores/viewportStore';
import type { AnchorPoint } from '../../lib/coordinates/coords';
import { bucketForScale, wheelZoomFactor } from '../../lib/coordinates/coords';
import { pageIndexAt, visibleRange } from '../../lib/coordinates/layout';
import PageView from './PageView';
import { openFromDialog } from '../document/tabsController';
import { IconOpen } from '../../components/icons';
import { engine } from '../../lib/transport/engine';
import { TabContext } from '../../app/TabContext';
import type { TabRecord } from '../../stores/tabsStore';

/** WebKit's non-standard pinch event. Not in lib.dom, and deliberately typed
 * as only the fields used here — `scale` is cumulative since gesturestart. */
interface WebKitGestureEvent extends UIEvent {
  readonly scale: number;
  readonly clientX: number;
  readonly clientY: number;
}

function ViewerContent(props: { tab: TabRecord }) {
  const { documentStore, viewport: vp, zoom } = props.tab;
  let scroller!: HTMLDivElement;
  const doc = documentStore.state;

  const geoms = createMemo(() => zoom.pagesGeom());
  const layout = createMemo(() => zoom.layoutFor(vp.state.zoom, geoms()));
  const range = createMemo(() =>
    visibleRange(layout(), vp.state.scrollTop, vp.state.containerH, vp.state.containerH)
  );
  const indices = createMemo(() => {
    const [first, last] = range();
    return Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => first + k);
  });

  // Quantized render scale, debounced so zoom gestures reuse the existing
  // raster (CSS-scaled) and only request one final bucketed re-render.
  const [renderScaleMilli, setRenderScaleMilli] = createSignal(
    Math.round(bucketForScale(vp.state.zoom * vp.state.dpr) * 1000)
  );
  let lastScaleApply = 0;
  let scaleRequestSeq = 0;
  const applyRenderScale = (target: number) => {
    const seq = ++scaleRequestSeq;
    const docId = doc.docId;
    // Mark queued work for the previous scale stale before minting URLs for
    // the new bucket. A sequence guard prevents out-of-order invoke replies
    // from rolling the UI back during rapid zoom gestures.
    void engine
      .bumpGeneration(docId)
      .then((generation) => {
        if (seq !== scaleRequestSeq || doc.docId !== docId) return;
        documentStore.setGeneration(generation);
        setRenderScaleMilli(target);
      })
      .catch(() => {
        if (seq === scaleRequestSeq && doc.docId === docId) setRenderScaleMilli(target);
      });
  };
  createEffect(() => {
    const target = Math.round(bucketForScale(vp.state.zoom * vp.state.dpr) * 1000);
    if (target === renderScaleMilli()) return;
    const now = Date.now();
    if (now - lastScaleApply > 400) {
      lastScaleApply = now;
      applyRenderScale(target);
      return;
    }
    const t = setTimeout(() => {
      lastScaleApply = Date.now();
      applyRenderScale(target);
    }, 160);
    onCleanup(() => clearTimeout(t));
  });

  // A page change means an expensive render for the page left behind is no
  // longer useful. Bump after a short scroll debounce; PDFium's progressive
  // pause callback observes the generation and aborts in-flight stale work.
  let previousPage = -1;
  createEffect(() => {
    const currentPage = vp.state.currentPage;
    if (previousPage < 0 || currentPage === previousPage) {
      previousPage = currentPage;
      return;
    }
    previousPage = currentPage;
    const timer = setTimeout(() => applyRenderScale(renderScaleMilli()), 80);
    onCleanup(() => clearTimeout(timer));
  });
  onCleanup(() => {
    scaleRequestSeq += 1;
  });

  let scrollRaf = 0;
  const onScroll = () => {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      const top = scroller.scrollTop;
      const current = pageIndexAt(layout(), top + vp.state.containerH * 0.4);
      vp.setState({ scrollTop: top, scrollLeft: scroller.scrollLeft, currentPage: current });
    });
  };

  // --- pinch / ctrl-wheel zoom -------------------------------------------
  //
  // Two input sources, because the engines disagree about how a trackpad
  // pinch is reported: WebKit (which is what Tauri embeds on macOS) raises
  // the non-standard gesturestart/gesturechange pair carrying an absolute
  // `scale`, while Chromium and Gecko synthesise wheel events with ctrlKey
  // set. Both are handled, and a gesture in flight suppresses the wheel path
  // so an engine that emits both cannot apply the zoom twice.
  //
  // Every source funnels into one target applied once per animation frame:
  // a 120Hz trackpad delivers events faster than the compositor can paint,
  // and re-laying-out per event is wasted work that only adds latency.

  // The scroller's viewport origin, needed to turn a client point into an
  // anchor. Cached because reading it per event forces a synchronous layout
  // in the middle of a gesture; it only moves when the window/panels resize.
  let viewportLeft = 0;
  let viewportTop = 0;
  const refreshViewportOrigin = () => {
    const r = scroller.getBoundingClientRect();
    viewportLeft = r.left;
    viewportTop = r.top;
  };

  let zoomRaf = 0;
  let pendingZoom = vp.state.zoom;
  let pendingAnchor: AnchorPoint = { x: 0, y: 0 };
  const applyPendingZoom = () => {
    if (zoomRaf) return;
    zoomRaf = requestAnimationFrame(() => {
      zoomRaf = 0;
      zoom.setZoomAnchored(pendingZoom, pendingAnchor);
    });
  };
  const anchorFrom = (clientX: number, clientY: number): AnchorPoint => ({
    x: clientX - viewportLeft,
    y: clientY - viewportTop,
  });

  // WebKit's GestureEvent: `scale` is cumulative from gesturestart, so the
  // target is always start-zoom × scale. That is inherently drift-free — no
  // accumulation of per-event factors — and it is why this path is preferred
  // when the engine offers it.
  let gestureStartZoom = 1;
  let gestureAnchor: AnchorPoint = { x: 0, y: 0 };
  // Timestamp rather than a plain "in progress" flag: a gesture cut short
  // (window blur, an interrupted trackpad) can swallow gestureend, and a
  // stuck flag would silently kill ctrl-wheel zoom for the rest of the
  // session. Suppression that expires on its own cannot.
  let lastGestureAt = 0;
  const GESTURE_LOCKOUT_MS = 200;
  const gestureInProgress = () => performance.now() - lastGestureAt < GESTURE_LOCKOUT_MS;

  const onWheel = (e: WheelEvent) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    // Always swallow it, even when the gesture path owns the zoom: otherwise
    // the engine applies its own magnification to the whole app chrome.
    e.preventDefault();
    if (gestureInProgress()) return;
    // Re-read the accumulator from the store whenever a new frame begins, so
    // it picks up clamping at the zoom limits instead of running away.
    if (!zoomRaf) pendingZoom = vp.state.zoom;
    pendingZoom *= wheelZoomFactor(e.deltaY, e.deltaMode, vp.state.containerH);
    pendingAnchor = anchorFrom(e.clientX, e.clientY);
    applyPendingZoom();
  };

  const onGestureStart = (e: WebKitGestureEvent) => {
    e.preventDefault();
    lastGestureAt = performance.now();
    gestureStartZoom = vp.state.zoom;
    refreshViewportOrigin();
    gestureAnchor = anchorFrom(e.clientX, e.clientY);
  };
  const onGestureChange = (e: WebKitGestureEvent) => {
    e.preventDefault();
    lastGestureAt = performance.now();
    pendingZoom = gestureStartZoom * e.scale;
    // Anchor on where the pinch started, not where the fingers are now: the
    // centroid wanders by a few px during a pinch, and chasing it makes the
    // page creep even though each individual step is correctly anchored.
    pendingAnchor = gestureAnchor;
    applyPendingZoom();
  };
  const onGestureEnd = (e: WebKitGestureEvent) => {
    e.preventDefault();
    lastGestureAt = 0;
  };

  onMount(() => {
    zoom.registerScroller(scroller);
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (!r) return;
      vp.setState({ containerW: r.width, containerH: r.height });
      // Opening a panel or resizing the window moves the scroller's origin,
      // which the cached anchor basis depends on.
      refreshViewportOrigin();
      zoom.refreshFit();
    });
    ro.observe(scroller);
    refreshViewportOrigin();
    scroller.addEventListener('wheel', onWheel, { passive: false });
    // Non-standard and WebKit-only; absent elsewhere, where the wheel path
    // covers pinch instead. Registered non-passive so preventDefault can stop
    // the engine from magnifying the whole app chrome.
    scroller.addEventListener('gesturestart', onGestureStart as EventListener, { passive: false });
    scroller.addEventListener('gesturechange', onGestureChange as EventListener, {
      passive: false,
    });
    scroller.addEventListener('gestureend', onGestureEnd as EventListener, { passive: false });
    let dprMedia: MediaQueryList | null = null;
    const dprListener = () => {
      const next = window.devicePixelRatio || 1;
      vp.setState('dpr', next);
      // A resolution query only observes transitions away from the value it
      // was created for. Re-arm it after every change so 2→3→1 transitions
      // cannot leave the render scale stale.
      dprMedia?.removeEventListener?.('change', dprListener);
      dprMedia = window.matchMedia(`(resolution: ${next}dppx)`);
      dprMedia.addEventListener?.('change', dprListener);
    };
    dprListener();
    window.addEventListener('resize', dprListener);
    window.visualViewport?.addEventListener('resize', dprListener);
    onCleanup(() => {
      ro.disconnect();
      scroller.removeEventListener('wheel', onWheel);
      scroller.removeEventListener('gesturestart', onGestureStart as EventListener);
      scroller.removeEventListener('gesturechange', onGestureChange as EventListener);
      scroller.removeEventListener('gestureend', onGestureEnd as EventListener);
      dprMedia?.removeEventListener?.('change', dprListener);
      window.removeEventListener('resize', dprListener);
      window.visualViewport?.removeEventListener('resize', dprListener);
      if (scrollRaf) cancelAnimationFrame(scrollRaf);
      if (zoomRaf) cancelAnimationFrame(zoomRaf);
      zoom.registerScroller(null);
    });
  });

  // Programmatic scroll requests (page navigation, search jumps).
  createEffect(() => {
    const req = vp.state.scrollRequest;
    if (!req) return;
    if (req.kind === 'position') {
      requestAnimationFrame(() => {
        scroller.scrollTop = req.top;
        scroller.scrollLeft = req.left;
      });
      return;
    }
    const l = layout();
    const top = l.tops[req.page];
    if (top === undefined) return;
    requestAnimationFrame(() => {
      scroller.scrollTop = Math.max(0, top - VIEW_PADDING + (req.offsetCss ?? 0));
    });
  });

  return (
    <div
      class="viewer"
      ref={scroller}
      onScroll={onScroll}
      role="document"
      aria-label="Document pages"
      tabIndex={-1}
    >
      <div
        class="viewer-canvas"
        style={{ height: `${layout().totalH}px`, width: `${layout().totalW}px` }}
      >
        <For each={indices()}>
          {(i) => (
            <Show when={geoms()[i]}>
              <PageView
                index={i}
                layout={layout()}
                geom={geoms()[i]!}
                scaleMilli={renderScaleMilli()}
              />
            </Show>
          )}
        </For>
      </div>
    </div>
  );
}

export default function Viewer() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;

  return (
    <Show
      when={!tab.opening}
      fallback={
        <div class="viewer">
          <div class="empty-state">
            <div class="empty-card">
              <p>Opening…</p>
            </div>
          </div>
        </div>
      }
    >
      <Show
        when={doc.loaded}
        fallback={
          <div class="viewer">
            <div class="empty-state">
              <div class="empty-card">
                <h1>SpeedyF</h1>
                <p>Open a PDF to get started, or drop a file anywhere in this window.</p>
                <button type="button" class="primary-btn" onClick={() => void openFromDialog()}>
                  <IconOpen /> Open PDF…
                </button>
              </div>
            </div>
          </div>
        }
      >
        <ViewerContent tab={tab} />
      </Show>
    </Show>
  );
}

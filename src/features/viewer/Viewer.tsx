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
import { smartCopyForSelection } from '../../lib/text/copySelection';
import { TabContext } from '../../app/TabContext';
import { tabsStore, type TabRecord } from '../../stores/tabsStore';
import { recentStore } from '../../stores/recentStore';

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

  /** Background tabs stay mounted (see .tab-workspace in global.css) so their
   * scroller, layout and scroll position survive a switch — but they must not
   * hold page rasters. A hidden tab still has a real laid-out height, so
   * without this every open document keeps ~3 viewport-heights of full-scale
   * PNGs decoded in the webview, and re-requests them on every change. */
  const isActive = createMemo(() => tabsStore.state.activeId === props.tab.id);

  const geoms = createMemo(() => zoom.pagesGeom());
  const layout = createMemo(() => zoom.layoutFor(vp.state.zoom, geoms()));
  const range = createMemo(() =>
    visibleRange(layout(), vp.state.scrollTop, vp.state.containerH, vp.state.containerH)
  );
  const indices = createMemo(() => {
    if (!isActive()) return [];
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
  // longer useful. Cancel it after a short scroll debounce; PDFium's
  // progressive pause callback observes the cancel stamp and aborts in-flight
  // work. Deliberately NOT a generation bump: the scale hasn't changed, so
  // every cached raster and minted URL is still correct, and invalidating
  // them would make each page turn re-fetch and re-decode the whole visible
  // document — the expensive thing this is trying to avoid.
  let previousPage = -1;
  createEffect(() => {
    const currentPage = vp.state.currentPage;
    if (previousPage < 0 || currentPage === previousPage) {
      previousPage = currentPage;
      return;
    }
    previousPage = currentPage;
    const docId = doc.docId;
    const timer = setTimeout(() => {
      if (doc.docId === docId) void engine.cancelRenders(docId);
    }, 80);
    onCleanup(() => clearTimeout(timer));
  });
  onCleanup(() => {
    scaleRequestSeq += 1;
  });

  let scrollRaf = 0;
  let scrollRequestRaf = 0;
  const onScroll = () => {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      const top = scroller.scrollTop;
      const current = pageIndexAt(layout(), top + vp.state.containerH * 0.4);
      vp.setState({ scrollTop: top, scrollLeft: scroller.scrollLeft, currentPage: current });
      rememberPosition();
    });
  };

  // Publish where a text selection sits, in the same page space anchors use,
  // so the context header can name what was highlighted. Computed from the
  // range's client rect rather than the DOM: the text layer is one span per
  // run, with nothing on it that identifies a page.
  const onSelectionChange = () => {
    const selection = document.getSelection();
    if (!selection || selection.isCollapsed || selection.rangeCount === 0) {
      if (vp.state.selectionAnchor) vp.setState('selectionAnchor', null);
      return;
    }
    const range = selection.getRangeAt(0);
    if (!scroller.contains(range.commonAncestorContainer)) return;
    const rect = range.getBoundingClientRect();
    if (rect.height === 0 && rect.width === 0) return;
    const contentY = rect.top - scroller.getBoundingClientRect().top + scroller.scrollTop;
    const l = layout();
    const page = pageIndexAt(l, contentY);
    const geom = documentStore.state.pages[page];
    const pageTop = l.tops[page];
    if (!geom || pageTop === undefined) return;
    const rotation = (geom.baseRotation + geom.userRotation + vp.state.viewRotation) % 360;
    const heightPt = rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
    vp.setState('selectionAnchor', {
      page,
      y: heightPt - (contentY - pageTop) / vp.state.zoom,
    });
  };
  // A PDF selection copies as one fragment per positioned run: broken lines,
  // ligature glyphs, and words split at line ends. Rebuild it from the run
  // geometry instead, and only for selections that are actually in this
  // document's pages.
  const onCopy = (event: ClipboardEvent) => {
    const selection = document.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    if (!scroller.contains(selection.getRangeAt(0).commonAncestorContainer)) return;
    const text = smartCopyForSelection(selection, scroller);
    if (text === null || !event.clipboardData) return;
    event.preventDefault();
    event.clipboardData.setData('text/plain', text);
  };

  onMount(() => {
    document.addEventListener('selectionchange', onSelectionChange);
    document.addEventListener('copy', onCopy);
    onCleanup(() => {
      document.removeEventListener('selectionchange', onSelectionChange);
      document.removeEventListener('copy', onCopy);
    });
  });

  // Persist where reading stopped, debounced: this writes to localStorage, so
  // it must not run on every scroll frame.
  let rememberTimer: ReturnType<typeof setTimeout> | undefined;
  const rememberPosition = () => {
    clearTimeout(rememberTimer);
    rememberTimer = setTimeout(() => {
      const path = documentStore.state.path;
      if (!path) return;
      const l = layout();
      const page = vp.state.currentPage;
      const top = l.tops[page];
      const height = l.heights[page];
      if (top === undefined || !height) return;
      const fraction = Math.min(1, Math.max(0, (scroller.scrollTop - top + VIEW_PADDING) / height));
      recentStore.recordPosition(path, { page, fraction });
    }, 500);
  };
  onCleanup(() => clearTimeout(rememberTimer));

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
      if (scrollRequestRaf) cancelAnimationFrame(scrollRequestRaf);
      if (zoomRaf) cancelAnimationFrame(zoomRaf);
      zoom.registerScroller(null);
    });
  });

  // Programmatic scroll requests (page navigation, search jumps).
  createEffect(() => {
    // These are commands, not durable viewport state. Leaving the last page
    // request in the store made this effect subscribe to layout(); the first
    // zoom update then replayed that old request (the initial one targets page
    // 0), overriding the zoom controller's anchored scroll correction.
    const req = vp.takeScrollRequest();
    if (!req) return;

    if (scrollRequestRaf) cancelAnimationFrame(scrollRequestRaf);

    const apply = () => {
      scrollRequestRaf = 0;
      if (req.kind === 'position') {
        scroller.scrollTop = req.top;
        scroller.scrollLeft = req.left;
        return;
      }

      // Resolve against the layout in effect when the frame runs. A zoom or
      // resize between the request and this callback must not use stale page
      // coordinates.
      const l = layout();
      const top = l.tops[req.page];
      const height = l.heights[req.page];
      if (top === undefined || height === undefined) return;
      scroller.scrollTop = Math.max(
        0,
        top - VIEW_PADDING + (req.fraction ?? 0) * height + (req.offsetCss ?? 0)
      );
    };

    // A restored reading position has to outlast the initial fit: the
    // ResizeObserver that applies fit-width runs *after* rAF callbacks, so a
    // single frame's delay would place the view at the pre-fit zoom and let the
    // fit re-anchor it somewhere else. A second frame lands after the zoom has
    // settled, and `apply` re-reads the layout there so the fraction resolves
    // against the real page.
    scrollRequestRaf = requestAnimationFrame(
      req.kind === 'page' && req.settle
        ? () => {
            scrollRequestRaf = requestAnimationFrame(apply);
          }
        : apply
    );
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

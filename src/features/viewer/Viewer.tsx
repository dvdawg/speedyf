/** Continuous-scroll virtualized viewer. Mounts PageView components only for
 * the visible range ± one viewport of overscan; everything else is empty
 * scroll space (the canvas div carries the full layout height). */
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { documentStore } from '../document/documentStore';
import { setViewport, viewport, VIEW_PADDING } from '../../stores/viewportStore';
import {
  layoutFor,
  pagesGeom,
  refreshFit,
  registerScroller,
  setZoomAnchored,
} from './zoomController';
import { bucketForScale } from '../../lib/coordinates/coords';
import { pageIndexAt, visibleRange } from '../../lib/coordinates/layout';
import PageView from './PageView';
import { openFromDialog } from '../document/controller';
import { IconOpen } from '../../components/icons';
import { engine } from '../../lib/transport/engine';

export default function Viewer() {
  let scroller!: HTMLDivElement;
  const doc = documentStore.state;

  const geoms = createMemo(pagesGeom);
  const layout = createMemo(() => layoutFor(viewport.zoom));
  const range = createMemo(() =>
    visibleRange(layout(), viewport.scrollTop, viewport.containerH, viewport.containerH)
  );
  const indices = createMemo(() => {
    const [first, last] = range();
    return Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => first + k);
  });

  // Quantized render scale, debounced so zoom gestures reuse the existing
  // raster (CSS-scaled) and only request one final bucketed re-render.
  const [renderScaleMilli, setRenderScaleMilli] = createSignal(
    Math.round(bucketForScale(viewport.zoom * viewport.dpr) * 1000)
  );
  let lastScaleApply = 0;
  let scaleRequestSeq = 0;
  const applyRenderScale = (target: number) => {
    const seq = ++scaleRequestSeq;
    const docId = doc.docId;
    if (!doc.loaded) {
      setRenderScaleMilli(target);
      return;
    }
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
    const target = Math.round(bucketForScale(viewport.zoom * viewport.dpr) * 1000);
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
  onCleanup(() => {
    scaleRequestSeq += 1;
  });

  let scrollRaf = 0;
  const onScroll = () => {
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      const top = scroller.scrollTop;
      const current = pageIndexAt(layout(), top + viewport.containerH * 0.4);
      setViewport({ scrollTop: top, currentPage: current });
    });
  };

  const onWheel = (e: WheelEvent) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.preventDefault();
    if (!doc.loaded) return;
    const rect = scroller.getBoundingClientRect();
    const factor = Math.exp(-e.deltaY * 0.0022);
    setZoomAnchored(viewport.zoom * factor, e.clientY - rect.top);
  };

  onMount(() => {
    registerScroller(scroller);
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (!r) return;
      setViewport({ containerW: r.width, containerH: r.height });
      refreshFit();
    });
    ro.observe(scroller);
    scroller.addEventListener('wheel', onWheel, { passive: false });
    const dprListener = () => setViewport('dpr', window.devicePixelRatio || 1);
    const mq = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    mq.addEventListener?.('change', dprListener);
    onCleanup(() => {
      ro.disconnect();
      scroller.removeEventListener('wheel', onWheel);
      mq.removeEventListener?.('change', dprListener);
      registerScroller(null);
    });
  });

  // Programmatic scroll requests (page navigation, search jumps).
  createEffect(() => {
    const req = viewport.scrollRequest;
    if (!req || !doc.loaded) return;
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
      <Show
        when={doc.loaded}
        fallback={
          <div class="empty-state">
            <div class="empty-card">
              <h1>SpeedyF</h1>
              <p>Open a PDF to get started, or drop a file anywhere in this window.</p>
              <button type="button" class="primary-btn" onClick={() => void openFromDialog()}>
                <IconOpen /> Open PDF…
              </button>
            </div>
          </div>
        }
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
      </Show>
    </div>
  );
}

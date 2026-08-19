/** Thumbnail list: virtualized (only near-viewport thumbs mount an <img>),
 * current-page tracking, click navigation, and edit-mode page operations
 * (drag reorder, rotate, duplicate, delete, add blank page). Auto-scrolls to
 * keep the current page's thumbnail in view as the main viewer scrolls. */
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
import { renderUrl } from '../../lib/rendering/renderSource';
import type { Layout } from '../../lib/coordinates/layout';
import { visibleRange } from '../../lib/coordinates/layout';
import {
  cornerDragScale,
  pageViewportFraction,
  scrollForPageFraction,
  viewportOriginFraction,
} from '../../lib/coordinates/viewportIndicator';
import IconButton from '../../components/IconButton';
import {
  IconChevronDown,
  IconChevronUp,
  IconDuplicate,
  IconPlus,
  IconRotate,
  IconTrash,
} from '../../components/icons';
import { addBlankPageAfter } from '../editor/editorActions';
import type { Rotation } from '../../types/model';
import { TabContext } from '../../app/TabContext';

const THUMB_W = 148;
const SLOT_PAD = 30; // label + spacing
const GAP = 10;

export default function PageThumbnails() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp, zoom } = tab;
  const requestScrollToPage = vp.requestScrollToPage;
  const doc = documentStore.state;
  let scroller!: HTMLDivElement;
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewH, setViewH] = createSignal(600);
  const [dragIndex, setDragIndex] = createSignal<number | null>(null);
  const [dropIndex, setDropIndex] = createSignal<number | null>(null);
  const [panPage, setPanPage] = createSignal<number | null>(null);
  const [zoomPage, setZoomPage] = createSignal<number | null>(null);

  onMount(() => {
    setViewH(scroller.clientHeight || 600);
    const ro = new ResizeObserver((es) => {
      const h = es[0]?.contentRect.height;
      if (h) setViewH(h);
    });
    ro.observe(scroller);
    onCleanup(() => ro.disconnect());
  });

  const geom = createMemo(() =>
    doc.pages.map((p) => {
      const rot = (p.baseRotation + p.userRotation + vp.state.viewRotation) % 360;
      const [w, h] = rot % 180 === 0 ? [p.widthPt, p.heightPt] : [p.heightPt, p.widthPt];
      return { thumbH: (h / w) * THUMB_W, rot: rot as Rotation, srcIndex: p.srcIndex, id: p.id };
    })
  );

  const layout = createMemo<Layout>(() => {
    const tops: number[] = [];
    const heights: number[] = [];
    let y = 12;
    for (const g of geom()) {
      tops.push(y);
      const slotH = g.thumbH + SLOT_PAD;
      heights.push(slotH);
      y += slotH + GAP;
    }
    return {
      tops,
      heights,
      lefts: [],
      widths: [],
      totalH: y + 12,
      totalW: THUMB_W,
    };
  });

  const range = createMemo(() => visibleRange(layout(), scrollTop(), viewH(), viewH()));
  const indices = createMemo(() => {
    const [a, b] = range();
    return Array.from({ length: Math.max(0, b - a + 1) }, (_, k) => a + k);
  });

  // The main viewer's page layout, so each thumbnail can show which slice of
  // its page is on screen. Memoized on zoom / rotation / container width and
  // deliberately NOT on scrollTop: recomputing an O(pages) layout on every
  // scroll frame would make the sidebar the bottleneck on long documents.
  const mainLayout = createMemo(() => zoom.layoutFor(vp.state.zoom));

  // Follow the main viewer, but continuously rather than in page-sized steps.
  //
  // Keying this off `currentPage` alone meant nothing moved while you scrolled
  // through a page and then the sidebar snapped a whole thumbnail the instant
  // the index ticked over. Instead we track how far through the current page
  // the main view actually is, project that onto the sidebar's own layout, and
  // ease toward it — so the sidebar glides as you scroll and never lurches on
  // arrival at a page boundary.
  //
  // The deadzone is what keeps this from fighting you: while the tracked point
  // sits in the middle half of the sidebar nothing moves at all, so scrolling
  // the sidebar by hand is left alone.
  let followRaf = 0;
  let followTarget: number | null = null;
  const stepFollow = () => {
    followRaf = 0;
    if (followTarget === null) return;
    const delta = followTarget - scroller.scrollTop;
    if (Math.abs(delta) < 0.5) {
      scroller.scrollTop = followTarget;
      followTarget = null;
      return;
    }
    scroller.scrollTop += delta * 0.18;
    followRaf = requestAnimationFrame(stepFollow);
  };
  onCleanup(() => {
    if (followRaf) cancelAnimationFrame(followRaf);
  });

  createEffect(() => {
    const l = layout();
    const page = vp.state.currentPage;
    const top = l.tops[page];
    const height = l.heights[page];
    if (top === undefined || height === undefined) return;
    // A drag is steering the main view from this sidebar; scrolling the
    // sidebar now would slide the thumbnail out from under the pointer.
    if (panPage() !== null || zoomPage() !== null) return;

    // How far through the current page the main viewer sits, 0..1.
    const ml = mainLayout();
    const mTop = ml.tops[page];
    const mH = ml.heights[page];
    const progress =
      mTop === undefined || !mH ? 0 : Math.min(1, Math.max(0, (vp.state.scrollTop - mTop) / mH));

    // The same point in the sidebar's layout, and the band we keep it inside.
    const pos = top + progress * height;
    const view = viewH();
    const margin = view * 0.25;
    const from = followTarget ?? scroller.scrollTop;
    let target = from;
    if (pos - from < margin) target = pos - margin;
    else if (pos - from > view - margin) target = pos - (view - margin);
    target = Math.max(0, Math.min(Math.max(0, l.totalH - view), target));
    if (Math.abs(target - scroller.scrollTop) < 0.5) return;

    followTarget = target;
    // Honour reduced-motion by landing immediately instead of easing.
    if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
      scroller.scrollTop = target;
      followTarget = null;
      return;
    }
    if (!followRaf) followRaf = requestAnimationFrame(stepFollow);
  });

  const viewOf = () => ({
    scrollTop: vp.state.scrollTop,
    scrollLeft: vp.state.scrollLeft,
    containerW: vp.state.containerW,
    containerH: vp.state.containerH,
  });

  const indicatorFor = (i: number) =>
    pageViewportFraction(mainLayout(), i, {
      scrollTop: vp.state.scrollTop,
      scrollLeft: vp.state.scrollLeft,
      containerW: vp.state.containerW,
      containerH: vp.state.containerH,
    });

  // There is one zoom, so there is one grip, and it belongs on the LAST page
  // the view touches: that is where the viewport's bottom-right corner
  // actually falls. Picking by visible area instead left the grip stranded on
  // the upper page while you scrolled into the next one.
  const gripPage = createMemo(() => {
    const l = mainLayout();
    const view = {
      scrollTop: vp.state.scrollTop,
      scrollLeft: vp.state.scrollLeft,
      containerW: vp.state.containerW,
      containerH: vp.state.containerH,
    };
    const [first, last] = visibleRange(l, vp.state.scrollTop, vp.state.containerH, 0);
    for (let i = last; i >= first; i--) {
      if (pageViewportFraction(l, i, view)) return i;
    }
    return -1;
  });

  // Both drags run their listeners on `window`, not on the box: the thumbnail
  // can unmount mid-gesture — the page scrolls out of the main view, or the
  // sidebar's virtualization drops that row — and element-bound listeners
  // would take the drag down with it, stalling the gesture halfway.
  let endDrag: (() => void) | null = null;
  onCleanup(() => endDrag?.());

  const trackDrag = (onMove: (ev: PointerEvent) => void, onDone: () => void) => {
    const move = (ev: PointerEvent) => onMove(ev);
    const finish = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      endDrag = null;
      onDone();
    };
    endDrag?.();
    endDrag = finish;
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
  };

  // Drag the box to pan the main view. The new position is always the fraction
  // the box started at plus the *total* pointer delta, never an incremental
  // step, so the scroll updates we cause feeding back into the box's position
  // cannot accumulate drift.
  const startPan = (e: PointerEvent, i: number) => {
    if (vp.state.editMode) return; // edit mode: the reorder drag owns the thumb
    // The DRAWN box is clipped to the page; its origin would read 0 whenever
    // the view starts above this page, teleporting instead of panning. Anchor
    // on the unclipped viewport origin.
    const start = viewportOriginFraction(mainLayout(), i, viewOf());
    const thumbH = geom()[i]?.thumbH;
    if (!start || !thumbH) return;
    e.preventDefault();
    e.stopPropagation();

    const x0 = e.clientX;
    const y0 = e.clientY;
    setPanPage(i);

    trackDrag(
      (ev) => {
        const pos = scrollForPageFraction(
          mainLayout(),
          i,
          start.x + (ev.clientX - x0) / THUMB_W,
          start.y + (ev.clientY - y0) / thumbH
        );
        if (pos) vp.requestScrollToPosition(pos.top, pos.left);
      },
      () => setPanPage(null)
    );
  };

  // Drag the corner grip to resize the box, which *is* a zoom change: the box
  // is the viewport measured against the page, so its size is proportional to
  // 1/zoom. A bigger box means more page on screen — zoom out — and vice versa.
  const startZoomDrag = (e: PointerEvent, i: number) => {
    if (vp.state.editMode) return;
    const start = viewportOriginFraction(mainLayout(), i, viewOf());
    const thumbH = geom()[i]?.thumbH;
    const l0 = mainLayout();
    const pageW = l0.widths[i];
    const pageH = l0.heights[i];
    if (!start || !thumbH || !pageW || !pageH) return;
    e.preventDefault();
    e.stopPropagation(); // the box below this grip pans; the grip zooms

    const x0 = e.clientX;
    const y0 = e.clientY;
    // Scale against the UNCLIPPED viewport, not the drawn box. When the view
    // straddles two pages the box on the lower one is only a sliver, and
    // scaling off a sliver turns a small drag into a huge zoom jump. The
    // viewport's full size is also what makes the drag 1:1: growing it by 10px
    // moves its bottom edge — and so the grip — by exactly 10px.
    const w0 = (vp.state.containerW / pageW) * THUMB_W;
    const h0 = (vp.state.containerH / pageH) * thumbH;
    const startZoom = vp.state.zoom;
    setZoomPage(i);

    trackDrag(
      (ev) => {
        // Anchor the viewport's top-left: a bottom-right grip must grow the
        // box away from its origin, not recentre the view under the cursor.
        // setZoomAnchored clamps to the zoom limits for us.
        const scale = cornerDragScale(w0, h0, ev.clientX - x0, ev.clientY - y0);
        zoom.setZoomAnchored(startZoom / scale, { x: 0, y: 0 });

        // Then re-pin the view to the fraction this gesture STARTED at,
        // reading the zoom back so a clamp at the limits is accounted for.
        // Without this the drag walks: setZoomAnchored anchors off the live
        // scrollTop, and zooming out near the end of a document clamps that
        // value, so every pointermove compounds the last one's error until you
        // land on page one. Deriving from the start fraction makes each move
        // absolute, so the view also returns exactly where it was if you drag
        // back the other way.
        const pos = scrollForPageFraction(zoom.layoutFor(vp.state.zoom), i, start.x, start.y);
        if (pos) vp.requestScrollToPosition(pos.top, pos.left);
      },
      () => setZoomPage(null)
    );
  };

  const thumbUrl = (i: number) => {
    const g = geom()[i];
    if (!g || g.srcIndex === null) return null;
    const p = doc.pages[i]!;
    const rotW = g.rot % 180 === 0 ? p.widthPt : p.heightPt;
    const scale = (THUMB_W * vp.state.dpr) / rotW;
    const scaleMilli = Math.max(20, Math.round((scale * 1000) / 10) * 10);
    return renderUrl({
      docId: doc.docId,
      srcIndex: g.srcIndex,
      rotation: g.rot,
      scaleMilli,
      generation: doc.generation,
      kind: 'thumb',
    });
  };

  const reorderTo = (from: number, to: number) => {
    if (from === to || from + 1 === to) return;
    const page = doc.pages[from];
    if (!page) return;
    const target = to > from ? to - 1 : to;
    documentStore.apply({ type: 'reorder', pageId: page.id, toIndex: target });
  };

  return (
    <>
      <div class="sidebar-scroll" ref={scroller} onScroll={() => setScrollTop(scroller.scrollTop)}>
        <div style={{ height: `${layout().totalH}px`, position: 'relative' }}>
          <For each={indices()}>
            {(i) => {
              const g = () => geom()[i];
              return (
                <Show when={g()}>
                  <div
                    class="thumb-slot"
                    classList={{
                      'is-current': vp.state.currentPage === i,
                      'is-drop-target': dropIndex() === i,
                    }}
                    style={{ top: `${layout().tops[i]}px`, height: `${layout().heights[i]}px` }}
                    draggable={vp.state.editMode}
                    onDragStart={(e) => {
                      setDragIndex(i);
                      e.dataTransfer!.effectAllowed = 'move';
                      e.dataTransfer!.setData('text/plain', String(i));
                    }}
                    onDragOver={(e) => {
                      if (dragIndex() === null) return;
                      e.preventDefault();
                      setDropIndex(i);
                    }}
                    onDragLeave={() => setDropIndex((d) => (d === i ? null : d))}
                    onDrop={(e) => {
                      e.preventDefault();
                      const from = dragIndex();
                      setDragIndex(null);
                      setDropIndex(null);
                      if (from !== null) reorderTo(from, i);
                    }}
                    onDragEnd={() => {
                      setDragIndex(null);
                      setDropIndex(null);
                    }}
                  >
                    <button
                      type="button"
                      class="thumb-btn"
                      style={{ height: `${g()!.thumbH}px` }}
                      onClick={() => requestScrollToPage(i)}
                      aria-label={`Go to page ${i + 1}`}
                      aria-current={vp.state.currentPage === i ? 'page' : undefined}
                    >
                      <Show
                        when={thumbUrl(i)}
                        fallback={<div class="thumb-blank" aria-hidden="true" />}
                      >
                        <img src={thumbUrl(i)!} alt="" loading="lazy" draggable={false} />
                      </Show>
                    </button>
                    <Show when={indicatorFor(i)}>
                      {(frac) => (
                        <div
                          class="thumb-viewport"
                          classList={{
                            'is-panning': panPage() === i,
                            'is-zooming': zoomPage() === i,
                            'is-static': vp.state.editMode,
                          }}
                          style={{
                            left: `${frac().x * THUMB_W}px`,
                            top: `${frac().y * g()!.thumbH}px`,
                            width: `${frac().w * THUMB_W}px`,
                            height: `${frac().h * g()!.thumbH}px`,
                          }}
                          onPointerDown={(e) => startPan(e, i)}
                          onClick={(e) => e.stopPropagation()}
                          aria-hidden="true"
                        >
                          <Show when={gripPage() === i}>
                            <span
                              class="thumb-viewport-grip"
                              onPointerDown={(e) => startZoomDrag(e, i)}
                            />
                          </Show>
                        </div>
                      )}
                    </Show>
                    <div class="thumb-meta">
                      <span class="thumb-num">{i + 1}</span>
                      <Show when={vp.state.editMode}>
                        <span class="thumb-ops">
                          <IconButton
                            label={`Move page ${i + 1} up`}
                            disabled={i === 0}
                            onClick={() =>
                              documentStore.apply({
                                type: 'reorder',
                                pageId: doc.pages[i]!.id,
                                toIndex: i - 1,
                              })
                            }
                          >
                            <IconChevronUp />
                          </IconButton>
                          <IconButton
                            label={`Move page ${i + 1} down`}
                            disabled={i >= doc.pages.length - 1}
                            onClick={() =>
                              documentStore.apply({
                                type: 'reorder',
                                pageId: doc.pages[i]!.id,
                                toIndex: i + 1,
                              })
                            }
                          >
                            <IconChevronDown />
                          </IconButton>
                          <IconButton
                            label={`Rotate page ${i + 1} 90°`}
                            onClick={() =>
                              documentStore.apply({
                                type: 'rotate',
                                pageId: doc.pages[i]!.id,
                                delta: 90,
                              })
                            }
                          >
                            <IconRotate />
                          </IconButton>
                          <IconButton
                            label={`Duplicate page ${i + 1}`}
                            onClick={() =>
                              documentStore.apply({ type: 'duplicate', pageId: doc.pages[i]!.id })
                            }
                          >
                            <IconDuplicate />
                          </IconButton>
                          <IconButton
                            label={`Delete page ${i + 1}`}
                            disabled={doc.pages.length <= 1}
                            onClick={() =>
                              documentStore.apply({ type: 'delete', pageId: doc.pages[i]!.id })
                            }
                          >
                            <IconTrash />
                          </IconButton>
                        </span>
                      </Show>
                    </div>
                  </div>
                </Show>
              );
            }}
          </For>
        </div>
      </div>
      <Show when={vp.state.editMode}>
        <div class="sidebar-footer">
          <button
            type="button"
            class="secondary-btn"
            onClick={() => addBlankPageAfter(documentStore, vp.state.currentPage)}
          >
            <IconPlus /> Blank page
          </button>
        </div>
      </Show>
    </>
  );
}

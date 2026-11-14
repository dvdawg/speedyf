/** The crosshair over the reader while a region is being picked: drag a
 * rectangle on a page, Escape cancels. Pointer events, not drag and drop
 * (see projects/pointerDrag.ts for why). */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { activeTab } from '../../stores/tabsStore';
import { captureRequest, regionFromDrag, type CaptureRequest } from './regionCapture';

interface Drag {
  pageEl: HTMLElement;
  page: number;
  startX: number;
  startY: number;
  x: number;
  y: number;
}

export default function RegionCaptureLayer() {
  return <Show when={captureRequest()}>{(request) => <Overlay request={request()} />}</Show>;
}

function pageAt(clientX: number, clientY: number): { el: HTMLElement; index: number } | null {
  for (const el of document.elementsFromPoint(clientX, clientY)) {
    const page = (el as HTMLElement).closest?.('.page[data-page-index]') as HTMLElement | null;
    if (page) return { el: page, index: Number(page.dataset.pageIndex) };
  }
  return null;
}

function Overlay(props: { request: CaptureRequest }) {
  const [drag, setDrag] = createSignal<Drag | null>(null);
  let layer!: HTMLDivElement;

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      props.request.finish(null);
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  const down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    // The overlay sits above the pages; look through it for the page.
    layer.style.pointerEvents = 'none';
    const hit = pageAt(e.clientX, e.clientY);
    layer.style.pointerEvents = '';
    if (!hit) return;
    e.preventDefault();
    layer.setPointerCapture?.(e.pointerId);
    setDrag({
      pageEl: hit.el,
      page: hit.index,
      startX: e.clientX,
      startY: e.clientY,
      x: e.clientX,
      y: e.clientY,
    });
  };
  const move = (e: PointerEvent) => {
    const current = drag();
    if (current) setDrag({ ...current, x: e.clientX, y: e.clientY });
  };
  const up = () => {
    const current = drag();
    setDrag(null);
    if (!current) return;
    const tab = activeTab();
    const geom = tab?.zoom.pagesGeom()[current.page];
    if (!geom) return;
    const rect = current.pageEl.getBoundingClientRect();
    const widthPt = geom.rotation % 180 === 0 ? geom.widthPt : geom.heightPt;
    const heightPt = geom.rotation % 180 === 0 ? geom.heightPt : geom.widthPt;
    const box = regionFromDrag(
      { x: current.startX - rect.left, y: current.startY - rect.top },
      { x: current.x - rect.left, y: current.y - rect.top },
      rect.width / widthPt,
      { widthPt, heightPt }
    );
    if (box) props.request.finish({ page: current.page, box });
  };

  const box = () => {
    const d = drag();
    if (!d) return null;
    return {
      left: `${Math.min(d.startX, d.x)}px`,
      top: `${Math.min(d.startY, d.y)}px`,
      width: `${Math.abs(d.x - d.startX)}px`,
      height: `${Math.abs(d.y - d.startY)}px`,
    };
  };

  return (
    <div
      ref={layer}
      class="region-capture"
      role="application"
      aria-label={props.request.prompt}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={() => setDrag(null)}
    >
      <div class="region-capture-hint" role="status" onPointerDown={(e) => e.stopPropagation()}>
        {props.request.prompt}
        <button type="button" class="paper-mini-btn" onClick={() => props.request.finish(null)}>
          Cancel
        </button>
      </div>
      <Show when={box()}>{(style) => <div class="region-capture-box" style={style()} />}</Show>
    </div>
  );
}

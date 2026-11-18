/** Prepare form mode on one page: every field as a box to select, move,
 * resize and delete; the tool in hand draws new ones (a drag, or a click
 * for the default size), snapping to the printed blanks; auto-detect's
 * proposals wait here to be accepted or rejected; and with "Show tab order"
 * each field carries its number.
 *
 * Pointer events throughout (HTML5 drag and drop does not work in the app's
 * webview). Moves and resizes are previewed locally and recorded as one
 * edit when the pointer is released. */
import { createEffect, createMemo, createSignal, For, Show, useContext } from 'solid-js';
import type { PageId, PdfPoint, PdfRect } from '../../types/model';
import type { PageGeom } from '../../lib/coordinates/coords';
import { cssToPdf } from '../../lib/coordinates/coords';
import { TabContext } from '../../app/TabContext';
import { blanksOf, loadBlanks } from '../sign/fillStore';
import { userDatePicture } from '../forms/formBehaviour';
import {
  addFields,
  blankAtPoint,
  blankUnder,
  defaultSize,
  newField,
  patchFields,
  snapToEdges,
  tabOrderOf,
  type DesignField,
} from './model';
import {
  acceptProposals,
  commitDesign,
  designOf,
  prepareState,
  preparing,
  rejectProposals,
  select,
  setTool,
} from './prepareStore';

interface Props {
  pageId: PageId;
  src: number;
  geom: PageGeom;
  zoom: number;
}

type Handle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';
const HANDLES: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

/** A drag in progress, previewed until release. */
type Drag =
  | { kind: 'move'; ids: string[]; start: PdfPoint; dx: number; dy: number; moved: boolean }
  | { kind: 'resize'; id: string; handle: Handle; start: PdfPoint; rect: PdfRect }
  | { kind: 'draw'; start: PdfPoint; end: PdfPoint };

/** Smallest box a drag makes, in points. */
const MIN_SIDE = 6;
/** Less pointer travel than this (CSS px) is a click. */
const CLICK_SLOP = 3;

export default function PrepareLayer(props: Props) {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;
  const on = () => preparing(tab);
  const design = () => designOf(tab);
  const fields = createMemo(() => design().fields.filter((f) => f.pageId === props.pageId));
  const proposals = () => prepareState.proposals.filter((p) => p.pageId === props.pageId);
  const order = createMemo(() =>
    prepareState.showTabs ? tabOrderOf(design(), props.pageId) : ([] as string[])
  );
  const blanks = () => blanksOf(doc.docId, props.src) ?? [];
  /** Widgets from the file that moved or went: their old place is covered,
   * since the raster still draws them there. */
  const covers = createMemo(() => {
    const out: PdfRect[] = [];
    const d = design();
    for (const f of d.fields) {
      if (!f.origin || f.origin.src !== props.src) continue;
      const b = f.origin.base.rect;
      if (
        f.pageId !== f.origin.pageId ||
        Math.abs(f.rect.x - b.x) > 0.01 ||
        Math.abs(f.rect.y - b.y) > 0.01 ||
        Math.abs(f.rect.w - b.w) > 0.01 ||
        Math.abs(f.rect.h - b.h) > 0.01
      ) {
        out.push(b);
      }
    }
    for (const r of d.deleted) if (r.src === props.src) out.push(r.rect);
    return out;
  });

  createEffect(() => {
    if (on()) void loadBlanks(doc.docId, props.src);
  });

  const [drag, setDrag] = createSignal<Drag | null>(null);
  let layerEl!: HTMLDivElement;
  let downAt: { x: number; y: number } | null = null;

  // Optional: not every DOM (tests) implements pointer capture.
  const capture = (e: PointerEvent) => layerEl.setPointerCapture?.(e.pointerId);
  const release = (e: PointerEvent) => {
    if (layerEl.hasPointerCapture?.(e.pointerId)) layerEl.releasePointerCapture(e.pointerId);
  };
  const H = () => props.geom.heightPt;
  const layerCssW = () =>
    (props.geom.rotation % 180 === 0 ? props.geom.widthPt : props.geom.heightPt) * props.zoom;
  const toPdf = (e: PointerEvent): PdfPoint => {
    const r = layerEl.getBoundingClientRect();
    return cssToPdf(
      { x: e.clientX - r.left, y: e.clientY - r.top },
      props.geom,
      props.zoom * (r.width / Math.max(1, layerCssW()))
    );
  };
  const box = (r: PdfRect) => ({
    left: `${r.x * props.zoom}px`,
    top: `${(H() - r.y - r.h) * props.zoom}px`,
    width: `${r.w * props.zoom}px`,
    height: `${r.h * props.zoom}px`,
  });

  /** Where a field is drawn now: its rect, or the drag's preview of it. */
  const shown = (f: DesignField): PdfRect => {
    const d = drag();
    if (d?.kind === 'move' && d.ids.includes(f.id)) {
      return { ...f.rect, x: f.rect.x + d.dx, y: f.rect.y + d.dy };
    }
    if (d?.kind === 'resize' && d.id === f.id) return d.rect;
    return f.rect;
  };

  const isSelected = (id: string) => prepareState.selected.includes(id);

  const onFieldDown = (e: PointerEvent, f: DesignField) => {
    if (e.button !== 0 || prepareState.tool !== 'select') return;
    e.stopPropagation();
    e.preventDefault();
    if (e.shiftKey || e.metaKey || e.ctrlKey) {
      select(
        isSelected(f.id)
          ? prepareState.selected.filter((x) => x !== f.id)
          : [...prepareState.selected, f.id]
      );
      return;
    }
    if (!isSelected(f.id)) select([f.id]);
    capture(e);
    downAt = { x: e.clientX, y: e.clientY };
    setDrag({
      kind: 'move',
      ids: [...prepareState.selected],
      start: toPdf(e),
      dx: 0,
      dy: 0,
      moved: false,
    });
  };

  const onHandleDown = (e: PointerEvent, f: DesignField, handle: Handle) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    select([f.id]);
    capture(e);
    setDrag({ kind: 'resize', id: f.id, handle, start: toPdf(e), rect: { ...f.rect } });
  };

  const onLayerDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    if (prepareState.tool === 'select') {
      select([]);
      return;
    }
    capture(e);
    downAt = { x: e.clientX, y: e.clientY };
    const p = toPdf(e);
    setDrag({ kind: 'draw', start: p, end: p });
  };

  const onMove = (e: PointerEvent) => {
    const d = drag();
    if (!d) return;
    const p = toPdf(e);
    if (d.kind === 'move') {
      const travelled =
        downAt && Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > CLICK_SLOP;
      if (!d.moved && !travelled) return;
      let dx = p.x - d.start.x;
      let dy = p.y - d.start.y;
      // Snap the first field's edges to a blank's.
      const lead = fields().find((f) => f.id === d.ids[0]);
      if (lead && !e.altKey) {
        const moved = { ...lead.rect, x: lead.rect.x + dx, y: lead.rect.y + dy };
        const snapped = snapToEdges(moved, blanks());
        dx += snapped.x - moved.x;
        dy += snapped.y - moved.y;
      }
      setDrag({ ...d, dx, dy, moved: true });
    } else if (d.kind === 'resize') {
      const f = fields().find((x) => x.id === d.id);
      if (!f) return;
      setDrag({ ...d, rect: resized(f.rect, d.handle, p.x - d.start.x, p.y - d.start.y) });
    } else {
      setDrag({ ...d, end: p });
    }
  };

  const onUp = (e: PointerEvent) => {
    const d = drag();
    setDrag(null);
    release(e);
    if (!d) return;
    if (d.kind === 'move') {
      if (d.moved && (Math.abs(d.dx) > 0.01 || Math.abs(d.dy) > 0.01)) {
        commitDesign(
          tab,
          patchFields(design(), d.ids, (f) => ({
            rect: { ...f.rect, x: f.rect.x + d.dx, y: f.rect.y + d.dy },
          }))
        );
      }
    } else if (d.kind === 'resize') {
      commitDesign(
        tab,
        patchFields(design(), [d.id], () => ({ rect: d.rect }))
      );
    } else {
      place(d, e);
    }
    downAt = null;
  };

  /** Make a field from a drawing drag or a click. */
  const place = (d: Extract<Drag, { kind: 'draw' }>, e: PointerEvent) => {
    const tool = prepareState.tool;
    if (tool === 'select') return;
    const click = !downAt || Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) <= CLICK_SLOP;
    const button = tool === 'checkbox' || tool === 'radio';
    let rect: PdfRect;
    if (click) {
      const blank = blankAtPoint(d.start.x, d.start.y, blanks());
      const size = defaultSize(tool);
      if (blank && (!button || blank.kind === 'check')) {
        rect = { ...blank.rect };
      } else {
        rect = button
          ? { x: d.start.x - size.w / 2, y: d.start.y - size.h / 2, ...size }
          : { x: d.start.x, y: d.start.y - size.h, ...size };
      }
    } else {
      rect = normalise(d.start, d.end);
      if (rect.w < MIN_SIDE || rect.h < MIN_SIDE) return;
      const blank = e.altKey ? null : blankUnder(rect, blanks());
      if (blank) rect = { ...blank.rect };
      else if (!e.altKey) rect = snapToEdges(rect, blanks()) as PdfRect;
    }
    const f = newField(tool, props.pageId, rect, design(), userDatePicture());
    const blank = blankUnder(rect, blanks());
    if (blank && f.kind === 'text') {
      f.fontSize = blank.fontSizePt;
      if (blank.kind === 'comb' && blank.cells > 1) {
        f.comb = true;
        f.maxLen = blank.cells;
      }
      // A blank on the page needs no border of its own.
      f.border = false;
    }
    if (blank && (f.kind === 'checkbox' || f.kind === 'radio')) f.border = false;
    commitDesign(tab, addFields(design(), [f]));
    select([f.id]);
    setTool('select');
  };

  const drawing = () => {
    const d = drag();
    return d?.kind === 'draw' ? normalise(d.start, d.end) : null;
  };

  return (
    <Show when={on()}>
      <div
        ref={layerEl}
        class="prepare-layer"
        classList={{ 'is-drawing': prepareState.tool !== 'select' }}
        onPointerDown={onLayerDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={() => setDrag(null)}
      >
        <For each={covers()}>
          {(r) => <div class="prepare-cover" style={box(r)} aria-hidden="true" />}
        </For>
        <For each={fields()}>
          {(f) => (
            <div
              class="prepare-field"
              classList={{
                'is-selected': isSelected(f.id),
                'is-button': f.kind === 'checkbox' || f.kind === 'radio',
                'is-radio': f.kind === 'radio',
              }}
              style={box(shown(f))}
              title={f.tooltip || f.name}
              data-field={f.id}
              onPointerDown={(e) => onFieldDown(e, f)}
            >
              <Show when={f.kind !== 'checkbox' && f.kind !== 'radio'}>
                <span
                  class="prepare-field-name"
                  style={{
                    'font-size': `${Math.max(8, Math.min(11, f.rect.h * props.zoom * 0.55))}px`,
                  }}
                >
                  {f.name}
                </span>
              </Show>
              <Show when={prepareState.showTabs && order().includes(f.id)}>
                <span class="prepare-tab-badge">{order().indexOf(f.id) + 1}</span>
              </Show>
              <Show when={isSelected(f.id) && prepareState.selected.length === 1}>
                <For each={HANDLES}>
                  {(h) => (
                    <span
                      class={`prepare-handle is-${h}`}
                      onPointerDown={(e) => onHandleDown(e, f, h)}
                    />
                  )}
                </For>
              </Show>
            </div>
          )}
        </For>
        <For each={proposals()}>
          {(p) => (
            <div class="prepare-proposal" style={box(p.rect)} title={p.label || p.name}>
              <span class="prepare-proposal-actions" onPointerDown={(e) => e.stopPropagation()}>
                <button
                  type="button"
                  class="prepare-proposal-btn"
                  aria-label={`Accept ${p.name}`}
                  title={`Accept: ${p.name}`}
                  onClick={() => acceptProposals(tab, [p.id])}
                >
                  <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                    <path
                      d="M2.5 6.2l2.3 2.3 4.7-5"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.6"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
                <button
                  type="button"
                  class="prepare-proposal-btn"
                  aria-label={`Reject ${p.name}`}
                  title="Reject"
                  onClick={() => rejectProposals([p.id])}
                >
                  <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                    <path
                      d="M3 3l6 6M9 3l-6 6"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.6"
                      stroke-linecap="round"
                    />
                  </svg>
                </button>
              </span>
            </div>
          )}
        </For>
        <Show when={drawing()}>{(r) => <div class="prepare-rubber" style={box(r())} />}</Show>
      </div>
    </Show>
  );
}

function normalise(a: PdfPoint, b: PdfPoint): PdfRect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.abs(a.x - b.x),
    h: Math.abs(a.y - b.y),
  };
}

/** `r` with the edges `handle` holds moved by (dx, dy), y-up. */
export function resized(r: PdfRect, handle: Handle, dx: number, dy: number): PdfRect {
  let x0 = r.x;
  let y0 = r.y;
  let x1 = r.x + r.w;
  let y1 = r.y + r.h;
  if (handle.includes('w')) x0 += dx;
  if (handle.includes('e')) x1 += dx;
  if (handle.includes('n')) y1 += dy;
  if (handle.includes('s')) y0 += dy;
  if (x1 - x0 < MIN_SIDE) {
    if (handle.includes('w')) x0 = x1 - MIN_SIDE;
    else x1 = x0 + MIN_SIDE;
  }
  if (y1 - y0 < MIN_SIDE) {
    if (handle.includes('s')) y0 = y1 - MIN_SIDE;
    else y1 = y0 + MIN_SIDE;
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

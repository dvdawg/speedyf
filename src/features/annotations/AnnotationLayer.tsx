/** Vector annotation overlay for one page.
 *
 * Rendering happens in UNROTATED page space (the parent container applies the
 * rotation transform); pointer input arrives in rotated display coordinates
 * and is mapped through lib/coordinates. Annotations stay pure vectors during
 * interaction — the page raster is never re-rendered for annotation edits.
 */
import { createMemo, createResource, createSignal, For, Show, useContext } from 'solid-js';
import type { Annotation, PageEntry, PdfPoint, PdfQuad, PdfRect } from '../../types/model';
import type { PageGeom } from '../../lib/coordinates/coords';
import { cssToPdf } from '../../lib/coordinates/coords';
import type { TextRunDto } from '../../types/engine';
import { toolState, activeStyle, setActiveTool } from './toolStore';
import { imagePreviewUrl, newAnnotId } from '../editor/editorActions';
import { IconTrash } from '../../components/icons';
import { TabContext } from '../../app/TabContext';

interface Props {
  page: PageEntry;
  geom: PageGeom;
  zoom: number;
  runs: TextRunDto[];
}

interface DragState {
  kind: 'move' | 'resize' | 'draw';
  annotId?: string;
  handle?: 'nw' | 'ne' | 'sw' | 'se';
  startPdf: PdfPoint;
  lastPdf: PdfPoint;
  points?: PdfPoint[];
}

export default function AnnotationLayer(props: Props) {
  const { documentStore } = useContext(TabContext)!;
  const H = () => props.page.heightPt;
  const annots = () => documentStore.state.annotations[props.page.id] ?? [];
  const selectedId = () =>
    documentStore.state.selected?.pageId === props.page.id
      ? documentStore.state.selected.annotId
      : null;

  const [drag, setDrag] = createSignal<DragState | null>(null);
  const [draft, setDraft] = createSignal<{ rect: PdfRect; points?: PdfPoint[] } | null>(null);
  const [editingId, setEditingId] = createSignal<string | null>(null);

  let layerEl!: HTMLDivElement;

  const toPdf = (e: PointerEvent): PdfPoint => {
    const rect = layerEl.getBoundingClientRect();
    return cssToPdf(
      { x: e.clientX - rect.left, y: e.clientY - rect.top },
      props.geom,
      props.zoom * (rect.width / Math.max(1, layerCssW()))
    );
  };
  // display box width of the rotated page in css px (for DPI-safe mapping)
  const layerCssW = () =>
    (props.geom.rotation % 180 === 0 ? props.geom.widthPt : props.geom.heightPt) * props.zoom;

  const cssX = (x: number) => x * props.zoom;
  const cssY = (y: number, h: number) => (H() - y - h) * props.zoom;

  const drawingTool = () => toolState.active !== 'select';

  // ---------- creation ----------

  const finishRectDraw = (rect: PdfRect) => {
    const style = activeStyle();
    if (toolState.active === 'rect') {
      if (rect.w < 3 || rect.h < 3) return;
      documentStore.apply({
        type: 'addAnnot',
        annot: {
          id: newAnnotId(),
          pageId: props.page.id,
          kind: 'rect',
          rect,
          color: style.color,
          opacity: style.opacity,
          strokeWidth: style.strokeWidth,
        },
      });
    } else if (toolState.active === 'highlight') {
      const quads: PdfQuad[] = [];
      let union: PdfRect | null = null;
      for (const run of props.runs) {
        const x0 = Math.max(rect.x, run.x);
        const x1 = Math.min(rect.x + rect.w, run.x + run.w);
        const y0 = Math.max(rect.y, run.y);
        const y1 = Math.min(rect.y + rect.h, run.y + run.h);
        if (x1 - x0 < 1) continue;
        if (y1 - y0 < run.h * 0.35) continue;
        const q: PdfRect = { x: x0, y: run.y, w: x1 - x0, h: run.h };
        quads.push({
          p1: { x: q.x, y: q.y },
          p2: { x: q.x + q.w, y: q.y },
          p3: { x: q.x + q.w, y: q.y + q.h },
          p4: { x: q.x, y: q.y + q.h },
        });
        union = union
          ? {
              x: Math.min(union.x, q.x),
              y: Math.min(union.y, q.y),
              w: Math.max(union.x + union.w, q.x + q.w) - Math.min(union.x, q.x),
              h: Math.max(union.y + union.h, q.y + q.h) - Math.min(union.y, q.y),
            }
          : q;
      }
      if (!union || quads.length === 0) return;
      documentStore.apply({
        type: 'addAnnot',
        annot: {
          id: newAnnotId(),
          pageId: props.page.id,
          kind: 'highlight',
          rect: union,
          color: style.color,
          opacity: style.opacity,
          quads,
        },
      });
    }
  };

  const createAtPoint = (p: PdfPoint) => {
    const style = activeStyle();
    if (toolState.active === 'textbox') {
      const hPt = style.fontSizePt * 1.8;
      const id = newAnnotId();
      documentStore.apply({
        type: 'addAnnot',
        annot: {
          id,
          pageId: props.page.id,
          kind: 'textbox',
          rect: { x: p.x, y: p.y - hPt, w: 200, h: hPt * 1.6 },
          color: style.color,
          opacity: style.opacity,
          text: '',
          fontSizePt: style.fontSizePt,
        },
      });
      documentStore.setSelected({ pageId: props.page.id, annotId: id });
      setEditingId(id);
      setActiveTool('select');
    } else if (toolState.active === 'note') {
      const id = newAnnotId();
      documentStore.apply({
        type: 'addAnnot',
        annot: {
          id,
          pageId: props.page.id,
          kind: 'note',
          rect: { x: p.x - 9, y: p.y - 9, w: 18, h: 18 },
          color: style.color,
          opacity: style.opacity,
          text: '',
        },
      });
      documentStore.setSelected({ pageId: props.page.id, annotId: id });
      setEditingId(id);
      setActiveTool('select');
    }
  };

  // ---------- pointer handling ----------

  const onLayerPointerDown = (e: PointerEvent) => {
    if (!drawingTool() || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const p = toPdf(e);
    if (toolState.active === 'textbox' || toolState.active === 'note') {
      createAtPoint(p);
      return;
    }
    layerEl.setPointerCapture(e.pointerId);
    setDrag({ kind: 'draw', startPdf: p, lastPdf: p, points: [p] });
    setDraft({
      rect: { x: p.x, y: p.y, w: 0, h: 0 },
      ...(toolState.active === 'ink' ? { points: [p] } : {}),
    });
  };

  const onLayerPointerMove = (e: PointerEvent) => {
    const d = drag();
    if (!d) return;
    const p = toPdf(e);
    if (d.kind === 'draw') {
      if (toolState.active === 'ink') {
        const pts = d.points!;
        const last = pts[pts.length - 1]!;
        if (Math.hypot(p.x - last.x, p.y - last.y) > 1.2) pts.push(p);
        setDraft({ rect: bboxOf(pts), points: [...pts] });
      } else {
        setDraft({ rect: rectFrom(d.startPdf, p) });
      }
      setDrag({ ...d, lastPdf: p });
    } else if (d.kind === 'move' && d.annotId) {
      setDrag({ ...d, lastPdf: p });
    } else if (d.kind === 'resize' && d.annotId) {
      setDrag({ ...d, lastPdf: p });
    }
  };

  const onLayerPointerUp = (e: PointerEvent) => {
    const d = drag();
    if (!d) return;
    setDrag(null);
    const p = toPdf(e);
    if (d.kind === 'draw') {
      const draftState = draft();
      setDraft(null);
      if (!draftState) return;
      if (toolState.active === 'ink') {
        const pts = d.points ?? [];
        if (pts.length < 2) return;
        const style = activeStyle();
        documentStore.apply({
          type: 'addAnnot',
          annot: {
            id: newAnnotId(),
            pageId: props.page.id,
            kind: 'ink',
            rect: padRect(bboxOf(pts), (style.strokeWidth || 2) / 2 + 1),
            color: style.color,
            opacity: style.opacity,
            strokeWidth: style.strokeWidth || 2,
            strokes: [pts],
          },
        });
      } else {
        finishRectDraw(rectFrom(d.startPdf, p));
      }
    } else if (d.kind === 'move' && d.annotId) {
      const dx = p.x - d.startPdf.x;
      const dy = p.y - d.startPdf.y;
      if (Math.abs(dx) < 0.01 && Math.abs(dy) < 0.01) return;
      const a = annots().find((x) => x.id === d.annotId);
      if (!a) return;
      documentStore.apply({
        type: 'patchAnnot',
        pageId: props.page.id,
        id: a.id,
        patch: movedPatch(a, dx, dy),
      });
    } else if (d.kind === 'resize' && d.annotId && d.handle) {
      const a = annots().find((x) => x.id === d.annotId);
      if (!a) return;
      const rect = resizeRect(a.rect, d.handle, p);
      documentStore.apply({ type: 'patchAnnot', pageId: props.page.id, id: a.id, patch: { rect } });
    }
  };

  const startMove = (e: PointerEvent, a: Annotation) => {
    if (toolState.active !== 'select' || e.button !== 0) return;
    e.stopPropagation();
    documentStore.setSelected({ pageId: props.page.id, annotId: a.id });
    const p = toPdf(e);
    layerEl.setPointerCapture(e.pointerId);
    setDrag({ kind: 'move', annotId: a.id, startPdf: p, lastPdf: p });
  };

  const startResize = (e: PointerEvent, a: Annotation, handle: 'nw' | 'ne' | 'sw' | 'se') => {
    e.stopPropagation();
    const p = toPdf(e);
    layerEl.setPointerCapture(e.pointerId);
    setDrag({ kind: 'resize', annotId: a.id, handle, startPdf: p, lastPdf: p });
  };

  /** live-transformed annotation (drag/resize previews without store writes) */
  const liveAnnot = (a: Annotation): Annotation => {
    const d = drag();
    if (!d || d.annotId !== a.id) return a;
    if (d.kind === 'move') {
      const dx = d.lastPdf.x - d.startPdf.x;
      const dy = d.lastPdf.y - d.startPdf.y;
      return { ...a, ...movedPatch(a, dx, dy) } as Annotation;
    }
    if (d.kind === 'resize' && d.handle) {
      return { ...a, rect: resizeRect(a.rect, d.handle, d.lastPdf) };
    }
    return a;
  };

  const commitText = (a: Annotation, text: string) => {
    setEditingId(null);
    if (text === (a.text ?? '')) {
      if (text.trim() === '' && a.kind === 'textbox') {
        documentStore.apply({ type: 'deleteAnnot', pageId: props.page.id, id: a.id });
      }
      return;
    }
    documentStore.apply({ type: 'patchAnnot', pageId: props.page.id, id: a.id, patch: { text } });
  };

  const deleteSelected = (a: Annotation) => {
    documentStore.apply({ type: 'deleteAnnot', pageId: props.page.id, id: a.id });
  };

  return (
    <div
      ref={layerEl}
      class="annot-layer"
      classList={{ 'is-drawing': drawingTool() }}
      onPointerDown={onLayerPointerDown}
      onPointerMove={onLayerPointerMove}
      onPointerUp={onLayerPointerUp}
    >
      <svg
        class="annot-svg"
        viewBox={`0 0 ${props.page.widthPt} ${H()}`}
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        <For each={annots()}>
          {(raw) => {
            const a = createMemo(() => liveAnnot(raw));
            return (
              <>
                <Show when={a().kind === 'ink'}>
                  <For each={a().strokes ?? []}>
                    {(stroke) => (
                      <path
                        class="annot-shape"
                        classList={{ 'is-selected': selectedId() === a().id }}
                        d={strokePath(stroke, H())}
                        fill="none"
                        stroke={a().color}
                        stroke-opacity={a().opacity}
                        stroke-width={a().strokeWidth ?? 2}
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        pointer-events={toolState.active === 'select' ? 'stroke' : 'none'}
                        onPointerDown={(e) => startMove(e, raw)}
                      />
                    )}
                  </For>
                </Show>
                <Show when={a().kind === 'rect'}>
                  <rect
                    class="annot-shape"
                    classList={{ 'is-selected': selectedId() === a().id }}
                    x={a().rect.x}
                    y={H() - a().rect.y - a().rect.h}
                    width={a().rect.w}
                    height={a().rect.h}
                    fill="none"
                    stroke={a().color}
                    stroke-opacity={a().opacity}
                    stroke-width={a().strokeWidth ?? 2}
                    pointer-events={toolState.active === 'select' ? 'stroke' : 'none'}
                    onPointerDown={(e) => startMove(e, raw)}
                  />
                </Show>
              </>
            );
          }}
        </For>
        {/* live drafts */}
        <Show when={draft() && toolState.active === 'ink'}>
          <path
            d={strokePath(draft()!.points ?? [], H())}
            fill="none"
            stroke={activeStyle().color}
            stroke-opacity={activeStyle().opacity}
            stroke-width={activeStyle().strokeWidth || 2}
            stroke-linecap="round"
          />
        </Show>
        <Show when={draft() && (toolState.active === 'rect' || toolState.active === 'highlight')}>
          <rect
            x={draft()!.rect.x}
            y={H() - draft()!.rect.y - draft()!.rect.h}
            width={draft()!.rect.w}
            height={draft()!.rect.h}
            fill={toolState.active === 'highlight' ? activeStyle().color : 'none'}
            fill-opacity={toolState.active === 'highlight' ? 0.25 : 0}
            stroke={activeStyle().color}
            stroke-width={1}
            stroke-dasharray="4 3"
          />
        </Show>
      </svg>

      {/* HTML-positioned annotations: highlights, textboxes, notes, images */}
      <For each={annots()}>
        {(raw) => {
          const a = createMemo(() => liveAnnot(raw));
          const isSel = () => selectedId() === a().id;
          return (
            <>
              <Show when={a().kind === 'highlight'}>
                <For each={a().quads ?? []}>
                  {(q) => (
                    <div
                      class="annot-highlight"
                      style={{
                        left: `${cssX(q.p1.x)}px`,
                        top: `${cssY(q.p1.y, q.p4.y - q.p1.y)}px`,
                        width: `${(q.p2.x - q.p1.x) * props.zoom}px`,
                        height: `${(q.p4.y - q.p1.y) * props.zoom}px`,
                        background: a().color,
                        opacity: a().opacity,
                        'pointer-events': toolState.active === 'select' ? 'auto' : 'none',
                      }}
                      onPointerDown={(e) => startMove(e, raw)}
                    />
                  )}
                </For>
              </Show>

              <Show when={a().kind === 'textbox'}>
                <div
                  class="annot-textbox"
                  classList={{ 'is-selected': isSel() }}
                  style={{
                    left: `${cssX(a().rect.x)}px`,
                    top: `${cssY(a().rect.y, a().rect.h)}px`,
                    width: `${a().rect.w * props.zoom}px`,
                    height: `${a().rect.h * props.zoom}px`,
                    color: a().color,
                    opacity: a().opacity,
                    'font-size': `${(a().fontSizePt ?? 14) * props.zoom}px`,
                    'pointer-events': toolState.active === 'select' ? 'auto' : 'none',
                  }}
                  onPointerDown={(e) => {
                    if (editingId() !== a().id) startMove(e, raw);
                  }}
                  onDblClick={() => setEditingId(a().id)}
                >
                  <Show
                    when={editingId() === a().id}
                    fallback={<div class="annot-text-content">{a().text}</div>}
                  >
                    <textarea
                      class="annot-text-editor"
                      value={a().text ?? ''}
                      ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                      onPointerDown={(e) => e.stopPropagation()}
                      onBlur={(e) => commitText(raw, e.currentTarget.value)}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === 'Escape') e.currentTarget.blur();
                      }}
                      aria-label="Text box content"
                    />
                  </Show>
                </div>
              </Show>

              <Show when={a().kind === 'note'}>
                <div
                  class="annot-note"
                  classList={{ 'is-selected': isSel() }}
                  style={{
                    left: `${cssX(a().rect.x)}px`,
                    top: `${cssY(a().rect.y, a().rect.h)}px`,
                    width: `${a().rect.w * props.zoom}px`,
                    height: `${a().rect.h * props.zoom}px`,
                    background: a().color,
                    opacity: a().opacity,
                    'pointer-events': toolState.active === 'select' ? 'auto' : 'none',
                  }}
                  title={a().text || 'Note'}
                  onPointerDown={(e) => startMove(e, raw)}
                  onDblClick={() => setEditingId(a().id)}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <path
                      d="M2.5 2.5h11v8h-5l-3 3v-3h-3v-8z"
                      fill="rgba(255,255,255,0.85)"
                      stroke="rgba(0,0,0,0.4)"
                    />
                  </svg>
                </div>
              </Show>

              <Show when={a().kind === 'image'}>
                <ImageAnnot
                  annot={a()}
                  cssX={cssX}
                  cssY={cssY}
                  zoom={props.zoom}
                  selected={isSel()}
                  onPointerDown={(e) => startMove(e, raw)}
                />
              </Show>

              <button
                type="button"
                class="annot-keyboard-target"
                style={{
                  left: `${cssX(a().rect.x)}px`,
                  top: `${cssY(a().rect.y, a().rect.h)}px`,
                  width: `${Math.max(12, a().rect.w * props.zoom)}px`,
                  height: `${Math.max(12, a().rect.h * props.zoom)}px`,
                }}
                tabIndex={toolState.active === 'select' ? 0 : -1}
                aria-label={`${a().kind} annotation${a().text ? `: ${a().text}` : ''}`}
                aria-pressed={isSel()}
                title={`Select ${a().kind} annotation`}
                onFocus={() =>
                  documentStore.setSelected({ pageId: props.page.id, annotId: a().id })
                }
                onClick={() => {
                  documentStore.setSelected({ pageId: props.page.id, annotId: a().id });
                  if (a().kind === 'textbox' || a().kind === 'note') setEditingId(a().id);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Delete' || e.key === 'Backspace') {
                    e.preventDefault();
                    e.stopPropagation();
                    deleteSelected(raw);
                  } else if (e.key === 'Escape') {
                    e.preventDefault();
                    documentStore.setSelected(null);
                  }
                }}
              />

              {/* note popover editor */}
              <Show when={a().kind === 'note' && editingId() === a().id}>
                <div
                  class="note-popover"
                  style={{
                    left: `${cssX(a().rect.x)}px`,
                    top: `${cssY(a().rect.y, a().rect.h) + a().rect.h * props.zoom + 4}px`,
                  }}
                  onPointerDown={(e) => e.stopPropagation()}
                >
                  <textarea
                    value={a().text ?? ''}
                    ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                    onBlur={(e) => commitText(raw, e.currentTarget.value)}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === 'Escape') e.currentTarget.blur();
                    }}
                    aria-label="Note text"
                  />
                </div>
              </Show>

              {/* selection chrome: resize handles + delete */}
              <Show when={isSel() && ['rect', 'textbox', 'image'].includes(a().kind)}>
                <For each={['nw', 'ne', 'sw', 'se'] as const}>
                  {(h) => (
                    <div
                      class={`annot-handle handle-${h}`}
                      style={handleStyle(a().rect, h, props.zoom, H())}
                      onPointerDown={(e) => startResize(e, raw, h)}
                    />
                  )}
                </For>
              </Show>
              <Show when={isSel()}>
                <button
                  type="button"
                  class="annot-delete"
                  style={{
                    left: `${cssX(a().rect.x) + a().rect.w * props.zoom + 6}px`,
                    top: `${cssY(a().rect.y, a().rect.h) - 6}px`,
                  }}
                  title="Delete annotation (Del)"
                  aria-label="Delete annotation"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => deleteSelected(raw)}
                >
                  <IconTrash />
                </button>
              </Show>
            </>
          );
        }}
      </For>
    </div>
  );
}

function ImageAnnot(props: {
  annot: Annotation;
  cssX: (x: number) => number;
  cssY: (y: number, h: number) => number;
  zoom: number;
  selected: boolean;
  onPointerDown: (e: PointerEvent) => void;
}) {
  const [url] = createResource(
    () => props.annot.sourcePath,
    (p) => (p ? imagePreviewUrl(p).catch(() => null) : null)
  );
  return (
    <div
      class="annot-image"
      classList={{ 'is-selected': props.selected }}
      style={{
        left: `${props.cssX(props.annot.rect.x)}px`,
        top: `${props.cssY(props.annot.rect.y, props.annot.rect.h)}px`,
        width: `${props.annot.rect.w * props.zoom}px`,
        height: `${props.annot.rect.h * props.zoom}px`,
        'pointer-events': toolState.active === 'select' ? 'auto' : 'none',
      }}
      onPointerDown={(e) => props.onPointerDown(e)}
    >
      <Show when={url()} fallback={<div class="annot-image-ph">image</div>}>
        <img src={url()!} alt="Placed image" draggable={false} />
      </Show>
    </div>
  );
}

// ---------- pure helpers ----------

function rectFrom(a: PdfPoint, b: PdfPoint): PdfRect {
  return {
    x: Math.min(a.x, b.x),
    y: Math.min(a.y, b.y),
    w: Math.abs(a.x - b.x),
    h: Math.abs(a.y - b.y),
  };
}

function bboxOf(pts: PdfPoint[]): PdfRect {
  let x0 = Infinity,
    y0 = Infinity,
    x1 = -Infinity,
    y1 = -Infinity;
  for (const p of pts) {
    x0 = Math.min(x0, p.x);
    y0 = Math.min(y0, p.y);
    x1 = Math.max(x1, p.x);
    y1 = Math.max(y1, p.y);
  }
  return { x: x0, y: y0, w: Math.max(0, x1 - x0), h: Math.max(0, y1 - y0) };
}

function padRect(r: PdfRect, pad: number): PdfRect {
  return { x: r.x - pad, y: r.y - pad, w: r.w + 2 * pad, h: r.h + 2 * pad };
}

function strokePath(pts: PdfPoint[], pageH: number): string {
  if (pts.length === 0) return '';
  const parts = [`M ${pts[0]!.x.toFixed(2)} ${(pageH - pts[0]!.y).toFixed(2)}`];
  for (let i = 1; i < pts.length; i++) {
    parts.push(`L ${pts[i]!.x.toFixed(2)} ${(pageH - pts[i]!.y).toFixed(2)}`);
  }
  return parts.join(' ');
}

function movedPatch(a: Annotation, dx: number, dy: number): Partial<Annotation> {
  const patch: Partial<Annotation> = {
    rect: { x: a.rect.x + dx, y: a.rect.y + dy, w: a.rect.w, h: a.rect.h },
  };
  if (a.strokes) {
    patch.strokes = a.strokes.map((s) => s.map((p) => ({ x: p.x + dx, y: p.y + dy })));
  }
  if (a.quads) {
    patch.quads = a.quads.map((q) => ({
      p1: { x: q.p1.x + dx, y: q.p1.y + dy },
      p2: { x: q.p2.x + dx, y: q.p2.y + dy },
      p3: { x: q.p3.x + dx, y: q.p3.y + dy },
      p4: { x: q.p4.x + dx, y: q.p4.y + dy },
    }));
  }
  return patch;
}

function resizeRect(r: PdfRect, handle: 'nw' | 'ne' | 'sw' | 'se', p: PdfPoint): PdfRect {
  // PDF space y-up: "n" (screen top) = high y, "s" = low y
  let x0 = r.x,
    y0 = r.y,
    x1 = r.x + r.w,
    y1 = r.y + r.h;
  if (handle === 'nw') {
    x0 = p.x;
    y1 = p.y;
  } else if (handle === 'ne') {
    x1 = p.x;
    y1 = p.y;
  } else if (handle === 'sw') {
    x0 = p.x;
    y0 = p.y;
  } else {
    x1 = p.x;
    y0 = p.y;
  }
  return {
    x: Math.min(x0, x1),
    y: Math.min(y0, y1),
    w: Math.max(8, Math.abs(x1 - x0)),
    h: Math.max(8, Math.abs(y1 - y0)),
  };
}

function handleStyle(r: PdfRect, handle: 'nw' | 'ne' | 'sw' | 'se', zoom: number, pageH: number) {
  const x = handle === 'nw' || handle === 'sw' ? r.x : r.x + r.w;
  const y = handle === 'nw' || handle === 'ne' ? r.y + r.h : r.y;
  return {
    left: `${x * zoom - 5}px`,
    top: `${(pageH - y) * zoom - 5}px`,
  };
}

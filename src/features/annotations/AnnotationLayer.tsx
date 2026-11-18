/** Vector annotation overlay for one page.
 *
 * Rendering happens in UNROTATED page space (the parent container applies the
 * rotation transform); pointer input arrives in rotated display coordinates
 * and is mapped through lib/coordinates. Annotations stay pure vectors during
 * interaction — the page raster is never re-rendered for annotation edits.
 */
import {
  createEffect,
  createMemo,
  createResource,
  createSignal,
  For,
  onCleanup,
  Show,
  untrack,
  useContext,
} from 'solid-js';
import type { Annotation, PageEntry, PdfPoint, PdfQuad, PdfRect } from '../../types/model';
import type { PageGeom } from '../../lib/coordinates/coords';
import { cssToPdf } from '../../lib/coordinates/coords';
import type { Bubble, TextRunDto } from '../../types/engine';
import { engine } from '../../lib/transport/engine';
import { toolState, activeStyle, setActiveTool } from './toolStore';
import { shadeRect, shadingFor } from './bubbles';
import { invertInk } from '../../lib/rendering/invertColor';
import { pagesInverted } from '../../stores/settings';
import { imagePreviewUrl, newAnnotId } from '../editor/editorActions';
import { IconTrash } from '../../components/icons';
import { TabContext } from '../../app/TabContext';
import { activeTab } from '../../stores/tabsStore';
import { fitTextBox, textFontCss, textStyleOf } from './textStyle';
import { highlightRects } from './highlightRects';
import type { EquationBlock } from '../extensions/EquationLayer';
import { placePendingSignature, signState } from '../sign/signStore';
import { resizeKeepingAspect } from '../sign/art';
import SignatureArtView from '../sign/SignatureArtView';
import {
  blankOfAnnot,
  blanksOf,
  fillActive,
  fillState,
  linkBlank,
  loadBlanks,
  requestArrival,
  setFillMode,
  takeArrival,
  arrival,
} from '../sign/fillStore';
import {
  blankAt,
  formatDate,
  freeText,
  markPath,
  markPen,
  markRect,
  nearbyFontSize,
  nextStop,
  snapText,
} from '../sign/fill';
import { settings } from '../../stores/settings';
import type { Blank } from '../../types/engine';
import type { MarkKind } from '../../types/model';

interface Props {
  page: PageEntry;
  geom: PageGeom;
  zoom: number;
  runs: TextRunDto[];
  /** the page's display equations, boxes from its top-left */
  equations?: readonly EquationBlock[];
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
  const tab = useContext(TabContext)!;
  const { documentStore, viewport } = tab;
  const H = () => props.page.heightPt;
  const annots = () => documentStore.state.annotations[props.page.id] ?? [];
  const selectedId = () =>
    documentStore.state.selected?.pageId === props.page.id
      ? documentStore.state.selected.annotId
      : null;

  const [drag, setDrag] = createSignal<DragState | null>(null);
  const [draft, setDraft] = createSignal<{ rect: PdfRect; points?: PdfPoint[] } | null>(null);
  const [editingId, setEditingId] = createSignal<string | null>(null);
  let typingGroup = Symbol('text edit');

  const finishEditing = (deselect = false) => {
    const a = annots().find((a) => a.id === editingId());
    setEditingId(null);
    if (deselect && selectedId() === a?.id) documentStore.setSelected(null);
    if (a?.kind === 'textbox' && !(a.text ?? '').trim()) {
      documentStore.apply({ type: 'deleteAnnot', pageId: props.page.id, id: a.id });
    }
  };

  /** Tools a text box can be typed into under: Select, and Fill, which
   * keeps the tool in hand while answering one blank after another. */
  const typingTool = () => toolState.active === 'select' || toolState.active === 'fill';

  createEffect(() => {
    const id = editingId();
    if (!id) return;
    if (selectedId() !== id || !typingTool()) untrack(finishEditing);
  });

  createEffect(() => {
    const id = editingId();
    if (!id || annots().find((a) => a.id === id)?.kind !== 'textbox') return;
    viewport.setState('editingText', { pageId: props.page.id, annotId: id });
    onCleanup(() => {
      if (
        viewport.state.editingText?.pageId === props.page.id &&
        viewport.state.editingText.annotId === id
      )
        viewport.setState('editingText', null);
    });
  });

  createEffect(() => {
    const id = selectedId();
    if (!id || annots().find((a) => a.id === id)?.kind !== 'textbox') return;
    if (!typingTool() || (activeTab() && activeTab()!.id !== tab.id)) {
      untrack(() => {
        finishEditing();
        documentStore.setSelected(null);
      });
      return;
    }
    // A native color picker can blur the textarea with no relatedTarget.
    // Dismiss only on interaction outside this box and its editing controls.
    const leaveTextBox = (e: Event) => {
      const target = e.target;
      if (!(target instanceof Element)) return;
      if (editingId() === id && target.closest('.text-tool-options')) return;
      const owner = target.closest<HTMLElement>('[data-annot-id]');
      if (owner?.dataset.annotId === id && layerEl.contains(owner)) return;
      finishEditing();
      if (selectedId() === id) documentStore.setSelected(null);
    };
    document.addEventListener('pointerdown', leaveTextBox, true);
    document.addEventListener('focusin', leaveTextBox);
    onCleanup(() => {
      document.removeEventListener('pointerdown', leaveTextBox, true);
      document.removeEventListener('focusin', leaveTextBox);
    });
  });

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
  /** A saved signature is armed: the next click on this page places it. */
  const placing = () => signState.pending !== null;

  // ---------- bubbles ----------

  // Only fetched once the tool is picked up: most pages have no bubbles, and
  // a reader who never reaches for the tool should never pay to find out.
  const [bubbles] = createResource(
    () => {
      const src = props.page.srcIndex;
      if (toolState.active !== 'bubble' || src === null) return null;
      return { docId: documentStore.state.docId, src };
    },
    ({ docId, src }): Promise<Bubble[]> => engine.getPageBubbles(docId, src).catch(() => [])
  );

  const shadedAt = (b: Bubble) => shadingFor(annots(), b.charIndex, b.rect);

  /** The colour to paint a shading in. A shaded bubble is standing in for ink
   * on the page — near-black, like the ring it fills — so on an inverted page
   * it has to go through the same flip the raster did, or it disappears into
   * the dark paper. Every other annotation keeps its authored colour: those
   * are the reader's marks sitting on top of the page, not part of it. */
  const shadeColor = (color: string) => (pagesInverted() ? invertInk(color) : color);

  /** Shade an empty bubble, or clear one already shaded. */
  const toggleBubble = (b: Bubble) => {
    const existing = shadedAt(b);
    if (existing) {
      documentStore.apply({ type: 'deleteAnnot', pageId: props.page.id, id: existing.id });
      return;
    }
    const style = activeStyle();
    documentStore.apply({
      type: 'addAnnot',
      annot: {
        id: newAnnotId(),
        pageId: props.page.id,
        kind: 'bubble',
        rect: shadeRect(b.rect),
        color: style.color,
        opacity: style.opacity,
        bubbleCharIndex: b.charIndex,
      },
    });
  };

  // ---------- Fill & Sign ----------

  const docId = () => documentStore.state.docId;
  // Asked for only while the tool is held, like bubbles: a reader who never
  // fills a form never pays to have its blanks found.
  createEffect(() => {
    const src = props.page.srcIndex;
    if (fillActive() && src !== null) void loadBlanks(docId(), src);
  });
  const blanks = (): Blank[] => {
    const src = props.page.srcIndex;
    return fillActive() && src !== null ? (blanksOf(docId(), src) ?? []) : [];
  };

  /** The text box already answering blank `index`: the one typed into it
   * this session, or one sitting on it (read back from a saved file). */
  const answerOf = (index: number): Annotation | undefined => {
    const b = blanks()[index];
    if (!b) return undefined;
    return annots().find((a) => {
      if (a.kind !== 'textbox') return false;
      const link = blankOfAnnot(a.id);
      if (link) return link.pageId === props.page.id && link.blank === index;
      const cx = a.rect.x + Math.min(a.rect.w, 12) / 2;
      const cy = a.rect.y + a.rect.h / 2;
      return (
        cx >= b.rect.x &&
        cx <= b.rect.x + b.rect.w &&
        cy >= b.rect.y - 4 &&
        cy <= b.rect.y + b.rect.h + 4
      );
    });
  };

  const editText = (id: string) => {
    documentStore.setSelected({ pageId: props.page.id, annotId: id });
    setEditingId(id);
  };

  /** A new text box from the Fill tool's style. */
  const fillTextBox = (
    rect: PdfRect,
    fontSizePt: number,
    text: string,
    combCells?: number
  ): Annotation => {
    const style = toolState.styles.fill;
    const annot: Annotation = {
      id: newAnnotId(),
      pageId: props.page.id,
      kind: 'textbox',
      rect,
      color: style.color,
      opacity: style.opacity,
      text,
      autoSize: false,
      ...textStyleOf(style),
      fontSizePt,
      ...(combCells ? { combCells } : {}),
    };
    annot.rect = fitTextBox(annot, props.page);
    return annot;
  };

  /** Open blank `index` for typing: the answer already there, or a new text
   * box snapped onto it. */
  const openBlank = (index: number) => {
    const existing = answerOf(index);
    if (existing) {
      editText(existing.id);
      return;
    }
    const b = blanks()[index];
    if (!b) return;
    const snapped = snapText(b);
    const annot = fillTextBox(snapped.rect, snapped.fontSizePt, '', snapped.combCells);
    documentStore.apply({ type: 'addAnnot', annot });
    linkBlank(annot.id, props.page.id, index);
    editText(annot.id);
  };

  // Tab from another page: open the blank it was headed for once this page
  // is mounted and its blanks are known.
  createEffect(() => {
    const a = arrival();
    if (!a || a.pageId !== props.page.id || !fillActive() || blanks().length === 0) return;
    const index = untrack(() => takeArrival(props.page.id));
    if (index !== null) untrack(() => openBlank(index));
  });

  /** Tab and Shift-Tab: the next blank in reading order, on this page or the
   * next one that has any. The box being left goes if it is still empty. */
  const tabFrom = (a: Annotation, direction: 1 | -1) => {
    const link = blankOfAnnot(a.id);
    const here =
      link?.pageId === props.page.id
        ? link.blank
        : blankAt(blanks(), { x: a.rect.x + 2, y: a.rect.y + a.rect.h / 2 });
    finishEditing(true);
    const next = nextStop(blanks(), here, direction);
    if (next !== null) {
      openBlank(next);
      return;
    }
    void tabToPage(direction);
  };

  const tabToPage = async (direction: 1 | -1) => {
    const pages = documentStore.state.pages;
    const from = pages.findIndex((p) => p.id === props.page.id);
    for (
      let i = from + direction, seen = 0;
      i >= 0 && i < pages.length && seen < 50;
      i += direction, seen++
    ) {
      const page = pages[i]!;
      if (page.srcIndex === null) continue;
      const found = await loadBlanks(docId(), page.srcIndex);
      const stop = nextStop(found, null, direction);
      if (stop === null) continue;
      const b = found[stop]!;
      requestArrival(page.id, stop);
      const rotation = (page.baseRotation + page.userRotation + viewport.state.viewRotation) % 360;
      const heightPt = rotation % 180 === 0 ? page.heightPt : page.widthPt;
      const top = b.rect.y + b.rect.h;
      viewport.requestScrollToPage(
        i,
        Math.max(0, (heightPt - top) * viewport.state.zoom - viewport.state.containerH * 0.3)
      );
      return;
    }
  };

  const placeMark = (mark: MarkKind, p: PdfPoint) => {
    const index = blankAt(blanks(), p, ['check', 'box']);
    const box = index === null ? null : blanks()[index]!.rect;
    // Only a box about the size of a mark centres it; a big box takes the
    // mark where it was dropped.
    const fits = box && Math.max(box.w, box.h) <= 30 ? box : null;
    const size = fits ? Math.min(fits.w, fits.h) : nearbyFontSize(props.runs, p);
    const rect = markRect(mark, p, size, fits);
    const style = toolState.styles.fill;
    documentStore.apply({
      type: 'addAnnot',
      annot: {
        id: newAnnotId(),
        pageId: props.page.id,
        kind: 'mark',
        mark,
        rect,
        color: style.color,
        opacity: style.opacity,
        strokeWidth: markPen(rect),
      },
    });
  };

  /** A click with the Fill tool. */
  const fillClick = (p: PdfPoint) => {
    const mode = fillState.mode;
    if (mode !== 'text' && mode !== 'date') {
      placeMark(mode, p);
      return;
    }
    const all = blanks();
    const index = blankAt(all, p);
    const b = index === null ? undefined : all[index];
    if (mode === 'text' && b?.kind === 'check') {
      placeMark('check', p);
      return;
    }
    if (mode === 'date') {
      const text = formatDate(new Date(), settings.dateFormat);
      const size = b ? b.fontSizePt : nearbyFontSize(props.runs, p);
      const rect = b
        ? snapText({ ...b, kind: b.kind === 'comb' ? 'box' : b.kind }).rect
        : freeText(p, size, props.page.widthPt);
      const annot = fillTextBox(rect, size, text);
      if (!b) annot.autoSize = true;
      annot.rect = fitTextBox(annot, props.page);
      documentStore.apply({ type: 'addAnnot', annot });
      documentStore.setSelected({ pageId: props.page.id, annotId: annot.id });
      // A stamp, not a mode: the next click types again.
      setFillMode('text');
      return;
    }
    if (index !== null) {
      openBlank(index);
      return;
    }
    const size = nearbyFontSize(props.runs, p);
    const annot = fillTextBox(freeText(p, size, props.page.widthPt), size, '');
    annot.autoSize = true;
    annot.rect = fitTextBox(annot, props.page);
    documentStore.apply({ type: 'addAnnot', annot });
    editText(annot.id);
  };

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
      const displays = (props.equations ?? []).map(({ bbox: [x, y, w, h], number }) => ({
        x,
        y: props.geom.heightPt - y - h,
        w,
        h,
        number,
      }));
      for (const q of highlightRects(props.runs, rect, displays)) {
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
      const id = newAnnotId();
      const annot: Annotation = {
        id,
        pageId: props.page.id,
        kind: 'textbox',
        rect: {
          x: Math.max(0, Math.min(p.x, props.page.widthPt - style.fontSizePt * 8)),
          y: p.y,
          w: 0,
          h: 0,
        },
        color: style.color,
        opacity: style.opacity,
        text: '',
        autoSize: true,
        ...textStyleOf(style),
      };
      annot.rect = fitTextBox(annot, props.page);
      documentStore.apply({
        type: 'addAnnot',
        annot,
      });
      setActiveTool('select');
      documentStore.setSelected({ pageId: props.page.id, annotId: id });
      setEditingId(id);
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
      setActiveTool('select');
      documentStore.setSelected({ pageId: props.page.id, annotId: id });
      setEditingId(id);
    }
  };

  // ---------- pointer handling ----------

  const onLayerPointerDown = (e: PointerEvent) => {
    if (placing() && e.button === 0) {
      e.preventDefault();
      e.stopPropagation();
      void placePendingSignature(documentStore, props.page, toPdf(e));
      return;
    }
    if (!drawingTool() || e.button !== 0) return;
    // The bubble tool has no drag gesture of its own: it clicks targets that
    // are already on the page, and each target handles its own pointer.
    if (toolState.active === 'bubble') return;
    if (toolState.active === 'fill') {
      e.preventDefault();
      e.stopPropagation();
      fillClick(toPdf(e));
      return;
    }
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
      const rect = resizeFor(a, d.handle, p);
      documentStore.apply({
        type: 'patchAnnot',
        pageId: props.page.id,
        id: a.id,
        patch: { rect, ...(a.kind === 'textbox' ? { autoSize: false } : {}) },
      });
    }
  };

  const startMove = (e: PointerEvent, a: Annotation) => {
    // Armed to place a signature: let the press reach the layer instead.
    if (placing()) return;
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
      return { ...a, rect: resizeFor(a, d.handle, d.lastPdf) };
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
      classList={{
        'is-drawing': drawingTool() || placing(),
        'is-placing-signature': placing(),
        'is-filling': fillActive() && fillState.mode === 'text',
      }}
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
                <Show when={a().kind === 'bubble'}>
                  <ellipse
                    class="annot-shape annot-bubble"
                    classList={{
                      'is-selected': selectedId() === a().id,
                      'is-inert': toolState.active !== 'select',
                    }}
                    cx={a().rect.x + a().rect.w / 2}
                    cy={H() - a().rect.y - a().rect.h / 2}
                    rx={a().rect.w / 2}
                    ry={a().rect.h / 2}
                    fill={shadeColor(a().color)}
                    fill-opacity={a().opacity}
                    onPointerDown={(e) => startMove(e, raw)}
                  />
                </Show>
                <Show when={a().kind === 'mark' && a().mark}>
                  {(mark) => (
                    <path
                      class="annot-shape annot-mark"
                      classList={{ 'is-selected': selectedId() === a().id }}
                      d={markPath(mark(), a().rect, H())}
                      fill={mark() === 'dot' ? a().color : 'none'}
                      fill-opacity={a().opacity}
                      stroke={mark() === 'dot' ? 'none' : a().color}
                      stroke-opacity={a().opacity}
                      stroke-width={a().strokeWidth ?? markPen(a().rect)}
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      pointer-events={toolState.active === 'select' ? 'all' : 'none'}
                      onPointerDown={(e) => startMove(e, raw)}
                    />
                  )}
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

        {/* Printed-form blanks, outlined only while the Fill tool is held.
            Clicks go to the layer, which works out which blank was meant. */}
        <Show when={fillActive()}>
          <For each={blanks()}>
            {(b) => (
              <rect
                class="fill-blank"
                classList={{ 'is-line': b.kind === 'line' }}
                x={b.rect.x}
                y={H() - b.rect.y - b.rect.h}
                width={b.rect.w}
                height={b.rect.h}
              />
            )}
          </For>
        </Show>

        {/* Bubble targets. Drawn last so they sit above the shadings they
            toggle, and only while the tool is held — the rest of the time a
            page must look exactly like the paper it came from. */}
        <Show when={toolState.active === 'bubble'}>
          <For each={bubbles() ?? []}>
            {(b) => (
              <ellipse
                class="bubble-target"
                classList={{ 'is-shaded': !!shadedAt(b) }}
                cx={b.rect.x + b.rect.w / 2}
                cy={H() - b.rect.y - b.rect.h / 2}
                rx={b.rect.w / 2}
                ry={b.rect.h / 2}
                onPointerDown={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  if (e.button === 0) toggleBubble(b);
                }}
              />
            )}
          </For>
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
                  data-annot-id={a().id}
                  classList={{ 'is-selected': isSel() }}
                  style={{
                    left: `${cssX(a().rect.x)}px`,
                    top: `${cssY(a().rect.y, a().rect.h)}px`,
                    width: `${a().rect.w * props.zoom}px`,
                    height: `${a().rect.h * props.zoom}px`,
                    color: a().color,
                    opacity: a().opacity,
                    'font-size': `${textStyleOf(a()).fontSizePt * props.zoom}px`,
                    'font-family': textFontCss[textStyleOf(a()).fontFamily],
                    'font-weight': a().bold ? '700' : '400',
                    'font-style': a().italic ? 'italic' : 'normal',
                    'text-decoration': a().underline ? 'underline' : 'none',
                    'pointer-events': typingTool() ? 'auto' : 'none',
                  }}
                  onPointerDown={(e) => {
                    if (editingId() === a().id) return;
                    if (toolState.active === 'fill') {
                      // Filling: a click on an answer edits it again.
                      e.stopPropagation();
                      e.preventDefault();
                      editText(a().id);
                      return;
                    }
                    startMove(e, raw);
                  }}
                  onDblClick={() => {
                    if (toolState.active !== 'select') return;
                    documentStore.setSelected({ pageId: props.page.id, annotId: a().id });
                    setEditingId(a().id);
                  }}
                >
                  <Show
                    when={editingId() === a().id}
                    fallback={
                      <Show
                        when={a().combCells}
                        fallback={<div class="annot-text-content">{a().text}</div>}
                      >
                        {(cells) => (
                          <div class="annot-text-comb">
                            <For each={combChars(a().text ?? '', cells())}>
                              {(c) => (
                                <span style={{ width: `${(a().rect.w / cells()) * props.zoom}px` }}>
                                  {c}
                                </span>
                              )}
                            </For>
                          </div>
                        )}
                      </Show>
                    }
                  >
                    <textarea
                      class="annot-text-editor"
                      classList={{ 'is-comb': !!a().combCells }}
                      style={combEditorStyle(a(), props.zoom)}
                      maxLength={a().combCells}
                      value={a().text ?? ''}
                      ref={(el) => queueMicrotask(() => el.isConnected && el.focus())}
                      onPointerDown={(e) => e.stopPropagation()}
                      onFocus={() => {
                        typingGroup = Symbol('text edit');
                      }}
                      onInput={(e) => {
                        const text = e.currentTarget.value;
                        documentStore.apply(
                          {
                            type: 'patchAnnot',
                            pageId: props.page.id,
                            id: raw.id,
                            patch: { text, rect: fitTextBox({ ...raw, text }, props.page) },
                          },
                          typingGroup
                        );
                      }}
                      onKeyDown={(e) => {
                        // File shortcuts remain available while typing; the
                        // app's typing guard already protects normal editing.
                        if (e.key === 'Escape') {
                          e.stopPropagation();
                          finishEditing(true);
                        } else if (
                          e.key === 'Tab' &&
                          toolState.active === 'fill' &&
                          !e.metaKey &&
                          !e.ctrlKey &&
                          !e.altKey
                        ) {
                          e.preventDefault();
                          e.stopPropagation();
                          tabFrom(raw, e.shiftKey ? -1 : 1);
                        } else if (e.key === 'Enter' && a().combCells) {
                          // One line of cells: Enter has nowhere to go.
                          e.preventDefault();
                        }
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

              <Show when={a().kind === 'signature' && a().signature}>
                {(art) => (
                  <div
                    class="annot-signature"
                    classList={{ 'is-selected': isSel() }}
                    title="Visual signature (not a certificate signature)"
                    style={{
                      left: `${cssX(a().rect.x)}px`,
                      top: `${cssY(a().rect.y, a().rect.h)}px`,
                      width: `${a().rect.w * props.zoom}px`,
                      height: `${a().rect.h * props.zoom}px`,
                      'pointer-events': toolState.active === 'select' ? 'auto' : 'none',
                    }}
                    onPointerDown={(e) => startMove(e, raw)}
                  >
                    <SignatureArtView art={art()} color={a().color} />
                  </div>
                )}
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
                data-annot-id={a().id}
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
              <Show
                when={
                  isSel() &&
                  toolState.active === 'select' &&
                  ['rect', 'textbox', 'image', 'signature', 'mark'].includes(a().kind)
                }
              >
                <For each={['nw', 'ne', 'sw', 'se'] as const}>
                  {(h) => (
                    <div
                      class={`annot-handle handle-${h}`}
                      data-annot-id={a().id}
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
                  data-annot-id={a().id}
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

/** The characters of a comb answer, one per cell, padded to fill the row. */
function combChars(text: string, cells: number): string[] {
  const chars = [...text.replace(/[\r\n]/g, '')].slice(0, cells);
  while (chars.length < cells) chars.push('');
  return chars;
}

/** Typing into a comb: a fixed-pitch font spaced to the cells, so each
 * character lands in its box as it is typed. */
function combEditorStyle(a: Annotation, zoom: number): Record<string, string> | undefined {
  if (!a.combCells) return undefined;
  const size = textStyleOf(a).fontSizePt;
  const pitch = a.rect.w / a.combCells;
  // Courier advances 0.6 em per character.
  const spacing = Math.max(0, pitch - size * 0.6);
  return {
    'font-family': textFontCss.Courier,
    'letter-spacing': `${spacing * zoom}px`,
    'padding-left': `${(spacing / 2) * zoom}px`,
    'white-space': 'nowrap',
    overflow: 'hidden',
  };
}

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

/** A signature keeps its proportions; everything else resizes freely. */
function resizeFor(a: Annotation, handle: 'nw' | 'ne' | 'sw' | 'se', p: PdfPoint): PdfRect {
  return a.kind === 'signature' && a.signature
    ? resizeKeepingAspect(a.rect, handle, p, a.rect.w / Math.max(1e-3, a.rect.h))
    : resizeRect(a.rect, handle, p);
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

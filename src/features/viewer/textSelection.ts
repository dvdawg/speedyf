/** The reader's own text selection gesture.
 *
 * A press on a page's text layer is taken from the browser: the caret goes
 * where `textHit` says the pointer is (the nearest line, then the nearest
 * character on it), and a drag moves the selection's focus the same way, on
 * whichever mounted page the pointer is over or nearest, keeping to the
 * column it started in when the pointer wanders into a gutter. The result is
 * still an ordinary DOM selection over the layer's spans, so copy, the Quote
 * and Comment actions and everything else that reads `document.getSelection()`
 * see what they always did.
 *
 * Double-click selects a word and a drag after it extends by words;
 * triple-click selects a paragraph; shift-click extends the selection there
 * is; a drag near the top or bottom of the viewer scrolls it.
 */
import { cssToPdf, type PageGeom } from '../../lib/coordinates/coords';
import {
  hitTest,
  wordAt,
  type PagePoint,
  type SelectedPiece,
  type TextModel,
  type TextPosition,
} from '../../lib/text/textHit';
import { paragraphRunRange } from '../../lib/text/paragraph';
import type { TextRunDto } from '../../types/engine';

export interface LayerEntry {
  layer: HTMLElement;
  page: number;
  model: () => TextModel;
  geom: () => PageGeom;
  zoom: () => number;
}

const layers = new Set<LayerEntry>();

/** Make a mounted text layer a place a drag can reach. Returns the undo. */
export function registerTextLayer(entry: LayerEntry): () => void {
  layers.add(entry);
  return () => layers.delete(entry);
}

/** Select the paragraph holding `span`, a run of `layer`, the way a drag
 * across it would: one DOM range from its first run to its last, so copy,
 * the Quote button and everything else that reads the selection see it. */
export function selectParagraphAt(
  layer: HTMLElement,
  span: Element,
  runs: readonly TextRunDto[]
): boolean {
  const spans = layer.children;
  const index = Array.prototype.indexOf.call(spans, span) as number;
  const range = paragraphRunRange(runs, index);
  const selection = document.getSelection();
  if (!range || !selection) return false;
  const first = spans[range[0]];
  const last = spans[range[1]];
  if (!first || !last) return false;
  const dom = document.createRange();
  dom.setStart(first.firstChild ?? first, 0);
  const end = last.lastChild ?? last;
  dom.setEnd(end, end.nodeType === Node.TEXT_NODE ? (end.textContent ?? '').length : 0);
  selection.removeAllRanges();
  selection.addRange(dom);
  return true;
}

/** How far a press may move before it is a drag and not a click. */
const DRAG_SLOP_PX = 3;
/** Within this distance of the viewer's top or bottom edge, a drag scrolls. */
const SCROLL_EDGE_PX = 28;
/** Fastest auto-scroll, in pixels per frame. */
const SCROLL_MAX_PX = 36;

interface Located {
  entry: LayerEntry;
  pos: TextPosition;
}

interface DomPoint {
  node: Node;
  offset: number;
}

function pagePoint(entry: LayerEntry, clientX: number, clientY: number): PagePoint {
  const box = entry.layer.getBoundingClientRect();
  const geom = entry.geom();
  const zoom = entry.zoom();
  const displayW = (geom.rotation % 180 === 0 ? geom.widthPt : geom.heightPt) * zoom;
  // The box is what is on screen; reading the scale off it keeps this right
  // while a zoom gesture is still settling.
  const scale = box.width > 0 && displayW > 0 ? box.width / displayW : 1;
  return cssToPdf({ x: clientX - box.left, y: clientY - box.top }, geom, zoom * scale);
}

/** The layer under the pointer, or the nearest one to it. */
function layerAt(root: Element, clientX: number, clientY: number): LayerEntry | null {
  let best: LayerEntry | null = null;
  let bestDistance = Infinity;
  for (const entry of layers) {
    if (!entry.layer.isConnected || !root.contains(entry.layer)) continue;
    const r = entry.layer.getBoundingClientRect();
    const dx = Math.max(0, r.left - clientX, clientX - r.right);
    const dy = Math.max(0, r.top - clientY, clientY - r.bottom);
    const distance = Math.hypot(dx, dy);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = entry;
    }
  }
  return best;
}

function domPoint(entry: LayerEntry, pos: TextPosition): DomPoint | null {
  const span = entry.layer.children[pos.run];
  if (!span) return null;
  const text = span.firstChild;
  if (!text) return { node: span, offset: 0 };
  return { node: text, offset: Math.min(pos.offset, text.textContent?.length ?? 0) };
}

/** Where a DOM selection boundary is, if it is in a registered layer. */
function locate(node: Node | null, offset: number): Located | null {
  if (!node) return null;
  const span = node.nodeType === Node.TEXT_NODE ? node.parentElement : (node as Element);
  const layer = span?.parentElement;
  if (!span || !layer) return null;
  for (const entry of layers) {
    if (entry.layer !== layer) continue;
    const run = Array.prototype.indexOf.call(layer.children, span) as number;
    if (run < 0) return null;
    return { entry, pos: { run, offset: node.nodeType === Node.TEXT_NODE ? offset : 0 } };
  }
  return null;
}

/** Document order of two positions: pages first, then runs, then offsets. */
function compare(a: Located, b: Located): number {
  return a.entry.page - b.entry.page || a.pos.run - b.pos.run || a.pos.offset - b.pos.offset;
}

function wordBounds(at: Located): [Located, Located] {
  const text = at.entry.model().runs[at.pos.run]?.text ?? '';
  const [start, end] = wordAt(text, at.pos.offset);
  return [
    { entry: at.entry, pos: { run: at.pos.run, offset: start } },
    { entry: at.entry, pos: { run: at.pos.run, offset: end } },
  ];
}

function select(anchor: DomPoint, focus: DomPoint): void {
  const selection = document.getSelection();
  if (!selection) return;
  if (
    selection.rangeCount > 0 &&
    selection.anchorNode === anchor.node &&
    selection.anchorOffset === anchor.offset &&
    selection.focusNode === focus.node &&
    selection.focusOffset === focus.offset
  ) {
    return;
  }
  selection.setBaseAndExtent(anchor.node, anchor.offset, focus.node, focus.offset);
}

function selectBetween(anchor: Located, focus: Located): void {
  const a = domPoint(anchor.entry, anchor.pos);
  const f = domPoint(focus.entry, focus.pos);
  if (a && f) select(a, f);
}

function hitAt(entry: LayerEntry, clientX: number, clientY: number, column?: Located) {
  const model = entry.model();
  const sameLayer = column && column.entry === entry;
  const line = sameLayer ? model.lines[model.lineOfRun[column.pos.run] ?? -1] : undefined;
  const pos = hitTest(
    model,
    pagePoint(entry, clientX, clientY),
    line ? { minX: line.reachLeft, maxX: line.reachRight } : undefined
  );
  return pos ? { entry, pos } : null;
}

/** Start the gesture for a press on `entry`'s layer. Returns whether the
 * press was taken (and its default prevented). */
export function pressTextLayer(entry: LayerEntry, event: MouseEvent): boolean {
  if (event.button !== 0 || event.ctrlKey || event.metaKey || event.altKey) return false;
  const at = hitAt(entry, event.clientX, event.clientY);
  if (!at) return false;
  event.preventDefault();
  const root = entry.layer.closest('.viewer') ?? document.body;
  // The press no longer reaches the browser, so neither does its focus move.
  if (root instanceof HTMLElement && root !== document.body) root.focus({ preventScroll: true });

  if (event.detail >= 3) {
    const span = entry.layer.children[at.pos.run];
    if (span) selectParagraphAt(entry.layer, span, entry.model().runs);
    return true;
  }

  const selection = document.getSelection();
  let anchor: Located = at;
  let word: [Located, Located] | null = null;
  if (event.detail === 2) {
    word = wordBounds(at);
    selectBetween(word[0], word[1]);
  } else if (event.shiftKey && selection && selection.rangeCount > 0) {
    anchor = locate(selection.anchorNode, selection.anchorOffset) ?? at;
    selectBetween(anchor, at);
  } else {
    selectBetween(at, at);
  }

  let moved = event.detail === 2 || event.shiftKey;
  let lastX = event.clientX;
  let lastY = event.clientY;
  let raf = 0;

  const update = () => {
    const target = layerAt(root, lastX, lastY);
    if (!target) return;
    const focus = hitAt(target, lastX, lastY, anchor);
    if (!focus) return;
    if (!anchor.entry.layer.isConnected) {
      // The page the drag began on was scrolled out of the DOM; carry on
      // from wherever the browser left that end of the selection.
      const now = document.getSelection();
      const kept = now ? locate(now.anchorNode, now.anchorOffset) : null;
      if (!kept) return;
      anchor = kept;
      word = null;
    }
    if (word) {
      const [start, end] = word;
      const [focusStart, focusEnd] = wordBounds(focus);
      if (compare(focus, start) >= 0) selectBetween(start, focusEnd);
      else selectBetween(end, focusStart);
      return;
    }
    selectBetween(anchor, focus);
  };

  const scrollStep = (): number => {
    if (!(root instanceof HTMLElement) || root === document.body) return 0;
    const r = root.getBoundingClientRect();
    let over = 0;
    if (lastY < r.top + SCROLL_EDGE_PX) over = lastY - (r.top + SCROLL_EDGE_PX);
    else if (lastY > r.bottom - SCROLL_EDGE_PX) over = lastY - (r.bottom - SCROLL_EDGE_PX);
    return Math.max(-SCROLL_MAX_PX, Math.min(SCROLL_MAX_PX, over / 2));
  };
  const tick = () => {
    raf = 0;
    const step = scrollStep();
    if (step === 0 || !(root instanceof HTMLElement)) return;
    const before = root.scrollTop;
    root.scrollTop = before + step;
    if (root.scrollTop === before) return;
    update();
    raf = requestAnimationFrame(tick);
  };

  const onMove = (e: MouseEvent) => {
    lastX = e.clientX;
    lastY = e.clientY;
    if (!moved) {
      if (Math.hypot(lastX - event.clientX, lastY - event.clientY) < DRAG_SLOP_PX) return;
      moved = true;
    }
    update();
    if (!raf && scrollStep() !== 0) raf = requestAnimationFrame(tick);
  };
  const onUp = () => {
    window.removeEventListener('mousemove', onMove, true);
    window.removeEventListener('mouseup', onUp, true);
    window.removeEventListener('blur', onUp);
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
  };
  window.addEventListener('mousemove', onMove, true);
  window.addEventListener('mouseup', onUp, true);
  window.addEventListener('blur', onUp);
  return true;
}

/** The stretch of each of `layer`'s runs the selection covers, in run
 * order. */
export function selectedPieces(selection: Selection | null, layer: Element): SelectedPiece[] {
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return [];
  const range = selection.getRangeAt(0);
  if (!range.intersectsNode(layer)) return [];
  const startsInText = range.startContainer.nodeType === Node.TEXT_NODE;
  const endsInText = range.endContainer.nodeType === Node.TEXT_NODE;
  const out: SelectedPiece[] = [];
  const spans = layer.children;
  for (let run = 0; run < spans.length; run += 1) {
    const span = spans[run]!;
    if (!range.intersectsNode(span)) continue;
    const length = span.textContent?.length ?? 0;
    const from = startsInText && span.contains(range.startContainer) ? range.startOffset : 0;
    const to = endsInText && span.contains(range.endContainer) ? range.endOffset : length;
    if (to > from) out.push({ run, from, to });
  }
  return out;
}

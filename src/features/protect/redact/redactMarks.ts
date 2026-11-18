/** Redaction marks: where a drag or a selection puts one, and adding them to
 * the document model. Pure geometry plus thin store calls.
 *
 * A mark over text snaps to whole glyphs - the edges the text layer already
 * carries per character - so what is marked is exactly what will be removed.
 * A drag that crosses no text (or is made with Alt held) marks the dragged
 * area itself. Page points, y up. */
import type { TextRunDto } from '../../../types/engine';
import type { Annotation, PdfQuad, PdfRect } from '../../../types/model';
import type { DocumentStore } from '../../document/documentStore';
import { newAnnotId } from '../../editor/editorActions';

/** A run counts when the drag covers this share of its height. */
const MIN_VERTICAL_COVER = 0.35;
/** A glyph counts when the drag covers this share of its width. */
const MIN_GLYPH_COVER = 0.5;

export function rectQuad(r: PdfRect): PdfQuad {
  return {
    p1: { x: r.x, y: r.y },
    p2: { x: r.x + r.w, y: r.y },
    p3: { x: r.x + r.w, y: r.y + r.h },
    p4: { x: r.x, y: r.y + r.h },
  };
}

export function unionRect(rects: readonly PdfRect[]): PdfRect | null {
  if (rects.length === 0) return null;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const r of rects) {
    x0 = Math.min(x0, r.x);
    y0 = Math.min(y0, r.y);
    x1 = Math.max(x1, r.x + r.w);
    y1 = Math.max(y1, r.y + r.h);
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** The glyph-snapped line boxes a dragged rectangle covers: per run, from the
 * first to the last glyph the drag covers at least half of. */
export function snappedTextRects(runs: readonly TextRunDto[], drag: PdfRect): PdfRect[] {
  const out: PdfRect[] = [];
  for (const run of runs) {
    const y0 = Math.max(drag.y, run.y);
    const y1 = Math.min(drag.y + drag.h, run.y + run.h);
    if (y1 - y0 < run.h * MIN_VERTICAL_COVER) continue;
    const edges = run.edges;
    if (!edges || edges.length < 2) {
      const x0 = Math.max(drag.x, run.x);
      const x1 = Math.min(drag.x + drag.w, run.x + run.w);
      if (x1 - x0 >= 1) out.push({ x: x0, y: run.y, w: x1 - x0, h: run.h });
      continue;
    }
    let first = -1;
    let last = -1;
    for (let i = 0; i + 1 < edges.length; i += 1) {
      const a = run.x + edges[i]!;
      const b = run.x + edges[i + 1]!;
      const width = b - a;
      const cover = Math.min(b, drag.x + drag.w) - Math.max(a, drag.x);
      if (width <= 0 ? a >= drag.x && a <= drag.x + drag.w : cover >= width * MIN_GLYPH_COVER) {
        if (first < 0) first = i;
        last = i;
      }
    }
    if (first < 0) continue;
    const x0 = run.x + edges[first]!;
    const x1 = run.x + edges[last + 1]!;
    if (x1 - x0 > 0.5) out.push({ x: x0, y: run.y, w: x1 - x0, h: run.h });
  }
  return out;
}

export interface MarkShape {
  rect: PdfRect;
  quads?: PdfQuad[];
}

/** What a drag marks: glyph-snapped text, or the area itself. */
export function markFromDrag(
  runs: readonly TextRunDto[],
  drag: PdfRect,
  forceArea: boolean
): MarkShape | null {
  if (!forceArea) {
    const lines = snappedTextRects(runs, drag);
    const union = unionRect(lines);
    if (union) return { rect: union, quads: lines.map(rectQuad) };
  }
  if (drag.w < 3 || drag.h < 3) return null;
  return { rect: drag };
}

/** A mark from line rectangles (a selection or a search match). */
export function markFromRects(rects: readonly PdfRect[]): MarkShape | null {
  const usable = rects.filter((r) => r.w > 0 && r.h > 0);
  const union = unionRect(usable);
  return union ? { rect: union, quads: usable.map(rectQuad) } : null;
}

export interface MarkStyle {
  color: string;
  overlay: string;
}

export function redactAnnotation(pageId: string, shape: MarkShape, style: MarkStyle): Annotation {
  const overlay = style.overlay.trim();
  return {
    id: newAnnotId(),
    pageId,
    kind: 'redact',
    rect: shape.rect,
    color: style.color,
    opacity: 1,
    ...(shape.quads ? { quads: shape.quads } : {}),
    ...(overlay ? { text: overlay } : {}),
  };
}

/** Add several marks; each is its own undo step. */
export function addMarks(
  store: DocumentStore,
  marks: readonly { pageId: string; shape: MarkShape }[],
  style: MarkStyle
): number {
  for (const m of marks) {
    store.apply({ type: 'addAnnot', annot: redactAnnotation(m.pageId, m.shape, style) });
  }
  return marks.length;
}

/** How many unapplied marks a document holds. */
export function markCount(store: DocumentStore): number {
  let n = 0;
  for (const list of Object.values(store.state.annotations)) {
    for (const a of list) if (a.kind === 'redact') n += 1;
  }
  return n;
}

/** Black or white overlay text, whichever reads on the fill. */
export function inkOn(fill: string): string {
  const v = Number.parseInt(fill.replace('#', ''), 16);
  if (Number.isNaN(v)) return 'white';
  const r = (v >> 16) & 0xff;
  const g = (v >> 8) & 0xff;
  const b = v & 0xff;
  return 0.299 * r + 0.587 * g + 0.114 * b < 128 ? 'white' : 'black';
}

/** The boxes a mark covers: one per quad, else its rectangle. */
export function markBoxes(a: Annotation): PdfRect[] {
  if (a.quads && a.quads.length > 0) {
    return a.quads.map((q) => {
      const xs = [q.p1.x, q.p2.x, q.p3.x, q.p4.x];
      const ys = [q.p1.y, q.p2.y, q.p3.y, q.p4.y];
      const x = Math.min(...xs);
      const y = Math.min(...ys);
      return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
    });
  }
  return [a.rect];
}

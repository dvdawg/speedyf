/** Pure geometry for visual signatures: drawing a stroke, turning a drawing
 * into artwork, and sizing a signature on a page. No DOM, no stores. */
import type { PdfPoint, PdfRect, SignatureArt } from '../../types/model';

export type SignatureRole = 'signature' | 'initials';

const fmt = (n: number) => Number(n.toFixed(2)).toString();

/** SVG path data for one drawn stroke: a quadratic through each captured
 * point, from the midpoint before it to the midpoint after it, so a pointer
 * trail has no corners. The engine draws the same curve when it saves
 * (`stroke_segs` in src-tauri/src/engine/signature.rs). */
export function strokeD(points: readonly PdfPoint[]): string {
  const first = points[0];
  if (!first) return '';
  if (points.length === 1)
    return `M${fmt(first.x)} ${fmt(first.y)}L${fmt(first.x + 0.01)} ${fmt(first.y)}`;
  const parts = [`M${fmt(first.x)} ${fmt(first.y)}`];
  if (points.length === 2) {
    const last = points[1]!;
    parts.push(`L${fmt(last.x)} ${fmt(last.y)}`);
    return parts.join('');
  }
  for (let i = 1; i < points.length - 1; i++) {
    const q = points[i]!;
    const next = points[i + 1]!;
    const end =
      i + 1 === points.length - 1 ? next : { x: (q.x + next.x) / 2, y: (q.y + next.y) / 2 };
    parts.push(`Q${fmt(q.x)} ${fmt(q.y)} ${fmt(end.x)} ${fmt(end.y)}`);
  }
  return parts.join('');
}

/** Minimum travel, in pad units, before another point is kept. */
const MIN_STEP = 1.5;
/** How far each kept point moves toward the pointer: lower is smoother. */
const FOLLOW = 0.5;

/** Extend a stroke being drawn with a raw pointer position.
 *
 * Each point follows the pointer only part of the way, which takes out the
 * jitter of a trackpad and the stair-steps of a mouse, and points closer than
 * a pixel or two to the last one are dropped. Returns whether one was kept. */
export function extendStroke(stroke: PdfPoint[], raw: PdfPoint): boolean {
  const last = stroke[stroke.length - 1];
  if (!last) {
    stroke.push(raw);
    return true;
  }
  const next = { x: last.x + (raw.x - last.x) * FOLLOW, y: last.y + (raw.y - last.y) * FOLLOW };
  if (Math.hypot(next.x - last.x, next.y - last.y) < MIN_STEP / 2) return false;
  stroke.push(next);
  return true;
}

/** End a stroke exactly where the pointer was lifted, since smoothing lags. */
export function finishStroke(stroke: PdfPoint[], raw: PdfPoint): void {
  const last = stroke[stroke.length - 1];
  if (!last || Math.hypot(raw.x - last.x, raw.y - last.y) >= 0.5) stroke.push(raw);
}

/** A drawing pad's strokes as artwork: cropped to the ink, with room for
 * the pen around it. Null when there is nothing drawn. */
export function drawnArt(strokes: readonly PdfPoint[][], width: number): SignatureArt | null {
  const points = strokes.flat();
  if (points.length === 0) return null;
  const x0 = Math.min(...points.map((p) => p.x));
  const y0 = Math.min(...points.map((p) => p.y));
  const x1 = Math.max(...points.map((p) => p.x));
  const y1 = Math.max(...points.map((p) => p.y));
  const pad = width;
  return {
    type: 'drawn',
    w: Math.max(1, x1 - x0) + 2 * pad,
    h: Math.max(1, y1 - y0) + 2 * pad,
    width,
    strokes: strokes
      .filter((s) => s.length > 0)
      .map((s) => s.map((p) => ({ x: p.x - x0 + pad, y: p.y - y0 + pad }))),
  };
}

export const aspectOf = (art: SignatureArt) => art.w / Math.max(1e-3, art.h);

/** The largest a placed mark starts at, in points. A signature line is about
 * two inches long and a third of an inch tall; initials fit a small box. */
const START = {
  signature: { w: 150, h: 42 },
  initials: { w: 60, h: 30 },
} as const;

/** Where a signature lands when placed with a click at `at`: centred on it,
 * at a sensible size, kept on the page. */
export function placementRect(
  art: SignatureArt,
  role: SignatureRole,
  at: PdfPoint,
  page: { widthPt: number; heightPt: number }
): PdfRect {
  const aspect = aspectOf(art);
  const box = START[role];
  let w = box.w;
  let h = w / aspect;
  if (h > box.h) {
    h = box.h;
    w = h * aspect;
  }
  return {
    x: clamp(at.x - w / 2, 0, Math.max(0, page.widthPt - w)),
    y: clamp(at.y - h / 2, 0, Math.max(0, page.heightPt - h)),
    w,
    h,
  };
}

/** A signature fitted inside a form field: as large as the field allows
 * with a little margin, centred, never stretched. */
export function fitIntoField(art: SignatureArt, field: PdfRect): PdfRect {
  const inset = Math.min(3, field.h * 0.1, field.w * 0.1);
  const room = { w: field.w - 2 * inset, h: field.h - 2 * inset };
  const aspect = aspectOf(art);
  let w = room.w;
  let h = w / aspect;
  if (h > room.h) {
    h = room.h;
    w = h * aspect;
  }
  return { x: field.x + (field.w - w) / 2, y: field.y + (field.h - h) / 2, w, h };
}

/** Smallest width a signature can be resized to, in points. */
const MIN_W = 12;

/** Resize from a corner handle keeping the artwork's proportions: the
 * opposite corner stays put and the dragged one follows the pointer along
 * whichever axis asks for more room. */
export function resizeKeepingAspect(
  r: PdfRect,
  handle: 'nw' | 'ne' | 'sw' | 'se',
  p: PdfPoint,
  aspect: number
): PdfRect {
  // PDF space is y-up: "n" is the top edge, at y + h.
  const west = handle === 'nw' || handle === 'sw';
  const north = handle === 'nw' || handle === 'ne';
  const anchor = { x: west ? r.x + r.w : r.x, y: north ? r.y : r.y + r.h };
  const dx = west ? anchor.x - p.x : p.x - anchor.x;
  const dy = north ? p.y - anchor.y : anchor.y - p.y;
  const w = Math.max(MIN_W, dx, dy * aspect);
  const h = w / aspect;
  return {
    x: west ? anchor.x - w : anchor.x,
    y: north ? anchor.y : anchor.y - h,
    w,
    h,
  };
}

function clamp(v: number, lo: number, hi: number) {
  return Math.min(hi, Math.max(lo, v));
}

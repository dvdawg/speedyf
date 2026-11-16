/** How the citation graph fills its canvas. Pure, so the view only applies
 * it and the tests can measure it.
 *
 * The layout comes back spaced for its labels at zoom 1, which for a few
 * nodes is a small cluster in a large canvas. Zooming in would fill it, but
 * zoom also drives the label quota; instead the positions are spread: the
 * distances between node centres are multiplied, x and y each by its own
 * factor, while dots, strokes and labels keep their size. A spread is at
 * least 1 on each axis (a large graph that already overflows is left to
 * zoom out as before), at most MAX_SPREAD (two nodes are not flung to the
 * corners), and the two factors differ by at most MAX_SKEW (a cluster may
 * be stretched to the canvas's shape, not distorted out of its own).
 *
 * Positions stay in layout units everywhere they are kept (the layout, the
 * dragged positions in graphPositions.ts); the spread is applied on the way
 * to the screen and undone on the way back from a drag, so a resize changes
 * the spread and nothing stored. */
import type { Point } from './graphLayout';

/** At most this many times the layout's distances: edges stay readable. */
export const MAX_SPREAD = 4;
/** The larger factor is at most this many times the smaller. */
export const MAX_SKEW = 2.5;

export interface Spread {
  x: number;
  y: number;
}

export const NO_SPREAD: Spread = { x: 1, y: 1 };

export interface Bounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** How far drawings reach past the outermost centres, in pixels. */
export interface Reach {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Transform {
  k: number;
  x: number;
  y: number;
}

export interface Padding {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

/** The extent of some points; null for none. */
export function boundsOf(points: Iterable<Point>): Bounds | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of points) {
    minX = Math.min(minX, p.x);
    maxX = Math.max(maxX, p.x);
    minY = Math.min(minY, p.y);
    maxY = Math.max(maxY, p.y);
  }
  return minX === Infinity ? null : { minX, minY, maxX, maxY };
}

const clamp = (value: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, value));

/** The spread that makes the centres in `centres`, plus what their
 * drawings reach past them, fill `room`, within the limits above. An axis
 * with no extent (a single row of nodes) takes the other's factor, so the
 * pair is never skewed for nothing. */
export function spreadFor(centres: Bounds | null, reach: Reach, room: Size): Spread {
  if (!centres) return NO_SPREAD;
  const spanX = centres.maxX - centres.minX;
  const spanY = centres.maxY - centres.minY;
  const roomX = room.width - reach.left - reach.right;
  const roomY = room.height - reach.top - reach.bottom;
  const raw = (span: number, space: number) =>
    span < 1 ? null : clamp(space / span, 1, MAX_SPREAD);
  let x = raw(spanX, roomX);
  let y = raw(spanY, roomY);
  if (x === null && y === null) return NO_SPREAD;
  x ??= y!;
  y ??= x;
  if (x > y * MAX_SKEW) x = y * MAX_SKEW;
  if (y > x * MAX_SKEW) y = x * MAX_SKEW;
  return { x, y };
}

export const spreadPoint = (p: Point, s: Spread): Point => ({ x: p.x * s.x, y: p.y * s.y });

export const unspreadPoint = (p: Point, s: Spread): Point => ({ x: p.x / s.x, y: p.y / s.y });

/** The view that frames `extent` (drawings included) inside the canvas
 * less its padding, centred, never past zoom 1: zoom 1 is where the
 * labels are spaced, and the spread is what fills a large canvas. */
export function fitTransform(
  extent: Bounds,
  canvas: Size,
  pad: Padding,
  minZoom: number
): Transform {
  const width = Math.max(1, extent.maxX - extent.minX);
  const height = Math.max(1, extent.maxY - extent.minY);
  const availX = Math.max(1, canvas.width - pad.left - pad.right);
  const availY = Math.max(1, canvas.height - pad.top - pad.bottom);
  const k = clamp(Math.min(availX / width, availY / height), minZoom, 1);
  return {
    k,
    x: pad.left + (availX - width * k) / 2 - extent.minX * k,
    y: pad.top + (availY - height * k) / 2 - extent.minY * k,
  };
}

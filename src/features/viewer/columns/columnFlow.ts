/** Column zoom's arithmetic, without a DOM: which segment a point is in,
 * the zoom and scroll that fit one, and when a scroll past its end means
 * "go on to the next one".
 *
 * Segments arrive from the engine in display page space, y-up, as
 * `[x, y_bottom, w, h]` (src-tauri/src/engine/columns.rs). A page the engine
 * has no confident reading for is read as one segment covering the whole
 * page, so the flow never stops at a figure page or a blank one. */
import type { ColumnLayout, ReadingSegment } from '../../../types/engine';
import type { CssRect, PageGeom } from '../../../lib/coordinates/coords';
import { pdfRectToCssRect } from '../../../lib/coordinates/coords';
import type { Layout } from '../../../lib/coordinates/layout';
import { clampZoom } from '../../../stores/viewportStore';

/** Breathing room either side of the fitted segment, CSS px. */
export const MARGIN_X = 14;
/** And above and below it, so its first and last lines are not flush. */
export const MARGIN_Y = 16;
/** A new segment within this share of the current zoom keeps the zoom: two
 * columns a point apart in width must not make the page twitch. */
export const ZOOM_STICKINESS = 0.06;

/** One stop of the reading flow on a page. */
export interface FlowSegment {
  kind: ReadingSegment['kind'] | 'page';
  rect: [number, number, number, number];
}

/** How a page is read in column zoom. `columns` is false for a page with no
 * confident reading, which is read as a whole page, at its width. */
export interface PageFlow {
  kind: ColumnLayout['kind'];
  segments: FlowSegment[];
}

/** A page's flow: its segments, or the whole page when the engine has none
 * or the page is turned (columns are read upright only). */
export function pageFlow(
  layout: ColumnLayout | null,
  geom: PageGeom,
  widthPt: number,
  heightPt: number
): PageFlow {
  if (!layout || geom.rotation !== 0 || layout.kind === 'none' || layout.segments.length === 0) {
    return { kind: 'none', segments: [{ kind: 'page', rect: [0, 0, widthPt, heightPt] }] };
  }
  return {
    kind: layout.kind,
    segments: layout.segments.map((s) => ({ kind: s.kind, rect: s.rect })),
  };
}

/** A segment's box in content CSS px at the layout's zoom. */
export function segmentBox(
  seg: FlowSegment,
  page: number,
  geom: PageGeom,
  layout: Layout,
  zoom: number
): CssRect {
  const [x, y, w, h] = seg.rect;
  const r = pdfRectToCssRect({ x, y, w, h }, geom, zoom);
  return { x: (layout.lefts[page] ?? 0) + r.x, y: (layout.tops[page] ?? 0) + r.y, w: r.w, h: r.h };
}

/** The zoom that fits a segment's width to the view, within the zoom
 * limits; `current` is kept when it is close enough and still fits. */
export function fitZoom(
  seg: FlowSegment,
  geom: PageGeom,
  containerW: number,
  current: number | null
): number {
  const widthAt1 = pdfRectToCssRect(
    { x: seg.rect[0], y: seg.rect[1], w: seg.rect[2], h: seg.rect[3] },
    geom,
    1
  ).w;
  if (widthAt1 <= 0) return clampZoom(current ?? 1);
  const target = clampZoom((containerW - 2 * MARGIN_X) / widthAt1);
  if (
    current !== null &&
    Math.abs(target - current) / current < ZOOM_STICKINESS &&
    widthAt1 * current <= containerW - MARGIN_X
  ) {
    return current;
  }
  return target;
}

/** The scroll tops that keep the view inside a segment: `lo` shows its top,
 * `hi` its bottom. A segment shorter than the view has `lo === hi`. */
export function scrollRange(box: CssRect, containerH: number): { lo: number; hi: number } {
  const lo = Math.max(0, box.y - MARGIN_Y);
  const hi = Math.max(lo, box.y + box.h + MARGIN_Y - containerH);
  return { lo, hi };
}

/** The scroll left that centres a segment, within the content's width. */
export function centredLeft(box: CssRect, containerW: number, totalW: number): number {
  const left = box.x + box.w / 2 - containerW / 2;
  return Math.min(Math.max(0, left), Math.max(0, totalW - containerW));
}

/** Which segment of a page a point is in, in display page points (y-up).
 * With no x (a destination that names only a height), the first segment in
 * reading order that spans that height. A point between segments goes to
 * the nearest one, preferring what comes next in reading order. */
export function segmentAt(
  segments: readonly FlowSegment[],
  x: number | null,
  y: number,
  slack = 4
): number {
  if (segments.length === 0) return -1;
  const spans = (s: FlowSegment) => y >= s.rect[1] - slack && y <= s.rect[1] + s.rect[3] + slack;
  const across = (s: FlowSegment) =>
    x === null || (x >= s.rect[0] - slack && x <= s.rect[0] + s.rect[2] + slack);
  const hit = segments.findIndex((s) => spans(s) && across(s));
  if (hit >= 0) return hit;
  if (x !== null) {
    // Same height, nearest across: a point in the gutter.
    let best = -1;
    let bestD = Infinity;
    segments.forEach((s, i) => {
      if (!spans(s)) return;
      const d = Math.max(s.rect[0] - x, 0, x - (s.rect[0] + s.rect[2]));
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    });
    if (best >= 0) return best;
  }
  // Between bands: the first segment that starts below the point (y-up, so
  // "below" is a smaller top), else the last one.
  const next = segments.findIndex((s) => s.rect[1] + s.rect[3] <= y + slack);
  return next >= 0 ? next : segments.length - 1;
}

/** Whether a point is inside a column (not a full-width block): where a
 * double-click on blank space zooms. */
export function columnAt(segments: readonly FlowSegment[], x: number, y: number): number {
  return segments.findIndex(
    (s) =>
      s.kind === 'column' &&
      x >= s.rect[0] - 2 &&
      x <= s.rect[0] + s.rect[2] + 2 &&
      y >= s.rect[1] - 2 &&
      y <= s.rect[1] + s.rect[3] + 2
  );
}

// ---------- scrolling past the end ----------

/** Wheel events further apart than this belong to separate gestures. A
 * trackpad's momentum tail arrives every frame, well inside it. */
export const GESTURE_GAP_MS = 220;
/** How far a fresh gesture must push past the end before the flow moves on:
 * about one mouse-wheel notch, a short deliberate swipe. */
export const PUSH_PX = 90;

/** Per-view state of the overscroll rule. */
export interface EdgeState {
  lastAt: number;
  dir: -1 | 0 | 1;
  /** The current gesture scrolled into the edge itself (and so cannot
   * also carry on past it: that is momentum, not intent). */
  arrived: boolean;
  pushed: number;
}

export function edgeState(): EdgeState {
  return { lastAt: -Infinity, dir: 0, arrived: false, pushed: 0 };
}

/** One wheel step inside a segment's scroll range. Returns where to scroll,
 * and -1/1 when this step asks for the previous/next segment.
 *
 * Moving on takes a gesture that *starts* at the end and pushes past it by
 * `PUSH_PX`: the swipe that reaches the end of a column, and its momentum,
 * stop there. Reversing direction, or pausing, starts a new gesture. */
export function wheelStep(
  s: EdgeState,
  now: number,
  dy: number,
  top: number,
  lo: number,
  hi: number
): { top: number; advance: -1 | 0 | 1 } {
  if (dy === 0) return { top, advance: 0 };
  const dir: 1 | -1 = dy > 0 ? 1 : -1;
  if (now - s.lastAt > GESTURE_GAP_MS || dir !== s.dir) {
    s.dir = dir;
    s.arrived = false;
    s.pushed = 0;
  }
  s.lastAt = now;
  const edge = dir > 0 ? hi : lo;
  const atEdge = dir > 0 ? top >= hi - 0.5 : top <= lo + 0.5;
  if (!atEdge) {
    const next = Math.min(hi, Math.max(lo, top + dy));
    if (next === edge) s.arrived = true;
    return { top: next, advance: 0 };
  }
  if (s.arrived) return { top: edge, advance: 0 };
  s.pushed += Math.abs(dy);
  if (s.pushed >= PUSH_PX) {
    // The rest of this gesture scrolls the next segment, but cannot skip it.
    s.pushed = 0;
    s.arrived = true;
    return { top: edge, advance: dir };
  }
  return { top: edge, advance: 0 };
}

/** One key step (arrows, Space): scrolls within the segment; at its end, a
 * fresh press moves on, a held key's repeats do not. */
export function keyStep(
  amount: number,
  repeat: boolean,
  top: number,
  lo: number,
  hi: number
): { top: number; advance: -1 | 0 | 1 } {
  const dir: 1 | -1 = amount > 0 ? 1 : -1;
  const atEdge = dir > 0 ? top >= hi - 0.5 : top <= lo + 0.5;
  if (atEdge) return { top, advance: repeat ? 0 : dir };
  return { top: Math.min(hi, Math.max(lo, top + amount)), advance: 0 };
}

/** A wheel delta in CSS px, whatever unit the device reported it in. */
export function wheelPixels(delta: number, mode: number, viewportH: number): number {
  return mode === 1 ? delta * 16 : mode === 2 ? delta * Math.max(1, viewportH) : delta;
}

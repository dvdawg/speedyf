/** Where a comment is on its page: quads from a selection, its bounding box,
 * and the reading order comments are numbered in. Pure. */
import type { Anchor, MarkupComment, Quad, Rect } from './types';

/** A rectangle `[x, y, w, h]` in page points from the displayed page's
 * top-left corner, as `selectedGeometry` reports them. */
export type TopLeftRect = readonly [number, number, number, number];

const round = (v: number) => Math.round(v * 100) / 100;

/** One quad per line of a selection: rectangles on one line (vertical
 * overlap over half the shorter) are joined, then flipped y-up on a page
 * `pageHeight` points tall. */
export function quadsFromRects(rects: readonly TopLeftRect[], pageHeight: number): Quad[] {
  const lines: [number, number, number, number][] = [];
  const sorted = [...rects]
    .filter((r) => r[2] > 0 && r[3] > 0)
    .sort((a, b) => a[1] - b[1] || a[0] - b[0]);
  for (const r of sorted) {
    const last = lines.find((l) => {
      const overlap = Math.min(l[1] + l[3], r[1] + r[3]) - Math.max(l[1], r[1]);
      return overlap > 0.5 * Math.min(l[3], r[3]);
    });
    if (last) {
      const left = Math.min(last[0], r[0]);
      const top = Math.min(last[1], r[1]);
      const right = Math.max(last[0] + last[2], r[0] + r[2]);
      const bottom = Math.max(last[1] + last[3], r[1] + r[3]);
      last.splice(0, 4, left, top, right - left, bottom - top);
    } else {
      lines.push([r[0], r[1], r[2], r[3]]);
    }
  }
  lines.sort((a, b) => a[1] - b[1] || a[0] - b[0]);
  return lines.map(([x, top, w, h]) => {
    const yTop = round(pageHeight - top);
    const yBottom = round(pageHeight - top - h);
    return {
      x1: round(x),
      y1: yTop,
      x2: round(x + w),
      y2: yTop,
      x3: round(x),
      y3: yBottom,
      x4: round(x + w),
      y4: yBottom,
    };
  });
}

/** A top-left box `[x, y, w, h]` as a y-up rect on a page `pageHeight` tall. */
export function rectFromBox(box: TopLeftRect, pageHeight: number): Rect {
  return {
    x: round(box[0]),
    y: round(pageHeight - box[1] - box[3]),
    w: round(box[2]),
    h: round(box[3]),
  };
}

/** The y-up bounding rect of a quad. */
export function quadRect(q: Quad): Rect {
  const xs = [q.x1, q.x2, q.x3, q.x4];
  const ys = [q.y1, q.y2, q.y3, q.y4];
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}

/** The anchor's bounding rect, y-up. */
export function anchorBounds(anchor: Anchor): Rect {
  if (anchor.rect) return anchor.rect;
  const rects = (anchor.quads ?? []).map(quadRect);
  if (rects.length === 0) return { x: 0, y: 0, w: 0, h: 0 };
  const x0 = Math.min(...rects.map((r) => r.x));
  const y0 = Math.min(...rects.map((r) => r.y));
  const x1 = Math.max(...rects.map((r) => r.x + r.w));
  const y1 = Math.max(...rects.map((r) => r.y + r.h));
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** Where reading meets the anchor: the top-left of its first line (or of
 * its box), y-up. */
export function anchorStart(anchor: Anchor): { x: number; y: number } {
  const first = anchor.quads?.[0];
  if (first) {
    const r = quadRect(first);
    return { x: r.x, y: r.y + r.h };
  }
  const b = anchorBounds(anchor);
  return { x: b.x, y: b.y + b.h };
}

/** The anchor's top edge as points down from the top of a page
 * `pageHeight` tall: where a margin note aims and where a link lands. */
export function anchorTopFromTop(anchor: Anchor, pageHeight: number): number {
  return Math.max(0, pageHeight - anchorStart(anchor).y);
}

/** Two tops this close are one line. */
const SAME_LINE_PT = 3;

/** Reading order: page, then down the page, then across. */
export function compareReading(a: Anchor, b: Anchor): number {
  if (a.page !== b.page) return a.page - b.page;
  const sa = anchorStart(a);
  const sb = anchorStart(b);
  if (Math.abs(sa.y - sb.y) > SAME_LINE_PT) return sb.y - sa.y;
  return sa.x - sb.x;
}

/** Comments in reading order (ties by creation), each with its number. */
export function numberComments(
  comments: readonly MarkupComment[]
): { comment: MarkupComment; number: number }[] {
  return [...comments]
    .sort((a, b) => compareReading(a.anchor, b.anchor) || a.created - b.created)
    .map((comment, i) => ({ comment, number: i + 1 }));
}

/** A y-up rect as `[x, y, w, h]`, the shape `revealPlace` takes. */
export function revealRects(anchor: Anchor): [number, number, number, number][] {
  if (anchor.rect) return [[anchor.rect.x, anchor.rect.y, anchor.rect.w, anchor.rect.h]];
  return (anchor.quads ?? []).map((q) => {
    const r = quadRect(q);
    return [r.x, r.y, r.w, r.h];
  });
}

/** Vertical continuous-scroll layout of pages at a zoom level.
 * Pure math — no DOM. All values are CSS px in scroll-content space. */
import type { PageGeom } from './coords';
import { pageCssDims } from './coords';

export interface LayoutOptions {
  /** vertical gap between pages */
  gap: number;
  /** padding around the whole stack */
  padding: number;
  /** current scroll container client width */
  containerW: number;
}

export interface Layout {
  tops: number[];
  lefts: number[];
  widths: number[];
  heights: number[];
  totalH: number;
  totalW: number;
}

export function layoutPages(pages: readonly PageGeom[], zoom: number, o: LayoutOptions): Layout {
  const tops: number[] = [];
  const lefts: number[] = [];
  const widths: number[] = [];
  const heights: number[] = [];
  let y = o.padding;
  let maxW = 0;
  for (const p of pages) {
    const { w, h } = pageCssDims(p, zoom);
    tops.push(y);
    widths.push(w);
    heights.push(h);
    lefts.push(Math.max(o.padding, (o.containerW - w) / 2));
    y += h + o.gap;
    maxW = Math.max(maxW, w);
  }
  const contentBottom = pages.length > 0 ? y - o.gap : y;
  return {
    tops,
    lefts,
    widths,
    heights,
    totalH: contentBottom + o.padding,
    totalW: Math.max(o.containerW, maxW + 2 * o.padding),
  };
}

/** [first, last] page indexes intersecting the viewport ± overscan (inclusive).
 * Returns [0, -1] when there are no pages. */
export function visibleRange(
  layout: Layout,
  scrollTop: number,
  viewH: number,
  overscan: number
): [number, number] {
  const n = layout.tops.length;
  if (n === 0) return [0, -1];
  const top = scrollTop - overscan;
  const bottom = scrollTop + viewH + overscan;

  // first page whose bottom edge is below `top`
  let lo = 0;
  let hi = n - 1;
  let first = n - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (layout.tops[mid]! + layout.heights[mid]! > top) {
      first = mid;
      hi = mid - 1;
    } else {
      lo = mid + 1;
    }
  }

  // last page whose top edge is above `bottom`
  lo = 0;
  hi = n - 1;
  let last = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (layout.tops[mid]! < bottom) {
      last = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }

  if (last < first) last = first;
  return [first, last];
}

/** Index of the page containing content-space y (gaps snap to the following
 * page; out-of-range clamps to the first/last page). */
export function pageIndexAt(layout: Layout, y: number): number {
  const n = layout.tops.length;
  if (n === 0) return 0;
  let lo = 0;
  let hi = n - 1;
  let idx = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (layout.tops[mid]! <= y) {
      idx = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  if (y > layout.tops[idx]! + layout.heights[idx]!) idx = Math.min(idx + 1, n - 1);
  return idx;
}

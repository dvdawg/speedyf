/** What a comment's card shows of the page: the anchored words cropped
 * from the page as the engine renders it, so a formula looks exactly as it
 * is printed whatever a text reading of it would make of it. Pure: where
 * the crop is and at what scale; `QuoteCrop` asks for the pixels.
 *
 * Page points, y up, on the displayed page (the space anchors are in). */
import { bucketForScale } from '../../lib/coordinates/coords';
import type { TileRect } from '../../lib/coordinates/coords';
import type { Anchor } from './types';

/** Room around the words, in points. */
const PAD_PT = 3;
/** Never shown larger than this many CSS pixels a point: a short quote
 * would otherwise be blown up to the card's width. */
const MAX_CSS_SCALE = 1.6;
/** Tallest crop asked for, in points: the card shows the top of anything
 * taller, faded out. */
const MAX_HEIGHT_PT = 360;
/** The engine renders a preview of at most this many pixels a side. */
const MAX_TILE_PX = 4096;

export interface CropSpec {
  /** what to render, in device pixels at `scaleMilli` */
  tile: TileRect;
  scaleMilli: number;
  /** the crop's CSS size */
  width: number;
  height: number;
}

/** The anchored area: the quads' union (or the region), padded, on the page. */
export function anchorBox(
  anchor: Anchor,
  page: { widthPt: number; heightPt: number }
): { x: number; y: number; w: number; h: number } | null {
  let left = Infinity;
  let bottom = Infinity;
  let right = -Infinity;
  let top = -Infinity;
  for (const q of anchor.quads ?? []) {
    left = Math.min(left, q.x1, q.x2, q.x3, q.x4);
    right = Math.max(right, q.x1, q.x2, q.x3, q.x4);
    bottom = Math.min(bottom, q.y1, q.y2, q.y3, q.y4);
    top = Math.max(top, q.y1, q.y2, q.y3, q.y4);
  }
  if (anchor.rect) {
    const r = anchor.rect;
    left = Math.min(left, r.x);
    right = Math.max(right, r.x + r.w);
    bottom = Math.min(bottom, r.y);
    top = Math.max(top, r.y + r.h);
  }
  if (!(right > left && top > bottom)) return null;
  left = Math.max(0, left - PAD_PT);
  bottom = Math.max(0, bottom - PAD_PT);
  right = Math.min(page.widthPt, right + PAD_PT);
  top = Math.min(page.heightPt, top + PAD_PT);
  if (!(right > left && top > bottom)) return null;
  // The top of a long passage is what the card shows.
  bottom = Math.max(bottom, top - MAX_HEIGHT_PT);
  return { x: left, y: bottom, w: right - left, h: top - bottom };
}

/** The crop of `anchor` for a card `widthPx` CSS pixels wide on a screen of
 * `dpr` device pixels a CSS pixel. Null when there is nothing to crop. */
export function cropSpec(
  anchor: Anchor,
  page: { widthPt: number; heightPt: number },
  widthPx: number,
  dpr: number
): CropSpec | null {
  const box = anchorBox(anchor, page);
  if (!box) return null;
  const css = Math.min(MAX_CSS_SCALE, Math.max(0.1, widthPx) / box.w);
  // Rendered at one of the engine's fixed scales, at or above what the
  // screen needs, and no larger than a preview may be.
  let scale = bucketForScale(css * Math.max(1, dpr));
  while (scale > 0.25 && Math.max(box.w, box.h) * scale > MAX_TILE_PX) {
    scale = bucketForScale(scale / 2);
  }
  const x = Math.max(0, Math.floor(box.x * scale));
  const y = Math.max(0, Math.floor((page.heightPt - box.y - box.h) * scale));
  return {
    tile: {
      x,
      y,
      w: Math.max(1, Math.min(MAX_TILE_PX, Math.ceil(box.w * scale))),
      h: Math.max(1, Math.min(MAX_TILE_PX, Math.ceil(box.h * scale))),
    },
    scaleMilli: Math.round(scale * 1000),
    width: box.w * css,
    height: box.h * css,
  };
}

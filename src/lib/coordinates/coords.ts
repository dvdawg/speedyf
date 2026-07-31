/**
 * Coordinate conversions between:
 *  - PDF page space  : points, origin bottom-left of the crop box, y-up, unrotated
 *  - page CSS space  : CSS px, origin top-left of the ROTATED page element, y-down
 *  - raw PDF user space: page space + crop-box origin offset
 *  - device space    : physical pixels (CSS px × devicePixelRatio), used for tiles
 *
 * Every conversion in the app goes through this module — UI components never
 * do their own rotation/flip math.
 */
import type { PdfPoint, PdfRect, Rotation } from '../../types/model';
import type { Layout } from './layout';
import { pageIndexAt } from './layout';

export interface PageGeom {
  widthPt: number;
  heightPt: number;
  /** total displayed rotation: (baseRotation + userRotation + viewRotation) % 360 */
  rotation: Rotation;
}

export interface CssPoint {
  x: number;
  y: number;
}

export interface CssRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Quantized raster scales (device px per PDF point ÷ 72dpi unit). Rendering only
 * ever happens at these buckets so zoom gestures cannot fill the cache with
 * one-off rasters. */
export const RENDER_SCALE_BUCKETS: readonly number[] = [
  0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4, 6, 8, 12,
];

const EPS = 1e-9;

/** Smallest bucket ≥ scale (clamped to the largest bucket). */
export function bucketForScale(scale: number): number {
  for (const b of RENDER_SCALE_BUCKETS) {
    if (b >= scale - EPS) return b;
  }
  return RENDER_SCALE_BUCKETS[RENDER_SCALE_BUCKETS.length - 1]!;
}

/** CSS size of the rotated page at a zoom level. */
export function pageCssDims(page: PageGeom, zoom: number): { w: number; h: number } {
  const swap = page.rotation === 90 || page.rotation === 270;
  const w = (swap ? page.heightPt : page.widthPt) * zoom;
  const h = (swap ? page.widthPt : page.heightPt) * zoom;
  return { w, h };
}

/** PDF page space (y-up, unrotated) → CSS point inside the rotated page element. */
export function pdfToPageCss(pt: PdfPoint, page: PageGeom, zoom: number): CssPoint {
  const W = page.widthPt * zoom;
  const H = page.heightPt * zoom;
  // Unrotated CSS (top-left origin, y-down):
  const u = pt.x * zoom;
  const v = H - pt.y * zoom;
  switch (page.rotation) {
    case 0:
      return { x: u, y: v };
    case 90: // clockwise
      return { x: H - v, y: u };
    case 180:
      return { x: W - u, y: H - v };
    case 270:
      return { x: v, y: W - u };
  }
}

/** CSS point inside the rotated page element → PDF page space. */
export function cssToPdf(css: CssPoint, page: PageGeom, zoom: number): PdfPoint {
  const W = page.widthPt * zoom;
  const H = page.heightPt * zoom;
  let u: number;
  let v: number;
  switch (page.rotation) {
    case 0:
      u = css.x;
      v = css.y;
      break;
    case 90:
      u = css.y;
      v = H - css.x;
      break;
    case 180:
      u = W - css.x;
      v = H - css.y;
      break;
    case 270:
      u = W - css.y;
      v = css.x;
      break;
  }
  return { x: u / zoom, y: (H - v) / zoom };
}

/** Rect in PDF page space → rect in rotated page CSS space. */
export function pdfRectToCssRect(rect: PdfRect, page: PageGeom, zoom: number): CssRect {
  const a = pdfToPageCss({ x: rect.x, y: rect.y }, page, zoom);
  const b = pdfToPageCss({ x: rect.x + rect.w, y: rect.y + rect.h }, page, zoom);
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

/** Rect in rotated page CSS space → rect in PDF page space. */
export function cssRectToPdfRect(rect: CssRect, page: PageGeom, zoom: number): PdfRect {
  const a = cssToPdf({ x: rect.x, y: rect.y }, page, zoom);
  const b = cssToPdf({ x: rect.x + rect.w, y: rect.y + rect.h }, page, zoom);
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

/** Normalized page space → raw PDF user space (adds the crop-box origin). */
export function pageNormToPdfUser(pt: PdfPoint, geom: { cropX: number; cropY: number }): PdfPoint {
  return { x: pt.x + geom.cropX, y: pt.y + geom.cropY };
}

/** Raw PDF user space → normalized page space. */
export function pdfUserToPageNorm(pt: PdfPoint, geom: { cropX: number; cropY: number }): PdfPoint {
  return { x: pt.x - geom.cropX, y: pt.y - geom.cropY };
}

export interface TileRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Cover a device-pixel page area with fixed-size tiles (edge tiles clipped). */
export function tileGrid(devW: number, devH: number, tilePx: number): TileRect[] {
  const tiles: TileRect[] = [];
  for (let y = 0; y < devH; y += tilePx) {
    for (let x = 0; x < devW; x += tilePx) {
      tiles.push({ x, y, w: Math.min(tilePx, devW - x), h: Math.min(tilePx, devH - y) });
    }
  }
  return tiles;
}

/** Zoom that fits the whole (rotated) page into the viewport. */
export function fitPageZoom(page: PageGeom, viewW: number, viewH: number, margin: number): number {
  const { w, h } = pageCssDims(page, 1);
  return Math.min((viewW - 2 * margin) / w, (viewH - 2 * margin) / h);
}

/** Zoom that fits the (rotated) page width into the viewport. */
export function fitWidthZoom(page: PageGeom, viewW: number, margin: number): number {
  const { w } = pageCssDims(page, 1);
  return (viewW - 2 * margin) / w;
}

/**
 * New scrollTop that keeps the content under `anchorViewportY` stable across a
 * layout change (zoom / rotation / resize). Uses the page-relative fraction so
 * it is independent of the coordinate details of the change.
 */
export function anchorScrollTop(
  oldLayout: Layout,
  newLayout: Layout,
  oldScrollTop: number,
  anchorViewportY: number
): number {
  if (oldLayout.tops.length === 0) return 0;
  const cy = oldScrollTop + anchorViewportY;
  const i = pageIndexAt(oldLayout, cy);
  const oldTop = oldLayout.tops[i]!;
  const oldH = oldLayout.heights[i]!;
  const frac = Math.min(1, Math.max(0, (cy - oldTop) / oldH));
  const newCy = newLayout.tops[i]! + frac * newLayout.heights[i]!;
  return Math.max(0, newCy - anchorViewportY);
}

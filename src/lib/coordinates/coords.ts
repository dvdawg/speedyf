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

export interface ScrollPos {
  top: number;
  left: number;
}

/** A point in the scroll container's own viewport, CSS px from its top-left. */
export interface AnchorPoint {
  x: number;
  y: number;
}

/**
 * New scroll position that keeps the content under `anchor` stable across a
 * layout change (zoom / rotation / resize). Uses page-relative fractions so it
 * is independent of the coordinate details of the change.
 *
 * Both axes are anchored against the page under the anchor. Horizontal matters
 * once a page is wider than the viewport: pages are centred by layoutPages, so
 * without it the content slides sideways out from under the cursor as a pinch
 * grows the page. Fractions are clamped to the page box, so an anchor in the
 * margin or in an inter-page gap pins the nearest edge instead of
 * extrapolating across constant (unscaled) padding and gaps.
 */
export function anchorScroll(
  oldLayout: Layout,
  newLayout: Layout,
  oldScroll: ScrollPos,
  anchor: AnchorPoint
): ScrollPos {
  if (oldLayout.tops.length === 0) return { top: 0, left: 0 };
  const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

  const cy = oldScroll.top + anchor.y;
  const i = pageIndexAt(oldLayout, cy);
  const fracY = clamp01((cy - oldLayout.tops[i]!) / oldLayout.heights[i]!);
  const newCy = newLayout.tops[i]! + fracY * newLayout.heights[i]!;

  const cx = oldScroll.left + anchor.x;
  const fracX = clamp01((cx - oldLayout.lefts[i]!) / oldLayout.widths[i]!);
  const newCx = newLayout.lefts[i]! + fracX * newLayout.widths[i]!;

  return {
    top: Math.max(0, newCy - anchor.y),
    left: Math.max(0, newCx - anchor.x),
  };
}

/**
 * Multiplicative zoom factor for one wheel event.
 *
 * `deltaY` arrives in different units per device and engine, so it is
 * normalised to CSS px first (line mode is what Firefox and some mice report;
 * page mode is rare but real). The per-event clamp keeps a coarse mouse notch
 * — often a single ±100px event — to a ~12% step instead of the ~25% jump raw
 * exponentiation gives it, while leaving fine-grained trackpad deltas
 * untouched so a pinch stays continuous.
 */
export function wheelZoomFactor(deltaY: number, deltaMode: number, viewportH: number): number {
  const LINE_PX = 16;
  const MAX_STEP_PX = 48;
  const SENSITIVITY = 0.0022;
  const unit = deltaMode === 1 ? LINE_PX : deltaMode === 2 ? Math.max(1, viewportH) : 1;
  const px = Math.min(MAX_STEP_PX, Math.max(-MAX_STEP_PX, deltaY * unit));
  return Math.exp(-px * SENSITIVITY);
}

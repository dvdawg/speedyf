/** Viewport indicator: which slice of a page is currently on screen, expressed
 * as fractions of that page's box. Pure math — no DOM.
 *
 * The thumbnail sidebar uses this to draw a "what you're looking at" box over
 * a page thumbnail. Fractions (rather than px) are what make that work: the
 * main viewer and the sidebar lay pages out at different scales but share the
 * same rotation convention, so a fraction transfers between them unchanged. */
import type { Layout } from './layout';

/** A sub-rect of a page, each component a fraction of the page box (0..1). */
export interface PageFraction {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The scroll container's window onto the layout, in content-space CSS px. */
export interface ScrollView {
  scrollTop: number;
  scrollLeft: number;
  containerW: number;
  containerH: number;
}

/** Fraction of `page` visible in `view`, or null when the page is off screen
 * (or the index is out of range). */
export function pageViewportFraction(
  layout: Layout,
  page: number,
  view: ScrollView
): PageFraction | null {
  const top = layout.tops[page];
  const left = layout.lefts[page];
  const width = layout.widths[page];
  const height = layout.heights[page];
  if (top === undefined || left === undefined || !width || !height) return null;

  const x0 = Math.max(view.scrollLeft, left);
  const y0 = Math.max(view.scrollTop, top);
  const x1 = Math.min(view.scrollLeft + view.containerW, left + width);
  const y1 = Math.min(view.scrollTop + view.containerH, top + height);
  if (x1 <= x0 || y1 <= y0) return null;

  return {
    x: (x0 - left) / width,
    y: (y0 - top) / height,
    w: (x1 - x0) / width,
    h: (y1 - y0) / height,
  };
}

/** Where the viewport's top-left corner sits in a page's own space, as
 * fractions of that page's box.
 *
 * Unlike `pageViewportFraction` this is deliberately NOT clipped to the page:
 * when the view straddles two pages, its origin is genuinely *above* the lower
 * one, so the honest answer is negative. Clamping it to 0 — which the clipped
 * fraction does — makes a drag that starts on the lower page teleport the view
 * to that page's top edge instead of panning from where it already is.
 *
 * This is the value to anchor a drag on; `pageViewportFraction` is the value
 * to draw. */
export function viewportOriginFraction(
  layout: Layout,
  page: number,
  view: ScrollView
): { x: number; y: number } | null {
  const top = layout.tops[page];
  const left = layout.lefts[page];
  const width = layout.widths[page];
  const height = layout.heights[page];
  if (top === undefined || left === undefined || !width || !height) return null;

  return { x: (view.scrollLeft - left) / width, y: (view.scrollTop - top) / height };
}

/** Inverse of `pageViewportFraction`'s origin: the scroll position that puts
 * page-space fraction (fx, fy) at the viewport's top-left corner.
 *
 * Deliberately unclamped. Dragging the indicator past a page edge should carry
 * the view into the neighbouring page — this is a continuous scroll, not a
 * per-page one — and the scroll container clamps the real bounds anyway. */
export function scrollForPageFraction(
  layout: Layout,
  page: number,
  fx: number,
  fy: number
): { top: number; left: number } | null {
  const top = layout.tops[page];
  const left = layout.lefts[page];
  const width = layout.widths[page];
  const height = layout.heights[page];
  if (top === undefined || left === undefined || !width || !height) return null;

  return { top: top + fy * height, left: left + fx * width };
}

/** Uniform scale factor for dragging a box's bottom-right corner by (dx, dy).
 *
 * The box has one degree of freedom (zoom) but the pointer has two, so the
 * corner cannot follow the cursor exactly in both axes — the box's aspect is
 * the viewport's and has to stay that way. We therefore track whichever axis
 * the user is actually dragging, measured *relative to the box*, and match it
 * exactly. Drag down 10px on a 20px-tall box and the box becomes 30px tall:
 * the corner stays under the cursor on the axis being pulled.
 *
 * Projecting onto the box diagonal instead (the obvious alternative) minimises
 * total cursor-to-corner distance but under-responds badly on the short axis:
 * on a fit-width band, roughly 148x20, a 10px vertical drag moves the corner
 * 0.2px and the cursor leaves the grip immediately.
 *
 * Callers turn this into a zoom: box size is proportional to 1/zoom, so a
 * scale of s means `zoom / s`. */
export function cornerDragScale(w: number, h: number, dx: number, dy: number): number {
  const rx = w > 0 ? dx / w : 0;
  const ry = h > 0 ? dy / h : 0;
  const scale = 1 + (Math.abs(rx) >= Math.abs(ry) ? rx : ry);
  // A drag past the box's own origin would invert or zero the scale; floor it
  // so the zoom stays finite and the gesture can always be dragged back out.
  return Math.max(0.05, scale);
}

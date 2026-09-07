/** Where a tab's context menu sits, and how tall it is before it exists.
 *
 * Kept apart from the component so the clamping is an ordinary unit test —
 * the same split `tabPreviewLayout.ts` uses for hover cards. The height is
 * computed rather than measured so the menu can be placed on the frame it
 * opens, instead of appearing in the wrong place and jumping.
 */

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Viewport {
  width: number;
  height: number;
}

/** Keep these in step with `.tab-menu` in global.css. */
export const MENU_WIDTH = 232;
const ITEM_HEIGHT = 28;
const SEPARATOR_HEIGHT = 9;
const VERTICAL_PADDING = 8;
/** Closest the menu may come to the window edge. */
const MARGIN = 8;

/** Height of a menu with `items` rows and `separators` rules between them. */
export function tabMenuHeight(items: number, separators: number): number {
  return items * ITEM_HEIGHT + separators * SEPARATOR_HEIGHT + VERTICAL_PADDING;
}

/**
 * Drop the menu from the pointer, then pull it back inside the window.
 *
 * It flips above the pointer near the bottom edge rather than sliding up the
 * way the hover card does. A menu that slid would end up *under* the cursor,
 * putting some item other than the one aimed at directly beneath it — and
 * that item closes tabs. Sideways there is no such hazard, so it slides.
 */
export function tabMenuPosition(at: Point, size: Size, viewport: Viewport) {
  const rightLimit = viewport.width - size.width - MARGIN;
  // max() last so a menu wider than the window pins to the left edge rather
  // than hanging off it.
  const left = Math.max(MARGIN, Math.min(at.x, rightLimit));

  const fitsBelow = at.y + size.height + MARGIN <= viewport.height;
  const top = fitsBelow ? at.y : Math.max(MARGIN, at.y - size.height);

  return { left, top };
}

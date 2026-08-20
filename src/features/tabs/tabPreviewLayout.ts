/** Where a tab's hover preview sits.
 *
 * Kept apart from the component so the clamping is an ordinary unit test —
 * the same split `previewLayout.ts` uses for citation popovers. */

export interface Anchor {
  left: number;
  right: number;
  bottom: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface Viewport {
  width: number;
  height: number;
}

/** Gap between the tab and the card below it. */
const OFFSET = 6;
/** Closest the card may come to the window edge. */
const MARGIN = 8;

/**
 * Centre the card under its tab, then pull it back inside the window.
 *
 * Clamping rather than flipping: the strip is always at the top of the window,
 * so there is never room above, and a card that jumped sides as you swept
 * along the tabs would be harder to read than one that slides.
 */
export function tabPreviewPosition(anchor: Anchor, size: Size, viewport: Viewport) {
  const centered = (anchor.left + anchor.right) / 2 - size.width / 2;
  const rightLimit = viewport.width - size.width - MARGIN;
  // max() last so a card wider than the window pins to the left edge rather
  // than hanging off it.
  const left = Math.max(MARGIN, Math.min(centered, rightLimit));

  const below = anchor.bottom + OFFSET;
  const bottomLimit = viewport.height - size.height - MARGIN;
  const top = Math.max(MARGIN, Math.min(below, bottomLimit));

  return { left, top };
}

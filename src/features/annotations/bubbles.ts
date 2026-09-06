/** Geometry for shading a drawn bubble.
 *
 * Kept out of the layer component so the arithmetic can be tested without a
 * page, a pointer, or a running engine.
 */
import type { Annotation, PdfRect } from '../../types/model';

/** How far the shading is pulled in from the glyph box, as a fraction of the
 * box's shorter side.
 *
 * Zero: a shaded bubble is meant to read as *filled*, the way one shaded in
 * pen does, so the fill covers the printed ring instead of sitting inside it
 * as a smaller dot. The box PDFium reports is the font's box for the glyph and
 * is a little larger than the ring drawn inside it, which is what makes an
 * inset of zero cover the outline cleanly rather than crop it. */
export const SHADE_INSET = 0;

/** The circle to paint for a bubble: the largest one the glyph box holds,
 * centred, and round even when the box is not — so a shaded bubble covers the
 * whole control and never comes out an oval. */
export function shadeRect(box: PdfRect): PdfRect {
  const d = Math.min(box.w, box.h) * (1 - 2 * SHADE_INSET);
  return { x: box.x + (box.w - d) / 2, y: box.y + (box.h - d) / 2, w: d, h: d };
}

/** The shading covering a given bubble, if there is one.
 *
 * Matched by character index first, which is exact for a shading made this
 * session. A shading read back out of a saved file has no index to carry — it
 * is a Stamp annotation, and the glyph it was aimed at is not recorded in the
 * PDF — so it is matched by where it sits instead. Without that fallback,
 * reopening a shaded quiz would offer every bubble as empty and a click would
 * stack a second mark on top of the first.
 */
export function shadingFor(
  annots: readonly Annotation[],
  charIndex: number,
  box?: PdfRect
): Annotation | undefined {
  const byIndex = annots.find((a) => a.kind === 'bubble' && a.bubbleCharIndex === charIndex);
  if (byIndex || !box) return byIndex;
  return annots.find(
    (a) => a.kind === 'bubble' && a.bubbleCharIndex === undefined && covers(a, box)
  );
}

/** Whether a shading sits on a given bubble: its centre falls inside the
 * glyph box. Bubbles are far enough apart that a centre cannot be ambiguous,
 * and this tolerates the small differences a save-and-reload introduces. */
function covers(shading: Annotation, box: PdfRect): boolean {
  const cx = shading.rect.x + shading.rect.w / 2;
  const cy = shading.rect.y + shading.rect.h / 2;
  return cx >= box.x && cx <= box.x + box.w && cy >= box.y && cy <= box.y + box.h;
}

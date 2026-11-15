/** What the highlight tool marks for a dragged rectangle: the part of each
 * text run it crosses, one rectangle per run, at the run's full height. A
 * display equation it touches is marked whole, as one rectangle around it:
 * a formula highlighted as a stack of glyph boxes (a numerator, a bar's
 * worth of nothing, a denominator, a subscript) reads as noise.
 *
 * Pure. Page points, y up. */
import type { TextRunDto } from '../../types/engine';
import type { PdfRect } from '../../types/model';
import { displaysOf, type DisplayBox } from '../../lib/text/textHit';

/** A run counts when the rectangle covers this share of its height. */
const MIN_VERTICAL_COVER = 0.35;

export function highlightRects(
  runs: readonly TextRunDto[],
  rect: PdfRect,
  displays: readonly DisplayBox[] = []
): PdfRect[] {
  const out: PdfRect[] = [];
  const found = displaysOf(runs, displays);
  const displayOf = new Map<number, number>();
  found.forEach((d, i) => {
    for (let r = d.first; r <= d.last; r += 1) displayOf.set(r, i);
  });
  const marked = new Set<number>();
  runs.forEach((run, index) => {
    const x0 = Math.max(rect.x, run.x);
    const x1 = Math.min(rect.x + rect.w, run.x + run.w);
    const y0 = Math.max(rect.y, run.y);
    const y1 = Math.min(rect.y + rect.h, run.y + run.h);
    if (x1 - x0 < 1) return;
    if (y1 - y0 < run.h * MIN_VERTICAL_COVER) return;
    const display = displayOf.get(index);
    if (display !== undefined) {
      if (marked.has(display)) return;
      marked.add(display);
      const [x, y, w, h] = found[display]!.box;
      out.push({ x, y, w, h });
      return;
    }
    out.push({ x: x0, y: run.y, w: x1 - x0, h: run.h });
  });
  return out;
}

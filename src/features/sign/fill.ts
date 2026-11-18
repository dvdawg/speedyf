/** Fill & Sign geometry for printed forms: where a typed answer, a mark or a
 * date goes for a blank the engine found (src-tauri/src/engine/blanks.rs) or
 * for a plain click.
 *
 * Kept free of components so it can be tested without a page or an engine.
 * Every rect is normalized PDF page space, y-up, like the blanks themselves.
 */
import type { Blank } from '../../types/engine';
import type { MarkKind, PdfPoint, PdfRect } from '../../types/model';
import type { DateFormat } from '../../stores/settings';

/** Writing size for a click away from any blank, before the text near it
 * says otherwise. */
export const FALLBACK_FONT_PT = 10;
const MIN_FONT_PT = 8;
const MAX_FONT_PT = 12;

/** How far from a blank a click still lands in it, in points. */
const SNAP_SLOP_PT = 6;

/** The saved text's first baseline sits one font size under the box top
 * (engine save.rs), so a box is placed by where its baseline should be. */
export function boxForBaseline(
  x: number,
  baseline: number,
  w: number,
  fontSizePt: number
): PdfRect {
  const h = fontSizePt * 1.3 + 2;
  const top = baseline + fontSizePt;
  return { x, y: top - h, w, h };
}

export interface SnappedText {
  rect: PdfRect;
  fontSizePt: number;
  combCells?: number;
}

/** A text box snapped onto a blank: its baseline on the rule of a line
 * blank, vertically centred in a box or cell, one character a cell in a
 * comb. */
export function snapText(blank: Blank): SnappedText {
  const size = blank.fontSizePt;
  const r = blank.rect;
  if (blank.kind === 'line') {
    // Just clear of the rule, the way handwriting sits on one.
    const baseline = r.y + Math.max(1, size * 0.12);
    return {
      rect: boxForBaseline(r.x + 2, baseline, Math.max(8, r.w - 2), size),
      fontSizePt: size,
    };
  }
  // Capitals and figures centre on about a third of an em above the baseline.
  const baseline = r.y + r.h / 2 - size * 0.35;
  if (blank.kind === 'comb' && blank.cells > 0) {
    return {
      rect: boxForBaseline(r.x, baseline, r.w, size),
      fontSizePt: size,
      combCells: blank.cells,
    };
  }
  const pad = Math.min(2, r.w / 8);
  return {
    rect: boxForBaseline(r.x + pad, baseline, Math.max(8, r.w - 2 * pad), size),
    fontSizePt: size,
  };
}

function distance(r: PdfRect, p: PdfPoint): number {
  const dx = Math.max(r.x - p.x, 0, p.x - (r.x + r.w));
  const dy = Math.max(r.y - p.y, 0, p.y - (r.y + r.h));
  return Math.hypot(dx, dy);
}

/** The blank a click means: one it falls in (the smallest, when blanks nest),
 * else the nearest within a few points. */
export function blankAt(
  blanks: readonly Blank[],
  p: PdfPoint,
  kinds?: readonly Blank['kind'][]
): number | null {
  let best: number | null = null;
  let bestScore = Infinity;
  blanks.forEach((b, i) => {
    if (kinds && !kinds.includes(b.kind)) return;
    const d = distance(b.rect, p);
    if (d > SNAP_SLOP_PT) return;
    // Inside beats near; among those inside, the tighter fit wins.
    const score = d > 0 ? 1e9 + d : b.rect.w * b.rect.h;
    if (score < bestScore) {
      bestScore = score;
      best = i;
    }
  });
  return best;
}

/** The blanks Tab visits: everything that takes typing, in the engine's
 * reading order. Checkboxes take a mark, not text. */
export function typingStops(blanks: readonly Blank[]): number[] {
  return blanks.flatMap((b, i) => (b.kind === 'check' ? [] : [i]));
}

/** The next (or previous) typing stop after `from` on the same page, or
 * null when Tab should leave the page. */
export function nextStop(
  blanks: readonly Blank[],
  from: number | null,
  direction: 1 | -1
): number | null {
  const stops = typingStops(blanks);
  if (stops.length === 0) return null;
  if (from === null) return direction === 1 ? stops[0]! : stops[stops.length - 1]!;
  const at = stops.indexOf(from);
  const next = at === -1 ? (direction === 1 ? 0 : stops.length - 1) : at + direction;
  return next >= 0 && next < stops.length ? stops[next]! : null;
}

/** Writing size from the text around a point: the nearest run on its line.
 * A run's box is about one em tall. */
export function nearbyFontSize(
  runs: readonly { x: number; y: number; w: number; h: number }[],
  p: PdfPoint
): number {
  let best: { d: number; h: number } | null = null;
  for (const r of runs) {
    if (r.h <= 0 || p.y < r.y - 6 || p.y > r.y + r.h + 6) continue;
    const d = Math.max(r.x - p.x, 0, p.x - (r.x + r.w));
    if (d > 200) continue;
    if (!best || d < best.d) best = { d, h: r.h };
  }
  if (!best) return FALLBACK_FONT_PT;
  return Math.min(MAX_FONT_PT, Math.max(MIN_FONT_PT, Math.round(best.h * 2) / 2));
}

/** A free text box for a click on no blank: the click is the middle of the
 * first line, so the caret appears where the pointer was. */
export function freeText(p: PdfPoint, fontSizePt: number, pageWidthPt: number): PdfRect {
  const x = Math.max(0, Math.min(p.x, pageWidthPt - fontSizePt * 8));
  return boxForBaseline(x, p.y - fontSizePt * 0.35, fontSizePt * 8, fontSizePt);
}

/** Where a mark goes: centred in a small box it was dropped on, else
 * centred on the click and sized from the text around it. */
export function markRect(
  mark: MarkKind,
  p: PdfPoint,
  sizePt: number,
  box?: PdfRect | null
): PdfRect {
  if (box && mark !== 'line') {
    const inset = mark === 'circle' ? 0 : Math.min(box.w, box.h) * 0.1;
    const side = Math.min(box.w, box.h) - 2 * inset;
    const w = mark === 'circle' ? box.w : side;
    const h = mark === 'circle' ? box.h : side;
    return { x: box.x + (box.w - w) / 2, y: box.y + (box.h - h) / 2, w, h };
  }
  const [w, h] =
    mark === 'line'
      ? [sizePt * 3, Math.max(4, sizePt * 0.5)]
      : mark === 'circle'
        ? [sizePt * 2.2, sizePt * 1.4]
        : [sizePt, sizePt];
  return { x: p.x - w / 2, y: p.y - h / 2, w, h };
}

/** The pen a mark is drawn with, matching the engine's default. */
export function markPen(rect: PdfRect): number {
  return Math.min(2.5, Math.max(1, 0.1 * Math.min(rect.w, rect.h)));
}

/** A mark as SVG path data in page space (y down from `pageH`), the same
 * shapes engine/marks.rs writes. */
export function markPath(mark: MarkKind, r: PdfRect, pageH: number): string {
  const top = pageH - r.y - r.h;
  const at = (fx: number, fy: number) =>
    `${(r.x + fx * r.w).toFixed(2)} ${(top + fy * r.h).toFixed(2)}`;
  switch (mark) {
    case 'check':
      return `M ${at(0.14, 0.55)} L ${at(0.4, 0.82)} L ${at(0.88, 0.18)}`;
    case 'cross':
      return `M ${at(0.2, 0.2)} L ${at(0.8, 0.8)} M ${at(0.8, 0.2)} L ${at(0.2, 0.8)}`;
    case 'line':
      return `M ${at(0, 0.5)} L ${at(1, 0.5)}`;
    case 'dot': {
      const rad = 0.3 * Math.min(r.w, r.h);
      const cx = r.x + r.w / 2;
      const cy = top + r.h / 2;
      return `M ${cx - rad} ${cy} a ${rad} ${rad} 0 1 0 ${2 * rad} 0 a ${rad} ${rad} 0 1 0 ${-2 * rad} 0 Z`;
    }
    case 'circle': {
      const pen = markPen(r);
      const rx = Math.max(0.5, r.w / 2 - pen / 2);
      const ry = Math.max(0.5, r.h / 2 - pen / 2);
      const cx = r.x + r.w / 2;
      const cy = top + r.h / 2;
      return `M ${cx - rx} ${cy} a ${rx} ${ry} 0 1 0 ${2 * rx} 0 a ${rx} ${ry} 0 1 0 ${-2 * rx} 0 Z`;
    }
  }
}

/** Today's date as the setting says to write it. */
export function formatDate(date: Date, format: DateFormat): string {
  const y = String(date.getFullYear());
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  switch (format) {
    case 'iso':
      return `${y}-${m}-${d}`;
    case 'dmy':
      return `${d}/${m}/${y}`;
    default:
      return `${m}/${d}/${y}`;
  }
}

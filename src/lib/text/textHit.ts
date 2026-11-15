/** Where a pointer lands in a page's text, and how a selection of it is
 * drawn: the geometry behind the reader's own text selection.
 *
 * The browser's selection over the text layer is driven by hit testing its
 * transparent spans, and those are bad targets: a run's box is the ink of its
 * glyphs, so every run on a line has its own top and height, lines leave
 * gaps between them, and a press or a drag that falls into one hits the
 * layer behind the spans, where WebKit snaps the selection to the start or
 * the end of the whole page. Here the pointer is matched against lines
 * instead: the nearest line, then the nearest character boundary on it, and
 * a line counts as reaching across the white space a paragraph leaves (a
 * short last line, an indent), so that space selects the line beside it and
 * not the next one down. Painting uses the same lines, so a selected line is
 * one even band rather than a stack of differently sized run boxes.
 *
 * Pure: runs in page points, y up, as the engine sends them.
 */
import type { TextRunDto } from '../../types/engine';
import { linesOf, type Line } from './paragraph';

export interface PagePoint {
  x: number;
  y: number;
}

/** A caret position: a run, and a character offset into its text (UTF-16
 * units, as a DOM text node counts them). */
export interface TextPosition {
  run: number;
  offset: number;
}

export interface HitLine extends Line {
  /** How far left and right a pointer may stray and still be on this line:
   * its own extent widened to the lines just above and below it in the same
   * block, so the margin of a short line belongs to it. */
  reachLeft: number;
  reachRight: number;
  /** The band a selection of this line is painted in. */
  paintTop: number;
  paintBottom: number;
}

export interface TextModel {
  runs: readonly TextRunDto[];
  lines: readonly HitLine[];
  /** the line holding each run */
  lineOfRun: readonly number[];
  /** x of each character boundary of a run, `text.length + 1` of them */
  edges(run: number): readonly number[];
}

/** Width of a piece of text in any unit; only proportions are used. */
export type MeasureText = (text: string) => number;

/** Two lines this close (in ems) and overlapping across are the same block,
 * for the purpose of lending each other their width. */
const NEIGHBOUR_GAP_EMS = 1.5;
/** Paint pads a line by this share of its type above and below its ink, at
 * most half way to the line next to it. */
const PAINT_PAD_EMS = 0.2;
/** While dragging, horizontal distance to a line in the column the drag
 * started in counts for less, and to a line in another column for more, so
 * the gutter between two columns stays with the column being selected. */
const SAME_COLUMN_WEIGHT = 0.5;
const OTHER_COLUMN_WEIGHT = 1.5;

const overlaps = (a0: number, a1: number, b0: number, b1: number) =>
  Math.min(a1, b1) - Math.max(a0, b0) > 0;

/** Vertical distance between two lines' bands (0 when they overlap). */
const bandGap = (a: Line, b: Line) => Math.max(0, a.bottom - b.top, b.bottom - a.top);

function neighbours(a: Line, b: Line): boolean {
  const em = Math.min(a.size, b.size);
  return (
    overlaps(a.minX, a.maxX, b.minX, b.maxX) &&
    bandGap(a, b) <= NEIGHBOUR_GAP_EMS * em &&
    // a line beside, not above or below: another segment
    !overlaps(a.bottom, a.top, b.bottom, b.top)
  );
}

export function buildTextModel(
  runs: readonly TextRunDto[],
  measure: MeasureText = (text) => text.length
): TextModel {
  const base = linesOf(runs);
  const lines: HitLine[] = base.map((line, i) => {
    let reachLeft = line.minX;
    let reachRight = line.maxX;
    let paintTop = line.top + PAINT_PAD_EMS * line.size;
    let paintBottom = line.bottom - PAINT_PAD_EMS * line.size;
    for (const j of [i - 1, i + 1]) {
      const other = base[j];
      if (!other || !neighbours(line, other)) continue;
      reachLeft = Math.min(reachLeft, other.minX);
      reachRight = Math.max(reachRight, other.maxX);
      const half = bandGap(line, other) / 2;
      if (other.top <= line.bottom) paintBottom = Math.max(paintBottom, line.bottom - half);
      else paintTop = Math.min(paintTop, line.top + half);
    }
    return { ...line, reachLeft, reachRight, paintTop, paintBottom };
  });
  const lineOfRun: number[] = [];
  lines.forEach((line, i) => {
    for (let r = line.first; r <= line.last; r += 1) lineOfRun[r] = i;
  });

  const cache = new Map<number, number[]>();
  const edges = (index: number): readonly number[] => {
    const hit = cache.get(index);
    if (hit) return hit;
    const run = runs[index];
    if (!run) return [];
    const widths: number[] = [];
    let total = 0;
    for (let k = 0; k < run.text.length; k += 1) {
      // The low half of a surrogate pair is no character of its own.
      const w = isLowSurrogate(run.text, k) ? 0 : Math.max(0, measure(charAt(run.text, k)));
      widths.push(w);
      total += w;
    }
    const out = [run.x];
    let sum = 0;
    for (let k = 0; k < widths.length; k += 1) {
      sum += widths[k]!;
      out.push(
        total > 0 ? run.x + (run.w * sum) / total : run.x + (run.w * (k + 1)) / widths.length
      );
    }
    cache.set(index, out);
    return out;
  };

  return { runs, lines, lineOfRun, edges };
}

function isLowSurrogate(text: string, k: number): boolean {
  const c = text.charCodeAt(k);
  return c >= 0xdc00 && c <= 0xdfff && k > 0;
}

/** The whole character starting at UTF-16 offset `k`. */
function charAt(text: string, k: number): string {
  const c = text.charCodeAt(k);
  return c >= 0xd800 && c <= 0xdbff ? text.slice(k, k + 2) : text.charAt(k);
}

/** The line nearest `p`. `column`, while dragging, is the horizontal extent
 * of the line the drag started on. */
export function nearestLine(
  model: TextModel,
  p: PagePoint,
  column?: { minX: number; maxX: number }
): number {
  let best = -1;
  let bestCost = Infinity;
  model.lines.forEach((line, i) => {
    const dy = Math.max(0, line.bottom - p.y, p.y - line.top);
    const dx = Math.max(0, line.reachLeft - p.x, p.x - line.reachRight);
    let weight = 1;
    if (column) {
      weight = overlaps(line.reachLeft, line.reachRight, column.minX, column.maxX)
        ? SAME_COLUMN_WEIGHT
        : OTHER_COLUMN_WEIGHT;
    }
    const cost = dy + dx * weight;
    if (cost < bestCost) {
      bestCost = cost;
      best = i;
    }
  });
  return best;
}

/** The character boundary on line `index` nearest `x`. Left of the line is
 * its start, right of it its end. */
export function positionOnLine(model: TextModel, index: number, x: number): TextPosition | null {
  const line = model.lines[index];
  if (!line) return null;
  if (x <= line.minX) return { run: line.first, offset: 0 };
  if (x >= line.maxX) {
    return { run: line.last, offset: model.runs[line.last]?.text.length ?? 0 };
  }
  let best = line.first;
  let bestDistance = Infinity;
  for (let r = line.first; r <= line.last; r += 1) {
    const run = model.runs[r]!;
    const d = Math.max(0, run.x - x, x - (run.x + run.w));
    if (d < bestDistance) {
      bestDistance = d;
      best = r;
    }
  }
  const run = model.runs[best]!;
  if (x <= run.x) return { run: best, offset: 0 };
  if (x >= run.x + run.w) return { run: best, offset: run.text.length };
  const edges = model.edges(best);
  let offset = 0;
  let distance = Infinity;
  edges.forEach((edge, k) => {
    if (k > 0 && k < run.text.length && isLowSurrogate(run.text, k)) return;
    const d = Math.abs(edge - x);
    if (d < distance) {
      distance = d;
      offset = k;
    }
  });
  return { run: best, offset };
}

/** Where a press or a drag at `p` puts the caret. */
export function hitTest(
  model: TextModel,
  p: PagePoint,
  column?: { minX: number; maxX: number }
): TextPosition | null {
  const line = nearestLine(model, p, column);
  return line < 0 ? null : positionOnLine(model, line, p.x);
}

const WORD_CHAR = /[\p{L}\p{N}\p{M}_'\u2019]/u;

/** The word around `offset` in `text`, as [start, end): what a double-click
 * selects. Off a word (a space, a bracket) it is the one character there. */
export function wordAt(text: string, offset: number): [number, number] {
  if (text.length === 0) return [0, 0];
  let at = Math.min(Math.max(offset, 0), text.length - 1);
  // At the end of a word, the click was on it.
  if (!WORD_CHAR.test(text.charAt(at)) && at > 0 && WORD_CHAR.test(text.charAt(at - 1))) at -= 1;
  if (!WORD_CHAR.test(text.charAt(at))) {
    return [at, Math.min(text.length, at + charAt(text, at).length)];
  }
  let start = at;
  while (start > 0 && WORD_CHAR.test(text.charAt(start - 1))) start -= 1;
  let end = at;
  while (end < text.length && WORD_CHAR.test(text.charAt(end))) end += 1;
  return [start, end];
}

/** A selected stretch of one run. */
export interface SelectedPiece {
  run: number;
  from: number;
  to: number;
}

/** What to paint for a selection: one rectangle per line, [x, y, w, h] in
 * page points, y up, spanning the selected characters and the word spaces
 * between them at the line's own height. */
export function selectionRects(
  model: TextModel,
  pieces: readonly SelectedPiece[]
): [number, number, number, number][] {
  const out: [number, number, number, number][] = [];
  let lastLine = -1;
  let lastRun = -2;
  for (const { run: index, from, to } of pieces) {
    const run = model.runs[index];
    const lineIndex = model.lineOfRun[index];
    if (!run || lineIndex === undefined || to <= from) continue;
    const line = model.lines[lineIndex]!;
    const edges = from === 0 && to === run.text.length ? null : model.edges(index);
    const left = edges ? edges[from]! : run.x;
    const right = edges ? edges[to]! : run.x + run.w;
    const last = out.at(-1);
    if (last && lineIndex === lastLine && index === lastRun + 1) {
      const l = Math.min(last[0], left);
      const r = Math.max(last[0] + last[2], right);
      last[0] = l;
      last[2] = r - l;
    } else {
      out.push([left, line.paintBottom, right - left, line.paintTop - line.paintBottom]);
    }
    lastLine = lineIndex;
    lastRun = index;
  }
  return out;
}

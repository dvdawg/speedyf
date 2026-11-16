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
import { marginNumbers } from './marginNumbers';

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

/** A display equation as the engine finds it (`get_equation_blocks`),
 * turned y-up: its box, without the number, and the number it prints. */
export interface DisplayBox {
  x: number;
  y: number;
  w: number;
  h: number;
  number: string | null;
}

/** A display equation in the text layer: the consecutive runs that draw it
 * (its number among them when that follows it), selected as one unit, and
 * the box its selection is painted as. */
export interface Display {
  first: number;
  last: number;
  box: [number, number, number, number];
}

export interface TextModel {
  runs: readonly TextRunDto[];
  lines: readonly HitLine[];
  /** the line holding each run */
  lineOfRun: readonly number[];
  displays: readonly Display[];
  /** the display holding each run, or -1 */
  displayOfRun: readonly number[];
  /** runs that are margin line numbers (`marginNumbers`) */
  margin: readonly boolean[];
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
/** Paint pads a display equation's box by this much, in points. */
const DISPLAY_PAINT_PAD_PT = 2;
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

/** How far outside an equation's box a run may stick and still be part of
 * it, in points: a box from glyph ink and a run's box disagree a little. */
const DISPLAY_SLACK_PT = 2;

const inside = (run: TextRunDto, b: DisplayBox, slack: number) =>
  run.x >= b.x - slack &&
  run.x + run.w <= b.x + b.w + slack &&
  run.y >= b.y - slack &&
  run.y + run.h <= b.y + b.h + slack;

/** The runs of each display equation, kept only when they are one stretch of
 * the text layer: a selection is a DOM range, so a display whose runs are
 * interleaved with prose could not be selected alone, and is left to be
 * selected like text. */
export function displaysOf(runs: readonly TextRunDto[], boxes: readonly DisplayBox[]): Display[] {
  const out: Display[] = [];
  for (const b of boxes) {
    let first = -1;
    let last = -1;
    runs.forEach((run, i) => {
      if (!inside(run, b, DISPLAY_SLACK_PT)) return;
      if (first < 0) first = i;
      last = i;
    });
    if (first < 0) continue;
    let whole = true;
    for (let i = first; i <= last && whole; i += 1) {
      whole = inside(runs[i]!, b, DISPLAY_SLACK_PT);
    }
    if (!whole) continue;
    const box: [number, number, number, number] = [b.x, b.y, b.x + b.w, b.y + b.h];
    // The printed number, "(3)", to the right on the equation's rows.
    const next = runs[last + 1];
    if (b.number && next) {
      const centre = next.y + next.h / 2;
      const label = next.text.replace(/\s+/g, '');
      if (
        next.x >= b.x + b.w - DISPLAY_SLACK_PT &&
        centre >= b.y &&
        centre <= b.y + b.h &&
        label === `(${b.number})`
      ) {
        last += 1;
        box[2] = Math.max(box[2], next.x + next.w);
      }
    }
    for (let i = first; i <= last; i += 1) {
      const run = runs[i]!;
      box[0] = Math.min(box[0], run.x);
      box[1] = Math.min(box[1], run.y);
      box[2] = Math.max(box[2], run.x + run.w);
      box[3] = Math.max(box[3], run.y + run.h);
    }
    out.push({ first, last, box: [box[0], box[1], box[2] - box[0], box[3] - box[1]] });
  }
  return out.sort((a, b) => a.first - b.first);
}

export function buildTextModel(
  runs: readonly TextRunDto[],
  measure: MeasureText = (text) => text.length,
  displayBoxes: readonly DisplayBox[] = []
): TextModel {
  // Margin line numbers are no part of the text: on no line, so a press or
  // a drag never lands on one and a selection's band never covers one.
  const margin = marginNumbers(runs);
  const base = linesOf(runs, margin);
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
    // The engine's own: where each glyph's advance begins.
    if (run.edges && run.edges.length === run.text.length + 1) {
      const out = run.edges.map((e) => run.x + e);
      cache.set(index, out);
      return out;
    }
    // Else an estimate: the run's width shared out by measured widths.
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

  const displays = displaysOf(runs, displayBoxes);
  const displayOfRun: number[] = runs.map(() => -1);
  displays.forEach((d, i) => {
    for (let r = d.first; r <= d.last; r += 1) displayOfRun[r] = i;
  });

  return { runs, lines, lineOfRun, edges, displays, displayOfRun, margin };
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

/** The character boundary a press at `x` on line `index` means. On a glyph,
 * the boundary before it in its left half and after it in its right half;
 * on a space, or in the gap between two runs, the start of the word that
 * follows (a drag begun just before a word takes all of it); left of the
 * line its start, right of it its end. */
export function positionOnLine(model: TextModel, index: number, x: number): TextPosition | null {
  const line = model.lines[index];
  if (!line) return null;
  const order: number[] = [];
  for (let r = line.first; r <= line.last; r += 1) order.push(r);
  const left = (r: number) => model.edges(r)[0] ?? model.runs[r]!.x;
  const right = (r: number) => {
    const e = model.edges(r);
    return e[e.length - 1] ?? model.runs[r]!.x + model.runs[r]!.w;
  };
  order.sort((a, b) => left(a) - left(b));
  const first = order[0]!;
  if (x <= left(first)) return { run: first, offset: 0 };
  for (const r of order) {
    if (x < left(r)) return { run: r, offset: 0 };
    if (x <= right(r)) return { run: r, offset: offsetInRun(model, r, x) };
  }
  const last = order.reduce((a, b) => (right(b) > right(a) ? b : a));
  return { run: last, offset: model.runs[last]!.text.length };
}

/** The boundary in run `r` for a press at `x` within it. */
function offsetInRun(model: TextModel, r: number, x: number): number {
  const text = model.runs[r]!.text;
  const edges = model.edges(r);
  // the character under the pointer: the last one starting at or before x
  let k = 0;
  while (k + 1 < text.length && (edges[k + 1] ?? Infinity) <= x) k += 1;
  const skip = (at: number) => (at < text.length && isLowSurrogate(text, at) ? at + 1 : at);
  if (/\s/.test(text.charAt(k))) return skip(k + 1);
  const start = edges[k] ?? x;
  const end = edges[skip(k + 1)] ?? start;
  return x < (start + end) / 2 ? k : skip(k + 1);
}

/** A selection end at `pos`, taken out to the edge of the display
 * equation it is in (its end when `toEnd`, else its start), so a display is
 * only ever selected whole; `pos` itself when it is in none. */
export function displayEdge(model: TextModel, pos: TextPosition, toEnd: boolean): TextPosition {
  const d = model.displays[model.displayOfRun[pos.run] ?? -1];
  if (!d) return pos;
  return toEnd
    ? { run: d.last, offset: model.runs[d.last]?.text.length ?? 0 }
    : { run: d.first, offset: 0 };
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
  let lastDisplay = -1;
  for (const { run: index, from, to } of pieces) {
    const run = model.runs[index];
    const lineIndex = model.lineOfRun[index];
    if (!run || lineIndex === undefined || to <= from) continue;
    // A display equation is one box, however many rows it has.
    const display = model.displayOfRun[index] ?? -1;
    if (display >= 0) {
      if (display !== lastDisplay) {
        const [x, y, w, h] = model.displays[display]!.box;
        const pad = DISPLAY_PAINT_PAD_PT;
        out.push([x - pad, y - pad, w + 2 * pad, h + 2 * pad]);
      }
      lastDisplay = display;
      lastLine = -1;
      continue;
    }
    lastDisplay = -1;
    const line = model.lines[lineIndex]!;
    // The same boundaries a press resolves to, so the band covers exactly
    // the characters a copy takes.
    const edges = model.edges(index);
    const left = edges[from] ?? run.x;
    const right = edges[to] ?? run.x + run.w;
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

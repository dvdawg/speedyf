/** The paragraph under a triple-click: the run range a selection should cover.
 *
 * A text layer is one span per run, in the extractor's reading order, so a
 * paragraph is a contiguous range of runs. The runs are grouped into lines
 * with the rules the copy rebuild uses (`smartCopy`), a line split where a
 * wide gap separates two columns on one baseline (the engine's
 * `SEGMENT_GAP_EMS` in layout_text.rs), and consecutive lines are one block
 * until a paragraph gap, an indent, a change of type size (a heading) or a
 * move to another column ends it.
 *
 * Pure and geometry-only. Runs are in page points, y up, as the engine sends
 * them.
 */
import type { TextRunDto } from '../../types/engine';

/** A vertical move smaller than this share of the type is the same line. */
const SAME_LINE_RATIO = 0.6;
/** A horizontal gap wider than this many ems on one baseline splits a line
 * into segments: the space between page columns, never a word space. */
const SEGMENT_GAP_EMS = 2.5;
/** Blank space between lines that reads as a paragraph boundary. */
const PARAGRAPH_GAP_RATIO = 0.9;
/** Indent that reads as a paragraph boundary (LaTeX indents about 1em). */
const PARAGRAPH_INDENT_RATIO = 0.6;
/** Type this much larger than its neighbour is another block: a heading over
 * its section, a caption beside body text. */
const SIZE_CHANGE_RATIO = 1.2;

interface Line {
  first: number;
  last: number;
  top: number;
  bottom: number;
  minX: number;
  maxX: number;
  /** the tallest run's height, a stand-in for the type size */
  size: number;
}

function linesOf(runs: readonly TextRunDto[]): Line[] {
  const lines: Line[] = [];
  let current: Line | null = null;
  runs.forEach((run, index) => {
    const h = Math.max(1, run.h);
    const top = run.y + h;
    if (current !== null) {
      const em = Math.max(current.size, h);
      const sameBand = Math.abs(current.bottom - run.y) <= SAME_LINE_RATIO * em;
      const gap = run.x - current.maxX;
      if (sameBand && gap > -em && gap <= SEGMENT_GAP_EMS * em) {
        current.last = index;
        current.top = Math.max(current.top, top);
        current.bottom = Math.min(current.bottom, run.y);
        current.minX = Math.min(current.minX, run.x);
        current.maxX = Math.max(current.maxX, run.x + run.w);
        current.size = Math.max(current.size, h);
        return;
      }
    }
    current = {
      first: index,
      last: index,
      top,
      bottom: run.y,
      minX: run.x,
      maxX: run.x + run.w,
      size: h,
    };
    lines.push(current);
  });
  return lines;
}

/** Whether `next` carries on the block `previous` belongs to. */
function continuesBlock(previous: Line, next: Line): boolean {
  const em = Math.min(previous.size, next.size);
  // Back up the page, or level with it: a new column, or a segment beside.
  if (next.top >= previous.top) return false;
  if (Math.max(previous.size, next.size) > SIZE_CHANGE_RATIO * em) return false;
  if (previous.bottom - next.top > PARAGRAPH_GAP_RATIO * em) return false;
  // Not under the line above at all: another column.
  if (next.minX >= previous.maxX || next.maxX <= previous.minX) return false;
  return next.minX - previous.minX <= PARAGRAPH_INDENT_RATIO * em;
}

/** The first and last run index of the paragraph holding run `index`, or
 * null when there is no such run. */
export function paragraphRunRange(
  runs: readonly TextRunDto[],
  index: number
): [number, number] | null {
  if (index < 0 || index >= runs.length) return null;
  const lines = linesOf(runs);
  const at = lines.findIndex((line) => index >= line.first && index <= line.last);
  if (at < 0) return null;
  let from = at;
  while (from > 0 && continuesBlock(lines[from - 1]!, lines[from]!)) from -= 1;
  let to = at;
  while (to < lines.length - 1 && continuesBlock(lines[to]!, lines[to + 1]!)) to += 1;
  return [lines[from]!.first, lines[to]!.last];
}

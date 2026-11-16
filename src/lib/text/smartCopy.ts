/** Turning a PDF text selection into text worth pasting.
 *
 * A PDF has no paragraphs — only glyphs at coordinates. Copying the raw
 * selection gives one fragment per positioned run, which pastes as a column
 * of broken lines with `ﬁ` ligatures and words split across line ends. This
 * module rebuilds the prose from the geometry the extractor already gives us.
 *
 * Pure and geometry-only, so every heuristic below is an ordinary unit test.
 */
import type { TextRunDto } from '../../types/engine';

/** A selected run, plus the page it came from — runs are only comparable
 * within a page, since PDF coordinates restart on each one. */
export interface CopyRun extends TextRunDto {
  page: number;
  /** The run's place in its page's text layer, which the engine sends in
   * reading order (`reading_order.rs`). Without it runs are taken in the
   * order of their characters in the PDF, which in a page whose stream
   * interleaves its columns is not the order they read in. */
  order?: number;
  /** The box of the display equation the run is part of, if any: its runs
   * are read row by row rather than in the order the PDF drew them. */
  display?: readonly number[];
}

/** Glyphs PDFs store as single characters that nobody wants in their notes. */
const LIGATURES = new Map<string, string>([
  ['ﬀ', 'ff'],
  ['ﬁ', 'fi'],
  ['ﬂ', 'fl'],
  ['ﬃ', 'ffi'],
  ['ﬄ', 'ffl'],
  ['ﬅ', 'st'],
  ['ﬆ', 'st'],
]);

/** Characters that can end a line as "this word continues below". U+00AD is
 * the explicit soft hyphen; the others are hyphens a typesetter chose. */
const LINE_END_HYPHENS = ['-', '‐', '‑', '­'];

/** Fraction of a line's height that counts as a word gap rather than kerning. */
const WORD_GAP_RATIO = 0.25;
/** A vertical move smaller than this is the same line (superscripts, math). */
const SAME_LINE_RATIO = 0.6;
/** A vertical move larger than this is a new line no matter where x sits. */
const FORCED_LINE_RATIO = 1.6;
/** Blank space between lines that reads as a paragraph boundary. */
const PARAGRAPH_GAP_RATIO = 0.9;
/** Indent that reads as a paragraph boundary (LaTeX indents about 1em). */
const PARAGRAPH_INDENT_RATIO = 0.6;

/** Soft hyphens survive this step: at a line end one is the marker that a
 * word continues below, and dropping it here would hide that from the wrap
 * check. Whatever is left over is removed once the lines are joined. */
function normalizeGlyphs(text: string): string {
  let out = '';
  for (const ch of text) {
    // a variation selector (a font's hint about the glyph before it)
    const cp = ch.codePointAt(0)!;
    if (cp >= 0xfe00 && cp <= 0xfe0f) continue;
    const expanded = LIGATURES.get(ch);
    out += expanded ?? ch;
  }
  return out;
}

function medianHeight(runs: readonly CopyRun[]): number {
  const heights = runs.map((run) => Math.max(1, run.h)).sort((a, b) => a - b);
  if (heights.length === 0) return 11;
  const middle = heights.length >> 1;
  return heights.length % 2 === 0
    ? (heights[middle - 1]! + heights[middle]!) / 2
    : heights[middle]!;
}

interface Line {
  page: number;
  /** y of the line's baseline band, taken from its first run */
  y: number;
  top: number;
  bottom: number;
  minX: number;
  maxX: number;
  text: string;
}

/** Group runs into visual lines, following the order they come in: reading
 * order, columns one after another. */
function groupLines(runs: readonly CopyRun[], medianH: number): Line[] {
  const lines: Line[] = [];
  let current: Line | null = null;

  for (const run of runs) {
    const text = normalizeGlyphs(run.text);
    if (text.length === 0) continue;
    const top = run.y + run.h;
    const drop = current === null ? Infinity : Math.abs(current.y - run.y);
    // A run that sits at a different height but keeps moving rightwards is
    // still this line — that is what a superscript citation marker or an
    // inline formula looks like, and breaking there would insert a space
    // into the middle of a word.
    const startsNewLine =
      current === null ||
      run.page !== current.page ||
      drop > FORCED_LINE_RATIO * medianH ||
      (drop > SAME_LINE_RATIO * medianH && run.x < current.maxX - 0.1 * medianH);

    // The redundant-looking null check is what narrows `current` for the
    // continuation path below; a stored boolean would not.
    if (current === null || startsNewLine) {
      current = {
        page: run.page,
        y: run.y,
        top,
        bottom: run.y,
        minX: run.x,
        maxX: run.x + run.w,
        text,
      };
      lines.push(current);
      continue;
    }

    const gap = run.x - current.maxX;
    const needsSpace =
      gap > WORD_GAP_RATIO * medianH && !/\s$/.test(current.text) && !/^\s/.test(text);
    current.text += (needsSpace ? ' ' : '') + text;
    current.top = Math.max(current.top, top);
    current.bottom = Math.min(current.bottom, run.y);
    current.minX = Math.min(current.minX, run.x);
    current.maxX = Math.max(current.maxX, run.x + run.w);
  }
  return lines;
}

/** The runs in the order they read on the page, where the PDF drew them in
 * another: within a line, left to right (a superscript or a formula's
 * pieces drawn after the text beside them), and a display equation row by
 * row, top to bottom. Lines, columns and pages keep the extractor's order,
 * which is reading order already. */
export function visualOrder(runs: readonly CopyRun[], medianH: number): CopyRun[] {
  const out: CopyRun[] = [];
  let i = 0;
  while (i < runs.length) {
    const first = runs[i]!;
    let j = i + 1;
    if (first.display) {
      while (j < runs.length && sameBox(runs[j]!.display, first.display)) j += 1;
      out.push(...byRows(runs.slice(i, j), medianH));
    } else {
      let bottom = first.y;
      let top = first.y + first.h;
      while (j < runs.length) {
        const run = runs[j]!;
        const centre = run.y + run.h / 2;
        if (run.display || run.page !== first.page || centre < bottom || centre > top) break;
        if (run.h <= 1.5 * medianH) {
          bottom = Math.min(bottom, run.y);
          top = Math.max(top, run.y + run.h);
        }
        j += 1;
      }
      out.push(...runs.slice(i, j).sort((a, b) => a.x - b.x));
    }
    i = j;
  }
  return out;
}

function sameBox(a: readonly number[] | undefined, b: readonly number[]): boolean {
  return a !== undefined && (a === b || a.every((v, k) => v === b[k]));
}

/** A display's runs as rows, top first, each left to right. */
function byRows(runs: readonly CopyRun[], medianH: number): CopyRun[] {
  const rows: CopyRun[][] = [];
  const centre = (r: CopyRun) => r.y + r.h / 2;
  for (const run of [...runs].sort((a, b) => centre(b) - centre(a))) {
    const row = rows.at(-1);
    if (row && centre(row[0]!) - centre(run) <= 0.5 * medianH) row.push(run);
    else rows.push([run]);
  }
  return rows.flatMap((row) => row.sort((a, b) => a.x - b.x));
}

/** Pages in turn, and on a page the text layer's order where the runs
 * know it, else their characters' order in the PDF. */
function readingOrder(a: CopyRun, b: CopyRun): number {
  if (a.page !== b.page) return a.page - b.page;
  if (a.order !== undefined && b.order !== undefined) return a.order - b.order;
  return a.start - b.start;
}

/** True when `previous` ends mid-word and `next` finishes that word. */
function isWrappedWord(previous: string, next: string): boolean {
  const trimmed = previous.trimEnd();
  if (!LINE_END_HYPHENS.some((hyphen) => trimmed.endsWith(hyphen))) return false;
  // A capital or a digit after the break means a new token, not the tail of
  // the broken one.
  return /^[a-z]/.test(next.trimStart());
}

/** Whether the step from `previous` to `next` reads as a new paragraph. */
function startsParagraph(previous: Line, next: Line, medianH: number): boolean {
  // Flowing into a new column or page continues the prose rather than ending
  // it: the text moves back up the page, so vertical gaps say nothing here.
  if (next.page !== previous.page || next.top > previous.top) return false;
  const gap = previous.bottom - next.top;
  if (gap > PARAGRAPH_GAP_RATIO * medianH) return true;
  return next.minX - previous.minX > PARAGRAPH_INDENT_RATIO * medianH;
}

/**
 * Rebuild pasteable prose from the selected runs.
 *
 * Wrapped words are rejoined, which is right for the hyphenation LaTeX
 * inserts and occasionally wrong for a compound that happened to break at its
 * own hyphen ("state-of-the-art" split after "state-"). Rejoining is the far
 * more common case in the papers this reads, so it wins the tie.
 */
export function smartCopyText(runs: readonly CopyRun[]): string {
  if (runs.length === 0) return '';
  const medianH = medianHeight(runs);
  const ordered = visualOrder([...runs].sort(readingOrder), medianH);
  const lines = groupLines(ordered, medianH);
  if (lines.length === 0) return '';

  let out = lines[0]!.text;
  for (let i = 1; i < lines.length; i += 1) {
    const previous = lines[i - 1]!;
    const next = lines[i]!;
    if (isWrappedWord(previous.text, next.text)) {
      out = out.trimEnd().slice(0, -1) + next.text.trimStart();
    } else if (startsParagraph(previous, next, medianH)) {
      out += '\n\n' + next.text;
    } else {
      out += ' ' + next.text;
    }
  }
  // Collapse the runs of spaces that positioned text is full of, and drop any
  // soft hyphen that was not a line break after all, but keep the paragraph
  // breaks built above.
  return out
    .replace(/\u00ad/g, '')
    .split('\n')
    .map((line) => line.replace(/[ \t]+/g, ' ').trim())
    .join('\n')
    .trim();
}

/** What the reader knows about a selection, for an extension quoting it
 * (`selection.context`): the formal statement it quotes, if any, cut out
 * of what surrounds it, and the cross-references in it with where each
 * points.
 *
 * A selection quotes a statement when it covers the statement's label, or
 * when it starts on the label's line, or starts inside the statement and
 * runs to its end (the label missed). The statement's extent is the
 * engine's: label to the end of the statement, before "Proof". Text before
 * and after it on the page is handed back as parts of their own, so a quote
 * of "Proposition 2.2 and the paragraph after it" becomes the statement and
 * a quote. Everything here is pure but the last function. */
import type { HoverSpan } from '../hovers/hoverTypes';
import { topLeft, type Rect, type Statement } from './paperDocs';

/** A selection as the core keeps it: its display page, its rectangles
 * there (top-left page points) and its smart-copied text. */
export interface SelectionShape {
  page: number;
  rects: Rect[];
  text: string;
}

export interface StatementHit {
  statement: Statement;
  /** The statement's extent, top-left page points. */
  box: Rect;
  /** Its label, top-left page points. */
  labelBox: Rect;
}

function union(rects: readonly Rect[]): Rect | null {
  if (rects.length === 0) return null;
  const x0 = Math.min(...rects.map((r) => r[0]));
  const y0 = Math.min(...rects.map((r) => r[1]));
  const x1 = Math.max(...rects.map((r) => r[0] + r[2]));
  const y1 = Math.max(...rects.map((r) => r[1] + r[3]));
  return [x0, y0, x1 - x0, y1 - y0];
}

function overlaps(a: Rect, b: Rect): boolean {
  const w = Math.min(a[0] + a[2], b[0] + b[2]) - Math.max(a[0], b[0]);
  const h = Math.min(a[1] + a[3], b[1] + b[3]) - Math.max(a[1], b[1]);
  return w > 0 && h > 0.4 * Math.min(a[3], b[3]);
}

const centre = (r: Rect) => r[1] + r[3] / 2;

/** The statement a selection quotes, among the statements on its page
 * (`height` the page's height in points), or null. */
export function statementAt(
  selection: Pick<SelectionShape, 'rects'>,
  statements: readonly Statement[],
  height: number
): StatementHit | null {
  if (selection.rects.length === 0) return null;
  const first = selection.rects.reduce((a, b) => (b[1] < a[1] ? b : a));
  const last = selection.rects.reduce((a, b) => (b[1] + b[3] > a[1] + a[3] ? b : a));
  for (const statement of statements) {
    if (statement.kind === 'equation') continue;
    const labelBox = union(statement.labelRects.map((r) => topLeft(r, height)));
    if (!labelBox) continue;
    const box = topLeft(statement.region, height);
    const inside = (r: Rect) =>
      centre(r) >= box[1] &&
      centre(r) <= box[1] + box[3] &&
      r[0] < box[0] + box[2] &&
      r[0] + r[2] > box[0];
    const covers = selection.rects.some((r) => overlaps(r, labelBox));
    const onLabelLine =
      inside(first) && centre(first) >= labelBox[1] && centre(first) <= labelBox[1] + labelBox[3];
    // Started in the statement and ran to its last line: the label missed.
    const toItsEnd =
      inside(first) && last[1] + last[3] >= box[1] + box[3] - Math.max(4, 1.5 * labelBox[3]);
    if (covers || onLabelLine || toItsEnd) return { statement, box, labelBox };
  }
  return null;
}

/** Words of `text` with where each starts and ends. */
function words(text: string): { w: string; start: number; end: number }[] {
  const out: { w: string; start: number; end: number }[] = [];
  const re = /[\p{L}\p{N}]+/gu;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    out.push({
      w: m[0].normalize('NFKC').toLowerCase(),
      start: m.index,
      end: m.index + m[0].length,
    });
  }
  return out;
}

/** Where `run` (words) occurs in `hay` (words), from `from`: its index, or
 * -1. */
function find(hay: { w: string }[], run: string[], from = 0, last = false): number {
  let found = -1;
  for (let i = from; i + run.length <= hay.length; i++) {
    if (run.every((w, k) => hay[i + k]!.w === w)) {
      found = i;
      if (!last) return i;
    }
  }
  return found;
}

/** The selection's text cut into what comes before the statement, the
 * statement, and what follows it: by the statement's own words (its first
 * and last few) found in the text; a side not found runs to the text's
 * end. */
export function splitStatementText(
  text: string,
  statementText: string
): { before: string; statement: string; after: string } {
  const hay = words(text);
  const own = words(statementText).map((w) => w.w);
  let startAt = 0;
  // The opening: the label, or (a selection that missed it) some words
  // just after it.
  for (let skip = 0; skip < Math.min(12, own.length - 2); skip++) {
    const i = find(hay, own.slice(skip, skip + 3));
    if (i >= 0) {
      startAt = hay[i]!.start;
      break;
    }
  }
  let endAt = text.length;
  for (let back = 0; back < Math.min(12, own.length - 2); back++) {
    const run = own.slice(own.length - 3 - back, own.length - back);
    const i = find(hay, run, 0, true);
    if (i >= 0 && hay[i + run.length - 1]!.end > startAt) {
      endAt = hay[i + run.length - 1]!.end;
      // A closing period (or bracket) belongs to it.
      const tail = /^[.)\]]+/.exec(text.slice(endAt));
      if (tail) endAt += tail[0].length;
      break;
    }
  }
  return {
    before: text.slice(0, startAt).trim(),
    statement: text.slice(startAt, endAt).trim(),
    after: text.slice(endAt).trim(),
  };
}

/** The selection's rectangles before, in and after the statement's
 * extent, by where each line's middle is. */
export function splitStatementRects(
  rects: readonly Rect[],
  box: Rect
): { before: Rect[]; statement: Rect[]; after: Rect[] } {
  const out = { before: [] as Rect[], statement: [] as Rect[], after: [] as Rect[] };
  for (const r of rects) {
    const c = centre(r);
    if (c < box[1]) out.before.push(r);
    else if (c > box[1] + box[3]) out.after.push(r);
    else out.statement.push(r);
  }
  return out;
}

/** A cross-reference in a selection and where it points. */
export interface SelectionXref {
  /** "Theorem 2.1", "Equation 12", "Figure 3", "Section 2". */
  label: string;
  /** "theorem", "equation", "figure", "table", "algorithm", "section". */
  kind: string;
  /** The label's number as printed: "2.1", "12". */
  number: string;
  /** The page in the file the target is on. */
  page: number;
  /** The target's extent, top-left page points. */
  box: Rect | null;
}

/** The page's cross-reference spans that lie in the selection. `height`
 * is the selection page's height, `targetHeight` any page's. */
export function xrefsIn(
  selection: Pick<SelectionShape, 'rects'>,
  spans: readonly HoverSpan[],
  height: number,
  targetHeight: (page: number) => number | null
): SelectionXref[] {
  const out: SelectionXref[] = [];
  for (const span of spans) {
    if (span.kind === 'footnote' || span.kind === 'term') continue;
    const inside = span.rects.some((r) =>
      selection.rects.some((s) => overlaps(topLeft(r as Rect, height), s))
    );
    if (!inside) continue;
    const h = targetHeight(span.target.page);
    const region = span.target.region;
    const number = /\S+$/.exec(span.label)?.[0] ?? span.label;
    if (out.some((x) => x.label === span.label)) continue;
    out.push({
      label: span.label,
      kind: span.kind,
      number,
      page: span.target.page,
      box:
        region && h !== null
          ? (topLeft(region as Rect, h).map((v) => Math.round(v)) as Rect)
          : null,
    });
  }
  return out;
}

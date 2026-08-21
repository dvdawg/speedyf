/** Recovering the words underneath a highlight.
 *
 * A highlight annotation stores quadpoints — rectangles, one per line of text
 * it covers — and no text at all. To show what was highlighted, the rectangles
 * have to be matched back against the page's own text.
 *
 * The ordering is the part worth being careful about. Text runs are gathered by
 * `start`, their position in PDFium's character stream, **not** by where they
 * sit on the page. Sorting geometrically is what spliced two-column lines
 * together when the figures panel was built: a highlight crossing a column
 * break would come back interleaved, alternating between columns. The character
 * stream already follows reading order, so using it is both simpler and right.
 */
import type { PdfQuad } from '../../types/model';
import type { TextRunDto } from '../../types/engine';

/** How much of a run must fall inside the highlight to count as highlighted.
 *
 * Runs are roughly word-sized, so a highlight that stops mid-word still catches
 * the whole word — which is what a reader means by highlighting it. Requiring
 * most of the run keeps the line above and below from bleeding in. */
const MIN_OVERLAP = 0.5;

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A quad's bounding box. Quads are axis-aligned in every PDF SpeedyF writes
 * and in practically every one it reads; a rotated quad simply covers a little
 * more than it should. */
function boundsOf(quad: PdfQuad): Box {
  const xs = [quad.p1.x, quad.p2.x, quad.p3.x, quad.p4.x];
  const ys = [quad.p1.y, quad.p2.y, quad.p3.y, quad.p4.y];
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}

/** The fraction of `run` that lies inside `box`. */
function overlapFraction(run: TextRunDto, box: Box): number {
  const width = Math.min(run.x + run.w, box.x + box.w) - Math.max(run.x, box.x);
  const height = Math.min(run.y + run.h, box.y + box.h) - Math.max(run.y, box.y);
  if (width <= 0 || height <= 0) return 0;
  const area = run.w * run.h;
  return area > 0 ? (width * height) / area : 0;
}

/** The runs a set of quads covers, in reading order.
 *
 * Returned as runs rather than a string so the caller can hand them to
 * `smartCopyText`, which is what already knows how to rejoin a word broken
 * across a line and how to unpick ligatures. */
export function runsUnderQuads(
  quads: readonly PdfQuad[],
  runs: readonly TextRunDto[]
): TextRunDto[] {
  if (quads.length === 0 || runs.length === 0) return [];
  const boxes = quads.map(boundsOf);
  return runs
    .filter((run) => boxes.some((box) => overlapFraction(run, box) >= MIN_OVERLAP))
    .slice()
    .sort((a, b) => a.start - b.start);
}

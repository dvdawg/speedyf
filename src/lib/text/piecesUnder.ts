/** A kept comment's quote, taken apart again from where it is rather than
 * from a live selection: the pieces of the page's runs under its quads,
 * as `selectedQuotePieces` would have collected them when the words were
 * selected. For quotes written before their math could be read, and for
 * reading one again.
 *
 * A character is under a quad when its middle is; a display equation any
 * of whose runs is under one is taken whole, as a selection takes it; margin
 * line numbers are never taken. Pure: page points, y up. */
import type { TextRunDto } from '../../types/engine';
import type { QuotePiece } from './copySelection';
import { buildTextModel, type DisplayBox } from './textHit';

type Rect = readonly [number, number, number, number];

/** How much of a run's height a quad must cover for the run to be on it. */
const ON_LINE = 0.5;

export function piecesUnderQuads(
  runs: readonly TextRunDto[],
  /** the quads, each as its box `[x, y, w, h]` */
  rects: readonly Rect[],
  displays: readonly DisplayBox[],
  /** the reader's page the runs are on */
  page: number,
  /** the words the comment quoted when it was made: where it began and
   * ended, when its quads reach a little past them */
  quote?: string
): QuotePiece[] {
  const model = buildTextModel(runs, undefined, displays);
  const spans = new Map<number, [number, number]>();
  runs.forEach((run, index) => {
    if (model.margin[index] || run.text.length === 0) return;
    const edges = model.edges(index);
    for (const [qx, qy, qw, qh] of rects) {
      const cover = Math.min(run.y + run.h, qy + qh) - Math.max(run.y, qy);
      if (cover < ON_LINE * Math.min(run.h, qh)) continue;
      let from = -1;
      let to = -1;
      for (let k = 0; k < run.text.length; k += 1) {
        const middle = (edges[k]! + edges[k + 1]!) / 2;
        if (middle < qx || middle > qx + qw) continue;
        if (from < 0) from = k;
        to = k + 1;
      }
      if (from < 0) continue;
      const span = spans.get(index);
      spans.set(index, span ? [Math.min(span[0], from), Math.max(span[1], to)] : [from, to]);
    }
  });
  // a display is selected whole
  for (const d of model.displays) {
    let touched = false;
    for (let r = d.first; r <= d.last; r += 1) touched ||= spans.has(r);
    if (!touched) continue;
    for (let r = d.first; r <= d.last; r += 1) spans.set(r, [0, runs[r]!.text.length]);
  }
  const ordered = [...spans.entries()].sort((a, b) => a[0] - b[0]);
  // The quote's two ends on whole words: a word the quads cut is taken
  // when most of it is under them, else left out. (Selections used to start
  // a letter late and end a letter early.)
  const word = (ch: string | undefined) => ch !== undefined && /[\p{L}\p{N}]/u.test(ch);
  if (quote) alignToQuote(runs, ordered, quote);
  // nor on the punctuation that ended the sentence before it
  const lead = ordered[0];
  if (lead) {
    const text = runs[lead[0]]!.text;
    let from = lead[1][0];
    while (from < lead[1][1] && /[\s.,;:)\]]/.test(text[from]!)) from += 1;
    lead[1] = [from, lead[1][1]];
  }
  const first = ordered[0];
  if (first) {
    const text = runs[first[0]]!.text;
    const to = first[1][1];
    let from = first[1][0];
    if (word(text[from]) && word(text[from - 1])) {
      let start = from;
      while (word(text[start - 1])) start -= 1;
      let end = from;
      while (word(text[end])) end += 1;
      from = 2 * (end - from) >= end - start ? start : end;
      first[1] = [Math.min(from, to), to];
    }
  }
  const last = ordered.at(-1);
  if (last) {
    const text = runs[last[0]]!.text;
    const from = last[1][0];
    let to = last[1][1];
    if (word(text[to - 1]) && word(text[to])) {
      let end = to;
      while (word(text[end])) end += 1;
      let start = to;
      while (word(text[start - 1])) start -= 1;
      to = 2 * (to - start) >= end - start ? end : start;
      last[1] = [from, Math.max(from, to)];
    }
  }
  return ordered
    .filter(([, [from, to]]) => to > from)
    .map(([index, [from, to]]) => {
      const run = runs[index]!;
      const display = model.displays[model.displayOfRun[index] ?? -1] ?? null;
      return { run: { ...run, page }, from, to, display };
    });
}

/** Trim `ordered` (run index, [from, to)) to where `quote` begins and ends:
 * its first word of three letters or more, less what came before that word
 * in the quote, and likewise its last. Quads a selection was measured into
 * with estimated letter widths can reach a word or two past it. */
function alignToQuote(
  runs: readonly TextRunDto[],
  ordered: [number, [number, number]][],
  quote: string
): void {
  // the pieces' text, with where each character came from
  let flat = '';
  const origin: { piece: number; k: number }[] = [];
  ordered.forEach(([index, [from, to]], piece) => {
    if (piece > 0) {
      flat += ' ';
      origin.push({ piece, k: from });
    }
    for (let k = from; k < to; k += 1) {
      flat += runs[index]!.text[k];
      origin.push({ piece, k });
    }
  });
  const words = [...quote.matchAll(/[A-Za-z]{3,}/g)];
  const firstWord = words[0];
  const lastWord = words.at(-1);
  if (!firstWord || !lastWord) return;
  const whole = (w: string) => new RegExp(`(?<![A-Za-z])${w}(?![A-Za-z])`, 'g');
  const found = [...flat.matchAll(whole(firstWord[0]))];
  const start = found[0] ? found[0].index - firstWord.index : 0;
  const ends = [...flat.matchAll(whole(lastWord[0]))];
  const tail = quote.length - (lastWord.index + lastWord[0].length);
  const lastEnd = ends.at(-1);
  const end = lastEnd ? lastEnd.index + lastWord[0].length + tail : flat.length;
  // only ever trims, and only near the ends
  let shift = 0;
  if (start > 0 && start < flat.length / 2) {
    shift = start;
    const at = origin[start]!;
    ordered.splice(0, at.piece);
    const head = ordered[0]!;
    head[1] = [Math.max(head[1][0], at.k), head[1][1]];
    origin.splice(0, start);
    for (const o of origin) o.piece -= at.piece;
  }
  const cut = end - shift;
  if (end < flat.length && cut > 0 && cut > origin.length / 2 && cut < origin.length) {
    const at = origin[cut]!;
    ordered.splice(at.piece + 1);
    const tailPiece = ordered[at.piece]!;
    tailPiece[1] = [tailPiece[1][0], Math.min(tailPiece[1][1], Math.max(at.k, tailPiece[1][0]))];
  }
}

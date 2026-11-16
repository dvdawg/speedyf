/** Quoting real pages, for tests: what a comment on a selection of a
 * dumped page (`quote_eval_dump`, src-tauri/src/engine/quote_eval.rs)
 * reads as, through Math to LaTeX's own code run here - the old way (the
 * whole selection at once) and the new (`planQuote`, each formula and
 * display on its own, `fillQuote`). The engine's glyph selection is
 * mirrored (`inRects`); a display's glyphs are the ones the engine grew
 * for it, dumped with the page. Used by quoteEval.test.ts and
 * quoteReal.test.ts only. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import type { TextRunDto } from '../../types/engine';
import type { QuotePiece } from '../../lib/text/copySelection';
import { buildTextModel } from '../../lib/text/textHit';
import { fillQuote, oneFormula, planQuote, prosy } from '../../lib/text/quoteParts';
import { plausibleFormula, plausibleQuoteTex, texParses } from './quoteTexCheck';

interface Glyph {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
  size: number;
  font: string;
  ink: [number, number, number, number] | null;
}
interface Layout {
  glyphs: Glyph[];
  groups: number[][];
  lines: number[][];
}
export interface Dump {
  path: string;
  page: number;
  size: [number, number];
  runs: TextRunDto[];
  displays: { bbox: [number, number, number, number]; number: string | null; layout: Layout }[];
  glyphs: Glyph[];
}
interface Ext {
  selectionMarkdown(
    glyphs: Glyph[],
    opts?: { text?: string; layout?: Layout | null }
  ): { markdown: string } | null;
  reconstruct(glyphs: Glyph[], layout?: Layout | null): { latex: string } | null;
  dropNumber(glyphs: Glyph[], number: string): Glyph[];
  asciiLatex(latex: string): string;
}

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.mathlatex/worker.js'),
  'utf8'
);
const ext = new Function(
  'self',
  `${SOURCE}\nreturn { selectionMarkdown, reconstruct, dropNumber, asciiLatex };`
)({}) as Ext;

/** The engine's `select_in_rects`: glyphs whose centre is inside. */
function inRects(glyphs: Glyph[], rects: number[][]): Glyph[] {
  return glyphs.filter((g) =>
    rects.some(([x, y, w, h]) => {
      if (g.font === '__rule')
        return g.x <= x! + w! && g.x + g.w >= x! && g.y <= y! + h! && g.y + g.h >= y!;
      let cx = g.x + g.w / 2;
      let cy = g.y + g.h / 2;
      if (g.ink && Math.abs(g.ink[1] + g.ink[3] / 2 - cy) > Math.max(1, g.size) / 3) {
        cx = g.ink[0] + g.ink[2] / 2;
        cy = g.ink[1] + g.ink[3] / 2;
      }
      return cx >= x! && cx <= x! + w! && cy >= y! && cy <= y! + h!;
    })
  );
}

async function trusted(tex: string | null, display: boolean): Promise<string | null> {
  if (!tex) return null;
  const wrapped = display ? `$$ ${tex} $$` : `$${tex}$`;
  return plausibleFormula(tex) && (await texParses(wrapped)) ? tex : null;
}

/** Quote runs `first..last` of a dumped page, both ways. */
export async function quote(dump: Dump, first: number, last: number) {
  const H = dump.size[1];
  const model = buildTextModel(
    dump.runs,
    undefined,
    dump.displays.map(({ bbox: [x, y, w, h], number }) => ({ x, y: H - y - h, w, h, number }))
  );
  const pieces: QuotePiece[] = [];
  for (let i = first; i <= last; i += 1) {
    if (model.margin[i]) continue;
    const run = dump.runs[i]!;
    const d = model.displays[model.displayOfRun[i] ?? -1];
    pieces.push({ run: { ...run, page: 0 }, from: 0, to: run.text.length, display: d ?? null });
  }
  const topLeft = ([x, y, w, h]: number[]) => [x!, H - y! - h!, w!, h!];

  // before: the whole selection's glyphs at once
  const rects = pieces.map((p) => topLeft([p.run.x, p.run.y, p.run.w, p.run.h]));
  const plan = planQuote(pieces);
  const before = ext.selectionMarkdown(inRects(dump.glyphs, rects), { text: plan.plain });

  // after: each part on its own
  const tex = new Map<string, string | null>();
  for (const part of plan.inline) {
    const glyphs = inRects(dump.glyphs, part.rects.map(topLeft));
    const read = ext.selectionMarkdown(glyphs, { text: part.text });
    tex.set(part.key, await trusted(oneFormula(read?.markdown), false));
  }
  for (const part of plan.displays) {
    const found = dump.displays.find(
      (d) =>
        Math.abs(d.bbox[0] - part.block[0]) < 0.5 &&
        Math.abs(H - d.bbox[1] - d.bbox[3] - part.block[1]) < 0.5
    );
    let latex: string | null = null;
    if (found && !prosy(part.text)) {
      const glyphs = part.number
        ? ext.dropNumber(found.layout.glyphs, part.number)
        : found.layout.glyphs;
      const read = ext.reconstruct(glyphs, found.layout);
      latex = read?.latex ? ext.asciiLatex(read.latex) : null;
      if (latex && part.number && !latex.includes('\\tag')) latex += ` \\tag{${part.number}}`;
    }
    tex.set(part.key, await trusted(latex, true));
  }
  const after = fillQuote(plan, tex);
  return {
    plain: plan.plain,
    before: before?.markdown ?? null,
    beforeTrusted: before ? plausibleQuoteTex(before.markdown, plan.plain) : false,
    after,
    afterTrusted: plausibleQuoteTex(after, plan.plain),
  };
}

/** The part of a dump runs `first..last` need: those runs, the glyphs
 * around them, and the displays among them. Small enough to keep as a
 * test fixture. */
export function trimDump(dump: Dump, first: number, last: number): Dump {
  const H = dump.size[1];
  const runs = dump.runs.slice(first, last + 1);
  const boxes = runs.map((r) => [r.x - 3, H - r.y - r.h - 3, r.w + 6, r.h + 6]);
  const near = (g: Glyph) =>
    boxes.some(
      ([x, y, w, h]) => g.x + g.w >= x! && g.x <= x! + w! && g.y + g.h >= y! && g.y <= y! + h!
    );
  const displays = dump.displays.filter((d) =>
    runs.some((r) => {
      const [x, y, w, h] = d.bbox;
      const top = H - r.y - r.h;
      return r.x < x + w && r.x + r.w > x && top < y + h && top + r.h > y;
    })
  );
  // to the hundredth of a point, which is all the geometry needs
  const round = (value: unknown): unknown =>
    typeof value === 'number'
      ? Math.round(value * 100) / 100
      : Array.isArray(value)
        ? value.map(round)
        : value && typeof value === 'object'
          ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, round(v)]))
          : value;
  return round({
    ...dump,
    path: dump.path.split('/').at(-1)!,
    runs,
    displays,
    glyphs: dump.glyphs.filter(near),
  }) as Dump;
}

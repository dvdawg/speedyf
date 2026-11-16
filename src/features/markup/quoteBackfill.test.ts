/** Re-reading kept quotes: the pieces under a comment's quads are the ones
 * its selection had (checked on real pages), and the backfill reads each
 * comment that wants it once, one at a time, leaving typo fixes, regions
 * and quotes already read alone. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import type { TextRunDto } from '../../types/engine';
import type { QuotePiece } from '../../lib/text/copySelection';
import { piecesUnderQuads } from '../../lib/text/piecesUnder';
import { buildTextModel } from '../../lib/text/textHit';
import { quadsFromRects } from './geometry';
import { backfillQuotes, quadBox, resetBackfillForTest, wantsReading } from './quoteBackfill';
import type { Dump } from './quoteHarness';
import type { MarkupComment } from './types';

const real = JSON.parse(readFileSync(join(__dirname, 'fixtures/quotes-real.json'), 'utf8')) as {
  dump: Dump;
}[];

/** What a selection of runs `first..last` (whole) collects, and the quads
 * a comment on it keeps. */
function selection(dump: Dump, first: number, last: number) {
  const H = dump.size[1];
  const displays = dump.displays.map(({ bbox: [x, y, w, h], number }) => ({
    x,
    y: H - y - h,
    w,
    h,
    number,
  }));
  const model = buildTextModel(dump.runs, undefined, displays);
  const pieces: QuotePiece[] = [];
  const rects: [number, number, number, number][] = [];
  const boxes = new Set<unknown>();
  for (let i = first; i <= last; i += 1) {
    if (model.margin[i]) continue;
    const run = dump.runs[i]!;
    const d = model.displays[model.displayOfRun[i] ?? -1] ?? null;
    pieces.push({ run: { ...run, page: 0 }, from: 0, to: run.text.length, display: d });
    // as selectedRunRects reports them: a display as its one box
    const [x, y, w, h] = d ? d.box : [run.x, run.y, run.w, run.h];
    if (d && boxes.has(d)) continue;
    boxes.add(d);
    rects.push([x, H - y - h, w, h]);
  }
  return { pieces, quads: quadsFromRects(rects, H), displays };
}

const key = (p: QuotePiece) => `${p.run.start}:${p.run.x}:${p.from}-${p.to}`;

describe('piecesUnderQuads', () => {
  it('finds the pieces the selection had, on real pages', () => {
    for (const { dump } of real) {
      const { pieces, quads, displays } = selection(dump, 0, dump.runs.length - 1);
      const under = piecesUnderQuads(dump.runs, quads.map(quadBox), displays, 0);
      expect(under.map(key)).toEqual(pieces.map(key));
    }
  });

  it('takes part of a run, on whole words', () => {
    const run: TextRunDto = {
      text: 'the bound holds here',
      start: 0,
      x: 100,
      y: 700,
      w: 100,
      h: 10,
      edges: [...Array(21).keys()].map((k) => k * 5),
    };
    // "bound holds", measured a little short at both ends
    const [piece] = piecesUnderQuads([run], [[122, 700, 51, 10]], [], 3);
    expect(piece).toMatchObject({ from: 4, to: 15 });
    expect(piece!.run.page).toBe(3);
  });

  it('trims to the words the comment quoted', () => {
    const runs: TextRunDto[] = [
      { text: 'as before. Without a prior it fails', start: 0, x: 60, y: 700, w: 175, h: 10 },
    ];
    const [piece] = piecesUnderQuads(runs, [[60, 700, 175, 10]], [], 0, 'Without a prior it fails');
    expect(runs[0]!.text.slice(piece!.from, piece!.to)).toBe('Without a prior it fails');
  });
});

function comment(id: string, over: Partial<MarkupComment['anchor']> = {}, kind = 'comment') {
  return {
    id,
    kind,
    anchor: {
      page: 0,
      quads: [{ x1: 0, y1: 10, x2: 50, y2: 10, x3: 0, y3: 0, x4: 50, y4: 0 }],
      rect: null,
      quote: `words of ${id}`,
      ...over,
    },
    body: '',
    replacement: null,
    quickmark: null,
    created: 1,
  } as MarkupComment;
}

describe('backfillQuotes', () => {
  it('reads each comment that wants it once, and nothing else', async () => {
    resetBackfillForTest();
    const comments = [
      comment('plain'),
      comment('typo', {}, 'typo'),
      comment('region', { quads: null, rect: { x: 0, y: 0, w: 10, h: 10 }, quote: '' }),
      comment('read', { quoteTex: 'words of $x$' }),
      comment('garbled', { quoteTex: 'bcy $\\frac{1}{2$ 746' }),
    ];
    expect(comments.map(wantsReading)).toEqual([true, false, false, false, true]);
    const reread = vi.fn(async () => {});
    const host = { comments: () => comments, reread };
    await backfillQuotes('/p.pdf', host);
    expect(reread.mock.calls.map((c) => (c as unknown as [string, MarkupComment])[1].id)).toEqual([
      'plain',
      'garbled',
    ]);
    // a second showing of the review reads nothing again
    await backfillQuotes('/p.pdf', host);
    expect(reread).toHaveBeenCalledTimes(2);
  });

  it('reads one at a time', async () => {
    resetBackfillForTest();
    let running = 0;
    let most = 0;
    const reread = vi.fn(async () => {
      running += 1;
      most = Math.max(most, running);
      await new Promise((resolve) => setTimeout(resolve, 5));
      running -= 1;
    });
    const host = { comments: () => [comment('a'), comment('b'), comment('c')], reread };
    await Promise.all([backfillQuotes('/q.pdf', host), backfillQuotes('/q.pdf', host)]);
    expect(reread).toHaveBeenCalledTimes(3);
    expect(most).toBe(1);
  });
});

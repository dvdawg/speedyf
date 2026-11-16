import { describe, expect, it } from 'vitest';
import {
  buildExportRequest,
  NOTE_SIZES,
  plainContents,
  type BuildInput,
  type MeasureItem,
  type Measured,
} from './exportBuild';
import { quadsFromRects } from './geometry';
import { MARGIN } from './layout';
import type { CommentLabel } from './labels';
import { buildEntries } from './model';
import type { Anchor, MarkupComment } from './types';

const PAGE = { w: 612, h: 792 };

function anchor(page: number, top: number, quote: string): Anchor {
  return { page, quads: quadsFromRects([[72, top, 120, 10]], PAGE.h), rect: null, quote };
}

let n = 0;
function comment(over: Partial<MarkupComment> & { anchor: Anchor }): MarkupComment {
  n += 1;
  return {
    id: `c${n}`,
    kind: 'comment',
    body: '',
    replacement: null,
    quickmark: null,
    created: n,
    ...over,
  };
}

const label = (a: Anchor): CommentLabel => ({
  page: a.page + 1,
  section: null,
  statement: null,
  equation: null,
});

/** A stand-in for the browser: a note's height is its lines at its type
 * size, about half an em a character; a stub is two lines. */
async function measure(items: MeasureItem[]): Promise<Measured[]> {
  return items.map((item) => {
    const size = Number(/font-size:([\d.]+)px/.exec(item.html)?.[1] ?? 9);
    const text = item.html.replace(/<[^>]+>/g, '');
    const perLine = Math.max(1, Math.floor(item.width / (size * 0.5)));
    const lines = item.html.includes('is-stub') ? 2 : Math.max(1, Math.ceil(text.length / perLine));
    const links = [...item.html.matchAll(/data-link="([^"]+)"/g)].map((m) => ({
      key: m[1]!,
      x: 0,
      y: 1,
      w: 12,
      h: 9,
    }));
    return { height: Math.round(lines * size * 1.34 * 100) / 100, links };
  });
}

function input(comments: MarkupComment[], over: Partial<BuildInput> = {}): BuildInput {
  return {
    source: '/papers/draft.pdf',
    dest: '/papers/draft - markup.pdf',
    summary: '',
    entries: buildEntries(comments, label),
    sourcePages: 2,
    pageSize: () => PAGE,
    marginPt: 180,
    math: (tex) => `<k>${tex}</k>`,
    cite: (key) => (key === 'jar97' ? 'Jar97' : null),
    measure,
    ...over,
  };
}

const MEDIUM =
  'The argument here skips the case where the sequence is not bounded; please add a line ' +
  'explaining why the limit exists, or cite the lemma that gives it.';

/** Every note's top in a margin sheet's html. */
const topsOf = (html: string) => [...html.matchAll(/top:([\d.]+)px/g)].map((m) => Number(m[1]));

describe('marks', () => {
  it('highlights comments, strikes typo fixes with a caret, and boxes regions', async () => {
    const list = [
      comment({ anchor: anchor(0, 100, 'the claim'), kind: 'comment', body: 'Justify this step.' }),
      comment({
        anchor: anchor(0, 200, 'teh'),
        kind: 'typo',
        replacement: 'the',
        quickmark: 'typo',
      }),
      comment({ anchor: anchor(0, 300, 'the the'), kind: 'typo', replacement: '' }),
      comment({
        anchor: { page: 1, quads: null, rect: { x: 72, y: 300, w: 200, h: 100 }, quote: '' },
        body: 'Label the axes.',
      }),
    ];
    const request = await buildExportRequest(input(list));
    const native = request.marks.filter((m) => m.kind !== 'label');
    const labels = request.marks.filter((m) => m.kind === 'label');
    // Each phrase also carries its number, in its note's full colour.
    expect(labels.map((m) => [m.page, m.contents, m.color])).toEqual([
      [0, '1', '#694fe3'],
      [0, '2', '#5c6570'],
      [0, '3', '#5c6570'],
      [1, '4', '#694fe3'],
    ]);
    // Just after the last word of "the claim" (x 72..192, top 100 from the top).
    expect(labels[0]!.rect!.x).toBeGreaterThan(192);
    expect(labels[0]!.rect!.x).toBeLessThan(194);
    expect(native.map((m) => [m.page, m.kind])).toEqual([
      [0, 'highlight'],
      [0, 'strikeout'],
      [0, 'caret'],
      [0, 'strikeout'],
      [1, 'square'],
    ]);
    expect(native.map((m) => m.contents)).toEqual([
      '1. Justify this step.',
      '2. Replace "teh" with "the"',
      '2. Insert "the"',
      '3. Delete "the the"',
      '4. Label the axes.',
    ]);
    expect(native[0]!.color).toMatch(/^#[0-9a-f]{6}$/);
    expect(native[0]!.color).not.toBe('#694fe3');
    expect(request.marginPt).toBe(180);
  });

  it('links each number by its phrase to its note, and the note back', async () => {
    const list = [
      comment({ anchor: anchor(0, 100, 'a'), body: 'First.' }),
      comment({ anchor: anchor(0, 400, 'b'), body: 'Second.' }),
      comment({ anchor: anchor(1, 250, 'c'), body: 'Third.' }),
    ];
    const request = await buildExportRequest(input(list));
    const labels = request.marks.filter((m) => m.kind === 'label');
    const margins = request.sheets.filter((s) => s.kind === 'margin');
    for (const [i, mark] of labels.entries()) {
      const sheet = margins.find((s) => s.page === mark.page)!;
      const tops = topsOf(sheet.html);
      const k = labels.filter((m, j) => j < i && m.page === mark.page).length;
      // Same page, just above where its note is set in the strip.
      expect(mark.target).toEqual({ kind: 'page', page: mark.page, y: Math.round(tops[k]! - 12) });
    }
    // The badges: back to their phrase, a little above its line.
    const back = margins.flatMap((s) => s.links!.map((l) => [s.page, l.target]));
    expect(back).toEqual([
      [0, { kind: 'page', page: 0, y: 76 }],
      [0, { kind: 'page', page: 0, y: 376 }],
      [1, { kind: 'page', page: 1, y: 226 }],
    ]);
  });

  it('carries web addresses into the exported notes as links', async () => {
    const c = comment({
      anchor: anchor(0, 100, 'a'),
      body: 'See https://arxiv.org/abs/1234.5678.',
    });
    const request = await buildExportRequest(input([c]));
    expect(request.sheets[0]!.html).toContain(
      '<a href="https://arxiv.org/abs/1234.5678">https://arxiv.org/abs/1234.5678</a>.'
    );
  });

  it('writes plain contents with citations as labels', () => {
    const c = comment({ anchor: anchor(0, 1, 'x'), body: 'Compare  [@jar97]\nand more.' });
    expect(plainContents(4, c, input([c]))).toBe('4. Compare [Jar97] and more.');
  });
});

describe('the margin', () => {
  it('is all there is: one sheet per page with notes, no pages added', async () => {
    const list = [
      comment({ anchor: anchor(0, 100, 'We study'), body: 'Add motivation here. See [@jar97].' }),
      comment({ anchor: anchor(1, 500, 'teh'), kind: 'typo', replacement: 'the' }),
    ];
    const request = await buildExportRequest(input(list));
    expect(request.sheets.map((s) => [s.kind, s.page, s.widthPt, s.heightPt])).toEqual([
      ['margin', 0, 180, 792],
      ['margin', 1, 180, 792],
    ]);
    const all = request.sheets.map((s) => s.html).join('');
    expect(all).toContain('<a class="mk-badge mk-k-comment" data-link="back">1</a>');
    expect(all).toContain('Add motivation here. See <span class="mk-cite">[Jar97]</span>.');
    expect(all).toContain('<span class="mk-del">teh</span> <span class="mk-ins">the</span>');
    // No letter, labels, pointers or kind tags.
    for (const gone of ['Comments on', 'see 1 on', 'cont.', 'mk-kind', 'p. 1', 'Typo']) {
      expect(all).not.toContain(gone);
    }
    expect(all).toMatch(/^[\x20-\x7e]*$/);
  });

  it('adds no page for a realistic dense page of eight medium notes', async () => {
    const list = Array.from({ length: 8 }, (_, i) =>
      comment({ anchor: anchor(0, 90 + i * 70, `phrase ${i}`), body: MEDIUM })
    );
    const request = await buildExportRequest(input(list));
    expect(request.sheets.filter((s) => s.kind !== 'margin')).toEqual([]);
    expect(request.sheets[0]!.html).not.toContain('is-stub');
  });

  it('keeps notes level with their anchors when they fit', async () => {
    const list = [
      comment({ anchor: anchor(0, 150, 'a'), body: 'Short.' }),
      comment({ anchor: anchor(0, 400, 'b'), body: 'Short too.' }),
    ];
    const request = await buildExportRequest(input(list));
    expect(topsOf(request.sheets[0]!.html)).toEqual([149, 399]);
  });

  it('moves colliding notes up as well as down, inside the page', async () => {
    const list = Array.from({ length: 5 }, (_, i) =>
      comment({ anchor: anchor(0, 300 + i, `w${i}`), body: MEDIUM })
    );
    const request = await buildExportRequest(input(list));
    const tops = topsOf(request.sheets[0]!.html);
    expect(tops[0]!).toBeLessThan(299);
    expect(tops.at(-1)!).toBeGreaterThan(303);
    expect(tops[0]!).toBeGreaterThanOrEqual(MARGIN.top);
    expect([...tops].sort((a, b) => a - b)).toEqual(tops);
  });

  it('heads the first page with the summary, notes below it', async () => {
    const list = [
      comment({ anchor: anchor(0, 5, 'Title words'), body: 'A note at the very top.' }),
    ];
    const request = await buildExportRequest(
      input(list, { summary: 'A careful paper; the bound in $x$ is new.' })
    );
    const html = request.sheets[0]!.html;
    expect(html.indexOf('mk-summary')).toBeLessThan(html.indexOf('mk-note'));
    expect(html).toContain('<k>x</k>');
    const [summaryTop, noteTop] = topsOf(html);
    expect(summaryTop).toBe(MARGIN.top);
    expect(noteTop!).toBeGreaterThan(summaryTop! + 10);
  });

  it('puts the summary on page 1 even with no note there', async () => {
    const list = [comment({ anchor: anchor(1, 100, 'x'), body: 'Later.' })];
    const request = await buildExportRequest(input(list, { summary: 'Overall fine.' }));
    expect(request.sheets.map((s) => s.page)).toEqual([0, 1]);
    expect(request.sheets[0]!.html).toContain('Overall fine.');
  });

  it('sets a crowded page smaller before anything else', async () => {
    const list = Array.from({ length: 18 }, (_, i) =>
      comment({ anchor: anchor(0, 40 + i * 40, `w${i}`), body: MEDIUM })
    );
    const request = await buildExportRequest(input(list));
    const html = request.sheets[0]!.html;
    expect(html).not.toContain(`font-size:${NOTE_SIZES[0]}px`);
    expect(request.sheets.every((s) => s.kind === 'margin')).toBe(true);
  });
});

describe('continuation, the last resort', () => {
  it('cuts the longest notes to a line and carries their text to one page at the end', async () => {
    const long = `${MEDIUM} ${MEDIUM} ${MEDIUM}`;
    const list = Array.from({ length: 24 }, (_, i) =>
      comment({ anchor: anchor(0, 30 + i * 30, `w${i}`), body: i % 2 ? long : 'Short.' })
    );
    const request = await buildExportRequest(input(list));
    const added = request.sheets.filter((s) => s.kind === 'feedback');
    expect(added).toHaveLength(1);
    // Added pages come first in the request, so a link's sheet is its index.
    expect(request.sheets[0]!.kind).toBe('feedback');
    const margin = request.sheets.find((s) => s.kind === 'margin')!;
    expect(margin.html).toContain('is-stub');
    // The paper has two pages, so the continuation prints as page 3.
    expect(margin.html).toContain('(cont. p. 3)');
    const onward = margin.links!.filter((l) => l.target.kind === 'sheet');
    expect(onward.length).toBe(margin.html.split('(cont. p. 3)').length - 1);
    for (const link of onward) expect(link.target).toMatchObject({ kind: 'sheet', sheet: 0 });
    // Every badge, cut or not, still goes back to its phrase.
    expect(margin.links!.filter((l) => l.target.kind === 'page')).toHaveLength(24);
    expect(added[0]!.html).toContain('Notes continued from p. 1');
    expect(added[0]!.html).not.toContain('Short.');
    for (const link of added[0]!.links!)
      expect(link.target).toMatchObject({ kind: 'page', page: 0 });
    const tops = topsOf(margin.html);
    expect(Math.max(...tops)).toBeLessThan(PAGE.h - MARGIN.bottom);
  });
});

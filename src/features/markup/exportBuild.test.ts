import { describe, expect, it } from 'vitest';
import {
  buildExportRequest,
  feedbackColumn,
  FEEDBACK,
  plainContents,
  type BuildInput,
  type MeasureItem,
  type Measured,
} from './exportBuild';
import { quadsFromRects } from './geometry';
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
    kind: 'exposition',
    body: '',
    replacement: null,
    quickmark: null,
    created: n,
    ...over,
  };
}

function label(a: Anchor): CommentLabel {
  return {
    page: a.page + 1,
    section: a.page === 0 ? null : { title: '2 Method', short: 'Sec. 2', page: 1, y: 700 },
    statement: a.page === 1 ? 'Lemma 2.3' : null,
    equation: null,
  };
}

/** A stand-in for the browser: fixed heights by kind of block, and every
 * `data-link` at the block's top-left. */
function fakeMeasure(heights: Partial<Record<string, number>> = {}) {
  const seen: MeasureItem[] = [];
  const measure = async (items: MeasureItem[]): Promise<Measured[]> => {
    seen.push(...items);
    return items.map((item) => {
      const kind =
        item.sheet === 'margin'
          ? item.html.includes('is-tiny')
            ? 'tiny'
            : item.html.includes('is-short')
              ? 'short'
              : 'full'
          : (/mk-block mk-([a-z]+)/.exec(item.html)?.[1] ?? 'other');
      const defaults: Record<string, number> = { full: 30, short: 20, tiny: 10, entry: 60 };
      const height = heights[kind] ?? defaults[kind] ?? 24;
      const links = [...item.html.matchAll(/data-link="([^"]+)"/g)].map((m, i) => ({
        key: m[1]!,
        x: 1,
        y: 2 + i,
        w: 10,
        h: 10,
      }));
      return { height, links };
    });
  };
  return { measure, seen };
}

function input(comments: MarkupComment[], over: Partial<BuildInput> = {}): BuildInput {
  return {
    source: '/papers/draft.pdf',
    dest: '/papers/draft - markup.pdf',
    title: 'On <Things> & $x$',
    date: 'November 15, 2026',
    summary: 'A careful paper. See [@jar97].',
    entries: buildEntries(comments, label),
    sourcePages: 2,
    pageSize: () => PAGE,
    marginPt: 180,
    math: (tex) => `<k>${tex}</k>`,
    cite: (key) => (key === 'jar97' ? 'Jar97' : null),
    reference: (key) => (key === 'jar97' ? 'Jarvis. On things. 1997.' : null),
    measure: fakeMeasure().measure,
    ...over,
  };
}

describe('marks', () => {
  it('highlights comments, strikes typo fixes with a caret, and boxes regions', async () => {
    const list = [
      comment({ anchor: anchor(0, 100, 'the claim'), kind: 'proof', body: 'Justify this step.' }),
      comment({
        anchor: anchor(0, 200, 'teh'),
        kind: 'typo',
        replacement: 'the',
        quickmark: 'typo',
      }),
      comment({
        anchor: { page: 1, quads: null, rect: { x: 72, y: 300, w: 200, h: 100 }, quote: '' },
        body: 'Label the axes.',
      }),
    ];
    const request = await buildExportRequest(input(list));
    expect(request.marks.map((m) => [m.page, m.kind])).toEqual([
      [0, 'highlight'],
      [0, 'strikeout'],
      [0, 'caret'],
      [1, 'square'],
    ]);
    expect(request.marks[0]!.contents).toBe('1. Justify this step.');
    expect(request.marks[1]!.contents).toBe('2. Replace "teh" with "the"');
    expect(request.marks[2]!.contents).toBe('2. Insert "the"');
    expect(request.marks[3]!.contents).toBe('3. Label the axes.');
    // Soft tints of the kind colours, a strike in the typo grey.
    expect(request.marks[0]!.color).toMatch(/^#[0-9a-f]{6}$/);
    expect(request.marks[0]!.color).not.toBe('#c62828');
    expect(request.marks[1]!.color).toBe('#5c6570');
    expect(request.source).toBe('/papers/draft.pdf');
    expect(request.dest).toBe('/papers/draft - markup.pdf');
    expect(request.marginPt).toBe(180);
  });

  it('writes plain contents with citations as labels', () => {
    const c = comment({ anchor: anchor(0, 1, 'x'), body: 'Compare  [@jar97]\nand more.' });
    expect(plainContents(4, c, input([c]))).toBe('4. Compare [Jar97] and more.');
  });
});

describe('sheets', () => {
  const list = [
    comment({ anchor: anchor(0, 100, 'We study'), body: 'Add motivation here.' }),
    comment({
      anchor: anchor(1, 300, 'by compactness'),
      kind: 'proof',
      quickmark: 'justify',
      body: 'Justify this step.',
    }),
    comment({ anchor: anchor(1, 500, 'teh'), kind: 'typo', replacement: 'the' }),
    comment({
      anchor: anchor(1, 600, 'by compactness'),
      kind: 'proof',
      quickmark: 'justify',
      body: 'Justify this step.',
    }),
  ];

  it('sends the feedback pages first, then one margin per page with comments', async () => {
    const request = await buildExportRequest(input(list));
    const kinds = request.sheets.map((s) => [s.kind, s.page ?? null, s.widthPt, s.heightPt]);
    expect(kinds).toEqual([
      ['feedback', null, 792, 792],
      ['margin', 0, 180, 792],
      ['margin', 1, 180, 792],
    ]);
    const feedback = request.sheets[0]!.html;
    expect(feedback).toContain(
      'Comments on On &lt;Things&gt; &amp; <span class="mk-math"><k>x</k></span>'
    );
    expect(feedback).toContain('Overall');
    expect(feedback).toContain('<span class="mk-cite">[Jar97]</span>');
    expect(feedback).toContain('Typos and wording');
    expect(feedback).toContain('References');
    expect(feedback).toContain('Jarvis. On things. 1997.');
    // The repeat is folded into its first occurrence.
    expect(feedback).toContain('Also <a class="mk-link" data-link="also0">p. 2</a>.');
    expect(feedback.match(/class="mk-num"/g)).toHaveLength(2);
    expect(feedback).toMatch(/^[\x20-\x7e]*$/);
  });

  it('links each entry number to its place on the page', async () => {
    const request = await buildExportRequest(input(list));
    const targets = request.sheets[0]!.links.map((l) => l.target);
    expect(targets).toContainEqual({ kind: 'page', page: 0, y: 76 });
    expect(targets).toContainEqual({ kind: 'page', page: 1, y: 276 });
    // The folded repeat's "also" link.
    expect(targets).toContainEqual({ kind: 'page', page: 1, y: 576 });
    for (const link of request.sheets[0]!.links) {
      expect(link.rect.x).toBeGreaterThanOrEqual(feedbackColumn(792).left);
      expect(link.rect.y).toBeGreaterThanOrEqual(FEEDBACK.top);
    }
  });

  it('links each margin note to its entry in the letter, the repeat to the first', async () => {
    const request = await buildExportRequest(input(list));
    const margin = request.sheets[2]!;
    expect(margin.links).toHaveLength(3);
    for (const link of margin.links) expect(link.target).toMatchObject({ kind: 'sheet', sheet: 0 });
    const ys = margin.links.map((l) => (l.target as { y: number }).y);
    expect(ys[0]).toBe(ys[2]);
    expect(margin.html).toContain('mk-margin');
    expect(margin.html).toContain(
      '<span class="mk-del">teh</span> <span class="mk-ins">the</span>'
    );
  });

  it('places the notes level with their anchors', async () => {
    const request = await buildExportRequest(input(list));
    const tops = [...request.sheets[2]!.html.matchAll(/top:([\d.]+)px/g)].map((m) => Number(m[1]));
    expect(tops).toEqual([299, 499, 599]);
  });

  it('shortens notes that do not fit and points to the letter for the rest', async () => {
    const crowded = Array.from({ length: 12 }, (_, i) =>
      comment({ anchor: anchor(0, 60 + i * 50, `w${i}`), body: `Long remark ${i}.` })
    );
    const { measure } = fakeMeasure({ full: 90, short: 40, tiny: 12 });
    const request = await buildExportRequest(input(crowded, { measure }));
    const margin = request.sheets.find((s) => s.kind === 'margin')!;
    expect(margin.html).toContain('is-short');
    // Feedback page 1 follows the two pages of the paper: page 3.
    expect(margin.html).toMatch(/\.\.\. see \d+ on p\. 3/);
    const tops = [...margin.html.matchAll(/top:([\d.]+)px/g)].map((m) => Number(m[1]));
    expect(Math.max(...tops)).toBeLessThan(792);
  });

  it('breaks the letter over pages and repeats the typo table head', async () => {
    const typos = Array.from({ length: 30 }, (_, i) =>
      comment({ anchor: anchor(0, 20 + i * 20, `wrod${i}`), kind: 'typo', replacement: `word${i}` })
    );
    const { measure } = fakeMeasure({ trow: 40 });
    const request = await buildExportRequest(input(typos, { measure, summary: '' }));
    const feedback = request.sheets.filter((s) => s.kind === 'feedback');
    expect(feedback.length).toBeGreaterThan(1);
    expect(feedback[1]!.html).toContain('mk-thead is-continued');
    expect(feedback[0]!.html).toContain(`1 of ${feedback.length}`);
    expect(feedback.at(-1)!.html).toContain(`${feedback.length} of ${feedback.length}`);
    // Margin notes say which page of the letter to see.
    const margin = request.sheets.find((s) => s.kind === 'margin')!;
    const sheetTargets = new Set(margin.links.map((l) => (l.target as { sheet: number }).sheet));
    expect(sheetTargets.size).toBeGreaterThan(1);
  });
});

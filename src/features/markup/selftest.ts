/** Development builds only: the markup self-test's frontend half.
 *
 * `SPEEDYF_MARKUP_SELFTEST` makes the Rust side ask the main window to
 * export a synthetic review of a copy of a paper through the REAL path: the
 * file opened in a tab, the review laid out by `buildExportRequest` with its
 * real measurement in print.css, written by `markup_export`. The report
 * carries where each comment was anchored, so the output can be checked. */
import { activeTab } from '../../stores/tabsStore';
import { openInNewTabOrFocus } from '../document/tabsController';
import { markupSelftestApi } from '../../lib/transport/markup';
import { exportReview, sourcePageCount } from './exportActions';
import { MARKUP_VERSION, type MarkupComment, type MarkupDoc, type Quad } from './types';

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

async function openAndWait(path: string): Promise<void> {
  await openInNewTabOrFocus(path);
  for (let i = 0; i < 300; i++) {
    const doc = activeTab()?.documentStore.state;
    if (doc?.loaded && doc.path === path && doc.pages.length > 0) return;
    await sleep(100);
  }
  throw new Error(`the paper did not open: ${path}`);
}

/** One line of "text" `top` pt below the top of a page `h` tall. */
function line(h: number, top: number, x: number, w: number): Quad {
  const y1 = h - top;
  const y3 = y1 - 10;
  return { x1: x, y1, x2: x + w, y2: y1, x3: x, y3, x4: x + w, y4: y3 };
}

export interface SelftestAnchor {
  number: number;
  page: number;
  /** Display pt from the page top to the anchor's top edge. */
  top: number;
  kind: string;
}

/** A review touching every kind of comment, math, a typo fix, a region. */
export function syntheticReview(size: (page: number) => { w: number; h: number }): {
  doc: MarkupDoc;
  anchors: SelftestAnchor[];
} {
  const comments: MarkupComment[] = [];
  const anchors: SelftestAnchor[] = [];
  let created = 1_700_000_000_000;
  const add = (c: Omit<MarkupComment, 'id' | 'created' | 'quickmark'>, top: number) => {
    comments.push({
      ...c,
      id: `c_selftest${comments.length}`,
      created: created++,
      quickmark: null,
    });
    anchors.push({ number: comments.length, page: c.anchor.page, top, kind: c.kind });
  };
  const p0 = size(0);
  const p1 = size(Math.min(1, 1));
  // Places are given for a letter page and scaled to the page's height, so
  // a landscape (or turned) copy keeps every anchor on its page.
  const t0 = (top: number) => Math.round((top * p0.h) / 792);
  const t1 = (top: number) => Math.round((top * p1.h) / 792);
  add(
    {
      kind: 'comment',
      anchor: {
        page: 0,
        quads: [line(p0.h, t0(150), 90, 200)],
        rect: null,
        quote: 'by induction on n',
      },
      body: 'The induction step needs $n \\ge 2$; check $$\\sum_{k=1}^n k = \\frac{n(n+1)}{2}$$ holds at the base.',
      replacement: null,
    },
    t0(150)
  );
  add(
    {
      kind: 'comment',
      anchor: {
        page: 0,
        quads: [line(p0.h, t0(420), 90, 300), line(p0.h, t0(432), 90, 120)],
        rect: null,
        quote: 'it is clear that',
      },
      body:
        'This paragraph would read better if the definition came first (see https://arxiv.org/abs/2401.00001). ' +
        'As written the reader meets the notation before it is introduced, and has to go back.',
      replacement: null,
    },
    t0(420)
  );
  add(
    {
      kind: 'typo',
      anchor: { page: 0, quads: [line(p0.h, t0(600), 120, 30)], rect: null, quote: 'teh' },
      body: '',
      replacement: 'the',
    },
    t0(600)
  );
  add(
    {
      kind: 'question',
      anchor: {
        page: 1,
        quads: [line(p1.h, t1(200), 90, 220)],
        rect: null,
        quote: 'as shown before',
      },
      body: 'Cite the original source here, e.g. [@selftest2020]; the bound $O(\\log n)$ is older.',
      replacement: null,
    },
    t1(200)
  );
  add(
    {
      kind: 'comment',
      anchor: {
        page: 1,
        quads: null,
        rect: { x: 80, y: p1.h - t1(560), w: 300, h: t1(160) },
        quote: '',
      },
      body: 'This figure needs axis labels.',
      replacement: null,
    },
    t1(400)
  );
  add(
    {
      kind: 'typo',
      anchor: { page: 1, quads: [line(p1.h, t1(650), 100, 60)], rect: null, quote: 'recieve' },
      body: 'spelling',
      replacement: 'receive',
    },
    t1(650)
  );
  return {
    doc: {
      version: MARKUP_VERSION,
      summary:
        'A careful paper. The main result, a bound of $\\alpha^2$ on the error, is new; ' +
        'the exposition in Section 2 could be tightened.',
      comments,
    },
    anchors,
  };
}

async function run(source: string, dest: string): Promise<void> {
  try {
    await openAndWait(source);
    const doc = activeTab()!.documentStore.state;
    const size = (page: number) => {
      const p = doc.pages.find((x) => x.srcIndex === page) ?? doc.pages[0]!;
      return { w: p.widthPt, h: p.heightPt };
    };
    const { doc: review, anchors } = syntheticReview(size);
    const result = await exportReview(source, review, dest);
    // A normal review is the paper, widened: never a page more.
    const pages = sourcePageCount(doc.pages);
    if (result.pages !== pages) {
      throw new Error(`${result.pages} pages written for a ${pages}-page paper`);
    }
    await markupSelftestApi.done({
      ok: true,
      message: `${result.pages} pages, ${result.links} links`,
      detail: { result, anchors, sizes: [size(0), size(1)] },
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : JSON.stringify(error);
    await markupSelftestApi.done({ ok: false, message });
  }
}

export async function installMarkupSelftest(): Promise<void> {
  await markupSelftestApi.onRun(({ source, dest }) => void run(source, dest));
  await markupSelftestApi.ready();
}

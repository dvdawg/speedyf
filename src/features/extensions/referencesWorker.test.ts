/** Drives the shipped References worker the way the host does.
 *
 * The worker is plain JavaScript with a `postMessage` protocol, so it can be
 * loaded into a fake `self` and answered with canned API results. That makes
 * the document-switch path testable end to end without a webview: this is the
 * one piece the component tests cannot reach, and the one where switching
 * documents actually goes wrong.
 *
 * The documents below are small on purpose but not simplified: a reference
 * list is only found at all if the destinations, the printed lines and the
 * heading agree with each other, so a fixture that omits any of them tests a
 * worker that could not read a real paper.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it } from 'vitest';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.references/worker.js'),
  'utf8'
);

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}
interface Link {
  uri: string;
  host: string | null;
  page: number;
  citation: { kind: string; value: string; page: number } | null;
  box: Box;
}
interface Jump {
  page: number;
  box: Box;
  toPage: number;
  toX: number | null;
  toY: number | null;
}
interface Page {
  links?: Link[];
  jumps?: Jump[];
  runs?: { text: string; box: Box }[];
}
interface Doc {
  name: string;
  pages: Page[];
  /** Identifiers the library holds, so a row can be badged "have it". */
  library?: Record<string, string>;
}

/** One line of a page, at `y`, starting at the left margin. */
function line(text: string, y: number, x = 50): { text: string; box: Box } {
  return { text, box: { x, y, w: Math.min(text.length * 4.6, 440), h: 9 } };
}

/** A citation of the entry anchored at `toY`, printed on page `from` at `y` —
 * which has to be a line that page actually carries, since that line is what
 * the opened row shows. */
function cite(from: number, toPage: number, toY: number, y = 700, x = 100, w = 10): Jump {
  return { page: from, box: { x, y, w, h: 9 }, toPage, toX: 46, toY };
}

/** A paper whose bibliography SpeedyF can read: four entries on page 1, each
 * with a destination three points above its first line, cited from the body. */
const TYPESET: Doc = {
  name: 'typeset.pdf',
  library: { '2101.00001': 'On the Analytical Engine' },
  pages: [
    {
      runs: [line('We build on earlier work [1], [2].', 700)],
      jumps: [
        // The first citation is printed as two links with a space between
        // them, the way an author-year style sets "Saxe et al., 2014".
        cite(0, 1, 703, 700, 100, 10),
        cite(0, 1, 703, 700, 113, 8),
        cite(0, 1, 653),
        cite(0, 1, 613),
      ],
    },
    {
      runs: [
        // Body text above the list, on the page the list begins on.
        line('The bound follows from [2] directly.', 760),
        line('References', 720),
        line('[1] Ada Lovelace and Charles Babbage. On the analytical engine.', 700),
        line('Journal of Engines, 1843.', 688),
        line('[2] Grace Hopper. A compiler for business. ACM, 1952.', 650),
        line('[3] Alan Turing. Computing machinery and intelligence. Mind, 1950.', 610),
        line('[4] Katherine Johnson. Orbital mechanics by hand. NASA, 1961.', 570),
      ],
      links: [
        {
          uri: 'https://arxiv.org/abs/2101.00001',
          host: 'arxiv.org',
          page: 1,
          citation: { kind: 'arxiv', value: '2101.00001', page: 1 },
          box: { x: 60, y: 688, w: 120, h: 9 },
        },
      ],
      jumps: [
        // A citation printed above the list, on the same page as the list.
        cite(1, 1, 653, 760),
        // A "see also [4]" inside the reference list itself. It anchors entry
        // 4 — which is how the entry is found at all — without being a place
        // the work is cited.
        cite(1, 1, 573),
      ],
    },
    {
      // A citation broken across a line break: the end of one line and the
      // start of the next, one work, one place.
      runs: [line('As shown by the argument of [1,', 700), line('Theorem 2].', 688)],
      jumps: [cite(2, 1, 703, 700, 500, 10), cite(2, 1, 703, 688, 50, 8)],
    },
  ],
};

/** A paper with no internal links at all — an older scan, or anything from a
 * word processor. Only the printed addresses are available. */
const UNLINKED: Doc = {
  name: 'unlinked.pdf',
  pages: [
    { runs: [line('Body text.', 700)], links: [], jumps: [] },
    {
      runs: [line('References', 720)],
      links: [
        {
          uri: 'https://arxiv.org/abs/2202.00002',
          host: 'arxiv.org',
          page: 1,
          citation: { kind: 'arxiv', value: '2202.00002', page: 1 },
          box: { x: 50, y: 700, w: 120, h: 9 },
        },
        {
          uri: 'https://jmlr.org/papers/v20/18-598.html',
          host: 'jmlr.org',
          page: 1,
          citation: null,
          box: { x: 50, y: 680, w: 120, h: 9 },
        },
      ],
      jumps: [],
    },
  ],
};

/** A running worker with the host on the other end of the pipe. */
function startWorker(
  docs: Record<string, Doc>,
  settings: Record<string, string> = {},
  /** A scan written by an older build, answered for whatever key is asked
   * for — which is what a document with a cache from a previous release
   * looks like from in here. */
  staleCache?: unknown
) {
  const storage = new Map<string, string>();
  const drawn: unknown[] = [];
  const visited: string[] = [];
  let open: string | null = null;

  const fake = {
    onmessage: undefined as ((e: { data: unknown }) => void) | undefined,
    postMessage(message: Record<string, unknown>) {
      if (message.type !== 'call') return;
      void answer(message);
    },
  };

  async function answer(message: Record<string, unknown>) {
    const args = (message.args ?? {}) as Record<string, unknown>;
    const doc = open ? docs[open] : undefined;
    const page = doc?.pages[Number(args.page)];
    let value: unknown = null;
    let ok = true;
    switch (message.api) {
      case 'document.info':
        if (!doc) ok = false;
        else value = { name: doc.name, pageCount: doc.pages.length, currentPage: 0 };
        break;
      case 'document.links':
        value = page?.links ?? [];
        break;
      case 'document.jumps':
        value = page?.jumps ?? [];
        break;
      case 'document.pageRuns':
        value = page?.runs ?? [];
        break;
      case 'citations.resolve': {
        const title = doc?.library?.[String(args.value)];
        value = title
          ? { inLibrary: true, fileName: `${String(args.value)}.pdf`, title, pageCount: 9 }
          : { inLibrary: false, fileName: null, title: null, pageCount: null };
        break;
      }
      case 'navigate.toPage':
        visited.push(`page ${Number(args.page)}`);
        break;
      case 'navigate.toPlace': {
        const boxes = (args.boxes ?? []) as { x: number; y: number; w: number; h: number }[];
        visited.push(
          `page ${Number(args.page)} marking ${
            boxes.length === 0
              ? `y=${Number(args.y)}`
              : boxes.map((b) => `${b.x},${b.y} ${b.w}x${b.h}`).join(' + ')
          }`
        );
        break;
      }
      case 'settings.get':
        value = settings[String(args.key)] ?? (args.key === 'sort' ? 'appearance' : 'false');
        break;
      case 'storage.get':
        value =
          staleCache !== undefined
            ? JSON.stringify(staleCache)
            : (storage.get(String(args.key)) ?? null);
        break;
      case 'storage.set':
        storage.set(String(args.key), String(args.value));
        break;
      case 'ui.render':
        drawn.push(args.ui);
        break;
      default:
        value = null;
    }
    fake.onmessage?.({
      data: { type: 'reply', requestId: message.requestId, ok, value, error: 'no document' },
    });
  }

  new Function('self', SOURCE)(fake);

  return {
    drawn,
    visited,
    openDocument(key: string | null) {
      open = key;
    },
    /** One `invoke`, resolved when the worker reports the command finished. */
    async invoke(command: string, value?: string) {
      return new Promise<void>((resolve, reject) => {
        const requestId = 10_000 + drawn.length + visited.length;
        const post = fake.postMessage.bind(fake);
        fake.postMessage = (message: Record<string, unknown>) => {
          if (message.type === 'done' && message.requestId === requestId) {
            fake.postMessage = post;
            if (message.ok) resolve();
            else reject(new Error(String(message.error)));
            return;
          }
          post(message);
        };
        fake.onmessage?.({ data: { type: 'invoke', command, requestId, value } });
      });
    },
  };
}

interface Row {
  title: string;
  subtitle?: string;
  badge?: string;
  nested?: boolean;
  action?: { command: string; value: string };
}

function rowsIn(ui: unknown): Row[] {
  const description = ui as { sections?: { rows?: Row[] }[] };
  return (description.sections ?? []).flatMap((s) => s.rows ?? []);
}

const DOCS: Record<string, Doc> = {
  typeset: TYPESET,
  unlinked: UNLINKED,
  // Two papers that differ only in what they cite, for the switch tests.
  a: { ...UNLINKED, name: 'paper-a.pdf' },
  b: {
    ...UNLINKED,
    name: 'paper-b.pdf',
    pages: [
      UNLINKED.pages[0]!,
      {
        ...UNLINKED.pages[1]!,
        links: [
          {
            ...UNLINKED.pages[1]!.links![0]!,
            uri: 'https://arxiv.org/abs/2303.00003',
            citation: { kind: 'arxiv', value: '2303.00003', page: 1 },
          },
        ],
      },
    ],
  },
};

describe('References worker: reading a typeset bibliography', () => {
  let worker: ReturnType<typeof startWorker>;

  beforeEach(() => {
    worker = startWorker(DOCS);
    worker.openDocument('typeset');
  });

  it('reads a title and its authors out of the entry, not out of the URL', async () => {
    await worker.invoke('refs');
    const rows = rowsIn(worker.drawn.at(-1));
    expect(rows.map((row) => row.title)).toContain('On the analytical engine');
    expect(rows[0]!.subtitle).toBe('Ada Lovelace and Charles Babbage (1843), cited on pages 1, 3');
  });

  /** The whole point: a reference is reported where the body cites it, not
   * where the bibliography prints it. Entry 1 is cited from pages 0 and 2, and
   * is printed on page 1 — which must not appear. */
  it('reports the pages a work is cited on, not the page it is listed on', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    expect(first.subtitle).toContain('cited on pages 1, 3');
    expect(first.subtitle).not.toContain('listed on');
    // Page 2 is where the entry is *printed*, and must not be reported as a
    // place the work is cited.
    expect(first.subtitle).not.toContain('2');
  });

  /** A "see [4]" printed inside the bibliography points at an entry the same
   * way a citation in the body does. It is not one, and counting it would put
   * the reference page itself in the list of places a work is cited. */
  it('does not count a cross-reference between entries as a citation', async () => {
    await worker.invoke('refs');
    const rows = rowsIn(worker.drawn.at(-1));
    const crossReferenced = rows.find((row) => row.title.includes('Orbital mechanics'))!;
    expect(crossReferenced.subtitle).toContain('listed on');
    expect(crossReferenced.subtitle).not.toContain('cited on');
  });

  it('badges a reference the library already holds', async () => {
    await worker.invoke('refs');
    const rows = rowsIn(worker.drawn.at(-1));
    expect(rows.find((row) => row.badge === 'have it')?.title).toBe('On the analytical engine');
  });

  it('leaves the section heading out of the list', async () => {
    await worker.invoke('refs');
    const rows = rowsIn(worker.drawn.at(-1));
    expect(rows.map((row) => row.title)).not.toContain('References');
    expect(rows).toHaveLength(4);
  });

  /** A row that jumped somewhere on activation had to choose one of the places
   * the work is cited, and any choice is wrong the rest of the time. */
  it('opens a reference into the places it is cited', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);

    const rows = rowsIn(worker.drawn.at(-1));
    // The two citation sites, listed under the reference they belong to.
    expect(rows[1]!.subtitle).toBe('page 1');
    expect(rows[1]!.nested).toBe(true);
    expect(rows[0]!.nested).toBeUndefined();
    expect(rows[2]!.subtitle).toBe('page 3');
    expect(rows[3]!.title).toBe('A compiler for business');
  });

  /** Each opened site shows the line it appears on, so you can tell the five
   * places a work is cited apart before going to any of them. */
  it('shows the line each citation sits on', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);
    expect(rowsIn(worker.drawn.at(-1))[1]!.title).toBe('We build on earlier work [1], [2].');
  });

  /** Opening a row adds a row for each place the work is cited. A heading that
   * counted rows then said the paper cited five more works than a moment ago. */
  it('counts references in the heading, not rows', async () => {
    const grouped = startWorker(DOCS, { group: 'true' });
    grouped.openDocument('typeset');
    await grouped.invoke('refs');
    const titleOf = (ui: unknown) =>
      ((ui as { sections?: { title?: string }[] }).sections ?? []).map((s) => s.title).join(' ');
    const before = titleOf(grouped.drawn.at(-1));
    const first = rowsIn(grouped.drawn.at(-1))[0]!;
    await grouped.invoke('jump', first.action!.value);
    expect(titleOf(grouped.drawn.at(-1))).toBe(before);
  });

  it('closes again when the reference is activated a second time', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);
    expect(rowsIn(worker.drawn.at(-1))).toHaveLength(6);
    await worker.invoke('jump', first.action!.value);
    expect(rowsIn(worker.drawn.at(-1))).toHaveLength(4);
  });

  /** "Cited on page 4" is a page number. Being put on the sentence that cites
   * the work is a destination, and it is the point of following positions. */
  it('goes to where the citation is printed, not to the top of its page', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);
    const site = rowsIn(worker.drawn.at(-1))[1]!;
    await worker.invoke('goto', site.action!.value);
    expect(worker.visited[0]).toContain('page 0 marking');
  });

  /** An author-year style sets one citation as two links — "Saxe et al.," and
   * "2014" — with the comma between them belonging to neither. Marking one of
   * them draws a box around half a name, and listing both makes two rows that
   * say the same page. */
  it('marks a citation split into two links as one thing, comma and all', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);
    const site = rowsIn(worker.drawn.at(-1))[1]!;
    await worker.invoke('goto', site.action!.value);
    // One mark, spanning both pieces and the gap between them: the pieces are
    // 100..110 and 113..121, so anything narrower has left something out.
    expect(worker.visited).toEqual(['page 0 marking 100,700 21x9']);
  });

  /** A citation broken across a line break is still one citation — but it is
   * in two places on the page, so it takes two marks rather than one box
   * spanning everything between them. */
  it('marks a citation broken across a line in both places', async () => {
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);
    const onPageThree = rowsIn(worker.drawn.at(-1)).find((row) =>
      row.subtitle?.includes('page 3')
    )!;
    await worker.invoke('goto', onPageThree.action!.value);
    expect(worker.visited).toEqual(['page 2 marking 500,700 10x9 + 50,688 8x9']);
  });

  /** A bibliography often begins halfway down a page whose top half is still
   * body text. Skipping such a page whole threw away every citation printed
   * above the list, and the panel looked like it had the wrong pages. */
  it('counts a citation printed above the list on the list’s own page', async () => {
    await worker.invoke('refs');
    const second = rowsIn(worker.drawn.at(-1)).find((row) =>
      row.title.includes('A compiler for business')
    )!;
    expect(second.subtitle).toContain('cited on pages 1, 2');
  });
});

describe('References worker: a document with no internal links', () => {
  let worker: ReturnType<typeof startWorker>;

  beforeEach(() => {
    worker = startWorker(DOCS);
  });

  /** Nothing in the file says where these are cited, and the panel must not
   * imply otherwise by reporting the page they are printed on as though it
   * were a citation site. */
  it('falls back to the printed links and says only where they are listed', async () => {
    worker.openDocument('unlinked');
    await worker.invoke('refs');
    const rows = rowsIn(worker.drawn.at(-1));
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      expect(row.subtitle).toContain('listed on page 2');
      expect(row.subtitle).not.toContain('cited on');
    }
  });

  /** There is nothing to open, so activating the row does the only useful
   * thing left: goes to where the work is listed. */
  it('goes to the listing when there are no citation sites to show', async () => {
    worker.openDocument('unlinked');
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await worker.invoke('jump', first.action!.value);
    expect(worker.visited).toEqual(['page 1']);
  });

  it('lists works cited by URL alone, not only the ones it can identify', async () => {
    worker.openDocument('unlinked');
    await worker.invoke('refs');
    expect(rowsIn(worker.drawn.at(-1)).map((row) => row.title)).toContain(
      'jmlr.org/papers/v20/18-598.html'
    );
  });
});

describe('References worker: what reaches the panel', () => {
  /** Typographic punctuation is how a typesetter sets a page, not how a list
   * should read: it varies with whatever font the PDF happened to use, so the
   * same title arrives looking different from two papers. */
  it('draws nothing but plain characters', async () => {
    const worker = startWorker(DOCS);
    worker.openDocument('typeset');
    await worker.invoke('refs');
    const drawn = JSON.stringify(worker.drawn.at(-1));
    // eslint-disable-next-line no-control-regex
    expect(drawn).not.toMatch(/[^\u0000-\u007f]/);
  });
});

/** The cache is keyed by a version number that whoever changes the scan is
 * supposed to bump. That promise was broken once already: `sites` changed
 * shape under a number that had been bumped for a different reason, and every
 * document with a cache stopped responding to clicks — the row was built by
 * code reading a field that was no longer there, and the throw only reached
 * the extension log.
 *
 * So the shape is checked rather than trusted, and these are the tests that
 * hold that. They deliberately do not name the version: seeding the cache for
 * whatever key is asked for is what makes them independent of it. */
describe('References worker: a cache from an older build', () => {
  const OLD_SHAPE = {
    name: 'typeset.pdf',
    pageCount: 3,
    readBy: 'entries',
    citations: [
      {
        key: 'stale',
        title: 'A reference from the old cache',
        pages: [1],
        citedOn: [0],
        citations: 1,
        // What the previous build wrote: one box per site, no `rects`.
        sites: [{ page: 0, x: 100, y: 700, w: 10, h: 9 }],
        inLibrary: false,
      },
    ],
  };

  it('rescans rather than drawing a scan it cannot use', async () => {
    const worker = startWorker(DOCS, {}, OLD_SHAPE);
    worker.openDocument('typeset');
    await worker.invoke('refs');
    const titles = rowsIn(worker.drawn.at(-1)).map((row) => row.title);
    expect(titles).not.toContain('A reference from the old cache');
    expect(titles).toContain('On the analytical engine');
  });

  it('still opens a reference, rather than failing where the row is built', async () => {
    const worker = startWorker(DOCS, {}, OLD_SHAPE);
    worker.openDocument('typeset');
    await worker.invoke('refs');
    const first = rowsIn(worker.drawn.at(-1))[0]!;
    await expect(worker.invoke('jump', first.action!.value)).resolves.toBeUndefined();
    expect(rowsIn(worker.drawn.at(-1)).length).toBeGreaterThan(4);
  });
});

describe('References worker across documents', () => {
  let worker: ReturnType<typeof startWorker>;

  beforeEach(() => {
    worker = startWorker(DOCS);
  });

  it('lists the open document’s references', async () => {
    worker.openDocument('a');
    await worker.invoke('refs');
    expect(JSON.stringify(worker.drawn.at(-1))).toContain('2202.00002');
  });

  it('lists the new document’s references after a switch', async () => {
    worker.openDocument('a');
    await worker.invoke('refs');
    worker.openDocument('b');
    await worker.invoke('refs');

    const last = JSON.stringify(worker.drawn.at(-1));
    expect(last).toContain('2303.00003');
    expect(last).not.toContain('2202.00002');
  });

  it('says so when no document is open', async () => {
    worker.openDocument(null);
    await worker.invoke('refs');
    expect(JSON.stringify(worker.drawn.at(-1))).toContain('Open a paper');
  });

  /** The panel refreshes twice in quick succession when a tab is opened: once
   * while the tab is empty, once when its document arrives. The second must
   * not be dropped. */
  it('does not drop a second render issued while the first is still running', async () => {
    worker.openDocument(null);
    const first = worker.invoke('refs');
    worker.openDocument('b');
    const second = worker.invoke('refs');
    await Promise.all([first, second]);

    expect(JSON.stringify(worker.drawn.at(-1))).toContain('2303.00003');
  });
});

// @vitest-environment happy-dom
/** Review mode and each PDF's review: loaded once, saved a moment after a
 * change, never written over when it could not be read; comments written
 * from the selection, kept or let go; a delete that can be undone. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SelectionGeometry } from '../../stores/selectionStore';
import type { QuotePiece } from '../../lib/text/copySelection';

let usable = true;
/** What the math converter answers: for an equation, by mode; for a
 * formula in a quote, by its text; for the whole passage from the source,
 * `exact`. Nothing is no converter. */
let converted: Record<'local' | 'exact', string | null> = { local: null, exact: null };
let formulas: Record<string, string> = {};
const resolveExact: (() => void)[] = [];
const snapshots = new Map<string, string>();
vi.mock('../extensions/extensionStore', () => ({
  extensionUsable: () => usable,
  snapshotSelection: () => 'whole',
  registerSnapshot: (snap: { text: string }) => {
    const token = `part-${snapshots.size}`;
    snapshots.set(token, snap.text);
    return token;
  },
  convertSelection: async (token: string, mode: 'local' | 'exact') => {
    let markdown: string | null = null;
    if (token === 'whole' && mode === 'exact') {
      await new Promise<void>((resolve) => resolveExact.push(resolve));
      markdown = converted.exact;
    } else if (mode === 'local') {
      markdown = formulas[snapshots.get(token) ?? ''] ?? null;
    }
    return markdown ? { markdown, source: 'fonts', confidence: 'high', converter: 'X' } : null;
  },
  convertEquation: async (_token: string, _ref: unknown, mode: 'local' | 'exact') => {
    const latex = converted[mode];
    return latex ? { latex, display: true, source: 'fonts', confidence: 'high' } : null;
  },
}));
vi.mock('../extensions/surfaces', () => ({ closeReaderPane: vi.fn(), shownPane: () => null }));
vi.mock('../extensions/paperDocs', () => ({ docStatements: async () => [] }));
vi.mock('../extensions/regionCapture', () => ({
  pickRegion: async () => ({ page: 0, box: [10, 20, 100, 50] }),
}));
vi.mock('../../lib/transport/engine', () => ({
  engine: {
    getTextLayout: async () => ({ src: 0, runs: [], charCount: 0 }),
    libraryPapers: async () => ({ rows: [], total: 0 }),
    onLibraryChanged: async () => () => {},
  },
}));
let geometry: SelectionGeometry | null = null;
let text = '';
let pieces: QuotePiece[] = [];
vi.mock('../../stores/selectionStore', () => ({
  selectedGeometry: () => geometry,
  selectedText: () => text,
  selectedPieces: () => pieces,
  settleSelectionNow: () => {},
}));

/** A selected line, its `math` characters set in a math font. */
function line(words: string, math: [number, number][]): QuotePiece[] {
  return [
    {
      run: { text: words, start: 0, x: 72, y: 682, w: 120, h: 10, page: 1, math },
      from: 0,
      to: words.length,
      display: null,
    },
  ];
}
const flashStatus = vi.fn();
vi.mock('../../stores/statusMessage', () => ({
  flashStatus: (...args: unknown[]) => flashStatus(...args),
}));

const store = await import('./markupStore');
const { addTab, createTab, removeTab, setActiveId, tabsStore } =
  await import('../../stores/tabsStore');

const PATH = '/papers/draft.pdf';
const saved: string[] = [];
let stored: string | null = null;
let failLoad = false;

function openDoc() {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 7,
    path: PATH,
    name: 'draft.pdf',
    pageCount: 2,
    sizes: [
      [612, 792, 0, 0, 0],
      [612, 792, 0, 0, 0],
    ],
    estimatedSize: [612, 792],
  });
  addTab(tab);
  setActiveId(tab.id);
  return tab;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  usable = true;
  saved.length = 0;
  stored = null;
  failLoad = false;
  store.setMarkupTransport({
    load: async () => {
      if (failLoad) throw new Error('unreadable');
      return stored;
    },
    save: async (_path, json) => {
      saved.push(json);
    },
    exportPdf: async () => ({ path: '', pages: 0, links: 0 }),
  });
  openDoc();
});

afterEach(() => {
  store.resetMarkupForTest();
  for (const tab of [...tabsStore.state.tabs]) removeTab(tab.id);
  setActiveId(null);
  vi.useRealTimers();
  flashStatus.mockClear();
});

async function loaded() {
  store.startReviewing();
  store.ensureLoaded(PATH);
  await vi.advanceTimersByTimeAsync(0);
}

describe('review mode', () => {
  it('needs the extension switched on', () => {
    usable = false;
    store.startReviewing();
    expect(store.reviewing()).toBe(false);
    usable = true;
    store.toggleReviewing();
    expect(store.reviewing()).toBe(true);
    store.toggleReviewing();
    expect(store.reviewing()).toBe(false);
  });
});

describe('a review', () => {
  it('is loaded from the backend and saved shortly after a change', async () => {
    stored = JSON.stringify({ version: 1, summary: 'Old.', comments: [] });
    await loaded();
    expect(store.reviewFor(PATH)?.doc.summary).toBe('Old.');
    store.setSummary(PATH, 'New');
    store.setSummary(PATH, 'Newer');
    expect(saved).toEqual([]);
    await vi.advanceTimersByTimeAsync(600);
    expect(saved).toHaveLength(1);
    expect(JSON.parse(saved[0]!).summary).toBe('Newer');
  });

  it('is never written over when it could not be read', async () => {
    failLoad = true;
    await loaded();
    expect(store.reviewFor(PATH)?.status).toBe('failed');
    store.setSummary(PATH, 'Anything');
    await vi.advanceTimersByTimeAsync(600);
    expect(saved).toEqual([]);
  });
});

describe('writing comments', () => {
  beforeEach(() => {
    geometry = { docId: 7, page: 1, rects: [[72, 100, 120, 10]] };
    text = 'by  compactness';
    pieces = [];
    formulas = {};
    converted = { local: null, exact: null };
  });

  it('starts from the selection and keeps what was written', async () => {
    await loaded();
    expect(store.beginComment('comment')).toBe(true);
    const d = store.draft()!;
    expect(d.anchor.page).toBe(1);
    expect(d.anchor.quote).toBe('by compactness');
    expect(d.anchor.quads![0]).toMatchObject({ x1: 72, y1: 692, x2: 192, y3: 682 });
    const id = store.commitDraft({
      body: ' Justify this step. ',
      kind: 'comment',
      quickmark: 'justify',
      replacement: null,
    });
    expect(id).toMatch(/^c_/);
    expect(store.draft()).toBeNull();
    const c = store.reviewFor(PATH)!.doc.comments[0]!;
    expect(c).toMatchObject({ kind: 'comment', body: 'Justify this step.', quickmark: 'justify' });
    await vi.advanceTimersByTimeAsync(600);
    expect(JSON.parse(saved.at(-1)!).comments).toHaveLength(1);
  });

  it('reads the quote math, then upgrades the kept comment quietly', async () => {
    pieces = line('by K holds', [[3, 4]]);
    formulas = { K: '$\\mathcal{K}$' };
    converted = { local: null, exact: 'by $\\mathcal{C}$ holds' };
    await loaded();
    store.beginComment('comment');
    expect(store.draft()!.anchor.quote).toBe('by K holds');
    expect(store.quoteReading(store.draft()!.anchor)).toBe(true);
    await vi.waitFor(() => expect(store.draft()!.anchor.quoteTex).toBe('by $\\mathcal{K}$ holds'));
    expect(store.quoteReading(store.draft()!.anchor)).toBe(false);
    store.commitDraft({ body: 'Why?', kind: 'comment', quickmark: null, replacement: null });
    await vi.waitFor(() => expect(resolveExact).toHaveLength(1));
    resolveExact.splice(0).forEach((resolve) => resolve());
    await vi.waitFor(() =>
      expect(store.reviewFor(PATH)!.doc.comments[0]!.anchor.quoteTex).toBe(
        'by $\\mathcal{C}$ holds'
      )
    );
    await vi.advanceTimersByTimeAsync(600);
    expect(JSON.parse(saved.at(-1)!).comments[0].anchor.quoteTex).toBe('by $\\mathcal{C}$ holds');
  });

  it('sets a formula that cannot be read as its text, and keeps the rest', async () => {
    pieces = line('as x and y hold', [
      [3, 4],
      [9, 10],
    ]);
    // y read as something KaTeX cannot typeset
    formulas = { x: '$x$', y: '$\\frac{1}{2$' };
    await loaded();
    store.beginComment('comment');
    await vi.waitFor(() => expect(store.draft()!.anchor.quoteTex).toBe('as $x$ and *y* hold'));
    resolveExact.splice(0).forEach((resolve) => resolve());
  });

  it('keeps no reading when nothing in the quote could be read', async () => {
    pieces = line('by K holds', [[3, 4]]);
    formulas = { K: 'bcy $\\frac{1}{2$ 746' };
    await loaded();
    store.beginComment('comment');
    await vi.waitFor(() => expect(store.quoteReading(store.draft()!.anchor)).toBe(false));
    expect(store.draft()!.anchor.quoteTex).toBeUndefined();
    expect(resolveExact).toHaveLength(0);
  });

  it('quotes the plain text without a converter', async () => {
    pieces = line('by K holds', [[3, 4]]);
    await loaded();
    store.beginComment('comment');
    await vi.waitFor(() => expect(store.quoteReading(store.draft()!.anchor)).toBe(false));
    expect(store.draft()!.anchor).toMatchObject({ quote: 'by K holds' });
    expect(store.draft()!.anchor.quoteTex).toBeUndefined();
  });

  it('comments on a display equation by its box', async () => {
    converted = { local: 'a^2', exact: null };
    await loaded();
    expect(store.beginEquationComment({ page: 1, bbox: [100, 200, 300, 40], number: '3' })).toBe(
      true
    );
    const d = store.draft()!;
    expect(d.anchor.quote).toBe('Equation (3)');
    expect(d.anchor.quads).toHaveLength(1);
    expect(d.anchor.quads![0]).toMatchObject({ x1: 100, y1: 592, x2: 400, y3: 552 });
    await vi.waitFor(() => expect(store.draft()!.anchor.quoteTex).toBe('$$ a^2 \\tag{3} $$'));
    store.cancelDraft();
    resolveExact.splice(0).forEach((resolve) => resolve());
    converted = { local: null, exact: null };
  });

  it('lets an empty comment go, but keeps a typo fix that deletes', async () => {
    await loaded();
    store.beginComment('comment');
    expect(
      store.commitDraft({ body: '  ', kind: 'comment', quickmark: null, replacement: null })
    ).toBeNull();
    store.beginComment('typo');
    expect(store.draft()?.kind).toBe('typo');
    store.commitDraft({ body: '', kind: 'typo', quickmark: 'typo', replacement: '' });
    expect(store.reviewFor(PATH)!.doc.comments[0]).toMatchObject({ kind: 'typo', replacement: '' });
  });

  it('does nothing outside review mode', () => {
    expect(store.beginComment('comment')).toBe(false);
  });

  it('marks a region', async () => {
    await loaded();
    expect(await store.beginRegion()).toBe(true);
    expect(store.draft()?.anchor.rect).toEqual({ x: 10, y: 722, w: 100, h: 50 });
  });

  it('deletes with an undo in the status bar', async () => {
    await loaded();
    store.beginComment('comment');
    const id = store.commitDraft({
      body: 'x',
      kind: 'comment',
      quickmark: null,
      replacement: null,
    })!;
    store.deleteComment(PATH, id);
    expect(store.reviewFor(PATH)!.doc.comments).toHaveLength(0);
    const [message, options] = flashStatus.mock.calls.at(-1)! as [
      string,
      { action: { run(): void } },
    ];
    expect(message).toBe('Comment deleted');
    options.action.run();
    expect(store.reviewFor(PATH)!.doc.comments.map((c) => c.id)).toEqual([id]);
  });

  it('drops a comment being written when its document leaves the screen', async () => {
    await loaded();
    store.beginComment('comment');
    setActiveId(null);
    await vi.advanceTimersByTimeAsync(0);
    expect(store.draft()).toBeNull();
  });
});

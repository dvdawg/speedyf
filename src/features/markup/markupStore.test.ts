// @vitest-environment happy-dom
/** Review mode and each PDF's review: loaded once, saved a moment after a
 * change, never written over when it could not be read; comments written
 * from the selection, kept or let go; a delete that can be undone. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SelectionGeometry } from '../../stores/selectionStore';

let usable = true;
vi.mock('../extensions/extensionStore', () => ({ extensionUsable: () => usable }));
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
vi.mock('../../stores/selectionStore', () => ({
  selectedGeometry: () => geometry,
  selectedText: () => text,
  settleSelectionNow: () => {},
}));
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
      kind: 'proof',
      quickmark: 'justify',
      replacement: null,
    });
    expect(id).toMatch(/^c_/);
    expect(store.draft()).toBeNull();
    const c = store.reviewFor(PATH)!.doc.comments[0]!;
    expect(c).toMatchObject({ kind: 'proof', body: 'Justify this step.', quickmark: 'justify' });
    await vi.advanceTimersByTimeAsync(600);
    expect(JSON.parse(saved.at(-1)!).comments).toHaveLength(1);
  });

  it('lets an empty comment go, but keeps a typo fix that deletes', async () => {
    await loaded();
    store.beginComment('comment');
    expect(
      store.commitDraft({ body: '  ', kind: 'exposition', quickmark: null, replacement: null })
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
      kind: 'exposition',
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

// @vitest-environment happy-dom
/** The review drawer: there only while reviewing; comments grouped by
 * section and numbered in reading order; a filter by kind; a click going
 * to the comment on the page; Overall started from the counts; the two
 * ways out. */
import { cleanup, fireEvent, render, screen, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { quadsFromRects } from './geometry';
import type { MarkupComment } from './types';

vi.mock('../extensions/extensionStore', () => ({ extensionUsable: () => true }));
vi.mock('../extensions/surfaces', () => ({
  closeReaderPane: vi.fn(),
  shownPane: () => null,
  paneWidth: () => 380,
  setPaneWidth: vi.fn(),
}));
vi.mock('../extensions/readerContext', () => ({ readerItem: () => null }));
vi.mock('../extensions/paperDocs', () => ({ docStatements: async () => [] }));
vi.mock('../extensions/regionCapture', () => ({ pickRegion: async () => null }));
vi.mock('../../lib/transport/engine', () => ({
  engine: {
    getTextLayout: async () => ({ src: 0, runs: [], charCount: 0 }),
    libraryPapers: async () => ({ rows: [], total: 0 }),
    onLibraryChanged: async () => () => {},
  },
}));
vi.mock('./mathHtml', () => ({ loadMath: async () => {}, mathRenderer: () => null }));
const exportMarkupPdf = vi.fn();
const copyMarkupText = vi.fn();
vi.mock('./exportActions', () => ({
  exportMarkupPdf: () => exportMarkupPdf(),
  copyMarkupText: () => copyMarkupText(),
}));
const jumpToAnchor = vi.fn();
vi.mock('../outline/jumpToAnchor', () => ({
  jumpToAnchor: (...args: unknown[]) => jumpToAnchor(...args),
  layoutIndexOf: (_doc: unknown, page: number) => page,
}));

const store = await import('./markupStore');
const { default: MarkupDrawer } = await import('./MarkupDrawer');
const { addTab, createTab, removeTab, setActiveId, tabsStore } =
  await import('../../stores/tabsStore');

const PATH = '/papers/draft.pdf';

function comment(id: string, page: number, top: number, over: Partial<MarkupComment> = {}) {
  return {
    id,
    kind: 'comment',
    anchor: {
      page,
      quads: quadsFromRects([[72, top, 100, 10]], 792),
      rect: null,
      quote: `q ${id}`,
    },
    body: `Body ${id}`,
    replacement: null,
    quickmark: null,
    created: 1,
    ...over,
  } as MarkupComment;
}

let stored: string | null = null;

beforeEach(async () => {
  store.setMarkupTransport({
    load: async () => stored,
    save: async () => {},
    exportPdf: async () => ({ path: '', pages: 0, links: 0 }),
  });
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 3,
    path: PATH,
    name: 'draft.pdf',
    pageCount: 3,
    sizes: [
      [612, 792, 0, 0, 0],
      [612, 792, 0, 0, 0],
      [612, 792, 0, 0, 0],
    ],
    estimatedSize: [612, 792],
  });
  tab.structureStore.reset();
  addTab(tab);
  setActiveId(tab.id);
});

afterEach(() => {
  cleanup();
  store.resetMarkupForTest();
  for (const tab of [...tabsStore.state.tabs]) removeTab(tab.id);
  setActiveId(null);
  jumpToAnchor.mockClear();
  stored = null;
});

async function open() {
  render(() => <MarkupDrawer />);
  store.startReviewing();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe('MarkupDrawer', () => {
  it('is not there until review mode is on', () => {
    render(() => <MarkupDrawer />);
    expect(screen.queryByRole('complementary', { name: 'Review' })).toBeNull();
  });

  it('says how to start when there is nothing yet', async () => {
    await open();
    const drawer = screen.getByRole('complementary', { name: 'Review' });
    expect(drawer.textContent).toContain('to comment on it');
    expect(drawer.textContent).toContain('0 comments');
  });

  it('numbers comments in reading order and filters them by kind', async () => {
    stored = JSON.stringify({
      version: 1,
      summary: '',
      comments: [
        comment('b', 1, 300, { kind: 'question' }),
        comment('a', 0, 100),
        comment('c', 2, 50, { kind: 'typo', replacement: 'the' }),
      ],
    });
    await open();
    const cards = () => screen.getAllByRole('article').map((a) => a.getAttribute('aria-label'));
    expect(cards()).toEqual(['Comment 1', 'Comment 2', 'Comment 3']);
    const first = screen.getByRole('article', { name: 'Comment 1' });
    expect(first.textContent).toContain('Body a');
    expect(first.textContent).toContain('p. 1');
    expect(
      screen.getByRole('article', { name: 'Comment 3' }).querySelector('.mk-ins')?.textContent
    ).toBe('the');

    const filter = screen.getByRole('group', { name: 'Show comments of kind' });
    fireEvent.click(within(filter).getByRole('button', { name: /Question/ }));
    expect(cards()).toEqual(['Comment 2']);
    fireEvent.click(within(filter).getByRole('button', { name: /All/ }));
    expect(cards()).toHaveLength(3);
  });

  it('quotes words as text with their math, and shows a region as the page draws it', async () => {
    const region = {
      ...comment('r', 2, 100),
      anchor: { page: 2, quads: null, rect: { x: 72, y: 600, w: 100, h: 50 }, quote: '' },
    };
    stored = JSON.stringify({
      version: 1,
      summary: '',
      comments: [
        comment('a', 0, 100, {
          anchor: { ...comment('a', 0, 100).anchor, quoteTex: 'q $x$ *y*' },
        }),
        // a reading saved before readings were checked, and not to be trusted
        comment('b', 1, 100, { anchor: { ...comment('b', 1, 100).anchor, quoteTex: '$q$ 746' } }),
        region,
      ],
    });
    await open();
    const first = screen.getByRole('article', { name: 'Comment 1' });
    expect(first.querySelector('img')).toBeNull();
    const quote = first.querySelector('.mk-card-quote.is-tex')!;
    expect(quote.querySelector('.mk-math-raw')?.textContent).toBe('$x$');
    expect(quote.querySelector('.mk-math-plain')?.textContent).toBe('y');
    const second = screen.getByRole('article', { name: 'Comment 2' });
    expect(second.querySelector('.mk-card-quote')?.textContent).toBe('q b');
    const third = screen.getByRole('article', { name: 'Comment 3' });
    const img = third.querySelector('.mk-crop img') as HTMLImageElement;
    expect(img.alt).toBe('Region on the page');
    expect(img.src).toContain('kind=preview');
    // the render fails, again and again: words instead
    for (let i = 0; i < 3; i += 1) fireEvent.error(third.querySelector('.mk-crop img')!);
    expect(third.querySelector('.mk-card-quote')?.textContent).toBe('Region on the page');
  });

  it('goes to a comment on the page from its label line', async () => {
    stored = JSON.stringify({ version: 1, summary: '', comments: [comment('a', 1, 100)] });
    await open();
    fireEvent.click(screen.getByTitle('Show on the page'));
    expect(jumpToAnchor).toHaveBeenCalled();
    expect(jumpToAnchor.mock.calls[0]![2]).toBe(1);
  });

  it('starts Overall from the counts', async () => {
    stored = JSON.stringify({
      version: 1,
      summary: '',
      comments: [comment('a', 0, 100, { kind: 'comment' })],
    });
    await open();
    fireEvent.click(screen.getByRole('button', { name: 'Start from an outline' }));
    expect(store.reviewFor(PATH)?.doc.summary).toContain('1 note: 1 comment.');
  });

  it('exports and copies from its foot', async () => {
    await open();
    fireEvent.click(screen.getByRole('button', { name: 'Export PDF...' }));
    fireEvent.click(screen.getByRole('button', { name: 'Copy as text' }));
    expect(exportMarkupPdf).toHaveBeenCalled();
    expect(copyMarkupText).toHaveBeenCalled();
  });
});

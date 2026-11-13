// @vitest-environment happy-dom
/**
 * The quick-open switcher: reached by its chord and by the palette command,
 * filters the library by title, author, year and cite key, puts an exact cite
 * key first, opens the first file still on disk, closes on Escape, and hands
 * a query it cannot answer to the full-text search.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Paper, PaperFile, PaperRow } from '../../types/engine';

function row(over: Partial<PaperRow> & { id: string; title: string }): PaperRow {
  return {
    authors: '',
    year: null,
    venue: null,
    citeKey: over.id,
    alphaLabel: '',
    projectIds: [],
    missing: false,
    hasIdentifier: false,
    incompleteCount: 0,
    isPaper: true,
    candidateCount: 0,
    fileCount: 1,
    ...over,
  } as PaperRow;
}

const ROWS: PaperRow[] = [
  row({
    id: 'jarzynski1997',
    title: 'Nonequilibrium equality for free energy differences',
    authors: 'C. Jarzynski',
    year: 1997,
    venue: 'Physical Review Letters',
    alphaLabel: 'Jar97',
  }),
  row({
    id: 'crooks1999',
    title: 'Entropy production fluctuation theorem',
    authors: 'G. E. Crooks',
    year: 1999,
    venue: 'Physical Review E',
    alphaLabel: 'Cro99',
  }),
  row({
    id: 'erdos1959',
    title: 'On random graphs',
    authors: 'P. Erdős, A. Rényi',
    year: 1959,
    alphaLabel: 'ER59',
  }),
  // A title that mentions another item's cite key, to check that the exact
  // key wins over a title match.
  row({
    id: 'seifert2012',
    title: 'Crooks1999 revisited: stochastic thermodynamics',
    authors: 'U. Seifert',
    year: 2012,
    alphaLabel: 'Sei12',
  }),
  row({
    id: 'gone2001',
    title: 'A paper whose file was moved',
    authors: 'N. Body',
    year: 2001,
    missing: true,
  }),
  row({ id: 'manual', title: 'Printer manual', isPaper: false }),
];

function file(path: string, missing = false): PaperFile {
  return { path, name: path, size: 1, mtimeMs: 0, fingerprint: null, missing };
}

const FILES: Record<string, PaperFile[]> = {
  jarzynski1997: [file('/lib/jarzynski.pdf')],
  crooks1999: [file('/old/crooks.pdf', true), file('/lib/crooks.pdf')],
  erdos1959: [file('/lib/erdos.pdf')],
  seifert2012: [file('/lib/seifert.pdf')],
  gone2001: [file('/gone.pdf', true)],
};

const engineMock = {
  fileMetadata: vi.fn(async () => ({ sizeBytes: 1 })),
  libraryPapers: vi.fn(async () => ({ rows: ROWS, total: ROWS.length })),
  libraryPaper: vi.fn(async (id: string) => ({ id, files: FILES[id] ?? [] }) as unknown as Paper),
  libraryPaperForPath: vi.fn(async (path: string) => {
    const id = Object.keys(FILES).find((key) => FILES[key]!.some((f) => f.path === path));
    return id ? ({ id, files: FILES[id] } as unknown as Paper) : null;
  }),
};

vi.mock('../../lib/transport/engine', () => ({
  engine: engineMock,
  isEngineError: () => false,
}));
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
  goHome: vi.fn(),
}));
vi.mock('../projects/fullTextHandoff', () => ({ searchFullText: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => true) }));
vi.mock('../../stores/modalStore', () => ({ showError: vi.fn(), modal: { kind: null } }));

const { default: QuickOpen } = await import('./QuickOpen');
const { closeQuickOpen, openQuickOpen, quickOpenOpen } = await import('./quickOpenStore');
const { openInNewTabOrFocus, goHome } = await import('../document/tabsController');
const { searchFullText } = await import('../projects/fullTextHandoff');
const { invoke } = await import('@tauri-apps/api/core');
const { recentStore } = await import('../../stores/recentStore');
const { runCommand, commandById } = await import('../../app/commands');
const { rankRows, prepare, fold } = await import('./quickOpenItems');

const input = () => screen.getByRole('combobox') as HTMLInputElement;
const titles = () =>
  Array.from(document.querySelectorAll('.quick-open-row .palette-title')).map(
    (el) => el.textContent
  );
const keys = () =>
  Array.from(document.querySelectorAll('.quick-open-key')).map((el) => el.textContent);

async function openWith(query = '') {
  render(() => <QuickOpen />);
  openQuickOpen();
  await waitFor(() => expect(keys().length).toBeGreaterThan(0));
  if (query) fireEvent.input(input(), { target: { value: query } });
}

beforeEach(() => {
  recentStore.clear();
});

afterEach(() => {
  closeQuickOpen();
  cleanup();
  vi.clearAllMocks();
});

describe('reaching it', () => {
  it('opens with Cmd/Ctrl+Shift+O', async () => {
    const { installShortcuts } = await import('../../app/shortcuts');
    const uninstall = installShortcuts();
    try {
      const isMac = /mac/i.test(navigator.userAgent);
      const event = new KeyboardEvent('keydown', {
        key: 'O',
        shiftKey: true,
        metaKey: isMac,
        ctrlKey: !isMac,
        cancelable: true,
      });
      window.dispatchEvent(event);
      expect(quickOpenOpen()).toBe(true);
      expect(event.defaultPrevented).toBe(true);
    } finally {
      uninstall();
    }
  });

  it('is a Library command the palette can run', () => {
    const command = commandById('library.quickOpen');
    expect(command?.title).toBe('Open Item...');
    expect(command?.group).toBe('Library');
    expect(command?.shortcut).toBe('mod+shift+o');
    expect(runCommand('library.quickOpen', { tab: null })).toBe(true);
    expect(quickOpenOpen()).toBe(true);
  });

  it('reads the library once per opening and lists items only', async () => {
    await openWith();
    expect(engineMock.libraryPapers).toHaveBeenCalledTimes(1);
    fireEvent.input(input(), { target: { value: 'p' } });
    fireEvent.input(input(), { target: { value: 'pr' } });
    expect(engineMock.libraryPapers).toHaveBeenCalledTimes(1);
    expect(keys()).not.toContain('manual');
  });
});

describe('filtering', () => {
  it('finds by title words', async () => {
    await openWith('free energy');
    expect(keys()).toEqual(['jarzynski1997']);
  });

  it('finds by author, ignoring case and diacritics', async () => {
    await openWith('erdos renyi');
    expect(keys()).toEqual(['erdos1959']);
  });

  it('finds by year', async () => {
    await openWith('1997');
    expect(keys()).toEqual(['jarzynski1997']);
  });

  it('finds by cite key and alpha label', async () => {
    await openWith('jar97');
    expect(keys()[0]).toBe('jarzynski1997');
  });

  it('needs every term to match', async () => {
    await openWith('crooks 1997');
    expect(keys()).toEqual([]);
  });

  it('puts an exact cite key above a title that contains it', async () => {
    await openWith('crooks1999');
    expect(keys().slice(0, 2)).toEqual(['crooks1999', 'seifert2012']);
  });

  it('ends the list with the full-text hand-off', async () => {
    await openWith('fluctuation');
    expect(titles().at(-1)).toBe('Search full text for "fluctuation"');
  });

  it('dims an item whose files are all missing', async () => {
    await openWith('moved');
    const missing = document.querySelector('.quick-open-row.is-missing');
    expect(missing?.textContent).toContain('file missing');
  });
});

describe('the untyped list', () => {
  it('shows the library order with no recents', async () => {
    await openWith();
    expect(keys()).toEqual(['jarzynski1997', 'crooks1999', 'erdos1959', 'seifert2012', 'gone2001']);
  });

  it('shows recently opened items first when there are any', async () => {
    recentStore.recordOpen({ path: '/lib/jarzynski.pdf', name: 'jarzynski.pdf' });
    recentStore.recordOpen({ path: '/lib/erdos.pdf', name: 'erdos.pdf' });
    await openWith();
    await waitFor(() => expect(keys()).toEqual(['erdos1959', 'jarzynski1997']));
  });
});

describe('keys', () => {
  it('opens the highlighted item on Enter', async () => {
    await openWith('random graphs');
    fireEvent.keyDown(input(), { key: 'Enter' });
    await waitFor(() => expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/erdos.pdf'));
    expect(quickOpenOpen()).toBe(false);
  });

  it('opens the item in a new window on Cmd+Enter', async () => {
    await openWith('random graphs');
    fireEvent.keyDown(input(), { key: 'Enter', metaKey: true });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('open_in_new_window', { path: '/lib/erdos.pdf' })
    );
    expect(openInNewTabOrFocus).not.toHaveBeenCalled();
    expect(quickOpenOpen()).toBe(false);
  });

  it('opens in a new window on Ctrl+Enter too', async () => {
    await openWith('entropy');
    fireEvent.keyDown(input(), { key: 'Enter', ctrlKey: true });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('open_in_new_window', { path: '/lib/crooks.pdf' })
    );
  });

  it('opens in a new window on Cmd-click', async () => {
    await openWith('random graphs');
    fireEvent.click(document.querySelector('.quick-open-row')!, { metaKey: true });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('open_in_new_window', { path: '/lib/erdos.pdf' })
    );
    expect(openInNewTabOrFocus).not.toHaveBeenCalled();
  });

  it('skips a missing file for the next one on disk', async () => {
    await openWith('entropy');
    fireEvent.keyDown(input(), { key: 'Enter' });
    await waitFor(() => expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/crooks.pdf'));
  });

  it('moves with the arrows and with Ctrl+N / Ctrl+P', async () => {
    await openWith('physical review');
    expect(keys()).toEqual(['crooks1999', 'jarzynski1997']);
    fireEvent.keyDown(input(), { key: 'ArrowDown' });
    fireEvent.keyDown(input(), { key: 'n', ctrlKey: true });
    fireEvent.keyDown(input(), { key: 'p', ctrlKey: true });
    fireEvent.keyDown(input(), { key: 'Enter' });
    await waitFor(() => expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/jarzynski.pdf'));
  });

  it('closes on Escape and gives focus back', async () => {
    const before = document.createElement('button');
    document.body.appendChild(before);
    before.focus();
    await openWith();
    await waitFor(() => expect(document.activeElement).toBe(input()));
    fireEvent.keyDown(input(), { key: 'Escape' });
    expect(quickOpenOpen()).toBe(false);
    expect(screen.queryByRole('combobox')).toBeNull();
    await waitFor(() => expect(document.activeElement).toBe(before));
    expect(openInNewTabOrFocus).not.toHaveBeenCalled();
    before.remove();
  });

  it('hands the query to the full-text search', async () => {
    await openWith('jarzynski equality');
    const total = document.querySelectorAll('.quick-open-row').length;
    for (let i = 1; i < total; i += 1) fireEvent.keyDown(input(), { key: 'ArrowDown' });
    fireEvent.keyDown(input(), { key: 'Enter' });
    expect(searchFullText).toHaveBeenCalledWith('jarzynski equality');
    expect(goHome).toHaveBeenCalled();
    expect(quickOpenOpen()).toBe(false);
  });
});

describe('ranking', () => {
  it('prefers a title prefix, then word starts, then newer', () => {
    const items = prepare([
      row({ id: 'a', title: 'Graphs of random things', year: 2000 }),
      row({ id: 'b', title: 'Random graphs', year: 1990 }),
      row({ id: 'c', title: 'Theory of random graphs', year: 2010 }),
      row({ id: 'd', title: 'Theory of random graphs', year: 2020 }),
    ]);
    expect(rankRows(items, 'random').map((r) => r.id)).toEqual(['b', 'd', 'c', 'a']);
  });

  it('folds case and diacritics', () => {
    expect(fold('Erdős RÉNYI')).toBe('erdos renyi');
  });
});

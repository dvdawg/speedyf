// @vitest-environment happy-dom
/**
 * Library undo as the window sees it: the status bar's quiet line (one at
 * a time, newest wins, gone by itself, held while hovered), the Undo it
 * offers after each action, the palette's two commands, and Cmd+Z, which
 * is the library's on the Home and Projects screens and the document's in
 * the reader, and nobody's while typing.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LibraryUndoState } from '../../types/engine';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
  openFromDialog: vi.fn(async () => {}),
  goHome: vi.fn(),
  closeOtherTabs: vi.fn(),
  closeTab: vi.fn(),
  closeTabsToTheRight: vi.fn(),
  cycleTab: vi.fn(),
  detachTab: vi.fn(),
  jumpToTab: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));
vi.mock('../home/HomeScreen', () => ({ showHomePanel: vi.fn(), default: () => null }));

const { engine } = await import('../../lib/transport/engine');
const { default: StatusMessage } = await import('../../components/StatusMessage');
const { ACTION_MS, PLAIN_MS, dismissStatus, flashStatus, statusMessage } =
  await import('../../stores/statusMessage');
const { refreshLibraryUndo, resetLibraryUndoForTests, libraryUndoState } =
  await import('./libraryUndo');
const { commandById, runCommand } = await import('../../app/commands');
const { installShortcuts } = await import('../../app/shortcuts');
const { addTab, createTab, removeTab, setActiveId, tabsStore } =
  await import('../../stores/tabsStore');
const { lookedUp } = await import('./projectCommands');

const isMac = /mac/i.test(navigator.userAgent);

function press(key: string, opts: { shift?: boolean; target?: EventTarget } = {}) {
  const event = new KeyboardEvent('keydown', {
    key,
    metaKey: isMac,
    ctrlKey: !isMac,
    shiftKey: !!opts.shift,
    bubbles: true,
    cancelable: true,
  });
  (opts.target ?? window).dispatchEvent(event);
  return event;
}

function openTab() {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 1,
    path: '/a.pdf',
    name: 'a.pdf',
    pageCount: 2,
    sizes: Array.from({ length: 2 }, () => [612, 792, 0, 0, 0]),
    estimatedSize: [612, 792],
  });
  addTab(tab);
  setActiveId(tab.id);
  return tab;
}

let hasFocus: ReturnType<typeof vi.spyOn>;

const STEP: LibraryUndoState = {
  undo: { seq: 1, description: 'Removed from pac_learning' },
  redo: null,
};

beforeEach(() => {
  for (const t of [...tabsStore.state.tabs]) removeTab(t.id);
  setActiveId(null);
  resetLibraryUndoForTests();
  dismissStatus();
  vi.mocked(engine.libraryUndoState).mockResolvedValue({ undo: null, redo: null });
  hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  hasFocus.mockRestore();
  vi.clearAllMocks();
});

describe('the status line', () => {
  it('shows a message, hides it by itself, and lets the newest win', () => {
    vi.useFakeTimers();
    render(() => <StatusMessage />);
    flashStatus('Copied cite key');
    expect(screen.getByText('Copied cite key')).toBeTruthy();
    flashStatus('Synced pac_learning.bib');
    expect(screen.queryByText('Copied cite key')).toBeNull();
    expect(screen.getByText('Synced pac_learning.bib')).toBeTruthy();
    vi.advanceTimersByTime(PLAIN_MS + 10);
    expect(screen.queryByText('Synced pac_learning.bib')).toBeNull();
    expect(statusMessage()).toBeNull();
  });

  it('is announced politely and stays while hovered', () => {
    vi.useFakeTimers();
    const { container } = render(() => <StatusMessage />);
    const line = container.querySelector('.sb-flash')!;
    expect(line.getAttribute('aria-live')).toBe('polite');
    flashStatus('Removed from pac_learning', { action: { label: 'Undo', run: () => {} } });
    fireEvent.mouseEnter(line);
    vi.advanceTimersByTime(ACTION_MS * 3);
    expect(screen.getByText('Removed from pac_learning.')).toBeTruthy();
    fireEvent.mouseLeave(line);
    vi.advanceTimersByTime(ACTION_MS);
    expect(statusMessage()).toBeNull();
  });
});

describe('library undo', () => {
  it('says a new step with Undo, which undoes it and says so', async () => {
    render(() => <StatusMessage />);
    vi.mocked(engine.libraryUndoState).mockResolvedValue(STEP);
    await refreshLibraryUndo();
    expect(screen.getByText('Removed from pac_learning.')).toBeTruthy();
    vi.mocked(engine.libraryUndoState).mockResolvedValue({ undo: null, redo: STEP.undo });
    fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    expect(engine.libraryUndo).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(screen.getByText('Undid: Removed from Noneq')).toBeTruthy());
    expect(libraryUndoState().redo?.description).toBe('Removed from pac_learning');
  });

  it('says nothing for a step already seen, a redone one, or in a window without focus', async () => {
    vi.mocked(engine.libraryUndoState).mockResolvedValue(STEP);
    await refreshLibraryUndo(false);
    expect(statusMessage()).toBeNull();
    // A scan changed the library: same step on top.
    await refreshLibraryUndo();
    expect(statusMessage()).toBeNull();
    hasFocus.mockReturnValue(false);
    vi.mocked(engine.libraryUndoState).mockResolvedValue({
      undo: { seq: 2, description: 'Created drafts' },
      redo: null,
    });
    await refreshLibraryUndo();
    expect(statusMessage()).toBeNull();
    expect(libraryUndoState().undo?.description).toBe('Created drafts');
  });

  it('offers palette commands only when there is something to undo or redo', async () => {
    const ctx = { tab: null };
    const undo = commandById('library.undo')!;
    const redo = commandById('library.redo')!;
    expect(undo.enabled(ctx)).toBe(false);
    expect(redo.enabled(ctx)).toBe(false);
    vi.mocked(engine.libraryUndoState).mockResolvedValue(STEP);
    await refreshLibraryUndo(false);
    expect(undo.enabled(ctx)).toBe(true);
    expect(undo.title).toBe('Undo Library Change: Removed from pac_learning');
    expect(redo.enabled(ctx)).toBe(false);
    expect(runCommand('library.undo', ctx)).toBe(true);
    expect(engine.libraryUndo).toHaveBeenCalledTimes(1);
  });

  it('says how many items a lookup run from the palette looked up', () => {
    const batch = { attempted: 9, applied: 4, skipped: 0, failed: 0, parked: 0 };
    expect(lookedUp(batch)).toBe('Looked up 9 items');
    expect(lookedUp({ ...batch, attempted: 1 })).toBe('Looked up 1 item');
    expect(lookedUp({ ...batch, attempted: 0 })).toBe('Nothing to look up');
  });
});

describe('Cmd+Z', () => {
  it('runs library undo and redo on the Home and Projects screens', async () => {
    vi.mocked(engine.libraryUndoState).mockResolvedValue({
      undo: STEP.undo,
      redo: { seq: 1, description: 'Added to thesis' },
    });
    await refreshLibraryUndo(false);
    const uninstall = installShortcuts();
    try {
      expect(press('z').defaultPrevented).toBe(true);
      expect(engine.libraryUndo).toHaveBeenCalledTimes(1);
      press('z', { shift: true });
      expect(engine.libraryRedo).toHaveBeenCalledTimes(1);
    } finally {
      uninstall();
    }
  });

  it('stays the document undo in the reader', async () => {
    vi.mocked(engine.libraryUndoState).mockResolvedValue(STEP);
    await refreshLibraryUndo(false);
    const tab = openTab();
    const undo = vi.spyOn(tab.documentStore, 'undo').mockImplementation(() => true);
    const uninstall = installShortcuts();
    try {
      press('z');
      expect(undo).toHaveBeenCalledTimes(1);
      expect(engine.libraryUndo).not.toHaveBeenCalled();
    } finally {
      uninstall();
    }
  });

  it('is left to the field while typing', async () => {
    vi.mocked(engine.libraryUndoState).mockResolvedValue(STEP);
    await refreshLibraryUndo(false);
    const input = document.createElement('input');
    document.body.appendChild(input);
    const uninstall = installShortcuts();
    try {
      expect(press('z', { target: input }).defaultPrevented).toBe(false);
      expect(engine.libraryUndo).not.toHaveBeenCalled();
    } finally {
      uninstall();
      input.remove();
    }
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Paper } from '../types/engine';
import type { CommandCtx } from './commands';
import type * as engineModule from '../lib/transport/engine';

/** What the Paper panel would have cached for the active document. */
const cache: { paper: Paper | null; known: boolean } = { paper: null, known: false };
const refreshActivePaper = vi.fn(async () => cache.paper);
vi.mock('../features/projects/activePaper', () => ({
  activePaper: () => cache.paper,
  activePaperKnown: () => cache.known,
  refreshActivePaper: (...args: unknown[]) => refreshActivePaper(...(args as [])),
}));

const libraryBibtex = vi.fn(async (_ids: string[]) => '@article{jarzynski1997nonequilibrium}');
const libraryLookup = vi.fn(async () => ({ kind: 'applied' as const }));
vi.mock('../lib/transport/engine', async (importOriginal) => {
  const actual = await importOriginal<typeof engineModule>();
  return {
    ...actual,
    engine: {
      ...actual.engine,
      libraryBibtex: (...args: unknown[]) => libraryBibtex(...(args as [string[]])),
      libraryLookup: (...args: unknown[]) => libraryLookup(...(args as [])),
    },
  };
});

// Going home publishes the window's tabs over IPC, which a Node test has no
// bridge for; the commands only need to be seen asking.
const goHome = vi.fn();
const closeOtherTabs = vi.fn();
const closeTabsToTheRight = vi.fn();
vi.mock('../features/document/tabsController', () => ({
  closeOtherTabs: (id: string) => closeOtherTabs(id),
  closeTab: vi.fn(),
  closeTabsToTheRight: (id: string) => closeTabsToTheRight(id),
  cycleTab: vi.fn(),
  detachTab: vi.fn(),
  goHome: () => goHome(),
  openFromDialog: vi.fn(),
}));

const { COMMANDS, COMMAND_GROUPS, commandById, formatShortcut, runCommand } =
  await import('./commands');
const { settings, updateSettings } = await import('../stores/settings');
const { ui, setUi } = await import('../stores/uiStore');
const { addTab, createTab, removeTab, tabsStore } = await import('../stores/tabsStore');

/** A tab with just enough shape for `enabled` to interrogate. Anything a
 * command actually calls is a spy, so running one is observable without a
 * real document store. */
function fakeTab(over: {
  loaded?: boolean;
  pages?: number;
  selected?: { pageId: string; annotId: string } | null;
  currentPage?: number;
}) {
  const apply = vi.fn();
  const pages = Array.from({ length: over.pages ?? 3 }, (_, i) => ({
    id: `p${i}`,
    widthPt: 612,
    heightPt: 792,
  }));
  return {
    apply,
    tab: {
      id: 'tab-1',
      opening: false,
      documentStore: {
        state: {
          loaded: over.loaded ?? true,
          pages,
          selected: over.selected ?? null,
        },
        apply,
        undo: vi.fn(),
        redo: vi.fn(),
        setSelected: vi.fn(),
      },
      viewport: {
        state: { currentPage: over.currentPage ?? 0, searchOpen: false, formPanelOpen: false },
        setState: vi.fn(),
        requestScrollToPage: vi.fn(),
      },
      zoom: {
        zoomStep: vi.fn(),
        applyFit: vi.fn(),
        setZoomAnchored: vi.fn(),
      },
    } as unknown as NonNullable<CommandCtx['tab']>,
  };
}

const empty: CommandCtx = { tab: null };

describe('command registry', () => {
  it('has no duplicate ids', () => {
    const ids = COMMANDS.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('never shows a blank title', () => {
    for (const command of COMMANDS) expect(command.title.trim().length).toBeGreaterThan(0);
  });

  /** An unused group is a palette heading nobody can ever reach. */
  it('uses every group at least once', () => {
    const used = new Set(COMMANDS.map((c) => c.group));
    for (const group of COMMAND_GROUPS) expect(used.has(group), group).toBe(true);
  });

  /** Two commands on one chord would make the keymap's first match win
   * silently; the display spec is the only record a palette user sees. */
  it('gives no two commands the same shortcut', () => {
    const specs = COMMANDS.map((c) => c.shortcut).filter((s): s is string => s !== undefined);
    expect(new Set(specs).size).toBe(specs.length);
  });

  it('survives being asked about an empty app', () => {
    // Every enabled() runs on the home screen, where there is no tab at all.
    // A throw here would take the whole palette down with it.
    for (const command of COMMANDS) {
      expect(() => command.enabled(empty)).not.toThrow();
    }
  });

  it('disables document commands when nothing is open', () => {
    for (const id of [
      'file.save',
      'file.print',
      'file.detachTab',
      'edit.undo',
      'page.rotateRight',
      'view.fitPage',
    ]) {
      expect(commandById(id)!.enabled(empty), id).toBe(false);
    }
  });

  it('keeps the always-available commands reachable with no document', () => {
    for (const id of [
      'file.open',
      'file.newWindow',
      'file.goHome',
      'search.library',
      'library.goToProjects',
      'library.switchLaunchScreen',
    ]) {
      expect(commandById(id)!.enabled(empty), id).toBe(true);
    }
  });

  it('refuses to run a disabled command', () => {
    const ran = runCommand('page.rotateRight', empty);
    expect(ran).toBe(false);
  });

  it('reports an unknown id rather than throwing', () => {
    expect(runCommand('nope.not.a.command', empty)).toBe(false);
    expect(commandById('nope.not.a.command')).toBeUndefined();
  });
});

describe('tab commands', () => {
  const ids = ['t1', 't2', 't3'];
  beforeEach(() => {
    for (const id of ids) {
      const tab = createTab();
      tab.id = id;
      addTab(tab);
    }
  });
  afterEach(() => {
    for (const tab of [...tabsStore.state.tabs]) {
      tab.citationStore.dispose();
      removeTab(tab.id);
    }
  });
  const ctxFor = (id: string) =>
    ({ tab: tabsStore.state.tabs.find((t) => t.id === id)! }) as unknown as CommandCtx;

  it('offers closing the other tabs only when there are others', () => {
    expect(commandById('file.closeOtherTabs')!.enabled(ctxFor('t2'))).toBe(true);
    removeTab('t1');
    removeTab('t3');
    expect(commandById('file.closeOtherTabs')!.enabled(ctxFor('t2'))).toBe(false);
  });

  it('offers closing the tabs to the right except from the last tab', () => {
    expect(commandById('file.closeTabsToTheRight')!.enabled(ctxFor('t1'))).toBe(true);
    expect(commandById('file.closeTabsToTheRight')!.enabled(ctxFor('t3'))).toBe(false);
  });

  it('runs them against the tab they were asked about', () => {
    runCommand('file.closeOtherTabs', ctxFor('t2'));
    runCommand('file.closeTabsToTheRight', ctxFor('t1'));
    expect(closeOtherTabs).toHaveBeenCalledWith('t2');
    expect(closeTabsToTheRight).toHaveBeenCalledWith('t1');
  });
});

describe('page commands', () => {
  it('rotates the page the viewport is on, not the first one', () => {
    const { tab, apply } = fakeTab({ currentPage: 2 });
    runCommand('page.rotateRight', { tab });
    expect(apply).toHaveBeenCalledWith({ type: 'rotate', pageId: 'p2', delta: 90 });
  });

  it('rotates left as a 270 delta, so undo stays a plain inverse', () => {
    const { tab, apply } = fakeTab({});
    runCommand('page.rotateLeft', { tab });
    expect(apply).toHaveBeenCalledWith({ type: 'rotate', pageId: 'p0', delta: 270 });
  });

  it('clamps a viewport index past the end of a shortened document', () => {
    const { tab, apply } = fakeTab({ pages: 2, currentPage: 9 });
    runCommand('page.duplicate', { tab });
    expect(apply).toHaveBeenCalledWith({ type: 'duplicate', pageId: 'p1' });
  });

  it('will not delete the only page', () => {
    const { tab } = fakeTab({ pages: 1 });
    expect(commandById('page.delete')!.enabled({ tab })).toBe(false);
  });

  it('deletes a page when others remain', () => {
    const { tab, apply } = fakeTab({ pages: 2 });
    expect(runCommand('page.delete', { tab })).toBe(true);
    expect(apply).toHaveBeenCalledWith({ type: 'delete', pageId: 'p0' });
  });
});

describe('selection commands', () => {
  it('is disabled with nothing selected', () => {
    const { tab } = fakeTab({ selected: null });
    expect(commandById('edit.deleteSelection')!.enabled({ tab })).toBe(false);
  });

  it('deletes exactly the selected annotation', () => {
    const { tab, apply } = fakeTab({ selected: { pageId: 'p1', annotId: 'a7' } });
    expect(runCommand('edit.deleteSelection', { tab })).toBe(true);
    expect(apply).toHaveBeenCalledWith({ type: 'deleteAnnot', pageId: 'p1', id: 'a7' });
  });
});

describe('navigation commands', () => {
  it('stops at the last page', () => {
    const { tab } = fakeTab({ pages: 3, currentPage: 2 });
    runCommand('nav.nextPage', { tab });
    expect(tab.viewport.requestScrollToPage).toHaveBeenCalledWith(2);
  });

  it('stops at the first page', () => {
    const { tab } = fakeTab({ currentPage: 0 });
    runCommand('nav.previousPage', { tab });
    expect(tab.viewport.requestScrollToPage).toHaveBeenCalledWith(0);
  });
});

describe('library commands', () => {
  const paper: Paper = {
    id: 'paper-1',
    files: [],
    doi: '10.1103/physrevlett.78.2690',
    arxiv: null,
    isbn: null,
    meta: {
      title: null,
      authors: null,
      year: null,
      venue: null,
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: null,
    },
    citeKey: 'jarzynski1997nonequilibrium',
    citeKeyPinned: false,
    projectIds: [],
    addedAt: '',
    updatedAt: '',
    incomplete: [],
    isPaper: true,
    paperReason: 'doi on the first page',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  };
  const writeText = vi.fn(async (_text: string) => {});

  beforeEach(() => {
    cache.paper = null;
    cache.known = false;
    writeText.mockReset();
    writeText.mockImplementation(async () => {});
    refreshActivePaper.mockClear();
    libraryBibtex.mockClear();
    libraryLookup.mockClear();
    goHome.mockClear();
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    setUi({ sidebarOpen: false, sidebarMode: 'pages' });
    updateSettings({ onlineLookup: false, launchScreen: 'home' });
  });

  afterEach(() => vi.unstubAllGlobals());

  it('needs a loaded document to copy anything', () => {
    for (const id of ['library.copyCiteKey', 'library.copyBibtex']) {
      expect(commandById(id)!.enabled(empty), id).toBe(false);
      expect(commandById(id)!.enabled({ tab: fakeTab({ loaded: false }).tab }), id).toBe(false);
      expect(commandById(id)!.enabled({ tab: fakeTab({}).tab }), id).toBe(true);
    }
  });

  /** The write has to land inside the click that asked for it, so the cached
   * record is read and nothing is awaited on the way to the clipboard. */
  it('copies the cite key from the cached record without a round trip', async () => {
    cache.paper = paper;
    cache.known = true;
    expect(runCommand('library.copyCiteKey', { tab: fakeTab({}).tab })).toBe(true);
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith('jarzynski1997nonequilibrium'));
    expect(refreshActivePaper).not.toHaveBeenCalled();
    expect(ui.sidebarMode).toBe('pages');
  });

  it('fetches the record when the panel has never cached one', async () => {
    cache.paper = paper;
    cache.known = false;
    runCommand('library.copyCiteKey', { tab: fakeTab({}).tab });
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith('jarzynski1997nonequilibrium'));
    expect(refreshActivePaper).toHaveBeenCalledTimes(1);
  });

  /** Where the text can still be copied by hand. */
  it('opens the Paper panel when the clipboard refuses the write', async () => {
    cache.paper = paper;
    cache.known = true;
    writeText.mockRejectedValueOnce(new Error('not allowed by the user agent'));
    runCommand('library.copyCiteKey', { tab: fakeTab({}).tab });
    await vi.waitFor(() => expect(ui.sidebarMode).toBe('paper'));
    expect(ui.sidebarOpen).toBe(true);
  });

  it('opens the Paper panel for a file outside the library instead of copying', async () => {
    cache.paper = null;
    cache.known = true;
    runCommand('library.copyCiteKey', { tab: fakeTab({}).tab });
    await vi.waitFor(() => expect(ui.sidebarMode).toBe('paper'));
    expect(writeText).not.toHaveBeenCalled();
  });

  it('asks the engine for the BibTeX entry and copies it', async () => {
    cache.paper = paper;
    cache.known = true;
    runCommand('library.copyBibtex', { tab: fakeTab({}).tab });
    await vi.waitFor(() =>
      expect(writeText).toHaveBeenCalledWith('@article{jarzynski1997nonequilibrium}')
    );
    expect(libraryBibtex).toHaveBeenCalledWith(['paper-1']);
  });

  it('opens the Paper panel when the engine cannot build the entry', async () => {
    cache.paper = paper;
    cache.known = true;
    libraryBibtex.mockRejectedValueOnce({ code: 'unsupported', message: 'not implemented yet' });
    runCommand('library.copyBibtex', { tab: fakeTab({}).tab });
    await vi.waitFor(() => expect(ui.sidebarMode).toBe('paper'));
    expect(writeText).not.toHaveBeenCalled();
  });

  it('shows the Paper panel for any open tab', () => {
    // The panel is labelled Library in the sidebar; the palette says the same.
    expect(commandById('library.showPaperPanel')!.title).toBe('Show Library Panel');
    expect(commandById('library.showPaperPanel')!.enabled(empty)).toBe(false);
    expect(runCommand('library.showPaperPanel', { tab: fakeTab({ loaded: false }).tab })).toBe(
      true
    );
    expect(ui.sidebarOpen).toBe(true);
    expect(ui.sidebarMode).toBe('paper');
  });

  it('offers lookup only with the setting on and an identifier to send', () => {
    const { tab } = fakeTab({});
    const lookup = commandById('library.lookup')!;
    expect(lookup.enabled({ tab })).toBe(false);

    updateSettings({ onlineLookup: true });
    // Nothing cached yet: offered, and the run sorts out the rest.
    expect(lookup.enabled({ tab })).toBe(true);
    expect(lookup.enabled(empty)).toBe(false);

    cache.known = true;
    cache.paper = { ...paper, doi: null, arxiv: null };
    expect(lookup.enabled({ tab })).toBe(false);
    cache.paper = { ...paper, doi: null, arxiv: '1706.03762' };
    expect(lookup.enabled({ tab })).toBe(true);
    expect(lookup.disabledReason).toBeTruthy();
  });

  /** A manual in a library folder has a record, and may even carry a DOI
   * on some page; it is still nothing a lookup should be run for. */
  it('refuses lookup for a record the scan did not take for a paper', () => {
    const { tab } = fakeTab({});
    updateSettings({ onlineLookup: true });
    cache.known = true;
    cache.paper = { ...paper, isPaper: false, paperReason: 'no abstract or venue' };
    expect(commandById('library.lookup')!.enabled({ tab })).toBe(false);
  });

  it('looks the active paper up by identifier', async () => {
    updateSettings({ onlineLookup: true });
    cache.known = true;
    cache.paper = paper;
    expect(runCommand('library.lookup', { tab: fakeTab({}).tab })).toBe(true);
    await vi.waitFor(() => expect(libraryLookup).toHaveBeenCalledWith('paper-1', 'id'));
  });

  it('goes to the Projects screen and remembers it as the launch screen', () => {
    runCommand('library.goToProjects', empty);
    expect(settings.launchScreen).toBe('projects');
    expect(goHome).toHaveBeenCalledTimes(1);
  });

  it('switches the launch screen both ways', () => {
    runCommand('library.switchLaunchScreen', empty);
    expect(settings.launchScreen).toBe('projects');
    runCommand('library.switchLaunchScreen', empty);
    expect(settings.launchScreen).toBe('home');
    expect(goHome).toHaveBeenCalledTimes(2);
  });
});

describe('formatShortcut', () => {
  it('spells shortcuts as words joined by +, on every platform', () => {
    expect(formatShortcut('mod+shift+c', true)).toBe('Cmd+Shift+C');
    expect(formatShortcut('mod+shift+c', false)).toBe('Ctrl+Shift+C');
    expect(formatShortcut('alt+backspace', true)).toBe('Option+Backspace');
    expect(formatShortcut('alt+backspace', false)).toBe('Alt+Backspace');
    expect(formatShortcut('mod+plus', true)).toBe('Cmd+Plus');
    expect(formatShortcut('mod+minus', false)).toBe('Ctrl+Minus');
    expect(formatShortcut('ctrl+tab', true)).toBe('Ctrl+Tab');
  });

  it('renders every command shortcut in plain ASCII', () => {
    for (const command of COMMANDS) {
      if (!command.shortcut) continue;
      for (const mac of [true, false]) {
        expect(formatShortcut(command.shortcut, mac)).toMatch(/^[\x21-\x7e]+$/);
      }
    }
  });
});

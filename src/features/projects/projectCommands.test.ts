/**
 * The Projects screen's palette commands and per-thing rows, against a
 * stand-in store: what each needs to be offered, what it changes, and that
 * nothing destructive happens without the screen's own confirmation.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CommandCtx } from '../../app/commands';
import type { Paper, PaperFile, Project, ProjectSuggestion } from '../../types/engine';
import type * as GraphPositions from './graphPositions';
import type * as ProjectsStore from './projectsStore';

const h = vi.hoisted(() => {
  type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any
  const state: Any = {};
  const reset = () =>
    Object.assign(state, {
      initialized: true,
      projects: [],
      scope: { kind: 'all' },
      query: '',
      view: 'list',
      focus: null,
      selected: null,
      selectedId: null,
      ghostKey: null,
      missing: { on: false, min: 2 },
      missingList: false,
    });
  reset();
  const store = {
    state,
    initialize: vi.fn(async () => {}),
    setScope: vi.fn((scope: Any) => {
      state.scope = scope;
      state.focus = null;
    }),
    setView: vi.fn((view: Any) => {
      state.view = view;
      state.missingList = false;
    }),
    setMissing: vi.fn((on: boolean) => {
      state.missing.on = on;
    }),
    showInGraph: vi.fn((paperId: string, depth: number) => {
      state.focus = { paperId, depth };
      state.view = 'graph';
    }),
    leaveFocus: vi.fn(() => {
      state.focus = null;
    }),
    select: vi.fn((id: string | null) => {
      state.selectedId = id;
    }),
    lookupAll: vi.fn(async () => undefined),
    lookupProject: vi.fn(async (_id: string) => undefined),
    setMembership: vi.fn(async (_paper: string, _project: string, _member: boolean) => {}),
    updatePaper: vi.fn(async (id: string, patch: Any) => ({ ...state.selected, id, ...patch })),
    unmerge: vi.fn(async (_id: string) => []),
    mergePapers: vi.fn(async () => undefined),
    reportError: vi.fn(),
  };
  return {
    reset,
    store,
    goHome: vi.fn(),
    openInNewTabOrFocus: vi.fn(async (_path: string) => true),
    exportBib: vi.fn(async (_p: Any) => {}),
    syncBib: vi.fn(async (_p: Any) => {}),
    stopSyncingBib: vi.fn(async (_p: Any) => {}),
    searchFullText: vi.fn(),
    suggest: vi.fn(async (_id: string): Promise<Any[]> => []),
    active: { paper: null as Any, known: false },
    refreshActivePaper: vi.fn(async () => null as Any),
    rememberPaper: vi.fn(),
    cite: { rows: [] as Any[] },
    positions: {} as Record<string, unknown>,
    clearPositions: vi.fn(),
  };
});

vi.mock('./projectsStore', async (importOriginal) => ({
  ...(await importOriginal<typeof ProjectsStore>()),
  projectsStore: h.store,
}));
vi.mock('../../lib/transport/engine', () => ({
  engine: { librarySuggestProjects: (id: string) => h.suggest(id) },
  isEngineError: () => false,
}));
vi.mock('../document/tabsController', () => ({
  goHome: () => h.goHome(),
  openInNewTabOrFocus: (path: string) => h.openInNewTabOrFocus(path),
}));
vi.mock('./activePaper', () => ({
  activePaper: () => h.active.paper,
  activePaperKnown: () => h.active.known,
  refreshActivePaper: () => h.refreshActivePaper(),
  rememberPaper: (...args: unknown[]) => h.rememberPaper(...args),
  forgetPapers: vi.fn(),
}));
vi.mock('./bibActions', () => ({
  exportBib: (p: Project) => h.exportBib(p),
  syncBib: (p: Project) => h.syncBib(p),
  stopSyncingBib: (p: Project) => h.stopSyncingBib(p),
}));
vi.mock('./fullTextHandoff', () => ({
  searchFullText: (...args: unknown[]) => h.searchFullText(...args),
}));
vi.mock('./citeSelection', () => ({ citeSelection: () => h.cite.rows }));
vi.mock('./graphPositions', async (importOriginal) => ({
  ...(await importOriginal<typeof GraphPositions>()),
  loadPositions: (key: string) => (h.positions[key] as Record<string, unknown>) ?? {},
  clearPositions: (key: string) => h.clearPositions(key),
}));

const { PROJECT_COMMANDS, projectPaletteRows, primeProjectPalette, loadSuggestions } =
  await import('./projectCommands');
const intents = await import('./screenIntents');
const { dismissSuggestion, resetDismissedSuggestions } = await import('./suggestionDismissals');
const { settings, updateSettings } = await import('../../stores/settings');
const { PROJECTS } = await import('./testFixtures');

const command = (id: string) => {
  const found = PROJECT_COMMANDS.find((c) => c.id === id);
  if (!found) throw new Error(`no command ${id}`);
  return found;
};
const titles = (ctx: CommandCtx) => projectPaletteRows(ctx).map((row) => row.title);
const row = (ctx: CommandCtx, title: string) => {
  const found = projectPaletteRows(ctx).find((r) => r.title === title);
  if (!found) throw new Error(`no row ${title}`);
  return found;
};

function file(name: string, missing = false): PaperFile {
  return { path: `/lib/${name}`, name, size: 1, mtimeMs: 0, fingerprint: null, missing };
}

function paper(over: Partial<Paper> = {}): Paper {
  return {
    id: 'a',
    citeKey: 'jarzynski1997',
    files: [file('a.pdf')],
    isPaper: true,
    projectIds: [],
    mergedCount: 0,
    candidates: [],
    ...over,
  } as Paper;
}

/** No tab: the launch screen shows. */
const screenCtx: CommandCtx = { tab: null };
function readerCtx(path = '/lib/a.pdf'): CommandCtx {
  return {
    tab: { documentStore: { state: { loaded: true, path } } },
  } as unknown as CommandCtx;
}

beforeEach(() => {
  h.reset();
  h.store.state.projects = structuredClone(PROJECTS);
  h.active.paper = null;
  h.active.known = false;
  h.cite.rows = [];
  for (const key of Object.keys(h.positions)) delete h.positions[key];
  updateSettings({ launchScreen: 'projects', onlineLookup: false });
});

afterEach(() => {
  vi.clearAllMocks();
  resetDismissedSuggestions();
});

describe('the screen', () => {
  it('sets the scope, switching to the Projects screen first when it is not showing', () => {
    updateSettings({ launchScreen: 'home' });
    command('library.showNotFiled').run(screenCtx);
    expect(settings.launchScreen).toBe('projects');
    expect(h.goHome).toHaveBeenCalled();
    expect(h.store.setScope).toHaveBeenLastCalledWith({ kind: 'unfiled' });

    command('library.showOtherPdfs').run(readerCtx());
    expect(h.store.setScope).toHaveBeenLastCalledWith({ kind: 'others' });
  });

  it('does not offer the scope already showing', () => {
    expect(command('library.showAllItems').enabled(screenCtx)).toBe(false);
    expect(command('library.showAllItems').disabledReason).toBe('already showing');
    // From a document it is a way back, so it is offered.
    expect(command('library.showAllItems').enabled(readerCtx())).toBe(true);
    expect(command('library.showNotFiled').enabled(screenCtx)).toBe(true);
  });

  it('toggles between list and graph, landing on the graph from elsewhere', () => {
    const toggle = command('library.toggleGraphView');
    toggle.run(screenCtx);
    expect(h.store.state.view).toBe('graph');
    toggle.run(screenCtx);
    expect(h.store.state.view).toBe('list');

    // Not filed has no graph: the graph is All items'.
    h.store.state.scope = { kind: 'unfiled' };
    toggle.run(screenCtx);
    expect(h.store.setScope).toHaveBeenLastCalledWith({ kind: 'all' });
    expect(h.store.state.view).toBe('graph');

    // From a document, the toggle shows the graph rather than hiding it.
    toggle.run(readerCtx());
    expect(h.store.state.view).toBe('graph');
    expect(h.goHome).toHaveBeenCalled();
  });

  it('names the missing-works toggle by what it will do', () => {
    expect(titles(screenCtx)).toContain('Show Missing Works in Graph');
    row(screenCtx, 'Show Missing Works in Graph').run();
    expect(h.store.setMissing).toHaveBeenLastCalledWith(true);
    expect(h.store.state.view).toBe('graph');
    expect(titles(screenCtx)).toContain('Hide Missing Works in Graph');
    row(screenCtx, 'Hide Missing Works in Graph').run();
    expect(h.store.setMissing).toHaveBeenLastCalledWith(false);
  });

  it('fits the graph on screen, or shows the graph (which fits as it arrives)', () => {
    const fit = vi.fn();
    const off = intents.registerGraphControls({ fit, resetLayout: vi.fn(), hasMoved: () => false });
    try {
      h.store.state.view = 'graph';
      command('library.fitGraph').run(screenCtx);
      expect(fit).toHaveBeenCalledTimes(1);
      command('library.fitGraph').run(readerCtx());
      expect(fit).toHaveBeenCalledTimes(1);
      expect(h.goHome).toHaveBeenCalled();
    } finally {
      off();
    }
  });

  it("resets the current scope's stored positions, and only when there are some", () => {
    const reset = command('library.resetGraphLayout');
    expect(reset.enabled(screenCtx)).toBe(false);
    h.store.state.scope = { kind: 'project', id: 'p1' };
    h.positions['p1'] = { a: { x: 1, y: 2 } };
    expect(reset.enabled(screenCtx)).toBe(true);
    reset.run(screenCtx);
    // No graph mounted: the positions go, and the graph comes up fresh.
    expect(h.clearPositions).toHaveBeenCalledWith('p1');
    expect(h.store.state.view).toBe('graph');

    const resetLayout = vi.fn();
    const off = intents.registerGraphControls({ fit: vi.fn(), resetLayout, hasMoved: () => true });
    try {
      h.store.state.focus = { paperId: 'a', depth: 1 };
      reset.run(screenCtx);
      expect(resetLayout).toHaveBeenCalled();
    } finally {
      off();
    }
  });

  it('focuses the graph on the selected item, or on the open one', () => {
    const show = command('library.showItemInGraph');
    expect(show.enabled(screenCtx)).toBe(false);
    h.store.state.selected = paper({ id: 'b' });
    show.run(screenCtx);
    expect(h.store.showInGraph).toHaveBeenLastCalledWith('b', 1);

    h.active.paper = paper({ id: 'c' });
    h.active.known = true;
    show.run(readerCtx());
    expect(h.store.select).toHaveBeenLastCalledWith('c');
    expect(h.store.showInGraph).toHaveBeenLastCalledWith('c', 1);
    expect(h.goHome).toHaveBeenCalled();

    expect(command('library.leaveGraphFocus').enabled(screenCtx)).toBe(true);
    command('library.leaveGraphFocus').run(screenCtx);
    expect(h.store.leaveFocus).toHaveBeenCalled();
    expect(command('library.leaveGraphFocus').enabled(screenCtx)).toBe(false);
  });

  it('ignores a selection hidden behind a picked missing work', () => {
    h.store.state.selected = paper();
    h.store.state.ghostKey = 'ghost';
    expect(command('library.showItemInGraph').enabled(screenCtx)).toBe(false);
  });
});

describe('projects', () => {
  it('asks the left column for its new-project field', () => {
    updateSettings({ launchScreen: 'home' });
    command('library.newProject').run(screenCtx);
    expect(settings.launchScreen).toBe('projects');
    expect(intents.newProjectRequested()).toBe(true);
    intents.takeNewProjectRequest();
  });

  it('gates lookups on the online lookup setting', () => {
    const all = command('library.lookUpAll');
    const one = command('library.lookUpProject');
    h.store.state.scope = { kind: 'project', id: 'p2' };
    expect(all.enabled(screenCtx)).toBe(false);
    expect(all.disabledReason).toMatch(/Settings/);
    expect(one.enabled(screenCtx)).toBe(false);
    updateSettings({ onlineLookup: true });
    expect(all.enabled(screenCtx)).toBe(true);
    expect(one.enabled(screenCtx)).toBe(true);
    all.run(screenCtx);
    expect(h.store.lookupAll).toHaveBeenCalled();
    one.run(screenCtx);
    expect(h.store.lookupProject).toHaveBeenCalledWith('p2');
    // No project open: nothing to look up "this project" for.
    h.store.state.scope = { kind: 'all' };
    expect(one.enabled(screenCtx)).toBe(false);
  });

  it('runs the .bib actions for the project open on the screen', () => {
    const exportCmd = command('library.exportProjectBib');
    const sync = command('library.syncProjectBib');
    const stop = command('library.stopSyncingProjectBib');
    expect(exportCmd.enabled(screenCtx)).toBe(false);

    h.store.state.scope = { kind: 'project', id: 'p1' };
    expect(exportCmd.enabled(screenCtx)).toBe(true);
    expect(sync.enabled(screenCtx)).toBe(true);
    // p1 keeps no .bib.
    expect(stop.enabled(screenCtx)).toBe(false);
    exportCmd.run(screenCtx);
    expect(h.exportBib).toHaveBeenCalledWith(expect.objectContaining({ id: 'p1' }));
    sync.run(screenCtx);
    expect(h.syncBib).toHaveBeenCalledWith(expect.objectContaining({ id: 'p1' }));

    h.store.state.scope = { kind: 'project', id: 'p2' };
    expect(stop.enabled(screenCtx)).toBe(true);
    stop.run(screenCtx);
    expect(h.stopSyncingBib).toHaveBeenCalledWith(expect.objectContaining({ id: 'p2' }));

    // The scope stays put under an open document, but it is not on screen.
    expect(exportCmd.enabled(readerCtx())).toBe(false);
  });

  it('hands the full-text search over with the project as its scope', () => {
    h.store.state.scope = { kind: 'project', id: 'p1' };
    h.store.state.query = 'fluctuation';
    command('library.searchProjectFullText').run(screenCtx);
    expect(h.searchFullText).toHaveBeenCalledWith('fluctuation', 'p1');
  });

  it('lists a Go to Project row per project, and follows the projects', () => {
    expect(titles(screenCtx)).toEqual(
      expect.arrayContaining(['Go to Project: Noneq stat mech', 'Go to Project: Active matter'])
    );
    row(screenCtx, 'Go to Project: Active matter').run();
    expect(h.store.setScope).toHaveBeenLastCalledWith({ kind: 'project', id: 'p2' });

    h.store.state.projects = h.store.state.projects.slice(0, 1);
    expect(titles(screenCtx)).not.toContain('Go to Project: Active matter');
    h.store.state.projects = [];
    expect(titles(screenCtx).some((t) => t.startsWith('Go to Project'))).toBe(false);
  });
});

describe('items', () => {
  it('opens the merge confirmation and never merges by itself', () => {
    const merge = command('library.mergeSelected');
    expect(merge.enabled(screenCtx)).toBe(false);
    h.cite.rows = [
      { id: 'a', citeKey: 'x' },
      { id: 'b', citeKey: 'y' },
    ];
    expect(merge.enabled(screenCtx)).toBe(true);
    expect(merge.enabled(readerCtx())).toBe(false);
    const begin = vi.fn();
    const off = intents.registerMergeRequest(begin);
    try {
      merge.run(screenCtx);
      expect(begin).toHaveBeenCalledTimes(1);
      expect(h.store.mergePapers).not.toHaveBeenCalled();
    } finally {
      off();
    }
  });

  it('splits a merged item on the screen only', () => {
    const split = command('library.splitItem');
    h.store.state.selected = paper();
    expect(split.enabled(screenCtx)).toBe(false);
    h.store.state.selected = paper({ mergedCount: 1 });
    expect(split.enabled(screenCtx)).toBe(true);
    split.run(screenCtx);
    expect(h.store.unmerge).toHaveBeenCalledWith('a');
  });

  it('marks the selected or open record, offering only the change', async () => {
    const asItem = command('library.markAsItem');
    const notItem = command('library.markNotItem');
    h.store.state.selected = paper({ isPaper: false });
    expect(asItem.enabled(screenCtx)).toBe(true);
    expect(notItem.enabled(screenCtx)).toBe(false);
    asItem.run(screenCtx);
    expect(h.store.updatePaper).toHaveBeenLastCalledWith('a', { isPaper: true });

    h.active.paper = paper({ id: 'r', isPaper: true });
    h.active.known = true;
    expect(notItem.enabled(readerCtx())).toBe(true);
    notItem.run(readerCtx());
    expect(h.store.updatePaper).toHaveBeenLastCalledWith('r', { isPaper: false });
    // The reader's cached record takes the answer at once.
    await vi.waitFor(() =>
      expect(h.rememberPaper).toHaveBeenCalledWith(
        '/lib/a.pdf',
        expect.objectContaining({ isPaper: false })
      )
    );
  });

  it('looks up by title where the matches can be picked, online only', () => {
    const byTitle = command('library.lookUpByTitle');
    h.active.paper = paper({ id: 'r' });
    h.active.known = true;
    expect(byTitle.enabled(readerCtx())).toBe(false);
    updateSettings({ onlineLookup: true });
    expect(byTitle.enabled(readerCtx())).toBe(true);
    byTitle.run(readerCtx());
    expect(h.store.select).toHaveBeenLastCalledWith('r');
    expect(settings.launchScreen).toBe('projects');
    expect(intents.titleLookupRequested()).toBe('r');
    intents.takeTitleLookupRequest();
  });

  it('adds the item to its top suggestion, skipping what was dismissed', async () => {
    const add = command('library.addToSuggestedProject');
    h.store.state.selected = paper({ projectIds: ['p1'] });
    const list: ProjectSuggestion[] = [
      { projectId: 'p1', name: 'Noneq stat mech', score: 3, reason: 'cites 3' },
      { projectId: 'p2', name: 'Active matter', score: 2, reason: 'cites 2' },
    ] as ProjectSuggestion[];
    h.suggest.mockResolvedValueOnce(list);
    expect(add.enabled(screenCtx)).toBe(false);
    await loadSuggestions('a');
    // p1 already holds it, so p2 is the suggestion.
    expect(add.enabled(screenCtx)).toBe(true);
    add.run(screenCtx);
    expect(h.store.setMembership).toHaveBeenCalledWith('a', 'p2', true);

    dismissSuggestion('a', 'p2');
    expect(add.enabled(screenCtx)).toBe(false);
    expect(add.disabledReason).toBe('no project is suggested for this item');
  });

  it("primes the open item's record and its suggestions as the palette opens", async () => {
    h.refreshActivePaper.mockResolvedValueOnce(paper({ id: 'r' }));
    primeProjectPalette(readerCtx());
    await vi.waitFor(() => expect(h.suggest).toHaveBeenCalledWith('r'));
  });

  it('offers Add to Project rows for the projects the item is not in', () => {
    expect(titles(screenCtx).some((t) => t.startsWith('Add to Project'))).toBe(false);
    h.store.state.selected = paper({ projectIds: ['p1'] });
    expect(titles(screenCtx)).toContain('Add to Project: Active matter');
    expect(titles(screenCtx)).not.toContain('Add to Project: Noneq stat mech');
    row(screenCtx, 'Add to Project: Active matter').run();
    expect(h.store.setMembership).toHaveBeenCalledWith('a', 'p2', true);

    // In the reader, the open record is the item.
    h.active.paper = paper({ id: 'r', projectIds: ['p2'] });
    h.active.known = true;
    expect(titles(readerCtx())).toContain('Add to Project: Noneq stat mech');
    expect(titles(readerCtx())).not.toContain('Add to Project: Active matter');

    // Every project holds it: no rows.
    h.active.paper = paper({ id: 'r', projectIds: ['p1', 'p2'] });
    expect(titles(readerCtx()).some((t) => t.startsWith('Add to Project'))).toBe(false);
  });

  it("opens the item's other PDFs, never the one already open or a missing one", () => {
    const files = [file('preprint.pdf'), file('journal.pdf'), file('gone.pdf', true)];
    h.store.state.selected = paper({ files });
    expect(titles(screenCtx).filter((t) => t.startsWith('Open '))).toEqual(['Open journal.pdf']);
    row(screenCtx, 'Open journal.pdf').run();
    expect(h.openInNewTabOrFocus).toHaveBeenCalledWith('/lib/journal.pdf');

    h.active.paper = paper({ files });
    h.active.known = true;
    expect(titles(readerCtx('/lib/journal.pdf')).filter((t) => t.startsWith('Open '))).toEqual([
      'Open preprint.pdf',
    ]);

    // A single file has no other.
    h.store.state.selected = paper();
    expect(titles(screenCtx).some((t) => t.startsWith('Open '))).toBe(false);
  });
});

describe('ASCII', () => {
  it('titles and reasons are plain ASCII', () => {
    for (const c of PROJECT_COMMANDS) {
      expect(c.title).toMatch(/^[\x20-\x7e]+$/);
      if (c.disabledReason) expect(c.disabledReason).toMatch(/^[\x20-\x7e]+$/);
    }
  });
});

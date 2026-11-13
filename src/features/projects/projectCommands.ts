/** The Projects screen and the library, as palette commands.
 *
 * Two halves. `PROJECT_COMMANDS` are ordinary registry entries (commands.ts
 * spreads them into COMMANDS), with fixed titles. `projectPaletteRows` is
 * the palette's provider for rows that exist per thing: one per project, one
 * per other file of the current item, and the missing-works toggle, whose
 * title says what it will do.
 *
 * "The current item" is the one the user is looking at: the record selected
 * on the Projects screen while that screen shows, else the open document's
 * record (activePaper.ts). Commands that act on the screen switch to it
 * rather than refuse when it is not showing; commands that need something
 * only the screen can have (a list selection, a project open) say so. */
import { createSignal } from 'solid-js';
import type { Command, CommandCtx } from '../../app/commands';
import { engine } from '../../lib/transport/engine';
import { settings, updateSettings } from '../../stores/settings';
import type { Paper, Project, ProjectSuggestion } from '../../types/engine';
import { goHome, openInNewTabOrFocus } from '../document/tabsController';
import { openInNewWindow } from '../windows/windowController';
import type { Candidate } from '../palette/paletteItems';
import { activePaper, activePaperKnown, refreshActivePaper, rememberPaper } from './activePaper';
import { exportBib, exportBundle, stopSyncingBib, syncBib } from './bibActions';
import { citeSelection } from './citeSelection';
import { searchFullText } from './fullTextHandoff';
import { clearPositions, loadPositions, positionsKey } from './graphPositions';
import { healthCount, projectsStore, sameScope, type Scope } from './projectsStore';
import {
  graphControls,
  requestMerge,
  requestNewProject,
  requestTitleLookup,
} from './screenIntents';
import { suggestionDismissed } from './suggestionDismissals';

const store = projectsStore;

/* ---- where the user is ---- */

/** The Projects launch screen is what the window shows. */
export function projectsShowing(ctx: CommandCtx): boolean {
  return ctx.tab === null && settings.launchScreen === 'projects';
}

export function showProjectsScreen(): void {
  if (settings.launchScreen !== 'projects') updateSettings({ launchScreen: 'projects' });
  goHome();
}

/** The record selected on the Projects screen while it shows, else the
 * open document's record. A picked missing work hides the selection, so it
 * is not acted on unseen. */
export function currentItem(ctx: CommandCtx): Paper | null {
  if (ctx.tab) return ctx.tab.documentStore.state.loaded ? activePaper() : null;
  if (!projectsShowing(ctx)) return null;
  return store.state.ghostKey ? null : store.state.selected;
}

/** The project open on the Projects screen. */
export function currentProject(ctx: CommandCtx): Project | null {
  const scope = store.state.scope;
  if (!projectsShowing(ctx) || scope.kind !== 'project') return null;
  return store.state.projects.find((p) => p.id === scope.id) ?? null;
}

/** A scope with a graph: a project, All items, or any focused item. */
function graphable(): boolean {
  const kind = store.state.scope.kind;
  return kind === 'project' || kind === 'all' || store.state.focus !== null;
}

function graphShowing(ctx: CommandCtx): boolean {
  return projectsShowing(ctx) && graphable() && store.state.view === 'graph';
}

/** The Projects screen, in the graph. Not filed and Other PDFs have none,
 * so they give way to All items. */
function showGraph(): void {
  showProjectsScreen();
  if (!graphable()) store.setScope({ kind: 'all' });
  store.setView('graph');
}

function currentPositionsKey(): string {
  return positionsKey(store.state.scope, store.state.focus);
}

/* ---- suggestions ---- */

const [suggested, setSuggested] = createSignal<{
  paperId: string;
  list: ProjectSuggestion[];
} | null>(null);
let suggestSeq = 0;

/** Reads the projects suggested for an item; the latest ask wins. */
export function loadSuggestions(paperId: string): Promise<void> {
  const mine = ++suggestSeq;
  return engine.librarySuggestProjects(paperId).then(
    (list) => {
      if (mine === suggestSeq) setSuggested({ paperId, list });
    },
    () => {
      // A suggestion is a nicety: none is the quiet answer.
      if (mine === suggestSeq) setSuggested({ paperId, list: [] });
    }
  );
}

/** What the item's suggestion line would offer: the strongest project it
 * is not in and was not told Not now about. */
export function topSuggestion(paper: Paper): ProjectSuggestion | null {
  const known = suggested();
  if (!known || known.paperId !== paper.id) return null;
  return (
    known.list.find(
      (s) => !paper.projectIds.includes(s.projectId) && !suggestionDismissed(paper.id, s.projectId)
    ) ?? null
  );
}

/** Called as the palette opens: the reads its rows depend on. None of them
 * touches a page; each is one engine call answered from library.json. */
export function primeProjectPalette(ctx: CommandCtx): void {
  if (!store.state.initialized) void store.initialize();
  if (ctx.tab?.documentStore.state.loaded && !activePaperKnown()) {
    refreshActivePaper().then(
      (paper) => {
        if (paper) void loadSuggestions(paper.id);
      },
      () => undefined
    );
    return;
  }
  const item = currentItem(ctx);
  if (item) void loadSuggestions(item.id);
}

/* ---- actions ---- */

async function markItem(ctx: CommandCtx, paper: Paper, isPaper: boolean): Promise<void> {
  const updated = await store.updatePaper(paper.id, { isPaper });
  // The reader's record is cached for the other Library commands; the
  // change event empties that cache, but this answer is already the truth.
  const path = ctx.tab?.documentStore.state.path;
  if (updated && path) rememberPaper(path, updated);
}

/** The item's files worth offering to open: on the Projects screen every
 * file but the main one (Open is that), in the reader every file but the
 * one open. Missing files cannot be opened. */
export function otherFiles(ctx: CommandCtx, paper: Paper): Paper['files'] {
  if (paper.files.length < 2) return [];
  const open = ctx.tab?.documentStore.state.path ?? null;
  return paper.files.filter((file, index) =>
    file.missing ? false : ctx.tab ? file.path !== open : index > 0
  );
}

function scopeCommand(id: string, title: string, scope: Scope, keywords: string[]): Command {
  return {
    id,
    title,
    group: 'Library',
    keywords: ['projects', 'scope', ...keywords],
    enabled: (ctx) => !(projectsShowing(ctx) && sameScope(store.state.scope, scope)),
    disabledReason: 'already showing',
    run: () => {
      showProjectsScreen();
      store.setScope(scope);
    },
  };
}

const NEEDS_ITEM = 'needs an item selected on the Projects screen, or an open item';
const NEEDS_PROJECT = 'needs a project open on the Projects screen';

export const PROJECT_COMMANDS: readonly Command[] = [
  // ---- the screen ----
  scopeCommand('library.showAllItems', 'Show All Items', { kind: 'all' }, ['library', 'every']),
  scopeCommand('library.showNotFiled', 'Show Not Filed', { kind: 'unfiled' }, [
    'unfiled',
    'inbox',
    'no project',
  ]),
  scopeCommand('library.showOtherPdfs', 'Show Other PDFs', { kind: 'others' }, [
    'not items',
    'manuals',
    'files',
  ]),
  // New and Health are rows only while they hold something; so are these.
  {
    ...scopeCommand('library.showNewItems', 'Show New Items', { kind: 'new' }, [
      'new',
      'arrivals',
      'triage',
      'recent',
    ]),
    enabled: (ctx) =>
      store.state.newItems.length > 0 &&
      !(projectsShowing(ctx) && store.state.scope.kind === 'new'),
    disabledReason: 'nothing new to file',
  },
  {
    ...scopeCommand('library.showLibraryHealth', 'Show Library Health', { kind: 'health' }, [
      'health',
      'problems',
      'duplicates',
      'missing',
      'metadata',
    ]),
    enabled: (ctx) =>
      healthCount(store.state.health) > 0 &&
      !(projectsShowing(ctx) && store.state.scope.kind === 'health'),
    disabledReason: 'nothing to check',
  },
  {
    id: 'library.toggleGraphView',
    title: 'Toggle Graph View',
    group: 'Library',
    keywords: ['citation', 'graph', 'list', 'network', 'projects', 'view'],
    enabled: () => true,
    run: (ctx) => {
      if (graphShowing(ctx)) store.setView('list');
      else showGraph();
    },
  },
  {
    id: 'library.fitGraph',
    title: 'Fit Graph',
    group: 'Library',
    keywords: ['citation', 'zoom', 'frame', 'view', 'center'],
    enabled: () => true,
    run: (ctx) => {
      const controls = graphControls();
      if (controls && graphShowing(ctx) && !store.state.missingList) controls.fit();
      // A graph that arrives is fitted as it arrives.
      else showGraph();
    },
  },
  {
    id: 'library.resetGraphLayout',
    title: 'Reset Graph Layout',
    group: 'Library',
    keywords: ['citation', 'positions', 'dragged', 'arrange', 'undo'],
    enabled: () =>
      graphable() &&
      (Object.keys(loadPositions(currentPositionsKey())).length > 0 ||
        graphControls()?.hasMoved() === true),
    disabledReason: 'nothing has been dragged in this graph',
    run: (ctx) => {
      const controls = graphControls();
      if (controls && graphShowing(ctx)) {
        controls.resetLayout();
        return;
      }
      clearPositions(currentPositionsKey());
      showGraph();
    },
  },
  {
    id: 'library.showItemInGraph',
    title: 'Show Selected Item in Graph',
    group: 'Library',
    keywords: ['citation', 'focus', 'neighbourhood', 'neighborhood', 'cites', 'cited by'],
    enabled: (ctx) => currentItem(ctx) !== null,
    disabledReason: NEEDS_ITEM,
    run: (ctx) => {
      const item = currentItem(ctx);
      if (!item) return;
      showProjectsScreen();
      if (store.state.selectedId !== item.id) store.select(item.id);
      store.showInGraph(item.id, 1);
    },
  },
  {
    id: 'library.leaveGraphFocus',
    title: 'Leave Graph Focus',
    group: 'Library',
    keywords: ['citation', 'whole graph', 'unfocus', 'back'],
    enabled: () => store.state.focus !== null,
    disabledReason: 'the graph is not focused on an item',
    run: () => store.leaveFocus(),
  },

  // ---- projects ----
  {
    id: 'library.newProject',
    title: 'New Project...',
    group: 'Library',
    keywords: ['create', 'add', 'collection', 'folder'],
    enabled: () => true,
    run: () => {
      showProjectsScreen();
      requestNewProject();
    },
  },
  {
    id: 'library.lookUpAll',
    title: 'Look Up All Items',
    group: 'Library',
    keywords: ['metadata', 'crossref', 'arxiv', 'doi', 'online', 'fetch', 'batch', 'library'],
    enabled: () => settings.onlineLookup,
    disabledReason: 'turn on online lookup in Settings',
    run: () => void store.lookupAll(),
  },
  {
    id: 'library.lookUpProject',
    title: 'Look Up This Project',
    group: 'Library',
    keywords: ['metadata', 'crossref', 'arxiv', 'doi', 'online', 'fetch', 'batch'],
    enabled: (ctx) => settings.onlineLookup && currentProject(ctx) !== null,
    disabledReason: `${NEEDS_PROJECT}, and online lookup on in Settings`,
    run: (ctx) => {
      const project = currentProject(ctx);
      if (project) void store.lookupProject(project.id);
    },
  },
  {
    id: 'library.exportProjectBib',
    title: 'Export Project .bib...',
    group: 'Library',
    keywords: ['bibtex', 'save', 'latex', 'references', 'write'],
    enabled: (ctx) => currentProject(ctx) !== null,
    disabledReason: NEEDS_PROJECT,
    run: (ctx) => {
      const project = currentProject(ctx);
      if (project) void exportBib(project);
    },
  },
  {
    id: 'library.exportProjectBundle',
    title: 'Export Project Bundle...',
    group: 'Library',
    keywords: ['zip', 'archive', 'pdfs', 'bibtex', 'share', 'send', 'co-author', 'save'],
    enabled: (ctx) => currentProject(ctx) !== null,
    disabledReason: NEEDS_PROJECT,
    run: (ctx) => {
      const project = currentProject(ctx);
      if (project) void exportBundle(project);
    },
  },
  {
    id: 'library.syncProjectBib',
    title: 'Sync Project .bib...',
    group: 'Library',
    keywords: ['bibtex', 'keep', 'latex', 'references', 'auto', 'update'],
    enabled: (ctx) => currentProject(ctx) !== null,
    disabledReason: NEEDS_PROJECT,
    run: (ctx) => {
      const project = currentProject(ctx);
      if (project) void syncBib(project);
    },
  },
  {
    id: 'library.stopSyncingProjectBib',
    title: 'Stop Syncing Project .bib',
    group: 'Library',
    keywords: ['bibtex', 'forget', 'unsync', 'latex'],
    enabled: (ctx) => Boolean(currentProject(ctx)?.bibPath),
    disabledReason: `${NEEDS_PROJECT} that keeps a .bib`,
    run: (ctx) => {
      const project = currentProject(ctx);
      if (project?.bibPath) void stopSyncingBib(project);
    },
  },
  {
    id: 'library.searchProjectFullText',
    title: 'Search Full Text in This Project...',
    group: 'Library',
    keywords: ['find', 'text', 'contents', 'which paper', 'quote'],
    enabled: (ctx) => currentProject(ctx) !== null,
    disabledReason: NEEDS_PROJECT,
    run: (ctx) => {
      const project = currentProject(ctx);
      if (project) searchFullText(store.state.query, project.id);
    },
  },

  // ---- items ----
  {
    id: 'library.mergeSelected',
    title: 'Merge Selected Items',
    group: 'Library',
    keywords: ['duplicate', 'same', 'combine', 'join', 'preprint'],
    // The command only opens the list's confirmation; Merge there merges.
    enabled: (ctx) => projectsShowing(ctx) && citeSelection().length >= 2,
    disabledReason: 'needs two or more items selected in the Projects list',
    run: () => void requestMerge(),
  },
  {
    id: 'library.splitItem',
    title: 'Split Item',
    group: 'Library',
    keywords: ['unmerge', 'separate', 'undo merge', 'restore'],
    enabled: (ctx) => {
      const item = currentItem(ctx);
      return projectsShowing(ctx) && item !== null && item.mergedCount > 0;
    },
    disabledReason: 'needs a merged item selected on the Projects screen',
    run: (ctx) => {
      const item = currentItem(ctx);
      if (item && item.mergedCount > 0) void store.unmerge(item.id);
    },
  },
  {
    id: 'library.markAsItem',
    title: 'Mark as Item',
    group: 'Library',
    keywords: ['paper', 'is a paper', 'verdict', 'classify', 'include'],
    enabled: (ctx) => currentItem(ctx)?.isPaper === false,
    disabledReason: 'needs a selected or open record not treated as an item',
    run: (ctx) => {
      const item = currentItem(ctx);
      if (item) void markItem(ctx, item, true);
    },
  },
  {
    id: 'library.markNotItem',
    title: 'Mark as Not an Item',
    group: 'Library',
    keywords: ['not a paper', 'manual', 'form', 'verdict', 'classify', 'exclude'],
    enabled: (ctx) => currentItem(ctx)?.isPaper === true,
    disabledReason: 'needs a selected or open item',
    run: (ctx) => {
      const item = currentItem(ctx);
      if (item) void markItem(ctx, item, false);
    },
  },
  {
    id: 'library.lookUpByTitle',
    title: 'Look Up by Title',
    group: 'Library',
    keywords: ['metadata', 'find by title', 'search', 'crossref', 'online', 'match'],
    // The matches are picked in the Projects screen's detail column, so the
    // reader's item is shown there.
    enabled: (ctx) => settings.onlineLookup && currentItem(ctx)?.isPaper === true,
    disabledReason: `${NEEDS_ITEM}, and online lookup on in Settings`,
    run: (ctx) => {
      const item = currentItem(ctx);
      if (!item) return;
      showProjectsScreen();
      if (store.state.selectedId !== item.id) store.select(item.id);
      requestTitleLookup(item.id);
    },
  },
  {
    id: 'library.addToSuggestedProject',
    title: 'Add to Suggested Project',
    group: 'Library',
    keywords: ['file', 'project', 'suggestion', 'citations', 'membership'],
    enabled: (ctx) => {
      const item = currentItem(ctx);
      return item !== null && topSuggestion(item) !== null;
    },
    disabledReason: 'no project is suggested for this item',
    run: (ctx) => {
      const item = currentItem(ctx);
      const top = item ? topSuggestion(item) : null;
      if (item && top) void store.setMembership(item.id, top.projectId, true);
    },
  },
];

/* ---- per-thing rows ---- */

/** The palette's rows for projects, the current item's projects and files,
 * and the missing-works toggle. Each row is runnable when listed: a row with
 * nothing to act on is left out rather than greyed. */
export function projectPaletteRows(ctx: CommandCtx): Candidate[] {
  const rows: Candidate[] = [];
  const missingOn = store.state.missing.on;
  rows.push({
    key: 'library.toggleMissingWorks',
    title: missingOn ? 'Hide Missing Works in Graph' : 'Show Missing Works in Graph',
    group: 'Library',
    keywords: ['ghost', 'cited', 'not in library', 'citation', 'graph', 'toggle', 'missing'],
    enabled: true,
    run: () => {
      if (missingOn) {
        store.setMissing(false);
        return;
      }
      store.setMissing(true);
      showGraph();
    },
  });

  for (const project of store.state.projects) {
    rows.push({
      key: `project:go:${project.id}`,
      title: `Go to Project: ${project.name}`,
      group: 'Projects',
      keywords: ['open', 'switch', 'show'],
      enabled: true,
      run: () => {
        showProjectsScreen();
        store.setScope({ kind: 'project', id: project.id });
      },
    });
  }

  const item = currentItem(ctx);
  if (!item) return rows;
  for (const project of store.state.projects) {
    if (item.projectIds.includes(project.id)) continue;
    rows.push({
      key: `project:add:${project.id}`,
      title: `Add to Project: ${project.name}`,
      group: 'Projects',
      keywords: ['file', 'membership', 'put'],
      enabled: true,
      run: () => void store.setMembership(item.id, project.id, true),
    });
  }
  for (const file of otherFiles(ctx, item)) {
    rows.push({
      key: `item:file:${file.path}`,
      title: `Open ${file.name}`,
      group: 'Projects',
      keywords: ['other pdf', 'file', 'version', 'copy', 'this item'],
      context: 'other PDF of this item',
      enabled: true,
      run: () => {
        openInNewTabOrFocus(file.path).catch((error: unknown) => store.reportError(error));
      },
      runAlt: () => void openInNewWindow(file.path),
    });
  }
  return rows;
}

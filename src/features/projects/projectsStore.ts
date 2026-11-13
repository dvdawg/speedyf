/** Projects and paper records: the state behind the Projects launch screen
 * and the reader's Paper panel.
 *
 * Global like libraryStore, and for the same reason: a project is not a
 * property of any open document. Every write goes to the engine, which owns
 * the one `library.json`, and the engine answers every write — from this
 * window, another window, or a background scan — with a `library:changed`
 * event. This store re-reads on that event and never assumes it is the only
 * writer. See projects.md §3.
 *
 * Reads are sequenced the way libraryStore sequences searches: a slower
 * earlier answer must not overwrite a faster later one. */
import { createStore, reconcile } from 'solid-js/store';
import { engine, isEngineError } from '../../lib/transport/engine';
import { forgetPapers } from './activePaper';
import type { GraphFocus } from '../../lib/transport/engine';
import type {
  DuplicateGroup,
  LibraryCounts,
  LookupBatch,
  MetaPatch,
  MissingWork,
  Paper,
  PaperFilter,
  PaperRow,
  Project,
} from '../../types/engine';

/** What the middle column shows. */
export type Scope =
  { kind: 'all' } | { kind: 'unfiled' } | { kind: 'others' } | { kind: 'project'; id: string };

export function filterFor(scope: Scope, query: string): PaperFilter {
  const trimmed = query.trim();
  switch (scope.kind) {
    case 'all':
      return { query: trimmed };
    case 'unfiled':
      return { unfiled: true, query: trimmed };
    case 'others':
      return { others: true, query: trimmed };
    case 'project':
      return { project: scope.id, query: trimmed };
  }
}

/** How the middle column shows a project or All items: rows, or the
 * citation graph between them. */
export type ProjectsView = 'list' | 'graph';

export function sameScope(a: Scope, b: Scope): boolean {
  return a.kind === b.kind && (a.kind !== 'project' || b.kind !== 'project' || a.id === b.id);
}

/** The records the engine suspects are one work with a given record: the
 * others in its group, and why. */
export interface DuplicateMatch {
  others: string[];
  reason: string;
}

/** Group by group, every member to the rest of its group. A record in two
 * groups (the engine should not answer so, but may) gets both lists. */
export function duplicateIndex(groups: DuplicateGroup[]): Record<string, DuplicateMatch> {
  const out: Record<string, DuplicateMatch> = {};
  for (const group of groups) {
    for (const id of group.paperIds) {
      const others = group.paperIds.filter((other) => other !== id);
      if (others.length === 0) continue;
      const had = out[id];
      out[id] = had
        ? {
            others: [...new Set([...had.others, ...others])],
            reason: had.reason === group.reason ? had.reason : `${had.reason}; ${group.reason}`,
          }
        : { others, reason: group.reason };
    }
  }
  return out;
}

/** Works the view's items cite that the library lacks, drawn as ghosts in
 * the graph. Session-only, like the view. */
export const MISSING_MIN = 1;
export const MISSING_MAX = 5;

interface State {
  initialized: boolean;
  projects: Project[];
  counts: LibraryCounts;
  scope: Scope;
  query: string;
  rows: PaperRow[];
  total: number;
  loadingRows: boolean;
  selectedId: string | null;
  selected: Paper | null;
  /** Project groups the user folded in the All papers view. Session-only. */
  collapsed: Record<string, boolean>;
  /** List or graph, kept across scope changes for the session. Scopes that
   * have no graph (Not filed, Other PDFs) show the list regardless. */
  view: ProjectsView;
  /** Record id → the others the engine thinks are the same work. */
  duplicates: Record<string, DuplicateMatch>;
  /** The graph narrowed to one item's neighbourhood; null is the scope's
   * whole graph. Cleared by a scope change. */
  focus: GraphFocus | null;
  /** Ghost nodes for works cited by at least `min` items of the view. */
  missing: { on: boolean; min: number };
  missingWorks: MissingWork[];
  /** The middle column lists the missing works instead of drawing them. */
  missingList: boolean;
  /** A missing work picked in the graph or its list. Not a record, so not
   * `selectedId`: the detail column shows it in place of the record until
   * a record is selected again. */
  ghostKey: string | null;
  /** The graph's labels for its items ("ZD24a"), so the missing works'
   * cited-by lists name items the way the graph does. */
  graphLabels: Record<string, string>;
  error: string | null;
}

const [state, setState] = createStore<State>({
  initialized: false,
  projects: [],
  counts: { all: 0, unfiled: 0, others: 0 },
  scope: { kind: 'all' },
  query: '',
  rows: [],
  total: 0,
  loadingRows: false,
  selectedId: null,
  selected: null,
  collapsed: {},
  view: 'list',
  duplicates: {},
  focus: null,
  missing: { on: false, min: 2 },
  missingWorks: [],
  missingList: false,
  ghostKey: null,
  graphLabels: {},
  error: null,
});

let rowsSeq = 0;
let detailSeq = 0;
let missingSeq = 0;
let queryTimer: ReturnType<typeof setTimeout> | undefined;
let unlisten: (() => void) | null = null;

const message = (error: unknown) => (isEngineError(error) ? error.message : String(error));

async function guarded<T>(work: () => Promise<T>): Promise<T | undefined> {
  try {
    const result = await work();
    setState('error', null);
    return result;
  } catch (error) {
    setState('error', message(error));
    return undefined;
  }
}

export const projectsStore = {
  state,

  /** Idempotent. Loads projects and rows and subscribes to changes. */
  async initialize(): Promise<void> {
    if (!state.initialized) {
      setState('initialized', true);
      unlisten = await engine.onLibraryChanged(() => {
        // The commands' side table of records is stale the moment anything
        // changed; the Paper panel refills it, but only while it is mounted.
        forgetPapers();
        void projectsStore.refresh();
      });
    }
    await projectsStore.refresh();
  },

  dispose(): void {
    unlisten?.();
    unlisten = null;
    setState('initialized', false);
  },

  /** Re-reads everything the screen shows. Called on every change event, so
   * it must be cheap to call often: three reads, no UI work. */
  async refresh(): Promise<void> {
    await Promise.all([
      projectsStore.loadProjects(),
      projectsStore.loadCounts(),
      projectsStore.loadRows(),
      projectsStore.loadDuplicates(),
      projectsStore.loadMissing(),
      state.selectedId ? projectsStore.loadDetail(state.selectedId) : Promise.resolve(),
    ]);
  },

  async loadProjects(): Promise<void> {
    const projects = await guarded(() => engine.libraryProjects());
    // Keyed so a background change (a scan flush, another window) updates
    // rows in place instead of remounting them, which would throw away a
    // rename in progress or close an open menu.
    if (projects) setState('projects', reconcile(projects, { key: 'id' }));
  },

  async loadCounts(): Promise<void> {
    const counts = await guarded(() => engine.libraryCounts());
    if (counts) setState('counts', counts);
  },

  async loadRows(): Promise<void> {
    const seq = ++rowsSeq;
    setState('loadingRows', true);
    // Snapshot before the await: the filter must describe the scope the
    // request was made for, not whatever the user has clicked since.
    const filter = filterFor(state.scope, state.query);
    const list = await guarded(() => engine.libraryPapers(filter));
    if (seq !== rowsSeq) return;
    setState('loadingRows', false);
    if (!list) return;
    setState({ rows: list.rows, total: list.total });
    // The selection may have left the list (filtered out, moved project).
    // Keep the detail: the user is still looking at it.
  },

  async loadDetail(id: string): Promise<void> {
    const seq = ++detailSeq;
    const paper = await guarded(() => engine.libraryPaper(id));
    if (seq !== detailSeq) return;
    if (paper) setState('selected', paper);
  },

  /** A background read on every refresh, so a failure stays quiet: the
   * screen works without suspicions, and the bar would come back on every
   * change event. */
  async loadDuplicates(): Promise<void> {
    try {
      const groups = await engine.libraryDuplicates();
      setState('duplicates', reconcile(duplicateIndex(groups)));
    } catch {
      setState('duplicates', reconcile({}));
    }
  },

  /** The project the graph and the missing works are read for; null for
   * the library. */
  graphProjectId(): string | null {
    return state.scope.kind === 'project' ? state.scope.id : null;
  },

  /** Reads the missing works while they are asked for, sequenced like the
   * rows; off, it forgets them. */
  async loadMissing(): Promise<void> {
    const seq = ++missingSeq;
    if (!state.missing.on) {
      setState({ missingWorks: [], missingList: false, ghostKey: null });
      return;
    }
    const projectId = projectsStore.graphProjectId();
    const min = state.missing.min;
    const focus = state.focus ? { ...state.focus } : undefined;
    const works = await guarded(() => engine.libraryMissingWorks(projectId, min, focus));
    if (seq !== missingSeq || !works) return;
    setState('missingWorks', reconcile(works, { key: 'key' }));
    if (state.ghostKey && !works.some((w) => w.key === state.ghostKey)) {
      setState('ghostKey', null);
    }
  },

  setMissing(on: boolean): void {
    if (on === state.missing.on) return;
    setState('missing', 'on', on);
    void projectsStore.loadMissing();
  },

  setMissingMin(min: number): void {
    const clamped = Math.max(MISSING_MIN, Math.min(MISSING_MAX, Math.round(min)));
    if (clamped === state.missing.min) return;
    setState('missing', 'min', clamped);
    void projectsStore.loadMissing();
  },

  showMissingList(show: boolean): void {
    setState('missingList', show && state.missing.on);
  },

  /** Picks a missing work for the detail column; null lets it go. */
  selectGhost(key: string | null): void {
    setState('ghostKey', key);
  },

  setGraphLabels(labels: Record<string, string>): void {
    setState('graphLabels', reconcile(labels));
  },

  /** The graph around one item, `depth` citation steps out. */
  showInGraph(paperId: string, depth: 1 | 2 = 1): void {
    setState({
      focus: { paperId, depth },
      view: 'graph',
      missingList: false,
      ghostKey: null,
    });
    void projectsStore.loadMissing();
  },

  setFocusDepth(depth: 1 | 2): void {
    if (!state.focus || state.focus.depth === depth) return;
    setState('focus', 'depth', depth);
    void projectsStore.loadMissing();
  },

  /** Back to the scope's whole graph. */
  leaveFocus(): void {
    if (!state.focus) return;
    setState({ focus: null, ghostKey: null });
    void projectsStore.loadMissing();
  },

  setScope(scope: Scope): void {
    if (sameScope(scope, state.scope)) return;
    // Replace, not merge: a store merges objects by default, which would
    // leave a project's `id` behind on the `all` scope that follows it.
    setState('scope', reconcile(scope));
    // A focus and a picked missing work belong to the graph they were in.
    setState({ focus: null, ghostKey: null, missingList: false });
    void projectsStore.loadRows();
    void projectsStore.loadMissing();
  },

  /** Debounced, like the library search, so typing does not query per key. */
  setQuery(query: string): void {
    setState('query', query);
    clearTimeout(queryTimer);
    queryTimer = setTimeout(() => void projectsStore.loadRows(), 150);
  },

  select(id: string | null): void {
    setState('selectedId', id);
    if (id !== null) setState('ghostKey', null);
    if (id === null) {
      detailSeq += 1;
      setState('selected', null);
      return;
    }
    void projectsStore.loadDetail(id);
  },

  toggleGroup(projectId: string): void {
    setState('collapsed', projectId, (folded) => !folded);
  },

  setView(view: ProjectsView): void {
    setState({ view, missingList: false });
  },

  /* ---- writes. Each is answered by library:changed, which refreshes. ---- */

  async createProject(name: string): Promise<Project | undefined> {
    return guarded(() => engine.libraryCreateProject(name));
  },

  async renameProject(id: string, name: string): Promise<void> {
    await guarded(() => engine.libraryRenameProject(id, name));
  },

  async deleteProject(id: string): Promise<void> {
    await guarded(() => engine.libraryDeleteProject(id));
    if (state.scope.kind === 'project' && state.scope.id === id) {
      projectsStore.setScope({ kind: 'all' });
    }
  },

  async reorderProjects(ids: string[]): Promise<void> {
    await guarded(() => engine.libraryReorderProjects(ids));
  },

  async setMembership(paperId: string, projectId: string, member: boolean): Promise<void> {
    await guarded(() => engine.librarySetMembership(paperId, projectId, member));
  },

  async updatePaper(id: string, patch: MetaPatch): Promise<Paper | undefined> {
    const paper = await guarded(() => engine.libraryUpdatePaper(id, patch));
    if (paper && state.selectedId === id) setState('selected', paper);
    return paper;
  },

  /** The paper for an open document, or null if its file is outside the
   * library. Not cached: the Paper panel asks per document. */
  async paperForPath(path: string): Promise<Paper | null | undefined> {
    return guarded(() => engine.libraryPaperForPath(path));
  },

  async bibtexFor(paperIds: string[]): Promise<string | undefined> {
    return guarded(() => engine.libraryBibtex(paperIds));
  },

  /** Writes the project's entries once, to a path the user chose. */
  async exportBib(projectId: string, path: string): Promise<void> {
    await guarded(() => engine.libraryExportBib(projectId, path));
  },

  /** Records (or with null, forgets) the .bib the engine keeps rewriting for
   * this project. The project's `bibPath` arrives with the next refresh. */
  async setProjectBib(projectId: string, path: string | null): Promise<void> {
    await guarded(() => engine.librarySetProjectBib(projectId, path));
  },

  /** Looks up every paper in the project that has an identifier. Sequential
   * on the engine side, so a large project takes a while; the status bar
   * shows the activity for as long as it runs. */
  async lookupProject(projectId: string): Promise<LookupBatch | undefined> {
    return guarded(() => engine.libraryLookupProject(projectId));
  },

  /** Every paper in the library, by identifier or by title. */
  async lookupAll(): Promise<LookupBatch | undefined> {
    return guarded(() => engine.libraryLookupAll());
  },

  /** Folds `mergeIds` into `keepId` and selects the result. */
  async mergePapers(keepId: string, mergeIds: string[]): Promise<Paper | undefined> {
    const paper = await guarded(() => engine.libraryMergePapers(keepId, mergeIds));
    if (paper) {
      setState('selected', paper);
      projectsStore.select(paper.id);
    }
    return paper;
  },

  /** Undoes the merges into a record and selects the first record back. */
  async unmerge(paperId: string): Promise<Paper[] | undefined> {
    const restored = await guarded(() => engine.libraryUnmerge(paperId));
    const first = restored?.[0];
    if (first) {
      setState('selected', first);
      projectsStore.select(first.id);
    }
    return restored;
  },

  async discardCandidates(paperId: string): Promise<void> {
    await guarded(() => engine.libraryDiscardCandidates(paperId));
  },

  clearError(): void {
    setState('error', null);
  },

  /** For work the screen does outside the store (a lookup, an open) that
   * still belongs in the screen's one error bar. Returns the message so the
   * caller can also show it next to the control that failed. */
  reportError(error: unknown): string {
    const text = message(error);
    setState('error', text);
    return text;
  },
};

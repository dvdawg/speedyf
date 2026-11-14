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
import {
  CITED_WITH_MAX,
  CITED_WITH_MIN,
  loadPrefs,
  MISSING_MAX,
  MISSING_MIN,
  savePrefs,
} from './graphViewPrefs';
import type { EdgeMode, LabelMode, UnlinkedMode } from './graphViewPrefs';
import { loadSort, saveSort } from './listSortPrefs';
import type { ListSort } from './listSortPrefs';
import type {
  BundleReport,
  CoCitedWork,
  DuplicateGroup,
  HealthGroup,
  LibraryCounts,
  LookupBatch,
  MetaPatch,
  MissingWork,
  Paper,
  PaperFilter,
  PaperRow,
  PersonItems,
  Project,
} from '../../types/engine';

/** What the middle column shows. */
export type Scope =
  | { kind: 'all' }
  | { kind: 'unfiled' }
  | { kind: 'others' }
  | { kind: 'project'; id: string }
  /** items from the last week not yet triaged */
  | { kind: 'new' }
  /** what the engine suspects (library_health) */
  | { kind: 'health' };

export function filterFor(scope: Scope, query: string): PaperFilter {
  const trimmed = query.trim();
  switch (scope.kind) {
    case 'all':
    case 'new':
    case 'health':
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

/** A view name as stored or asked for, read back: anything but the graph is
 * the list. An extension's notepad was once a view (`ext:...`); it lives in
 * the detail column now, so such a value lands on the list. */
export function readView(view: string | null | undefined): ProjectsView {
  return view === 'graph' ? 'graph' : 'list';
}

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

export { CITED_WITH_MAX, CITED_WITH_MIN, MISSING_MAX, MISSING_MIN };

/** Which list the missing-works list shows: works the view's items cite
 * ("Cited by these"), or works the library cites alongside a project's
 * members ("Cited with these"). */
export type MissingMode = 'by' | 'with';

/** A cited-with work's key where a picked ghost's key goes: never a
 * missing-works key, which has no such prefix. */
export const withKey = (key: string) => `with:${key}`;

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
  /** "Items by John Smith": a person's items listed in place of the
   * scope's until left, like the focus. Cleared by a scope change. */
  person: PersonItems | null;
  /** Ghost nodes for works cited by at least `min` items of the view.
   * Kept across sessions with the rest of the graph's View choices. */
  missing: { on: boolean; min: number };
  /** Ghost nodes for works cited with at least `min` members. */
  citedWith: { on: boolean; min: number };
  /** The graph's display choices (graphViewPrefs.ts). */
  viewPrefs: { edges: EdgeMode; labels: LabelMode; unlinked: UnlinkedMode };
  /** Works often cited with the project's members; empty outside a project. */
  coCited: CoCitedWork[];
  /** Which works the missing-works list shows. */
  missingMode: MissingMode;
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
  /** Library health findings, grouped; empty when nothing is suspected. */
  health: HealthGroup[];
  /** Untriaged new items, newest first, for the New row's count. */
  newItems: PaperRow[];
  /** How the list is sorted (listSortPrefs.ts), for every scope. */
  sort: ListSort;
  /** Item id -> items of the scope citing it, while sorting by it. */
  citedHere: Record<string, number>;
  error: string | null;
  /** The outcome of a one-off action worth reading (an exported bundle),
   * shown in the screen's bar until dismissed or replaced. */
  notice: string | null;
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
  person: null,
  ...(() => {
    const prefs = loadPrefs();
    return {
      missing: prefs.missing,
      citedWith: prefs.citedWith,
      viewPrefs: { edges: prefs.edges, labels: prefs.labels, unlinked: prefs.unlinked },
    };
  })(),
  coCited: [],
  missingMode: 'by',
  missingWorks: [],
  missingList: false,
  ghostKey: null,
  graphLabels: {},
  health: [],
  newItems: [],
  sort: loadSort(),
  citedHere: {},
  error: null,
  notice: null,
});

let rowsSeq = 0;
let detailSeq = 0;
let missingSeq = 0;
let coCitedSeq = 0;
let citedSeq = 0;

/** How many findings the health list holds. */
export function healthCount(groups: readonly HealthGroup[]): number {
  return groups.reduce((sum, group) => sum + group.findings.length, 0);
}
let personSeq = 0;

function persistPrefs(): void {
  savePrefs({
    ...state.viewPrefs,
    missing: { ...state.missing },
    citedWith: { ...state.citedWith },
  });
}
let queryTimer: ReturnType<typeof setTimeout> | undefined;
let unlisten: (() => void) | null = null;

/** How many keys a bundle's missing list names before it says "and N more". */
const MISSING_SHOWN = 8;

/** "Exported 23 PDFs and the .bib; 2 missing: key1, key2". */
export function bundleSummary(report: BundleReport): string {
  const count = report.pdfs.length;
  const pdfs = count === 1 ? '1 PDF' : `${count} PDFs`;
  let text = `Exported ${pdfs} and the .bib`;
  const missing = report.missing;
  if (missing.length > 0) {
    const named = missing.slice(0, MISSING_SHOWN).join(', ');
    const more =
      missing.length > MISSING_SHOWN ? ` and ${missing.length - MISSING_SHOWN} more` : '';
    text += `; ${missing.length} missing: ${named}${more}`;
  }
  return text;
}

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
      projectsStore.loadCoCited(),
      projectsStore.loadHealth(),
      projectsStore.loadNew(),
      projectsStore.loadCitedHere(),
      state.selectedId ? projectsStore.loadDetail(state.selectedId) : Promise.resolve(),
      state.person ? projectsStore.loadPerson(state.person.id) : Promise.resolve(),
    ]);
  },

  /** List a person's items in place of the scope's. */
  async showPerson(id: string): Promise<void> {
    const seq = ++personSeq;
    const person = await guarded(() => engine.libraryPerson(id));
    if (seq !== personSeq || !person) return;
    setState('person', person);
  },

  /** Re-read the person shown, quietly: one that is gone (its items
   * merged or edited apart) just leaves the list. */
  async loadPerson(id: string): Promise<void> {
    const seq = ++personSeq;
    try {
      const person = await engine.libraryPerson(id);
      if (seq === personSeq && state.person?.id === id) setState('person', person);
    } catch {
      if (seq === personSeq) setState('person', null);
    }
  },

  /** Back to the scope's list. */
  leavePerson(): void {
    personSeq += 1;
    if (state.person) setState('person', null);
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
    const kind = state.scope.kind;
    if (kind === 'health') {
      setState({ loadingRows: false, rows: [], total: 0 });
      return;
    }
    const list =
      kind === 'new'
        ? await guarded(async () => {
            const rows = await engine.libraryNewItems(filter.query ?? '');
            return { rows, total: rows.length };
          })
        : await guarded(() => engine.libraryPapers(filter));
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
    if (!paper) return;
    setState('selected', paper);
    // Merged away (automatically, say): the engine answers with the item
    // it went into, and that is now the selection.
    if (paper.id !== id && state.selectedId === id) setState('selectedId', paper.id);
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

  /** A background read, quiet like the duplicates. With nothing left the
   * Health row goes, and a screen showing it falls back to All items. */
  async loadHealth(): Promise<void> {
    let groups: HealthGroup[];
    try {
      groups = await engine.libraryHealth();
    } catch {
      groups = [];
    }
    setState('health', reconcile(groups));
    if (groups.length === 0 && state.scope.kind === 'health') {
      projectsStore.setScope({ kind: 'all' });
    }
  },

  /** The New row's items, unfiltered. When the last one is triaged the row
   * goes, and a screen showing it falls back to All items. */
  async loadNew(): Promise<void> {
    let rows: PaperRow[];
    try {
      rows = await engine.libraryNewItems('');
    } catch {
      rows = [];
    }
    setState('newItems', reconcile(rows, { key: 'id' }));
    if (rows.length === 0 && state.scope.kind === 'new') {
      projectsStore.setScope({ kind: 'all' });
    }
  },

  /** The "Cited here" counts for the scope, read only while that sort is
   * on. A background read: quiet. */
  async loadCitedHere(): Promise<void> {
    const seq = ++citedSeq;
    const scope = state.scope.kind;
    if (state.sort !== 'cited' || scope === 'health' || scope === 'new') {
      setState('citedHere', reconcile({}));
      return;
    }
    let counts: Record<string, number>;
    try {
      counts = await engine.libraryCitedHere(filterFor(state.scope, ''));
    } catch {
      counts = {};
    }
    if (seq !== citedSeq) return;
    setState('citedHere', reconcile(counts));
  },

  setSort(sort: ListSort): void {
    if (sort === state.sort) return;
    setState('sort', sort);
    saveSort(sort);
    void projectsStore.loadCitedHere();
  },

  /** Never show this finding again; the change event refreshes the list. */
  async ignoreFinding(paperId: string, kind: string): Promise<void> {
    await guarded(() => engine.libraryHealthIgnore(paperId, kind));
  },

  /** Triaged: these new items leave the New list. */
  async markReviewed(paperIds: string[]): Promise<void> {
    if (paperIds.length === 0) return;
    await guarded(() => engine.libraryMarkReviewed(paperIds));
  },

  /** Files a new item into a project, which also triages it. */
  async fileNew(paperId: string, projectId: string): Promise<void> {
    await guarded(() => engine.librarySetMembership(paperId, projectId, true));
    await projectsStore.markReviewed([paperId]);
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
    // The graph's ghosts, or the list open on "Cited by these".
    const wanted = state.missing.on || (state.missingList && state.missingMode === 'by');
    if (!wanted) {
      setState('missingWorks', []);
      if (state.missingMode === 'by') setState('missingList', false);
      if (state.ghostKey && !state.ghostKey.startsWith('with:')) setState('ghostKey', null);
      return;
    }
    const projectId = projectsStore.graphProjectId();
    const min = state.missing.min;
    const focus = state.focus ? { ...state.focus } : undefined;
    const works = await guarded(() => engine.libraryMissingWorks(projectId, min, focus));
    if (seq !== missingSeq || !works) return;
    setState('missingWorks', reconcile(works, { key: 'key' }));
    const key = state.ghostKey;
    if (key && !key.startsWith('with:') && !works.some((w) => w.key === key)) {
      setState('ghostKey', null);
    }
  },

  /** Reads the works cited with the project's members, for the header
   * line, the list and the ghosts alike; outside a project there are none.
   * The missing-works threshold goes along so the two lists never repeat
   * each other. */
  async loadCoCited(): Promise<void> {
    const seq = ++coCitedSeq;
    const projectId = state.scope.kind === 'project' ? state.scope.id : null;
    if (projectId === null) {
      setState('coCited', []);
      return;
    }
    const min = state.citedWith.min;
    const byMin = state.missing.min;
    let works: CoCitedWork[];
    try {
      works = await engine.libraryCocitedWorks(projectId, min, byMin);
    } catch {
      // A background read: quiet, like the duplicates.
      works = [];
    }
    if (seq !== coCitedSeq) return;
    setState('coCited', reconcile(works, { key: 'key' }));
    const key = state.ghostKey;
    if (key?.startsWith('with:') && !works.some((w) => withKey(w.key) === key)) {
      setState('ghostKey', null);
    }
  },

  setMissing(on: boolean): void {
    if (on === state.missing.on) return;
    setState('missing', 'on', on);
    persistPrefs();
    void projectsStore.loadMissing();
  },

  setMissingMin(min: number): void {
    const clamped = Math.max(MISSING_MIN, Math.min(MISSING_MAX, Math.round(min)));
    if (clamped === state.missing.min) return;
    setState('missing', 'min', clamped);
    persistPrefs();
    void projectsStore.loadMissing();
    void projectsStore.loadCoCited();
  },

  setCitedWith(on: boolean): void {
    if (on === state.citedWith.on) return;
    setState('citedWith', 'on', on);
    persistPrefs();
  },

  setCitedWithMin(min: number): void {
    const clamped = Math.max(CITED_WITH_MIN, Math.min(CITED_WITH_MAX, Math.round(min)));
    if (clamped === state.citedWith.min) return;
    setState('citedWith', 'min', clamped);
    persistPrefs();
    void projectsStore.loadCoCited();
  },

  setEdgeMode(edges: EdgeMode): void {
    setState('viewPrefs', 'edges', edges);
    persistPrefs();
  },

  setLabelMode(labels: LabelMode): void {
    setState('viewPrefs', 'labels', labels);
    persistPrefs();
  },

  setUnlinkedMode(unlinked: UnlinkedMode): void {
    setState('viewPrefs', 'unlinked', unlinked);
    persistPrefs();
  },

  showMissingList(show: boolean): void {
    setState('missingList', show && (state.missing.on || state.missingMode === 'with'));
  },

  /** Switches the list between "Cited by these" and "Cited with these". */
  setMissingMode(mode: MissingMode): void {
    if (mode === state.missingMode) return;
    setState('missingMode', mode);
    void projectsStore.loadMissing();
  },

  /** The header line's way in: the graph's list, on "Cited with these". */
  openCitedWith(): void {
    setState({ view: 'graph', missingMode: 'with', missingList: true });
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
    personSeq += 1;
    setState({ focus: null, ghostKey: null, missingList: false, person: null });
    void projectsStore.loadRows();
    void projectsStore.loadMissing();
    void projectsStore.loadCoCited();
    void projectsStore.loadCitedHere();
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

  setView(view: ProjectsView | string): void {
    setState({ view: readView(view), missingList: false });
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

  /** Writes the project's bundle (.bib and main PDFs as one .zip) and says
   * what it held in the screen's notice bar. */
  async exportBundle(projectId: string, path: string): Promise<BundleReport | undefined> {
    setState('notice', null);
    const report = await guarded(() => engine.libraryExportBundle(projectId, path));
    if (report) setState('notice', bundleSummary(report));
    return report;
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

  /** Makes the file at `path` the item's main one. */
  async setPrimaryFile(paperId: string, path: string): Promise<Paper | undefined> {
    const paper = await guarded(() => engine.librarySetPrimaryFile(paperId, path));
    if (paper && state.selected?.id === paper.id) setState('selected', paper);
    return paper;
  },

  async discardCandidates(paperId: string): Promise<void> {
    await guarded(() => engine.libraryDiscardCandidates(paperId));
  },

  /** Say the outcome of a one-off action in the screen's bar. */
  showNotice(text: string): void {
    setState('notice', text);
  },

  clearError(): void {
    setState('error', null);
  },

  clearNotice(): void {
    setState('notice', null);
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

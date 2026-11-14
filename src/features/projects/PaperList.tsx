/** The middle column: the papers in the current scope, grouped under All
 * papers and flat otherwise.
 *
 * Grouping happens here, not in the engine: the rows already carry their
 * project ids and the groups are the projects in their stored order, so the
 * engine would only be handing the same rows back under headers. A paper in
 * three projects is listed three times on purpose; the overview is there to
 * show where things are, not to deduplicate them.
 *
 * Three fixed bands again: the header and search, the list, and nothing
 * below. Only the list scrolls, for the reason LibraryPanel gives.
 *
 * Keyboard handling lives on the list element with a roving tabindex, so
 * nothing here listens on the window and nothing fights the app's
 * shortcuts. Up and down move the selection, left and right leave the column
 * (or fold a group), Enter opens.
 *
 * A row pressed and pulled becomes a pointer drag (pointerDrag.ts) carrying
 * the multi-selection it is part of, or itself; it lands on a project row in
 * the left column, or on the project's notepad in the right one, where it
 * becomes citations. Not HTML5 drag and drop: see pointerDrag.ts for why. */
import {
  createEffect,
  createMemo,
  createSignal,
  Index,
  Match,
  on,
  onCleanup,
  Show,
  Switch,
} from 'solid-js';
import { IconChevronDown, IconChevronRight, IconClose, IconSearch } from '../../components/icons';
import { libraryStore } from '../citations/libraryStore';
import { settings } from '../../stores/settings';
import type { LookupBatch, PaperRow, Project } from '../../types/engine';
import { baseName, exportBib, syncBib } from './bibActions';
import { copyCite, registerCiteSelection } from './citeSelection';
import { projectRowAt, setDropHover } from './dnd';
import { searchFullText } from './fullTextHandoff';
import GraphView from './GraphView';
import HealthList from './HealthList';
import { nextSort, SORT_LABEL, sortRows } from './listSortPrefs';
import { MergeConfirm } from './MergeConfirm';
import { MissingList } from './MissingWorks';
import { openPaper } from './openPaper';
import { metaLine } from './paperFields';
import { beginPointerDrag } from './pointerDrag';
import { projectsStore } from './projectsStore';
import { PersonBar } from './PersonScope';
import { registerMergeRequest } from './screenIntents';
import MathText from '../../components/MathText';
import { citeText, noteDropAt, type NoteDropTarget } from '../extensions/editor/noteDrop';
import { backToProjectPane, shownProjectPane } from './ProjectPane';
import { projectPane } from '../extensions/surfaces';
import ViewToggle, { currentView } from './ViewToggle';

/** The group every paper with no project falls into under All papers. Not a
 * project id, so `collapsed` can hold it beside the real ones. */
export const UNFILED_GROUP = 'unfiled';

/** "also in Thesis, Noneq": the names of a row's other projects, in project
 * order, three at most and "+N" for the rest; `except` is the project the
 * row is being listed under. Empty for a row in one project. */
export function alsoIn(row: PaperRow, projects: Project[], except: string | null): string {
  if (row.projectIds.length < 2) return '';
  const names = projects
    .filter((p) => p.id !== except && row.projectIds.includes(p.id))
    .map((p) => p.name);
  if (names.length === 0) return '';
  const shown = names.slice(0, 3);
  const rest = names.length - shown.length;
  return `also in ${shown.join(', ')}${rest > 0 ? ` +${rest}` : ''}`;
}

/** "cited by 5 here", for the meta line while the list sorts by it. */
export function citedHereText(count: number | undefined): string {
  return count ? `cited by ${count} here` : '';
}

/** The digit pressed on a new item, as a 0-based project index; null for
 * any other key, or a digit with a modifier held (a shortcut, not triage). */
export function triageDigit(e: KeyboardEvent): number | null {
  if (e.metaKey || e.ctrlKey || e.altKey || !/^[1-9]$/.test(e.key)) return null;
  return Number(e.key) - 1;
}

type Item =
  | {
      kind: 'header';
      key: string;
      groupId: string;
      name: string;
      count: number;
      collapsed: boolean;
    }
  | { kind: 'row'; key: string; row: PaperRow; groupId?: string }
  | { kind: 'fulltext'; key: string };

export default function PaperList(props: { onMoveFocus: (dir: -1 | 1) => void }) {
  const store = projectsStore;
  const hasQuery = () => store.state.query.trim().length > 0;
  /** A person's items while one is shown, else the scope's. */
  const sourceRows = () => store.state.person?.rows ?? store.state.rows;
  const grouped = () => store.state.scope.kind === 'all' && !store.state.person;
  /** New items are triaged newest first; Health is not a list of rows. */
  const triage = () => store.state.scope.kind === 'new';
  const health = () => store.state.scope.kind === 'health';
  const sortable = () => !triage() && !health();
  /** The rows in the chosen order; under All items the groups below keep
   * it, so the sort applies within each. */
  const sortedRows = createMemo(() =>
    sortable()
      ? sortRows(sourceRows(), store.state.sort, store.state.citedHere)
      : sourceRows().slice()
  );
  const rowMeta = (row: PaperRow) => {
    const cited = store.state.sort === 'cited' && sortable() ? store.state.citedHere[row.id] : 0;
    return [metaLine(row), citedHereText(cited)].filter(Boolean).join(', ');
  };
  const project = () => {
    const scope = store.state.scope;
    return scope.kind === 'project'
      ? store.state.projects.find((p) => p.id === scope.id)
      : undefined;
  };
  /** A project and All items have a graph; Not filed and Other PDFs are
   * selections with no citations to show between them, so they list. */
  const view = currentView;
  /** The other projects a row is in; nothing under Not filed or Other PDFs. */
  const alsoText = (item: { row: PaperRow; groupId?: string }) => {
    const scope = store.state.scope;
    if (scope.kind === 'unfiled' || scope.kind === 'others') return '';
    const except = scope.kind === 'project' ? scope.id : (item.groupId ?? null);
    return alsoIn(item.row, store.state.projects, except);
  };

  const rowItems = createMemo<Item[]>(() => {
    const rows = sortedRows();
    if (!grouped()) return rows.map((row) => ({ kind: 'row', key: row.id, row }));
    const groups = [
      ...store.state.projects.map((p) => ({
        id: p.id,
        name: p.name,
        rows: rows.filter((r) => r.projectIds.includes(p.id)),
      })),
      {
        id: UNFILED_GROUP,
        name: 'Not in a project',
        rows: rows.filter((r) => r.projectIds.length === 0),
      },
    ];
    const out: Item[] = [];
    for (const group of groups) {
      // Under a query an empty group is noise between the matches; without
      // one it is information: the project exists and has nothing in it.
      if (group.rows.length === 0 && hasQuery()) continue;
      const collapsed = store.state.collapsed[group.id] === true;
      out.push({
        kind: 'header',
        key: `h:${group.id}`,
        groupId: group.id,
        name: group.name,
        count: group.rows.length,
        collapsed,
      });
      if (collapsed) continue;
      for (const row of group.rows) {
        out.push({ kind: 'row', key: `${group.id}:${row.id}`, row, groupId: group.id });
      }
    }
    return out;
  });
  const items = createMemo<Item[]>(() =>
    hasQuery() ? [...rowItems(), { kind: 'fulltext', key: 'fulltext' }] : rowItems()
  );

  /* ---- focus and selection ---- */
  const [focusIndex, setFocusIndex] = createSignal(0);
  /** The rows a drag carries: shift and cmd/ctrl clicks grow it. Local, and
   * separate from the store's one selected paper, which is what the detail
   * column shows. */
  const [multi, setMulti] = createSignal<string[]>([]);
  let anchor: number | null = null;
  /** The multi-selection being merged, while its confirmation shows. */
  const [merging, setMerging] = createSignal<string[] | null>(null);
  /** The list element; absent while the graph is showing. */
  let list: HTMLDivElement | undefined;

  const current = () => Math.min(focusIndex(), Math.max(0, items().length - 1));
  const focusItem = (index: number) =>
    list?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.focus();

  createEffect(
    on(
      // The fields, not the object: the store may update a scope in place.
      () => {
        const scope = store.state.scope;
        const at = scope.kind === 'project' ? scope.id : scope.kind;
        return `${at}/${store.state.person?.id ?? ''}`;
      },
      () => {
        setFocusIndex(0);
        setMulti([]);
        anchor = null;
        setBatch(null);
        setMerging(null);
      },
      { defer: true }
    )
  );

  const rowsBetween = (a: number, b: number): string[] => {
    const [from, to] = a < b ? [a, b] : [b, a];
    const ids = items()
      .slice(from, to + 1)
      .flatMap((item) => (item.kind === 'row' ? [item.row.id] : []));
    return Array.from(new Set(ids));
  };

  const clickRow = (index: number, row: PaperRow, e: MouseEvent) => {
    setFocusIndex(index);
    if (e.shiftKey && anchor !== null) {
      setMulti(rowsBetween(anchor, index));
    } else if (e.metaKey || e.ctrlKey) {
      setMulti((ids) =>
        ids.includes(row.id) ? ids.filter((id) => id !== row.id) : [...ids, row.id]
      );
      anchor = index;
    } else {
      setMulti([row.id]);
      anchor = index;
    }
    store.select(row.id);
  };

  /** The multi-selection as rows, in the order the list shows them, each
   * once; a selected row folded away under a collapsed group comes after,
   * in list order. What Copy \cite writes. */
  const selectedRows = (): PaperRow[] => {
    const ids = new Set(multi());
    if (ids.size === 0) return [];
    const out: PaperRow[] = [];
    const seen = new Set<string>();
    const take = (row: PaperRow) => {
      if (ids.has(row.id) && !seen.has(row.id)) {
        seen.add(row.id);
        out.push(row);
      }
    };
    for (const item of items()) if (item.kind === 'row') take(item.row);
    for (const row of sourceRows()) take(row);
    return out;
  };
  const [citeCopied, setCiteCopied] = createSignal(false);
  let citeTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(citeTimer));
  const citeFailed = () =>
    store.reportError('Could not write to the clipboard; press Copy \\cite{} again');
  const copySelectionCite = async () => {
    if (!(await copyCite(selectedRows()))) {
      citeFailed();
      return;
    }
    setCiteCopied(true);
    clearTimeout(citeTimer);
    citeTimer = setTimeout(() => setCiteCopied(false), 1_200);
  };
  // The Library command reads the selection from here while the list shows:
  // read on demand at the key press, never tracked, which is the point.
  onCleanup(
    // eslint-disable-next-line solid/reactivity -- read on demand, not tracked
    registerCiteSelection(() => (view() === 'list' ? selectedRows() : []), citeFailed)
  );

  // The palette's Merge Selected Items opens the same inline confirmation
  // the Merge button does; it never merges by itself.
  onCleanup(
    // eslint-disable-next-line solid/reactivity -- read on demand, not tracked
    registerMergeRequest(() => {
      if (view() === 'list' && multi().length >= 2) setMerging(multi().slice());
    })
  );

  /** The rows a drag from this row carries: the multi-selection when the
   * row is in it, else the row alone. */
  const draggedIds = (row: PaperRow) => (multi().includes(row.id) ? multi() : [row.id]);

  /** Each membership is a save and a change event in every window, so a
   * row already in the project is not written again. */
  const dropInto = (ids: string[], projectId: string) => {
    const already = new Set(
      sourceRows()
        .filter((row) => row.projectIds.includes(projectId))
        .map((row) => row.id)
    );
    for (const id of ids) {
      if (!already.has(id)) void store.setMembership(id, projectId, true);
    }
    // Filing a new item is triaging it.
    if (triage()) void store.markReviewed(ids);
  };

  /** Under New: a digit files the item into that project, D or Backspace
   * marks it done. Either way it leaves the list. */
  const triageKey = (e: KeyboardEvent, row: PaperRow): boolean => {
    const digit = triageDigit(e);
    if (digit !== null) {
      const target = store.state.projects[digit];
      if (!target) return false;
      void store.fileNew(row.id, target.id);
      return true;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    if (e.key === 'd' || e.key === 'D' || e.key === 'Backspace') {
      void store.markReviewed([row.id]);
      return true;
    }
    return false;
  };

  /** The cite keys of dragged rows, in list order; a record that is not a
   * paper is not cited. */
  const citeKeysOf = (ids: string[]) => {
    const wanted = new Set(ids);
    const seen = new Set<string>();
    const keys: string[] = [];
    for (const row of sortedRows()) {
      if (!wanted.has(row.id) || seen.has(row.id) || !row.isPaper || !row.citeKey) continue;
      seen.add(row.id);
      keys.push(row.citeKey);
    }
    return keys;
  };

  type RowDrop =
    { kind: 'project'; id: string } | { kind: 'note'; note: NoteDropTarget; x: number; y: number };

  const onRowPointerDown = (row: PaperRow, e: PointerEvent) => {
    const ids = draggedIds(row);
    beginPointerDrag<RowDrop>(e, {
      label: ids.length === 1 ? row.title : `${ids.length} items`,
      targetAt: (x, y) => {
        const project = projectRowAt(x, y);
        if (project) return { kind: 'project', id: project.id };
        // Only the project's own notepad, showing beside the list.
        const note = shownProjectPane() ? noteDropAt(x, y) : null;
        return note && citeKeysOf(ids).length > 0 ? { kind: 'note', note, x, y } : null;
      },
      onHover: (target) =>
        setDropHover(target?.kind === 'project' ? { kind: 'into', id: target.id } : null),
      onDrop: (target) => {
        if (target.kind === 'project') dropInto(ids, target.id);
        else target.note.insertAt(target.x, target.y, citeText(citeKeysOf(ids)));
      },
    });
  };

  /** A click on empty space in the list lets the selection go, back to the
   * project's notepad; only while there is one to go back to. */
  const onListClick = (e: MouseEvent) => {
    if ((e.target as HTMLElement).closest('.paper-row, .paper-group-head, button, a')) return;
    if (!projectPane()) return;
    setMulti([]);
    anchor = null;
    backToProjectPane();
  };

  const moveTo = (next: number, extend: boolean) => {
    const all = items();
    const index = Math.max(0, Math.min(all.length - 1, next));
    setFocusIndex(index);
    const item = all[index];
    if (item?.kind === 'row') {
      if (extend && anchor !== null) setMulti(rowsBetween(anchor, index));
      else {
        setMulti([item.row.id]);
        anchor = index;
      }
      store.select(item.row.id);
    }
    focusItem(index);
  };

  const activate = (item: Item | undefined) => {
    if (!item) return;
    if (item.kind === 'header') store.toggleGroup(item.groupId);
    else if (item.kind === 'row') void openPaper(item.row.id);
    else searchFullText(store.state.query, project()?.id ?? null);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const all = items();
    const item = all[current()];
    if (triage() && item?.kind === 'row' && triageKey(e, item.row)) {
      e.preventDefault();
      return;
    }
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        moveTo(current() + 1, e.shiftKey);
        break;
      case 'ArrowUp':
        e.preventDefault();
        moveTo(current() - 1, e.shiftKey);
        break;
      case 'Home':
        e.preventDefault();
        moveTo(0, e.shiftKey);
        break;
      case 'End':
        e.preventDefault();
        moveTo(all.length - 1, e.shiftKey);
        break;
      case 'ArrowLeft':
        e.preventDefault();
        if (item?.kind === 'header' && !item.collapsed) store.toggleGroup(item.groupId);
        else props.onMoveFocus(-1);
        break;
      case 'ArrowRight':
        e.preventDefault();
        if (item?.kind === 'header' && item.collapsed) store.toggleGroup(item.groupId);
        else props.onMoveFocus(1);
        break;
      case 'Enter':
        e.preventDefault();
        activate(item);
        break;
      case 'Escape':
        if (store.state.person && !e.defaultPrevented) {
          e.preventDefault();
          store.leavePerson();
        }
        break;
      case ' ':
        e.preventDefault();
        if (item?.kind === 'header') store.toggleGroup(item.groupId);
        else if (item?.kind === 'row') clickRow(current(), item.row, e as unknown as MouseEvent);
        break;
    }
  };

  const emptyText = () => {
    if (store.state.loadingRows) return 'Loading...';
    if (hasQuery()) return 'No items match';
    if (libraryStore.state.library.roots.length === 0) {
      return 'No library folders yet. Add one on the Home screen, in the Library panel';
    }
    switch (store.state.scope.kind) {
      case 'project':
        return "This project is empty. Drag items here or tick it in an item's details";
      case 'unfiled':
        return 'Every item is in a project';
      case 'others':
        return 'Nothing here: every PDF in the library folders is treated as an item';
      default:
        return 'No items yet. The library scan fills this in';
    }
  };
  const offerAddFolder = () =>
    !store.state.loadingRows && !hasQuery() && libraryStore.state.library.roots.length === 0;

  // The outcome of a batch lookup, shown where the button was until the
  // next one or the next scope. Not in the store: it is this column's own
  // passing remark. Under a project the batch is that project's papers with
  // an identifier; under All papers it is the whole library, by identifier
  // or, failing that, by title. Other PDFs offers none: nothing there is a
  // paper, so there is nothing to look up.
  const [batch, setBatch] = createSignal<LookupBatch | null>(null);
  const [lookingUp, setLookingUp] = createSignal(false);
  const lookupScope = () => {
    const scope = store.state.scope;
    return scope.kind === 'project' || scope.kind === 'all' ? scope : null;
  };
  const lookUpAll = async () => {
    const target = lookupScope();
    if (!target) return;
    setLookingUp(true);
    setBatch(null);
    const result =
      target.kind === 'project' ? await store.lookupProject(target.id) : await store.lookupAll();
    setLookingUp(false);
    if (result) setBatch(result);
  };
  /** "3 parked" are title searches that found something but nothing sure
   * enough to apply; the matches wait on each paper for the user to pick. */
  const batchSummary = (b: LookupBatch) =>
    b.attempted === 0
      ? 'Nothing to look up'
      : `${b.applied} of ${b.attempted} updated` +
        (b.parked > 0 ? `, ${b.parked} parked` : '') +
        (b.failed > 0 ? `, ${b.failed} failed` : '') +
        (b.skipped > 0 ? `, ${b.skipped} skipped` : '');

  return (
    <>
      <div class="paper-list-head">
        <Show when={project() || (lookupScope() && settings.onlineLookup)}>
          <div class="paper-list-project">
            <Show when={project()}>
              {(p) => (
                <>
                  <Show
                    when={p().bibPath}
                    fallback={<span class="paper-list-bib is-none">no .bib</span>}
                  >
                    {(path) => (
                      <span class="paper-list-bib" title={path()}>
                        {baseName(path())}
                      </span>
                    )}
                  </Show>
                  <Show when={p().incompleteCount > 0}>
                    <span class="paper-list-incomplete sb-warn">
                      {p().incompleteCount} incomplete
                    </span>
                  </Show>
                </>
              )}
            </Show>
            <span class="paper-list-spacer" />
            <Show when={settings.onlineLookup && lookupScope()}>
              {(target) => (
                <>
                  <Show when={batch()}>
                    {(b) => <span class="paper-list-batch">{batchSummary(b())}</span>}
                  </Show>
                  <button
                    type="button"
                    class="secondary-btn"
                    disabled={lookingUp()}
                    title={
                      target().kind === 'project'
                        ? 'Fetch metadata for every item here that has a DOI or arXiv id'
                        : 'Fetch metadata for every item: by DOI or arXiv id, else by title'
                    }
                    onClick={() => void lookUpAll()}
                  >
                    {lookingUp() ? 'Looking up...' : 'Look up all'}
                  </button>
                </>
              )}
            </Show>
            <Show when={project()}>
              {(p) => (
                <>
                  <button type="button" class="secondary-btn" onClick={() => void exportBib(p())}>
                    Export .bib...
                  </button>
                  <button type="button" class="secondary-btn" onClick={() => void syncBib(p())}>
                    Sync .bib...
                  </button>
                </>
              )}
            </Show>
          </div>
        </Show>
        <div class="paper-list-view">
          <ViewToggle />
          {/* The search filters rows; the graph is the whole scope, so under
              it the field would promise something it does not do. */}
          <Show when={view() === 'list' && !health()}>
            <div class="library-search-field paper-search">
              <IconSearch />
              <input
                type="text"
                placeholder="Search title, authors, year, key..."
                aria-label="Search items"
                value={store.state.query}
                onInput={(e) => store.setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape' && hasQuery()) {
                    e.preventDefault();
                    e.stopPropagation();
                    store.setQuery('');
                  } else if (e.key === 'ArrowDown' || e.key === 'Enter') {
                    e.preventDefault();
                    focusItem(current());
                  }
                }}
              />
              <Show when={hasQuery()}>
                <button
                  type="button"
                  class="library-search-clear"
                  aria-label="Clear the search"
                  onClick={() => store.setQuery('')}
                >
                  <IconClose />
                </button>
              </Show>
            </div>
            <Show when={sortable()}>
              <button
                type="button"
                class="paper-sort-btn"
                title="Sort by: Default, Year, Title, or how many items here cite each"
                onClick={() => store.setSort(nextSort(store.state.sort))}
              >
                Sort: {SORT_LABEL[store.state.sort]}
              </button>
            </Show>
            <Show when={triage() && store.state.rows.length > 1 && !hasQuery()}>
              <button
                type="button"
                class="paper-sort-btn"
                title="Mark every new item done without filing it"
                onClick={() => void store.markReviewed(store.state.rows.map((r) => r.id))}
              >
                All done
              </button>
            </Show>
            <Show when={multi().length >= 1 && !merging()}>
              <button
                type="button"
                class="secondary-btn paper-cite-btn"
                title={"Copy \\cite{...} with the selected items' keys, in list order"}
                onClick={() => void copySelectionCite()}
              >
                {citeCopied() ? 'Copied' : 'Copy \\cite{}'}
              </button>
            </Show>
            <Show when={multi().length >= 2 && !merging()}>
              <button
                type="button"
                class="secondary-btn paper-merge-btn"
                title="Fold these records into one: the first selected is kept"
                onClick={() => setMerging(multi().slice())}
              >
                Merge {multi().length} items
              </button>
            </Show>
          </Show>
        </div>
        <Show when={view() === 'list' && merging()}>
          {(ids) => (
            <MergeConfirm
              ids={ids()}
              onCancel={() => setMerging(null)}
              onDone={() => {
                setMerging(null);
                setMulti([]);
                anchor = null;
              }}
            />
          )}
        </Show>
      </div>
      <Show when={view() === 'graph' && !store.state.missingList}>
        <GraphView onMoveFocus={props.onMoveFocus} />
      </Show>
      <Show when={view() === 'graph' && store.state.missingList}>
        <MissingList />
      </Show>
      <Show when={health()}>
        <HealthList />
      </Show>
      <Show when={view() === 'list' && !health()}>
        <PersonBar />
        <div
          ref={list}
          class="projects-col-scroll paper-list"
          role={grouped() ? 'tree' : 'listbox'}
          aria-label="Items"
          onKeyDown={onKeyDown}
          onClick={onListClick}
        >
          <Show
            when={sourceRows().length > 0 && rowItems().length > 0}
            fallback={
              <div class="paper-empty">
                <p class="library-hint">{emptyText()}</p>
                <Show when={offerAddFolder()}>
                  <button
                    type="button"
                    class="secondary-btn"
                    onClick={() => void libraryStore.addLibraryFolder()}
                  >
                    Add folder...
                  </button>
                </Show>
              </div>
            }
          >
            {/* Index, not For: the items are value objects rebuilt on every
              refresh and every fold, and For would rebuild every row's DOM
              with them. Index keeps the element at each position and
              updates what it shows. */}
            <Index each={rowItems()}>
              {(item, index) => {
                const header = () => {
                  const it = item();
                  return it.kind === 'header' ? it : null;
                };
                const entry = () => {
                  const it = item();
                  return it.kind === 'row' ? it : null;
                };
                return (
                  <Switch>
                    <Match when={header()}>
                      {(match) => {
                        // Read the last header this row held, not the
                        // Match's accessor: when the row turns into an item
                        // its old contents can update once more before they
                        // go, and the accessor throws then.
                        const head = createMemo<Extract<Item, { kind: 'header' }>>(
                          (last) => header() ?? last,
                          match()
                        );
                        return (
                          <div
                            class="paper-group-head"
                            role="treeitem"
                            aria-level={1}
                            aria-expanded={!head().collapsed}
                            tabindex={current() === index ? 0 : -1}
                            data-index={index}
                            onClick={() => {
                              setFocusIndex(index);
                              store.toggleGroup(head().groupId);
                            }}
                          >
                            <Show when={head().collapsed} fallback={<IconChevronDown />}>
                              <IconChevronRight />
                            </Show>
                            <span class="paper-group-name">{head().name}</span>
                            <span class="paper-group-count">{head().count}</span>
                          </div>
                        );
                      }}
                    </Match>
                    <Match when={entry()}>
                      {(match) => {
                        // As for the header: the last item this row held.
                        const found = createMemo<Extract<Item, { kind: 'row' }>>(
                          (last) => entry() ?? last,
                          match()
                        );
                        const row = () => found().row;
                        const selected = () => store.state.selectedId === row().id;
                        return (
                          <div
                            class="paper-row"
                            classList={{
                              'is-selected': selected(),
                              'is-multi': multi().includes(row().id),
                              'is-missing': row().missing,
                              'is-other': !row().isPaper,
                            }}
                            role={grouped() ? 'treeitem' : 'option'}
                            aria-level={grouped() ? 2 : undefined}
                            aria-selected={selected()}
                            tabindex={current() === index ? 0 : -1}
                            data-index={index}
                            title={row().missing ? 'file missing' : undefined}
                            onClick={(e) => clickRow(index, row(), e)}
                            onDblClick={() => void openPaper(row().id)}
                            onPointerDown={(e) => onRowPointerDown(row(), e)}
                          >
                            <span class="paper-row-main">
                              <MathText class="paper-row-title" text={row().title} />
                              <Show when={rowMeta(row())}>
                                <span class="paper-row-meta">{rowMeta(row())}</span>
                              </Show>
                            </span>
                            <Show when={(row().fileCount ?? 0) > 1}>
                              <span
                                class="paper-row-files"
                                title="One item, several PDFs; see the details"
                              >
                                {row().fileCount} PDFs
                              </span>
                            </Show>
                            <Show when={row().candidateCount > 0}>
                              <span
                                class="paper-row-matches"
                                title="A title search found these; pick one in the details"
                              >
                                {row().candidateCount}{' '}
                                {row().candidateCount === 1 ? 'match' : 'matches'}
                              </span>
                            </Show>
                            <Show when={store.state.duplicates[row().id]}>
                              <span
                                class="paper-row-dup"
                                title="Possibly the same work as another item; see the details"
                              >
                                duplicate?
                              </span>
                            </Show>
                            <Show when={triage()}>
                              <button
                                type="button"
                                class="health-action"
                                tabindex={-1}
                                title="Mark done without filing (D)"
                                onPointerDown={(e) => e.stopPropagation()}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void store.markReviewed([row().id]);
                                }}
                              >
                                Done
                              </button>
                            </Show>
                            <Show when={alsoText(found())}>
                              {(text) => <span class="paper-row-also">{text()}</span>}
                            </Show>
                            {/* A record that is not a paper has a key the way a
                              form has a page number: there, and not what you
                              came for. The file name is already its title. */}
                            <Show when={row().isPaper}>
                              <span class="paper-row-key">{row().citeKey}</span>
                            </Show>
                          </div>
                        );
                      }}
                    </Match>
                  </Switch>
                );
              }}
            </Index>
          </Show>
          <Show when={hasQuery()}>
            <button
              type="button"
              class="paper-fulltext"
              tabindex={current() === rowItems().length ? 0 : -1}
              data-index={rowItems().length}
              onClick={() => searchFullText(store.state.query, project()?.id ?? null)}
            >
              <IconSearch />
              <span>
                Search the full text of {project()?.name ?? 'every item'} for "{store.state.query}"
              </span>
            </button>
          </Show>
        </div>
      </Show>
    </>
  );
}

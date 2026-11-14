/** Tab lifecycle: open-in-new-tab-or-focus, close, activate, reorder, and
 * hand a tab to another window. Distinct from controller.ts, which acts on
 * one already-existing tab's content (open/save/guard) — this module is the
 * only place that creates or removes TabRecords.
 *
 * A tab registry is per-window (one module instance per webview), so anything
 * that crosses windows goes through the registry in Rust: see
 * lib/transport/windows.ts and src-tauri/src/windows.rs. */
import { flushNotes } from '../extensions/editorHost';
import { invoke } from '@tauri-apps/api/core';
import { getAllWindows } from '@tauri-apps/api/window';
import { type DocumentSnapshot } from './documentStore';
import { type ViewportState, VIEW_PADDING } from '../../stores/viewportStore';
import { windowTransferPending, setWindowTransferPending } from '../windows/windowController';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { engine, isEngineError } from '../../lib/transport/engine';
import {
  activeTab,
  addTab,
  createTab,
  findTabByPath,
  removeTab,
  reorderTabs,
  setActiveId,
  setTabOpening,
  tabsStore,
  type TabRecord,
} from '../../stores/tabsStore';
import { askPassword, showError } from '../../stores/modalStore';
import { clearImagePreviews } from '../editor/editorActions';
import { clearFormFields } from '../forms/formStore';
import { guardUnsaved, hydrateAnnotations, hydrateSizes } from './controller';
import { recentStore } from '../../stores/recentStore';
import {
  removeWindowSession,
  saveSession,
  takeWindowSessions,
  type SessionFile,
} from '../../stores/sessionStore';
import { MAIN_LABEL, windowApi, windowLabel } from '../../lib/transport/windows';
import { abandonPrintFor } from '../print/printStore';
import { captureView, forgetTab, noteTabSwitch, setNavTabActivator } from '../viewer/navHistory';
import { popClosedTab, pushClosedTab, type ClosedTab } from './closedTabs';

const MAX_TABS = 30;

function setActiveDocument(docId: number | null) {
  void engine.setActiveDocument(docId);
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

/** Debounced session write — snapshots which files are open, in what order,
 * and which is active. Never stores docId/zoom/scroll (see sessionStore.ts).
 *
 * The window registry is told at once rather than debounced: it is what the
 * *other* windows read to answer "does anyone already have this file open",
 * and a stale answer there opens a second copy of a document. */
function scheduleSave(): void {
  publishTabs();
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveCurrentSession, 250);
}

function publishTabs(): void {
  const tabs = tabsStore.state.tabs
    .map((t) => t.documentStore.state.path)
    .filter((path): path is string => typeof path === 'string');
  const activePath = activeTab()?.documentStore.state.path ?? null;
  void windowApi.publishTabs(tabs, activePath).catch(() => undefined);
}

/** Whether another window already has `path` open — in which case it has
 * been raised with that tab focused. A registry that cannot be reached is
 * never a reason to refuse to open a document. */
async function openElsewhere(path: string): Promise<boolean> {
  try {
    return (await windowApi.focusWindowForPath(path)) === true;
  } catch {
    return false;
  }
}

export function saveCurrentSession(): void {
  clearTimeout(saveTimer);
  const tabs = tabsStore.state.tabs
    .filter((t) => t.documentStore.state.path)
    .map((t) => ({ path: t.documentStore.state.path! }));
  const activePath = activeTab()?.documentStore.state.path ?? null;
  saveSession({ version: 1, tabs, activePath }, windowLabel());
  publishTabs();
}

export async function openFromDialog(): Promise<void> {
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: 'PDF documents', extensions: ['pdf'] }],
  });
  if (typeof picked === 'string') await openInNewTabOrFocus(picked);
}

/** Opens `path` in a new tab, or focuses its tab if already open. Every
 * entry point (dialog, drag-drop, recent-list click, session restore) shares
 * this so dedup-to-existing-tab is a single implementation. Returns whether
 * the open succeeded (a real file that failed to load returns false; the
 * caller can use that to prune a stale recent-files entry). */
export async function openInNewTabOrFocus(
  path: string,
  opts?: { skipGuard?: boolean; fromSessionRestore?: boolean }
): Promise<boolean> {
  const outcome = await openPath(path, opts);
  return outcome === 'opened' || outcome === 'focused';
}

/** How an open ended: a new tab, an existing one (here or in another
 * window) brought forward, a failure, a password prompt the user walked away
 * from, or no room for another tab. */
type OpenOutcome = 'opened' | 'focused' | 'failed' | 'cancelled' | 'full';

async function openPath(
  path: string,
  opts?: {
    skipGuard?: boolean;
    fromSessionRestore?: boolean;
    /** Reopening a closed tab: put the view back as it was, and fail quietly
     * so the caller can move on to the next one. */
    reopen?: ClosedTab;
  }
): Promise<OpenOutcome> {
  // Opening (or focusing) another document from a reading tab is a jump like
  // any other: Back in the tab it lands in returns here, to this exact view.
  const source = activeTab();
  const from = source && !opts?.fromSessionRestore ? captureView(source) : null;
  const existing = findTabByPath(path);
  if (existing) {
    activateTab(existing.id);
    noteTabSwitch(from, existing.id);
    return 'focused';
  }
  // Nobody opens a second copy of a document another window already has: that
  // is two PDFium handles, two independent dirty states, and whichever window
  // saves last silently overwriting the other's work. The window that has it
  // raises itself and focuses the tab instead. Not at restore: the restored
  // windows' sessions are already disjoint (see takeWindowSessions), and a
  // window coming up must not raise another over itself.
  if (!opts?.fromSessionRestore && (await openElsewhere(path))) return 'focused';
  if (tabsStore.state.tabs.length >= MAX_TABS) {
    showError(`Too many tabs open (limit ${MAX_TABS}). Close one before opening another.`);
    return 'full';
  }

  const record = createTab();
  record.opening = true;
  addTab(record);
  setActiveId(record.id);
  setActiveDocument(null); // will be set once the open resolves

  /** Every abandoned-open path must go through here: a TabRecord owns store
   * subscriptions (citation ↔ library) that outlive the DOM, so dropping it
   * from the registry alone leaks a listener per failed open. */
  const abandon = () => {
    record.citationStore.dispose();
    removeTab(record.id);
    if (tabsStore.state.activeId === record.id) setActiveId(null);
  };

  let password: string | undefined;
  for (;;) {
    try {
      const meta = await engine.open(path, password);
      clearImagePreviews();
      // The fields belong to whatever file this document id last named; a
      // reopened or replaced document must not inherit the old ones.
      clearFormFields(meta.docId);
      setTabOpening(record.id, false);
      record.documentStore.initFromMeta(meta);
      record.searchStore.resetForDocument(meta.docId, meta.pageCount);
      record.citationStore.syncDocument(meta.docId);
      record.structureStore.syncDocument(meta.docId);
      record.viewport.setState({ currentPage: 0, scrollTop: 0 });
      // Pick up where this document was last left, when we have a position for
      // it and it still points inside the document.
      const reopen = opts?.reopen;
      const resume = reopen ?? recentStore.positionFor(path);
      if (reopen) {
        // Column zoom is a mode over a settled layout; it comes back as its
        // zoom, the way Back treats it.
        record.viewport.setState({
          zoom: reopen.zoom,
          fitMode: reopen.fitMode === 'column' ? 'custom' : reopen.fitMode,
          viewRotation: reopen.viewRotation,
        });
      }
      if (resume && resume.page < meta.pageCount) {
        record.viewport.setState('currentPage', resume.page);
        record.viewport.requestRestorePosition(resume.page, resume.fraction);
      } else {
        record.viewport.requestScrollToPage(0);
      }
      void engine.startIndexing(meta.docId);
      record.ready = Promise.all([
        hydrateSizes(record, meta.pageCount),
        hydrateAnnotations(record, meta.docId),
      ]).then(() => undefined);
      if (activeTab()?.id === record.id) setActiveDocument(meta.docId);
      recentStore.recordOpen({ path, name: meta.name });
      noteTabSwitch(from, record.id);
      scheduleSave();
      return 'opened';
    } catch (e) {
      if (isEngineError(e) && e.code === 'password') {
        if (opts?.fromSessionRestore) {
          // Prompting for N passwords at launch would conflict with
          // modalStore's single in-flight prompt — just drop this tab.
          abandon();
          return 'failed';
        }
        const entered = await askPassword(
          password === undefined
            ? 'This PDF is password-protected. Enter the password to open it.'
            : 'Incorrect password. Try again.'
        );
        if (entered === null) {
          abandon();
          return 'cancelled';
        }
        password = entered;
        continue;
      }
      // A path that no longer exists during session restore is expected
      // (moved/deleted since last launch) — drop it silently, no modal spam
      // on every future relaunch.
      // Nor when reopening a closed tab: a file gone since is skipped for the
      // next one.
      if (!opts?.fromSessionRestore && !opts?.reopen) {
        showError(isEngineError(e) ? e.message : `Could not open ${path}: ${String(e)}`);
      }
      abandon();
      return 'failed';
    }
  }
}

export function activateTab(id: string): void {
  setActiveId(id);
  const tab = activeTab();
  setActiveDocument(tab?.documentStore.state.docId ?? null);
  scheduleSave();
}
// Back/Forward across tabs brings a tab forward the same way a click does.
setNavTabActivator(activateTab);

/** Closes a tab, prompting first if it has unsaved changes. Returns whether
 * it actually closed, so a caller closing several in a row can stop when the
 * user cancels. */
export async function closeTab(id: string): Promise<boolean> {
  const tab = tabsStore.state.tabs.find((t) => t.id === id);
  if (!tab || tab.opening || windowTransferPending()) return false;
  if (tab.documentStore.state.dirty) {
    const ok = await guardUnsaved(tab);
    if (!ok) return false;
  }
  rememberClosed(tab);
  if (tab.documentStore.state.loaded) {
    // A print dialog open on this document is now previewing something that
    // is about to be closed underneath it.
    abandonPrintFor(tab.documentStore.state.docId);
    void engine.close(tab.documentStore.state.docId);
  }
  tab.citationStore.dispose();
  removeTab(id);
  forgetTab(id);

  if (tabsStore.state.activeId === id) {
    const remaining = tabsStore.state.tabs;
    const neighbor = remaining[0] ?? null;
    if (neighbor) activateTab(neighbor.id);
    else {
      setActiveId(null);
      setActiveDocument(null);
    }
  }
  scheduleSave();
  return true;
}

/** Puts a closing tab on this window's Reopen Closed Tab stack, with where
 * its reader was. Only documents with a file behind them can come back. */
function rememberClosed(tab: TabRecord): void {
  const doc = tab.documentStore.state;
  if (!doc.loaded || !doc.path) return;
  const vp = tab.viewport.state;
  const view = captureView(tab);
  pushClosedTab({
    path: doc.path,
    page: view?.page ?? vp.currentPage,
    fraction: view ? Math.min(1, Math.max(0, view.fraction)) : 0,
    zoom: vp.zoom,
    fitMode: vp.fitMode,
    viewRotation: vp.viewRotation,
  });
}

/** Reopens the most recently closed tab where it was left. A file that can
 * no longer be opened is skipped for the one closed before it; a file open
 * already (here, or in another window, which raises itself) is focused and
 * its entry spent. */
export async function reopenClosedTab(): Promise<boolean> {
  for (let entry = popClosedTab(); entry; entry = popClosedTab()) {
    const outcome = await openPath(entry.path, { reopen: entry });
    if (outcome === 'failed') continue;
    // No room: keep it for when there is.
    if (outcome === 'full') pushClosedTab(entry);
    return outcome !== 'full';
  }
  return false;
}

/** Closes every tab but one. Sequential, because each may prompt and
 * modalStore only holds one prompt at a time; a cancelled prompt stops the
 * run rather than carrying on through the rest. */
export async function closeOtherTabs(id: string): Promise<void> {
  for (const other of tabsStore.state.tabs.filter((t) => t.id !== id)) {
    if (!(await closeTab(other.id))) return;
  }
}

export async function closeTabsToTheRight(id: string): Promise<void> {
  const index = tabsStore.state.tabs.findIndex((t) => t.id === id);
  if (index < 0) return;
  for (const doomed of tabsStore.state.tabs.slice(index + 1)) {
    if (!(await closeTab(doomed.id))) return;
  }
}

export function goHome(): void {
  setActiveId(null);
  setActiveDocument(null);
  scheduleSave();
}

/** Cycles the active tab forward/backward. No-op with 0 or 1 tabs. */
export function cycleTab(direction: 1 | -1): void {
  const tabs = tabsStore.state.tabs;
  if (tabs.length < 2) return;
  const currentIndex = tabs.findIndex((t) => t.id === tabsStore.state.activeId);
  const nextIndex = ((currentIndex < 0 ? 0 : currentIndex) + direction + tabs.length) % tabs.length;
  activateTab(tabs[nextIndex]!.id);
}

/** Jumps to the Nth tab (0-indexed). No-op if out of range. */
export function jumpToTab(index: number): void {
  const tab = tabsStore.state.tabs[index];
  if (tab) activateTab(tab.id);
}

export function reorderTab(fromIndex: number, toIndex: number): void {
  reorderTabs(fromIndex, toIndex);
  scheduleSave();
}

/** Used by the window-close guard. Prompts for each dirty tab in turn
 * (sequential — modalStore only supports one prompt in flight at a time).
 * Returns false if any prompt is cancelled. */
export async function closeAllWithGuard(): Promise<boolean> {
  for (const tab of tabsStore.state.tabs) {
    if (tab.documentStore.state.dirty) {
      const ok = await guardUnsaved(tab);
      if (!ok) return false;
    }
  }
  return true;
}

/** Reopens the windows that were open at last quit: this (main) window takes
 * its own tabs, and every other window comes back as a window of its own
 * with its own tabs. Called once, from initializeWindow. */
export async function restoreSession(): Promise<void> {
  const { main, others } = takeWindowSessions();
  for (const other of others) {
    try {
      await windowApi.restoreWindow({
        tabs: other.tabs.map((t) => t.path),
        activePath: other.activePath,
      });
    } catch {
      // A window that cannot be built costs the user those tabs, nothing more.
    }
  }
  await restoreTabs(main);
}

/** Reopens `session`'s files, in the same order, with the same tab active.
 * Missing files are dropped silently (see the fromSessionRestore branch
 * above); the pruned list is then re-saved so future launches don't keep
 * retrying dead paths. */
async function restoreTabs(session: SessionFile): Promise<void> {
  if (session.tabs.length === 0) return;

  // Sequential, not Promise.all: every open costs a PDFium parse plus a page-
  // size hydration sweep on the single engine thread, so firing ten at once
  // just queues them behind each other while starving the first tab's pages
  // of the renders it needs to actually show something.
  for (const t of session.tabs) {
    await openInNewTabOrFocus(t.path, { skipGuard: true, fromSessionRestore: true });
  }

  const restored = tabsStore.state.tabs;
  if (restored.length === 0) return;
  const activeMatch = restored.find((t) => t.documentStore.state.path === session.activePath);
  activateTab((activeMatch ?? restored[0]!).id);
  scheduleSave();
}

export type { TabRecord };

export interface TabSnapshot {
  document: DocumentSnapshot;
  viewport: Pick<
    ViewportState,
    | 'zoom'
    | 'fitMode'
    | 'viewRotation'
    | 'currentPage'
    | 'editMode'
    | 'searchOpen'
    | 'formPanelOpen'
  >;
  fraction: number;
  search: { query: string; caseSensitive: boolean };
}

/** Where a tab is handed: a window built for it (optionally at the point a
 * tab was dragged out to), or a window that already exists. */
type HandOffTarget =
  { kind: 'new'; position: [number, number] | undefined } | { kind: 'existing'; label: string };

/** The source is frozen until the target acknowledges adoption. The native
 * engine handle moves with the tab, so encrypted files need no second password. */
export function detachTab(id: string, position?: [number, number]): Promise<void> {
  return handOffTab(id, { kind: 'new', position });
}

/** Hands a tab to another, existing window — the tab menu's "Move to …". The
 * same transaction as detaching: unsaved edits and undo history travel with
 * the tab, so nothing has to be saved before it goes. */
export function moveTabToWindow(id: string, label: string): Promise<void> {
  return handOffTab(id, { kind: 'existing', label });
}

async function handOffTab(id: string, target: HandOffTarget): Promise<void> {
  const tab = tabsStore.state.tabs.find((t) => t.id === id);
  if (
    !tab ||
    tab.opening ||
    !tab.documentStore.state.loaded ||
    tab.documentStore.state.saving ||
    windowTransferPending()
  )
    return;
  setWindowTransferPending(true);
  try {
    await tab.ready;
    const vp = tab.viewport.state;
    const layout = tab.zoom.layoutFor(vp.zoom);
    const fraction = Math.min(
      1,
      Math.max(
        0,
        (vp.scrollTop - (layout.tops[vp.currentPage] ?? 0) + VIEW_PADDING) /
          (layout.heights[vp.currentPage] || 1)
      )
    );
    const snapshot: TabSnapshot = {
      document: tab.documentStore.snapshot(),
      viewport: {
        zoom: vp.zoom,
        fitMode: vp.fitMode,
        viewRotation: vp.viewRotation,
        currentPage: vp.currentPage,
        editMode: vp.editMode,
        searchOpen: vp.searchOpen,
        formPanelOpen: vp.formPanelOpen,
      },
      fraction,
      search: {
        query: tab.searchStore.state.query,
        caseSensitive: tab.searchStore.state.caseSensitive,
      },
    };
    const docId = snapshot.document.state.docId;
    if (target.kind === 'new') {
      await invoke('detach_tab', { docId, snapshot, position: target.position ?? null });
    } else {
      await invoke('send_tab_to_window', { docId, snapshot, target: target.label });
    }
    abandonPrintFor(docId);
    tab.citationStore.dispose();
    removeTab(id);
    if (tabsStore.state.activeId === id) {
      const next = tabsStore.state.tabs[0];
      if (next) activateTab(next.id);
      else goHome();
    }
    saveCurrentSession();
  } catch (error) {
    showError(`Could not move the tab: ${String(error)}`);
  } finally {
    setWindowTransferPending(false);
  }
}

/** Builds a tab from another window's snapshot and commits native ownership.
 * Nothing is visible or editable until the registry has moved the document
 * here; on failure the half-built tab is discarded and the source keeps it. */
async function adoptSnapshot(snapshot: TabSnapshot): Promise<void> {
  const tab = createTab();
  tab.documentStore.restore(snapshot.document);
  tab.viewport.setState(snapshot.viewport);
  tab.viewport.requestRestorePosition(snapshot.viewport.currentPage, snapshot.fraction);
  const docId = snapshot.document.state.docId;
  tab.searchStore.resetForDocument(docId, snapshot.document.state.searchIndex.total);
  tab.searchStore.setCaseSensitive(snapshot.search.caseSensitive);
  tab.searchStore.setQuery(snapshot.search.query);
  tab.citationStore.syncDocument(docId);
  tab.structureStore.syncDocument(docId);
  try {
    await invoke('accept_window_tab');
  } catch (error) {
    tab.citationStore.dispose();
    throw error;
  }
  addTab(tab);
  activateTab(tab.id);
  void engine.startIndexing(docId);
  saveCurrentSession();
}

/** Another window offered this one a tab (its "Move to …" menu). Declining
 * wakes the sender at once, which keeps the tab and says so. */
export async function acceptOfferedTab(): Promise<void> {
  const decline = () => invoke('decline_window_tab').catch(() => undefined);
  if (tabsStore.state.tabs.length >= MAX_TABS) {
    await decline();
    return;
  }
  try {
    const snapshot = await invoke<TabSnapshot | null>('window_tab_snapshot');
    if (snapshot) await adoptSnapshot(snapshot);
  } catch {
    await decline();
  }
}

export async function initializeWindow(): Promise<void> {
  const snapshot = await invoke<TabSnapshot | null>('window_tab_snapshot');
  if (snapshot) {
    await adoptSnapshot(snapshot);
    return;
  }
  // A window reopened from the previous launch's session.
  const restore = await windowApi.takeRestore();
  if (restore) {
    await restoreTabs({
      version: 1,
      tabs: restore.tabs.map((path) => ({ path })),
      activePath: restore.activePath,
    });
  } else if (
    windowLabel() === MAIN_LABEL &&
    !new URLSearchParams(window.location.search).has('window')
  ) {
    await restoreSession();
  }
}

/** Flush before destroying a window. Only the last window keeps a launch
 * session; closing one of several must not resurrect its tabs next launch. */
export async function prepareWindowClose(): Promise<void> {
  saveCurrentSession();
  // Notes typed in the last half second are saved before the window goes.
  await flushNotes();
  if ((await getAllWindows()).length > 1) removeWindowSession(windowLabel());
}

/** Tab lifecycle: open-in-new-tab-or-focus, close, activate, reorder.
 * Distinct from controller.ts, which acts on one already-existing tab's
 * content (open/save/guard) — this module is the only place that creates or
 * removes TabRecords. */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow, getAllWindows } from '@tauri-apps/api/window';
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
import { loadSession, saveSession, removeWindowSession } from '../../stores/sessionStore';
import { abandonPrintFor } from '../print/printStore';

const MAX_TABS = 30;

function setActiveDocument(docId: number | null) {
  void engine.setActiveDocument(docId);
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

/** Debounced session write — snapshots which files are open, in what order,
 * and which is active. Never stores docId/zoom/scroll (see sessionStore.ts). */
function scheduleSave(): void {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(saveCurrentSession, 250);
}

export function saveCurrentSession(): void {
  clearTimeout(saveTimer);
  const tabs = tabsStore.state.tabs
    .filter((t) => t.documentStore.state.path)
    .map((t) => ({ path: t.documentStore.state.path! }));
  const activePath = activeTab()?.documentStore.state.path ?? null;
  saveSession({ version: 1, tabs, activePath }, windowLabel());
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
  const existing = findTabByPath(path);
  if (existing) {
    activateTab(existing.id);
    return true;
  }
  if (tabsStore.state.tabs.length >= MAX_TABS) {
    showError(`Too many tabs open (limit ${MAX_TABS}). Close one before opening another.`);
    return false;
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
      const resume = recentStore.positionFor(path);
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
      scheduleSave();
      return true;
    } catch (e) {
      if (isEngineError(e) && e.code === 'password') {
        if (opts?.fromSessionRestore) {
          // Prompting for N passwords at launch would conflict with
          // modalStore's single in-flight prompt — just drop this tab.
          abandon();
          return false;
        }
        const entered = await askPassword(
          password === undefined
            ? 'This PDF is password-protected. Enter the password to open it.'
            : 'Incorrect password. Try again.'
        );
        if (entered === null) {
          abandon();
          return false;
        }
        password = entered;
        continue;
      }
      // A path that no longer exists during session restore is expected
      // (moved/deleted since last launch) — drop it silently, no modal spam
      // on every future relaunch.
      if (!opts?.fromSessionRestore) {
        showError(isEngineError(e) ? e.message : `Could not open ${path}: ${String(e)}`);
      }
      abandon();
      return false;
    }
  }
}

export function activateTab(id: string): void {
  setActiveId(id);
  const tab = activeTab();
  setActiveDocument(tab?.documentStore.state.docId ?? null);
  scheduleSave();
}

export async function closeTab(id: string): Promise<void> {
  const tab = tabsStore.state.tabs.find((t) => t.id === id);
  if (!tab || tab.opening || windowTransferPending()) return;
  if (tab.documentStore.state.dirty) {
    const ok = await guardUnsaved(tab);
    if (!ok) return;
  }
  if (tab.documentStore.state.loaded) {
    // A print dialog open on this document is now previewing something that
    // is about to be closed underneath it.
    abandonPrintFor(tab.documentStore.state.docId);
    void engine.close(tab.documentStore.state.docId);
  }
  tab.citationStore.dispose();
  removeTab(id);

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

/** Reopens whatever files were open at last quit, in the same order, with
 * the same tab active. Called once from App.tsx on mount. Missing files are
 * dropped silently (see the fromSessionRestore branch above); the pruned
 * list is then re-saved so future launches don't keep retrying dead paths. */
export async function restoreSession(): Promise<void> {
  const session = loadSession();
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

function windowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return 'main';
  }
}

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

/** The source is frozen until the target acknowledges adoption. The native
 * engine handle moves with the tab, so encrypted files need no second password. */
export async function detachTab(id: string, position?: [number, number]): Promise<void> {
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
    await invoke('detach_tab', {
      docId: snapshot.document.state.docId,
      snapshot,
      position: position ?? null,
    });
    abandonPrintFor(snapshot.document.state.docId);
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

export async function initializeWindow(): Promise<void> {
  const snapshot = await invoke<TabSnapshot | null>('window_tab_snapshot');
  if (snapshot) {
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
    // Nothing is visible or editable until native ownership is committed.
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
  } else if (
    windowLabel() === 'main' &&
    !new URLSearchParams(window.location.search).has('window')
  ) {
    await restoreSession();
  }
}

/** Flush before destroying a window. Only the last window keeps a launch
 * session; closing one of several must not resurrect its tabs next launch. */
export async function prepareWindowClose(): Promise<void> {
  saveCurrentSession();
  if ((await getAllWindows()).length > 1) removeWindowSession(windowLabel());
}

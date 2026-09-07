/** Tab lifecycle: open-in-new-tab-or-focus, close, activate, reorder, and
 * hand a tab to another window. Distinct from controller.ts, which acts on
 * one already-existing tab's content (open/save/guard) — this module is the
 * only place that creates or removes TabRecords.
 *
 * A tab registry is per-window (one module instance per webview), so anything
 * that crosses windows goes through the registry in Rust: see
 * lib/transport/windows.ts. */
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
import { takeLegacySession } from '../../stores/sessionStore';
import {
  isMainWindow,
  windowApi,
  windowLabel,
  type WindowStartup,
} from '../../lib/transport/windows';
import { windowFocused } from '../../stores/windowStore';
import { abandonPrintFor } from '../print/printStore';

const MAX_TABS = 30;

/** What this window would have the engine treat as foreground, whether or not
 * it is currently allowed to say so. */
let desiredActiveDoc: number | null = null;

function setActiveDocument(docId: number | null) {
  desiredActiveDoc = docId;
  // Only the window in front speaks. `set_active_document` is app-global and
  // destructive — it bumps the citation epoch, closes the library hover
  // session, and hands over the single text indexer — so a background window
  // asserting its own tab would cancel the work of the window being read.
  if (windowFocused()) void engine.setActiveDocument(docId);
}

/** Re-asserts this window's document when it comes back to the front. */
export function claimActiveDocument(): void {
  void engine.setActiveDocument(desiredActiveDoc);
}

let publishTimer: ReturnType<typeof setTimeout> | undefined;

/** Tells the registry what this window has open: which files, in what order,
 * and which is active. Never docId (meaningless across restarts), zoom, or
 * scroll.
 *
 * Immediate rather than debounced, because this is not only the saved
 * session — it is what the *other* windows read to answer "does anyone
 * already have this file open". A stale entry there is a wrong answer, and
 * the wrong answer opens a second copy of a document.
 */
function publishTabs(): void {
  clearTimeout(publishTimer);
  const tabs = tabsStore.state.tabs
    .map((t) => t.documentStore.state.path)
    .filter((path): path is string => typeof path === 'string');
  const activePath = activeTab()?.documentStore.state.path ?? null;
  void windowApi.publishTabs(windowLabel(), tabs, activePath).catch(() => undefined);
}

/** The same, coalesced — for changes nothing is waiting on, like activating a
 * tab or dragging one along the strip. */
function schedulePublish(): void {
  clearTimeout(publishTimer);
  publishTimer = setTimeout(publishTabs, 250);
}

/** Which other window already has `path` open, if any. A registry that cannot
 * be reached is never a reason to refuse to open a document. */
async function windowWithPath(path: string): Promise<string | null> {
  try {
    return await windowApi.findWindowForPath(path, windowLabel());
  } catch {
    return null;
  }
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
  // Nobody opens a second copy of a document another window already has: that
  // is two PDFium handles, two independent dirty states, and whichever window
  // saves last silently overwriting the other's work. Hand it back to the
  // window that has it, which raises itself and focuses the tab.
  const elsewhere = await windowWithPath(path);
  if (elsewhere) {
    // Except at restore: a duplicate across two saved windows is stale
    // bookkeeping, and dropping it is the whole point.
    if (opts?.fromSessionRestore) return false;
    try {
      await windowApi.sendPaths(elsewhere, [path]);
      return true;
    } catch {
      // That window went away between the two calls — open it here after all.
    }
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
      void hydrateSizes(record, meta.pageCount);
      void hydrateAnnotations(record, meta.docId);
      if (activeTab()?.id === record.id) setActiveDocument(meta.docId);
      recentStore.recordOpen({ path, name: meta.name });
      publishTabs();
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
  schedulePublish();
}

/** Closes a tab, prompting first if it has unsaved changes. Returns whether
 * it actually closed, so a caller closing several in a row (close-others, or
 * moving a tab to another window) can stop when the user cancels. */
export async function closeTab(id: string, opts?: { skipGuard?: boolean }): Promise<boolean> {
  const tab = tabsStore.state.tabs.find((t) => t.id === id);
  if (!tab) return false;
  if (!opts?.skipGuard && tab.documentStore.state.dirty) {
    const ok = await guardUnsaved(tab);
    if (!ok) return false;
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
  publishTabs();
  return true;
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
  schedulePublish();
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
  schedulePublish();
}

// --- Windows ---------------------------------------------------------------

/** A new, empty window. */
export async function openNewWindow(): Promise<void> {
  try {
    await windowApi.open([]);
  } catch (e) {
    showError(isEngineError(e) ? e.message : `Could not open a window: ${String(e)}`);
  }
}

/** Hands a tab to another window — an existing one, or a new one when
 * `target` is null.
 *
 * Unsaved edits live in this window's document store and cannot travel with
 * the tab, so they are settled here before it goes. And the tab is closed
 * *before* the document is handed over: while both windows hold the same
 * path, the receiving window's own duplicate check would find this one and
 * bounce the document straight back.
 */
async function moveTab(id: string, target: string | null): Promise<void> {
  const tab = tabsStore.state.tabs.find((t) => t.id === id);
  const path = tab?.documentStore.state.path;
  if (!tab || !path) return;
  if (tab.documentStore.state.dirty && !(await guardUnsaved(tab))) return;
  if (!(await closeTab(id, { skipGuard: true }))) return;
  try {
    if (target === null) await windowApi.open([path]);
    else await windowApi.sendPaths(target, [path]);
  } catch (e) {
    // The document has left this window and arrived nowhere. Take it back
    // rather than leaving the user with neither.
    showError(isEngineError(e) ? e.message : `Could not move that tab: ${String(e)}`);
    await openInNewTabOrFocus(path);
  }
}

export function moveTabToNewWindow(id: string): Promise<void> {
  return moveTab(id, null);
}

export function moveTabToWindow(id: string, label: string): Promise<void> {
  return moveTab(id, label);
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

/** Everything this window is owed at mount, in order: the files it had open
 * at last quit, then anything handed to it since — a document the OS asked us
 * to open during a cold launch, or the tab another window moved over when it
 * created this one. Called once from App.tsx.
 *
 * Missing files are dropped silently (see the fromSessionRestore branch
 * above); the pruned list is then republished so future launches don't keep
 * retrying dead paths. */
export async function restoreSession(): Promise<void> {
  let startup: WindowStartup;
  try {
    startup = await windowApi.startup(windowLabel());
  } catch {
    return; // no registry, no session and nothing waiting
  }

  let paths = startup.restore.tabs;
  let activePath = startup.restore.activePath;
  // First launch after the session moved out of localStorage and into the
  // window registry: carry the old one over rather than greeting a user who
  // upgraded with an empty window.
  if (paths.length === 0 && isMainWindow()) {
    const legacy = takeLegacySession();
    if (legacy) {
      paths = legacy.tabs.map((t) => t.path);
      activePath = legacy.activePath;
    }
  }

  // Sequential, not Promise.all: every open costs a PDFium parse plus a page-
  // size hydration sweep on the single engine thread, so firing ten at once
  // just queues them behind each other while starving the first tab's pages
  // of the renders it needs to actually show something.
  for (const path of paths) {
    await openInNewTabOrFocus(path, { skipGuard: true, fromSessionRestore: true });
  }

  const restored = tabsStore.state.tabs;
  if (restored.length > 0) {
    const activeMatch = restored.find((t) => t.documentStore.state.path === activePath);
    activateTab((activeMatch ?? restored[0]!).id);
  }
  publishTabs();

  for (const path of startup.opens) await openInNewTabOrFocus(path);
}

export type { TabRecord };

/** Where extension editors appear: the reader's right-hand drawer
 * (`readerPanes`) and a project's pane on the Projects screen
 * (`projectViews`), shown in the detail column while no item is selected.
 *
 * The drawer is window-level: whether it is open is this window's, kept
 * while moving between papers (it shows whenever the paper on screen meets
 * the pane's `when`), and its width is everyone's, remembered. Neither ever
 * appears for a document the pane's `when` does not hold for: an unfiled
 * PDF has no drawer and no toggle. */
import { createSignal } from 'solid-js';
import { settings, updateSettings } from '../../stores/settings';
import { activeTab } from '../../stores/tabsStore';
import { goHome } from '../document/tabsController';
import { projectsStore } from '../projects/projectsStore';
import { requestDetails } from '../projects/screenIntents';
import { projectViews, readerPanes } from './extensionStore';
import { chooseProjectFor, noteOrigin, readerItem } from './readerContext';
import { revealInEditor } from './editorHost';

/* ---- the drawer ---- */

const WIDTH_KEY = 'speedyf-reader-pane-width';
export const DEFAULT_PANE_WIDTH = 380;
const MIN_PANE_WIDTH = 260;
/** The page keeps at least this share of the window. */
const MAX_PANE_FRACTION = 0.6;

export function clampPaneWidth(desired: number, windowWidth: number): number {
  if (!Number.isFinite(desired)) return DEFAULT_PANE_WIDTH;
  const ceiling = Math.max(MIN_PANE_WIDTH, Math.floor(windowWidth * MAX_PANE_FRACTION));
  return Math.round(Math.min(Math.max(desired, MIN_PANE_WIDTH), ceiling));
}

function storedWidth(): number {
  try {
    const raw = localStorage.getItem(WIDTH_KEY);
    if (raw === null) return DEFAULT_PANE_WIDTH;
    return clampPaneWidth(Number(raw), typeof window === 'undefined' ? 1280 : window.innerWidth);
  } catch {
    return DEFAULT_PANE_WIDTH;
  }
}

/** The open pane's qualified id, or null when the drawer is shut. */
const [openPane, setOpenPane] = createSignal<string | null>(null);
const [paneWidth, setPaneWidthSignal] = createSignal(storedWidth());
export { openPane, paneWidth };

export function setPaneWidth(desired: number): void {
  const width = clampPaneWidth(desired, typeof window === 'undefined' ? 1280 : window.innerWidth);
  setPaneWidthSignal(width);
  try {
    localStorage.setItem(WIDTH_KEY, String(width));
  } catch {
    /* session-only */
  }
}

/** The pane the drawer shows right now: the open one, when it is offered
 * for the document on screen. */
export function shownPane() {
  const id = openPane();
  if (!id) return null;
  return readerPanes().find((pane) => pane.id === id) ?? null;
}

/** Open or shut a pane. Shutting one that is not shown does nothing, and
 * opening one that is not offered here neither. */
export function toggleReaderPane(paneId?: string): boolean {
  const panes = readerPanes();
  const pane = paneId ? panes.find((p) => p.id === paneId) : panes[0];
  if (!pane) return false;
  if (shownPane()?.id === pane.id) {
    setOpenPane(null);
    return true;
  }
  setOpenPane(pane.id);
  return true;
}

/** Open a pane for the reader, working in `projectId` when given (and the
 * paper is in it). Returns whether the pane is now shown. */
export function openReaderPane(paneId: string, projectId: string | null): boolean {
  const pane = readerPanes().find((p) => p.id === paneId);
  if (!pane) return false;
  const item = readerItem();
  const path = activeTab()?.documentStore.state.path;
  if (projectId && item?.projectIds.includes(projectId)) {
    chooseProjectFor(item.id, projectId);
    if (path) noteOrigin(path, projectId);
  }
  setOpenPane(pane.id);
  return true;
}

export function closeReaderPane(): void {
  setOpenPane(null);
}

/** The drawer's switcher: work in another of the paper's projects. */
export function switchReaderProject(projectId: string): void {
  const item = readerItem();
  const path = activeTab()?.documentStore.state.path;
  if (!item || !item.projectIds.includes(projectId)) return;
  chooseProjectFor(item.id, projectId);
  if (path) noteOrigin(path, projectId);
}

/* ---- the Projects screen ---- */

/** The pane `openProjectView` last asked for, when several extensions
 * contribute one; otherwise the first offered. Window-level, like the
 * drawer. */
const [chosenProjectPane, setChosenProjectPane] = createSignal<string | null>(null);

/** Show an extension's pane of a project (a notepad) on the Projects
 * screen: `projectId`'s, or the selected project's when none is named. It
 * lives in the detail column while no item is selected, so the selection
 * is let go. At `offset` in its note when given. Returns false when there
 * is no project to show it for. */
export function openProjectView(
  viewId: string,
  projectId: string | null,
  offset?: number
): boolean {
  const scope = projectsStore.state.scope;
  const target = projectId ?? (scope.kind === 'project' ? scope.id : null);
  if (!target) return false;
  const view = projectViews().find((v) => v.id === viewId);
  if (!view) return false;
  if (scope.kind !== 'project' || scope.id !== target) {
    projectsStore.setScope({ kind: 'project', id: target });
  }
  projectsStore.leavePerson();
  projectsStore.leaveFocus();
  projectsStore.select(null);
  projectsStore.selectGhost(null);
  setChosenProjectPane(viewId);
  requestDetails();
  if (settings.launchScreen !== 'projects') updateSettings({ launchScreen: 'projects' });
  goHome();
  if (offset !== undefined) {
    revealInEditor(
      { extensionId: view.extensionId, projectId: target, file: view.editor.file },
      offset
    );
  }
  return true;
}

/** The extension pane the Projects detail column offers for the scope:
 * only for a project (not All items, Not filed, Other PDFs, Health or New,
 * nor a person's items or a graph focus), and only while its extension is
 * usable. Whether it shows depends on the selection too (ProjectPane.tsx). */
export function projectPane() {
  const scope = projectsStore.state.scope;
  if (scope.kind !== 'project' || projectsStore.state.person || projectsStore.state.focus) {
    return null;
  }
  const views = projectViews();
  const chosen = chosenProjectPane();
  const found = (chosen ? views.find((v) => v.id === chosen) : undefined) ?? views[0];
  return found ? { view: found, projectId: scope.id } : null;
}

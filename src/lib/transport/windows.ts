/**
 * Typed transport to the window registry in Rust (`src-tauri/src/windows.rs`)
 * for the questions that span windows, which a webview cannot answer about
 * itself: which other windows a tab can be moved to, and whether another
 * window already has a file open.
 */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** One window's open documents, in tab-strip order. */
export interface WindowTabs {
  tabs: string[];
  activePath: string | null;
}

/** Another window, as a "move tab to…" menu sees it. */
export interface WindowSummary {
  label: string;
  /** The document it is showing — the only way to tell two windows apart. */
  title: string;
  tabCount: number;
}

export const MAIN_LABEL = 'main';

/** This window's label. Falls back to the main window's outside a webview
 * (the unit tests), where asking Tauri throws. */
export function windowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return MAIN_LABEL;
  }
}

export const windowApi = {
  /** Publishes what this window has open. The registry is what the *other*
   * windows read, so it is kept current on every tab change. */
  publishTabs: (tabs: string[], activePath: string | null): Promise<void> =>
    invoke('update_window_tabs', { tabs, activePath }),
  /** The other windows, in creation order. */
  roster: (): Promise<WindowSummary[]> => invoke('window_roster'),
  /** When another window already has `path` open, it is raised and focuses
   * that tab, and this answers true. */
  focusWindowForPath: (path: string): Promise<boolean> => invoke('focus_window_for_path', { path }),
  /** Opens a window that reopens `restore` — one window of a restored session. */
  restoreWindow: (restore: WindowTabs): Promise<void> => invoke('new_window', { restore }),
  /** Opens `path` in a window of its own; a window already showing it is
   * raised instead. Answers whether a new window was made. */
  openInNewWindow: (path: string): Promise<boolean> => invoke('open_in_new_window', { path }),
  /** The tabs this window was opened to restore, if any. Drained by the read. */
  takeRestore: (): Promise<WindowTabs | null> => invoke('take_window_restore'),
};

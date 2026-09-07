/**
 * Typed transport to the window registry in Rust. Like `engine.ts`, this is
 * the only module that talks to Tauri for what it covers.
 *
 * The registry has to live in Rust because every question it answers spans
 * windows, and a webview can only see itself. Two SpeedyF windows share one
 * `localStorage` and nothing else — no shared memory, no way to ask "does
 * anyone already have this file open", and no way to keep a session between
 * them, since both would write the same key and the last one would win.
 */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

/** One window's open documents, in tab-strip order. */
export interface WindowTabs {
  tabs: string[];
  activePath: string | null;
}

/** Everything a window is owed at mount, drained in one call: the tabs it
 * should reopen, and any paths already waiting for it (a cold launch, or a
 * tab another window handed over a moment ago). */
export interface WindowStartup {
  restore: WindowTabs;
  opens: string[];
}

/** Another window, as a "move tab to…" menu sees it. */
export interface WindowSummary {
  label: string;
  /** The document it is showing — the only way to tell two windows apart. */
  title: string;
  tabCount: number;
}

export const MAIN_LABEL = 'main';

let cachedLabel: string | null = null;

/** This window's label: its address for everything below.
 *
 * Resolved lazily and once. Asking Tauri at module scope would throw in the
 * unit tests, which import stores without a webview around them.
 */
export function windowLabel(): string {
  if (cachedLabel === null) {
    try {
      cachedLabel = getCurrentWindow().label;
    } catch {
      cachedLabel = MAIN_LABEL;
    }
  }
  return cachedLabel;
}

export function isMainWindow(): boolean {
  return windowLabel() === MAIN_LABEL;
}

export interface WindowApi {
  startup(label: string): Promise<WindowStartup>;
  /** Publishes what this window has open. Every window republishes on any tab
   * change: the registry is both the saved session and what the *other*
   * windows read, so a stale entry is a wrong answer, not just a stale one. */
  publishTabs(label: string, tabs: string[], activePath: string | null): Promise<void>;
  roster(exclude: string | null): Promise<WindowSummary[]>;
  /** Which other window already has `path` open, if any. */
  findWindowForPath(path: string, exclude: string | null): Promise<string | null>;
  focus(label: string): Promise<void>;
  /** Opens a window, optionally on some documents; returns its label. */
  open(paths: string[]): Promise<string>;
  /** Hands documents to an existing window and brings it forward. */
  sendPaths(label: string, paths: string[]): Promise<void>;
}

export const windowApi: WindowApi = {
  startup: (label) => invoke('window_startup', { label }),
  publishTabs: (label, tabs, activePath) =>
    invoke('update_window_tabs', { label, tabs, activePath }),
  roster: (exclude) => invoke('window_roster', { exclude }),
  findWindowForPath: (path, exclude) => invoke('find_window_for_path', { path, exclude }),
  focus: (label) => invoke('focus_window', { label }),
  open: (paths) => invoke('open_window', { paths }),
  sendPaths: (label, paths) => invoke('send_paths_to_window', { label, paths }),
};

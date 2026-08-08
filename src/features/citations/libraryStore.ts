/** Citation-library folder scan status — genuinely global/window-level,
 * independent of any open document or tab (unlike the per-tab hover/preview
 * session in linkStore.ts). */
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { createStore } from 'solid-js/store';
import { engine, isEngineError } from '../../lib/transport/engine';
import type { LibraryStatus } from '../../types/engine';
import { settings, updateSettings } from '../../stores/settings';

const defaultLibraryStatus: LibraryStatus = {
  root: settings.libraryRoot,
  indexed: 0,
  total: 0,
  scanning: false,
};

const [state, setState] = createStore<{
  library: LibraryStatus;
  libraryError: string | null;
}>({
  library: defaultLibraryStatus,
  libraryError: null,
});

let libraryStatusSeq = 0;

/** Cache-invalidation hooks external-citation hover caches subscribe to
 * (each tab's HoverMachine invalidates its own `external:` cache entries
 * whenever the library status materially changes). */
const changeListeners = new Set<() => void>();

export const libraryStore = {
  state,

  onChange(listener: () => void): () => void {
    changeListeners.add(listener);
    return () => changeListeners.delete(listener);
  },

  async initializeLibrary(): Promise<void> {
    try {
      const preferred = settings.libraryRoot;
      const current = await engine.libraryStatus();
      if (!current.root && preferred) await engine.setLibraryRoot(preferred);
      await libraryStore.refreshLibraryStatus();
    } catch (error) {
      setState('libraryError', isEngineError(error) ? error.message : String(error));
    }
  },

  async refreshLibraryStatus(): Promise<LibraryStatus> {
    const seq = ++libraryStatusSeq;
    try {
      const next = await engine.libraryStatus();
      if (seq !== libraryStatusSeq) return state.library;
      const changed =
        next.root !== state.library.root ||
        next.indexed !== state.library.indexed ||
        next.scanning !== state.library.scanning;
      setState('library', next);
      setState('libraryError', null);
      if (next.root !== settings.libraryRoot) updateSettings({ libraryRoot: next.root });
      if (changed) for (const listener of changeListeners) listener();
      return next;
    } catch (error) {
      if (seq === libraryStatusSeq) {
        setState('libraryError', isEngineError(error) ? error.message : String(error));
      }
      return state.library;
    }
  },

  async chooseLibraryFolder(): Promise<void> {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected !== 'string') return;
    try {
      await engine.setLibraryRoot(selected);
      updateSettings({ libraryRoot: selected });
      for (const listener of changeListeners) listener();
      await libraryStore.refreshLibraryStatus();
    } catch (error) {
      setState('libraryError', isEngineError(error) ? error.message : String(error));
    }
  },

  async disableLibrary(): Promise<void> {
    try {
      await engine.setLibraryRoot(null);
      updateSettings({ libraryRoot: null });
      for (const listener of changeListeners) listener();
      await libraryStore.refreshLibraryStatus();
    } catch (error) {
      setState('libraryError', isEngineError(error) ? error.message : String(error));
    }
  },
};

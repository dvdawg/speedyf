/** The library: which folders are in it, how far indexing has got, and
 * searching across all of it.
 *
 * Genuinely global — independent of any open document or tab, unlike the
 * per-tab hover session in linkStore.ts. */
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { createStore } from 'solid-js/store';
import { engine, isEngineError } from '../../lib/transport/engine';
import type { LibraryHit, LibraryStatus } from '../../types/engine';
import { settings, updateSettings } from '../../stores/settings';

const defaultLibraryStatus: LibraryStatus = {
  roots: settings.libraryRoots,
  indexed: 0,
  total: 0,
  scanning: false,
  textIndexed: 0,
  textTotal: 0,
};

const [state, setState] = createStore<{
  library: LibraryStatus;
  libraryError: string | null;
  query: string;
  results: LibraryHit[];
  searching: boolean;
  truncated: boolean;
}>({
  library: defaultLibraryStatus,
  libraryError: null,
  query: '',
  results: [],
  searching: false,
  truncated: false,
});

let libraryStatusSeq = 0;
let searchSeq = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;

/** Cache-invalidation hooks external-citation hover caches subscribe to
 * (each tab's HoverMachine invalidates its own `external:` cache entries
 * whenever the library status materially changes). */
const changeListeners = new Set<() => void>();

const sameRoots = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((root, index) => root === b[index]);

export const libraryStore = {
  state,

  onChange(listener: () => void): () => void {
    changeListeners.add(listener);
    return () => changeListeners.delete(listener);
  },

  async initializeLibrary(): Promise<void> {
    try {
      const current = await engine.libraryStatus();
      // Settings are the record of what the user chose; the engine forgets if
      // its index was discarded, so re-assert anything it is missing.
      const missing = settings.libraryRoots.filter((root) => !current.roots.includes(root));
      for (const root of missing) await engine.changeLibraryRoot(root, true);
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
        !sameRoots(next.roots, state.library.roots) ||
        next.indexed !== state.library.indexed ||
        next.scanning !== state.library.scanning ||
        next.textIndexed !== state.library.textIndexed;
      setState('library', next);
      setState('libraryError', null);
      if (!sameRoots(next.roots, settings.libraryRoots)) {
        updateSettings({ libraryRoots: next.roots });
      }
      if (changed) for (const listener of changeListeners) listener();
      return next;
    } catch (error) {
      if (seq === libraryStatusSeq) {
        setState('libraryError', isEngineError(error) ? error.message : String(error));
      }
      return state.library;
    }
  },

  async addLibraryFolder(): Promise<void> {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected !== 'string') return;
    await libraryStore.changeFolder(selected, true);
  },

  async removeLibraryFolder(path: string): Promise<void> {
    await libraryStore.changeFolder(path, false);
  },

  async changeFolder(path: string, add: boolean): Promise<void> {
    try {
      await engine.changeLibraryRoot(path, add);
      for (const listener of changeListeners) listener();
      const next = await libraryStore.refreshLibraryStatus();
      updateSettings({ libraryRoots: next.roots });
      // Results from a folder that just left would otherwise linger.
      if (state.query.trim()) void libraryStore.runSearch(state.query);
    } catch (error) {
      setState('libraryError', isEngineError(error) ? error.message : String(error));
    }
  },

  /** Debounced so typing does not launch a scan per keystroke. */
  setQuery(query: string): void {
    setState('query', query);
    clearTimeout(searchTimer);
    if (!query.trim()) {
      searchSeq += 1;
      setState({ results: [], searching: false, truncated: false });
      return;
    }
    setState('searching', true);
    searchTimer = setTimeout(() => void libraryStore.runSearch(query), 250);
  },

  async runSearch(query: string): Promise<void> {
    const seq = ++searchSeq;
    try {
      const result = await engine.librarySearch(query, false);
      // A slower earlier query must not overwrite a faster later one.
      if (seq !== searchSeq) return;
      setState({
        results: result.documents,
        truncated: result.truncated,
        searching: false,
      });
    } catch (error) {
      if (seq !== searchSeq) return;
      setState({ results: [], searching: false, truncated: false });
      setState('libraryError', isEngineError(error) ? error.message : String(error));
    }
  },
};

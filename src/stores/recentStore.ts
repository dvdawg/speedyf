/** Recent-files list persisted to localStorage, capped at 20 entries. */
import { createStore } from 'solid-js/store';
import { engine } from '../lib/transport/engine';

export interface RecentEntry {
  path: string;
  name: string;
  lastOpened: number;
  sizeBytes: number | null;
}

interface RecentFile {
  version: 1;
  entries: RecentEntry[];
}

const KEY = 'speedyf-recent';
const MAX_ENTRIES = 20;

function load(): RecentEntry[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<RecentFile>;
      if (Array.isArray(parsed.entries)) {
        return parsed.entries.filter(
          (e): e is RecentEntry =>
            typeof e === 'object' &&
            e !== null &&
            typeof e.path === 'string' &&
            typeof e.name === 'string' &&
            typeof e.lastOpened === 'number'
        );
      }
    }
  } catch {
    /* corrupted recent list falls back to empty */
  }
  return [];
}

const [state, setState] = createStore<{ entries: RecentEntry[] }>({ entries: load() });

function persist() {
  try {
    localStorage.setItem(KEY, JSON.stringify({ version: 1, entries: state.entries }));
  } catch {
    /* storage unavailable (private mode) — recent list stays session-only */
  }
}

export const recentStore = {
  state,

  recordOpen({ path, name }: { path: string; name: string }) {
    const next = state.entries.filter((e) => e.path !== path);
    next.unshift({ path, name, lastOpened: Date.now(), sizeBytes: null });
    setState('entries', next.slice(0, MAX_ENTRIES));
    persist();

    void engine
      .fileMetadata(path)
      .then(({ sizeBytes }) => {
        const idx = state.entries.findIndex((e) => e.path === path);
        if (idx < 0) return;
        setState('entries', idx, 'sizeBytes', sizeBytes);
        persist();
      })
      .catch(() => undefined);
  },

  remove(path: string) {
    setState(
      'entries',
      state.entries.filter((e) => e.path !== path)
    );
    persist();
  },

  clear() {
    setState('entries', []);
    persist();
  },
};

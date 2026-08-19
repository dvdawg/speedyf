/** Recent-files list persisted to localStorage, capped at 20 entries. */
import { createStore } from 'solid-js/store';
import { engine } from '../lib/transport/engine';

/** Where reading stopped, stored zoom-independently: a page plus how far down
 * it the viewport's top edge sat. Storing a raw scroll offset would restore
 * wrongly at any other zoom or window size. */
export interface ReadingPosition {
  page: number;
  /** 0..1 down the page */
  fraction: number;
}

export interface RecentEntry {
  path: string;
  name: string;
  lastOpened: number;
  sizeBytes: number | null;
  position?: ReadingPosition;
}

function validPosition(value: unknown): value is ReadingPosition {
  if (typeof value !== 'object' || value === null) return false;
  const p = value as Partial<ReadingPosition>;
  return (
    typeof p.page === 'number' &&
    Number.isInteger(p.page) &&
    p.page >= 0 &&
    typeof p.fraction === 'number' &&
    Number.isFinite(p.fraction) &&
    p.fraction >= 0 &&
    p.fraction <= 1
  );
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
        ).map((e) => {
          // drop a malformed position rather than the whole entry
          if (validPosition(e.position)) return e;
          const { position: _discarded, ...rest } = e;
          return rest;
        });
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
    // Carry the reading position across: this rebuilds the entry to move it to
    // the front, and dropping the position here would clear it on every open —
    // exactly when it is about to be used.
    const previous = state.entries.find((e) => e.path === path);
    const next = state.entries.filter((e) => e.path !== path);
    next.unshift({
      path,
      name,
      lastOpened: Date.now(),
      sizeBytes: null,
      ...(previous?.position ? { position: previous.position } : {}),
    });
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

  /** Remembers where reading stopped. Silently ignores paths that have fallen
   * off the list — a position with nothing to resume is not worth keeping. */
  recordPosition(path: string, position: ReadingPosition) {
    const index = state.entries.findIndex((e) => e.path === path);
    if (index < 0 || !validPosition(position)) return;
    const current = state.entries[index]!.position;
    if (current && current.page === position.page && current.fraction === position.fraction) {
      return;
    }
    setState('entries', index, 'position', position);
    persist();
  },

  positionFor(path: string): ReadingPosition | null {
    return state.entries.find((e) => e.path === path)?.position ?? null;
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

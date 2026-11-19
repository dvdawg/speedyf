/** Recent-files list persisted to localStorage, capped at 20 entries. */
import { createStore } from 'solid-js/store';
import { engine } from '../lib/transport/engine';
import { desktop } from '../lib/transport/desktop';
import type { FitMode } from '../types/model';
import { restoredZoom } from '../features/document/resumeView';

/** Where reading stopped, stored zoom-independently: a page plus how far down
 * it the viewport's top edge sat. Storing a raw scroll offset would restore
 * wrongly at any other zoom or window size.
 *
 * And the zoom it was read at, so the file comes back the same way however
 * it is reopened (see features/document/resumeView.ts). Added later, so both
 * are optional: a position from an older launch has neither and opens at the
 * default zoom, as it always did. */
export interface ReadingPosition {
  page: number;
  /** 0..1 down the page */
  fraction: number;
  /** The zoom factor; for a fit mode, the last one it worked out to. */
  zoom?: number;
  fitMode?: FitMode;
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

/** A saved position as it can be used: page and fraction checked, the zoom
 * clamped into range, and a zoom that cannot be used dropped on its own
 * rather than taking the position with it. Null when the position itself is
 * unusable. Pure. */
export function sanitizePosition(value: unknown): ReadingPosition | null {
  if (!validPosition(value)) return null;
  const position: ReadingPosition = { page: value.page, fraction: value.fraction };
  const zoom = restoredZoom(value);
  if (zoom) Object.assign(position, zoom);
  return position;
}

function samePosition(a: ReadingPosition, b: ReadingPosition): boolean {
  return (
    a.page === b.page && a.fraction === b.fraction && a.zoom === b.zoom && a.fitMode === b.fitMode
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
        return parsed.entries
          .filter(
            (e): e is RecentEntry =>
              typeof e === 'object' &&
              e !== null &&
              typeof e.path === 'string' &&
              typeof e.name === 'string' &&
              typeof e.lastOpened === 'number'
          )
          .map((e) => {
            // drop a malformed position rather than the whole entry
            const { position: saved, ...rest } = e;
            const position = sanitizePosition(saved);
            return position ? { ...rest, position } : rest;
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
    // The system's list too: the Dock menu's recent documents on macOS.
    void desktop.noteRecentDocument(path);

    void engine
      .fileMetadata(path)
      // An async completion is an event, not a tracked scope: reading the list
      // as it is *now* is the point, since the fetch may have outlived the
      // entry it was about.
      // eslint-disable-next-line solid/reactivity
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
    const next = sanitizePosition(position);
    if (index < 0 || !next) return;
    const current = state.entries[index]!.position;
    if (current && samePosition(current, next)) return;
    // Replaced whole, not merged: a store merge would keep a stale zoom.
    setState('entries', index, { position: next });
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
    void desktop.clearRecentDocuments();
  },
};

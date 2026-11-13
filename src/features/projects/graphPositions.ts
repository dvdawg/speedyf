/** Where the user dragged graph nodes, kept across sessions.
 *
 * Per scope (a project id, or "all"), a map from paper id to a position in
 * layout space (pixels at zoom 1), so zoom, pan and the window's size do
 * not matter. One versioned localStorage key holds every scope; v2 since
 * layout space stopped being the unit square, so v1 positions are not
 * misread as pixels. Storage can be
 * missing, full or broken: every read and write is wrapped, and the graph
 * works without it, it just forgets.
 *
 * Bounded: at most MAX_POSITIONS per scope (the most recently placed are
 * kept) and MAX_SCOPES scopes (the least recently saved goes). A save keeps
 * only the papers still in the scope, which prunes the departed. */
import type { Point } from './graphLayout';

export const STORAGE_KEY = 'speedyf-graph-positions-v2';
export const MAX_POSITIONS = 2000;
export const MAX_SCOPES = 50;

interface Scope {
  /** When it was last saved, for least-recently-used eviction. */
  used: number;
  positions: Record<string, [number, number]>;
}

type Stored = Record<string, Scope>;

function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

function readAll(): Stored {
  try {
    const text = storage()?.getItem(STORAGE_KEY);
    if (!text) return {};
    const parsed: unknown = JSON.parse(text);
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {};
    return parsed as Stored;
  } catch {
    return {};
  }
}

function writeAll(all: Stored): void {
  try {
    const store = storage();
    if (!store) return;
    if (Object.keys(all).length === 0) store.removeItem(STORAGE_KEY);
    else store.setItem(STORAGE_KEY, JSON.stringify(all));
  } catch {
    // Full or refused: the positions live for this session only.
  }
}

const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/** The scope's stored positions; anything malformed is skipped. */
export function loadPositions(scope: string): Record<string, Point> {
  const entry = readAll()[scope];
  const out: Record<string, Point> = {};
  if (!entry || typeof entry.positions !== 'object' || entry.positions === null) return out;
  for (const [id, value] of Object.entries(entry.positions)) {
    if (Array.isArray(value) && finite(value[0]) && finite(value[1])) {
      out[id] = { x: value[0], y: value[1] };
    }
  }
  return out;
}

/** Replaces the scope's positions with `positions`, less any paper not in
 * `present`. An empty result removes the scope. */
export function savePositions(
  scope: string,
  positions: Record<string, Point>,
  present?: ReadonlySet<string>
): void {
  const all = readAll();
  const kept = Object.entries(positions).filter(([id]) => !present || present.has(id));
  if (kept.length === 0) {
    delete all[scope];
    writeAll(all);
    return;
  }
  const recent = kept.slice(-MAX_POSITIONS);
  const round = (v: number) => Math.round(v * 1e5) / 1e5;
  // Strictly newer than every other scope, even within one millisecond.
  const newest = Math.max(0, ...Object.values(all).map((e) => e.used ?? 0));
  all[scope] = {
    used: Math.max(Date.now(), newest + 1),
    positions: Object.fromEntries(recent.map(([id, p]) => [id, [round(p.x), round(p.y)]])),
  };
  const scopes = Object.entries(all);
  if (scopes.length > MAX_SCOPES) {
    scopes.sort((a, b) => (b[1].used ?? 0) - (a[1].used ?? 0));
    for (const [key] of scopes.slice(MAX_SCOPES)) delete all[key];
  }
  writeAll(all);
}

export function clearPositions(scope: string): void {
  const all = readAll();
  if (!(scope in all)) return;
  delete all[scope];
  writeAll(all);
}

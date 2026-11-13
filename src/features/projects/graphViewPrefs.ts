/** How busy the citation graph is drawn: the View popover's choices, kept
 * for every graph and across sessions under one localStorage key. Storage
 * can be missing or broken: every read and write is wrapped, and the
 * graph falls back to the defaults, which draw it the way it always was. */

export type EdgeMode = 'all' | 'selected';
export type LabelMode = 'many' | 'few' | 'hover';
export type UnlinkedMode = 'hide' | 'show';

export interface GraphViewPrefs {
  /** All edges, or only those touching the hovered or selected node. */
  edges: EdgeMode;
  /** Many: the label quota as zoom allows; few: the most cited only;
   * hover: none until a node is hovered or picked. */
  labels: LabelMode;
  /** Items with no edge in the view: counted in a chip, or drawn too. */
  unlinked: UnlinkedMode;
  /** Ghosts for works cited by at least `min` items of the view. */
  missing: { on: boolean; min: number };
  /** Ghosts for works cited with at least `min` members. */
  citedWith: { on: boolean; min: number };
}

export const PREFS_KEY = 'speedyf-graph-view-v1';

export const MISSING_MIN = 1;
export const MISSING_MAX = 5;
/** The engine's own floor: two members, or it is not co-citation. */
export const CITED_WITH_MIN = 2;
export const CITED_WITH_MAX = 6;

export const DEFAULT_PREFS: GraphViewPrefs = {
  edges: 'all',
  labels: 'many',
  unlinked: 'hide',
  missing: { on: false, min: 2 },
  citedWith: { on: false, min: 2 },
};

const clampInt = (value: unknown, lo: number, hi: number, fallback: number) =>
  typeof value === 'number' && Number.isFinite(value)
    ? Math.max(lo, Math.min(hi, Math.round(value)))
    : fallback;

function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

/** The stored choices, anything malformed replaced by its default. */
export function loadPrefs(): GraphViewPrefs {
  let raw: Record<string, unknown> = {};
  try {
    const text = storage()?.getItem(PREFS_KEY);
    const parsed: unknown = text ? JSON.parse(text) : null;
    if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
      raw = parsed as Record<string, unknown>;
    }
  } catch {
    // Broken: the defaults.
  }
  const pair = (value: unknown) =>
    typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
  const missing = pair(raw.missing);
  const citedWith = pair(raw.citedWith);
  return {
    edges: raw.edges === 'selected' ? 'selected' : 'all',
    labels: raw.labels === 'few' || raw.labels === 'hover' ? raw.labels : 'many',
    unlinked: raw.unlinked === 'show' ? 'show' : 'hide',
    missing: {
      on: missing.on === true,
      min: clampInt(missing.min, MISSING_MIN, MISSING_MAX, DEFAULT_PREFS.missing.min),
    },
    citedWith: {
      on: citedWith.on === true,
      min: clampInt(citedWith.min, CITED_WITH_MIN, CITED_WITH_MAX, DEFAULT_PREFS.citedWith.min),
    },
  };
}

export function savePrefs(prefs: GraphViewPrefs): void {
  try {
    storage()?.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Full or refused: the choices last for this session.
  }
}

/** Genuinely window-level chrome state — shared across every tab, unlike
 * per-document viewport/search/citation state which lives on each tab. */
import { createStore } from 'solid-js/store';

/** Built-in sidebar views, plus `ext:<extension id>/<panel id>` for a panel an
 * extension contributed. One string rather than a discriminated pair because
 * the mode is persisted and compared all over the shell, and a contributed
 * panel should be no more special to that code than a built-in one. */
export type SidebarMode = 'pages' | 'outline' | 'formal' | 'figures' | 'notes' | `ext:${string}`;

export const EXTENSION_MODE_PREFIX = 'ext:';

export function extensionPanelId(mode: SidebarMode): string | null {
  return mode.startsWith(EXTENSION_MODE_PREFIX) ? mode.slice(EXTENSION_MODE_PREFIX.length) : null;
}

/** Default sidebar width. Fits the five built-in panel buttons on one row. */
export const DEFAULT_SIDEBAR_WIDTH = 188;
/** Narrow enough to be a sliver, wide enough that the mode buttons and a row
 * of content are still usable. Below this, collapsing is the better answer. */
const MIN_SIDEBAR_WIDTH = 150;
/** Absolute ceiling, further limited to a share of the window below. */
const MAX_SIDEBAR_WIDTH = 560;
/** The sidebar may never take more of the window than this. The document is
 * the point of the app; a panel that can squeeze it to nothing is a way to get
 * the window into a state the user cannot read out of. */
const MAX_SIDEBAR_FRACTION = 0.5;

/** Constrain a dragged width. Pure, so the bounds are testable without a DOM. */
export function clampSidebarWidth(desired: number, windowWidth: number): number {
  if (!Number.isFinite(desired)) return DEFAULT_SIDEBAR_WIDTH;
  const ceiling = Math.min(
    MAX_SIDEBAR_WIDTH,
    Math.max(MIN_SIDEBAR_WIDTH, Math.floor(windowWidth * MAX_SIDEBAR_FRACTION))
  );
  return Math.round(Math.min(Math.max(desired, MIN_SIDEBAR_WIDTH), ceiling));
}

const WIDTH_KEY = 'speedyf-sidebar-width';

function storedWidth(): number {
  try {
    const raw = localStorage.getItem(WIDTH_KEY);
    if (raw === null) return DEFAULT_SIDEBAR_WIDTH;
    const parsed = Number(raw);
    // Clamped against the window it is being restored into, not the one it was
    // set in: a width saved on an external display must not swallow a laptop.
    return clampSidebarWidth(parsed, typeof window === 'undefined' ? 1280 : window.innerWidth);
  } catch {
    return DEFAULT_SIDEBAR_WIDTH;
  }
}

export const [ui, setUi] = createStore({
  sidebarOpen: true,
  sidebarMode: 'pages' as SidebarMode,
  sidebarWidth: storedWidth(),
});

/** Set the sidebar width, clamped and persisted.
 *
 * Written on every pointer move during a drag, which is why the write is
 * cheap and guarded: a failing localStorage must not break resizing.
 */
export function setSidebarWidth(desired: number): void {
  const width = clampSidebarWidth(
    desired,
    typeof window === 'undefined' ? 1280 : window.innerWidth
  );
  setUi('sidebarWidth', width);
  try {
    localStorage.setItem(WIDTH_KEY, String(width));
  } catch {
    /* storage unavailable — the width stays session-only */
  }
}

export function resetSidebarWidth(): void {
  setSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
}

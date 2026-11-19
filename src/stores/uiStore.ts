/** Genuinely window-level chrome state — shared across every tab, unlike
 * per-document viewport/search/citation state which lives on each tab. */
import { createStore } from 'solid-js/store';

/** Built-in sidebar views, plus `ext:<extension id>/<panel id>` for a panel an
 * extension contributed. One string rather than a discriminated pair because
 * the mode is persisted and compared all over the shell, and a contributed
 * panel should be no more special to that code than a built-in one. */
export type SidebarMode =
  'pages' | 'outline' | 'formal' | 'figures' | 'notes' | 'paper' | `ext:${string}`;

export const EXTENSION_MODE_PREFIX = 'ext:';

export function extensionPanelId(mode: SidebarMode): string | null {
  return mode.startsWith(EXTENSION_MODE_PREFIX) ? mode.slice(EXTENSION_MODE_PREFIX.length) : null;
}

/** Default sidebar width. The smallest width that fits the six built-in panel
 * buttons on one row: six 28px buttons and five 2px gaps (178px) inside the
 * strip's 6px padding, plus the sidebar's own 1px right border, which the
 * border-box width includes. One pixel less and the sixth button wraps. */
export const DEFAULT_SIDEBAR_WIDTH = 191;
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

/** Where the sidebar stands in one window: shown or not, how wide, and which
 * panel it shows. Saved per window beside its tabs (sessionStore), and once
 * more as the last one used, which a brand-new window starts from. */
export interface SidebarState {
  open: boolean;
  width: number;
  mode: SidebarMode;
}

/** A saved sidebar as read back: any field may be missing, from an older
 * launch or a damaged record, and is then taken from somewhere else. */
export type SavedSidebar = Partial<SidebarState>;

export const DEFAULT_SIDEBAR: SidebarState = {
  open: true,
  width: DEFAULT_SIDEBAR_WIDTH,
  mode: 'pages',
};

const BUILT_IN_MODES: readonly string[] = [
  'pages',
  'outline',
  'formal',
  'figures',
  'notes',
  'paper',
];

/** A mode this version knows how to show. An extension panel is accepted on
 * its form alone: whether that extension is still installed is only known
 * once extensions have loaded (see Sidebar). */
export function isSidebarMode(value: unknown): value is SidebarMode {
  if (typeof value !== 'string') return false;
  if (BUILT_IN_MODES.includes(value)) return true;
  return value.startsWith(EXTENSION_MODE_PREFIX) && value.length > EXTENSION_MODE_PREFIX.length;
}

/** The well-formed fields of a saved sidebar; undefined when none are. Pure. */
export function sanitizeSidebar(value: unknown): SavedSidebar | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const v = value as Record<string, unknown>;
  const out: SavedSidebar = {};
  if (typeof v.open === 'boolean') out.open = v.open;
  if (typeof v.width === 'number' && Number.isFinite(v.width) && v.width > 0) {
    out.width = v.width;
  }
  if (isSidebarMode(v.mode)) out.mode = v.mode;
  return Object.keys(out).length > 0 ? out : undefined;
}

/** A saved sidebar made whole: missing fields from `fallback`, and the width
 * clamped to the window it is restored into, not the one it was saved in (a
 * width saved on an external display must not swallow a laptop). Pure. */
export function resolveSidebar(
  saved: SavedSidebar | undefined,
  fallback: SidebarState,
  windowWidth: number
): SidebarState {
  return {
    open: saved?.open ?? fallback.open,
    mode: saved?.mode ?? fallback.mode,
    width: clampSidebarWidth(saved?.width ?? fallback.width, windowWidth),
  };
}

const LAST_USED_KEY = 'speedyf-sidebar';
/** Before the whole sidebar was remembered, only its width was. */
const LEGACY_WIDTH_KEY = 'speedyf-sidebar-width';

function currentWindowWidth(): number {
  return typeof window === 'undefined' ? 1280 : window.innerWidth;
}

/** The sidebar the last window to change one left, for a window with nothing
 * of its own to restore. */
export function lastUsedSidebar(): SavedSidebar | undefined {
  try {
    const raw = localStorage.getItem(LAST_USED_KEY);
    if (raw !== null) return sanitizeSidebar(JSON.parse(raw));
    const legacy = localStorage.getItem(LEGACY_WIDTH_KEY);
    if (legacy !== null) return sanitizeSidebar({ width: Number(legacy) });
  } catch {
    /* unreadable: the defaults */
  }
  return undefined;
}

export function saveLastUsedSidebar(state: SidebarState): void {
  try {
    localStorage.setItem(LAST_USED_KEY, JSON.stringify(state));
  } catch {
    /* storage unavailable: remembered for this run only */
  }
}

const initial = resolveSidebar(lastUsedSidebar(), DEFAULT_SIDEBAR, currentWindowWidth());

export const [ui, setUi] = createStore({
  sidebarOpen: initial.open,
  sidebarMode: initial.mode,
  sidebarWidth: initial.width,
});

/** This window's sidebar as it stands. */
export function sidebarState(): SidebarState {
  return { open: ui.sidebarOpen, width: ui.sidebarWidth, mode: ui.sidebarMode };
}

/** Puts back a saved sidebar. Fields it lacks keep what the window has now,
 * and the width is clamped to `windowWidth` (the window as it is, unless the
 * caller knows the frame it is about to have). */
export function applySidebar(
  saved: SavedSidebar | undefined,
  windowWidth = currentWindowWidth()
): void {
  const next = resolveSidebar(saved, sidebarState(), windowWidth);
  setUi({ sidebarOpen: next.open, sidebarMode: next.mode, sidebarWidth: next.width });
}

/** Set the sidebar width, clamped. Written on every pointer move during a
 * drag, so it only touches the store: saving is debounced elsewhere
 * (features/windows/sidebarState.ts). */
export function setSidebarWidth(desired: number): void {
  setUi('sidebarWidth', clampSidebarWidth(desired, currentWindowWidth()));
}

export function resetSidebarWidth(): void {
  setSidebarWidth(DEFAULT_SIDEBAR_WIDTH);
}

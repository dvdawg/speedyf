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

export const [ui, setUi] = createStore({
  sidebarOpen: true,
  sidebarMode: 'pages' as SidebarMode,
});

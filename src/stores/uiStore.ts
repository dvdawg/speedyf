/** Genuinely window-level chrome state — shared across every tab, unlike
 * per-document viewport/search/citation state which lives on each tab. */
import { createStore } from 'solid-js/store';

export type SidebarMode = 'pages' | 'outline' | 'formal' | 'figures' | 'notes';

export const [ui, setUi] = createStore({
  sidebarOpen: true,
  sidebarMode: 'pages' as SidebarMode,
});

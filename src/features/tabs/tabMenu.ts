/** What you can do to one tab, on its right-click menu: close it, close the
 * ones around it, or hand it to another window.
 *
 * A context menu rather than more buttons on the pill: a pill is 132px wide
 * at its narrowest and already carries a name, a dirty dot and a close
 * button, and none of these is worth taking width from the name.
 *
 * The window list is fetched when the menu opens, never cached: windows come
 * and go while the menu is closed, and offering a destination that no longer
 * exists is worse than offering none. Moving a tab is the same transactional
 * handoff as dragging it out of the strip: unsaved edits go with it. */
import { popupMenu, SEPARATOR, type MenuEntry } from '../../lib/nativeMenu';
import { windowApi, type WindowSummary } from '../../lib/transport/windows';
import { tabsStore } from '../../stores/tabsStore';
import {
  closeOtherTabs,
  closeTab,
  closeTabsToTheRight,
  detachTab,
  moveTabToWindow,
} from '../document/tabsController';
import { windowTransferPending } from '../windows/windowController';

/** The menu for tab `id`, with `windows` as the destinations it can move
 * to. Empty when the tab has gone. */
export function tabMenuEntries(id: string, windows: readonly WindowSummary[]): MenuEntry[] {
  const tabs = tabsStore.state.tabs;
  const at = tabs.findIndex((t) => t.id === id);
  const tab = tabs[at];
  if (!tab) return [];
  // The same conditions as dragging a tab out of the strip.
  const movable =
    !tab.opening &&
    tab.documentStore.state.loaded &&
    !tab.documentStore.state.saving &&
    !windowTransferPending();
  return [
    { text: 'Close Tab', run: () => void closeTab(id) },
    { text: 'Close Other Tabs', enabled: tabs.length > 1, run: () => void closeOtherTabs(id) },
    {
      text: 'Close Tabs to the Right',
      enabled: at < tabs.length - 1,
      run: () => void closeTabsToTheRight(id),
    },
    SEPARATOR,
    { text: 'Move to New Window', enabled: movable, run: () => void detachTab(id) },
    ...windows.map((win): MenuEntry => ({
      text: `Move to "${win.title}"`,
      enabled: movable,
      run: () => void moveTabToWindow(id, win.label),
    })),
  ];
}

export async function openTabMenu(id: string): Promise<void> {
  const windows = await windowApi.roster().catch(() => []);
  await popupMenu(tabMenuEntries(id, windows));
}

/** What you can do to one tab: close it, close the ones around it, or hand it
 * to another window.
 *
 * A context menu rather than more buttons on the pill. A pill is 132px wide
 * at its narrowest and already carries a name, a dirty dot, and a close
 * button; the operations here are occasional, and none of them is worth
 * taking width from the name — which is the only thing on the pill that tells
 * two papers apart.
 *
 * The window list is fetched when the menu opens, never cached: windows come
 * and go while the menu is closed, and offering a destination that no longer
 * exists is worse than offering none.
 */
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { windowApi, windowLabel, type WindowSummary } from '../../lib/transport/windows';
import { tabsStore } from '../../stores/tabsStore';
import {
  closeOtherTabs,
  closeTab,
  closeTabsToTheRight,
  moveTabToNewWindow,
  moveTabToWindow,
} from '../document/tabsController';
import { MENU_WIDTH, tabMenuHeight, tabMenuPosition } from './tabMenuLayout';

export interface TabMenuTarget {
  tabId: string;
  x: number;
  y: number;
}

interface Item {
  label: string;
  run: () => void;
  enabled: boolean;
  /** Draw a rule above this item. */
  separated?: boolean;
}

export default function TabMenu(props: { target: TabMenuTarget; onClose: () => void }) {
  const [windows, setWindows] = createSignal<WindowSummary[]>([]);
  let menu!: HTMLDivElement;

  const index = () => tabsStore.state.tabs.findIndex((t) => t.id === props.target.tabId);
  const tab = () => tabsStore.state.tabs[index()] ?? null;
  const path = () => tab()?.documentStore.state.path ?? null;

  const items = (): Item[] => {
    const at = index();
    const total = tabsStore.state.tabs.length;
    const id = props.target.tabId;
    const movable = path() !== null;
    const list: Item[] = [
      { label: 'Close Tab', enabled: true, run: () => void closeTab(id) },
      { label: 'Close Other Tabs', enabled: total > 1, run: () => void closeOtherTabs(id) },
      {
        label: 'Close Tabs to the Right',
        enabled: at >= 0 && at < total - 1,
        run: () => void closeTabsToTheRight(id),
      },
      {
        label: 'Move to New Window',
        separated: true,
        // Moving the only tab of a window into a window of its own would just
        // rebuild the window it left.
        enabled: movable && total > 1,
        run: () => void moveTabToNewWindow(id),
      },
    ];
    for (const win of windows()) {
      list.push({
        label: `Move to “${win.title}”`,
        enabled: movable,
        run: () => void moveTabToWindow(id, win.label),
      });
    }
    return list;
  };

  const position = () =>
    tabMenuPosition(
      { x: props.target.x, y: props.target.y },
      {
        width: MENU_WIDTH,
        height: tabMenuHeight(items().length, items().filter((i) => i.separated).length),
      },
      { width: window.innerWidth, height: window.innerHeight }
    );

  const focusItem = (delta: number) => {
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>('button:not([disabled])')];
    if (buttons.length === 0) return;
    const current = buttons.findIndex((b) => b === document.activeElement);
    // Nothing focused yet: down starts at the top of the menu, up at the end.
    const next =
      current < 0
        ? delta > 0
          ? 0
          : buttons.length - 1
        : (current + delta + buttons.length) % buttons.length;
    buttons[next]?.focus();
  };

  onMount(() => {
    void windowApi
      .roster(windowLabel())
      .then(setWindows)
      .catch(() => setWindows([]));

    // Capture, so the menu answers Escape before the app's global keymap
    // reads it as "unwind whatever is open" and closes something else.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        props.onClose();
        return;
      }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        e.stopPropagation();
        focusItem(e.key === 'ArrowDown' ? 1 : -1);
      }
    };
    const onPointerDown = (e: PointerEvent) => {
      if (!menu.contains(e.target as Node)) props.onClose();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('pointerdown', onPointerDown, true);
    // A menu anchored to a pointer position is wrong the moment anything
    // moves underneath it.
    window.addEventListener('resize', props.onClose);
    window.addEventListener('blur', props.onClose);
    onCleanup(() => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('resize', props.onClose);
      window.removeEventListener('blur', props.onClose);
    });
  });

  return (
    <Portal>
      <div
        class="tab-menu"
        role="menu"
        ref={menu}
        aria-label={`Actions for ${tab()?.documentStore.state.name ?? 'tab'}`}
        style={{ left: `${position().left}px`, top: `${position().top}px` }}
      >
        <For each={items()}>
          {(item) => (
            <>
              <Show when={item.separated}>
                <div class="tab-menu-separator" role="separator" />
              </Show>
              <button
                type="button"
                class="tab-menu-item"
                role="menuitem"
                disabled={!item.enabled}
                onClick={() => {
                  // The menu closes first: every item here changes the tab
                  // strip underneath it, and half of them prompt.
                  props.onClose();
                  item.run();
                }}
              >
                {item.label}
              </button>
            </>
          )}
        </For>
      </div>
    </Portal>
  );
}

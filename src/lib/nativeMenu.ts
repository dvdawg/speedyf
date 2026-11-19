/** Native right-click menus: real OS popup menus, built from plain data.
 *
 * A surface describes its menu as `MenuEntry` values and calls `popupMenu`;
 * nothing here knows what any entry does. The data form is what the tests
 * read, and `tidyMenu` keeps separators honest when entries are left out
 * conditionally (no leading, trailing or doubled rules).
 *
 * Everywhere without a menu of its own, `installContextMenuGuard` suppresses
 * the webview's default one ("Reload", "Inspect Element", "Back"), which is
 * the browser showing through. Editable text keeps it: Cut, Copy and Paste
 * are what a right-click in a text field is for. */
import {
  Menu,
  type CheckMenuItemOptions,
  type MenuItemOptions,
  type PredefinedMenuItemOptions,
  type SubmenuOptions,
} from '@tauri-apps/api/menu';

export type MenuEntry =
  | {
      kind?: 'item';
      text: string;
      run: () => void;
      enabled?: boolean;
      /** Draws a checkmark. */
      checked?: boolean;
    }
  | { kind: 'separator' }
  | { kind: 'submenu'; text: string; items: MenuEntry[]; enabled?: boolean }
  /** The OS's own editing items, which act on whatever has focus. */
  | { kind: 'native'; item: 'Copy' | 'Cut' | 'Paste' | 'SelectAll' };

export const SEPARATOR: MenuEntry = { kind: 'separator' };

/** Drops separators that would separate nothing, and empty submenus. */
export function tidyMenu(entries: readonly MenuEntry[]): MenuEntry[] {
  const out: MenuEntry[] = [];
  for (const entry of entries) {
    if (entry.kind === 'submenu') {
      const items = tidyMenu(entry.items);
      if (items.length > 0) out.push({ ...entry, items });
      continue;
    }
    if (entry.kind === 'separator' && (out.length === 0 || out.at(-1)?.kind === 'separator')) {
      continue;
    }
    out.push(entry);
  }
  while (out.at(-1)?.kind === 'separator') out.pop();
  return out;
}

type NativeOption =
  MenuItemOptions | CheckMenuItemOptions | SubmenuOptions | PredefinedMenuItemOptions;

function toOptions(entry: MenuEntry): NativeOption {
  switch (entry.kind) {
    case 'separator':
      return { item: 'Separator' };
    case 'native':
      return { item: entry.item };
    case 'submenu':
      return {
        text: entry.text,
        enabled: entry.enabled ?? true,
        items: entry.items.map(toOptions) as SubmenuOptions['items'],
      };
    default: {
      const base: MenuItemOptions = {
        text: entry.text,
        enabled: entry.enabled ?? true,
        action: () => entry.run(),
      };
      return entry.checked === undefined ? base : { ...base, checked: entry.checked };
    }
  }
}

/** The menu built for the last right-click. Closed when the next one opens,
 * not right after it pops up: on some platforms the item's action arrives
 * after the popup call has returned, and a closed menu would drop it. */
let previous: Menu | null = null;

/** Pops `entries` up at the pointer, in this window. */
export async function popupMenu(entries: readonly MenuEntry[]): Promise<void> {
  const tidy = tidyMenu(entries);
  if (tidy.length === 0) return;
  try {
    const menu = await Menu.new({ items: tidy.map(toOptions) as never });
    void previous?.close().catch(() => undefined);
    previous = menu;
    await menu.popup();
  } catch {
    // No menu is better than a broken one; there is nothing to report.
  }
}

/** Whether right-clicking `target` should keep the system's text menu. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  const editable = target.closest('input, textarea, select, [contenteditable]');
  if (!editable) return false;
  if (editable instanceof HTMLInputElement) {
    return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file'].includes(
      editable.type
    );
  }
  if (editable instanceof HTMLElement && editable.hasAttribute('contenteditable')) {
    return editable.isContentEditable;
  }
  return true;
}

/** Suppresses the webview's own context menu wherever the app has not put
 * one of its own, keeping it for editable text. Registered on `window` so a
 * surface's handler (on the element, or Solid's delegated one on the
 * document) has already run and had its chance to claim the event. */
export function installContextMenuGuard(): () => void {
  const onContextMenu = (e: MouseEvent) => {
    if (e.defaultPrevented || isEditableTarget(e.target)) return;
    e.preventDefault();
  };
  window.addEventListener('contextmenu', onContextMenu);
  return () => window.removeEventListener('contextmenu', onContextMenu);
}

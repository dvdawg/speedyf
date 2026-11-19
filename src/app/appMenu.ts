/** The macOS menu bar, live.
 *
 * Why it is built here rather than in Rust: the commands, their enabled
 * state and their checkmarks are all reactive state in this webview, so the
 * menu reads them where they live and pushes only what changed. A
 * Rust-built menu would need every window to mirror that state across the
 * bridge on every change, and every click to come back the same way.
 *
 * Each window builds its own menu bar and makes it the app's whenever it
 * gains focus, so the menu always describes the focused window: its tab, its
 * selection, its undo stack. The Settings window has a smaller one of its
 * own. Windows and Linux get none at all - their toolbar and the command
 * palette are the menus there - so on those platforms this does nothing. */
import { createEffect, createRoot, createSignal } from 'solid-js';
import { getVersion } from '@tauri-apps/api/app';
import {
  CheckMenuItem,
  Menu,
  MenuItem,
  PredefinedMenuItem,
  Submenu,
  type MenuItemOptions,
} from '@tauri-apps/api/menu';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { IS_MAC } from '../lib/platform';
import { desktop } from '../lib/transport/desktop';
import { modal } from '../stores/modalStore';
import { recentStore } from '../stores/recentStore';
import { paletteOpen } from '../features/palette/paletteStore';
import { quickOpenOpen } from '../features/palette/quickOpenStore';
import { windowTransferPending } from '../features/windows/windowController';
import { openInNewTabOrFocus } from '../features/document/tabsController';
import { type CommandCtx } from './commands';
import {
  commandToRun,
  itemState,
  menuBar,
  type ItemState,
  type MenuNode,
  type WindowKind,
} from './menuBar';
import { recentMenuEntries } from './recentMenu';

type CommandNode = Extract<MenuNode, { kind: 'command' }>;
type NativeMenuItem = MenuItem | CheckMenuItem | PredefinedMenuItem | Submenu;

function isTypingTarget(element: Element | null): boolean {
  return (
    element instanceof HTMLInputElement ||
    element instanceof HTMLTextAreaElement ||
    (element instanceof HTMLElement && element.isContentEditable)
  );
}

/** Whether a menu choice should be ignored: a dialog, the palette or a tab
 * handoff owns the window, as it owns the keyboard (shortcuts.ts). */
function blocked(): boolean {
  return modal.kind !== null || paletteOpen() || quickOpenOpen() || windowTransferPending();
}

function choose(node: CommandNode, ctx: CommandCtx): void {
  // Undo in a text field is the field's, wherever the field is.
  if (node.textEditing && isTypingTarget(document.activeElement)) {
    document.execCommand(node.textEditing);
    return;
  }
  if (blocked()) return;
  commandToRun(node, ctx)?.run(ctx);
}

const CREDITS =
  'A fast, memory-bounded PDF reader and reference manager. Released under the MIT License; ' +
  'built on PDFium, Tauri and SolidJS, each under its own open-source license.';

/** Builds this window's menu bar and keeps it current. Resolves to a
 * function that stops following its state. */
export async function installAppMenu(
  kind: WindowKind,
  ctx: () => CommandCtx
): Promise<() => void> {
  if (!IS_MAC) return () => undefined;

  const [typing, setTyping] = createSignal(false);
  const onFocus = () => setTyping(isTypingTarget(document.activeElement));
  document.addEventListener('focusin', onFocus);
  document.addEventListener('focusout', onFocus);

  const live: { node: CommandNode; item: MenuItem | CheckMenuItem; state: ItemState }[] = [];
  let windowsMenu: Submenu | null = null;
  let helpMenu: Submenu | null = null;
  let recentMenu: Submenu | null = null;
  const version = await getVersion().catch(() => '');

  const build = async (node: MenuNode): Promise<NativeMenuItem | null> => {
    switch (node.kind) {
      case 'separator':
        return PredefinedMenuItem.new({ item: 'Separator' });
      case 'native':
        return PredefinedMenuItem.new({ item: node.item });
      case 'about':
        return PredefinedMenuItem.new({
          text: 'About SpeedyF',
          item: {
            About: {
              name: 'SpeedyF',
              version,
              copyright: 'Copyright (c) 2026 the SpeedyF authors.',
              credits: CREDITS,
            },
          },
        });
      case 'quit':
        return MenuItem.new({
          text: 'Quit SpeedyF',
          accelerator: 'CmdOrCtrl+Q',
          action: () => void desktop.requestQuit(),
        });
      case 'recent': {
        recentMenu = await Submenu.new({ text: 'Open Recent', items: [] });
        return recentMenu;
      }
      case 'submenu': {
        const items = (await Promise.all(node.items.map(build))).filter(
          (item): item is NativeMenuItem => item !== null
        );
        const sub = await Submenu.new({ text: node.text, items });
        if (node.role === 'window') windowsMenu = sub;
        if (node.role === 'help') helpMenu = sub;
        return sub;
      }
      case 'command': {
        const state = itemState(node, ctx(), typing());
        if (!state) return null;
        const options: MenuItemOptions = {
          text: state.text,
          enabled: state.enabled,
          action: () => choose(node, ctx()),
          ...(state.accelerator ? { accelerator: state.accelerator } : {}),
        };
        const item =
          state.checked === undefined
            ? await MenuItem.new(options)
            : await CheckMenuItem.new({ ...options, checked: state.checked });
        live.push({ node, item, state });
        return item;
      }
    }
  };

  // Built once, reading the state as it is now; the effects below follow it.
  // eslint-disable-next-line solid/reactivity
  const top = (await Promise.all(menuBar(kind).map(build))).filter(
    (item): item is NativeMenuItem => item !== null
  );
  const menu = await Menu.new({ items: top });

  const disposeEffects = createRoot((dispose) => {
    for (const entry of live) {
      createEffect(() => {
        const next = itemState(entry.node, ctx(), typing());
        if (!next) return;
        const previous = entry.state;
        entry.state = next;
        if (next.text !== previous.text) void entry.item.setText(next.text).catch(() => undefined);
        if (next.enabled !== previous.enabled) {
          void entry.item.setEnabled(next.enabled).catch(() => undefined);
        }
        if (
          next.checked !== undefined &&
          next.checked !== previous.checked &&
          entry.item instanceof CheckMenuItem
        ) {
          void entry.item.setChecked(next.checked).catch(() => undefined);
        }
      });
    }
    // Open Recent follows the recent list, whichever window changed it.
    createEffect(() => {
      const entries = recentMenuEntries(recentStore.state.entries);
      void fillRecent(entries);
    });
    return dispose;
  });

  let filling = Promise.resolve();
  function fillRecent(entries: ReturnType<typeof recentMenuEntries>): Promise<void> {
    const target = recentMenu;
    if (!target) return Promise.resolve();
    filling = filling.then(async () => {
      try {
        for (const old of await target.items()) await target.remove(old);
        const items: NativeMenuItem[] = [];
        for (const entry of entries) {
          items.push(
            await MenuItem.new({
              text: entry.text,
              action: () => void openRecent(entry.path),
            })
          );
        }
        if (items.length > 0) items.push(await PredefinedMenuItem.new({ item: 'Separator' }));
        items.push(
          await MenuItem.new({
            text: 'Clear Menu',
            enabled: entries.length > 0,
            action: () => recentStore.clear(),
          })
        );
        await target.append(items);
      } catch {
        // The submenu keeps its previous entries.
      }
    });
    return filling;
  }

  const activate = async () => {
    try {
      await menu.setAsAppMenu();
      await windowsMenu?.setAsWindowsMenuForNSApp();
      await helpMenu?.setAsHelpMenuForNSApp();
    } catch {
      // The previous window's menu stays up.
    }
  };

  const win = getCurrentWindow();
  let disposed = false;
  let unlistenFocus: (() => void) | undefined;
  void win
    .onFocusChanged(({ payload: focused }) => {
      if (focused) void activate();
    })
    .then((unlisten) => {
      if (disposed) unlisten();
      else unlistenFocus = unlisten;
    })
    .catch(() => undefined);
  // Focus may have arrived while the menu was being built.
  if (await win.isFocused().catch(() => false)) await activate();

  return () => {
    disposed = true;
    unlistenFocus?.();
    disposeEffects();
    document.removeEventListener('focusin', onFocus);
    document.removeEventListener('focusout', onFocus);
  };
}

async function openRecent(path: string): Promise<void> {
  if (blocked()) return;
  // An entry that no longer opens is dropped, as on the Recents panel.
  if (!(await openInNewTabOrFocus(path))) recentStore.remove(path);
}

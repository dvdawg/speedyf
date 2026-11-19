/** The macOS menu bar, as data built from the command registry.
 *
 * Every item that does something in SpeedyF is a reference to a command in
 * `commands.ts`: its title, its key, whether it is enabled and whether it is
 * checked are all read from that one entry, so the menu bar and the command
 * palette cannot disagree. What is here is only where each command sits.
 * The rest are the operating system's own items (Services, Hide, Cut, Copy,
 * Paste, Minimize...), which act on whatever has focus.
 *
 * Pure: `appMenu.ts` turns this into native menus and keeps them current. */
import { commandById, type Command, type CommandCtx } from './commands';

/** The operating system's own menu items. */
export type NativeItem =
  | 'Services'
  | 'Hide'
  | 'HideOthers'
  | 'ShowAll'
  | 'Undo'
  | 'Redo'
  | 'Cut'
  | 'Copy'
  | 'Paste'
  | 'SelectAll'
  | 'Minimize'
  | 'Maximize'
  | 'Fullscreen'
  | 'BringAllToFront'
  | 'CloseWindow';

export type MenuNode =
  | {
      kind: 'command';
      id: string;
      /** Commands tried before `id` when it is chosen; the item is enabled
       * when any of them is. Find reaches the Projects graph's find first,
       * as Cmd+F does. */
      before?: readonly string[];
      /** `before` applies only on the Home and Projects screens, where no
       * document tab shows: the library's Undo is not the document's. */
      beforeAtHome?: boolean;
      /** Undo and Redo act on a focused text field before the document. */
      textEditing?: 'undo' | 'redo';
    }
  | { kind: 'separator' }
  | { kind: 'native'; item: NativeItem }
  | { kind: 'about' }
  | { kind: 'quit' }
  | { kind: 'recent' }
  | { kind: 'submenu'; text: string; items: readonly MenuNode[]; role?: 'window' | 'help' };

export type WindowKind = 'document' | 'settings';

const cmd = (id: string, extra?: Partial<Extract<MenuNode, { kind: 'command' }>>): MenuNode => ({
  kind: 'command',
  id,
  ...extra,
});
const sep: MenuNode = { kind: 'separator' };
const native = (item: NativeItem): MenuNode => ({ kind: 'native', item });
const submenu = (text: string, items: readonly MenuNode[], role?: 'window' | 'help'): MenuNode => ({
  kind: 'submenu',
  text,
  items,
  ...(role ? { role } : {}),
});

const APP_MENU = (kind: WindowKind): MenuNode =>
  submenu('SpeedyF', [
    { kind: 'about' },
    sep,
    // The Settings window's own menu has it too: Cmd+, there raises it.
    ...(kind === 'document' ? [cmd('app.settings')] : []),
    sep,
    native('Services'),
    sep,
    native('Hide'),
    native('HideOthers'),
    native('ShowAll'),
    sep,
    { kind: 'quit' },
  ]);

const DOCUMENT_MENUS: readonly MenuNode[] = [
  submenu('File', [
    cmd('file.newWindow'),
    cmd('file.open'),
    cmd('library.quickOpen'),
    { kind: 'recent' },
    sep,
    cmd('file.closeTab'),
    cmd('file.closeWindow'),
    cmd('file.closeOtherTabs'),
    cmd('file.closeTabsToTheRight'),
    cmd('file.reopenClosedTab'),
    sep,
    cmd('file.save'),
    cmd('file.saveAs'),
    sep,
    cmd('forms.importData'),
    cmd('forms.exportData'),
    cmd('markup.export'),
    cmd('markup.copyText'),
    cmd('notepad.exportMarkdown'),
    cmd('notepad.copyLatex'),
    sep,
    cmd('file.print'),
  ]),
  submenu('Edit', [
    cmd('edit.undo', { before: ['library.undo'], beforeAtHome: true, textEditing: 'undo' }),
    cmd('edit.redo', { before: ['library.redo'], beforeAtHome: true, textEditing: 'redo' }),
    sep,
    native('Cut'),
    native('Copy'),
    native('Paste'),
    native('SelectAll'),
    cmd('edit.deleteSelection'),
    sep,
    cmd('search.findInDocument', { before: ['library.findInGraph'] }),
    cmd('search.library'),
    sep,
    cmd('library.copyCiteKey'),
    cmd('library.copyCiteSelection'),
    cmd('library.copyBibtex'),
  ]),
  submenu('View', [
    cmd('view.zoomIn'),
    cmd('view.zoomOut'),
    cmd('view.actualSize'),
    cmd('view.fitPage'),
    cmd('view.fitWidth'),
    sep,
    cmd('view.toggleSidebar'),
    submenu('Sidebar', [
      cmd('panel.pages'),
      cmd('panel.outline'),
      cmd('panel.formal'),
      cmd('panel.figures'),
      cmd('panel.notes'),
      cmd('signatures.showPanel'),
      cmd('library.showPaperPanel'),
    ]),
    sep,
    submenu('Theme', [
      cmd('settings.themeLight'),
      cmd('settings.themeDark'),
      cmd('settings.themeSystem'),
    ]),
    cmd('settings.toggleSleepyf'),
    cmd('settings.toggleInvertPages'),
    cmd('settings.toggleHighlightFields'),
    cmd('settings.toggleLowMemory'),
    sep,
    cmd('view.commandPalette'),
    sep,
    native('Fullscreen'),
  ]),
  submenu('Go', [
    cmd('nav.back'),
    cmd('nav.forward'),
    sep,
    cmd('nav.nextPage'),
    cmd('nav.previousPage'),
    sep,
    cmd('file.goHome'),
    cmd('library.goToProjects'),
    cmd('library.switchLaunchScreen'),
  ]),
  submenu('Tools', [
    cmd('tools.select'),
    cmd('tools.highlight'),
    cmd('tools.ink'),
    cmd('tools.rect'),
    cmd('tools.textbox'),
    cmd('tools.note'),
    cmd('tools.bubble'),
    sep,
    cmd('markup.toggle'),
    cmd('markup.comment'),
    cmd('markup.typo'),
    cmd('markup.region'),
    sep,
    cmd('tools.forgetContributedText'),
  ]),
  submenu('Document', [
    cmd('page.rotateRight'),
    cmd('page.rotateLeft'),
    cmd('page.duplicate'),
    cmd('page.addBlank'),
    cmd('page.delete'),
    sep,
    submenu('Forms', [
      cmd('forms.prepare'),
      cmd('forms.autodetect'),
      cmd('forms.flatten'),
      cmd('forms.reset'),
    ]),
    submenu('Fill & Sign', [
      cmd('sign.fill'),
      cmd('sign.fillCheck'),
      cmd('sign.fillCross'),
      cmd('sign.fillDot'),
      cmd('sign.fillLine'),
      cmd('sign.fillCircle'),
      cmd('sign.fillDate'),
      sep,
      cmd('sign.place'),
      cmd('sign.placeInitials'),
      cmd('sign.addSignature'),
      cmd('sign.addInitials'),
      cmd('sign.menu'),
      cmd('sign.flatten'),
      sep,
      cmd('sign.certificate'),
    ]),
    submenu('Redact', [
      cmd('redact.tool'),
      cmd('redact.selection'),
      cmd('redact.find'),
      cmd('redact.apply'),
      cmd('redact.cancelApply'),
      sep,
      cmd('protect.removeHidden'),
    ]),
    sep,
    cmd('file.protect'),
  ]),
  submenu('Library', [
    cmd('library.showAllItems'),
    cmd('library.showNotFiled'),
    cmd('library.showOtherPdfs'),
    cmd('library.showNewItems'),
    cmd('library.showLibraryHealth'),
    sep,
    cmd('library.lookup'),
    sep,
    cmd('library.newProject'),
    cmd('library.importProjectBundle'),
    cmd('library.exportProjectBib'),
    cmd('library.exportProjectBundle'),
    cmd('library.syncProjectBib'),
    cmd('library.stopSyncingProjectBib'),
    sep,
    cmd('library.lookUpAll'),
    cmd('library.lookUpProject'),
    cmd('library.lookUpByTitle'),
    cmd('library.searchProjectFullText'),
    sep,
    cmd('library.mergeSelected'),
    cmd('library.splitItem'),
    cmd('library.markAsItem'),
    cmd('library.markNotItem'),
    cmd('library.addToSuggestedProject'),
    sep,
    cmd('library.toggleGraphView'),
    cmd('library.fitGraph'),
    cmd('library.findInGraph'),
    cmd('library.resetGraphLayout'),
    cmd('library.showItemInGraph'),
    cmd('library.leaveGraphFocus'),
  ]),
  submenu(
    'Window',
    [
      native('Minimize'),
      native('Maximize'),
      sep,
      cmd('nav.nextTab'),
      cmd('nav.previousTab'),
      cmd('file.detachTab'),
      sep,
      native('BringAllToFront'),
    ],
    'window'
  ),
  submenu('Help', [cmd('view.keyboardShortcuts')], 'help'),
];

/** The Settings window holds text fields and nothing else: its menus are the
 * system's editing items and the ways out of it. */
const SETTINGS_MENUS: readonly MenuNode[] = [
  submenu('File', [cmd('file.newWindow'), sep, native('CloseWindow')]),
  submenu('Edit', [
    native('Undo'),
    native('Redo'),
    sep,
    native('Cut'),
    native('Copy'),
    native('Paste'),
    native('SelectAll'),
  ]),
  submenu('Window', [native('Minimize'), sep, native('BringAllToFront')], 'window'),
];

/** The whole menu bar for a window of `kind`. */
export function menuBar(kind: WindowKind): MenuNode[] {
  return [APP_MENU(kind), ...(kind === 'document' ? DOCUMENT_MENUS : SETTINGS_MENUS)];
}

/** Every command id the menu bar references, in order. */
export function menuCommandIds(nodes: readonly MenuNode[]): string[] {
  const ids: string[] = [];
  for (const node of nodes) {
    if (node.kind === 'command') ids.push(node.id);
    else if (node.kind === 'submenu') ids.push(...menuCommandIds(node.items));
  }
  return ids;
}

/** A command chord in the registry's neutral form (`mod+shift+s`) as a menu
 * accelerator (`CmdOrCtrl+Shift+S`). Only chords with a modifier: a menu
 * that took Backspace or PageDown for itself would take them from every text
 * field too. */
export function toAccelerator(spec: string): string | null {
  const parts = spec.toLowerCase().split('+');
  const key = parts.pop();
  if (!key || parts.length === 0) return null;
  const modifiers: Record<string, string> = {
    mod: 'CmdOrCtrl',
    ctrl: 'Ctrl',
    shift: 'Shift',
    alt: 'Alt',
  };
  const mods = parts.map((part) => modifiers[part]);
  if (mods.some((m) => m === undefined)) return null;
  const named: Record<string, string> = {
    plus: '=',
    minus: '-',
    tab: 'Tab',
    backspace: 'Backspace',
    delete: 'Delete',
    escape: 'Escape',
    enter: 'Enter',
    space: 'Space',
    pageup: 'PageUp',
    pagedown: 'PageDown',
    arrowleft: 'Left',
    arrowright: 'Right',
    arrowup: 'Up',
    arrowdown: 'Down',
  };
  const keyName = named[key] ?? (/^[a-z0-9]$/.test(key) ? key.toUpperCase() : key);
  if (!/^([A-Z0-9]|[[\],./;'`\\=-]|[A-Z][a-zA-Z]+)$/.test(keyName)) return null;
  return [...mods, keyName].join('+');
}

/** What a command item shows right now. */
export interface ItemState {
  text: string;
  enabled: boolean;
  /** Absent for a command that is not a toggle or a choice. */
  checked?: boolean;
  accelerator: string | null;
}

/** The commands a node runs, in the order they are tried. */
export function commandChain(
  node: Extract<MenuNode, { kind: 'command' }>,
  ctx: CommandCtx
): Command[] {
  const before = node.beforeAtHome && ctx.tab !== null ? [] : (node.before ?? []);
  return [...before, node.id]
    .map((id) => commandById(id))
    .filter((c): c is Command => c !== undefined);
}

/** A command item's state, read from the registry. `typing` is whether a
 * text field has focus, where Undo and Redo always have something to do. */
export function itemState(
  node: Extract<MenuNode, { kind: 'command' }>,
  ctx: CommandCtx,
  typing = false
): ItemState | null {
  const command = commandById(node.id);
  if (!command) return null;
  const enabled =
    (typing && node.textEditing !== undefined) ||
    commandChain(node, ctx).some((c) => c.enabled(ctx));
  const state: ItemState = {
    text: command.title,
    enabled,
    accelerator: command.shortcut ? toAccelerator(command.shortcut) : null,
  };
  if (command.checked) state.checked = command.checked(ctx);
  return state;
}

/** The first command of a node's chain that can run now. */
export function commandToRun(
  node: Extract<MenuNode, { kind: 'command' }>,
  ctx: CommandCtx
): Command | null {
  return commandChain(node, ctx).find((c) => c.enabled(ctx)) ?? null;
}

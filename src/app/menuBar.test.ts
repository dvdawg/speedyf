// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { COMMANDS, commandById, type CommandCtx } from './commands';
import {
  commandToRun,
  itemState,
  menuBar,
  menuCommandIds,
  toAccelerator,
  type MenuNode,
} from './menuBar';
import { updateSettings } from '../stores/settings';

const noTab: CommandCtx = { tab: null };
type CommandNode = Extract<MenuNode, { kind: 'command' }>;

function findNode(nodes: readonly MenuNode[], id: string): CommandNode | null {
  for (const node of nodes) {
    if (node.kind === 'command' && node.id === id) return node;
    if (node.kind === 'submenu') {
      const found = findNode(node.items, id);
      if (found) return found;
    }
  }
  return null;
}

describe('the menu bar', () => {
  it('references only commands that exist, so a renamed command breaks here', () => {
    for (const kind of ['document', 'settings'] as const) {
      for (const id of menuCommandIds(menuBar(kind))) {
        expect(commandById(id), id).toBeDefined();
      }
    }
  });

  it('offers every registered command somewhere in a document window', () => {
    const bar = menuBar('document');
    const ids = menuCommandIds(bar);
    // Undo and Redo reach the library's own on the Home screen.
    const chained = ids.flatMap((id) => findNode(bar, id)?.before ?? []);
    const listed = new Set([...ids, ...chained]);
    const missing = COMMANDS.map((c) => c.id).filter((id) => !listed.has(id));
    expect(missing).toEqual([]);
  });

  it('lists each command once', () => {
    const ids = menuCommandIds(menuBar('document'));
    expect(ids.length).toBe(new Set(ids).size);
  });

  it('has the standard macOS menus in order', () => {
    const titles = menuBar('document').map((n) => (n.kind === 'submenu' ? n.text : n.kind));
    expect(titles).toEqual([
      'SpeedyF',
      'File',
      'Edit',
      'View',
      'Go',
      'Tools',
      'Document',
      'Library',
      'Window',
      'Help',
    ]);
    const settingsTitles = menuBar('settings').map((n) => (n.kind === 'submenu' ? n.text : ''));
    expect(settingsTitles).toEqual(['SpeedyF', 'File', 'Edit', 'Window']);
  });

  it('gives each menu key to one item only', () => {
    const seen = new Map<string, string>();
    for (const id of menuCommandIds(menuBar('document'))) {
      const shortcut = commandById(id)?.shortcut;
      const accelerator = shortcut ? toAccelerator(shortcut) : null;
      if (!accelerator) continue;
      expect(seen.get(accelerator), `${accelerator}: ${id}`).toBeUndefined();
      seen.set(accelerator, id);
    }
    expect(seen.get('CmdOrCtrl+,')).toBe('app.settings');
    expect(seen.get('CmdOrCtrl+Shift+W')).toBe('file.closeWindow');
    expect(seen.get('CmdOrCtrl+W')).toBe('file.closeTab');
  });
});

describe('toAccelerator', () => {
  it('turns registry chords into menu accelerators', () => {
    expect(toAccelerator('mod+shift+s')).toBe('CmdOrCtrl+Shift+S');
    expect(toAccelerator('mod+plus')).toBe('CmdOrCtrl+=');
    expect(toAccelerator('mod+minus')).toBe('CmdOrCtrl+-');
    expect(toAccelerator('mod+[')).toBe('CmdOrCtrl+[');
    expect(toAccelerator('mod+,')).toBe('CmdOrCtrl+,');
    expect(toAccelerator('mod+alt+c')).toBe('CmdOrCtrl+Alt+C');
    expect(toAccelerator('ctrl+shift+tab')).toBe('Ctrl+Shift+Tab');
  });

  it('leaves unmodified keys to the page, where text fields need them', () => {
    expect(toAccelerator('backspace')).toBeNull();
    expect(toAccelerator('pagedown')).toBeNull();
    expect(toAccelerator('c')).toBeNull();
  });
});

describe('item state', () => {
  const bar = menuBar('document');

  it('reads title, enabled state and key from the registry', () => {
    const save = itemState(findNode(bar, 'file.save')!, noTab)!;
    expect(save).toEqual({ text: 'Save', enabled: false, accelerator: 'CmdOrCtrl+S' });
    const open = itemState(findNode(bar, 'file.open')!, noTab)!;
    expect(open.enabled).toBe(commandById('file.open')!.enabled(noTab));
  });

  it('agrees with the registry for every item, with no document open', () => {
    for (const id of menuCommandIds(bar)) {
      const node = findNode(bar, id)!;
      if (node.before) continue;
      expect(itemState(node, noTab)!.enabled, id).toBe(commandById(id)!.enabled(noTab));
    }
  });

  it('checks the theme in effect', () => {
    updateSettings({ theme: 'dark' });
    expect(itemState(findNode(bar, 'settings.themeDark')!, noTab)!.checked).toBe(true);
    expect(itemState(findNode(bar, 'settings.themeLight')!, noTab)!.checked).toBe(false);
    updateSettings({ theme: 'system' });
    expect(itemState(findNode(bar, 'settings.themeSystem')!, noTab)!.checked).toBe(true);
    // A plain command has no checkmark at all.
    expect('checked' in itemState(findNode(bar, 'file.open')!, noTab)!).toBe(false);
  });

  it('enables Undo in a text field even with nothing to undo in the document', () => {
    const undo = findNode(bar, 'edit.undo')!;
    expect(itemState(undo, noTab, false)!.enabled).toBe(
      commandById('library.undo')!.enabled(noTab)
    );
    expect(itemState(undo, noTab, true)!.enabled).toBe(true);
  });

  it('runs the first command of a chain that can run', () => {
    const find = findNode(bar, 'search.findInDocument')!;
    expect(find.before).toEqual(['library.findInGraph']);
    const runnable = commandToRun(find, noTab);
    const expected = [
      commandById('library.findInGraph')!,
      commandById('search.findInDocument')!,
    ].find((c) => c.enabled(noTab));
    expect(runnable ?? undefined).toBe(expected);
  });
});

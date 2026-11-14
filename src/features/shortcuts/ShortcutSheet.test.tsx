// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@solidjs/testing-library';
import type * as extensionStoreModule from '../extensions/extensionStore';

vi.mock('../document/tabsController', () => ({
  closeOtherTabs: vi.fn(),
  closeTab: vi.fn(),
  closeTabsToTheRight: vi.fn(),
  cycleTab: vi.fn(),
  detachTab: vi.fn(),
  goHome: vi.fn(),
  openFromDialog: vi.fn(),
  jumpToTab: vi.fn(),
  reopenClosedTab: vi.fn(),
}));
vi.mock('../home/HomeScreen', () => ({ showHomePanel: vi.fn(), default: () => null }));

/** One extension with a bound command and a reader pane with a key. */
vi.mock('../extensions/extensionStore', async (importOriginal) => {
  const actual = await importOriginal<typeof extensionStoreModule>();
  const id = 'dev.example.notes';
  return {
    ...actual,
    extensionState: {
      extensions: [
        {
          id,
          name: 'Example Notes',
          commands: [{ id: `${id}/quote`, localId: 'quote', title: 'Quote Selection' }],
          selectionActions: [],
          documentTasks: [],
        },
      ],
    },
    extensionBindings: () => [
      {
        extensionId: id,
        command: 'quote',
        chord: { key: 'e', mod: true, shift: true, alt: false },
      },
    ],
    allReaderPanes: () => [
      { id: `${id}/pad`, localId: 'pad', title: 'Pad', extensionId: id, key: 'mod+shift+n' },
    ],
  };
});

const { COMMANDS, formatShortcut } = await import('../../app/commands');
const { installShortcuts, reservedChords, UNLISTED_SHORTCUTS } =
  await import('../../app/shortcuts');
const { canonicalChord, parseChord } = await import('../extensions/capabilities');
const { default: ShortcutSheet } = await import('./ShortcutSheet');
const { closeShortcutSheet, shortcutSheetOpen, toggleShortcutSheet } =
  await import('./shortcutSheetStore');

const isMac = /mac/i.test(navigator.userAgent);

afterEach(() => {
  cleanup();
  closeShortcutSheet();
});

function open() {
  toggleShortcutSheet();
  render(() => <ShortcutSheet />);
  return screen.getByRole('dialog', { name: 'Keyboard shortcuts' });
}

const rowTexts = (root: HTMLElement) =>
  [...root.querySelectorAll('.shortcut-row')].map((row) => row.textContent ?? '');

describe('ShortcutSheet', () => {
  it('lists every built-in command that has a shortcut, with its keys', () => {
    const sheet = open();
    const rows = rowTexts(sheet);
    const withShortcut = COMMANDS.filter((command) => command.shortcut);
    expect(withShortcut.length).toBeGreaterThan(10);
    for (const command of withShortcut) {
      const keys = formatShortcut(command.shortcut!, isMac);
      expect(
        rows.some((row) => row.includes(command.title) && row.includes(keys)),
        command.id
      ).toBe(true);
    }
    expect(rows.some((row) => row.includes('Keyboard Shortcuts'))).toBe(true);
    expect(rows.some((row) => row.includes('Command Palette'))).toBe(true);
  });

  it('lists extension bindings and pane keys under Extensions with the extension name', () => {
    const sheet = open();
    const extensions = within(sheet).getByRole('region', { name: 'Extensions' });
    const rows = rowTexts(extensions);
    const mod = isMac ? 'Cmd' : 'Ctrl';
    expect(rows).toContain(`Quote SelectionExample Notes${mod}+Shift+E`);
    expect(rows).toContain(`Toggle PadExample Notes${mod}+Shift+N`);
  });

  it('filters by title, keys or extension', () => {
    const sheet = open();
    const filter = within(sheet).getByRole('textbox', { name: 'Filter shortcuts' });
    fireEvent.input(filter, { target: { value: 'zoom' } });
    const rows = rowTexts(sheet);
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.every((row) => row.toLowerCase().includes('zoom'))).toBe(true);

    fireEvent.input(filter, { target: { value: 'example' } });
    expect(rowTexts(sheet)).toHaveLength(2);

    fireEvent.input(filter, { target: { value: 'nothing binds this' } });
    expect(within(sheet).getByText('No matching shortcuts')).toBeTruthy();
  });

  it('closes on Escape and on the chord that opened it', () => {
    let sheet = open();
    fireEvent.keyDown(sheet, { key: 'Escape' });
    expect(shortcutSheetOpen()).toBe(false);
    expect(screen.queryByRole('dialog')).toBeNull();

    cleanup();
    sheet = open();
    fireEvent.keyDown(sheet, { key: '/', metaKey: isMac, ctrlKey: !isMac });
    expect(shortcutSheetOpen()).toBe(false);
  });

  it('opens from the keymap on Cmd+/ (Ctrl+/)', () => {
    const uninstall = installShortcuts();
    try {
      fireEvent.keyDown(window, { key: '/', metaKey: isMac, ctrlKey: !isMac });
      expect(shortcutSheetOpen()).toBe(true);
      fireEvent.keyDown(window, { key: '/', metaKey: isMac, ctrlKey: !isMac });
      expect(shortcutSheetOpen()).toBe(false);
    } finally {
      uninstall();
    }
  });
});

it('documents only chords the keymap really reserves', () => {
  const reserved = reservedChords();
  for (const { spec } of UNLISTED_SHORTCUTS) {
    // The tab digits are one row for nine chords.
    const chord = parseChord(spec.replace('1-9', '1'))!;
    expect(reserved.has(canonicalChord(chord)), spec).toBe(true);
  }
  expect(reserved.has('mod+/')).toBe(true);
});

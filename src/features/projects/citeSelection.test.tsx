// @vitest-environment happy-dom
/**
 * Copy \cite{} for the Projects list's selection: the header button appears
 * with a selection, writes the keys in the list's order, each once, inside
 * the click, and pins them only after the write; the Library command and its
 * chord reach the same selection and collide with nothing else.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
  openFromDialog: vi.fn(async () => {}),
  goHome: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));
vi.mock('../home/HomeScreen', () => ({
  showHomePanel: vi.fn(),
  default: () => null,
}));

const { default: PaperList } = await import('./PaperList');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { citeCommand } = await import('./citeSelection');
const { commandById, runCommand, COMMANDS } = await import('../../app/commands');
const { installShortcuts, reservedChords } = await import('../../app/shortcuts');

const JARZYNSKI = 'Nonequilibrium equality for free energy differences';
const CROOKS = 'Entropy production fluctuation theorem';
const BOTH = '\\cite{jarzynski1997nonequilibrium,crooks1999entropy}';

const writeText = vi.fn(async (_text: string) => {});
const isMac = /mac/i.test(navigator.userAgent);
const rowOf = (text: string, at = 0) => screen.getAllByText(text)[at]!.closest('.paper-row')!;
const citeButton = () => screen.queryByRole('button', { name: 'Copy \\cite{}' });

beforeEach(async () => {
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  projectsStore.setScope({ kind: 'project', id: 'p1' });
  projectsStore.select(null);
  await projectsStore.initialize();
  await projectsStore.refresh();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('citeCommand', () => {
  it('keeps the order given, each key once, and drops empty keys', () => {
    expect(
      citeCommand([
        { id: 'a', citeKey: 'k1' },
        { id: 'b', citeKey: 'k2' },
        { id: 'c', citeKey: 'k1' },
        { id: 'd', citeKey: ' ' },
      ])
    ).toBe('\\cite{k1,k2}');
    expect(citeCommand([])).toBeNull();
  });
});

describe('Copy \\cite{} in the list header', () => {
  it('appears once a row is selected', async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    expect(citeButton()).toBeNull();
    fireEvent.click(rowOf(CROOKS));
    expect(citeButton()).toBeTruthy();
  });

  it("writes the keys in the list's order, not the clicking order, then pins", async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    fireEvent.click(rowOf(CROOKS));
    fireEvent.click(rowOf(JARZYNSKI), { metaKey: true });
    fireEvent.click(citeButton()!);
    expect(writeText).toHaveBeenCalledWith(BOTH);
    await waitFor(() => expect(engine.libraryPinKeys).toHaveBeenCalledWith(['a', 'b']));
    expect(writeText.mock.invocationCallOrder[0]!).toBeLessThan(
      vi.mocked(engine.libraryPinKeys).mock.invocationCallOrder[0]!
    );
    expect(await screen.findByRole('button', { name: 'Copied' })).toBeTruthy();
  });

  it('cites an item listed twice under All items once', async () => {
    projectsStore.setScope({ kind: 'all' });
    await projectsStore.refresh();
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    // Jarzynski is in both projects: rows a, b under the first, a again.
    fireEvent.click(rowOf(JARZYNSKI, 0));
    fireEvent.click(rowOf(JARZYNSKI, 1), { shiftKey: true });
    fireEvent.click(citeButton()!);
    expect(writeText).toHaveBeenCalledWith(BOTH);
  });

  it('pins nothing when the clipboard refuses', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    fireEvent.click(rowOf(CROOKS));
    fireEvent.click(citeButton()!);
    await waitFor(() => expect(projectsStore.state.error).toMatch(/clipboard/));
    expect(engine.libraryPinKeys).not.toHaveBeenCalled();
  });
});

describe('the Library command', () => {
  it('is registered on a chord nothing else binds', () => {
    const command = commandById('library.copyCiteSelection');
    expect(command?.title).toBe('Copy \\cite for Selection');
    expect(command?.group).toBe('Library');
    expect(command?.shortcut).toBe('mod+alt+c');
    expect(COMMANDS.filter((c) => c.shortcut === 'mod+alt+c')).toHaveLength(1);
    expect(reservedChords().has('mod+alt+c')).toBe(true);
    // Its siblings keep theirs.
    expect(reservedChords().has('mod+shift+c')).toBe(true);
    expect(commandById('library.copyCiteKey')?.shortcut).toBe('mod+shift+c');
  });

  it('copies the list selection on the Projects screen', async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    expect(runCommand('library.copyCiteSelection', { tab: null })).toBe(false);
    fireEvent.click(rowOf(JARZYNSKI));
    fireEvent.click(rowOf(CROOKS), { shiftKey: true });
    expect(runCommand('library.copyCiteSelection', { tab: null })).toBe(true);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(BOTH));
    await waitFor(() => expect(engine.libraryPinKeys).toHaveBeenCalledWith(['a', 'b']));
  });

  it('runs from Cmd/Ctrl+Option/Alt+C, matched on the physical key', async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    fireEvent.click(rowOf(CROOKS));
    const uninstall = installShortcuts();
    try {
      // Option+C types a cedilla on a Mac; the code is still KeyC.
      const event = new KeyboardEvent('keydown', {
        key: isMac ? 'ç' : 'c',
        code: 'KeyC',
        altKey: true,
        metaKey: isMac,
        ctrlKey: !isMac,
        cancelable: true,
      });
      window.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
      await waitFor(() => expect(writeText).toHaveBeenCalledWith('\\cite{crooks1999entropy}'));
    } finally {
      uninstall();
    }
  });
});

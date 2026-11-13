// @vitest-environment happy-dom
/**
 * Go Back / Go Forward: palette commands enabled only with history in that
 * direction, bound to Cmd+[ / Cmd+] (Ctrl elsewhere, plus Alt+Left/Right)
 * without taking a chord from anything else.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const cycleTab = vi.fn();
vi.mock('../features/document/tabsController', () => ({
  closeOtherTabs: vi.fn(),
  closeTab: vi.fn(),
  closeTabsToTheRight: vi.fn(),
  cycleTab: (d: number) => cycleTab(d),
  detachTab: vi.fn(),
  goHome: vi.fn(),
  openFromDialog: vi.fn(),
  jumpToTab: vi.fn(),
}));
vi.mock('../features/home/HomeScreen', () => ({ showHomePanel: vi.fn(), default: () => null }));

const { COMMANDS, commandById, runCommand } = await import('./commands');
const { installShortcuts, reservedChords } = await import('./shortcuts');
const { addTab, createTab, removeTab, setActiveId, tabsStore } =
  await import('../stores/tabsStore');
const nav = await import('../features/viewer/navHistory');

const isMac = /mac/i.test(navigator.userAgent);

function openTab(path: string) {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 1,
    path,
    name: path,
    pageCount: 10,
    sizes: Array.from({ length: 10 }, () => [612, 792, 0, 0, 0]),
    estimatedSize: [612, 792],
  });
  tab.viewport.setState({ zoom: 1, containerW: 1000, containerH: 800 });
  addTab(tab);
  setActiveId(tab.id);
  return tab;
}

function press(key: string, opts: { mod?: boolean; shift?: boolean; alt?: boolean } = {}) {
  const event = new KeyboardEvent('keydown', {
    key,
    metaKey: isMac && !!opts.mod,
    ctrlKey: !isMac && !!opts.mod,
    shiftKey: !!opts.shift,
    altKey: !!opts.alt,
    bubbles: true,
    cancelable: true,
  });
  window.dispatchEvent(event);
  return event;
}

beforeEach(() => {
  for (const t of [...tabsStore.state.tabs]) removeTab(t.id);
  setActiveId(null);
  nav.resetNavHistory();
});
afterEach(() => vi.clearAllMocks());

describe('Go Back / Go Forward commands', () => {
  it('are Navigate commands with the browser chords, shown in the palette', () => {
    const back = commandById('nav.back')!;
    const forward = commandById('nav.forward')!;
    expect(back).toMatchObject({ title: 'Go Back', group: 'Navigate', shortcut: 'mod+[' });
    expect(forward).toMatchObject({ title: 'Go Forward', group: 'Navigate', shortcut: 'mod+]' });
    expect(COMMANDS).toContain(back);
  });

  it('are enabled only when there is history in that direction', () => {
    expect(commandById('nav.back')!.enabled({ tab: null })).toBe(false);
    const tab = openTab('/a.pdf');
    const ctx = { tab };
    expect(commandById('nav.back')!.enabled(ctx)).toBe(false);
    expect(commandById('nav.forward')!.enabled(ctx)).toBe(false);
    expect(runCommand('nav.back', ctx)).toBe(false);

    nav.jumpToPage(tab, 7);
    tab.viewport.takeScrollRequest();
    expect(commandById('nav.back')!.enabled(ctx)).toBe(true);
    expect(commandById('nav.forward')!.enabled(ctx)).toBe(false);

    expect(runCommand('nav.back', ctx)).toBe(true);
    expect(commandById('nav.back')!.enabled(ctx)).toBe(false);
    expect(commandById('nav.forward')!.enabled(ctx)).toBe(true);
  });
});

describe('chords', () => {
  it('reserves Back/Forward chords, distinct from tab cycling', () => {
    const reserved = reservedChords();
    for (const chord of ['mod+[', 'mod+]', 'alt+ArrowLeft', 'alt+ArrowRight']) {
      expect(reserved.has(chord), chord).toBe(true);
    }
    // The shifted brackets still belong to tab cycling.
    expect(reserved.has('mod+shift+[')).toBe(true);
    expect(reserved.has('mod+shift+]')).toBe(true);
    const specs = COMMANDS.map((c) => c.shortcut).filter((s): s is string => s !== undefined);
    expect(new Set(specs).size).toBe(specs.length);
  });

  it('dispatches mod+[ and mod+] to Back and Forward, and leaves shifted ones to tabs', () => {
    const uninstall = installShortcuts();
    try {
      const tab = openTab('/a.pdf');
      nav.jumpToPage(tab, 7);
      tab.viewport.takeScrollRequest();
      tab.viewport.setState('scrollTop', 5000);

      expect(press('[', { mod: true }).defaultPrevented).toBe(true);
      expect(nav.canGoBack(tab)).toBe(false);
      expect(nav.canGoForward(tab)).toBe(true);

      press(']', { mod: true });
      expect(nav.canGoForward(tab)).toBe(false);
      expect(nav.canGoBack(tab)).toBe(true);

      press('[', { mod: true, shift: true });
      expect(cycleTab).toHaveBeenCalledWith(-1);
      expect(nav.canGoBack(tab)).toBe(true);

      if (!isMac) {
        press('ArrowLeft', { alt: true });
        expect(nav.canGoBack(tab)).toBe(false);
        press('ArrowRight', { alt: true });
        expect(nav.canGoBack(tab)).toBe(true);
      }
    } finally {
      uninstall();
    }
  });
});

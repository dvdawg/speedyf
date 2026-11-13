import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { engine } from '../../lib/transport/engine';
import {
  addTab,
  createTab,
  removeTab,
  setActiveId,
  tabsStore,
  activeTab,
} from '../../stores/tabsStore';
import {
  acceptOfferedTab,
  closeOtherTabs,
  closeTabsToTheRight,
  detachTab,
  initializeWindow,
  moveTabToWindow,
  closeTab,
  openInNewTabOrFocus,
  reopenClosedTab,
  restoreSession,
  type TabSnapshot,
} from './tabsController';
import { guardUnsaved } from './controller';
import type * as controllerModule from './controller';
import { saveSession } from '../../stores/sessionStore';
import { windowTransferPending } from '../windows/windowController';
import { showError } from '../../stores/modalStore';
import {
  CLOSED_TABS_LIMIT,
  clearClosedTabs,
  closedTabCount,
  popClosedTab,
  pushClosedTab,
} from './closedTabs';
import { VIEW_PADDING } from '../../stores/viewportStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'speedyf-1' }),
  getAllWindows: vi.fn().mockResolvedValue([{ label: 'main' }, { label: 'speedyf-1' }]),
}));
vi.mock('../../stores/modalStore', () => ({ showError: vi.fn() }));
vi.mock('./controller', async (importOriginal) => ({
  ...(await importOriginal<typeof controllerModule>()),
  guardUnsaved: vi.fn().mockResolvedValue(true),
}));

const storage = new Map<string, string>();
vi.stubGlobal('localStorage', {
  getItem: (k: string) => storage.get(k) ?? null,
  setItem: (k: string, v: string) => void storage.set(k, v),
  removeItem: (k: string) => void storage.delete(k),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() {
    return storage.size;
  },
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
  vi.mocked(showError).mockClear();
  vi.mocked(guardUnsaved).mockReset().mockResolvedValue(true);
  storage.clear();
  vi.spyOn(engine, 'setActiveDocument').mockResolvedValue();
  vi.spyOn(engine, 'startIndexing').mockResolvedValue();
  vi.spyOn(engine, 'close').mockResolvedValue();
});
afterEach(() => {
  for (const tab of [...tabsStore.state.tabs]) {
    tab.citationStore.dispose();
    removeTab(tab.id);
  }
  setActiveId(null);
  clearClosedTabs();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function openedTab() {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 42,
    path: '/paper.pdf',
    name: 'paper.pdf',
    pageCount: 2,
    sizes: [
      [600, 800, 0, 0, 0],
      [600, 800, 0, 0, 0],
    ],
    estimatedSize: [600, 800],
  });
  tab.documentStore.apply({ type: 'setForm', field: 'name', value: 'Unsaved answer' });
  tab.viewport.setState({ zoom: 1.5, fitMode: 'custom', currentPage: 1, scrollTop: 1500 });
  addTab(tab);
  setActiveId(tab.id);
  return tab;
}

it('keeps a dirty tab until acknowledged, then moves it without closing its PDF handle', async () => {
  const tab = openedTab();
  let acknowledge!: () => void;
  vi.mocked(invoke).mockImplementation((command) =>
    command === 'detach_tab'
      ? new Promise<void>((resolve) => {
          acknowledge = resolve;
        })
      : Promise.resolve(undefined)
  );
  const moving = detachTab(tab.id, [300, 400]);
  await vi.waitFor(() => expect(acknowledge).toBeDefined());
  expect(windowTransferPending()).toBe(true);
  expect(tabsStore.state.tabs).toHaveLength(1);
  const payload = vi
    .mocked(invoke)
    .mock.calls.find(([command]) => command === 'detach_tab')![1] as {
    snapshot: TabSnapshot;
    position: number[];
  };
  expect(payload.snapshot.document.state.formEdits.name).toBe('Unsaved answer');
  expect(payload.snapshot.viewport.zoom).toBe(1.5);
  expect(payload.snapshot.viewport.currentPage).toBe(1);
  expect(payload.position).toEqual([300, 400]);
  acknowledge();
  await moving;
  expect(tabsStore.state.tabs).toHaveLength(0);
  expect(engine.close).not.toHaveBeenCalled();
  expect(windowTransferPending()).toBe(false);
});

it('leaves the source editable with all edits when window creation fails', async () => {
  const tab = openedTab();
  vi.mocked(invoke).mockRejectedValueOnce(new Error('Window failed'));
  await detachTab(tab.id);
  expect(activeTab()?.id).toBe(tab.id);
  expect(activeTab()?.documentStore.state.formEdits.name).toBe('Unsaved answer');
  expect(windowTransferPending()).toBe(false);
  expect(showError).toHaveBeenCalled();
  expect(engine.close).not.toHaveBeenCalled();
});

it('adopts the transferred document without reopening or discarding its undo history', async () => {
  const source = openedTab();
  const snapshot: TabSnapshot = {
    document: source.documentStore.snapshot(),
    viewport: {
      zoom: 1.5,
      fitMode: 'custom',
      viewRotation: 90,
      currentPage: 1,
      editMode: true,
      searchOpen: false,
      formPanelOpen: false,
    },
    fraction: 0.4,
    search: { query: '', caseSensitive: false },
  };
  source.citationStore.dispose();
  removeTab(source.id);
  setActiveId(null);
  vi.mocked(invoke).mockImplementation((command) =>
    Promise.resolve(command === 'window_tab_snapshot' ? snapshot : undefined)
  );
  await initializeWindow();
  const target = activeTab()!;
  expect(target.documentStore.state.docId).toBe(42);
  expect(target.documentStore.state.formEdits.name).toBe('Unsaved answer');
  expect(target.viewport.state).toMatchObject(snapshot.viewport);
  expect(target.viewport.state.scrollRequest).toMatchObject({
    page: 1,
    fraction: 0.4,
    settle: true,
  });
  target.documentStore.undo();
  expect(target.documentStore.state.dirty).toBe(false);
  expect(vi.mocked(invoke).mock.calls.map(([command]) => command)).not.toContain('open_document');
});

it('starts an ordinary new window empty instead of restoring another window’s tabs', async () => {
  vi.mocked(invoke).mockResolvedValue(null);
  await initializeWindow();
  expect(tabsStore.state.tabs).toHaveLength(0);
  // It asks whether it was handed a tab, then whether it was opened to
  // restore a window from the last session — and opens nothing.
  expect(vi.mocked(invoke).mock.calls).toEqual([['window_tab_snapshot'], ['take_window_restore']]);
});

function metaFor(path: string, docId: number) {
  return {
    docId,
    path,
    name: path.slice(1),
    pageCount: 1,
    sizes: [[600, 800, 0, 0, 0]] as [number, number, number, number, number][],
    estimatedSize: [600, 800] as [number, number],
  };
}

function stubOpen(missing: string[] = []) {
  let next = 100;
  vi.spyOn(engine, 'getAnnotations').mockResolvedValue([]);
  return vi.spyOn(engine, 'open').mockImplementation(async (path) => {
    if (missing.includes(path)) throw new Error('not a readable file');
    return metaFor(path, next++);
  });
}

function plainTab(path: string, opts: { dirty?: boolean } = {}) {
  const tab = createTab();
  tab.documentStore.initFromMeta(metaFor(path, tabsStore.state.tabs.length + 1));
  if (opts.dirty) tab.documentStore.apply({ type: 'setForm', field: 'f', value: 'x' });
  addTab(tab);
  return tab;
}

it('reopens the tabs of a window restored from the last session, in order', async () => {
  const open = stubOpen(['/gone.pdf']);
  vi.mocked(invoke).mockImplementation((command) =>
    Promise.resolve(
      command === 'take_window_restore'
        ? { tabs: ['/a.pdf', '/gone.pdf', '/b.pdf'], activePath: '/b.pdf' }
        : null
    )
  );
  await initializeWindow();
  expect(open.mock.calls.map(([path]) => path)).toEqual(['/a.pdf', '/gone.pdf', '/b.pdf']);
  expect(tabsStore.state.tabs.map((t) => t.documentStore.state.path)).toEqual(['/a.pdf', '/b.pdf']);
  expect(activeTab()?.documentStore.state.path).toBe('/b.pdf');
  // A missing file is dropped silently at restore, never a modal per launch.
  expect(showError).not.toHaveBeenCalled();
  // Restoring never sends a document to another window.
  expect(vi.mocked(invoke).mock.calls.map(([c]) => c)).not.toContain('focus_window_for_path');
});

it('reopens every other window of the last session as a window of its own', async () => {
  stubOpen();
  saveSession({ version: 1, tabs: [{ path: '/main.pdf' }], activePath: '/main.pdf' });
  saveSession({ version: 1, tabs: [{ path: '/b.pdf' }], activePath: '/b.pdf' }, 'speedyf-4');
  saveSession({ version: 1, tabs: [{ path: '/c.pdf' }], activePath: null }, 'speedyf-7');
  await restoreSession();
  const restores = vi
    .mocked(invoke)
    .mock.calls.filter(([command]) => command === 'new_window')
    .map(([, args]) => args);
  expect(restores).toEqual([
    { restore: { tabs: ['/b.pdf'], activePath: '/b.pdf' } },
    { restore: { tabs: ['/c.pdf'], activePath: null } },
  ]);
  expect(tabsStore.state.tabs.map((t) => t.documentStore.state.path)).toEqual(['/main.pdf']);
});

it('raises the window that already has a document rather than opening a second copy', async () => {
  const open = stubOpen();
  vi.mocked(invoke).mockImplementation((command) =>
    Promise.resolve(command === 'focus_window_for_path' ? true : undefined)
  );
  expect(await openInNewTabOrFocus('/elsewhere.pdf')).toBe(true);
  expect(invoke).toHaveBeenCalledWith('focus_window_for_path', { path: '/elsewhere.pdf' });
  expect(open).not.toHaveBeenCalled();
  expect(tabsStore.state.tabs).toHaveLength(0);
});

it('opens a document here when no other window has it, or the registry is unreachable', async () => {
  const open = stubOpen();
  vi.mocked(invoke).mockImplementation((command) =>
    command === 'focus_window_for_path'
      ? Promise.reject(new Error('no registry'))
      : Promise.resolve(false)
  );
  expect(await openInNewTabOrFocus('/here.pdf')).toBe(true);
  expect(open).toHaveBeenCalledOnce();
  expect(activeTab()?.documentStore.state.path).toBe('/here.pdf');
  // The other windows learn at once what this one now has open.
  expect(invoke).toHaveBeenCalledWith('update_window_tabs', {
    tabs: ['/here.pdf'],
    activePath: '/here.pdf',
  });
});

it('closes the other tabs, stopping at the first unsaved prompt that is cancelled', async () => {
  const a = plainTab('/a.pdf');
  const b = plainTab('/b.pdf', { dirty: true });
  plainTab('/c.pdf');
  setActiveId(a.id);
  vi.mocked(guardUnsaved).mockResolvedValueOnce(false);
  await closeOtherTabs(a.id);
  // b's prompt was cancelled, so c is never reached.
  expect(tabsStore.state.tabs.map((t) => t.documentStore.state.path)).toEqual([
    '/a.pdf',
    '/b.pdf',
    '/c.pdf',
  ]);
  await closeOtherTabs(b.id);
  expect(tabsStore.state.tabs.map((t) => t.documentStore.state.path)).toEqual(['/b.pdf']);
});

it('closes only the tabs to the right of the one asked about', async () => {
  plainTab('/a.pdf');
  const b = plainTab('/b.pdf');
  plainTab('/c.pdf');
  plainTab('/d.pdf');
  await closeTabsToTheRight(b.id);
  expect(tabsStore.state.tabs.map((t) => t.documentStore.state.path)).toEqual(['/a.pdf', '/b.pdf']);
  await closeTabsToTheRight(b.id);
  expect(tabsStore.state.tabs).toHaveLength(2);
});

it('moves a dirty tab to an existing window with its edits, once that window accepts', async () => {
  const tab = openedTab();
  await moveTabToWindow(tab.id, 'speedyf-3');
  const call = vi.mocked(invoke).mock.calls.find(([command]) => command === 'send_tab_to_window');
  const payload = call![1] as { docId: number; target: string; snapshot: TabSnapshot };
  expect(payload.target).toBe('speedyf-3');
  expect(payload.docId).toBe(42);
  expect(payload.snapshot.document.state.formEdits.name).toBe('Unsaved answer');
  expect(tabsStore.state.tabs).toHaveLength(0);
  // The PDF handle moves with the tab; nothing is closed or saved.
  expect(engine.close).not.toHaveBeenCalled();
  expect(guardUnsaved).not.toHaveBeenCalled();
});

it('keeps the tab when the other window does not take it', async () => {
  const tab = openedTab();
  vi.mocked(invoke).mockImplementation((command) =>
    command === 'send_tab_to_window'
      ? Promise.reject(new Error('The other window did not take the tab'))
      : Promise.resolve(undefined)
  );
  await moveTabToWindow(tab.id, 'speedyf-3');
  expect(activeTab()?.id).toBe(tab.id);
  expect(activeTab()?.documentStore.state.formEdits.name).toBe('Unsaved answer');
  expect(showError).toHaveBeenCalled();
  expect(windowTransferPending()).toBe(false);
});

it('adopts a tab another window offers, and declines when it has no room', async () => {
  const source = openedTab();
  const snapshot: TabSnapshot = {
    document: source.documentStore.snapshot(),
    viewport: {
      zoom: 1,
      fitMode: 'custom',
      viewRotation: 0,
      currentPage: 0,
      editMode: false,
      searchOpen: false,
      formPanelOpen: false,
    },
    fraction: 0,
    search: { query: '', caseSensitive: false },
  };
  source.citationStore.dispose();
  removeTab(source.id);
  plainTab('/already-here.pdf');
  vi.mocked(invoke).mockImplementation((command) =>
    Promise.resolve(command === 'window_tab_snapshot' ? snapshot : undefined)
  );
  await acceptOfferedTab();
  expect(invoke).toHaveBeenCalledWith('accept_window_tab');
  expect(tabsStore.state.tabs).toHaveLength(2);
  expect(activeTab()?.documentStore.state.formEdits.name).toBe('Unsaved answer');

  for (let i = tabsStore.state.tabs.length; i < 30; i++) plainTab(`/filler-${i}.pdf`);
  vi.mocked(invoke).mockClear();
  await acceptOfferedTab();
  expect(vi.mocked(invoke).mock.calls).toEqual([['decline_window_tab']]);
});

it('declines an offered tab it fails to adopt, so the sender is not left waiting', async () => {
  vi.mocked(invoke).mockImplementation((command) =>
    command === 'window_tab_snapshot'
      ? Promise.reject(new Error('expired'))
      : Promise.resolve(undefined)
  );
  await acceptOfferedTab();
  expect(invoke).toHaveBeenCalledWith('decline_window_tab');
  expect(tabsStore.state.tabs).toHaveLength(0);
});

const paths = () => tabsStore.state.tabs.map((t) => t.documentStore.state.path);

it('reopens closed tabs most recent first, closing several pushing each', async () => {
  const open = stubOpen();
  const a = plainTab('/a.pdf');
  plainTab('/b.pdf', { dirty: true });
  plainTab('/c.pdf');
  // b's unsaved changes were discarded at the prompt; it still counts.
  await closeOtherTabs(a.id);
  expect(closedTabCount()).toBe(2);
  await closeTab(a.id);
  expect(closedTabCount()).toBe(3);

  expect(await reopenClosedTab()).toBe(true);
  expect(await reopenClosedTab()).toBe(true);
  expect(await reopenClosedTab()).toBe(true);
  expect(open.mock.calls.map(([path]) => path)).toEqual(['/a.pdf', '/c.pdf', '/b.pdf']);
  expect(paths()).toEqual(['/a.pdf', '/c.pdf', '/b.pdf']);
  expect(closedTabCount()).toBe(0);
  expect(await reopenClosedTab()).toBe(false);
});

it('keeps only the last twenty closed tabs', () => {
  for (let i = 1; i <= CLOSED_TABS_LIMIT + 5; i++) {
    pushClosedTab({
      path: `/${i}.pdf`,
      page: 0,
      fraction: 0,
      zoom: 1,
      fitMode: 'fit-width',
      viewRotation: 0,
    });
  }
  expect(closedTabCount()).toBe(CLOSED_TABS_LIMIT);
  expect(popClosedTab()?.path).toBe(`/${CLOSED_TABS_LIMIT + 5}.pdf`);
  let last = null;
  for (let entry = popClosedTab(); entry; entry = popClosedTab()) last = entry;
  expect(last?.path).toBe('/6.pdf');
});

it('skips a closed file that can no longer be opened, quietly, for the one before it', async () => {
  const open = stubOpen(['/gone.pdf']);
  const a = plainTab('/a.pdf');
  const gone = plainTab('/gone.pdf');
  await closeTab(a.id);
  await closeTab(gone.id);
  expect(await reopenClosedTab()).toBe(true);
  expect(open.mock.calls.map(([path]) => path)).toEqual(['/gone.pdf', '/a.pdf']);
  expect(paths()).toEqual(['/a.pdf']);
  expect(showError).not.toHaveBeenCalled();
  expect(closedTabCount()).toBe(0);
});

it('focuses a closed file that is open again, here or in another window, and spends its entry', async () => {
  const open = stubOpen();
  const a = plainTab('/a.pdf');
  const b = plainTab('/b.pdf');
  await closeTab(a.id);
  const again = plainTab('/a.pdf');
  setActiveId(b.id);
  expect(await reopenClosedTab()).toBe(true);
  expect(activeTab()?.id).toBe(again.id);
  expect(open).not.toHaveBeenCalled();
  expect(closedTabCount()).toBe(0);

  await closeTab(b.id);
  vi.mocked(invoke).mockImplementation((command) =>
    Promise.resolve(command === 'focus_window_for_path' ? true : undefined)
  );
  expect(await reopenClosedTab()).toBe(true);
  expect(invoke).toHaveBeenCalledWith('focus_window_for_path', { path: '/b.pdf' });
  expect(open).not.toHaveBeenCalled();
  expect(closedTabCount()).toBe(0);
});

it('does not count a tab detached or moved to another window as closed', async () => {
  const moved = openedTab();
  await moveTabToWindow(moved.id, 'speedyf-3');
  expect(tabsStore.state.tabs).toHaveLength(0);
  const detached = openedTab();
  await detachTab(detached.id);
  expect(tabsStore.state.tabs).toHaveLength(0);
  expect(closedTabCount()).toBe(0);
});

it('reopens a closed tab at its page, offset, zoom and rotation', async () => {
  stubOpen();
  const tab = plainTab('/r.pdf');
  setActiveId(tab.id);
  tab.viewport.setState({ zoom: 1.5, fitMode: 'custom', viewRotation: 90 });
  // Two pages, so there is a second one to have scrolled into.
  tab.documentStore.initFromMeta({
    ...metaFor('/r.pdf', 7),
    pageCount: 2,
    sizes: [
      [600, 800, 0, 0, 0],
      [600, 800, 0, 0, 0],
    ],
  });
  const layout = tab.zoom.layoutFor(1.5);
  tab.viewport.setState({
    currentPage: 1,
    scrollTop: layout.tops[1]! - VIEW_PADDING + layout.heights[1]! / 4,
  });
  await closeTab(tab.id);

  vi.spyOn(engine, 'open').mockResolvedValue({
    ...metaFor('/r.pdf', 8),
    pageCount: 2,
    sizes: [
      [600, 800, 0, 0, 0],
      [600, 800, 0, 0, 0],
    ],
  });
  await reopenClosedTab();
  const back = activeTab()!;
  expect(back.documentStore.state.path).toBe('/r.pdf');
  expect(back.viewport.state).toMatchObject({
    zoom: 1.5,
    fitMode: 'custom',
    viewRotation: 90,
    currentPage: 1,
  });
  expect(back.viewport.state.scrollRequest).toMatchObject({ page: 1, settle: true });
  const request = back.viewport.state.scrollRequest;
  expect(request?.kind === 'page' ? request.fraction : null).toBeCloseTo(0.25);
});

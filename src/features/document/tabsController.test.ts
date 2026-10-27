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
import { detachTab, initializeWindow, type TabSnapshot } from './tabsController';
import { windowTransferPending } from '../windows/windowController';
import { showError } from '../../stores/modalStore';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'speedyf-1' }),
  getAllWindows: vi.fn().mockResolvedValue([{ label: 'main' }, { label: 'speedyf-1' }]),
}));
vi.mock('../../stores/modalStore', () => ({ showError: vi.fn() }));

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
  vi.mocked(showError).mockClear();
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
  expect(invoke).toHaveBeenCalledExactlyOnceWith('window_tab_snapshot');
});

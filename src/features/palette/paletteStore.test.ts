import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const KEY = 'speedyf.palette.recent';

/** A stand-in for the webview's storage; `blocked` makes every call throw,
 * as a private window's can. */
function stubStorage(blocked = false) {
  const values = new Map<string, string>();
  const guard = () => {
    if (blocked) throw new Error('denied');
  };
  const storage = {
    getItem: (key: string) => (guard(), values.get(key) ?? null),
    setItem: (key: string, value: string) => (guard(), void values.set(key, value)),
    removeItem: (key: string) => void values.delete(key),
    clear: () => values.clear(),
  };
  vi.stubGlobal('localStorage', storage);
  return storage;
}

beforeEach(() => {
  stubStorage();
  vi.resetModules();
});
afterEach(() => vi.unstubAllGlobals());

const load = () => import('./paletteStore');

it('keeps the most recent first, once each, and ignores places', async () => {
  const store = await load();
  for (const id of ['file.save', 'view.zoomIn', 'tab:3', 'sec:2:Intro', 'file.save']) {
    store.recordRecent(id);
  }
  expect(store.recentCommands()).toEqual(['file.save', 'view.zoomIn']);
});

it('persists across a reload', async () => {
  const first = await load();
  first.recordRecent('view.fitPage');
  first.recordRecent('nav.back');
  vi.resetModules();
  const second = await load();
  expect(second.recentCommands()).toEqual(['nav.back', 'view.fitPage']);
  expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual(['nav.back', 'view.fitPage']);
});

it('bounds what it keeps', async () => {
  const store = await load();
  for (let i = 0; i < 30; i += 1) store.recordRecent(`command.${i}`);
  expect(store.recentCommands().length).toBeLessThanOrEqual(10);
  expect(store.recentCommands()[0]).toBe('command.29');
});

it('survives corrupt or unavailable storage', async () => {
  localStorage.setItem(KEY, '{not json');
  const corrupt = await load();
  expect(corrupt.recentCommands()).toEqual([]);

  vi.resetModules();
  localStorage.setItem(KEY, JSON.stringify(['ok', 7, null]));
  expect((await load()).recentCommands()).toEqual(['ok']);

  vi.resetModules();
  stubStorage(true);
  const blocked = await load();
  expect(blocked.recentCommands()).toEqual([]);
  expect(() => blocked.recordRecent('file.open')).not.toThrow();
  expect(blocked.recentCommands()).toEqual(['file.open']);
});

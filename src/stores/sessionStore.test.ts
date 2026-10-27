import { beforeEach, describe, expect, it, vi } from 'vitest';
import { loadSession, saveSession, removeWindowSession } from './sessionStore';

const map = new Map<string, string>();
const storage: Storage = {
  getItem: (k: string) => map.get(k) ?? null,
  setItem: (k: string, v: string) => void map.set(k, v),
  removeItem: (k: string) => void map.delete(k),
  clear: () => map.clear(),
  key: (index) => [...map.keys()][index] ?? null,
  get length() {
    return map.size;
  },
};
vi.stubGlobal('localStorage', storage);

beforeEach(() => {
  map.clear();
});

describe('loadSession', () => {
  it('returns an empty session when nothing is stored', () => {
    expect(loadSession()).toEqual({ version: 1, tabs: [], activePath: null });
  });

  it('falls back to empty on corrupted JSON', () => {
    storage.setItem('speedyf-session', 'not json{');
    expect(loadSession()).toEqual({ version: 1, tabs: [], activePath: null });
  });

  it('drops malformed tab entries but keeps well-formed ones', () => {
    storage.setItem(
      'speedyf-session',
      JSON.stringify({
        version: 1,
        tabs: [{ path: '/tmp/a.pdf' }, { notPath: 'x' }, { path: 42 }],
        activePath: '/tmp/a.pdf',
      })
    );
    expect(loadSession()).toEqual({
      version: 1,
      tabs: [{ path: '/tmp/a.pdf' }],
      activePath: '/tmp/a.pdf',
    });
  });

  it('ignores a non-string activePath', () => {
    storage.setItem('speedyf-session', JSON.stringify({ version: 1, tabs: [], activePath: 123 }));
    expect(loadSession().activePath).toBeNull();
  });
});

describe('saveSession / loadSession round-trip', () => {
  it('persists and reloads the same session', () => {
    saveSession({
      version: 1,
      tabs: [{ path: '/tmp/a.pdf' }, { path: '/tmp/b.pdf' }],
      activePath: '/tmp/b.pdf',
    });
    expect(loadSession()).toEqual({
      version: 1,
      tabs: [{ path: '/tmp/a.pdf' }, { path: '/tmp/b.pdf' }],
      activePath: '/tmp/b.pdf',
    });
  });
});

it('keeps live window sessions separate and collects their files on cold launch', () => {
  saveSession({ version: 1, tabs: [{ path: '/main.pdf' }], activePath: '/main.pdf' });
  const mainBefore = storage.getItem('speedyf-session');
  saveSession(
    {
      version: 1,
      tabs: [{ path: '/detached.pdf' }, { path: '/main.pdf' }],
      activePath: '/detached.pdf',
    },
    'speedyf-1'
  );
  expect(storage.getItem('speedyf-session')).toBe(mainBefore);
  expect(loadSession().tabs).toEqual([{ path: '/main.pdf' }, { path: '/detached.pdf' }]);
  expect(storage.getItem('speedyf-session:speedyf-1')).toBeNull();
});

it('does not restore a window explicitly closed while another remains', () => {
  saveSession({ version: 1, tabs: [{ path: '/closed.pdf' }], activePath: '/closed.pdf' });
  saveSession(
    { version: 1, tabs: [{ path: '/remaining.pdf' }], activePath: '/remaining.pdf' },
    'speedyf-1'
  );
  removeWindowSession('main');
  expect(loadSession()).toEqual({
    version: 1,
    tabs: [{ path: '/remaining.pdf' }],
    activePath: '/remaining.pdf',
  });
});

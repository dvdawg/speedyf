import { beforeEach, describe, expect, it, vi } from 'vitest';
import { loadSession, saveSession } from './sessionStore';

const map = new Map<string, string>();
const storage: Storage = {
  getItem: (k: string) => map.get(k) ?? null,
  setItem: (k: string, v: string) => void map.set(k, v),
  removeItem: (k: string) => void map.delete(k),
  clear: () => map.clear(),
  key: () => null,
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

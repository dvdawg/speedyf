import { beforeEach, describe, expect, it, vi } from 'vitest';
import { takeLegacySession } from './sessionStore';

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

const KEY = 'speedyf-session';

beforeEach(() => {
  map.clear();
});

describe('takeLegacySession', () => {
  it('returns null when there is nothing to migrate', () => {
    expect(takeLegacySession()).toBeNull();
  });

  it('reads the old session once and then removes it', () => {
    storage.setItem(
      KEY,
      JSON.stringify({ version: 1, tabs: [{ path: '/tmp/a.pdf' }], activePath: '/tmp/a.pdf' })
    );
    expect(takeLegacySession()).toEqual({
      tabs: [{ path: '/tmp/a.pdf' }],
      activePath: '/tmp/a.pdf',
    });
    // A migration that ran twice would resurrect tabs the user has since
    // closed, on every launch, forever.
    expect(storage.getItem(KEY)).toBeNull();
    expect(takeLegacySession()).toBeNull();
  });

  it('drops malformed tab entries but keeps well-formed ones', () => {
    storage.setItem(
      KEY,
      JSON.stringify({
        version: 1,
        tabs: [{ path: '/tmp/a.pdf' }, { notPath: 'x' }, { path: 42 }],
        activePath: '/tmp/a.pdf',
      })
    );
    expect(takeLegacySession()?.tabs).toEqual([{ path: '/tmp/a.pdf' }]);
  });

  it('ignores a non-string activePath', () => {
    storage.setItem(KEY, JSON.stringify({ tabs: [{ path: '/tmp/a.pdf' }], activePath: 123 }));
    expect(takeLegacySession()?.activePath).toBeNull();
  });

  it('treats corrupt or empty sessions as nothing to migrate, and clears them', () => {
    storage.setItem(KEY, 'not json{');
    expect(takeLegacySession()).toBeNull();
    expect(storage.getItem(KEY)).toBeNull();

    storage.setItem(KEY, JSON.stringify({ version: 1, tabs: [], activePath: null }));
    expect(takeLegacySession()).toBeNull();
  });
});

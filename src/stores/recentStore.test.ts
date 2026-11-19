import { beforeEach, describe, expect, it, vi } from 'vitest';

// recentStore reads localStorage at module-load time (same pattern as
// settings.ts), so the mock must exist before the static import below runs.
const { storage } = vi.hoisted(() => {
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
  return { storage };
});
vi.stubGlobal('localStorage', storage);

// engine.fileMetadata invokes Tauri's IPC bridge, which doesn't exist in a
// plain Node test — recordOpen already treats that as a soft failure
// (.catch(() => undefined)), so no mock is needed for the core behavior
// under test here (dedupe/cap/persist), only for the size-backfill path.
vi.mock('../lib/transport/engine', () => ({
  engine: { fileMetadata: vi.fn(async () => ({ sizeBytes: 4096, modifiedMs: 0 })) },
}));

const { recentStore, sanitizePosition } = await import('./recentStore');
const { engine } = await import('../lib/transport/engine');

beforeEach(() => {
  storage.clear();
  recentStore.clear();
  vi.mocked(engine.fileMetadata).mockClear();
});

describe('recordOpen', () => {
  it('adds a new entry to the front of the list', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    expect(recentStore.state.entries).toHaveLength(1);
    expect(recentStore.state.entries[0]!.path).toBe('/tmp/a.pdf');
    expect(recentStore.state.entries[0]!.sizeBytes).toBeNull();
  });

  it('dedupes by path, moving the re-opened file back to the front', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordOpen({ path: '/tmp/b.pdf', name: 'b.pdf' });
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    expect(recentStore.state.entries.map((e) => e.path)).toEqual(['/tmp/a.pdf', '/tmp/b.pdf']);
  });

  it('caps the list at 20 entries, newest first', () => {
    for (let i = 0; i < 25; i++) {
      recentStore.recordOpen({ path: `/tmp/${i}.pdf`, name: `${i}.pdf` });
    }
    expect(recentStore.state.entries).toHaveLength(20);
    expect(recentStore.state.entries[0]!.path).toBe('/tmp/24.pdf');
  });

  it('persists to localStorage', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    const raw = storage.getItem('speedyf-recent');
    expect(raw).toBeTruthy();
    expect(JSON.parse(raw!).entries[0].path).toBe('/tmp/a.pdf');
  });

  it('backfills size asynchronously via engine.fileMetadata', async () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    expect(recentStore.state.entries[0]!.sizeBytes).toBeNull();
    await Promise.resolve();
    await Promise.resolve();
    expect(recentStore.state.entries[0]!.sizeBytes).toBe(4096);
  });
});

describe('remove / clear', () => {
  it('removes a single entry by path', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordOpen({ path: '/tmp/b.pdf', name: 'b.pdf' });
    recentStore.remove('/tmp/a.pdf');
    expect(recentStore.state.entries.map((e) => e.path)).toEqual(['/tmp/b.pdf']);
  });

  it('clears the whole list and persists the empty state', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.clear();
    expect(recentStore.state.entries).toHaveLength(0);
    expect(JSON.parse(storage.getItem('speedyf-recent')!).entries).toEqual([]);
  });
});

describe('load', () => {
  it('falls back to an empty list when localStorage has corrupted JSON', async () => {
    storage.setItem('speedyf-recent', '{not json');
    vi.resetModules();
    const { recentStore: reloaded } = await import('./recentStore');
    expect(reloaded.state.entries).toEqual([]);
  });
});

describe('reading position', () => {
  it('remembers and returns a position', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordPosition('/tmp/a.pdf', { page: 7, fraction: 0.25 });
    expect(recentStore.positionFor('/tmp/a.pdf')).toEqual({ page: 7, fraction: 0.25 });
  });

  it('survives reopening the same file', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordPosition('/tmp/a.pdf', { page: 7, fraction: 0.25 });
    // reopening moves the entry to the front by rebuilding it
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    expect(recentStore.positionFor('/tmp/a.pdf')).toEqual({ page: 7, fraction: 0.25 });
  });

  it('ignores a path that is not in the list', () => {
    recentStore.recordPosition('/tmp/missing.pdf', { page: 1, fraction: 0 });
    expect(recentStore.positionFor('/tmp/missing.pdf')).toBeNull();
  });

  it('rejects a malformed position', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordPosition('/tmp/a.pdf', { page: -1, fraction: 0.5 });
    recentStore.recordPosition('/tmp/a.pdf', { page: 2, fraction: 1.5 });
    expect(recentStore.positionFor('/tmp/a.pdf')).toBeNull();
  });

  it('has no position for a file never read', () => {
    recentStore.recordOpen({ path: '/tmp/b.pdf', name: 'b.pdf' });
    expect(recentStore.positionFor('/tmp/b.pdf')).toBeNull();
  });
});

describe('zoom with the reading position', () => {
  it('remembers the zoom and fit mode a file was read at', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordPosition('/tmp/a.pdf', {
      page: 3,
      fraction: 0.5,
      zoom: 1.5,
      fitMode: 'custom',
    });
    expect(recentStore.positionFor('/tmp/a.pdf')).toEqual({
      page: 3,
      fraction: 0.5,
      zoom: 1.5,
      fitMode: 'custom',
    });
    const saved = JSON.parse(storage.getItem('speedyf-recent')!).entries[0].position;
    expect(saved).toEqual({ page: 3, fraction: 0.5, zoom: 1.5, fitMode: 'custom' });
  });

  it('records a zoom change at the same place', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordPosition('/tmp/a.pdf', {
      page: 3,
      fraction: 0.5,
      zoom: 1.5,
      fitMode: 'custom',
    });
    recentStore.recordPosition('/tmp/a.pdf', {
      page: 3,
      fraction: 0.5,
      zoom: 1.2,
      fitMode: 'fit-width',
    });
    expect(recentStore.positionFor('/tmp/a.pdf')).toMatchObject({
      zoom: 1.2,
      fitMode: 'fit-width',
    });
  });

  it('does not keep a stale zoom when a position without one replaces it', () => {
    recentStore.recordOpen({ path: '/tmp/a.pdf', name: 'a.pdf' });
    recentStore.recordPosition('/tmp/a.pdf', {
      page: 3,
      fraction: 0.5,
      zoom: 1.5,
      fitMode: 'custom',
    });
    recentStore.recordPosition('/tmp/a.pdf', { page: 4, fraction: 0 });
    expect(recentStore.positionFor('/tmp/a.pdf')).toEqual({ page: 4, fraction: 0 });
  });
});

describe('sanitizePosition', () => {
  it('reads a position saved before zoom was remembered', () => {
    expect(sanitizePosition({ page: 2, fraction: 0.1 })).toEqual({ page: 2, fraction: 0.1 });
  });

  it('clamps an out-of-range zoom into what the viewer supports', () => {
    expect(sanitizePosition({ page: 0, fraction: 0, zoom: 40, fitMode: 'custom' })).toMatchObject({
      zoom: 6,
    });
    expect(sanitizePosition({ page: 0, fraction: 0, zoom: 0.01, fitMode: 'custom' })).toMatchObject(
      { zoom: 0.2 }
    );
  });

  it('drops an unusable zoom but keeps the place', () => {
    expect(sanitizePosition({ page: 1, fraction: 0.5, zoom: 'big', fitMode: 'custom' })).toEqual({
      page: 1,
      fraction: 0.5,
    });
    expect(sanitizePosition({ page: 1, fraction: 0.5, zoom: 2, fitMode: 'fit-height' })).toEqual({
      page: 1,
      fraction: 0.5,
    });
  });

  it('keeps a fit mode without a usable zoom: the fit works it out', () => {
    expect(
      sanitizePosition({ page: 1, fraction: 0, zoom: Number.NaN, fitMode: 'fit-page' })
    ).toEqual({ page: 1, fraction: 0, fitMode: 'fit-page' });
  });

  it('rejects a position that is not one', () => {
    expect(sanitizePosition({ page: 1.5, fraction: 0 })).toBeNull();
    expect(sanitizePosition(null)).toBeNull();
  });

  it('keeps the entry when its saved position is damaged', async () => {
    storage.setItem(
      'speedyf-recent',
      JSON.stringify({
        version: 1,
        entries: [
          {
            path: '/tmp/a.pdf',
            name: 'a.pdf',
            lastOpened: 1,
            sizeBytes: null,
            position: { page: 2, fraction: 0.3, zoom: 99, fitMode: 'custom' },
          },
          {
            path: '/tmp/b.pdf',
            name: 'b.pdf',
            lastOpened: 1,
            sizeBytes: null,
            position: { page: 'x' },
          },
        ],
      })
    );
    vi.resetModules();
    const { recentStore: reloaded } = await import('./recentStore');
    expect(reloaded.positionFor('/tmp/a.pdf')).toEqual({
      page: 2,
      fraction: 0.3,
      zoom: 6,
      fitMode: 'custom',
    });
    expect(reloaded.state.entries.map((e) => e.path)).toEqual(['/tmp/a.pdf', '/tmp/b.pdf']);
    expect(reloaded.positionFor('/tmp/b.pdf')).toBeNull();
  });
});

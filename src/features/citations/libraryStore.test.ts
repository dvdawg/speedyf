/**
 * The library search's scope: everything by default, or one project, kept
 * for the session and sent to the engine with every query — and down to the
 * Tauri command as `projectId`, null for everything.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => ({ documents: [], truncated: false })),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));

const { invoke } = await import('@tauri-apps/api/core');
const { engine } = await import('../../lib/transport/engine');
const { libraryStore } = await import('./libraryStore');

afterEach(() => {
  libraryStore.setQuery('');
  libraryStore.setScope(null);
  vi.clearAllMocks();
});

describe('library search scope', () => {
  it('searches everything until a project is chosen, then that project', async () => {
    expect(libraryStore.state.scope).toBeNull();
    await libraryStore.runSearch('entropy');
    expect(invoke).toHaveBeenLastCalledWith('library_search', {
      query: 'entropy',
      caseSensitive: false,
      projectId: null,
    });

    libraryStore.setScope('p1');
    expect(libraryStore.state.scope).toBe('p1');
    await libraryStore.runSearch('entropy');
    expect(invoke).toHaveBeenLastCalledWith('library_search', {
      query: 'entropy',
      caseSensitive: false,
      projectId: 'p1',
    });
  });

  it('runs a search in progress again, at once, in the new scope', async () => {
    const search = vi.spyOn(engine, 'librarySearch');
    libraryStore.setQuery('entropy');
    libraryStore.setScope('p2');
    expect(search).toHaveBeenCalledWith('entropy', false, 'p2');
    search.mockRestore();
  });

  it('with no query, changing scope searches nothing', () => {
    libraryStore.setScope('p2');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('the extension API stays unscoped', async () => {
    await engine.librarySearch('entropy', false);
    expect(invoke).toHaveBeenLastCalledWith('library_search', {
      query: 'entropy',
      caseSensitive: false,
      projectId: null,
    });
  });
});

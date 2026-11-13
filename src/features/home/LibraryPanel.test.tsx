// @vitest-environment happy-dom
/**
 * The Library panel's scope picker: All items and each project, All items
 * by default, and the choice sent with the search.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('../projects/testFixtures');
  // One folder, fully indexed: the field is enabled.
  const engine = Object.assign(fakeEngine(), {
    libraryStatus: vi.fn(async () => ({
      roots: ['/papers'],
      indexed: 3,
      total: 3,
      scanning: false,
      textIndexed: 3,
      textTotal: 3,
    })),
  });
  return { engine, isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }));

const { default: LibraryPanel } = await import('./LibraryPanel');
const { engine } = await import('../../lib/transport/engine');
const { libraryStore } = await import('../citations/libraryStore');

afterEach(() => {
  cleanup();
  libraryStore.setQuery('');
  libraryStore.setScope(null);
  vi.clearAllMocks();
});

describe('LibraryPanel scope picker', () => {
  it('offers All items and every project, All items chosen', async () => {
    await libraryStore.refreshLibraryStatus();
    render(() => <LibraryPanel />);
    const picker = (await screen.findByLabelText('Search in')) as HTMLSelectElement;
    const options = Array.from(picker.options).map((option) => option.textContent);
    expect(options).toEqual(['All items', 'Noneq stat mech', 'Active matter']);
    expect(picker.value).toBe('');
  });

  it('searches the chosen project', async () => {
    await libraryStore.refreshLibraryStatus();
    render(() => <LibraryPanel />);
    const picker = (await screen.findByLabelText('Search in')) as HTMLSelectElement;
    fireEvent.input(screen.getByPlaceholderText('Search every paper...'), {
      target: { value: 'entropy' },
    });
    fireEvent.change(picker, { target: { value: 'p2' } });
    expect(libraryStore.state.scope).toBe('p2');
    await waitFor(() => expect(engine.librarySearch).toHaveBeenCalledWith('entropy', false, 'p2'));
    expect(screen.getByPlaceholderText('Search this project...')).toBeTruthy();

    fireEvent.change(picker, { target: { value: '' } });
    expect(libraryStore.state.scope).toBeNull();
    await waitFor(() =>
      expect(engine.librarySearch).toHaveBeenLastCalledWith('entropy', false, null)
    );
  });

  it('falls back to All items when the chosen project is gone', async () => {
    libraryStore.setScope('deleted-project');
    render(() => <LibraryPanel />);
    await waitFor(() => expect(libraryStore.state.scope).toBeNull());
  });
});

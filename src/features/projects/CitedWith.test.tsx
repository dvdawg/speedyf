// @vitest-environment happy-dom
/**
 * "Cited with these": the header line that says how many works the library
 * cites alongside the project (hidden at zero) and opens the list on them;
 * the list's two-way switch; Add, which goes through the membership
 * command; and the ghosts, drawn only while their toggle is on, joined to
 * their members by dashed lines, leaving every item where it was.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
  openFromDialog: vi.fn(async () => {}),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));
vi.mock('../home/HomeScreen', () => ({
  showHomePanel: vi.fn(),
  default: () => null,
}));

const { default: GraphView } = await import('./GraphView');
const { default: PaperList } = await import('./PaperList');
const { default: PaperDetail } = await import('./PaperDetail');
const { default: ProjectsScreen } = await import('./ProjectsScreen');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { fixture, memoryStorage, resetFixture } = await import('./testFixtures');

const store = projectsStore;
const withGhosts = (root: HTMLElement) =>
  Array.from(root.querySelectorAll<SVGGElement>('.graph-node.is-ghost.is-with'));
const missingGhosts = (root: HTMLElement) =>
  Array.from(root.querySelectorAll<SVGGElement>('.graph-node.is-ghost:not(.is-with)'));
const nodeFor = (root: HTMLElement, id: string) =>
  root.querySelector<SVGGElement>(`.graph-node[data-id="${id}"]`)!;

beforeEach(async () => {
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (query: string) =>
      ({
        matches: query.includes('reduce'),
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }) as unknown as MediaQueryList
  );
  vi.stubGlobal('localStorage', memoryStorage());
  resetFixture();
  store.setScope({ kind: 'project', id: 'p1' });
  store.leaveFocus();
  store.setMissing(false);
  store.setMissingMin(2);
  store.setCitedWith(false);
  store.setCitedWithMin(2);
  store.setMissingMode('by');
  store.select(null);
  store.setView('list');
  await store.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the header line', () => {
  it('says how many works are often cited with the project and opens the list', async () => {
    render(() => <ProjectsScreen />);
    const line = await screen.findByRole('button', {
      name: '2 works are often cited with this project',
    });
    expect(engine.libraryCocitedWorks).toHaveBeenLastCalledWith('p1', 2, 2);
    fireEvent.click(line);
    expect(store.state.view).toBe('graph');
    expect(store.state.missingMode).toBe('with');
    expect(store.state.missingList).toBe(true);
    const list = await screen.findByRole('listbox', { name: 'Cited with these' });
    expect(list.querySelectorAll('.missing-row')).toHaveLength(2);
  });

  it('is hidden when there are none, and outside a project', async () => {
    fixture.cocited = [];
    await store.loadCoCited();
    render(() => <ProjectsScreen />);
    await screen.findByText('2 items');
    expect(screen.queryByText(/often cited with this project/)).toBeNull();

    fixture.cocited = [...(await import('./testFixtures')).COCITED];
    store.setScope({ kind: 'all' });
    await waitFor(() => expect(store.state.coCited).toHaveLength(0));
    expect(screen.queryByText(/often cited with this project/)).toBeNull();
  });
});

describe('the list', () => {
  async function list() {
    store.openCitedWith();
    const view = render(() => (
      <>
        <PaperList onMoveFocus={() => {}} />
        <PaperDetail onMoveFocus={() => {}} />
      </>
    ));
    await screen.findByRole('listbox', { name: 'Cited with these' });
    return view;
  }

  it('switches between cited by and cited with', async () => {
    await list();
    const rows = Array.from(
      screen.getByRole('listbox', { name: 'Cited with these' }).querySelectorAll('.missing-row')
    );
    expect(rows[0]!.textContent).toContain('A cited-with item outside the project');
    expect(rows[0]!.textContent).toContain('cited with 2 of these');
    expect(rows[1]!.textContent).toContain('K. Sekimoto, Langevin equation');
    expect(rows[1]!.textContent).toContain('cited with 3 of these');
    expect(
      screen.getByRole('radio', { name: 'Cited with these' }).getAttribute('aria-checked')
    ).toBe('true');

    fireEvent.click(screen.getByRole('radio', { name: 'Cited by these' }));
    expect(store.state.missingMode).toBe('by');
    const missing = await screen.findByRole('listbox', { name: 'Missing works' });
    expect(engine.libraryMissingWorks).toHaveBeenLastCalledWith('p1', 2, undefined);
    expect(missing.querySelectorAll('.missing-row')).toHaveLength(2);
    expect(screen.queryByRole('listbox', { name: 'Cited with these' })).toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: 'Cited with these' }));
    await screen.findByRole('listbox', { name: 'Cited with these' });
  });

  it('adds a library item to the project; a missing work has no Add', async () => {
    await list();
    const adds = screen.getAllByRole('button', { name: 'Add' });
    expect(adds).toHaveLength(1);
    fireEvent.click(adds[0]!);
    expect(engine.librarySetMembership).toHaveBeenCalledWith('e', 'p1', true);
    // Add does not pick the row.
    expect(store.state.ghostKey).toBeNull();
  });

  it('picks a row into the detail column', async () => {
    await list();
    const rows = screen
      .getByRole('listbox', { name: 'Cited with these' })
      .querySelectorAll<HTMLElement>('.missing-row');
    fireEvent.click(rows[1]!);
    expect(store.state.ghostKey).toBe('with:doi:10.1/w');
    const panel = await screen.findByRole('region', { name: 'Cited with these' });
    expect(panel.textContent).toContain('Not in the library');
    expect(panel.textContent).toContain('Jar97');
    expect(screen.getByRole('button', { name: 'Open DOI' })).toBeTruthy();
  });
});

describe('the ghosts', () => {
  async function graph() {
    const view = render(() => (
      <>
        <GraphView />
        <PaperDetail onMoveFocus={() => {}} />
      </>
    ));
    await waitFor(() =>
      expect(view.container.querySelectorAll('.graph-node:not(.is-ghost)')).toHaveLength(3)
    );
    return view;
  }

  it('draw only for the toggles that are on, and move no item', async () => {
    const { container } = await graph();
    const before = ['a', 'b', 'g'].map((id) => nodeFor(container, id).getAttribute('transform'));
    expect(withGhosts(container)).toHaveLength(0);

    store.setCitedWith(true);
    await waitFor(() => expect(withGhosts(container)).toHaveLength(2));
    expect(missingGhosts(container)).toHaveLength(0);
    const after = ['a', 'b', 'g'].map((id) => nodeFor(container, id).getAttribute('transform'));
    expect(after).toEqual(before);
    const edges = Array.from(container.querySelectorAll('.graph-edge.is-with')).map(
      (e) => `${e.getAttribute('data-from')}>${e.getAttribute('data-to')}`
    );
    expect(edges).toEqual([
      'a>with:item:e',
      'b>with:item:e',
      'a>with:doi:10.1/w',
      'b>with:doi:10.1/w',
      'g>with:doi:10.1/w',
    ]);
    expect(container.querySelector('.graph-notes')!.getAttribute('title')).toContain(
      '2 works cited with >= 2 of these'
    );

    // Both kinds at once, when both are on.
    store.setMissing(true);
    await waitFor(() => expect(missingGhosts(container)).toHaveLength(2));
    expect(withGhosts(container)).toHaveLength(2);

    store.setCitedWith(false);
    await waitFor(() => expect(withGhosts(container)).toHaveLength(0));
    expect(missingGhosts(container)).toHaveLength(2);
  });

  it('picks a ghost into the detail column, where Add works', async () => {
    store.setCitedWith(true);
    const { container } = await graph();
    await waitFor(() => expect(withGhosts(container)).toHaveLength(2));
    const ghost = container.querySelector<SVGGElement>('.graph-node[data-with="item:e"]')!;
    expect(ghost.querySelector('text')!.textContent).toBe('Cited with 2 of these');
    fireEvent.click(ghost);
    expect(store.state.ghostKey).toBe('with:item:e');
    expect(ghost.classList.contains('is-selected')).toBe(true);
    const panel = await screen.findByRole('region', { name: 'Cited with these' });
    expect(panel.textContent).toContain('In the library, not in this project');
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    expect(engine.librarySetMembership).toHaveBeenCalledWith('e', 'p1', true);
  });
});

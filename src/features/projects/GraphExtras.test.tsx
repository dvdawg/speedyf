// @vitest-environment happy-dom
/**
 * The graph's two extras. Missing works: off by default; the toggle reads
 * the works cited by at least two items and draws them as dashed ghosts
 * joined to their citers, unlabelled until hovered; the stepper rereads;
 * a ghost's card and panel say what is known and open its DOI or arXiv
 * page; the notes lead to a plain list. Focus: Show in graph narrows the
 * graph to one item's neighbourhood, with a header, a depth switch and two
 * ways out, and keeps dragged positions under their own key.
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
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { updateSettings } = await import('../../stores/settings');
const { MISSING, memoryStorage } = await import('./testFixtures');
const { STORAGE_KEY } = await import('./graphPositions');
const { layoutGraph } = await import('./graphLayout');

const store = projectsStore;
const ghosts = (root: HTMLElement) =>
  Array.from(root.querySelectorAll<SVGGElement>('.graph-node.is-ghost'));
const ghostFor = (root: HTMLElement, key: string) =>
  root.querySelector<SVGGElement>(`.graph-node.is-ghost[data-ghost="${key}"]`)!;
const nodeFor = (root: HTMLElement, id: string) =>
  root.querySelector<SVGGElement>(`.graph-node[data-id="${id}"]`)!;

let storage: Storage;

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
  storage = memoryStorage();
  vi.stubGlobal('localStorage', storage);
  store.setScope({ kind: 'project', id: 'p1' });
  store.leaveFocus();
  store.setMissing(false);
  store.setMissingMin(2);
  store.select(null);
  store.setView('list');
  await store.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  updateSettings({ onlineLookup: false });
});

async function graph() {
  const view = render(() => <GraphView />);
  await waitFor(() => expect(view.container.querySelectorAll('.graph-node')).toHaveLength(3));
  return view;
}

async function missingOn(container: HTMLElement, count = 2) {
  fireEvent.click(screen.getByRole('button', { name: 'Missing' }));
  await waitFor(() => expect(ghosts(container)).toHaveLength(count));
}

describe('missing works', () => {
  it('is off by default, and the toggle reads works cited by at least two', async () => {
    const { container } = await graph();
    expect(ghosts(container)).toHaveLength(0);
    expect(engine.libraryMissingWorks).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Missing' }).getAttribute('aria-pressed')).toBe(
      'false'
    );

    await missingOn(container);
    expect(engine.libraryMissingWorks).toHaveBeenLastCalledWith('p1', 2, undefined);
    expect(screen.getByRole('button', { name: 'Missing' }).getAttribute('aria-pressed')).toBe(
      'true'
    );
    // Dashed ghosts, one edge per citer, in the rest style's class.
    expect(ghostFor(container, 'm1').classList.contains('is-ghost')).toBe(true);
    const edges = Array.from(container.querySelectorAll('.graph-edge.is-ghost'));
    expect(edges.map((e) => `${e.getAttribute('data-from')}>${e.getAttribute('data-to')}`)).toEqual(
      ['a>missing:m1', 'g>missing:m1', 'a>missing:m2', 'b>missing:m2', 'g>missing:m2']
    );
    // Calm at rest: no ghost is labelled.
    expect(ghosts(container).some((g) => g.classList.contains('is-label-on'))).toBe(false);
    expect(ghostFor(container, 'm1').querySelector('text')?.textContent).toBe(
      'Missing · cited by 2'
    );
    expect(screen.getByText(/2 missing works cited by ≥ 2/)).toBeTruthy();
  });

  it('rereads when the threshold steps', async () => {
    const { container } = await graph();
    await missingOn(container);
    fireEvent.click(screen.getByRole('button', { name: 'More citers' }));
    await waitFor(() => expect(ghosts(container)).toHaveLength(1));
    expect(engine.libraryMissingWorks).toHaveBeenLastCalledWith('p1', 3, undefined);
    expect(screen.getByText('cited by ≥ 3')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Fewer citers' }));
    fireEvent.click(screen.getByRole('button', { name: 'Fewer citers' }));
    await waitFor(() => expect(ghosts(container)).toHaveLength(3));
    expect(engine.libraryMissingWorks).toHaveBeenLastCalledWith('p1', 1, undefined);
    // 1 is the floor.
    expect(
      (screen.getByRole('button', { name: 'Fewer citers' }) as HTMLButtonElement).disabled
    ).toBe(true);
  });

  it('shows a card for a hovered ghost, with its reference and citers', async () => {
    const { container } = await graph();
    await missingOn(container);
    fireEvent.pointerEnter(ghostFor(container, 'm1'));
    const card = await waitFor(() => {
      const el = container.querySelector<HTMLElement>('.graph-card.is-ghost');
      expect(el).toBeTruthy();
      return el!;
    });
    expect(card.textContent).toContain(MISSING[0]!.text!);
    expect(card.textContent).toContain('2000 · doi 10.1103/PhysRevE.61.2361');
    expect(card.textContent).toContain('cited by: Jar97, ES02');
    expect(ghostFor(container, 'm1').classList.contains('is-label-on')).toBe(true);
    expect(ghostFor(container, 'm2').classList.contains('is-label-on')).toBe(false);
    expect(engine.libraryPaper).not.toHaveBeenCalledWith('missing:m1');
  });

  it('picks a ghost into the detail column, with honest actions', async () => {
    updateSettings({ onlineLookup: true });
    const { container } = render(() => (
      <>
        <GraphView />
        <PaperDetail onMoveFocus={() => {}} />
      </>
    ));
    await waitFor(() => expect(container.querySelectorAll('.graph-node')).toHaveLength(3));
    await missingOn(container);
    store.select('a');
    fireEvent.click(ghostFor(container, 'm1'));
    expect(store.state.ghostKey).toBe('m1');
    // Picked locally: the store's selection stays where it was.
    expect(store.state.selectedId).toBe('a');
    const panel = await screen.findByRole('region', { name: 'Missing work' });
    expect(panel.textContent).toContain(MISSING[0]!.text!);
    expect(ghostFor(container, 'm1').classList.contains('is-selected')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Open DOI' }));
    expect(engine.openExternalUrl).toHaveBeenCalledWith('https://doi.org/10.1103/PhysRevE.61.2361');
    expect(screen.queryByRole('button', { name: 'Open arXiv' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Copy reference' })).toBeTruthy();
    const lookUp = screen.getByRole('button', { name: 'Look up' }) as HTMLButtonElement;
    expect(lookUp.disabled).toBe(true);
    expect(screen.getByText('Add it to the library first')).toBeTruthy();

    fireEvent.click(ghostFor(container, 'm2'));
    fireEvent.click(await screen.findByRole('button', { name: 'Open arXiv' }));
    expect(engine.openExternalUrl).toHaveBeenLastCalledWith('https://arxiv.org/abs/1205.4176');

    // A citer selects that item, and the record comes back.
    fireEvent.click(screen.getByRole('button', { name: 'Cro99' }));
    expect(store.state.selectedId).toBe('b');
    expect(store.state.ghostKey).toBeNull();
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Missing work' })).toBeNull());
  });

  it('lists the missing works in the middle column from the notes', async () => {
    store.setView('graph');
    const { container } = render(() => (
      <>
        <PaperList onMoveFocus={() => {}} />
        <PaperDetail onMoveFocus={() => {}} />
      </>
    ));
    await waitFor(() => expect(container.querySelectorAll('.graph-node')).toHaveLength(3));
    await missingOn(container);
    fireEvent.click(screen.getByRole('button', { name: 'List missing (2)' }));
    const list = await screen.findByRole('listbox', { name: 'Missing works' });
    expect(container.querySelector('.graph-view')).toBeNull();
    const rows = Array.from(list.querySelectorAll<HTMLElement>('.missing-row'));
    expect(rows).toHaveLength(2);
    expect(rows[1]!.textContent).toContain('cited by Jar97, Cro99, ES02');
    fireEvent.click(rows[1]!);
    expect(store.state.ghostKey).toBe('m2');
    await screen.findByRole('button', { name: 'Open arXiv' });
    fireEvent.click(screen.getByText('Back to graph'));
    await waitFor(() => expect(container.querySelector('.graph-view')).toBeTruthy());
  });
});

describe('focus', () => {
  async function focused() {
    store.select('a');
    await vi.waitFor(() => expect(store.state.selected?.id).toBe('a'));
    const view = render(() => (
      <>
        <PaperList onMoveFocus={() => {}} />
        <PaperDetail onMoveFocus={() => {}} />
      </>
    ));
    fireEvent.click(await screen.findByRole('button', { name: 'Show in graph' }));
    await waitFor(() => expect(view.container.querySelector('.graph-focus-bar')).toBeTruthy());
    await waitFor(() => expect(view.container.querySelectorAll('.graph-node')).toHaveLength(3));
    return view;
  }

  it('Show in graph narrows the graph to the item, drawn selected in the middle', async () => {
    const { container } = await focused();
    expect(store.state.focus).toEqual({ paperId: 'a', depth: 1 });
    expect(store.state.view).toBe('graph');
    expect(engine.libraryGraph).toHaveBeenLastCalledWith('p1', { paperId: 'a', depth: 1 });
    expect(container.querySelector('.graph-focus-bar')!.textContent).toContain('Around Jar97');
    expect(nodeFor(container, 'a').classList.contains('is-selected')).toBe(true);
    expect(nodeFor(container, 'a').classList.contains('is-centre')).toBe(true);
  });

  it('switches depth, which rereads the graph and the missing works', async () => {
    const { container } = await focused();
    await missingOn(container);
    expect(engine.libraryMissingWorks).toHaveBeenLastCalledWith('p1', 2, {
      paperId: 'a',
      depth: 1,
    });
    fireEvent.click(screen.getByRole('radio', { name: '2 steps' }));
    await waitFor(() =>
      expect(engine.libraryGraph).toHaveBeenLastCalledWith('p1', { paperId: 'a', depth: 2 })
    );
    expect(engine.libraryMissingWorks).toHaveBeenLastCalledWith('p1', 2, {
      paperId: 'a',
      depth: 2,
    });
    expect(screen.getByRole('radio', { name: '2 steps' }).getAttribute('aria-checked')).toBe(
      'true'
    );
  });

  it('× leaves focus for the scope’s whole graph', async () => {
    const { container } = await focused();
    fireEvent.click(screen.getByRole('button', { name: 'Leave focus' }));
    expect(store.state.focus).toBeNull();
    await waitFor(() => expect(engine.libraryGraph).toHaveBeenLastCalledWith('p1'));
    expect(container.querySelector('.graph-focus-bar')).toBeNull();
  });

  it('Escape leaves focus', async () => {
    const { container } = await focused();
    fireEvent.keyDown(nodeFor(container, 'b'), { key: 'Escape' });
    expect(store.state.focus).toBeNull();
    await waitFor(() => expect(container.querySelector('.graph-focus-bar')).toBeNull());
  });

  it('keeps dragged positions under the focus’s own key', async () => {
    const { container } = await focused();
    const node = nodeFor(container, 'b');
    fireEvent.pointerDown(node, { button: 0, pointerId: 1, clientX: 100, clientY: 100 });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 160, clientY: 130 });
    fireEvent.pointerUp(window, { pointerId: 1, clientX: 160, clientY: 130 });
    const saved = JSON.parse(storage.getItem(STORAGE_KEY) ?? '{}') as Record<string, unknown>;
    expect(Object.keys(saved)).toEqual(['focus:a']);
  });

  it('pins a node where the layout starts it, with the rest around it', () => {
    // A star: the hub is pinned and ends up inside its neighbours' spread.
    const ids = ['hub', 'n1', 'n2', 'n3', 'n4', 'n5'];
    const { points } = layoutGraph(
      ids.map((id, i) => ({ id, year: 2000 + i, pinned: id === 'hub' })),
      ids.slice(1).map((id) => ({ from: id, to: 'hub' }))
    );
    const hub = points[0]!;
    const rest = points.slice(1);
    expect(hub.x).toBeGreaterThan(Math.min(...rest.map((p) => p.x)));
    expect(hub.x).toBeLessThan(Math.max(...rest.map((p) => p.x)));
    expect(hub.y).toBeGreaterThan(Math.min(...rest.map((p) => p.y)));
    expect(hub.y).toBeLessThan(Math.max(...rest.map((p) => p.y)));
  });
});

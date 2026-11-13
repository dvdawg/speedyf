// @vitest-environment happy-dom
/**
 * The graph's View popover: its defaults draw today's graph; each choice
 * does what it says (edges of the item in hand only, fewer labels or none
 * until hovered, edgeless items drawn under the layout); the choices are
 * kept across sessions; and it opens, closes and takes the keyboard like
 * a popover should.
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
vi.mock('../home/HomeScreen', () => ({
  showHomePanel: vi.fn(),
  default: () => null,
}));

const { default: GraphView } = await import('./GraphView');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { bigGraph, memoryStorage } = await import('./testFixtures');
const { DEFAULT_PREFS, PREFS_KEY, loadPrefs } = await import('./graphViewPrefs');

const store = projectsStore;
const libraryGraph = engine.libraryGraph as unknown as {
  mockResolvedValueOnce: (value: unknown) => void;
};
const items = (root: HTMLElement) =>
  Array.from(root.querySelectorAll<SVGGElement>('.graph-node:not(.is-ghost)'));
const nodeFor = (root: HTMLElement, id: string) =>
  root.querySelector<SVGGElement>(`.graph-node[data-id="${id}"]`);
const restEdges = (root: HTMLElement) =>
  Array.from(root.querySelectorAll('.graph-edge:not(.is-ghost)')).map(
    (e) => `${e.getAttribute('data-from')}>${e.getAttribute('data-to')}`
  );
const labelled = (root: HTMLElement) =>
  items(root).filter((n) => n.classList.contains('is-label-on'));

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
  store.setEdgeMode('all');
  store.setLabelMode('many');
  store.setUnlinkedMode('hide');
  store.setMissing(false);
  store.setCitedWith(false);
  store.select(null);
  storage.clear();
  await store.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function drawn(count = 3) {
  const view = render(() => <GraphView />);
  await waitFor(() => expect(items(view.container)).toHaveLength(count));
  return view;
}

const openView = () => fireEvent.click(screen.getByRole('button', { name: 'View' }));
const checked = (group: string) =>
  Array.from(screen.getByRole('radiogroup', { name: group }).querySelectorAll('[role="radio"]'))
    .filter((r) => r.getAttribute('aria-checked') === 'true')
    .map((r) => r.textContent);

describe('the View popover', () => {
  it('defaults to today: every edge, many labels, unlinked counted, no ghosts', async () => {
    expect(loadPrefs()).toEqual(DEFAULT_PREFS);
    const { container } = await drawn();
    openView();
    expect(checked('Edges')).toEqual(['All']);
    expect(checked('Labels')).toEqual(['Many']);
    expect(checked('Unlinked items')).toEqual(['Hide']);
    expect(screen.getByRole('button', { name: 'Missing' }).getAttribute('aria-pressed')).toBe(
      'false'
    );
    expect(screen.getByRole('button', { name: 'Cited with' }).getAttribute('aria-pressed')).toBe(
      'false'
    );
    expect(restEdges(container)).toHaveLength(4);
    expect(labelled(container)).toHaveLength(3);
    expect(nodeFor(container, 'c')).toBeNull();
    expect(screen.getByRole('button', { name: '1 item with no citations here' })).toBeTruthy();
  });

  it('opens on its first control, closes on Escape back to View, and on a press outside', async () => {
    await drawn();
    const button = screen.getByRole('button', { name: 'View' });
    openView();
    const dialog = screen.getByRole('dialog', { name: 'Graph view' });
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement).toBe(screen.getByRole('radio', { name: 'All' }));
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(button);
    expect(button.getAttribute('aria-expanded')).toBe('false');

    openView();
    // Inside stays open.
    fireEvent.pointerDown(screen.getByRole('radio', { name: 'Few' }));
    expect(screen.getByRole('dialog')).toBeTruthy();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('Edges: Selected draws only the edges of the item in hand', async () => {
    const { container } = await drawn();
    openView();
    fireEvent.click(screen.getByRole('radio', { name: 'Selected' }));
    await waitFor(() => expect(restEdges(container)).toEqual([]));
    store.select('b');
    await waitFor(() => expect(restEdges(container)).toEqual(['b>a', 'g>b']));
    const before = nodeFor(container, 'a')!.getAttribute('transform');
    fireEvent.click(screen.getByRole('radio', { name: 'All' }));
    await waitFor(() => expect(restEdges(container)).toHaveLength(4));
    expect(nodeFor(container, 'a')!.getAttribute('transform')).toBe(before);
  });

  it('Labels: Few keeps the most cited, Hover keeps none until hovered', async () => {
    libraryGraph.mockResolvedValueOnce(bigGraph(30));
    const { container } = await drawn(30);
    expect(labelled(container).length).toBeGreaterThan(3);
    openView();
    fireEvent.click(screen.getByRole('radio', { name: 'Few' }));
    await waitFor(() =>
      expect(labelled(container).map((n) => n.getAttribute('data-id'))).toEqual(['n0', 'n1', 'n2'])
    );
    fireEvent.click(screen.getByRole('radio', { name: 'Hover' }));
    await waitFor(() => expect(labelled(container)).toHaveLength(0));
    fireEvent.pointerEnter(nodeFor(container, 'n5')!);
    await waitFor(() =>
      expect(labelled(container).map((n) => n.getAttribute('data-id'))).toEqual(['n5'])
    );
  });

  it('Unlinked: Show draws the edgeless items under the rest, and no chip', async () => {
    const { container } = await drawn();
    const before = ['a', 'b', 'g'].map((id) => nodeFor(container, id)!.getAttribute('transform'));
    openView();
    fireEvent.click(screen.getByRole('radio', { name: 'Show' }));
    await waitFor(() => expect(nodeFor(container, 'c')).not.toBeNull());
    expect(screen.queryByText(/with no citations here/)).toBeNull();
    expect(['a', 'b', 'g'].map((id) => nodeFor(container, id)!.getAttribute('transform'))).toEqual(
      before
    );
    const y = (id: string) =>
      Number(
        /translate\([^ ]+ ([^)]+)\)/.exec(nodeFor(container, id)!.getAttribute('transform')!)![1]
      );
    expect(y('c')).toBeGreaterThan(Math.max(y('a'), y('b'), y('g')));

    fireEvent.click(screen.getByRole('radio', { name: 'Hide' }));
    await waitFor(() => expect(nodeFor(container, 'c')).toBeNull());
    expect(screen.getByRole('button', { name: '1 item with no citations here' })).toBeTruthy();
  });

  it('keeps the choices across sessions, and survives a broken store', async () => {
    await drawn();
    openView();
    fireEvent.click(screen.getByRole('radio', { name: 'Selected' }));
    fireEvent.click(screen.getByRole('radio', { name: 'Hover' }));
    fireEvent.click(screen.getByRole('radio', { name: 'Show' }));
    fireEvent.click(screen.getByRole('button', { name: 'Missing' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cited with' }));
    fireEvent.click(screen.getByRole('button', { name: 'More members' }));
    expect(loadPrefs()).toEqual({
      edges: 'selected',
      labels: 'hover',
      unlinked: 'show',
      missing: { on: true, min: 2 },
      citedWith: { on: true, min: 3 },
    });
    expect(engine.libraryCocitedWorks).toHaveBeenLastCalledWith('p1', 3, 2);

    storage.setItem(PREFS_KEY, '{not json');
    expect(loadPrefs()).toEqual(DEFAULT_PREFS);
    storage.setItem(PREFS_KEY, JSON.stringify({ edges: 'x', citedWith: { on: true, min: 0 } }));
    expect(loadPrefs().citedWith).toEqual({ on: true, min: 2 });
    expect(loadPrefs().edges).toBe('all');
  });
});

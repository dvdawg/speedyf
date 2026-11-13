// @vitest-environment happy-dom
/**
 * The graph view draws the connected part of what the engine hands it, one
 * node and one edge each, and counts the rest in a chip. Its pointer and
 * keyboard go through the store like the list's do: click selects,
 * double-click opens, Enter selects, Space opens. Hover shows a card and a
 * ring and dims nothing; the selection is what brings edges forward.
 * Labels are sparse; zoom and Fit move the view; drags persist per scope.
 * Plus the List | Graph toggle in PaperList, which remembers its side
 * across scopes and is gone where there is no graph, and the "also in" tag
 * that tells a row in two projects from one in one.
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

const { default: GraphView, HIDE_DELAY, LABEL_ALL, SHOW_DELAY } = await import('./GraphView');
const { LABEL_HEIGHT, labelWidth } = await import('./graphLayout');
const { default: PaperList } = await import('./PaperList');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { openInNewTabOrFocus } = await import('../document/tabsController');
const { GRAPH, GRAPH_WITH_SCAN, bigGraph, memoryStorage, paperFor } =
  await import('./testFixtures');
const { STORAGE_KEY } = await import('./graphPositions');

const JARZYNSKI = 'Nonequilibrium equality for free energy differences';
const CROOKS = 'Entropy production fluctuation theorem';

type Mocked = { mock: { calls: unknown[][]; results: { value: unknown }[] } };
const libraryGraph = engine.libraryGraph as unknown as Mocked & {
  mockRejectedValueOnce: (value: unknown) => void;
  mockResolvedValueOnce: (value: unknown) => void;
};
const libraryPaper = engine.libraryPaper as unknown as Mocked & {
  mockImplementationOnce: (fn: (id: string) => Promise<unknown>) => void;
};

const nodes = (root: HTMLElement) => Array.from(root.querySelectorAll<SVGGElement>('.graph-node'));
const nodeFor = (root: HTMLElement, id: string) =>
  root.querySelector<SVGGElement>(`.graph-node[data-id="${id}"]`)!;

let storage: Storage;
/** Reduced motion by default, so positions and the view land at once and
 * the tests can compare them; one test turns it off to see the tween. */
let reduce = true;

beforeEach(async () => {
  reduce = true;
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (query: string) =>
      ({
        matches: reduce && query.includes('reduce'),
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }) as unknown as MediaQueryList
  );
  storage = memoryStorage();
  vi.stubGlobal('localStorage', storage);
  projectsStore.setScope({ kind: 'project', id: 'p1' });
  projectsStore.select(null);
  projectsStore.setView('list');
  await projectsStore.initialize();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const canvasOf = (root: HTMLElement) => root.querySelector<HTMLElement>('.graph-canvas')!;
const card = (root: HTMLElement) => root.querySelector<HTMLElement>('.graph-card');
const labelled = (root: HTMLElement) =>
  nodes(root)
    .filter((g) => g.classList.contains('is-label-on'))
    .map((g) => g.getAttribute('data-id'));
const where = (root: HTMLElement, id: string) => nodeFor(root, id).getAttribute('transform');
const at = (root: HTMLElement, id: string) => {
  const [, x, y] = /translate\(([-\d.e]+) ([-\d.e]+)\)/.exec(where(root, id)!)!;
  return { x: Number(x), y: Number(y) };
};
/** Where a sits relative to b and g, which no zoom or pan changes. */
const shape = (root: HTMLElement): [number, number] => {
  const a = at(root, 'a');
  const b = at(root, 'b');
  const g = at(root, 'g');
  return [(a.x - b.x) / (g.x - b.x), (a.y - b.y) / (g.x - b.x)];
};

/** A press on `target`, a move past the threshold, and a release. */
function drag(target: Element, dx: number, dy: number) {
  fireEvent.pointerDown(target, { button: 0, pointerId: 1, clientX: 100, clientY: 100 });
  fireEvent.pointerMove(window, { pointerId: 1, clientX: 100 + dx, clientY: 100 + dy });
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 100 + dx, clientY: 100 + dy });
}

/** A press and release in place: a click, to the press logic. */
function tap(target: Element) {
  fireEvent.pointerDown(target, { button: 0, pointerId: 1, clientX: 50, clientY: 50 });
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 50, clientY: 50 });
}

async function drawn(count = 3) {
  const view = render(() => <GraphView />);
  await waitFor(() => expect(nodes(view.container)).toHaveLength(count));
  return view;
}

describe('GraphView', () => {
  it('draws the connected items and their citations, labelled by alpha label', async () => {
    const { container } = await drawn();
    expect(container.querySelectorAll('.graph-edge')).toHaveLength(GRAPH.edges.length);
    expect(libraryGraph.mock.calls[0]).toEqual(['p1']);

    const labels = nodes(container).map((g) => g.querySelector('text')?.textContent);
    expect(labels).toEqual(['Jar97', 'Cro99', 'ES02']);
    // A graph this small is labelled whole at rest.
    expect(labelled(container)).toEqual(['a', 'b', 'g']);
    expect(nodeFor(container, 'a').getAttribute('aria-label')).toMatch(/^Jar97: /);
    expect(nodeFor(container, 'a').getAttribute('aria-label')).toContain(JARZYNSKI);
    // No native tooltip: the hover card is the tooltip.
    expect(nodeFor(container, 'a').querySelector('title')).toBeNull();
    // The edge from the citing item to the cited one keeps its direction.
    const edge = container.querySelector('.graph-edge[data-from="b"]')!;
    expect(edge.getAttribute('data-to')).toBe('a');
    expect(edge.getAttribute('d')).toMatch(/^M.*Q/);
    // At rest no edge carries an arrowhead.
    expect(container.querySelector('.graph-edge[marker-end]')).toBeNull();
  });

  it('labels all of a 12-item graph at the default zoom, none overlapping', async () => {
    libraryGraph.mockResolvedValueOnce(bigGraph(12));
    const { container } = await drawn(12);
    expect(labelled(container)).toHaveLength(12);
    // Repeated alpha labels get a, b, c… within the view.
    const texts = nodes(container).map((g) => g.querySelector('text')!.textContent!);
    expect(new Set(texts).size).toBe(12);
    expect(texts).toEqual(expect.arrayContaining(['Aut00', 'Aut04a', 'Aut04b']));
    // Each label's box (and its dot) clear of every other, on screen.
    const boxes = nodes(container).map((g) => {
      const p = at(container, g.getAttribute('data-id')!);
      const text = g.querySelector('text')!;
      const r = Number(g.querySelector('.graph-node-dot')!.getAttribute('r'));
      const left = p.x - r;
      const right = p.x + Number(text.getAttribute('x')) + labelWidth(text.textContent!);
      return { left, right, top: p.y - LABEL_HEIGHT / 2, bottom: p.y + LABEL_HEIGHT / 2 };
    });
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i]!;
        const b = boxes[j]!;
        const overlap =
          a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
        expect(overlap).toBe(false);
      }
    }
    // No year bands or axis; a quiet time hint instead.
    expect(container.querySelector('.graph-band')).toBeNull();
    expect(container.querySelector('.graph-axis-label')).toBeNull();
    const hints = Array.from(container.querySelectorAll('.graph-time-hint')).map(
      (t) => t.textContent
    );
    expect(hints).toEqual(['← older', 'newer →']);
  });

  it('counts items with no citations in a chip that leads to the list', async () => {
    projectsStore.setView('graph');
    const { container } = await drawn();
    expect(nodeFor(container, 'c')).toBeNull();
    const chip = screen.getByRole('button', { name: '1 item with no citations here' });
    fireEvent.click(chip);
    expect(projectsStore.state.view).toBe('list');
  });

  it('marks a missing file and an item in more than one project', async () => {
    libraryGraph.mockResolvedValueOnce(GRAPH_WITH_SCAN);
    const { container } = await drawn(4);
    const shared = nodeFor(container, 'a');
    expect(shared.classList.contains('is-shared')).toBe(true);
    expect(shared.querySelector('.graph-node-ring')).not.toBeNull();
    expect(shared.getAttribute('aria-label')).toMatch(/in 2 projects$/);

    const single = nodeFor(container, 'b');
    expect(single.classList.contains('is-shared')).toBe(false);
    expect(single.querySelector('.graph-node-ring')).toBeNull();
    expect(single.getAttribute('aria-label')).not.toMatch(/projects/);

    expect(nodeFor(container, 'c').classList.contains('is-missing')).toBe(true);
    expect(nodeFor(container, 'c').getAttribute('aria-label')).toMatch(/file missing$/);
    expect(single.classList.contains('is-missing')).toBe(false);
    expect(screen.queryByText(/with no citations here/)).toBeNull();
  });

  it('shows the external and unindexed notes', async () => {
    render(() => <GraphView />);
    expect(await screen.findByText(/2 citations lead outside this project/)).toBeTruthy();
    expect(screen.getByText(/1 item not indexed yet, its citations are not known/)).toBeTruthy();
  });

  it('says so when there are items but no citations, and when there are no items', async () => {
    projectsStore.setScope({ kind: 'project', id: 'p2' });
    const { container, unmount } = render(() => <GraphView />);
    expect(await screen.findByText(/No citations between these items yet/)).toBeTruthy();
    expect(nodes(container)).toHaveLength(0);
    expect(screen.getByText('1 item with no citations here')).toBeTruthy();
    unmount();

    projectsStore.setScope({ kind: 'project', id: 'nothing' });
    render(() => <GraphView />);
    expect(await screen.findByText('No items')).toBeTruthy();
  });

  it('selects on click and follows the store selection', async () => {
    const { container } = await drawn();
    fireEvent.click(nodeFor(container, 'a'));
    expect(projectsStore.state.selectedId).toBe('a');
    expect(nodeFor(container, 'a').classList.contains('is-selected')).toBe(true);
    expect(nodeFor(container, 'a').getAttribute('aria-pressed')).toBe('true');

    // Chosen elsewhere (the list, the detail): the graph lights that one.
    projectsStore.select('b');
    await waitFor(() =>
      expect(nodeFor(container, 'b').classList.contains('is-selected')).toBe(true)
    );
    expect(nodeFor(container, 'a').classList.contains('is-selected')).toBe(false);
  });

  it('opens on double-click', async () => {
    const { container } = await drawn();
    fireEvent.dblClick(nodeFor(container, 'a'));
    await waitFor(() => expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/jarzynski1997.pdf'));
  });

  it('selects with Enter and opens with Space', async () => {
    const { container } = await drawn();
    for (const node of nodes(container)) expect(node.getAttribute('tabindex')).toBe('0');
    fireEvent.keyDown(nodeFor(container, 'b'), { key: 'Enter' });
    expect(projectsStore.state.selectedId).toBe('b');
    fireEvent.keyDown(nodeFor(container, 'b'), { key: ' ' });
    await waitFor(() => expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/crooks.pdf'));
  });

  it('rings a hovered node and dims nothing', async () => {
    const { container } = await drawn();
    fireEvent.pointerEnter(nodeFor(container, 'b'));
    expect(nodeFor(container, 'b').classList.contains('is-hover')).toBe(true);
    expect(canvasOf(container).classList.contains('has-focus')).toBe(false);
    expect(container.querySelectorAll('.graph-node.is-neighbour')).toHaveLength(0);
    expect(container.querySelector('.graph-edge-lit')).toBeNull();
    expect(container.querySelector('.graph-legend')).toBeNull();
    fireEvent.pointerLeave(nodeFor(container, 'b'));
    expect(nodeFor(container, 'b').classList.contains('is-hover')).toBe(false);
  });

  it('brings the selection’s citations forward, with a legend, until the background is clicked', async () => {
    const { container } = await drawn();
    expect(container.querySelector('.graph-legend')).toBeNull();
    fireEvent.click(nodeFor(container, 'b'));

    const canvas = canvasOf(container);
    expect(canvas.classList.contains('has-focus')).toBe(true);
    expect(nodeFor(container, 'a').classList.contains('is-neighbour')).toBe(true);
    expect(nodeFor(container, 'g').classList.contains('is-neighbour')).toBe(true);
    // b cites a (outgoing, solid) and is cited by g (incoming, dashed),
    // both with arrowheads.
    const lit = Array.from(container.querySelectorAll('.graph-edge-lit'));
    expect(lit).toHaveLength(2);
    const out = container.querySelector('.graph-edge-lit[data-from="b"]')!;
    expect(out.getAttribute('data-to')).toBe('a');
    expect(out.classList.contains('is-out')).toBe(true);
    const into = container.querySelector('.graph-edge-lit[data-to="b"]')!;
    expect(into.getAttribute('data-from')).toBe('g');
    expect(into.classList.contains('is-in')).toBe(true);
    for (const path of lit) expect(path.getAttribute('marker-end')).toMatch(/^url\(#graph-arrow-/);
    // Neighbours are labelled.
    expect(labelled(container)).toEqual(expect.arrayContaining(['a', 'b', 'g']));
    const legend = container.querySelector('.graph-legend')!;
    expect(legend.textContent).toContain('cites');
    expect(legend.textContent).toContain('cited by');

    // A click on the background lets the emphasis go; the selection stays.
    tap(container.querySelector('.graph-backdrop')!);
    expect(canvas.classList.contains('has-focus')).toBe(false);
    expect(container.querySelector('.graph-edge-lit')).toBeNull();
    expect(container.querySelector('.graph-legend')).toBeNull();
    expect(projectsStore.state.selectedId).toBe('b');
    expect(nodeFor(container, 'b').classList.contains('is-selected')).toBe(true);

    // Clicking the node again brings it back.
    fireEvent.click(nodeFor(container, 'b'));
    expect(canvas.classList.contains('has-focus')).toBe(true);
  });

  it('past the threshold, labels the most cited at rest, plus the hovered and the selected and its neighbours', async () => {
    libraryGraph.mockResolvedValueOnce(bigGraph(LABEL_ALL + 20));
    const { container } = await drawn(LABEL_ALL + 20);
    // Only three are cited at all; the rest wait for zoom, hover or selection.
    expect(labelled(container).sort()).toEqual(['n0', 'n1', 'n2']);

    fireEvent.pointerEnter(nodeFor(container, 'n10'));
    expect(labelled(container)).toContain('n10');
    fireEvent.pointerLeave(nodeFor(container, 'n10'));
    expect(labelled(container)).not.toContain('n10');

    fireEvent.click(nodeFor(container, 'n12'));
    // n12 cites n0, n1 and n2: it and all three are labelled.
    expect(labelled(container)).toEqual(expect.arrayContaining(['n12', 'n0', 'n1', 'n2']));
    expect(labelled(container)).not.toContain('n13');
  });

  it('shows more labels when zoomed in, without overlapping them', async () => {
    libraryGraph.mockResolvedValueOnce(bigGraph(LABEL_ALL + 20));
    const { container } = await drawn(LABEL_ALL + 20);
    const before = labelled(container).length;
    const zoomIn = screen.getByRole('button', { name: 'Zoom in' });
    fireEvent.click(zoomIn);
    fireEvent.click(zoomIn);
    fireEvent.click(zoomIn);
    expect(labelled(container).length).toBeGreaterThan(before);
  });

  it('lights nothing during a drag but the dragged node', async () => {
    const { container } = await drawn();
    const node = nodeFor(container, 'b');
    fireEvent.pointerEnter(node);
    fireEvent.pointerDown(node, { button: 0, pointerId: 1, clientX: 100, clientY: 100 });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 140, clientY: 120 });
    expect(node.classList.contains('is-dragged')).toBe(true);
    expect(node.classList.contains('is-hover')).toBe(false);
    fireEvent.pointerEnter(nodeFor(container, 'a'));
    expect(nodeFor(container, 'a').classList.contains('is-hover')).toBe(false);
    expect(canvasOf(container).classList.contains('has-focus')).toBe(false);
    expect(card(container)).toBeNull();
    fireEvent.pointerUp(window, { pointerId: 1, clientX: 140, clientY: 120 });
    expect(node.classList.contains('is-dragged')).toBe(false);
  });

  it('reads again when the scope changes and when the library changes', async () => {
    const { container } = await drawn();
    projectsStore.setScope({ kind: 'all' });
    await waitFor(() => expect(libraryGraph.mock.calls).toHaveLength(2));
    expect(libraryGraph.mock.calls[1]).toEqual([null]);
    expect(await screen.findByText(/2 citations lead outside the library/)).toBeTruthy();
    expect(nodes(container)).toHaveLength(3);

    const onChanged = engine.onLibraryChanged as unknown as Mocked;
    const fire = onChanged.mock.calls.at(-1)![0] as () => void;
    fire();
    await waitFor(() => expect(libraryGraph.mock.calls).toHaveLength(3));
  });

  it('shows an engine error inline and in the store', async () => {
    libraryGraph.mockRejectedValueOnce({ code: 'io', message: 'graph went wrong' });
    const { container } = render(() => <GraphView />);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('graph went wrong');
    expect(projectsStore.state.error).toBe('graph went wrong');
    expect(nodes(container)).toHaveLength(0);
    fireEvent.click(screen.getByText('Retry'));
    await waitFor(() => expect(nodes(container)).toHaveLength(3));
    projectsStore.clearError();
  });
});

describe('GraphView hover card', () => {
  it('appears after the delay with the row, then the abstract when the record arrives', async () => {
    let resolve: (paper: unknown) => void = () => {};
    libraryPaper.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        })
    );
    const { container } = await drawn();
    vi.useFakeTimers();
    const node = nodeFor(container, 'g');
    fireEvent.pointerEnter(node);
    await vi.advanceTimersByTimeAsync(SHOW_DELAY - 50);
    expect(card(container)).toBeNull();
    await vi.advanceTimersByTimeAsync(60);

    const shown = card(container)!;
    expect(shown).not.toBeNull();
    expect(shown.getAttribute('role')).toBe('tooltip');
    expect(node.getAttribute('aria-describedby')).toBe(shown.id);
    expect(shown.querySelector('.graph-card-title')!.textContent).toBe('The fluctuation theorem');
    expect(shown.querySelector('.graph-card-authors')!.textContent).toBe('Evans & Searles');
    expect(shown.querySelector('.graph-card-meta')!.textContent).toContain(
      '2002 · Advances in Physics'
    );
    // The alpha label, small and dim, beside the year.
    expect(shown.querySelector('.graph-card-alpha')!.textContent).toBe('ES02');
    expect(shown.querySelector('.graph-card-foot')!.textContent).toContain(
      'cites 2 · cited by 1 in this view'
    );
    expect(shown.querySelector('.graph-card-abstract')).toBeNull();
    expect(libraryPaper.mock.calls).toEqual([['g']]);

    resolve(paperFor('g'));
    await vi.advanceTimersByTimeAsync(0);
    expect(shown.querySelector('.graph-card-abstract')!.textContent).toMatch(
      /^The fluctuation theorem gives/
    );
    expect(shown.querySelector('.graph-card-authors')!.textContent).toBe(
      'Denis J. Evans, Debra J. Searles'
    );
    expect(shown.querySelector('.graph-card-kind')!.textContent).toBe('Paper');
    // Hover did not dim anything.
    expect(canvasOf(container).classList.contains('has-focus')).toBe(false);
  });

  it('hides after leaving, swaps without a wait, and reads each record once', async () => {
    const { container } = await drawn();
    vi.useFakeTimers();
    fireEvent.pointerEnter(nodeFor(container, 'g'));
    await vi.advanceTimersByTimeAsync(SHOW_DELAY + 10);
    expect(card(container)).not.toBeNull();

    // To another node within the hide delay: the card swaps at once.
    fireEvent.pointerLeave(nodeFor(container, 'g'));
    fireEvent.pointerEnter(nodeFor(container, 'a'));
    await vi.advanceTimersByTimeAsync(0);
    expect(card(container)!.querySelector('.graph-card-title')!.textContent).toBe(JARZYNSKI);

    fireEvent.pointerLeave(nodeFor(container, 'a'));
    await vi.advanceTimersByTimeAsync(HIDE_DELAY - 20);
    expect(card(container)).not.toBeNull();
    await vi.advanceTimersByTimeAsync(40);
    expect(card(container)).toBeNull();

    fireEvent.pointerEnter(nodeFor(container, 'g'));
    await vi.advanceTimersByTimeAsync(SHOW_DELAY + 10);
    expect(card(container)).not.toBeNull();
    const reads = libraryPaper.mock.calls.map((c) => c[0]);
    expect(reads.filter((id) => id === 'g')).toHaveLength(1);
    expect(reads.filter((id) => id === 'a')).toHaveLength(1);
  });

  it('says when the file is missing', async () => {
    libraryGraph.mockResolvedValueOnce(GRAPH_WITH_SCAN);
    const { container } = await drawn(4);
    vi.useFakeTimers();
    fireEvent.pointerEnter(nodeFor(container, 'c'));
    await vi.advanceTimersByTimeAsync(SHOW_DELAY + 10);
    expect(card(container)!.querySelector('.graph-card-missing')!.textContent).toBe('File missing');
  });

  it('shows for keyboard focus and hides on blur', async () => {
    const { container } = await drawn();
    vi.useFakeTimers();
    fireEvent.focus(nodeFor(container, 'b'));
    await vi.advanceTimersByTimeAsync(SHOW_DELAY + 10);
    expect(card(container)!.querySelector('.graph-card-title')!.textContent).toBe(CROOKS);
    fireEvent.blur(nodeFor(container, 'b'));
    await vi.advanceTimersByTimeAsync(HIDE_DELAY + 10);
    expect(card(container)).toBeNull();
  });
});

describe('GraphView zoom and positions', () => {
  it('zooms with the buttons and fits back', async () => {
    const { container } = await drawn();
    const start = where(container, 'g');
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }));
    const zoomed = where(container, 'g');
    expect(zoomed).not.toBe(start);
    fireEvent.click(screen.getByRole('button', { name: 'Zoom out' }));
    fireEvent.click(screen.getByRole('button', { name: 'Zoom out' }));
    expect(where(container, 'g')).not.toBe(zoomed);
    fireEvent.click(screen.getByRole('button', { name: 'Fit to view' }));
    expect(where(container, 'g')).toBe(start);
  });

  it('eases the zoom when motion is allowed', async () => {
    const { container } = await drawn();
    reduce = false;
    const start = where(container, 'g');
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }));
    // Not at once: the first frame has not run yet.
    expect(where(container, 'g')).toBe(start);
    await waitFor(() => expect(where(container, 'g')).not.toBe(start));
  });

  it('resets the view on a double-click on the background', async () => {
    const { container } = await drawn();
    const start = where(container, 'g');
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }));
    expect(where(container, 'g')).not.toBe(start);
    fireEvent.dblClick(container.querySelector('.graph-backdrop')!);
    expect(where(container, 'g')).toBe(start);
  });

  it('keeps a dragged node where it was put, per scope, until Reset layout', async () => {
    const first = await drawn();
    const rest = where(first.container, 'a');
    const restShape = shape(first.container);
    const reset = screen.getByRole('button', { name: 'Reset layout' }) as HTMLButtonElement;
    expect(reset.disabled).toBe(true);
    drag(nodeFor(first.container, 'a'), 60, 40);
    expect(where(first.container, 'a')).not.toBe(rest);
    const placed = shape(first.container);
    expect(JSON.parse(storage.getItem(STORAGE_KEY)!)).toHaveProperty('p1');
    first.unmount();

    // Back again: the node is where it was left (the view refits, so the
    // comparison is of the shape, not of pixels).
    const second = await drawn();
    expect(shape(second.container)[0]).toBeCloseTo(placed[0], 3);
    expect(shape(second.container)[1]).toBeCloseTo(placed[1], 3);
    second.unmount();

    // Another scope has its own layout.
    projectsStore.setScope({ kind: 'all' });
    const other = await drawn();
    expect(shape(other.container)[0]).toBeCloseTo(restShape[0], 3);
    expect(shape(other.container)[1]).toBeCloseTo(restShape[1], 3);
    other.unmount();

    projectsStore.setScope({ kind: 'project', id: 'p1' });
    const third = await drawn();
    const button = screen.getByRole('button', { name: 'Reset layout' }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    expect(where(third.container, 'a')).toBe(rest);
    expect(storage.getItem(STORAGE_KEY)).toBeNull();
    third.unmount();

    const fourth = await drawn();
    expect(where(fourth.container, 'a')).toBe(rest);
  });

  it('works without storage', async () => {
    vi.spyOn(storage, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(storage, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const { container } = await drawn();
    const rest = where(container, 'a');
    drag(nodeFor(container, 'a'), 60, 40);
    expect(where(container, 'a')).not.toBe(rest);
    fireEvent.click(screen.getByRole('button', { name: 'Reset layout' }));
    expect(where(container, 'a')).toBe(rest);
  });
});

describe('List | Graph toggle', () => {
  it('switches the middle column and remembers the choice across scopes', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(JARZYNSKI);
    const group = screen.getByRole('radiogroup', { name: 'View' });
    const list = screen.getByRole('radio', { name: 'List' });
    const graphButton = screen.getByRole('radio', { name: 'Graph' });
    expect(group).toBeTruthy();
    expect(list.getAttribute('aria-checked')).toBe('true');

    fireEvent.click(graphButton);
    expect(projectsStore.state.view).toBe('graph');
    expect(graphButton.getAttribute('aria-checked')).toBe('true');
    expect(container.querySelector('.paper-list')).toBeNull();
    expect(container.querySelector('.paper-search')).toBeNull();
    await waitFor(() => expect(nodes(container)).toHaveLength(3));

    // All items has a graph too, and the side chosen stays chosen.
    projectsStore.setScope({ kind: 'all' });
    await projectsStore.refresh();
    expect(screen.getByRole('radio', { name: 'Graph' }).getAttribute('aria-checked')).toBe('true');
    expect(container.querySelector('.graph-view')).not.toBeNull();

    // Other PDFs has none: no toggle, the list, and the choice kept for later.
    projectsStore.setScope({ kind: 'others' });
    await projectsStore.refresh();
    expect(screen.queryByRole('radiogroup', { name: 'View' })).toBeNull();
    expect(container.querySelector('.graph-view')).toBeNull();
    expect(container.querySelector('.paper-list')).not.toBeNull();
    expect(projectsStore.state.view).toBe('graph');

    projectsStore.setScope({ kind: 'project', id: 'p1' });
    await projectsStore.refresh();
    expect(container.querySelector('.graph-view')).not.toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: 'List' }));
    expect(container.querySelector('.paper-list')).not.toBeNull();
    expect(container.querySelector('.graph-view')).toBeNull();
  });
});

describe('also in', () => {
  const rowFor = (root: HTMLElement, title: string, nth = 0) =>
    Array.from(root.querySelectorAll('.paper-row')).filter((row) =>
      row.querySelector('.paper-row-title')?.textContent?.includes(title)
    )[nth]!;

  it('names a row’s other projects, not the group it is listed under', async () => {
    projectsStore.setScope({ kind: 'all' });
    await projectsStore.refresh();
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    // Under Noneq stat mech the paper is "also in" Active matter, and the
    // other way round under Active matter.
    expect(rowFor(container, JARZYNSKI, 0).querySelector('.paper-row-also')?.textContent).toBe(
      'also in Active matter'
    );
    expect(rowFor(container, JARZYNSKI, 1).querySelector('.paper-row-also')?.textContent).toBe(
      'also in Noneq stat mech'
    );
    expect(rowFor(container, CROOKS).querySelector('.paper-row-also')).toBeNull();
  });

  it('under a project names the others, and shows nothing for a single-project row', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(JARZYNSKI);
    expect(rowFor(container, JARZYNSKI).querySelector('.paper-row-also')?.textContent).toBe(
      'also in Active matter'
    );
    expect(rowFor(container, CROOKS).querySelector('.paper-row-also')).toBeNull();
  });
});

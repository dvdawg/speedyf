// @vitest-environment happy-dom
/**
 * Find in graph: the middle column's search field shows over the graph as
 * well, with one query for both views. In the graph it spotlights: the
 * matches (by title, authors, year, cite key, the label as drawn, and a
 * ghost's reference) keep their strength and labels, the rest fade, and
 * nothing moves. Enter and Shift+Enter step through the matches in reading
 * order with a counter; Escape clears, then hands the keyboard back; Cmd+F
 * reaches the field only while the graph shows.
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
  goHome: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));
vi.mock('../home/HomeScreen', () => ({
  showHomePanel: vi.fn(),
  default: () => null,
}));

const { default: PaperList } = await import('./PaperList');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { bigGraph, memoryStorage } = await import('./testFixtures');
const { filterRows, fold, queryTerms } = await import('./itemMatch');
const { installShortcuts } = await import('../../app/shortcuts');
const { updateSettings } = await import('../../stores/settings');

const store = projectsStore;
const JARZYNSKI = 'Nonequilibrium equality for free energy differences';
const isMac = /mac/i.test(navigator.userAgent);
const libraryGraph = engine.libraryGraph as unknown as {
  mockResolvedValueOnce: (value: unknown) => void;
};

const nodes = (root: HTMLElement) => Array.from(root.querySelectorAll<SVGGElement>('.graph-node'));
const nodeFor = (root: HTMLElement, id: string) =>
  root.querySelector<SVGGElement>(`.graph-node[data-id="${id}"]`)!;
const has = (root: HTMLElement, cls: string) =>
  nodes(root)
    .filter((g) => g.classList.contains(cls))
    .map((g) => g.getAttribute('data-id'));
const field = () => document.querySelector<HTMLInputElement>('.paper-search input')!;
const count = () => document.querySelector('.paper-find-count')?.textContent ?? null;
const type = (text: string) => fireEvent.input(field(), { target: { value: text } });
const key = (k: string, shiftKey = false) => fireEvent.keyDown(field(), { key: k, shiftKey });
const where = (root: HTMLElement) =>
  Object.fromEntries(
    nodes(root).map((g) => [g.getAttribute('data-id'), g.getAttribute('transform')])
  );
const at = (root: HTMLElement, id: string) => {
  const [, x, y] = /translate\(([-\d.e]+) ([-\d.e]+)\)/.exec(
    nodeFor(root, id).getAttribute('transform')!
  )!;
  return { x: Number(x), y: Number(y) };
};

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
  store.setScope({ kind: 'project', id: 'p1' });
  store.leaveFocus();
  store.setMissing(false);
  store.setLabelMode('many');
  store.setQuery('');
  store.select(null);
  store.setView('list');
  await store.initialize();
});

afterEach(() => {
  cleanup();
  store.setQuery('');
  store.setLabelMode('many');
  store.setMissing(false);
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function graph(count = 3) {
  store.setView('graph');
  const view = render(() => <PaperList onMoveFocus={() => {}} />);
  await waitFor(() => expect(nodes(view.container)).toHaveLength(count));
  return view;
}

describe('itemMatch', () => {
  it('folds case and diacritics and needs every word', () => {
    expect(fold('GÄRTNER Étienne')).toBe('gartner etienne');
    expect(queryTerms('  Fodor   2016 ')).toEqual(['fodor', '2016']);
    const rows = store.state.rows;
    expect(filterRows(rows, 'jarzynski 1997').map((r) => r.id)).toEqual(['a']);
    expect(filterRows(rows, 'jarzynski 1999')).toEqual([]);
    expect(filterRows(rows, 'Cro99').map((r) => r.id)).toEqual(['b']);
    expect(filterRows(rows, ' ')).toHaveLength(rows.length);
  });
});

describe('find in graph', () => {
  it('shows the field over the graph, sharing one query with the list', async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(JARZYNSKI);
    expect(field().getAttribute('placeholder')).toBe('Search title, authors, year, key...');
    type('crooks');
    fireEvent.click(screen.getByRole('radio', { name: 'Graph' }));
    expect(field().getAttribute('placeholder')).toBe('Find in graph...');
    expect(field().value).toBe('crooks');
    await waitFor(() => expect(count()).toBe('1 match'));
    type('evans');
    fireEvent.click(screen.getByRole('radio', { name: 'List' }));
    expect(field().value).toBe('evans');
    expect(store.state.query).toBe('evans');
    expect(count()).toBeNull();
  });

  it('matches by title, author, year, cite key and alpha label, diacritics folded', async () => {
    const { container } = await graph();
    const matches = (q: string) => {
      type(q);
      return has(container, 'is-match');
    };
    expect(matches('entropy production')).toEqual(['b']);
    expect(matches('CROOKS')).toEqual(['b']);
    expect(matches('jarzýnski')).toEqual(['a']);
    expect(matches('evans 2002')).toEqual(['g']);
    expect(matches('jarzynski1997nonequilibrium')).toEqual(['a']);
    expect(matches('es02')).toEqual(['g']);
    expect(matches('evans 1997')).toEqual([]);
    expect(count()).toBe('No match');
  });

  it('matches the label as drawn, suffix and all', async () => {
    libraryGraph.mockResolvedValueOnce(bigGraph(12));
    const { container } = await graph(12);
    type('Aut04');
    expect(has(container, 'is-match').sort()).toEqual(['n1', 'n3']);
    expect(count()).toBe('2 matches');
    type('aut04b');
    expect(has(container, 'is-match')).toEqual(['n3']);
  });

  it('fades the rest and labels the matches even with Labels on Hover', async () => {
    store.setLabelMode('hover');
    const { container } = await graph();
    expect(has(container, 'is-label-on')).toEqual([]);
    type('fluctuation');
    expect(has(container, 'is-match')).toEqual(['b', 'g']);
    expect(has(container, 'is-faded')).toEqual(['a']);
    expect(has(container, 'is-label-on').sort()).toEqual(['b', 'g']);
    expect(container.querySelector('.graph-canvas')!.classList.contains('is-finding')).toBe(true);
    // An edge between two matches stays; one to a faded node fades.
    const edge = (from: string, to: string) =>
      container.querySelector(`.graph-edge[data-from="${from}"][data-to="${to}"]`)!;
    expect(edge('g', 'b').classList.contains('is-faded')).toBe(false);
    expect(edge('b', 'a').classList.contains('is-faded')).toBe(true);

    type('');
    expect(has(container, 'is-faded')).toEqual([]);
    expect(has(container, 'is-match')).toEqual([]);
    expect(has(container, 'is-label-on')).toEqual([]);
    expect(container.querySelectorAll('.graph-edge.is-faded')).toHaveLength(0);
    expect(count()).toBeNull();
  });

  it('steps through the matches in reading order with Enter and Shift+Enter', async () => {
    const { container } = await graph();
    type('e');
    expect(count()).toBe('3 matches');
    const order = ['a', 'b', 'g'].sort((p, q) => at(container, p).x - at(container, q).x);
    key('Enter');
    expect(store.state.selectedId).toBe(order[0]);
    expect(count()).toBe('1 of 3');
    key('Enter');
    expect(store.state.selectedId).toBe(order[1]);
    expect(count()).toBe('2 of 3');
    key('Enter');
    key('Enter');
    expect(store.state.selectedId).toBe(order[0]);
    expect(count()).toBe('1 of 3');
    key('Enter', true);
    expect(store.state.selectedId).toBe(order[2]);
    expect(count()).toBe('3 of 3');
    // The other matches keep their strength under the selection.
    expect(has(container, 'is-faded')).toEqual([]);
    expect(container.querySelector('.graph-canvas')!.classList.contains('has-focus')).toBe(false);
  });

  it('marks the node a step landed on, and only while finding', async () => {
    const { container } = await graph();
    type('e');
    expect(has(container, 'is-stepped')).toEqual([]);
    key('Enter');
    expect(has(container, 'is-stepped')).toEqual([store.state.selectedId]);
    key('Enter');
    expect(has(container, 'is-stepped')).toEqual([store.state.selectedId]);
    type('');
    expect(has(container, 'is-stepped')).toEqual([]);
  });

  it('says No match, Escape clears, and a second Escape returns to the canvas', async () => {
    await graph();
    type('nothing like this');
    expect(count()).toBe('No match');
    key('Enter');
    expect(store.state.selectedId).toBeNull();
    field().focus();
    key('Escape');
    expect(store.state.query).toBe('');
    expect(field().value).toBe('');
    expect(document.activeElement).toBe(field());
    key('Escape');
    expect((document.activeElement as Element).classList.contains('graph-node')).toBe(true);
  });

  it('matches missing works by their reference and ids while they are shown', async () => {
    const { container } = await graph();
    type('seifert');
    expect(count()).toBe('No match');
    store.setMissing(true);
    await waitFor(() => expect(container.querySelectorAll('.graph-node.is-ghost')).toHaveLength(2));
    expect(has(container, 'is-match')).toEqual(['missing:m2']);
    expect(has(container, 'is-label-on')).toEqual(['missing:m2']);
    key('Enter');
    expect(store.state.ghostKey).toBe('m2');
    expect(count()).toBe('1 of 1');
    type('10.1103/PhysRevE.61.2361');
    expect(has(container, 'is-match')).toEqual(['missing:m1']);
  });

  it('moves no node: no relayout, dragged positions kept', async () => {
    const { container } = await graph();
    const g = nodeFor(container, 'g');
    fireEvent.pointerDown(g, { button: 0, pointerId: 1, clientX: 100, clientY: 100 });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 160, clientY: 140 });
    fireEvent.pointerUp(window, { pointerId: 1, clientX: 160, clientY: 140 });
    const before = where(container);
    const calls = (engine.libraryGraph as unknown as { mock: { calls: unknown[] } }).mock.calls
      .length;
    type('crooks');
    expect(where(container)).toEqual(before);
    type('fluctuation theorem');
    expect(where(container)).toEqual(before);
    type('');
    expect(where(container)).toEqual(before);
    expect(
      (engine.libraryGraph as unknown as { mock: { calls: unknown[] } }).mock.calls.length
    ).toBe(calls);
  });

  it('takes Cmd+F only while the Projects screen shows the graph', async () => {
    updateSettings({ launchScreen: 'projects' });
    const uninstall = installShortcuts();
    const press = () => {
      const event = new KeyboardEvent('keydown', {
        key: 'f',
        metaKey: isMac,
        ctrlKey: !isMac,
        cancelable: true,
        bubbles: true,
      });
      window.dispatchEvent(event);
      return event;
    };
    try {
      render(() => <PaperList onMoveFocus={() => {}} />);
      await screen.findByText(JARZYNSKI);
      // The list: Cmd+F is the reader's, which has nothing open to find in.
      const inList = press();
      expect(document.activeElement).not.toBe(field());
      expect(inList.defaultPrevented).toBe(true);

      fireEvent.click(screen.getByRole('radio', { name: 'Graph' }));
      await waitFor(() => expect(document.querySelectorAll('.graph-node')).toHaveLength(3));
      const inGraph = press();
      expect(inGraph.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(field());
    } finally {
      uninstall();
    }
  });
});

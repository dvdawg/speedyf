// @vitest-environment happy-dom
/**
 * Three quiet additions to the Projects screen: the Health row and its
 * findings, the New row and keyboard triage, and the list's sort. Each row
 * is there only while it holds something, and every fix goes through a
 * command that already exists.
 *
 * PaperList renders with Index, which reuses elements by position, so
 * every assertion re-queries the rows after the list changes.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HealthGroup, PaperRow } from '../../types/engine';

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

const { default: PaperList } = await import('./PaperList');
const { default: ProjectsColumn } = await import('./ProjectsColumn');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { fixture, resetFixture, ROWS, memoryStorage } = await import('./testFixtures');
const { loadSort, SORT_KEY } = await import('./listSortPrefs');
const { updateSettings } = await import('../../stores/settings');

const JARZYNSKI = 'Nonequilibrium equality for free energy differences';
const CROOKS = 'Entropy production fluctuation theorem';

const HEALTH: HealthGroup[] = [
  {
    kind: 'duplicate',
    label: 'Likely duplicates or versions',
    findings: [
      {
        kind: 'duplicate',
        paperId: 'a',
        paperIds: ['a', 'b'],
        title: JARZYNSKI,
        reason: `same work as "${CROOKS}"? same title`,
        suggestion: null,
      },
    ],
  },
  {
    kind: 'year',
    label: 'Year may be wrong',
    findings: [
      {
        kind: 'year',
        paperId: 'b',
        paperIds: ['b'],
        title: CROOKS,
        reason: '1999: cites work from 2021; year may be wrong',
        suggestion: '2021',
      },
    ],
  },
  {
    kind: 'weak',
    label: 'Weak metadata',
    findings: [
      {
        kind: 'weak',
        paperId: 'b',
        paperIds: ['b'],
        title: 'Untitled scan',
        reason: 'no authors',
        suggestion: null,
      },
    ],
  },
  {
    kind: 'title',
    label: 'Title mismatch',
    findings: [
      {
        kind: 'title',
        paperId: 'a',
        paperIds: ['a'],
        title: JARZYNSKI,
        reason: 'the first page says "Something else"',
        suggestion: 'Something else',
      },
    ],
  },
];

const options = () => screen.getAllByRole('option').map((row) => row.textContent);
const titles = (root: HTMLElement) =>
  Array.from(root.querySelectorAll('.paper-row .paper-row-title')).map((el) => el.textContent);
const rowFor = (root: HTMLElement, title: string) =>
  Array.from(root.querySelectorAll<HTMLElement>('.paper-row')).find((row) =>
    row.textContent?.includes(title)
  )!;

beforeEach(async () => {
  vi.stubGlobal('localStorage', memoryStorage());
  resetFixture();
  projectsStore.setSort('default');
  projectsStore.setScope({ kind: 'all' });
  projectsStore.select(null);
  await projectsStore.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  updateSettings({ onlineLookup: false });
});

describe('Health', () => {
  it('has no row while nothing is suspected', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    expect(screen.queryByText('Health')).toBeNull();
  });

  it('shows a row with the count below Other PDFs, and the groups in order', async () => {
    fixture.health = HEALTH;
    await projectsStore.refresh();
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Health');
    expect(options().slice(0, 5)).toEqual([
      'All items3',
      'Not filed1',
      'Other PDFs1',
      'Health4',
      'Noneq stat mech2',
    ]);
    fireEvent.click(screen.getByText('Health'));
    expect(projectsStore.state.scope.kind).toBe('health');
    cleanup();

    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    expect(
      Array.from(container.querySelectorAll('.paper-group-name')).map((el) => el.textContent)
    ).toEqual([
      'Likely duplicates or versions',
      'Year may be wrong',
      'Weak metadata',
      'Title mismatch',
    ]);
    expect(screen.getByText('no authors')).toBeTruthy();
    // No search and no sort over findings.
    expect(screen.queryByLabelText('Search items')).toBeNull();
    expect(container.querySelector('.paper-sort-btn')).toBeNull();
  });

  it('fixes through the existing commands, and Ignore records the finding', async () => {
    fixture.health = HEALTH;
    await projectsStore.refresh();
    projectsStore.setScope({ kind: 'health' });
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);

    fireEvent.click(screen.getByText('Use 2021'));
    await waitFor(() =>
      expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('b', { year: 2021 })
    );

    fireEvent.click(screen.getByText('Use page title'));
    await waitFor(() =>
      expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('a', { title: 'Something else' })
    );

    const weak = container.querySelector('[data-health="weak"]') as HTMLElement;
    expect(weak.textContent).not.toContain('Look up');
    const ignore = Array.from(weak.querySelectorAll('button')).find(
      (b) => b.textContent === 'Ignore'
    )!;
    fireEvent.click(ignore);
    await waitFor(() => expect(engine.libraryHealthIgnore).toHaveBeenCalledWith('b', 'weak'));

    fireEvent.click(screen.getByText('Merge...'));
    expect(await screen.findByRole('group', { name: 'Merge items' })).toBeTruthy();
  });

  it('offers Look up for weak metadata only when lookups are on', async () => {
    updateSettings({ onlineLookup: true });
    fixture.health = HEALTH;
    await projectsStore.refresh();
    projectsStore.setScope({ kind: 'health' });
    render(() => <PaperList onMoveFocus={() => {}} />);
    fireEvent.click(screen.getByText('Look up'));
    await waitFor(() => expect(engine.libraryLookup).toHaveBeenCalledWith('b', 'title'));
  });

  it('falls back to All items when the last finding goes', async () => {
    fixture.health = HEALTH;
    await projectsStore.refresh();
    projectsStore.setScope({ kind: 'health' });
    fixture.health = [];
    await projectsStore.refresh();
    expect(projectsStore.state.scope.kind).toBe('all');
  });
});

describe('New', () => {
  const NEWEST: PaperRow[] = [ROWS[2]!, ROWS[1]!, ROWS[0]!];

  it('shows a row with the count under All items only while there are new items', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    expect(screen.queryByText('New')).toBeNull();
    fixture.newItems = NEWEST;
    await projectsStore.refresh();
    await screen.findByText('New');
    expect(options().slice(0, 3)).toEqual(['All items3', 'New3', 'Not filed1']);
    fixture.newItems = [];
    await projectsStore.refresh();
    await waitFor(() => expect(screen.queryByText('New')).toBeNull());
  });

  it('lists newest first whatever the sort, and shows digit hints on projects', async () => {
    fixture.newItems = NEWEST;
    await projectsStore.refresh();
    projectsStore.setSort('title');
    projectsStore.setScope({ kind: 'new' });
    await projectsStore.loadRows();
    const { container } = render(() => (
      <>
        <ProjectsColumn onMoveFocus={() => {}} />
        <PaperList onMoveFocus={() => {}} />
      </>
    ));
    expect(titles(container)).toEqual(['Untitled scan', CROOKS, JARZYNSKI]);
    expect(
      Array.from(container.querySelectorAll('.projects-row-hint')).map((el) => el.textContent)
    ).toEqual(['1', '2']);
    expect(container.querySelector('.paper-sort-btn:not([title^="Mark"])')).toBeNull();
  });

  it('files by digit, marks done by D or Done, and opens on Enter', async () => {
    fixture.newItems = NEWEST;
    await projectsStore.refresh();
    projectsStore.setScope({ kind: 'new' });
    await projectsStore.loadRows();
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    const list = screen.getByRole('listbox', { name: 'Items' });

    fireEvent.keyDown(list, { key: '2' });
    await waitFor(() => expect(engine.libraryMarkReviewed).toHaveBeenCalledWith(['c']));
    expect(engine.librarySetMembership).toHaveBeenCalledWith('c', 'p2', true);

    // Fewer projects than the digit: nothing happens.
    fireEvent.keyDown(list, { key: '7' });
    expect(engine.librarySetMembership).toHaveBeenCalledTimes(1);

    vi.mocked(engine.libraryMarkReviewed).mockClear();
    fireEvent.keyDown(list, { key: 'd' });
    await waitFor(() => expect(engine.libraryMarkReviewed).toHaveBeenCalledWith(['c']));
    expect(engine.librarySetMembership).toHaveBeenCalledTimes(1);

    // The engine has the first one triaged now; the list re-reads.
    fixture.newItems = NEWEST.slice(1);
    await projectsStore.refresh();
    expect(titles(container)).toEqual([CROOKS, JARZYNSKI]);
    vi.mocked(engine.libraryMarkReviewed).mockClear();
    const done = Array.from(rowFor(container, CROOKS).querySelectorAll('button')).find(
      (b) => b.textContent === 'Done'
    )!;
    fireEvent.click(done);
    await waitFor(() => expect(engine.libraryMarkReviewed).toHaveBeenCalledWith(['b']));
  });

  it('falls back to All items when the last new item is triaged', async () => {
    fixture.newItems = NEWEST;
    await projectsStore.refresh();
    projectsStore.setScope({ kind: 'new' });
    fixture.newItems = [];
    await projectsStore.refresh();
    expect(projectsStore.state.scope.kind).toBe('all');
  });
});

describe('Sort', () => {
  it('cycles Default, Year, Title, Cited here and keeps the choice', async () => {
    render(() => <PaperList onMoveFocus={() => {}} />);
    const button = () => screen.getByText(/^Sort:/);
    expect(button().textContent).toBe('Sort: Default');
    fireEvent.click(button());
    expect(button().textContent).toBe('Sort: Year');
    expect(localStorage.getItem(SORT_KEY)).toBe('year');
    expect(loadSort()).toBe('year');
    fireEvent.click(button());
    expect(button().textContent).toBe('Sort: Title');
    fireEvent.click(button());
    expect(button().textContent).toBe('Sort: Cited here');
    fireEvent.click(button());
    expect(button().textContent).toBe('Sort: Default');
  });

  it('orders by how many items here cite each, and says so in the meta line', async () => {
    fixture.citedHere = { a: 5, b: 1 };
    projectsStore.setScope({ kind: 'project', id: 'p1' });
    await projectsStore.loadRows();
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    expect(container.textContent).not.toContain('cited by');
    projectsStore.setSort('cited');
    await waitFor(() => expect(container.textContent).toContain('cited by 5 here'));
    expect(engine.libraryCitedHere).toHaveBeenLastCalledWith({ project: 'p1', query: '' });
    expect(titles(container)).toEqual([JARZYNSKI, CROOKS]);
    expect(rowFor(container, CROOKS).textContent).toContain('cited by 1 here');

    fixture.citedHere = { b: 3 };
    await projectsStore.refresh();
    await waitFor(() => expect(titles(container)).toEqual([CROOKS, JARZYNSKI]));
  });

  it('sorts within each group under All items', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    // Default: the engine's order, Jarzynski before Crooks in Noneq.
    expect(titles(container).slice(0, 2)).toEqual([JARZYNSKI, CROOKS]);
    projectsStore.setSort('title');
    await waitFor(() => expect(titles(container).slice(0, 2)).toEqual([CROOKS, JARZYNSKI]));
    expect(
      Array.from(container.querySelectorAll('.paper-group-name')).map((el) => el.textContent)
    ).toEqual(['Noneq stat mech', 'Active matter', 'Not in a project']);
  });
});

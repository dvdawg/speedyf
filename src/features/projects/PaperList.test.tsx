// @vitest-environment happy-dom
/**
 * The middle column's three promises: under All papers the list is grouped
 * by project, in project order, with a paper in two projects shown twice and
 * a group that folds; under a project it is flat; and the keyboard reaches
 * and opens a paper by the file that is still on disk. Plus the hand-off to
 * the full-text search, which has to leave three things set at once.
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

const { default: PaperList } = await import('./PaperList');
const { default: ProjectsColumn } = await import('./ProjectsColumn');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { libraryStore } = await import('../citations/libraryStore');
const { settings, updateSettings } = await import('../../stores/settings');
const { openInNewTabOrFocus } = await import('../document/tabsController');
const { showHomePanel } = await import('../home/HomeScreen');

const JARZYNSKI = 'Nonequilibrium equality for free energy differences';
const CROOKS = 'Entropy production fluctuation theorem';

const groupNames = (root: HTMLElement) =>
  Array.from(root.querySelectorAll('.paper-group-name')).map((el) => el.textContent);

beforeEach(async () => {
  projectsStore.setScope({ kind: 'all' });
  projectsStore.select(null);
  await projectsStore.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  updateSettings({ onlineLookup: false });
});

describe('PaperList under All papers', () => {
  it('groups by project in order, Not in a project last, and lists a paper per project', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    expect(groupNames(container)).toEqual(['Noneq stat mech', 'Active matter', 'Not in a project']);
    // In two projects, so listed twice: the overview shows where things are.
    expect(screen.getAllByText(JARZYNSKI)).toHaveLength(2);
    expect(screen.getAllByText(CROOKS)).toHaveLength(1);
    expect(container.querySelector('.paper-list')?.getAttribute('role')).toBe('tree');
  });

  it('folds a group on click and unfolds it again', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    const head = container.querySelector('.paper-group-head')!;
    fireEvent.click(head);
    expect(head.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText(CROOKS)).toBeNull();
    expect(screen.getAllByText(JARZYNSKI)).toHaveLength(1);
    fireEvent.click(head);
    expect(screen.getAllByText(JARZYNSKI)).toHaveLength(2);
  });
});

describe('PaperList under a project', () => {
  beforeEach(async () => {
    projectsStore.setScope({ kind: 'project', id: 'p1' });
    await projectsStore.refresh();
  });

  it('is flat', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(JARZYNSKI);
    expect(container.querySelector('.paper-group-head')).toBeNull();
    expect(container.querySelector('.paper-list')?.getAttribute('role')).toBe('listbox');
    expect(screen.getAllByRole('option')).toHaveLength(2);
    expect(screen.getByText('no .bib')).toBeTruthy();
    expect(screen.getByText('1 incomplete')).toBeTruthy();
  });

  it('moves the selection with the arrows and opens the file still on disk with Enter', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    const list = container.querySelector('.paper-list')!;
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    expect(projectsStore.state.selectedId).toBe('b');
    const rows = screen.getAllByRole('option');
    expect(rows[1]!.getAttribute('aria-selected')).toBe('true');
    expect(rows[1]!.getAttribute('tabindex')).toBe('0');
    expect(rows[0]!.getAttribute('tabindex')).toBe('-1');

    fireEvent.keyDown(list, { key: 'Enter' });
    await waitFor(() => expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/crooks.pdf'));
  });

  it('moves to the neighbouring column on left and right', async () => {
    const moved: number[] = [];
    const { container } = render(() => <PaperList onMoveFocus={(dir) => moved.push(dir)} />);
    await screen.findByText(CROOKS);
    const list = container.querySelector('.paper-list')!;
    fireEvent.keyDown(list, { key: 'ArrowLeft' });
    fireEvent.keyDown(list, { key: 'ArrowRight' });
    expect(moved).toEqual([-1, 1]);
  });
});

describe('lookups from the header', () => {
  it('looks the whole library up from All papers and counts the parked matches', async () => {
    updateSettings({ onlineLookup: true });
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    fireEvent.click(screen.getByRole('button', { name: 'Look up all' }));
    await waitFor(() => expect(engine.libraryLookupAll).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('1 of 3 updated, 1 parked, 1 skipped')).toBeTruthy();
    expect(engine.libraryLookupProject).not.toHaveBeenCalled();
  });

  it('looks a project up by identifier only', async () => {
    updateSettings({ onlineLookup: true });
    projectsStore.setScope({ kind: 'project', id: 'p1' });
    await projectsStore.refresh();
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(CROOKS);
    fireEvent.click(screen.getByRole('button', { name: 'Look up all' }));
    await waitFor(() => expect(engine.libraryLookupProject).toHaveBeenCalledWith('p1'));
    expect(await screen.findByText('1 of 2 updated, 1 failed')).toBeTruthy();
  });

  it('offers no lookup under Other PDFs', async () => {
    updateSettings({ onlineLookup: true });
    projectsStore.setScope({ kind: 'others' });
    await projectsStore.refresh();
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText('toaster-manual.pdf');
    expect(screen.queryByRole('button', { name: 'Look up all' })).toBeNull();
  });
});

describe('rows that are not papers, and rows with parked matches', () => {
  it('lists a non-paper by its file name with no cite key', async () => {
    projectsStore.setScope({ kind: 'others' });
    await projectsStore.refresh();
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    const row = (await screen.findByText('toaster-manual.pdf')).closest('.paper-row')!;
    expect(row.className).toContain('is-other');
    expect(row.querySelector('.paper-row-key')).toBeNull();
    expect(container.querySelector('.paper-list')?.getAttribute('role')).toBe('listbox');
    // Not an HTML5 drag: WebKit would hand it to the native file drop.
    expect(row.getAttribute('draggable')).toBeNull();
  });

  it('badges a paper whose title search parked matches', async () => {
    projectsStore.setScope({ kind: 'project', id: 'p1' });
    await projectsStore.refresh();
    render(() => <PaperList onMoveFocus={() => {}} />);
    const row = (await screen.findByText(CROOKS)).closest('.paper-row')!;
    expect(row.querySelector('.paper-row-matches')?.textContent).toBe('2 matches');
    const other = screen.getByText(JARZYNSKI).closest('.paper-row')!;
    expect(other.querySelector('.paper-row-matches')).toBeNull();
    expect(row.querySelector('.paper-row-key')?.textContent).toBe('crooks1999entropy');
  });
});

/** Rows reach a project by a pointer drag (pointerDrag.ts), never an HTML5
 * one. Both columns are mounted so the drop lands on a real project row and
 * the highlight is the one the user would see; what is under the pointer is
 * whatever the stubbed hit-test says. */
describe('dragging rows onto a project', () => {
  const mountBoth = () =>
    render(() => (
      <>
        <ProjectsColumn onMoveFocus={() => {}} />
        <PaperList onMoveFocus={() => {}} />
      </>
    ));
  const projectRow = (id: string) =>
    document.querySelector(`[data-drop-project="${id}"]`) as HTMLElement;
  let under: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    under = vi.spyOn(document, 'elementFromPoint').mockReturnValue(null);
  });
  afterEach(async () => {
    // The click swallowed after a release is let go on a timer; let it run
    // before the next test clicks anything.
    await new Promise((resolve) => setTimeout(resolve, 0));
    under.mockRestore();
  });

  it('adds every dragged row not already in the project, and only those', async () => {
    mountBoth();
    const first = (await screen.findAllByText(JARZYNSKI))[0]!.closest('.paper-row')!;
    const last = screen.getByText('Untitled scan').closest('.paper-row')!;
    // a, b and c: a and b are in Noneq already, c is in nothing.
    fireEvent.click(first);
    fireEvent.click(last, { shiftKey: true });
    const target = projectRow('p1');
    under.mockReturnValue(target);

    fireEvent.pointerDown(last, { clientX: 10, clientY: 10, button: 0 });
    fireEvent.pointerMove(last, { clientX: 30, clientY: 40 });
    expect(document.querySelector('.drag-ghost')?.textContent).toBe('3 items');
    expect(target.className).toContain('is-drop-target');
    expect(document.body.classList.contains('is-dragging')).toBe(true);

    fireEvent.pointerUp(last, { clientX: 30, clientY: 40 });
    expect(engine.librarySetMembership).toHaveBeenCalledTimes(1);
    expect(engine.librarySetMembership).toHaveBeenCalledWith('c', 'p1', true);
    expect(document.querySelector('.drag-ghost')).toBeNull();
    expect(target.className).not.toContain('is-drop-target');
    expect(document.body.classList.contains('is-dragging')).toBe(false);
    // The click the browser fires after the release is not a click on the
    // project row: the scope stays where it was.
    fireEvent.click(target);
    expect(projectsStore.state.scope).toEqual({ kind: 'all' });
  });

  it('carries a single row under its title, even one the scan passed over', async () => {
    projectsStore.setScope({ kind: 'others' });
    await projectsStore.refresh();
    mountBoth();
    const manual = (await screen.findByText('toaster-manual.pdf')).closest('.paper-row')!;
    under.mockReturnValue(projectRow('p2'));
    fireEvent.pointerDown(manual, { clientX: 10, clientY: 10, button: 0 });
    fireEvent.pointerMove(manual, { clientX: 10, clientY: 30 });
    expect(document.querySelector('.drag-ghost')?.textContent).toBe('toaster-manual.pdf');
    fireEvent.pointerUp(manual, { clientX: 10, clientY: 30 });
    expect(engine.librarySetMembership).toHaveBeenCalledWith('d', 'p2', true);
  });

  it('treats a release below the threshold as the click it was', async () => {
    mountBoth();
    const last = (await screen.findByText('Untitled scan')).closest('.paper-row')!;
    under.mockReturnValue(projectRow('p1'));
    // Cancelled at the press, so WebKit starts no text selection; the click
    // that ends the press still arrives.
    expect(fireEvent.pointerDown(last, { clientX: 10, clientY: 10, button: 0 })).toBe(false);
    fireEvent.pointerMove(last, { clientX: 12, clientY: 12 });
    expect(document.querySelector('.drag-ghost')).toBeNull();
    fireEvent.pointerUp(last, { clientX: 12, clientY: 12 });
    expect(engine.librarySetMembership).not.toHaveBeenCalled();
    fireEvent.click(last);
    expect(projectsStore.state.selectedId).toBe('c');
  });

  it('abandons the drag on Escape', async () => {
    mountBoth();
    const last = (await screen.findByText('Untitled scan')).closest('.paper-row')!;
    const target = projectRow('p1');
    under.mockReturnValue(target);
    fireEvent.pointerDown(last, { clientX: 10, clientY: 10, button: 0 });
    fireEvent.pointerMove(last, { clientX: 30, clientY: 40 });
    expect(target.className).toContain('is-drop-target');
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(document.querySelector('.drag-ghost')).toBeNull();
    expect(target.className).not.toContain('is-drop-target');
    expect(document.body.classList.contains('is-dragging')).toBe(false);
    fireEvent.pointerUp(last, { clientX: 30, clientY: 40 });
    expect(engine.librarySetMembership).not.toHaveBeenCalled();
  });

  it('drops nothing when released away from any project', async () => {
    mountBoth();
    const last = (await screen.findByText('Untitled scan')).closest('.paper-row')!;
    fireEvent.pointerDown(last, { clientX: 10, clientY: 10, button: 0 });
    fireEvent.pointerMove(last, { clientX: 30, clientY: 40 });
    expect(document.querySelector('.drag-ghost')).toBeTruthy();
    fireEvent.pointerUp(last, { clientX: 30, clientY: 40 });
    expect(engine.librarySetMembership).not.toHaveBeenCalled();
    expect(document.querySelector('.drag-ghost')).toBeNull();
  });
});

describe('full-text hand-off', () => {
  it('sets the library query, asks Home for its Library panel and switches screens', async () => {
    updateSettings({ launchScreen: 'projects' });
    render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findAllByText(JARZYNSKI);
    fireEvent.input(screen.getByLabelText('Search items'), { target: { value: 'entropy' } });
    fireEvent.click(screen.getByText(/Search the full text of every item for/));
    expect(libraryStore.state.query).toBe('entropy');
    expect(showHomePanel).toHaveBeenCalledWith('library');
    expect(settings.launchScreen).toBe('home');
  });
});

// @vitest-environment happy-dom
/**
 * Merging a preprint and its journal version: the detail column says which
 * records look like one work and why, a two-step inline confirmation says
 * what moves and can keep the other record instead, the list tags suspects
 * and merges a multi-selection, and a merged record splits back.
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

const { default: PaperDetail } = await import('./PaperDetail');
const { default: PaperList } = await import('./PaperList');
const { CAN_DISMISS_DUPLICATES, mergeMoves } = await import('./MergeConfirm');
const { projectsStore, duplicateIndex } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { DUPLICATES, PREPRINT_TITLE, fixture, paperFor, resetFixture } =
  await import('./testFixtures');

const JARZYNSKI = 'Nonequilibrium equality for free energy differences';

const select = async (id: string) => {
  projectsStore.select(id);
  await vi.waitFor(() => expect(projectsStore.state.selected?.id).toBe(id));
};

beforeEach(async () => {
  fixture.duplicates = DUPLICATES;
  projectsStore.setScope({ kind: 'project', id: 'p1' });
  projectsStore.setView('list');
  await projectsStore.initialize();
  await select('a');
});

afterEach(() => {
  cleanup();
  resetFixture();
  vi.clearAllMocks();
});

const confirmBox = () => screen.getByRole('group', { name: 'Merge items' });

describe('duplicates', () => {
  it('indexes each group member to the rest of its group', () => {
    expect(duplicateIndex([{ paperIds: ['a', 'e', 'f'], reason: 'same DOI' }])).toEqual({
      a: { others: ['e', 'f'], reason: 'same DOI' },
      e: { others: ['a', 'f'], reason: 'same DOI' },
      f: { others: ['a', 'e'], reason: 'same DOI' },
    });
  });

  it('counts what a merge moves: files, new projects, missing identifiers', () => {
    expect(mergeMoves(paperFor('a'), [paperFor('e')])).toEqual(['1 file', 'arXiv id']);
    expect(mergeMoves(paperFor('e'), [paperFor('a')])).toEqual(['1 file', '2 projects', 'DOI']);
  });

  it('names the suspected duplicate and why, and merges into the selected item', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(/Possibly the same work as:/);
    await screen.findByText(PREPRINT_TITLE);
    expect(screen.getByText(/\(same title\)/)).toBeTruthy();
    expect(CAN_DISMISS_DUPLICATES).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Merge' }));
    // Step two: what moves, in words; the first Merge is gone.
    await waitFor(() => expect(confirmBox().textContent).toContain('Moves: 1 file, arXiv id.'));
    expect(confirmBox().textContent).toContain(`Keep “${JARZYNSKI}”`);
    expect(engine.libraryMergePapers).not.toHaveBeenCalled();
    expect(screen.getAllByRole('button', { name: 'Merge' })).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: 'Merge' }));
    await waitFor(() => expect(engine.libraryMergePapers).toHaveBeenCalledWith('a', ['e']));
    await waitFor(() => expect(projectsStore.state.selected?.mergedCount).toBe(1));
  });

  it('records "not the same" with the engine', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(/Possibly the same work as:/);
    fireEvent.click(screen.getByRole('button', { name: 'Not the same' }));
    await waitFor(() => expect(engine.libraryNotDuplicates).toHaveBeenCalledWith(['a', 'e']));
    expect(engine.libraryMergePapers).not.toHaveBeenCalled();
  });

  it('keeps the other record instead when asked', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Merge' }));
    fireEvent.click(await screen.findByText('Keep the other one instead'));
    await waitFor(() => expect(confirmBox().textContent).toContain(`Keep “${PREPRINT_TITLE}”`));
    expect(confirmBox().textContent).toContain('Moves: 1 file, 2 projects, DOI.');
    fireEvent.click(screen.getByRole('button', { name: 'Merge' }));
    await waitFor(() => expect(engine.libraryMergePapers).toHaveBeenCalledWith('e', ['a']));
    await waitFor(() => expect(projectsStore.state.selectedId).toBe('e'));
  });

  it('cancels without merging', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Merge' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    await screen.findByText(/Possibly the same work as:/);
    expect(engine.libraryMergePapers).not.toHaveBeenCalled();
  });

  it('says nothing for an item with no suspected duplicate', async () => {
    await select('b');
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText('crooks1999entropy');
    expect(screen.queryByText(/Possibly the same work/)).toBeNull();
  });
});

describe('merged items', () => {
  it('shows how many records it came from and splits them back', async () => {
    const libraryPaper = vi.mocked(engine.libraryPaper);
    libraryPaper.mockImplementation(async (id: string) => ({
      ...paperFor(id),
      mergedCount: id === 'a' ? 1 : 0,
    }));
    try {
      projectsStore.select('a');
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      await screen.findByText('Merged from 2 items');
      fireEvent.click(screen.getByRole('button', { name: 'Split' }));
      await waitFor(() => expect(engine.libraryUnmerge).toHaveBeenCalledWith('a'));
      // The first record the engine restored is the one shown.
      await waitFor(() => expect(projectsStore.state.selectedId).toBe('e'));
    } finally {
      libraryPaper.mockImplementation(async (id: string) => paperFor(id));
    }
  });
});

describe('merging from the list', () => {
  const rowFor = (container: HTMLElement, title: string) =>
    Array.from(container.querySelectorAll<HTMLElement>('.paper-row')).find((row) =>
      row.textContent?.includes(title)
    )!;

  it('tags suspected duplicates quietly', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(JARZYNSKI);
    await waitFor(() =>
      expect(rowFor(container, JARZYNSKI).querySelector('.paper-row-dup')?.textContent).toBe(
        'duplicate?'
      )
    );
    expect(rowFor(container, 'Entropy production').querySelector('.paper-row-dup')).toBeNull();
  });

  it('merges a multi-selection, keeping the first selected', async () => {
    const { container } = render(() => <PaperList onMoveFocus={() => {}} />);
    await screen.findByText(JARZYNSKI);
    expect(screen.queryByText(/^Merge \d items$/)).toBeNull();
    fireEvent.click(rowFor(container, 'Entropy production'));
    fireEvent.click(rowFor(container, JARZYNSKI), { metaKey: true });
    fireEvent.click(await screen.findByRole('button', { name: 'Merge 2 items' }));
    await waitFor(() =>
      expect(confirmBox().textContent).toContain('Keep “Entropy production fluctuation theorem”')
    );
    fireEvent.click(screen.getByRole('button', { name: 'Merge' }));
    await waitFor(() => expect(engine.libraryMergePapers).toHaveBeenCalledWith('b', ['a']));
    await waitFor(() => expect(screen.queryByRole('group', { name: 'Merge items' })).toBeNull());
  });
});

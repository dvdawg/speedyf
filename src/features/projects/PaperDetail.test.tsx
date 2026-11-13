// @vitest-environment happy-dom
/**
 * The detail column edits in place and sends only what changed, shows where
 * each value came from, turns a title lookup's candidates into one applied
 * pick, and lets the user overrule the scan on whether a file is a paper.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LookupCandidate } from '../../types/engine';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: vi.fn(async () => true),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));

const { default: PaperDetail } = await import('./PaperDetail');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { updateSettings } = await import('../../stores/settings');
const { CANDIDATES } = await import('./testFixtures');
const { requestTitleLookup, titleLookupRequested } = await import('./screenIntents');

const select = async (id: string) => {
  projectsStore.select(id);
  await vi.waitFor(() => expect(projectsStore.state.selected?.id).toBe(id));
};

const TITLE = 'Nonequilibrium equality for free energy differences';

beforeEach(async () => {
  await projectsStore.initialize();
  projectsStore.select('a');
  await vi.waitFor(() => expect(projectsStore.state.selected?.id).toBe('a'));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  updateSettings({ onlineLookup: false });
});

describe('PaperDetail files and automatic merges', () => {
  const twoFiles = async () => {
    const { paperFor } = await import('./testFixtures');
    const paper = paperFor('a');
    paper.files = [
      { ...paper.files[0]!, path: '/lib/2606.13614v1.pdf', name: '2606.13614v1.pdf', size: 2048 },
      { ...paper.files[0]!, path: '/lib/review/mot_tmlr.pdf', name: 'mot_tmlr.pdf', size: 4096 },
      {
        ...paper.files[0]!,
        path: '/lib/gone.pdf',
        name: 'gone.pdf',
        missing: true,
      },
    ];
    paper.mergedCount = 1;
    paper.autoMerged = ['same arXiv id 2606.13614'];
    vi.mocked(engine.libraryPaper).mockResolvedValue(paper);
    await projectsStore.loadDetail('a');
    await vi.waitFor(() => expect(projectsStore.state.selected?.files.length).toBe(3));
  };

  afterEach(async () => {
    const { paperFor } = await import('./testFixtures');
    vi.mocked(engine.libraryPaper).mockImplementation(async (id: string) => paperFor(id));
  });

  it('lists every file with its size, main marker and missing badge', async () => {
    await twoFiles();
    const { container } = render(() => <PaperDetail onMoveFocus={() => {}} />);
    const items = await vi.waitFor(() => {
      const found = container.querySelectorAll('.paper-file');
      expect(found.length).toBe(3);
      return Array.from(found);
    });
    expect(items[0]!.textContent).toContain('2606.13614v1.pdf');
    expect(items[0]!.textContent).toContain('2.0 KB');
    expect(items[0]!.querySelector('.paper-field-source')?.textContent).toBe('main');
    expect(items[0]!.querySelector('.paper-file-name')?.getAttribute('title')).toBe('/lib');
    expect(items[1]!.querySelector('.paper-field-source')).toBeNull();
    expect(items[2]!.textContent).toContain('missing');
    // The main file has no Make main; the others do.
    expect(screen.queryByLabelText('Make 2606.13614v1.pdf the main file')).toBeNull();
    expect(screen.getByLabelText('Make mot_tmlr.pdf the main file')).toBeTruthy();
  });

  it('opens a file and makes another the main one through the transport', async () => {
    await twoFiles();
    const { openInNewTabOrFocus } = await import('../document/tabsController');
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    fireEvent.click(await screen.findByLabelText('Open mot_tmlr.pdf'));
    await waitFor(() =>
      expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/review/mot_tmlr.pdf')
    );
    fireEvent.click(screen.getByLabelText('Make mot_tmlr.pdf the main file'));
    await waitFor(() =>
      expect(engine.librarySetPrimaryFile).toHaveBeenCalledWith('a', '/lib/review/mot_tmlr.pdf')
    );
  });

  it('says the item was merged automatically, and why, beside Split', async () => {
    await twoFiles();
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    const line = await screen.findByText(/Merged automatically/);
    expect(line.textContent).toBe('Merged automatically (same arXiv id 2606.13614)');
    expect(line.closest('.paper-merged')?.querySelector('button')?.textContent).toBe('Split');
  });

  it('says nothing of automatic merges for an item merged by hand', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    expect(screen.queryByText(/Merged automatically/)).toBeNull();
  });
});

describe('PaperDetail', () => {
  it('shows each field with its provenance', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    expect(screen.getAllByText('Crossref').length).toBeGreaterThanOrEqual(4);
    expect(screen.getByText('Jarzynski, C.')).toBeTruthy();
    expect(screen.getByText('jarzynski1997nonequilibrium')).toBeTruthy();
    expect(screen.getByText('jarzynski1997.pdf')).toBeTruthy();
    const noneq = screen.getByLabelText('Noneq stat mech') as HTMLInputElement;
    expect(noneq.checked).toBe(true);
  });

  it('saves an edited title as a one-field patch and shows it as edited', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    const value = await screen.findByText(TITLE);
    fireEvent.click(value);
    const input = screen.getByLabelText('Title') as HTMLInputElement;
    expect(input.value).toBe(TITLE);
    fireEvent.input(input, { target: { value: '  Jarzynski equality  ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() =>
      expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('a', { title: 'Jarzynski equality' })
    );
    expect(engine.libraryUpdatePaper).toHaveBeenCalledTimes(1);
    await screen.findByText('Jarzynski equality');
    expect(screen.getByText('edited')).toBeTruthy();
  });

  it('cancels an edit on Escape without saving', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    fireEvent.click(await screen.findByText(TITLE));
    const input = screen.getByLabelText('Title');
    fireEvent.input(input, { target: { value: 'nope' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(engine.libraryUpdatePaper).not.toHaveBeenCalled();
    expect(screen.getByText(TITLE)).toBeTruthy();
  });

  it('refuses a year that is not one, in place', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    fireEvent.click(screen.getByText('1997'));
    const input = screen.getByLabelText('Year');
    fireEvent.input(input, { target: { value: 'soon' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(engine.libraryUpdatePaper).not.toHaveBeenCalled();
  });

  it('offers a title lookup only when lookup is on, and applies the picked candidate', async () => {
    const candidate: LookupCandidate = {
      source: 'openalex',
      title: 'Nonequilibrium Equality for Free Energy Differences',
      authors: [{ family: 'Jarzynski', given: 'Christopher' }],
      year: 1997,
      venue: 'Physical Review Letters',
      volume: '78',
      issue: '14',
      pages: '2690-2693',
      publisher: null,
      abstract: null,
      kind: 'article',
      doi: '10.1103/physrevlett.78.2690',
      arxiv: null,
      isbn: null,
    };
    vi.mocked(engine.libraryLookup).mockResolvedValueOnce({
      kind: 'candidates',
      candidates: [candidate],
    });

    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    expect(screen.queryByRole('button', { name: 'Find by title' })).toBeNull();

    updateSettings({ onlineLookup: true });
    fireEvent.click(screen.getByRole('button', { name: 'Find by title' }));
    expect(engine.libraryLookup).toHaveBeenCalledWith('a', 'title');
    const pick = await screen.findByText('Christopher Jarzynski, 1997, Physical Review Letters');
    expect(screen.getByText('OpenAlex')).toBeTruthy();
    fireEvent.click(pick);
    await waitFor(() => expect(engine.libraryApplyCandidate).toHaveBeenCalledWith('a', candidate));
    await waitFor(() => expect(screen.queryByText('Which is it?')).toBeNull());
  });

  it('edits the ISBN beside the other identifiers', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    fireEvent.click(screen.getByLabelText('ISBN'));
    const input = screen.getByLabelText('ISBN') as HTMLInputElement;
    fireEvent.input(input, { target: { value: ' 9780521635035 ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() =>
      expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('a', { isbn: '9780521635035' })
    );
    await screen.findByText('9780521635035');
  });

  describe('the paper verdict', () => {
    it('says why the scan took it for a paper, and sends the override as a patch', async () => {
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      expect(
        await screen.findByText(
          'Looks like a paper: doi 10.1103/physrevlett.78.2690 on the first page'
        )
      ).toBeTruthy();
      expect(screen.queryByText('Let the scan decide')).toBeNull();

      fireEvent.click(screen.getByRole('button', { name: 'Not an item' }));
      await waitFor(() =>
        expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('a', { isPaper: false })
      );
      // The engine's answer: overruled, and the verdict says by whom.
      expect(await screen.findByText('Marked as not an item by you')).toBeTruthy();
      expect(screen.getByRole('button', { name: 'It is an item' })).toBeTruthy();

      fireEvent.click(screen.getByRole('button', { name: 'Let the scan decide' }));
      await waitFor(() =>
        expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('a', { isPaper: null })
      );
      await screen.findByText(
        'Looks like a paper: doi 10.1103/physrevlett.78.2690 on the first page'
      );
    });

    it('names a thesis and a book as what the scan took them for', async () => {
      await select('t');
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      expect(
        await screen.findByText('Looks like a thesis: a dissertation title page')
      ).toBeTruthy();
      cleanup();

      await select('k');
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      expect(
        await screen.findByText('Looks like a book: isbn 9780521635035 on the first pages')
      ).toBeTruthy();
    });

    it('says the user marked it, whichever way', async () => {
      // Paper b carries an override already.
      await select('b');
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      expect(await screen.findByText('Marked as an item by you')).toBeTruthy();
      expect(screen.getByText('Let the scan decide')).toBeTruthy();
    });

    it('shows a non-paper with its reason and without the things only a paper has', async () => {
      updateSettings({ onlineLookup: true });
      await select('d');
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      expect(
        await screen.findByText(
          'Not treated as an item: no identifier, abstract or venue on the first page'
        )
      ).toBeTruthy();
      expect(screen.getByText('toaster-manual.pdf')).toBeTruthy();
      expect(screen.queryByText('toastermanual')).toBeNull();
      expect(screen.queryByText(/Missing for BibTeX/)).toBeNull();
      expect(screen.queryByRole('button', { name: 'Copy BibTeX' })).toBeNull();
      expect(screen.queryByRole('button', { name: 'Look up' })).toBeNull();
      expect(screen.queryByRole('button', { name: 'Find by title' })).toBeNull();
      // Still a file with a place: it opens, and it can be filed deliberately.
      expect(screen.getByRole('button', { name: 'Open' })).toBeTruthy();
      expect(screen.getByLabelText('Noneq stat mech')).toBeTruthy();

      fireEvent.click(screen.getByRole('button', { name: 'It is an item' }));
      await waitFor(() =>
        expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('d', { isPaper: true })
      );
      expect(await screen.findByText('toastermanual')).toBeTruthy();
      expect(screen.getByText('Marked as an item by you')).toBeTruthy();
    });
  });

  describe('parked matches', () => {
    beforeEach(() => select('b'));

    it('lists what a title search parked, with its source, and applies the pick', async () => {
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      expect(await screen.findByText('Possible matches (2)')).toBeTruthy();
      expect(screen.getByText('Gavin E. Crooks, 1999, Physical Review E')).toBeTruthy();
      expect(screen.getByText('G. Crooks, 2000')).toBeTruthy();
      expect(screen.getByText('OpenAlex')).toBeTruthy();
      expect(screen.getByText('Semantic Scholar')).toBeTruthy();

      const use = screen.getAllByRole('button', { name: 'Use this' });
      expect(use).toHaveLength(2);
      fireEvent.click(use[1]!);
      await waitFor(() =>
        expect(engine.libraryApplyCandidate).toHaveBeenCalledWith('b', CANDIDATES[1])
      );
      expect(engine.libraryDiscardCandidates).not.toHaveBeenCalled();
    });

    it('dismisses them all through the store', async () => {
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      await screen.findByText('Possible matches (2)');
      fireEvent.click(screen.getByRole('button', { name: 'Dismiss all' }));
      await waitFor(() => expect(engine.libraryDiscardCandidates).toHaveBeenCalledWith('b'));
      expect(engine.libraryApplyCandidate).not.toHaveBeenCalled();
    });

    it('shows none for a paper with nothing parked', async () => {
      await select('a');
      render(() => <PaperDetail onMoveFocus={() => {}} />);
      await screen.findByText(TITLE);
      expect(screen.queryByText(/Possible matches/)).toBeNull();
    });
  });
});

describe('PaperDetail project suggestion', () => {
  afterEach(async () => {
    const { resetFixture } = await import('./testFixtures');
    const { resetDismissedSuggestions } = await import('./ProjectSuggestion');
    resetFixture();
    resetDismissedSuggestions();
  });

  it('shows the top suggestion under the checkboxes and adds only on Add', async () => {
    const { fixture } = await import('./testFixtures');
    fixture.suggestions = [
      {
        projectId: 'p3',
        name: 'Thesis',
        score: 3,
        cites: 1,
        citedBy: 1,
        sharedRefs: 0,
        reason: 'cites 1 paper in it, cited by 1 paper in it',
      },
    ];
    const { container } = render(() => <PaperDetail onMoveFocus={() => {}} />);
    const line = await screen.findByText(
      'Add to Thesis? (cites 1 paper in it, cited by 1 paper in it)'
    );
    expect(line.closest('.paper-section')).toBe(
      container.querySelector('.paper-projects')!.closest('.paper-section')
    );
    expect(engine.librarySuggestProjects).toHaveBeenCalledWith('a');
    expect(engine.librarySetMembership).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(engine.librarySetMembership).toHaveBeenCalledWith('a', 'p3', true));
    expect(screen.queryByText(/Add to Thesis/)).toBeNull();
  });

  it('shows nothing without a suggestion', async () => {
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await waitFor(() => expect(engine.librarySuggestProjects).toHaveBeenCalledWith('a'));
    expect(screen.queryByText(/^Add to /)).toBeNull();
  });
});

describe('Look Up by Title from the palette', () => {
  it('runs once the requested item shows, and shows the matches here', async () => {
    vi.mocked(engine.libraryLookup).mockResolvedValueOnce({ kind: 'candidates', candidates: [] });
    updateSettings({ onlineLookup: true });
    render(() => <PaperDetail onMoveFocus={() => {}} />);
    await screen.findByText(TITLE);
    // Asked for another item: nothing runs until that item is the one shown.
    requestTitleLookup('b');
    expect(engine.libraryLookup).not.toHaveBeenCalled();
    await select('b');
    await waitFor(() => expect(engine.libraryLookup).toHaveBeenCalledWith('b', 'title'));
    expect(engine.libraryLookup).toHaveBeenCalledTimes(1);
    expect(titleLookupRequested()).toBeNull();
    await screen.findByText('Nothing found');
  });
});

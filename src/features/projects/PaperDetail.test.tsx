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

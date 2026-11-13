// @vitest-environment happy-dom
/**
 * The Paper panel's three states and the wiring between them.
 *
 * The record itself comes from the engine and the projects come from the
 * store, both already covered elsewhere. What only a mounted panel can show is
 * which state a given answer lands in: a record renders its projects as live
 * checkboxes, no record offers to file the document and then shows the copy,
 * and the citation graph stays out of sight entirely while the engine has not
 * grown it — an empty "In your library" would be a claim, and a wrong one.
 */
import { render, fireEvent, cleanup, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CitationLinks, MetaPatch, Paper, Project, Paperness } from '../../types/engine';
import type { TabRecord } from '../../stores/tabsStore';
import type * as engineModule from '../../lib/transport/engine';

const paper: Paper = {
  id: 'paper-1',
  files: [
    {
      path: '/lib/noneq/jarzynski1997.pdf',
      name: 'jarzynski1997.pdf',
      size: 1,
      mtimeMs: 0,
      fingerprint: null,
      missing: false,
    },
  ],
  doi: '10.1103/physrevlett.78.2690',
  arxiv: null,
  isbn: null,
  meta: {
    title: { value: 'Nonequilibrium Equality for Free Energy Differences', source: 'crossref' },
    authors: { value: [{ family: 'Jarzynski', given: 'C.' }], source: 'crossref' },
    year: { value: 1997, source: 'crossref' },
    venue: { value: 'Physical Review Letters', source: 'crossref' },
    volume: null,
    issue: null,
    pages: null,
    publisher: null,
    abstract: null,
    kind: null,
  },
  citeKey: 'jarzynski1997nonequilibrium',
  citeKeyPinned: false,
  projectIds: ['p1'],
  addedAt: '',
  updatedAt: '',
  incomplete: [],
  isPaper: true,
  paperReason: 'doi 10.1103/physrevlett.78.2690 on the first page',
  paperOverride: null,
  candidates: [],
  mergedCount: 0,
};

/** The same file as the scan sees a manual: in the library, not a paper. */
const manual: Paper = {
  ...paper,
  citeKey: 'toastermanual',
  isPaper: false,
  paperReason: 'no identifier, abstract or venue on the first page',
};

const projects: Project[] = [
  {
    id: 'p1',
    name: 'Noneq',
    order: 0,
    paperCount: 1,
    bibPath: null,
    incompleteCount: 0,
    createdAt: '',
  },
  {
    id: 'p2',
    name: 'Thesis',
    order: 1,
    paperCount: 0,
    bibPath: null,
    incompleteCount: 0,
    createdAt: '',
  },
];

const notImplemented = {
  code: 'unsupported',
  message: 'library_citation_links: not implemented yet',
};

const engine = {
  onLibraryChanged: vi.fn(async (_cb: () => void) => () => {}),
  libraryPaperForPath: vi.fn(async (_path: string): Promise<Paper | null> => paper),
  libraryClassifyDocument: vi.fn(async (_docId: number): Promise<Paperness> => ({
    verdict: 'unknown',
    reason: '',
    kind: null,
    doi: null,
    arxiv: null,
    isbn: null,
  })),
  libraryProjects: vi.fn(async () => projects),
  libraryCounts: vi.fn(async () => ({ all: 1, unfiled: 0, others: 0 })),
  libraryPapers: vi.fn(async () => ({ rows: [], total: 0 })),
  librarySetMembership: vi.fn(async (_paper: string, _project: string, _member: boolean) => {}),
  libraryBibtex: vi.fn(async (_ids: string[]) => '@article{jarzynski1997nonequilibrium}'),
  libraryCitationLinks: vi.fn(async (_id: string): Promise<CitationLinks> => {
    throw notImplemented;
  }),
  libraryFileIntoInbox: vi.fn(async (_path: string, _inbox: string) => paper),
  libraryPaper: vi.fn(async (_id: string) => paper),
  libraryLookup: vi.fn(async () => ({ kind: 'applied' as const, paper })),
  libraryUpdatePaper: vi.fn(async (_id: string, patch: MetaPatch): Promise<Paper> => ({
    ...paper,
    isPaper: patch.isPaper === true,
    paperOverride: patch.isPaper ?? null,
    paperReason: 'marked by you',
  })),
};

vi.mock('../../lib/transport/engine', async (importOriginal) => {
  const actual = await importOriginal<typeof engineModule>();
  return { ...actual, engine };
});

const openDialog = vi.fn(async (_opts?: unknown): Promise<string | null> => '/lib/inbox');
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialog(...(args as [])),
}));

const openInNewTabOrFocus = vi.fn(async (_path: string) => true);
const goHome = vi.fn();
vi.mock('../document/tabsController', () => ({
  openInNewTabOrFocus: (...args: unknown[]) => openInNewTabOrFocus(...(args as [string])),
  goHome: () => goHome(),
}));

const { default: PaperPanel, authorsLine } = await import('./PaperPanel');
const { TabContext } = await import('../../app/TabContext');
const { createDocumentStore } = await import('../document/documentStore');
const { updateSettings } = await import('../../stores/settings');
const { activePaper, activePaperKnown, forgetPapers } = await import('./activePaper');

const meta = (path: string) => ({
  docId: 1,
  path,
  name: path.slice(path.lastIndexOf('/') + 1),
  pageCount: 1,
  sizes: [[612, 792, 0, 0, 0]] as [number, number, number, number, number][],
  estimatedSize: [612, 792] as [number, number],
});

function mount(path = '/lib/noneq/jarzynski1997.pdf', loaded = true) {
  const documentStore = createDocumentStore();
  if (loaded) documentStore.initFromMeta(meta(path));
  const tab = { id: 'tab-1', documentStore, opening: false } as unknown as TabRecord;
  return render(() => (
    <TabContext.Provider value={tab}>
      <PaperPanel />
    </TabContext.Provider>
  ));
}

const writeText = vi.fn(async (_text: string) => {});

beforeEach(() => {
  for (const fn of Object.values(engine)) fn.mockClear();
  engine.libraryPaperForPath.mockImplementation(async () => paper);
  engine.libraryCitationLinks.mockImplementation(async () => {
    throw notImplemented;
  });
  openDialog.mockClear();
  openInNewTabOrFocus.mockClear();
  goHome.mockClear();
  writeText.mockClear();
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  updateSettings({
    libraryInbox: null,
    libraryRoots: [],
    onlineLookup: false,
    launchScreen: 'home',
  });
  forgetPapers();
});

afterEach(cleanup);

describe('authorsLine', () => {
  const author = (family: string) => ({ family, given: 'X.' });

  it('joins up to three names and trails off after that', () => {
    expect(authorsLine([author('Jarzynski')])).toBe('Jarzynski');
    expect(authorsLine([author('Crooks'), author('Jarzynski')])).toBe('Crooks & Jarzynski');
    expect(authorsLine([author('A'), author('B'), author('C')])).toBe('A, B & C');
    expect(authorsLine([author('A'), author('B'), author('C'), author('D')])).toBe('A et al.');
  });

  it('falls back to a given name and skips empty ones', () => {
    expect(authorsLine([{ family: '', given: 'Plato' }])).toBe('Plato');
    expect(authorsLine([{ family: ' ', given: '' }, author('B')])).toBe('B');
    expect(authorsLine([])).toBe('');
  });
});

describe('PaperPanel', () => {
  it('shows the record with a checkbox per project', async () => {
    mount();
    expect(
      await screen.findByText('Nonequilibrium Equality for Free Energy Differences')
    ).toBeTruthy();
    expect(screen.getByText('Jarzynski')).toBeTruthy();
    expect(screen.getByText('1997 · Physical Review Letters')).toBeTruthy();
    expect(screen.getByText('jarzynski1997nonequilibrium')).toBeTruthy();

    const noneq = (await screen.findByLabelText('Noneq')) as HTMLInputElement;
    const thesis = screen.getByLabelText('Thesis') as HTMLInputElement;
    expect(noneq.checked).toBe(true);
    expect(thesis.checked).toBe(false);
    // Not in the library: no Add button, and nothing about filing.
    expect(screen.queryByRole('button', { name: 'Add to library' })).toBeNull();
  });

  it('toggles membership through the store', async () => {
    mount();
    const thesis = (await screen.findByLabelText('Thesis')) as HTMLInputElement;
    thesis.checked = true;
    fireEvent.input(thesis);
    await waitFor(() =>
      expect(engine.librarySetMembership).toHaveBeenCalledWith('paper-1', 'p2', true)
    );
  });

  /** The commands read this cache synchronously; the panel is what fills it. */
  it('leaves the record where the Library commands can read it', async () => {
    mount();
    await screen.findByText('jarzynski1997nonequilibrium');
    // activePaper() keys on the active tab, which this test has none of; the
    // cache itself is what the panel is responsible for.
    expect(activePaperKnown()).toBe(false);
    expect(activePaper()).toBe(null);
    expect(engine.libraryPaperForPath).toHaveBeenCalledWith('/lib/noneq/jarzynski1997.pdf');
  });

  it('copies the cite key inside the click', async () => {
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Copy cite key' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('jarzynski1997nonequilibrium'));
  });

  it('copies the BibTeX the engine builds, and shows its refusal inline', async () => {
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Copy BibTeX' }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith('@article{jarzynski1997nonequilibrium}')
    );

    engine.libraryBibtex.mockRejectedValueOnce({
      code: 'unsupported',
      message: 'not implemented yet',
    });
    cleanup();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Copy BibTeX' }));
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(screen.getByRole('alert').textContent).toContain('not implemented yet');
  });

  it('offers lookup only when the setting is on', async () => {
    mount();
    await screen.findByText('jarzynski1997nonequilibrium');
    expect(screen.queryByRole('button', { name: 'Look up' })).toBeNull();
    updateSettings({ onlineLookup: true });
    fireEvent.click(await screen.findByRole('button', { name: 'Look up' }));
    await waitFor(() => expect(engine.libraryLookup).toHaveBeenCalledWith('paper-1', 'id'));
  });

  it('shows what is missing for BibTeX', async () => {
    engine.libraryPaperForPath.mockResolvedValueOnce({ ...paper, incomplete: ['authors', 'year'] });
    mount();
    expect(await screen.findByText('Missing for BibTeX: authors, year')).toBeTruthy();
  });

  it('sends the user to the Projects screen and remembers the choice', async () => {
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Show in Projects' }));
    expect(goHome).toHaveBeenCalledTimes(1);
    const { settings } = await import('../../stores/settings');
    expect(settings.launchScreen).toBe('projects');
  });

  describe('a PDF the scan did not take for a paper', () => {
    beforeEach(() => engine.libraryPaperForPath.mockImplementation(async () => manual));

    it('says so quietly and offers nothing a paper would have', async () => {
      updateSettings({ onlineLookup: true });
      mount();
      expect(
        await screen.findByText(
          'Not treated as an item (no identifier, abstract or venue on the first page).'
        )
      ).toBeTruthy();
      // The record is in the library, so there is nothing to add.
      expect(screen.queryByRole('button', { name: 'Add to library' })).toBeNull();
      // No key, no projects, no BibTeX, no lookup: none of them mean anything yet.
      expect(screen.queryByText('toastermanual')).toBeNull();
      expect(screen.queryByText('Projects')).toBeNull();
      expect(screen.queryByRole('button', { name: 'Copy BibTeX' })).toBeNull();
      expect(screen.queryByRole('button', { name: 'Look up' })).toBeNull();
      expect(screen.getByRole('button', { name: 'Show in Projects' })).toBeTruthy();
    });

    it('becomes a paper on Mark as item, without waiting for the change event', async () => {
      mount();
      fireEvent.click(await screen.findByRole('button', { name: 'Mark as item' }));
      await waitFor(() =>
        expect(engine.libraryUpdatePaper).toHaveBeenCalledWith('paper-1', { isPaper: true })
      );
      expect(await screen.findByText('jarzynski1997nonequilibrium')).toBeTruthy();
      expect(screen.getByText('Projects')).toBeTruthy();
      expect(screen.queryByText(/Not treated as an item/)).toBeNull();
    });
  });

  describe('outside the library', () => {
    beforeEach(() => engine.libraryPaperForPath.mockImplementation(async () => null));

    it('explains and offers to file the document', async () => {
      mount('/Users/someone/Downloads/draft.pdf');
      expect(await screen.findByRole('button', { name: 'Add to library' })).toBeTruthy();
      expect(screen.getByText(/not in your library/)).toBeTruthy();
      expect(screen.queryByText('Projects')).toBeNull();
    });

    it('says why when the document looks like a paper, and stays quiet otherwise', async () => {
      engine.libraryClassifyDocument.mockImplementationOnce(async () => ({
        verdict: 'paper',
        reason: 'arXiv:1706.03762 on the first page',
        kind: 'article',
        doi: null,
        arxiv: '1706.03762',
        isbn: null,
      }));
      mount('/Users/someone/Downloads/draft.pdf');
      expect(await screen.findByText(/looks like a paper/)).toBeTruthy();
      expect(screen.getByText(/arXiv:1706.03762 on the first page/)).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Add to library' }).className).toContain(
        'primary-btn'
      );
      cleanup();

      // The scan names what it saw: a thesis is offered as one.
      engine.libraryClassifyDocument.mockImplementationOnce(async () => ({
        verdict: 'likely',
        reason: 'a dissertation title page',
        kind: 'thesis',
        doi: null,
        arxiv: null,
        isbn: null,
      }));
      mount('/Users/someone/Downloads/thesis.pdf');
      expect(
        await screen.findByText(
          /This looks like a thesis \(a dissertation title page\) and is not in your library yet/
        )
      ).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Add to library' }).className).toContain(
        'primary-btn'
      );
      cleanup();

      // A manual: no reason, no pitch, the button stays secondary.
      mount('/Users/someone/Downloads/manual.pdf');
      expect(await screen.findByText('This PDF is not in your library.')).toBeTruthy();
      expect(screen.queryByText(/looks like a paper/)).toBeNull();
      expect(screen.getByRole('button', { name: 'Add to library' }).className).toContain(
        'secondary-btn'
      );
    });

    it('asks for an inbox once, copies the file there and shows the copy', async () => {
      mount('/Users/someone/Downloads/draft.pdf');
      fireEvent.click(await screen.findByRole('button', { name: 'Add to library' }));

      await waitFor(() =>
        expect(engine.libraryFileIntoInbox).toHaveBeenCalledWith(
          '/Users/someone/Downloads/draft.pdf',
          '/lib/inbox'
        )
      );
      expect(openDialog).toHaveBeenCalledTimes(1);
      expect(openDialog.mock.calls[0]![0]).toMatchObject({ directory: true });
      const { settings } = await import('../../stores/settings');
      expect(settings.libraryInbox).toBe('/lib/inbox');

      // The panel proper, for the copy: its record, and a way to open it.
      expect(await screen.findByText('jarzynski1997nonequilibrium')).toBeTruthy();
      expect(screen.getByText('Copied into /lib/inbox')).toBeTruthy();
      fireEvent.click(screen.getByRole('button', { name: 'Open the library copy' }));
      expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/noneq/jarzynski1997.pdf');
    });

    it('uses the inbox already chosen without asking again', async () => {
      updateSettings({ libraryInbox: '/lib/chosen' });
      mount('/Users/someone/Downloads/draft.pdf');
      fireEvent.click(await screen.findByRole('button', { name: 'Add to library' }));
      await waitFor(() =>
        expect(engine.libraryFileIntoInbox).toHaveBeenCalledWith(
          '/Users/someone/Downloads/draft.pdf',
          '/lib/chosen'
        )
      );
      expect(openDialog).not.toHaveBeenCalled();
    });

    it('keeps the offer and shows the reason when the copy fails', async () => {
      updateSettings({ libraryInbox: '/elsewhere' });
      engine.libraryFileIntoInbox.mockRejectedValueOnce({
        code: 'io',
        message: '/elsewhere is not inside a library folder',
      });
      mount('/Users/someone/Downloads/draft.pdf');
      fireEvent.click(await screen.findByRole('button', { name: 'Add to library' }));
      expect((await screen.findByRole('alert')).textContent).toContain('not inside a library');
      expect(screen.getByRole('button', { name: 'Add to library' })).toBeTruthy();
    });

    it('does nothing when the folder dialog is dismissed', async () => {
      openDialog.mockResolvedValueOnce(null);
      mount('/Users/someone/Downloads/draft.pdf');
      fireEvent.click(await screen.findByRole('button', { name: 'Add to library' }));
      await waitFor(() => expect(openDialog).toHaveBeenCalled());
      expect(engine.libraryFileIntoInbox).not.toHaveBeenCalled();
    });
  });

  describe('citation links', () => {
    it('hides the section entirely while the engine has not grown it', async () => {
      mount();
      await screen.findByText('jarzynski1997nonequilibrium');
      await waitFor(() => expect(engine.libraryCitationLinks).toHaveBeenCalledWith('paper-1'));
      expect(screen.queryByText('In your library')).toBeNull();
      expect(screen.queryByText('Cites')).toBeNull();
    });

    it('lists what the paper cites and what cites it, each row opening that paper', async () => {
      const row = {
        id: 'paper-2',
        title: 'Entropy Production Fluctuation Theorem',
        authors: 'Crooks',
        year: 1999,
        venue: null,
        citeKey: 'crooks1999entropy',
        alphaLabel: 'Cro99',
        projectIds: [],
        missing: false,
        hasIdentifier: true,
        incompleteCount: 0,
        isPaper: true,
        candidateCount: 0,
      };
      engine.libraryCitationLinks.mockResolvedValueOnce({
        cites: [],
        citedBy: [row],
        unresolved: 2,
      });
      engine.libraryPaper.mockResolvedValueOnce({
        ...paper,
        id: 'paper-2',
        files: [
          { ...paper.files[0]!, path: '/lib/noneq/missing.pdf', missing: true },
          { ...paper.files[0]!, path: '/lib/noneq/crooks1999.pdf', missing: false },
        ],
      });
      mount();
      expect(await screen.findByText('In your library')).toBeTruthy();
      expect(screen.getByText('Cited by')).toBeTruthy();
      expect(screen.queryByText('Cites')).toBeNull();
      expect(screen.getByText('Crooks · 1999')).toBeTruthy();
      expect(screen.getByText(/2 cited identifiers are not in your library/)).toBeTruthy();

      fireEvent.click(screen.getByText('Entropy Production Fluctuation Theorem'));
      // The first file that is still there, not merely the first file.
      await waitFor(() =>
        expect(openInNewTabOrFocus).toHaveBeenCalledWith('/lib/noneq/crooks1999.pdf')
      );
    });

    it('reports any other failure rather than hiding it', async () => {
      engine.libraryCitationLinks.mockRejectedValueOnce({
        code: 'io',
        message: 'sidecar unreadable',
      });
      mount();
      expect(await screen.findByText('sidecar unreadable')).toBeTruthy();
      expect(screen.getByText('In your library')).toBeTruthy();
    });
  });

  it('shows the empty note until a document is loaded', () => {
    mount('/lib/x.pdf', false);
    expect(screen.getByText('Open a document to see its place in your library.')).toBeTruthy();
    expect(engine.libraryPaperForPath).not.toHaveBeenCalled();
  });
});

/** A small library for the component tests: two projects, three papers, one
 * of them in both projects and one in neither, one PDF the scan did not take
 * for a paper, and an engine that answers from it. Shared so each test file
 * mocks the transport the same way. */
import { vi } from 'vitest';
import type {
  DuplicateGroup,
  Graph,
  LibraryCounts,
  LookupBatch,
  LookupCandidate,
  LookupResult,
  MetaPatch,
  MissingWork,
  Paper,
  PaperFilter,
  PaperRow,
  Project,
} from '../../types/engine';

const WHEN = '2026-11-11T20:00:00Z';

export const PROJECTS: Project[] = [
  {
    id: 'p1',
    name: 'Noneq stat mech',
    order: 0,
    paperCount: 2,
    bibPath: null,
    incompleteCount: 1,
    createdAt: WHEN,
  },
  {
    id: 'p2',
    name: 'Active matter',
    order: 1,
    paperCount: 1,
    bibPath: '/tmp/active.bib',
    incompleteCount: 0,
    createdAt: WHEN,
  },
];

export const ROWS: PaperRow[] = [
  {
    id: 'a',
    title: 'Nonequilibrium equality for free energy differences',
    authors: 'Jarzynski',
    year: 1997,
    venue: 'Physical Review Letters',
    citeKey: 'jarzynski1997nonequilibrium',
    alphaLabel: 'Jar97',
    projectIds: ['p1', 'p2'],
    missing: false,
    hasIdentifier: true,
    incompleteCount: 0,
    isPaper: true,
    candidateCount: 0,
    fileCount: 1,
  },
  {
    id: 'b',
    title: 'Entropy production fluctuation theorem',
    authors: 'Crooks',
    year: 1999,
    venue: 'Physical Review E',
    citeKey: 'crooks1999entropy',
    alphaLabel: 'Cro99',
    projectIds: ['p1'],
    missing: false,
    hasIdentifier: true,
    incompleteCount: 0,
    isPaper: true,
    candidateCount: 2,
    fileCount: 1,
  },
  {
    id: 'c',
    title: 'Untitled scan',
    authors: '',
    year: null,
    venue: null,
    citeKey: 'anonndscan',
    alphaLabel: 'Unt',
    projectIds: [],
    missing: true,
    hasIdentifier: false,
    incompleteCount: 2,
    isPaper: true,
    candidateCount: 0,
    fileCount: 1,
  },
  // Not a paper: the scan found no identifier, no abstract, no venue. The
  // engine shows the file name as its title and it only appears under
  // Other PDFs, or in a project once filed there deliberately.
  {
    id: 'd',
    title: 'toaster-manual.pdf',
    authors: '',
    year: null,
    venue: null,
    citeKey: 'toastermanual',
    alphaLabel: 'Toa',
    projectIds: [],
    missing: false,
    hasIdentifier: false,
    incompleteCount: 3,
    isPaper: false,
    candidateCount: 0,
    fileCount: 1,
  },
];

export const COUNTS: LibraryCounts = { all: 3, unfiled: 1, others: 1 };

/** A fourth paper for the graph alone: not in ROWS, so the lists above are
 * unchanged. Cites both of the others and is cited back by one. */
export const EVANS: PaperRow = {
  id: 'g',
  title: 'The fluctuation theorem',
  authors: 'Evans & Searles',
  year: 2002,
  venue: 'Advances in Physics',
  citeKey: 'evans2002fluctuation',
  alphaLabel: 'ES02',
  projectIds: ['p1'],
  missing: false,
  hasIdentifier: true,
  incompleteCount: 0,
  isPaper: true,
  candidateCount: 0,
  fileCount: 1,
};

/** The citation graph of project p1 (and, in this fixture, of the library):
 * Crooks cites Jarzynski, Evans cites both, Jarzynski cites Evans back (so
 * one pair runs both ways), and the untitled scan cites nothing known. Two
 * citations leave the set and one item is not indexed yet. */
export const GRAPH: Graph = {
  nodes: [ROWS[0]!, ROWS[1]!, ROWS[2]!, EVANS],
  edges: [
    { from: 'b', to: 'a' },
    { from: 'g', to: 'a' },
    { from: 'g', to: 'b' },
    { from: 'a', to: 'g' },
  ],
  external: 2,
  unindexed: 1,
};

/** GRAPH with the untitled scan citing Jarzynski: a missing file that is
 * connected, so the graph draws it. */
export const GRAPH_WITH_SCAN: Graph = {
  ...GRAPH,
  edges: [...GRAPH.edges, { from: 'c', to: 'a' }],
};

/** A larger graph for the label policy: `count` papers over ten years,
 * three hubs the others cite (the first most, the third least), nothing
 * else cited, and `isolated` papers with no citations at all. Ids
 * are `n0`…; the hubs are n0, n1 and n2, in years 2000, 2004 and 2008. */
export function bigGraph(count = 30, isolated = 0): Graph {
  const row = (i: number, year: number): PaperRow => ({
    id: `n${i}`,
    title: `Paper number ${i}`,
    authors: `Author${i}`,
    year,
    venue: null,
    citeKey: `author${i}`,
    alphaLabel: `Aut${String(year % 100).padStart(2, '0')}`,
    projectIds: ['p1'],
    missing: false,
    hasIdentifier: true,
    incompleteCount: 0,
    isPaper: true,
    candidateCount: 0,
    fileCount: 1,
  });
  const hubYears = [2000, 2004, 2008];
  const nodes: PaperRow[] = [];
  for (let i = 0; i < count; i++) nodes.push(row(i, i < 3 ? hubYears[i]! : 2001 + (i % 9)));
  const edges: { from: string; to: string }[] = [];
  for (let i = 3; i < count; i++) {
    edges.push({ from: `n${i}`, to: 'n0' });
    if (i % 2 === 0) edges.push({ from: `n${i}`, to: 'n1' });
    if (i % 3 === 0) edges.push({ from: `n${i}`, to: 'n2' });
  }
  for (let i = 0; i < isolated; i++) nodes.push(row(count + i, 2005));
  return { nodes, edges, external: 0, unindexed: 0 };
}

/** What a title search parked on paper b: two works it could be. */
export const CANDIDATES: LookupCandidate[] = [
  {
    source: 'openalex',
    title: 'Entropy production fluctuation theorem and the nonequilibrium work relation',
    authors: [{ family: 'Crooks', given: 'Gavin E.' }],
    year: 1999,
    venue: 'Physical Review E',
    volume: '60',
    issue: '3',
    pages: '2721-2726',
    publisher: null,
    abstract: null,
    kind: 'article',
    doi: '10.1103/physreve.60.2721',
    arxiv: null,
    isbn: null,
  },
  {
    source: 'semantic-scholar',
    title: 'Entropy production fluctuation theorem',
    authors: [{ family: 'Crooks', given: 'G.' }],
    year: 2000,
    venue: null,
    volume: null,
    issue: null,
    pages: null,
    publisher: null,
    abstract: null,
    kind: null,
    doi: null,
    arxiv: 'cond-mat/9901352',
    isbn: null,
  },
];

/** Jarzynski's preprint, a second record of paper a: one file, the arXiv
 * id a lacks, in no project. Not in ROWS; reached by id. */
export const PREPRINT_TITLE = 'A nonequilibrium equality for free energy differences';

/** One suspected pair: paper a and its preprint e. Off by default, so the
 * lists and the detail read as before; a test sets `fixture.duplicates`. */
export const DUPLICATES: DuplicateGroup[] = [{ paperIds: ['a', 'e'], reason: 'same title' }];

/** Works the graph's items cite that the library lacks. At the default
 * threshold of two citers: m1 and m2. */
export const MISSING: MissingWork[] = [
  {
    key: 'm1',
    text: 'G. E. Crooks, Path-ensemble averages in systems driven far from equilibrium, Phys. Rev. E 61, 2361 (2000).',
    year: 2000,
    doi: '10.1103/PhysRevE.61.2361',
    arxiv: null,
    citedBy: ['a', 'g'],
  },
  {
    key: 'm2',
    text: 'U. Seifert, Stochastic thermodynamics, fluctuation theorems and molecular machines, Rep. Prog. Phys. 75, 126001 (2012).',
    year: null,
    doi: null,
    arxiv: '1205.4176',
    citedBy: ['a', 'b', 'g'],
  },
  {
    key: 'm3',
    text: 'J. Kurchan, Fluctuation theorem for stochastic dynamics (1998).',
    year: 1998,
    doi: null,
    arxiv: null,
    citedBy: ['b'],
  },
];

/** What the fake engine answers that a test may change, reset by `resetFixture`. */
export const fixture = {
  duplicates: [] as DuplicateGroup[],
};

export function resetFixture(): void {
  fixture.duplicates = [];
}

const PAPERS: Record<string, Paper> = {
  a: {
    id: 'a',
    files: [
      {
        path: '/lib/jarzynski1997.pdf',
        name: 'jarzynski1997.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'f',
        missing: false,
      },
    ],
    doi: '10.1103/physrevlett.78.2690',
    arxiv: null,
    isbn: null,
    meta: {
      title: { value: 'Nonequilibrium equality for free energy differences', source: 'crossref' },
      authors: { value: [{ family: 'Jarzynski', given: 'C.' }], source: 'crossref' },
      year: { value: 1997, source: 'crossref' },
      venue: { value: 'Physical Review Letters', source: 'crossref' },
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: { value: 'article', source: 'crossref' },
    },
    citeKey: 'jarzynski1997nonequilibrium',
    citeKeyPinned: false,
    projectIds: ['p1', 'p2'],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: [],
    isPaper: true,
    paperReason: 'doi 10.1103/physrevlett.78.2690 on the first page',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  },
  b: {
    id: 'b',
    // The first file is gone; opening must pick the second.
    files: [
      {
        path: '/lib/old/crooks.pdf',
        name: 'crooks.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'g',
        missing: true,
      },
      {
        path: '/lib/crooks.pdf',
        name: 'crooks.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'g',
        missing: false,
      },
    ],
    doi: null,
    arxiv: null,
    isbn: null,
    meta: {
      title: { value: 'Entropy production fluctuation theorem', source: 'first-page' },
      authors: null,
      year: null,
      venue: null,
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: null,
    },
    citeKey: 'crooks1999entropy',
    citeKeyPinned: true,
    projectIds: ['p1'],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: ['authors', 'year'],
    isPaper: true,
    paperReason: 'marked by you',
    paperOverride: true,
    candidates: CANDIDATES,
    mergedCount: 0,
    autoMerged: [],
  },
  d: {
    id: 'd',
    files: [
      {
        path: '/lib/toaster-manual.pdf',
        name: 'toaster-manual.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'h',
        missing: false,
      },
    ],
    doi: null,
    arxiv: null,
    isbn: null,
    meta: {
      title: { value: 'toaster-manual', source: 'filename' },
      authors: null,
      year: null,
      venue: null,
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: null,
    },
    citeKey: 'toastermanual',
    citeKeyPinned: false,
    projectIds: [],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: ['authors', 'year', 'venue'],
    isPaper: false,
    paperReason: 'no identifier, abstract or venue on the first page',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  },
  // Evans for the graph's hover card: a full author list and an abstract.
  g: {
    id: 'g',
    files: [
      {
        path: '/lib/evans2002.pdf',
        name: 'evans2002.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'k',
        missing: false,
      },
    ],
    doi: '10.1080/00018730210155133',
    arxiv: null,
    isbn: null,
    meta: {
      title: { value: 'The fluctuation theorem', source: 'crossref' },
      authors: {
        value: [
          { family: 'Evans', given: 'Denis J.' },
          { family: 'Searles', given: 'Debra J.' },
        ],
        source: 'crossref',
      },
      year: { value: 2002, source: 'crossref' },
      venue: { value: 'Advances in Physics', source: 'crossref' },
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: {
        value:
          'The fluctuation theorem gives an analytical expression for the probability of observing second law violating dynamical fluctuations.',
        source: 'crossref',
      },
      kind: { value: 'article', source: 'crossref' },
    },
    citeKey: 'evans2002fluctuation',
    citeKeyPinned: false,
    projectIds: ['p1'],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: [],
    isPaper: true,
    paperReason: 'doi on the first page',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  },
  e: {
    id: 'e',
    files: [
      {
        path: '/lib/inbox/9610209.pdf',
        name: '9610209.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'm',
        missing: false,
      },
    ],
    doi: null,
    arxiv: 'cond-mat/9610209',
    isbn: null,
    meta: {
      title: { value: PREPRINT_TITLE, source: 'arxiv' },
      authors: { value: [{ family: 'Jarzynski', given: 'C.' }], source: 'arxiv' },
      year: { value: 1996, source: 'arxiv' },
      venue: null,
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: { value: 'article', source: 'arxiv' },
    },
    citeKey: 'jarzynski1996nonequilibrium',
    citeKeyPinned: false,
    projectIds: [],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: ['venue'],
    isPaper: true,
    paperReason: 'arXiv id on the first page',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  },
  // A thesis and a book: items too, with the kind the scan read off them.
  // Not in ROWS, so the lists above are unchanged; reached by id alone.
  t: {
    id: 't',
    files: [
      {
        path: '/lib/thesis.pdf',
        name: 'thesis.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'i',
        missing: false,
      },
    ],
    doi: null,
    arxiv: null,
    isbn: null,
    meta: {
      title: { value: 'Fluctuation theorems far from equilibrium', source: 'first-page' },
      authors: null,
      year: null,
      venue: null,
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: { value: 'thesis', source: 'first-page' },
    },
    citeKey: 'fluctuation',
    citeKeyPinned: false,
    projectIds: [],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: ['authors', 'year'],
    isPaper: true,
    paperReason: 'a dissertation title page',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  },
  k: {
    id: 'k',
    files: [
      {
        path: '/lib/book.pdf',
        name: 'book.pdf',
        size: 1,
        mtimeMs: 0,
        fingerprint: 'j',
        missing: false,
      },
    ],
    doi: null,
    arxiv: null,
    isbn: '9780521635035',
    meta: {
      title: { value: 'Stochastic Processes in Physics', source: 'first-page' },
      authors: null,
      year: null,
      venue: null,
      volume: null,
      issue: null,
      pages: null,
      publisher: null,
      abstract: null,
      kind: { value: 'book', source: 'first-page' },
    },
    citeKey: 'stochastic',
    citeKeyPinned: false,
    projectIds: [],
    addedAt: WHEN,
    updatedAt: WHEN,
    incomplete: ['authors', 'year'],
    isPaper: true,
    paperReason: 'isbn 9780521635035 on the first pages',
    paperOverride: null,
    candidates: [],
    mergedCount: 0,
    autoMerged: [],
  },
};

export function paperFor(id: string): Paper {
  const paper = PAPERS[id];
  if (!paper) throw { code: 'not_found', message: `no paper ${id}` };
  return structuredClone(paper);
}

/** The subset of EngineApi the Projects screen touches, as mocks. */
export function fakeEngine() {
  return {
    libraryPapers: vi.fn(async (filter: PaperFilter) => {
      let rows = ROWS;
      // The engine's rule: a project lists whatever was filed in it; every
      // other scope is papers only, unless `others` asks for the rest.
      if (filter.project) rows = rows.filter((r) => r.projectIds.includes(filter.project!));
      else if (filter.others) rows = rows.filter((r) => !r.isPaper);
      else if (filter.unfiled) rows = rows.filter((r) => r.isPaper && r.projectIds.length === 0);
      else rows = rows.filter((r) => r.isPaper);
      if (filter.query) {
        const q = filter.query.toLowerCase();
        rows = rows.filter((r) => r.title.toLowerCase().includes(q));
      }
      return { rows, total: rows.length };
    }),
    libraryCounts: vi.fn(async () => COUNTS),
    libraryPaper: vi.fn(async (id: string) => paperFor(id)),
    libraryPaperForPath: vi.fn(async () => null),
    libraryUpdatePaper: vi.fn(async (id: string, patch: MetaPatch) => {
      const paper = paperFor(id);
      if (patch.title !== undefined) {
        paper.meta.title = patch.title === null ? null : { value: patch.title, source: 'user' };
      }
      if (patch.isbn !== undefined) paper.isbn = patch.isbn;
      if (patch.isPaper !== undefined) {
        // The engine's answer in miniature: an override is the verdict and
        // its reason. The stored record already carries the scan's, so
        // handing the decision back is clearing the override alone.
        paper.paperOverride = patch.isPaper;
        if (patch.isPaper !== null) {
          paper.isPaper = patch.isPaper;
          paper.paperReason = 'marked by you';
        }
      }
      return paper;
    }),
    libraryProjects: vi.fn(async () => PROJECTS),
    libraryCreateProject: vi.fn(async (name: string): Promise<Project> => ({
      id: 'p3',
      name,
      order: 2,
      paperCount: 0,
      bibPath: null,
      incompleteCount: 0,
      createdAt: WHEN,
    })),
    libraryRenameProject: vi.fn(async () => {}),
    libraryDeleteProject: vi.fn(async () => {}),
    libraryReorderProjects: vi.fn(async () => {}),
    librarySetMembership: vi.fn(async () => {}),
    libraryPreview: vi.fn(async () => null),
    libraryFileIntoInbox: vi.fn(async (): Promise<Paper> => paperFor('a')),
    libraryBibtex: vi.fn(async () => '@article{jarzynski1997nonequilibrium,}'),
    libraryExportBib: vi.fn(async () => {}),
    librarySetProjectBib: vi.fn(async () => {}),
    libraryLookup: vi.fn(async (): Promise<LookupResult> => ({ kind: 'nothing' })),
    libraryApplyCandidate: vi.fn(async (id: string) => paperFor(id)),
    libraryLookupProject: vi.fn(async (): Promise<LookupBatch> => ({
      attempted: 2,
      applied: 1,
      skipped: 0,
      failed: 1,
      parked: 0,
    })),
    libraryLookupAll: vi.fn(async (): Promise<LookupBatch> => ({
      attempted: 3,
      applied: 1,
      skipped: 1,
      failed: 0,
      parked: 1,
    })),
    libraryDiscardCandidates: vi.fn(async () => {}),
    librarySetLookup: vi.fn(async () => {}),
    libraryCitationLinks: vi.fn(async () => ({ cites: [], citedBy: [], unresolved: 0 })),
    libraryGraph: vi.fn(async (projectId: string | null): Promise<Graph> => {
      if (projectId === null || projectId === 'p1') return GRAPH;
      // p2 holds one paper: a node and nothing between.
      if (projectId === 'p2') return { nodes: [ROWS[0]!], edges: [], external: 1, unindexed: 0 };
      return { nodes: [], edges: [], external: 0, unindexed: 0 };
    }),
    libraryMissingWorks: vi.fn(
      async (_projectId: string | null, minCitedBy: number): Promise<MissingWork[]> =>
        MISSING.filter((w) => w.citedBy.length >= minCitedBy)
    ),
    libraryDuplicates: vi.fn(async (): Promise<DuplicateGroup[]> => fixture.duplicates),
    libraryMergePapers: vi.fn(async (keepId: string): Promise<Paper> => {
      const paper = paperFor(keepId);
      paper.mergedCount = 1;
      return paper;
    }),
    libraryUnmerge: vi.fn(async (): Promise<Paper[]> => [paperFor('e'), paperFor('a')]),
    libraryNotDuplicates: vi.fn(async (_ids: string[]): Promise<void> => undefined),
    librarySetPrimaryFile: vi.fn(async (id: string, path: string): Promise<Paper> => {
      const paper = paperFor(id);
      const at = paper.files.findIndex((file) => file.path === path);
      if (at > 0) paper.files.unshift(...paper.files.splice(at, 1));
      return paper;
    }),
    openExternalUrl: vi.fn(async () => {}),
    libraryDataDir: vi.fn(async () => ({ dir: '/data', isDefault: true })),
    libraryMoveData: vi.fn(async () => ({ dir: '/data', isDefault: true })),
    onLibraryChanged: vi.fn(async () => () => {}),
    librarySearch: vi.fn(async () => ({ documents: [], truncated: false })),
    libraryStatus: vi.fn(async () => ({
      roots: [],
      indexed: 0,
      total: 0,
      scanning: false,
      textIndexed: 0,
      textTotal: 0,
    })),
    changeLibraryRoot: vi.fn(async () => {}),
  };
}

export function isEngineError(e: unknown): boolean {
  return typeof e === 'object' && e !== null && 'code' in e && 'message' in e;
}

/** A localStorage stand-in for tests: under Node the global one may be
 * missing or inert, so tests that persist stub this in with
 * `vi.stubGlobal('localStorage', memoryStorage())`. */
export function memoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (key: string) => map.get(key) ?? null,
    key: (index: number) => [...map.keys()][index] ?? null,
    removeItem: (key: string) => void map.delete(key),
    setItem: (key: string, value: string) => void map.set(key, String(value)),
  };
}

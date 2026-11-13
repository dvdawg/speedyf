import { describe, expect, it } from 'vitest';
import {
  API_PERMISSIONS,
  apiRefusalReason,
  canCallApi,
  readCitationArg,
  readIdArg,
  readPageArg,
  readQueryArg,
  toBox,
  toCitations,
  toFigures,
  toJumps,
  toLibraryHits,
  toLinks,
  toOutline,
  toPageRuns,
  toPageText,
  toPaper,
  toPapers,
  toProjects,
  toResolvedCitation,
} from './api';
import type {
  LibrarySearchResult,
  PageLinks,
  PageTextLayout,
  Paper,
  PaperList,
  Project,
} from '../../types/engine';

const paper: Paper = {
  id: '7c1e0b5a',
  files: [
    {
      path: '/Users/someone/papers/noneq/jarzynski1997.pdf',
      name: 'jarzynski1997.pdf',
      size: 310221,
      mtimeMs: 1699000000000,
      fingerprint: 'f1a2',
      missing: false,
    },
    {
      path: '/Users/someone/papers/noneq/jarzynski1997-arxiv.pdf',
      name: 'jarzynski1997-arxiv.pdf',
      size: 1,
      mtimeMs: 0,
      fingerprint: null,
      missing: true,
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
  citeKeyPinned: true,
  projectIds: ['3b9d'],
  addedAt: '2026-11-11T20:00:00Z',
  updatedAt: '2026-11-11T20:05:00Z',
  incomplete: [],
  isPaper: true,
  paperReason: 'doi 10.1103/physrevlett.78.2690 on the first page',
  paperOverride: null,
  candidates: [],
  mergedCount: 0,
};

const projects: Project[] = [
  {
    id: '3b9d',
    name: 'Noneq stat mech',
    order: 0,
    paperCount: 1,
    bibPath: '/Users/someone/papers/noneq/refs.bib',
    incompleteCount: 0,
    createdAt: '2026-11-11T20:00:00Z',
  },
];

const list: PaperList = {
  rows: [
    {
      id: '7c1e0b5a',
      title: 'Nonequilibrium Equality for Free Energy Differences',
      authors: 'Jarzynski',
      year: 1997,
      venue: 'Physical Review Letters',
      citeKey: 'jarzynski1997nonequilibrium',
      alphaLabel: 'Jar97',
      projectIds: ['3b9d'],
      missing: false,
      hasIdentifier: true,
      incompleteCount: 0,
      isPaper: true,
      candidateCount: 0,
    },
  ],
  total: 1,
};

describe('the API catalogue', () => {
  it('requires a permission for every API it offers', () => {
    for (const [api, permission] of Object.entries(API_PERMISSIONS)) {
      expect(permission, `${api} has no permission`).toBeTruthy();
      expect(canCallApi([permission], api)).toBe(true);
    }
  });

  it('puts page navigation under the same permission as running commands', () => {
    expect(canCallApi(['commands:run'], 'navigate.toPage')).toBe(true);
    expect(canCallApi(['document:read'], 'navigate.toPage')).toBe(false);
  });

  it('refuses an API the extension did not ask for', () => {
    expect(canCallApi(['library:read'], 'document.info')).toBe(false);
    expect(canCallApi([], 'document.info')).toBe(false);
  });

  it('puts the paper records under the library permission', () => {
    for (const api of ['library.papers', 'library.projects', 'library.paper']) {
      expect(canCallApi(['library:read'], api), api).toBe(true);
      expect(canCallApi(['document:read', 'citations:read'], api), api).toBe(false);
      expect(canCallApi([], api), api).toBe(false);
      expect(apiRefusalReason([], api)).toContain('library:read');
    }
  });

  /** A typo must be an error, never an accidental grant. */
  it('refuses an API that does not exist, whatever is granted', () => {
    expect(canCallApi(['document:read', 'library:read'], 'document.delete')).toBe(false);
    expect(apiRefusalReason([], 'document.delete')).toMatch(/not an API/);
  });

  it('names the permission needed when one is missing', () => {
    expect(apiRefusalReason([], 'citations.list')).toContain('citations:read');
  });

  /** Paths and internal ids are another permission's business. */
  it('exposes no filesystem paths or engine ids', () => {
    const surface = JSON.stringify([
      toResolvedCitation({
        docId: 7,
        title: 'A Paper',
        pageCount: 12,
        fileName: 'paper.pdf',
        path: '/Users/someone/papers/paper.pdf',
        preview: { docId: 7, src: 0, tile: { x: 0, y: 0, w: 1, h: 1 }, scaleMilli: 500, text: '' },
      }),
      toLibraryHits({
        documents: [
          {
            path: '/Users/someone/papers/other.pdf',
            name: 'other.pdf',
            title: 'Other',
            totalMatches: 3,
            pages: [],
          },
        ],
        truncated: false,
      } as LibrarySearchResult),
    ]);
    expect(surface).not.toContain('/Users/');
    expect(surface).not.toContain('docId');
  });

  /** A paper record is organised around its files on disk, and a project may
   * carry a synced .bib path. Neither location crosses the boundary. */
  it('exposes no paths from the paper records', () => {
    const surface = JSON.stringify([toPaper(paper), toPapers(list), toProjects(projects)]);
    expect(surface).not.toContain('/Users/');
    expect(surface).not.toContain('"path"');
    expect(surface).not.toContain('bibPath');
    expect(surface).not.toContain('fingerprint');
  });
});

describe('toPaper', () => {
  it('reduces each file to its name and whether it is still there', () => {
    expect(toPaper(paper).files).toEqual([
      { name: 'jarzynski1997.pdf', missing: false },
      { name: 'jarzynski1997-arxiv.pdf', missing: true },
    ]);
  });

  /** The provenance is the point of the full record: it is how an extension
   * tells a title the user typed from one a heuristic guessed. */
  it('keeps the metadata with its sources, the identifiers and the key', () => {
    const detail = toPaper(paper);
    expect(detail.meta.title).toEqual({
      value: 'Nonequilibrium Equality for Free Energy Differences',
      source: 'crossref',
    });
    expect(detail.doi).toBe('10.1103/physrevlett.78.2690');
    expect(detail.citeKey).toBe('jarzynski1997nonequilibrium');
    expect(detail.citeKeyPinned).toBe(true);
    expect(detail.projectIds).toEqual(['3b9d']);
    expect(detail.isPaper).toBe(true);
  });

  /** The verdict crosses, its reason and the parked lookup matches do not:
   * the reason is the scan's working, and the matches are a question for
   * the user, not data about the paper. */
  it('reports the verdict and nothing of how it was reached', () => {
    const detail = toPaper({ ...paper, isPaper: false, paperReason: 'no abstract' });
    expect(detail.isPaper).toBe(false);
    expect(JSON.stringify(detail)).not.toContain('paperReason');
    expect(JSON.stringify(detail)).not.toContain('candidates');
  });
});

describe('toPapers', () => {
  it('reports the display row with its key and projects', () => {
    expect(toPapers(list)).toEqual({
      papers: [
        {
          id: '7c1e0b5a',
          title: 'Nonequilibrium Equality for Free Energy Differences',
          authors: 'Jarzynski',
          year: 1997,
          venue: 'Physical Review Letters',
          citeKey: 'jarzynski1997nonequilibrium',
          projectIds: ['3b9d'],
          isPaper: true,
        },
      ],
    });
  });
});

describe('toProjects', () => {
  it('reports a project by id, name and size only', () => {
    expect(toProjects(projects)).toEqual([{ id: '3b9d', name: 'Noneq stat mech', paperCount: 1 }]);
  });
});

describe('toOutline', () => {
  it('keeps nesting and drops engine-only fields', () => {
    expect(
      toOutline([
        {
          title: 'One',
          page: 0,
          y: 700,
          children: [{ title: 'One A', page: 1, y: 500, children: [] }],
        },
      ])
    ).toEqual([{ title: 'One', page: 0, children: [{ title: 'One A', page: 1, children: [] }] }]);
  });

  it('carries a missing destination through as null', () => {
    expect(toOutline([{ title: 'Front matter', page: null, y: null, children: [] }])[0]!.page).toBe(
      null
    );
  });
});

describe('toFigures', () => {
  it('reports the label and caption without the crop geometry', () => {
    expect(
      toFigures([
        {
          label: 'Figure 3',
          title: 'Architecture of the model',
          page: 6,
          y: 300,
          tile: { x: 0, y: 0, w: 100, h: 100 },
          scaleMilli: 500,
        },
      ])
    ).toEqual([{ label: 'Figure 3', caption: 'Architecture of the model', page: 6 }]);
  });
});

describe('toPageText', () => {
  const layout = (texts: string[]): PageTextLayout => ({
    src: 0,
    charCount: 0,
    runs: texts.map((text, i) => ({ text, start: i, x: 0, y: 0, w: 0, h: 0 })),
  });

  it('joins runs in reading order', () => {
    expect(toPageText(layout(['Attention', 'Is', 'All'])).trim()).toBe('Attention Is All');
  });

  it('does not double a space a run already carries', () => {
    expect(toPageText(layout(['Attention ', 'Is']))).toBe('Attention Is');
    expect(toPageText(layout(['Attention', ' Is']))).toBe('Attention Is');
  });

  it('reads an absent layout as empty rather than throwing', () => {
    expect(toPageText(null)).toBe('');
  });
});

describe('toCitations', () => {
  const links = (targets: PageLinks['links']): PageLinks => ({ src: 4, links: targets });

  it('keeps only links that carry a citation', () => {
    const result = toCitations(
      links([
        {
          rect: [0, 0, 1, 1],
          target: {
            kind: 'uri',
            uri: 'https://doi.org/10.1/x',
            citation: { scheme: 'doi', value: '10.1/x' },
          },
        },
        { rect: [0, 0, 1, 1], target: { kind: 'uri', uri: 'https://example.com', citation: null } },
        { rect: [0, 0, 1, 1], target: { kind: 'internal', page: 2, x: null, y: null } },
        { rect: [0, 0, 1, 1], target: { kind: 'unknown' } },
      ])
    );
    expect(result).toEqual([{ kind: 'doi', value: '10.1/x', page: 4 }]);
  });

  /** Extensions should not each have to write the same dedupe. */
  it('collapses the same reference cited twice on a page', () => {
    const cite = {
      rect: [0, 0, 1, 1] as [number, number, number, number],
      target: {
        kind: 'uri' as const,
        uri: 'https://arxiv.org/abs/1706.03762',
        citation: { scheme: 'arXiv' as const, value: '1706.03762' },
      },
    };
    expect(toCitations(links([cite, cite]))).toHaveLength(1);
  });

  /** Authors should not have to learn our internal spelling of arXiv. */
  it('normalises the scheme to lowercase', () => {
    const result = toCitations(
      links([
        {
          rect: [0, 0, 1, 1],
          target: {
            kind: 'uri',
            uri: 'https://arxiv.org/abs/1706.03762',
            citation: { scheme: 'arXiv', value: '1706.03762' },
          },
        },
      ])
    );
    expect(result[0]!.kind).toBe('arxiv');
  });
});

describe('toLinks', () => {
  const uri = (
    u: string,
    citation: PageLinks['links'][number]['target'] extends never ? never : unknown = null
  ) => ({
    rect: [0, 0, 1, 1] as [number, number, number, number],
    target: { kind: 'uri' as const, uri: u, citation: citation as never },
  });

  it('reports links that carry no identifier, which is most of a reference list', () => {
    const result = toLinks({
      src: 7,
      links: [
        uri('https://jmlr.org/papers/v20/18-598.html'),
        uri('https://books.google.com/books?id=abc'),
      ],
    } as PageLinks);
    expect(result.map((l) => l.host)).toEqual(['jmlr.org', 'books.google.com']);
    expect(result.every((l) => l.citation === null)).toBe(true);
    expect(result[0]!.page).toBe(7);
  });

  it('fills in the citation for the schemes SpeedyF recognises', () => {
    const result = toLinks({
      src: 7,
      links: [
        {
          rect: [0, 0, 1, 1],
          target: {
            kind: 'uri',
            uri: 'https://arxiv.org/abs/1706.03762',
            citation: { scheme: 'arXiv', value: '1706.03762' },
          },
        },
      ],
    } as PageLinks);
    expect(result[0]!.citation).toEqual({ kind: 'arxiv', value: '1706.03762', page: 7 });
  });

  /** The engine finds a bibliography URL twice — once as an annotation, once
   * read out of the page text — and both are the same link. */
  it('deduplicates a destination found twice in the same place', () => {
    const result = toLinks({
      src: 0,
      links: [uri('https://jmlr.org/a'), uri('https://jmlr.org/a')],
    } as PageLinks);
    expect(result).toHaveLength(1);
  });

  /** …but the same address printed in two entries is two links, and
   * collapsing them moved the second one's position onto the first, which put
   * a reference in the wrong entry. */
  it('keeps a destination that appears twice in different places', () => {
    const result = toLinks({
      src: 0,
      links: [
        {
          rect: [10, 700, 40, 8],
          target: { kind: 'uri', uri: 'https://a.test/x', citation: null },
        },
        {
          rect: [10, 400, 40, 8],
          target: { kind: 'uri', uri: 'https://a.test/x', citation: null },
        },
      ],
    } as PageLinks);
    expect(result.map((link) => link.box.y)).toEqual([708, 408]);
  });

  it('reports where each link sits, with the top edge uppermost', () => {
    const result = toLinks({
      src: 0,
      links: [
        { rect: [12, 300, 40, 9], target: { kind: 'uri', uri: 'https://a.test', citation: null } },
      ],
    } as PageLinks);
    expect(result[0]!.box).toEqual({ x: 12, y: 309, w: 40, h: 9 });
  });

  it('reports a link with no parseable host rather than dropping it', () => {
    const result = toLinks({ src: 0, links: [uri('mailto:someone@example.com')] } as PageLinks);
    expect(result).toHaveLength(1);
    expect(result[0]!.host).toBe(null);
  });

  /** `document.links` means "somewhere this document points outside itself",
   * and every consumer of it assumes a `uri`. Jumps have their own call. */
  it('ignores internal links', () => {
    const result = toLinks({
      src: 0,
      links: [{ rect: [0, 0, 1, 1], target: { kind: 'internal', page: 3, x: null, y: null } }],
    } as PageLinks);
    expect(result).toEqual([]);
  });
});

describe('toBox', () => {
  /** The engine measures from the bottom edge; reading order wants the top,
   * so that sorting down a page is descending `y`. */
  it('turns a bottom-edge rect into a top-edge box', () => {
    expect(toBox([10, 200, 30, 12])).toEqual({ x: 10, y: 212, w: 30, h: 12 });
  });
});

describe('toJumps', () => {
  const jump = (x: number | null, y: number | null) => ({
    rect: [5, 100, 20, 10] as [number, number, number, number],
    target: { kind: 'internal' as const, page: 14, x, y },
  });

  it('reports where a jump starts and where it lands', () => {
    const result = toJumps({ src: 3, links: [jump(120, 655)] } as PageLinks);
    expect(result).toEqual([
      { page: 3, box: { x: 5, y: 110, w: 20, h: 10 }, toPage: 14, toX: 120, toY: 655 },
    ]);
  });

  /** A destination fixing only a page still says something worth knowing, and
   * dropping it would look like the citation is not there at all. */
  it('carries a destination with no position through as null', () => {
    const result = toJumps({ src: 0, links: [jump(null, null)] } as PageLinks);
    expect(result[0]!.toY).toBe(null);
    expect(result[0]!.toPage).toBe(14);
  });

  it('ignores outbound links', () => {
    const result = toJumps({
      src: 0,
      links: [
        { rect: [0, 0, 1, 1], target: { kind: 'uri', uri: 'https://a.test', citation: null } },
      ],
    } as PageLinks);
    expect(result).toEqual([]);
  });

  /** Two `[12]`s in one paragraph are two citations of the same work, and a
   * caller counting citation sites wants both. */
  it('keeps repeated jumps to the same destination', () => {
    const result = toJumps({ src: 0, links: [jump(1, 2), jump(1, 2)] } as PageLinks);
    expect(result).toHaveLength(2);
  });
});

describe('toPageRuns', () => {
  const layout = {
    src: 0,
    charCount: 0,
    runs: [
      { text: 'Fukumizu', start: 0, x: 40, y: 700, w: 50, h: 9 },
      { text: '   ', start: 8, x: 90, y: 700, w: 3, h: 9 },
      { text: 'Effect of', start: 11, x: 40, y: 688, w: 45, h: 9 },
    ],
  } as PageTextLayout;

  it('reports each run with its place on the page', () => {
    expect(toPageRuns(layout)).toEqual([
      { text: 'Fukumizu', box: { x: 40, y: 709, w: 50, h: 9 } },
      { text: 'Effect of', box: { x: 40, y: 697, w: 45, h: 9 } },
    ]);
  });

  it('reads an absent layout as no runs rather than throwing', () => {
    expect(toPageRuns(null)).toEqual([]);
  });
});

describe('toResolvedCitation', () => {
  it('turns "not found" into an explicit answer', () => {
    expect(toResolvedCitation(null)).toEqual({
      inLibrary: false,
      title: null,
      fileName: null,
      pageCount: null,
      citeKey: null,
    });
  });

  const resolved = {
    docId: 7,
    title: 'A Paper',
    pageCount: 12,
    fileName: 'paper.pdf',
    path: '/Users/someone/papers/paper.pdf',
    preview: { docId: 7, src: 0, tile: { x: 0, y: 0, w: 1, h: 1 }, scaleMilli: 500, text: '' },
  };

  /** The engine folds the key into its answer, so `citations.resolve` is one
   * engine call, not two. */
  it('takes the cite key from the engine’s answer', () => {
    expect(toResolvedCitation({ ...resolved, citeKey: 'zhong2024beyond' }).citeKey).toBe(
      'zhong2024beyond'
    );
  });

  it('carries the cite key when the library has a record', () => {
    expect(toResolvedCitation(resolved, 'jarzynski1997nonequilibrium').citeKey).toBe(
      'jarzynski1997nonequilibrium'
    );
  });

  /** In the index but not yet a paper: the file is there, the key is not. */
  it('reports a null key for a file with no paper record', () => {
    const result = toResolvedCitation(resolved);
    expect(result.inLibrary).toBe(true);
    expect(result.citeKey).toBe(null);
  });
});

describe('argument reading', () => {
  it('accepts an in-range page', () => {
    expect(readPageArg({ page: 3 }, 10)).toBe(3);
    expect(readPageArg({ page: 0 }, 1)).toBe(0);
  });

  it('refuses anything that is not a page', () => {
    for (const bad of [{}, { page: '3' }, { page: 1.5 }, { page: -1 }, { page: 10 }, undefined]) {
      expect(() => readPageArg(bad, 10)).toThrow();
    }
  });

  it('reads a citation back into the engine spelling', () => {
    expect(readCitationArg({ kind: 'doi', value: '10.1/x' })).toEqual({
      scheme: 'doi',
      value: '10.1/x',
    });
    expect(readCitationArg({ kind: 'arxiv', value: '1706.03762' })).toEqual({
      scheme: 'arXiv',
      value: '1706.03762',
    });
  });

  it('refuses a malformed citation', () => {
    for (const bad of [{}, { kind: 'doi' }, { kind: 'isbn', value: 'x' }, { value: 'x' }]) {
      expect(() => readCitationArg(bad)).toThrow();
    }
  });

  it('reads a record id and refuses anything else', () => {
    expect(readIdArg({ id: '7c1e0b5a' })).toBe('7c1e0b5a');
    for (const bad of [{}, { id: '' }, { id: '  ' }, { id: 7 }, undefined]) {
      expect(() => readIdArg(bad)).toThrow();
    }
  });

  it('refuses an empty query', () => {
    expect(readQueryArg({ query: 'transformers' })).toBe('transformers');
    for (const bad of [{}, { query: '' }, { query: '   ' }, { query: 7 }]) {
      expect(() => readQueryArg(bad)).toThrow();
    }
  });
});

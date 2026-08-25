import { describe, expect, it } from 'vitest';
import {
  API_PERMISSIONS,
  apiRefusalReason,
  canCallApi,
  readCitationArg,
  readPageArg,
  readQueryArg,
  toCitations,
  toFigures,
  toLibraryHits,
  toLinks,
  toOutline,
  toPageText,
  toResolvedCitation,
} from './api';
import type { LibrarySearchResult, PageLinks, PageTextLayout } from '../../types/engine';

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

  /** The same URL printed twice on a page is one link. */
  it('deduplicates by destination', () => {
    const result = toLinks({
      src: 0,
      links: [uri('https://jmlr.org/a'), uri('https://jmlr.org/a')],
    } as PageLinks);
    expect(result).toHaveLength(1);
  });

  it('reports a link with no parseable host rather than dropping it', () => {
    const result = toLinks({ src: 0, links: [uri('mailto:someone@example.com')] } as PageLinks);
    expect(result).toHaveLength(1);
    expect(result[0]!.host).toBe(null);
  });

  it('ignores internal links', () => {
    const result = toLinks({
      src: 0,
      links: [{ rect: [0, 0, 1, 1], target: { kind: 'internal', page: 3, x: null, y: null } }],
    } as PageLinks);
    expect(result).toEqual([]);
  });
});

describe('toResolvedCitation', () => {
  it('turns "not found" into an explicit answer', () => {
    expect(toResolvedCitation(null)).toEqual({
      inLibrary: false,
      title: null,
      fileName: null,
      pageCount: null,
    });
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

  it('refuses an empty query', () => {
    expect(readQueryArg({ query: 'transformers' })).toBe('transformers');
    for (const bad of [{}, { query: '' }, { query: '   ' }, { query: 7 }]) {
      expect(() => readQueryArg(bad)).toThrow();
    }
  });
});

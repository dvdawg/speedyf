import { describe, expect, it, vi } from 'vitest';
import {
  CONTRIBUTE_TERMS,
  TermSession,
  chooseTermProvider,
  readTermArgs,
  toEngineTerms,
  type TermDeps,
} from './termProvider';

const ext = (over: Partial<Parameters<typeof chooseTermProvider>[0][number]> = {}) => ({
  id: 'dev.speedyf.notation',
  version: '1.0.0',
  enabled: true,
  error: null,
  permissions: [CONTRIBUTE_TERMS],
  termProvider: 'terms',
  ...over,
});

describe('chooseTermProvider', () => {
  it('takes the first enabled extension declaring one with the permission', () => {
    expect(chooseTermProvider([ext()])).toEqual({
      extensionId: 'dev.speedyf.notation',
      version: '1.0.0',
      command: 'terms',
    });
    expect(chooseTermProvider([ext({ enabled: false })])).toBeNull();
    expect(chooseTermProvider([ext({ permissions: [] })])).toBeNull();
    expect(chooseTermProvider([ext({ termProvider: null })])).toBeNull();
    expect(chooseTermProvider([ext({ error: 'crashed' })])).toBeNull();
  });
});

describe('readTermArgs', () => {
  it('keeps well-formed terms and drops malformed ones one by one', () => {
    const { document, terms } = readTermArgs(
      {
        document: 'terms-1',
        terms: [
          { text: 'FDT', body: 'fluctuation-dissipation theorem', page: 2 },
          { text: 'A', body: 'single letters are never terms', page: 0 },
          { text: 'IT', body: 'information technology', page: 9 },
          { text: 'NESS', body: '', page: 0 },
          { text: ' PS', body: 'padded', page: 0 },
          { text: 'QoL', body: 'quality of life', page: 1, caseSensitive: false, title: 'QoL' },
          { text: 'mRNA', body: 'messenger RNA', page: 1, box: { x: 1, y: 2, w: 3, h: Infinity } },
          'not an object',
        ],
      },
      5
    );
    expect(document).toBe('terms-1');
    expect(terms.map((t) => t.text)).toEqual(['FDT', 'QoL', 'mRNA']);
    expect(terms[0]!.caseSensitive).toBe(true);
    expect(terms[0]!.title).toBe('FDT');
    expect(terms[1]!.caseSensitive).toBe(false);
    expect(terms[2]!.box).toBeNull();
  });

  it('needs the document token and a terms array', () => {
    expect(() => readTermArgs({ terms: [] }, 3)).toThrow(/document/);
    expect(() => readTermArgs({ document: 'x' }, 3)).toThrow(/terms/);
  });
});

describe('toEngineTerms', () => {
  it('maps reader pages to file pages and top-edge boxes to bottom-left rects', () => {
    const terms = readTermArgs(
      {
        document: 'd',
        terms: [
          { text: 'FDT', body: 'x', page: 0, box: { x: 10, y: 700, w: 50, h: 12 } },
          { text: 'NESS', body: 'y', page: 1 },
        ],
      },
      2
    ).terms;
    const out = toEngineTerms(terms, (page) => (page === 0 ? 4 : null));
    expect(out).toEqual([
      {
        text: 'FDT',
        caseSensitive: true,
        kind: null,
        title: 'FDT',
        body: 'x',
        targetPage: 4,
        targetRect: [10, 688, 50, 12],
      },
    ]);
  });
});

function session() {
  const deps = {
    invoke: vi.fn(async () => {}),
    store: vi.fn(async (_doc: number, terms: unknown[]) => terms.length),
    clearAll: vi.fn(async () => {}),
    log: vi.fn(),
  } satisfies TermDeps;
  return { deps, terms: new TermSession(deps) };
}

const view = (docId: number) => ({ docId, pageCount: 3, srcOf: (page: number) => page });
const choice = { extensionId: 'dev.speedyf.notation', version: '1.0.0', command: 'terms' };
const one = (document: string) => ({
  document,
  terms: [{ text: 'FDT', body: 'fluctuation-dissipation theorem', page: 0 }],
});

describe('TermSession', () => {
  it('asks once per document on screen and stores the answer', async () => {
    const { deps, terms } = session();
    terms.setProvider(choice);
    terms.noteActive(7);
    terms.ensure(7);
    expect(deps.invoke).toHaveBeenCalledTimes(1);
    const token = (deps.invoke.mock.calls[0] as unknown as [unknown, { document: string }])[1]
      .document;
    expect(await terms.contribute(choice.extensionId, one(token), view(7))).toEqual({
      stored: 1,
    });
    expect(deps.store).toHaveBeenCalledWith(7, expect.any(Array));
  });

  it('voids an answer once the reader has switched documents', async () => {
    const { deps, terms } = session();
    terms.setProvider(choice);
    terms.noteActive(7);
    const token = (deps.invoke.mock.calls[0] as unknown as [unknown, { document: string }])[1]
      .document;
    terms.noteActive(8);
    terms.noteActive(7);
    // The old token read pages from whatever was open meanwhile.
    expect(await terms.contribute(choice.extensionId, one(token), view(7))).toEqual({
      stored: 0,
    });
    expect(deps.store).not.toHaveBeenCalled();
  });

  it('refuses answers from anyone but the active provider', async () => {
    const { deps, terms } = session();
    terms.setProvider(choice);
    terms.noteActive(7);
    const token = (deps.invoke.mock.calls[0] as unknown as [unknown, { document: string }])[1]
      .document;
    await expect(terms.contribute('com.example.other', one(token), view(7))).rejects.toThrow();
  });

  it('takes every term back when the provider is switched off', () => {
    const { deps, terms } = session();
    terms.setProvider(choice);
    terms.noteActive(7);
    terms.setProvider(null);
    expect(deps.clearAll).toHaveBeenCalledTimes(1);
  });
});

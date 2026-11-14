/** Drives the shipped Math to LaTeX worker the way the host does, against a
 * fake host: canned glyphs, tags, identifiers and network answers, and a
 * record of what reached `ui.showMath`.
 *
 * The order is the feature: the document's own tags beat the arXiv source,
 * which beats the journal's JATS, which beats reading the glyphs. Settings
 * turn steps off, a failing network falls through to the next step, and a
 * scan is refused rather than guessed at.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.mathlatex/worker.js'),
  'utf8'
);

interface Glyph {
  text: string;
  code: number;
  font: string;
  size: number;
  x: number;
  y: number;
  w: number;
  h: number;
  baseline: number;
  bold: boolean;
  italic: boolean;
}

function g(font: string, code: number, x: number, baseline: number, size = 10, text = ''): Glyph {
  return {
    text,
    code,
    font,
    size,
    x,
    y: baseline - 0.7 * size,
    w: 0.55 * size,
    h: 0.9 * size,
    baseline,
    bold: false,
    italic: false,
  };
}

/** e^{-\beta E} as pdfTeX sets it. */
const BOLTZMANN: Glyph[] = [
  g('CMMI10', 101, 100, 200, 10, 'e'),
  g('CMSY7', 0, 104.7, 196.4, 7, '−'),
  g('CMMI7', 12, 110.9, 196.4, 7, 'β'),
  g('CMMI7', 69, 115, 196.4, 7, 'E'),
];

/** x = a + b over = c, relations lined up. */
const ALIGNED: Glyph[] = [
  g('CMMI10', 120, 100, 200),
  { ...g('CMR10', 61, 108.5, 200), w: 7.8 },
  g('CMMI10', 97, 119, 200),
  { ...g('CMR10', 43, 127, 200), w: 7.8 },
  g('CMMI10', 98, 137, 200),
  { ...g('CMR10', 61, 108.5, 215), w: 7.8 },
  g('CMMI10', 99, 119, 215),
];

interface Fetch {
  url: string;
  as: string;
}
interface HostSetup {
  selection?: { page: number; glyphs: Glyph[] } | null;
  region?: Glyph[];
  tags?: unknown[];
  identifiers?: { doi: string | null; arxiv: string | null; arxivVersion: number | null };
  settings?: Record<string, string>;
  fetch?: (req: Fetch) => Promise<{ status: number; contentType: string; body: unknown }>;
  /** Told of every call before it is answered. */
  onCall?: (api: string) => void;
}

function startWorker(setup: HostSetup) {
  const shown: Record<string, unknown>[] = [];
  const fetched: string[] = [];
  const logs: string[] = [];
  const storage = new Map<string, string>();
  const calls: string[] = [];
  const settings = { arxiv: 'true', journals: 'true', prefer: 'most exact', ...setup.settings };

  const fake = {
    onmessage: undefined as ((e: { data: unknown }) => void) | undefined,
    postMessage(message: Record<string, unknown>) {
      if (message.type === 'log') logs.push(String(message.message));
      if (message.type !== 'call') return;
      void answer(message);
    },
  };

  async function answer(message: Record<string, unknown>) {
    const args = (message.args ?? {}) as Record<string, unknown>;
    const api = String(message.api);
    calls.push(api);
    setup.onCall?.(api);
    let value: unknown = null;
    let ok = true;
    let error = '';
    try {
      switch (api) {
        case 'settings.get':
          value = settings[String(args.key) as keyof typeof settings];
          break;
        case 'selection.glyphs':
          value = setup.selection ?? null;
          break;
        case 'region.glyphs':
          value = { page: args.page, glyphs: setup.region ?? [] };
          break;
        case 'document.mathTags':
          value = setup.tags ?? [];
          break;
        case 'document.identifiers':
          value = setup.identifiers ?? { doi: null, arxiv: null, arxivVersion: null };
          break;
        case 'net.fetch': {
          const req = args as unknown as Fetch;
          fetched.push(req.url);
          if (!setup.fetch) throw new Error('offline');
          value = await setup.fetch(req);
          break;
        }
        case 'storage.get':
          value = storage.get(String(args.key)) ?? null;
          break;
        case 'storage.set':
          storage.set(String(args.key), String(args.value));
          break;
        case 'ui.showMath':
          shown.push(args);
          value = { shown: true };
          break;
        default:
          throw new Error(`unexpected ${api}`);
      }
    } catch (err) {
      ok = false;
      error = String((err as Error).message);
    }
    fake.onmessage?.({ data: { type: 'reply', requestId: message.requestId, ok, value, error } });
  }

  new Function('self', SOURCE)(fake);

  let nextInvoke = 50_000;
  return {
    shown,
    fetched,
    logs,
    storage,
    calls,
    invoke(command: string, value?: unknown) {
      return new Promise<void>((resolve, reject) => {
        const requestId = nextInvoke++;
        const post = fake.postMessage.bind(fake);
        fake.postMessage = (message: Record<string, unknown>) => {
          if (message.type === 'done' && message.requestId === requestId) {
            fake.postMessage = post;
            if (message.ok) resolve();
            else reject(new Error(String(message.error)));
            return;
          }
          post(message);
        };
        fake.onmessage?.({ data: { type: 'invoke', command, requestId, value } });
      });
    },
  };
}

// ---------- canned network answers ----------

function tarOf(files: Record<string, string>): Uint8Array {
  const enc = new TextEncoder();
  const parts: Uint8Array[] = [];
  for (const [name, text] of Object.entries(files)) {
    const data = enc.encode(text);
    const h = new Uint8Array(512);
    const put = (s: string, at: number) => h.set(enc.encode(s), at);
    put(name, 0);
    put('0000644\0', 100);
    put(`${data.length.toString(8).padStart(11, '0')}\0`, 124);
    put('        ', 148);
    put('0', 156);
    put('ustar\0', 257);
    let sum = 0;
    for (const b of h) sum += b;
    put(`${sum.toString(8).padStart(6, '0')}\0 `, 148);
    parts.push(h);
    const body = new Uint8Array(Math.ceil(data.length / 512) * 512);
    body.set(data);
    parts.push(body);
  }
  parts.push(new Uint8Array(1024));
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

async function gzip(bytes: Uint8Array): Promise<number[]> {
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new CompressionStream('gzip'));
  return Array.from(new Uint8Array(await new Response(stream).arrayBuffer()));
}

const PAPER = String.raw`\documentclass{article}
\newcommand{\be}{\beta}
\begin{document}
The partition function is
\begin{equation}
  Z = \sum_{s} e^{-\be E_s} \label{eq:z}
\end{equation}
\end{document}
`;

const JATS = `<?xml version="1.0"?><article xmlns:mml="http://www.w3.org/1998/Math/MathML"><body>
<p>Weights <inline-formula><mml:math><mml:msup><mml:mi>e</mml:mi><mml:mrow><mml:mo>−</mml:mo><mml:mi>β</mml:mi><mml:mi>E</mml:mi></mml:mrow></mml:msup></mml:math></inline-formula> are used.</p>
</body></article>`;

const IDS = { doi: '10.1371/journal.pone.0000001', arxiv: '2101.00001', arxivVersion: 2 };

function network(opts: { arxiv?: 'tex' | 'pdf' | 'fail'; jats?: boolean } = {}) {
  return async ({ url }: Fetch) => {
    if (url.startsWith('https://arxiv.org/e-print/')) {
      if (opts.arxiv === 'fail' || !opts.arxiv) throw new Error('network down');
      if (opts.arxiv === 'pdf') {
        return { status: 200, contentType: 'application/pdf', body: [0x25, 0x50, 0x44, 0x46] };
      }
      return {
        status: 200,
        contentType: 'application/x-eprint-tar',
        body: await gzip(tarOf({ 'paper.tex': PAPER })),
      };
    }
    if (url.includes('/europepmc/webservices/rest/search')) {
      if (!opts.jats) throw new Error('network down');
      return {
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ resultList: { result: [{ doi: IDS.doi, pmcid: 'PMC1234567' }] } }),
      };
    }
    if (url.endsWith('/PMC1234567/fullTextXML')) {
      return { status: 200, contentType: 'application/xml', body: JATS };
    }
    throw new Error(`no such host: ${url}`);
  };
}

const selection = { page: 0, glyphs: BOLTZMANN };
const tagOver = (alt: string | null, mathml: string | null = null) => ({
  bbox: [99, 190, 25, 14],
  alt,
  actualText: null,
  mathml,
});

// ---------- the pipeline ----------

describe('Math to LaTeX: the pipeline', () => {
  it('uses the tagged-PDF formula first, without touching the network', async () => {
    const w = startWorker({
      selection,
      tags: [tagOver('$e^{-\\beta E}$')],
      identifiers: IDS,
      fetch: network({ arxiv: 'tex', jats: true }),
    });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown).toEqual([
      { latex: 'e^{-\\beta E}', display: false, source: 'tags', confidence: 'high' },
    ]);
    expect(w.fetched).toEqual([]);
  });

  it('reads a tag’s MathML when its alt text is a description', async () => {
    const w = startWorker({
      selection,
      tags: [
        tagOver(
          'e to the minus beta E',
          '<math><msup><mi>e</mi><mrow><mo>−</mo><mi>β</mi><mi>E</mi></mrow></msup></math>'
        ),
      ],
    });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown[0]).toMatchObject({ latex: 'e^{-\\beta E}', source: 'tags' });
  });

  it('prefers the arXiv source to the journal version and the fonts', async () => {
    const w = startWorker({
      selection,
      identifiers: IDS,
      fetch: network({ arxiv: 'tex', jats: true }),
    });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown).toEqual([
      { latex: 'e^{-\\beta E_s}', display: false, source: 'arxiv', confidence: 'high' },
    ]);
    expect(w.fetched).toEqual(['https://arxiv.org/e-print/2101.00001v2']);
    // Small enough to keep across sessions.
    expect([...w.storage.keys()]).toEqual(['src:v1:arxiv:2101.00001v2']);
  });

  it('honours an equation number and returns the whole display', async () => {
    const w = startWorker({
      region: BOLTZMANN,
      identifiers: IDS,
      fetch: network({ arxiv: 'tex' }),
    });
    await w.invoke('copyEquation', { page: 0, bbox: [90, 180, 60, 30], number: '(1)' });
    expect(w.shown).toEqual([
      {
        latex: 'Z = \\sum_{s} e^{-\\beta E_s}',
        display: true,
        source: 'arxiv',
        confidence: 'high',
      },
    ]);
  });

  it('uses the journal version when there is no arXiv source', async () => {
    const w = startWorker({
      selection,
      identifiers: { ...IDS, arxiv: null, arxivVersion: null },
      fetch: network({ jats: true }),
    });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown).toEqual([
      { latex: 'e^{-\\beta E}', display: false, source: 'jats', confidence: 'high' },
    ]);
    expect(w.fetched[0]).toBe(
      'https://www.ebi.ac.uk/europepmc/webservices/rest/search?query=DOI:%2210.1371%2Fjournal.pone.0000001%22&format=json&resultType=lite'
    );
    expect(w.fetched[1]).toBe(
      'https://www.ebi.ac.uk/europepmc/webservices/rest/PMC1234567/fullTextXML'
    );
  });

  it('skips the arXiv step when its setting is off', async () => {
    const w = startWorker({
      selection,
      identifiers: IDS,
      settings: { arxiv: 'false' },
      fetch: network({ arxiv: 'tex', jats: true }),
    });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown[0]).toMatchObject({ source: 'jats' });
    expect(w.fetched.some((u) => u.includes('arxiv.org'))).toBe(false);
  });

  it('reads only the fonts, offline, when asked to', async () => {
    const w = startWorker({
      selection,
      identifiers: IDS,
      settings: { prefer: 'fonts only (offline)' },
      fetch: network({ arxiv: 'tex', jats: true }),
    });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown).toEqual([
      { latex: 'e^{-\\beta E}', display: false, source: 'fonts', confidence: 'high' },
    ]);
    expect(w.fetched).toEqual([]);
    expect(w.calls).not.toContain('document.identifiers');
  });

  it('falls through to the fonts when the network fails', async () => {
    const w = startWorker({ selection, identifiers: IDS, fetch: network({ arxiv: 'fail' }) });
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown).toEqual([
      { latex: 'e^{-\\beta E}', display: false, source: 'fonts', confidence: 'high' },
    ]);
    expect(w.fetched).toHaveLength(2);
    expect(w.logs.some((l) => l.includes('network down'))).toBe(true);
  });

  it('remembers a PDF-only submission and does not fetch it again', async () => {
    const w = startWorker({
      selection,
      identifiers: { ...IDS, doi: null },
      fetch: network({ arxiv: 'pdf' }),
    });
    await w.invoke('copyLatex', 'e−βE');
    await w.invoke('copyLatex', 'e−βE');
    expect(w.shown.map((s) => s.source)).toEqual(['fonts', 'fonts']);
    expect(w.fetched).toEqual(['https://arxiv.org/e-print/2101.00001v2']);
  });

  it('refuses a scan instead of guessing, and shows nothing', async () => {
    const ocr = BOLTZMANN.map((gl) => ({ ...gl, font: 'GlyphLessFont', text: 'e' }));
    const w = startWorker({ selection: { page: 0, glyphs: ocr }, identifiers: IDS });
    await expect(w.invoke('copyLatex', 'e-BE')).rejects.toThrow(
      "This looks like a scan; math can't be read from it."
    );
    expect(w.shown).toEqual([]);
    expect(w.fetched).toEqual([]);
  });

  it('calls ui.showMath with a display payload for an equation action', async () => {
    const w = startWorker({ region: ALIGNED });
    await w.invoke('copyEquation', { page: 3, bbox: [95, 190, 60, 30], number: null });
    expect(w.calls).toContain('region.glyphs');
    expect(w.shown).toEqual([
      {
        latex: '\\begin{aligned} x &= a + b \\\\ &= c \\end{aligned}',
        display: true,
        source: 'fonts',
        confidence: 'high',
      },
    ]);
  });

  it('makes do with the selected text when there are no glyphs', async () => {
    const w = startWorker({ selection: null });
    await w.invoke('copyLatex', 'α ≤ sin x');
    expect(w.shown).toEqual([
      { latex: '\\alpha \\leq \\sin x', display: false, source: 'fonts', confidence: 'low' },
    ]);
  });
});

// ---------- asked by another extension ----------

/** A clock the worker reads through `Date.now`. */
function fakeClock() {
  let now = 1_700_000_000_000;
  vi.spyOn(Date, 'now').mockImplementation(() => now);
  return { advance: (ms: number) => (now += ms) };
}

/** Wait for work the worker does after its command is done. */
async function until(condition: () => boolean, ms = 1_000) {
  const end = performance.now() + ms;
  while (!condition()) {
    if (performance.now() > end) throw new Error('timed out waiting');
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
}

const EQ = { page: 0, bbox: [90, 180, 60, 30], number: '(1)' };

describe('Math to LaTeX: as the equation converter', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('answers a local conversion from the page alone, echoing its ticket', async () => {
    const w = startWorker({
      region: BOLTZMANN,
      identifiers: IDS,
      fetch: network({ arxiv: 'tex', jats: true }),
    });
    await w.invoke('copyEquation', { ...EQ, mode: 'local', ticket: 't7' });
    expect(w.shown).toEqual([
      {
        latex: 'e^{-\\beta E}',
        display: true,
        source: 'fonts',
        confidence: 'high',
        ticket: 't7',
      },
    ]);
    expect(w.fetched).toEqual([]);
    expect(w.calls).not.toContain('net.fetch');
    expect(w.calls).not.toContain('document.identifiers');
    expect(w.logs.some((l) => /^equation \(local\): .*fonts high in \d+ms$/.test(l))).toBe(true);
  });

  it('answers an exact conversion only from a source online, else nothing', async () => {
    const found = startWorker({
      region: BOLTZMANN,
      tags: [tagOver('$e^{-\\beta E}$')],
      identifiers: IDS,
      fetch: network({ arxiv: 'tex' }),
    });
    await found.invoke('copyEquation', { ...EQ, mode: 'exact', ticket: 't1' });
    expect(found.shown).toEqual([
      {
        latex: 'Z = \\sum_{s} e^{-\\beta E_s}',
        display: true,
        source: 'arxiv',
        confidence: 'high',
        ticket: 't1',
      },
    ]);
    // The tags were the local answer's business.
    expect(found.calls).not.toContain('document.mathTags');

    const none = startWorker({ region: BOLTZMANN, identifiers: IDS, fetch: network({}) });
    await none.invoke('copyEquation', { ...EQ, mode: 'exact', ticket: 't2' });
    expect(none.shown).toEqual([]);
  });

  it('remembers a failed download for ten minutes, then tries again', async () => {
    const clock = fakeClock();
    const w = startWorker({
      selection,
      identifiers: { ...IDS, doi: null },
      fetch: network({ arxiv: 'fail' }),
    });
    await w.invoke('copyLatex', 'e\u2212\u03b2E');
    expect(w.fetched).toEqual(['https://arxiv.org/e-print/2101.00001v2']);
    clock.advance(9 * 60_000);
    await w.invoke('copyLatex', 'e\u2212\u03b2E');
    // Answered from the fonts at once, without asking arXiv again.
    expect(w.fetched).toHaveLength(1);
    expect(w.shown.map((x) => x.source)).toEqual(['fonts', 'fonts']);
    clock.advance(60_001);
    await w.invoke('copyLatex', 'e\u2212\u03b2E');
    expect(w.fetched).toHaveLength(2);
  });

  it('does not count a download cut off by the deadline as failed, and uses it when it lands', async () => {
    const clock = fakeClock();
    let land: () => void = () => {};
    const landed = new Promise<void>((resolve) => (land = resolve));
    const real = network({ arxiv: 'tex' });
    let first = true;
    const w = startWorker({
      region: BOLTZMANN,
      identifiers: { ...IDS, doi: null },
      fetch: async (req) => {
        await landed;
        return real(req);
      },
      // The budget is spent by the time the source is waited on.
      onCall: (api) => {
        if (api === 'document.identifiers' && first) {
          first = false;
          clock.advance(5_000);
        }
      },
    });
    await w.invoke('copyEquation', EQ);
    expect(w.shown.map((x) => x.source)).toEqual(['fonts']);
    expect(w.fetched).toHaveLength(1);
    land();
    await until(() => w.storage.size > 0);
    await w.invoke('copyEquation', EQ);
    expect(w.shown.map((x) => x.source)).toEqual(['fonts', 'arxiv']);
    expect(w.fetched).toHaveLength(1);
  });

  it('starts the download on a prefetch without waiting for it', async () => {
    let land: () => void = () => {};
    const landed = new Promise<void>((resolve) => (land = resolve));
    const real = network({ arxiv: 'tex', jats: true });
    const w = startWorker({
      region: BOLTZMANN,
      identifiers: IDS,
      fetch: async (req) => {
        await landed;
        return real(req);
      },
    });
    // Done while the download is still on its way.
    await w.invoke('copyEquation', { mode: 'prefetch' });
    await until(() => w.fetched.length > 0);
    expect(w.fetched).toEqual(['https://arxiv.org/e-print/2101.00001v2']);
    expect(w.shown).toEqual([]);
    expect(w.calls).not.toContain('region.glyphs');
    land();
    await w.invoke('copyEquation', { ...EQ, mode: 'exact', ticket: 't3' });
    expect(w.shown.map((x) => x.source)).toEqual(['arxiv']);
    expect(w.fetched).toHaveLength(1);
  });

  it('prefetches nothing when the internet is switched off in its settings', async () => {
    const w = startWorker({
      identifiers: IDS,
      settings: { prefer: 'fonts only (offline)' },
      fetch: network({ arxiv: 'tex', jats: true }),
    });
    await w.invoke('copyEquation', { mode: 'prefetch' });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(w.fetched).toEqual([]);
    expect(w.calls).not.toContain('document.identifiers');
  });
});

/** The built-in Notation extension: what it reads as a definition, and the
 * panel it draws from them.
 *
 * The extraction is reached the way the references parser tests reach theirs:
 * by returning the worker's own functions from the same function body the
 * host's blob would run. The panel is driven through a fake host, as
 * `referencesWorker.test.ts` does.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.notation/worker.js'),
  'utf8'
);

interface SymbolDef {
  key: string;
  tex: string;
  phrase: string;
  page: number;
  snippet: string;
}
interface AcronymDef {
  key: string;
  long: string;
  page: number;
  uses: number;
  snippet: string;
}
interface Extraction {
  symbols: SymbolDef[];
  acronyms: AcronymDef[];
}
interface Notation {
  createExtractor(): { addPage(page: number, text: string): void; result(): Extraction };
  symbolTex(symbol: string): string;
  asciiText(text: string): string;
  phraseText(phrase: string, known: Set<string>): string;
  acronymCore(token: string): string | null;
}

const notation = new Function(
  'self',
  `${SOURCE}\nreturn { createExtractor, symbolTex, asciiText, phraseText, acronymCore };`
)({}) as Notation;

function extract(...pages: string[]): Extraction {
  const extractor = notation.createExtractor();
  pages.forEach((text, page) => extractor.addPage(page, text));
  return extractor.result();
}

const acronym = (result: Extraction, key: string) => result.acronyms.find((a) => a.key === key);
const symbol = (result: Extraction, key: string) => result.symbols.find((s) => s.key === key);

describe('notation: acronyms', () => {
  it('reads "long form (ACRO)"', () => {
    const result = extract('The system relaxes to a nonequilibrium steady state (NESS) quickly.');
    expect(acronym(result, 'NESS')?.long).toBe('nonequilibrium steady state');
  });

  it('reads "ACRO (long form)"', () => {
    const result = extract('We sample with MCMC (Markov chain Monte Carlo) throughout.');
    expect(acronym(result, 'MCMC')?.long).toBe('Markov chain Monte Carlo');
  });

  it('skips small words, splits hyphenated words, and allows one slip', () => {
    const result = extract(
      'Funded by the Department of Energy (DOE). ' +
        'A mean-field theory (MFT) applies. ' +
        'Rising carbon dioxide (CO2) levels. ' +
        'We train deep neural networks (DNNs).'
    );
    expect(acronym(result, 'DOE')?.long).toBe('Department of Energy');
    expect(acronym(result, 'MFT')?.long).toBe('mean-field theory');
    expect(acronym(result, 'CO2')?.long).toBe('carbon dioxide');
    expect(acronym(result, 'DNN')?.long).toBe('deep neural networks');
  });

  it('rejects parentheses that are not acronyms', () => {
    const result = extract(
      'The data are shown in the figure (Fig. 2), as before (see [12]). ' +
        'Published last year (2024). Two cases (i) and (ii), parts (a) and (b), ' +
        'and the second part (II) of the series. A random walk (RW) of nothing (QXZ).'
    );
    expect(result.acronyms.map((a) => a.key)).toEqual(['RW']);
  });

  it('rejects a long form whose initials do not match', () => {
    expect(extract('This is a quite unrelated sentence (NESS).').acronyms).toEqual([]);
  });

  it('keeps the first definition and counts later uses', () => {
    const result = extract(
      'A nonequilibrium steady state (NESS) appears.',
      'The NESS is stable. Later, a NESS (new extra special state) is redefined. NESSs differ.'
    );
    const ness = acronym(result, 'NESS')!;
    expect(ness.long).toBe('nonequilibrium steady state');
    expect(ness.page).toBe(0);
    // Four occurrences in all, the plural among them, one the definition.
    expect(ness.uses).toBe(3);
  });

  it('knows what an acronym looks like', () => {
    expect(notation.acronymCore('NESS')).toBe('NESS');
    expect(notation.acronymCore('DNNs')).toBe('DNN');
    expect(notation.acronymCore('mRNA')).toBe('mRNA');
    expect(notation.acronymCore('Fig')).toBeNull();
    expect(notation.acronymCore('IV')).toBeNull();
    expect(notation.acronymCore('2024')).toBeNull();
  });
});

describe('notation: symbols', () => {
  it('reads each defining sentence shape', () => {
    const result = extract(
      'Let λ denote the control parameter, which we vary. ' +
        'Let N be the number of sites. ' +
        'Here E = mc^2 where m is the mass, k the spring constant, and g the gravity. ' +
        'σ denotes the noise strength. ' +
        'We write x_i for the position of the ith particle. ' +
        'We define β as the inverse temperature. ' +
        'F := -log Z. ' +
        'H is called the Hamiltonian. ' +
        'A chain with γ the damping rate. ' +
        'P_N stands for the probability of N particles. ' +
        'We denote the entropy by S. ' +
        'The partition function, denoted Z(θ), is central. ' +
        'Denote by Ω the sample space.'
    );
    const read = Object.fromEntries(result.symbols.map((s) => [s.key, s.phrase]));
    expect(read).toEqual({
      '\\lambda': 'the control parameter',
      N: 'the number of sites',
      m: 'the mass',
      k: 'the spring constant',
      g: 'the gravity',
      '\\sigma': 'the noise strength',
      'x_{i}': 'the position of the ith particle',
      '\\beta': 'the inverse temperature',
      F: '-log Z',
      H: 'the Hamiltonian',
      '\\gamma': 'the damping rate',
      'P_{N}': 'the probability of N particles',
      S: 'the entropy',
      'Z(\\theta)': 'The partition function',
      '\\Omega': 'the sample space',
    });
  });

  it('does not take prose for notation', () => {
    const result = extract(
      'Let us denote things carefully. Figure A denotes the setup. ' +
        'This denotes nothing. Table B represents data. Where it is the case.'
    );
    expect(result.symbols).toEqual([]);
  });

  it('normalises math italic letters and Greek variants', () => {
    // U+1D706 MATHEMATICAL ITALIC SMALL LAMDA, U+03D1 GREEK THETA SYMBOL.
    const result = extract(
      'Let \u{1d706} denote the coupling. Let ϑ be the angle. Let \u{1d465} be the input.'
    );
    expect(result.symbols.map((s) => s.tex)).toEqual(['\\lambda', '\\theta', 'x']);
  });

  it('reads sub- and superscripts as extracted', () => {
    expect(notation.symbolTex('x_i')).toBe('x_{i}');
    expect(notation.symbolTex('x1')).toBe('x_{1}');
    expect(notation.symbolTex('x i')).toBe('x_{i}');
    expect(notation.symbolTex('β_t')).toBe('\\beta_{t}');
    expect(notation.symbolTex('Z(θ)')).toBe('Z(\\theta)');
    expect(notation.symbolTex('x̂')).toBe('\\hat{x}');
    expect(notation.symbolTex("f'")).toBe("f'");
    expect(notation.symbolTex('ω^*')).toBe('\\omega^{*}');
    const result = extract('Let E₀ denote the ground-state energy. Let x² be the square.');
    expect(result.symbols.map((s) => s.tex)).toEqual(['E_{0}', 'x^{2}']);
  });

  it('keeps the first definition, on the page it is on', () => {
    const result = extract(
      'Nothing here.',
      'Let λ denote the control parameter.',
      'Let λ be something else entirely.'
    );
    const lambda = symbol(result, '\\lambda')!;
    expect(lambda.phrase).toBe('the control parameter');
    expect(lambda.page).toBe(1);
    expect(result.symbols).toHaveLength(1);
  });

  it('cuts a long phrase at a word', () => {
    const long = 'the ' + 'very '.repeat(40) + 'long thing';
    const phrase = extract(`Let q denote ${long}.`).symbols[0]!.phrase;
    expect(phrase.length).toBeLessThanOrEqual(123);
    expect(phrase.endsWith('...')).toBe(true);
  });
});

/** Sentences as a long statistical learning text prints them, each of which
 * once came out merged, cut mid-thought or with its math as plain text. */
describe('notation: definitions in a book-length text', () => {
  /** Each symbol's phrase as the panel shows it. */
  const shown = (...pages: string[]) => {
    const result = extract(...pages);
    const known = new Set(result.symbols.map((s) => s.tex));
    return Object.fromEntries(
      result.symbols.map((s) => [s.key, notation.phraseText(s.phrase, known)])
    );
  };

  it('splits "X is ... and Y is ..." into two definitions', () => {
    expect(
      shown(
        'Here, where D is the diameter of the point set and ρ is the distance between the convex hulls.'
      )
    ).toEqual({
      D: 'the diameter of the point set',
      '\\rho': 'the distance between the convex hulls',
    });
  });

  it('splits "Let F be ... and C a ..." and typesets the symbol in the phrase', () => {
    expect(shown('Let F be an infinite set and C a collection of subsets of F.')).toEqual({
      F: 'an infinite set',
      C: 'a collection of subsets of $F$',
    });
  });

  it('splits "a is ... and b the ..."', () => {
    expect(
      shown(
        'Here, where a is the number of elements in the set of subsamples of the first type and ' +
          'b the number of elements in the set of subsamples of the second type.'
      )
    ).toEqual({
      a: 'the number of elements in the set of subsamples of the first type',
      b: 'the number of elements in the set of subsamples of the second type',
    });
  });

  it('splits lists, and keeps an "and" that is part of the phrase', () => {
    expect(
      shown('Let Y be the set of all points, Z the set of lines, and W the union of Y and Z.')
    ).toEqual({ Y: 'the set of all points', Z: 'the set of lines', W: 'the union of $Y$ and $Z$' });
    expect(shown('Let G be a group and a ring with unity.')).toEqual({
      G: 'a group and a ring with unity',
    });
  });

  it('typesets a formula in a phrase as one span', () => {
    expect(
      shown('The family C is called a Vapnik-Chervonenkis or VC class if and only if S(C)< +∞.')
    ).toEqual({ C: 'a Vapnik-Chervonenkis or VC class if and only if $S(C) < +\\infty$' });
  });

  it('marks math the text dropped rather than ending on "on"', () => {
    expect(
      shown(
        'Let X be a set of elementary events on . ' +
          'Denote by X(l) the space of samples in X of size l.'
      )
    ).toEqual({
      X: 'a set of elementary events on ...',
      // Not the same thing as X, so both are kept.
      'X(l)': 'the space of samples in $X$ of size $l$',
    });
  });

  it('runs a phrase on past "which" rather than stop on "for"', () => {
    expect(shown('Let n be a constant equaling the value of r for which .')).toEqual({
      n: 'a constant equaling the value of $r$ for which ...',
    });
    expect(shown('Let m be the value of r for which the bound is tight.')).toEqual({
      m: 'the value of $r$ for which the bound is tight',
    });
  });

  it('runs a long phrase on to a natural end, or cuts it at a complete word', () => {
    // Cut at 120 characters this would end "taken to be"; the clause ends
    // within 200, so it is shown whole.
    expect(
      extract(
        'Let A be a matrix of the form whose entries in the upper half are listed and whose ' +
          'entries in the lower half are taken to be zeros of the polynomial which we described ' +
          'at length in the previous chapter on.'
      ).symbols[0]!.phrase
    ).toBe(
      'a matrix of the form whose entries in the upper half are listed and whose entries in ' +
        'the lower half are taken to be zeros of the polynomial'
    );

    // No end within 200: cut back past "of the" to a complete word.
    const long = extract(`Let q be ${'one of the '.repeat(30)}things.`).symbols[0]!.phrase;
    expect(long).toMatch(/ one\.\.\.$/);
    expect(long.length).toBeLessThanOrEqual(123);

    const bracket = extract(
      'Let h be the map (taking values in a set of all things of a kind that are listed in ' +
        'the table and so on, with many more words here than fit in a row of the panel at all'
    ).symbols[0]!.phrase;
    expect(bracket).toBe('the map...');
  });

  it('drops X(l) when it is defined as the same thing as X', () => {
    expect(
      shown('Let X be the sample space. Denote by X(l) the sample space.', 'Nothing.')
    ).toEqual({ X: 'the sample space' });
  });
});

describe('notation: text for the panel', () => {
  it('writes Greek and math as TeX, and punctuation as ASCII, keeping accented letters', () => {
    expect(notation.asciiText('the rate λ ∈ R, “quoted” café')).toBe(
      'the rate $\\lambda$ $\\in$ R, "quoted" café'
    );
    expect(notation.asciiText('costs $5')).toBe('costs \\$5');
  });

  it('typesets variables in a phrase, and leaves words as words', () => {
    const none = new Set<string>();
    expect(notation.phraseText('a set of x with x - y = 0 for 0 < t', none)).toBe(
      'a set of $x$ with $x - y = 0$ for $0 < t$'
    );
    expect(notation.phraseText('A map from I to a', none)).toBe('A map from $I$ to $a$');
    expect(notation.phraseText('as in e.g. J. Smith, an n-dimensional set', none)).toBe(
      'as in e.g. J. Smith, an n-dimensional set'
    );
    expect(notation.phraseText('the rate λ_t ≤ 1 costs $5', none)).toBe(
      'the rate $\\lambda_{t} \\leq 1$ costs \\$5'
    );
    // Decorated only by a digit, a letter is math when the paper defines it.
    expect(notation.phraseText('the gap x1', none)).toBe('the gap x1');
    expect(notation.phraseText('the gap x1', new Set(['x_{1}']))).toBe('the gap $x_{1}$');
  });
});

// ---------------------------------------------------------------------------
// The worker behind a fake host.

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}
interface Run {
  text: string;
  box: Box;
}
interface Doc {
  name: string;
  /** Each page as its printed lines, top to bottom. */
  pages: string[][];
}

/** A page's runs: one per line, 14 points apart from the top. */
function runsOf(lines: string[]): Run[] {
  return lines.map((text, i) => ({
    text,
    box: { x: 50, y: 700 - i * 14, w: Math.min(text.length * 5, 450), h: 10 },
  }));
}

function startWorker(docs: Record<string, Doc>, settings: Record<string, string> = {}) {
  const storage = new Map<string, string>();
  const drawn: unknown[] = [];
  const visited: string[] = [];
  const texts: number[] = [];
  const contributed: { document: unknown; terms: Record<string, unknown>[] }[] = [];
  let open: string | null = null;

  const fake = {
    onmessage: undefined as ((e: { data: unknown }) => void) | undefined,
    postMessage(message: Record<string, unknown>) {
      if (message.type !== 'call') return;
      void answer(message);
    },
  };

  async function answer(message: Record<string, unknown>) {
    const args = (message.args ?? {}) as Record<string, unknown>;
    const doc = open ? docs[open] : undefined;
    const lines = doc?.pages[Number(args.page)];
    let value: unknown = null;
    let ok = true;
    switch (message.api) {
      case 'document.info':
        if (!doc) ok = false;
        else value = { name: doc.name, pageCount: doc.pages.length, currentPage: 0 };
        break;
      case 'document.pageText':
        texts.push(Number(args.page));
        value = lines ? lines.join(' ') : '';
        break;
      case 'document.pageRuns':
        value = lines ? runsOf(lines) : [];
        break;
      case 'navigate.toPage':
        visited.push(`page ${Number(args.page)}`);
        break;
      case 'navigate.toPlace': {
        const boxes = (args.boxes ?? []) as Box[];
        visited.push(
          `page ${Number(args.page)} at ${Number(args.y)} marking ${boxes
            .map((b) => `${b.x},${b.y} ${b.w}x${b.h}`)
            .join(' + ')}`
        );
        break;
      }
      case 'settings.get': {
        const key = String(args.key);
        value = settings[key] ?? (key === 'pages' ? 'first 200' : 'true');
        break;
      }
      case 'storage.get':
        value = storage.get(String(args.key)) ?? null;
        break;
      case 'storage.set':
        storage.set(String(args.key), String(args.value));
        break;
      case 'storage.remove':
        storage.delete(String(args.key));
        break;
      case 'ui.render':
        drawn.push(args.ui);
        break;
      case 'terms.contribute':
        contributed.push({
          document: args.document,
          terms: args.terms as Record<string, unknown>[],
        });
        value = { stored: (args.terms as unknown[]).length };
        break;
      default:
        ok = false;
    }
    fake.onmessage?.({
      data: { type: 'reply', requestId: message.requestId, ok, value, error: 'no document' },
    });
  }

  new Function('self', SOURCE)(fake);

  let nextInvoke = 10_000;
  return {
    drawn,
    visited,
    texts,
    storage,
    contributed,
    openDocument(key: string | null) {
      open = key;
    },
    async invoke(command: string, value?: unknown) {
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
        fake.onmessage?.({ data: { type: 'invoke', command, requestId, value, reason: 'panel' } });
      });
    },
  };
}

interface Row {
  id: string;
  title: string;
  subtitle?: string;
  badge?: string;
  action?: { command: string; value: string };
}
interface Ui {
  sections?: { title?: string; rows?: Row[] }[];
  empty?: string;
}

const last = (worker: { drawn: unknown[] }) => worker.drawn.at(-1) as Ui;
const rowsIn = (ui: Ui) => (ui.sections ?? []).flatMap((s) => s.rows ?? []);

const THEORY: Doc = {
  name: 'theory.pdf',
  pages: [
    ['We study the nonequilibrium steady', 'state (NESS) of a driven lattice gas.'],
    [
      'Throughout, let λ denote the control parameter.',
      'The NESS depends on λ, where N is the number of sites.',
    ],
    ['The NESS is unique.'],
  ],
};

const PLAIN: Doc = { name: 'plain.pdf', pages: [['Nothing is defined in this paper.']] };

describe('Notation worker: the panel', () => {
  it('draws symbols and acronyms in their own sections', async () => {
    const worker = startWorker({ theory: THEORY });
    worker.openDocument('theory');
    await worker.invoke('notation');
    const ui = last(worker);
    expect(ui.sections!.map((s) => s.title)).toEqual(['Symbols (2)', 'Acronyms (1)']);
    const [lambda, n, ness] = rowsIn(ui);
    expect(lambda!.title).toBe('$\\lambda$');
    expect(lambda!.subtitle).toBe('the control parameter');
    expect(lambda!.badge).toBe('p. 2');
    expect(n!.title).toBe('$N$');
    expect(ness!.title).toBe('NESS');
    expect(ness!.subtitle).toBe('nonequilibrium steady state');
    expect(ness!.badge).toBe('p. 1, used 2x');
  });

  /** What is shown, that is: an action's value is handed back to the worker
   * and never displayed, and it carries the text as printed. */
  it('shows nothing but ASCII', async () => {
    const worker = startWorker({ theory: THEORY });
    worker.openDocument('theory');
    await worker.invoke('notation');
    const shown = JSON.stringify(last(worker), (key, value) =>
      key === 'action' ? undefined : (value as unknown)
    );
    // eslint-disable-next-line no-control-regex
    expect(shown).not.toMatch(/[^\u0000-\u007f]/);
  });

  it('goes to the line a definition is printed on, and marks it', async () => {
    const worker = startWorker({ theory: THEORY });
    worker.openDocument('theory');
    await worker.invoke('notation');
    const lambda = rowsIn(last(worker))[0]!;
    expect(lambda.action!.command).toBe('goto');
    await worker.invoke('goto', lambda.action!.value);
    expect(worker.visited).toEqual(['page 1 at 700 marking 50,700 235x10']);

    // A definition broken across two lines is marked on both.
    const ness = rowsIn(last(worker)).find((row) => row.title.startsWith('NESS'))!;
    await worker.invoke('goto', ness.action!.value);
    expect(worker.visited[1]).toBe('page 0 at 700 marking 50,700 170x10 + 50,686 185x10');
  });

  it('falls back to the page when the text cannot be found on it', async () => {
    const worker = startWorker({ theory: THEORY });
    worker.openDocument('theory');
    await worker.invoke('goto', '2|0|nothinglikethis');
    expect(worker.visited).toEqual(['page 2']);
  });

  it('can hide acronyms', async () => {
    const worker = startWorker({ theory: THEORY }, { acronyms: 'false' });
    worker.openDocument('theory');
    await worker.invoke('notation');
    expect(last(worker).sections!.map((s) => s.title)).toEqual(['Symbols (2)']);
  });

  it('says so when nothing is defined, and when no document is open', async () => {
    const worker = startWorker({ plain: PLAIN });
    worker.openDocument('plain');
    await worker.invoke('notation');
    expect(last(worker).empty).toBe('No notation found');
    worker.openDocument(null);
    await worker.invoke('notation');
    expect(last(worker).empty).toContain('Open a paper');
  });

  it('reads only as many pages as the setting allows', async () => {
    const long: Doc = {
      name: 'long.pdf',
      pages: Array.from({ length: 60 }, (_, i) =>
        i === 55 ? ['Let λ denote a late definition.'] : ['Filler text.']
      ),
    };
    const worker = startWorker({ long }, { pages: 'first 50' });
    worker.openDocument('long');
    await worker.invoke('notation');
    expect(Math.max(...worker.texts)).toBe(49);
    expect(last(worker).empty).toBe('No notation found in the first 50 pages.');
  });

  it('answers from its cache, and reads again on a rescan', async () => {
    const worker = startWorker({ theory: THEORY });
    worker.openDocument('theory');
    await worker.invoke('notation');
    const reads = worker.texts.length;
    expect(worker.storage.size).toBe(1);

    await worker.invoke('notation');
    expect(worker.texts.length).toBe(reads);

    await worker.invoke('rescan');
    expect(worker.texts.length).toBe(reads * 2);
    expect(rowsIn(last(worker))).toHaveLength(3);
  });

  it('lists the new document after a switch', async () => {
    const worker = startWorker({ theory: THEORY, plain: PLAIN });
    worker.openDocument('theory');
    await worker.invoke('notation');
    worker.openDocument('plain');
    await worker.invoke('notation');
    expect(last(worker).empty).toBe('No notation found');
    expect(rowsIn(last(worker))).toEqual([]);
  });
});

describe('Notation worker: terms for hover previews', () => {
  const PS: Doc = {
    name: 'ps.pdf',
    pages: [
      ['A fluctuation-dis-', 'sipation theorem (FDT) holds here.', 'We use photosystem (PS) data.'],
      ['The FDT and the PS are both used.'],
      ['Later, a phase space (PS) is meant.'],
    ],
  };

  it('contributes every meaning with its page and the box that defines it', async () => {
    const worker = startWorker({ ps: PS });
    worker.openDocument('ps');
    await worker.invoke('terms', { document: 'terms-1' });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(worker.contributed).toHaveLength(1);
    const { document, terms } = worker.contributed[0]!;
    expect(document).toBe('terms-1');
    const read = terms.map((t) => [t.text, t.body, t.page]);
    // A long form broken across lines is put back together.
    expect(read).toContainEqual(['FDT', 'fluctuation-dissipation theorem', 0]);
    // The same acronym defined twice: both meanings, each where it is
    // defined, so the core can take the nearest preceding one.
    expect(read).toContainEqual(['PS', 'photosystem', 0]);
    expect(read).toContainEqual(['PS', 'phase space', 2]);
    const fdt = terms.find((t) => t.text === 'FDT')!;
    expect(fdt.caseSensitive).toBe(true);
    expect(fdt.box).toEqual({ x: 50, y: 700, w: expect.any(Number), h: 24 });
  });

  it('reads the whole long form and refuses a function argument', () => {
    const result = extract(
      'our private populous estimator (PPE) works. ' +
        'Solving the resulting inequality yields P(RE,j) small. ' +
        'A multidi\u0002 mensional organization unit (MOU) is used.'
    );
    expect(acronym(result, 'PPE')?.long).toBe('private populous estimator');
    expect(acronym(result, 'RE')).toBeUndefined();
    expect(acronym(result, 'MOU')?.long).toBe('multidimensional organization unit');
  });

  it('closes up a word broken across lines but keeps a real hyphen', () => {
    const result = extract(
      'A fluctuation-dis- sipation theorem (FDT) and a mean- field theory (MFT) apply. ' +
        'The field is strong.'
    );
    expect(acronym(result, 'FDT')?.long).toBe('fluctuation-dissipation theorem');
    expect(acronym(result, 'MFT')?.long).toBe('mean-field theory');
  });

  it('contributes nothing when acronyms are switched off', async () => {
    const worker = startWorker({ ps: PS }, { acronyms: 'false' });
    worker.openDocument('ps');
    await worker.invoke('terms', { document: 'terms-2' });
    expect(worker.contributed).toEqual([{ document: 'terms-2', terms: [] }]);
  });

  it('refuses an invoke without a document token', async () => {
    const worker = startWorker({ ps: PS });
    worker.openDocument('ps');
    await expect(worker.invoke('terms', 'nope')).rejects.toThrow(/document/);
  });
});

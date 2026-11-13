/** The reference-list parser inside the built-in References extension.
 *
 * It used to be the engine's (`src-tauri/src/library/references.rs`); it moved
 * into the extension with the `referenceProvider` contribution point, and its
 * rules moved with their tests. These are those tests, case for case, plus
 * the provider command they serve: SpeedyF invokes it with a library
 * document's stored text and an opaque token, and it answers through
 * `references.contribute`.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.references/worker.js'),
  'utf8'
);

interface Entry {
  label: string | null;
  text: string;
}
interface Parser {
  referenceSection(text: string): string | null;
  splitEntries(section: string): Entry[];
  entryYear(text: string): number | null;
  parseReferences(text: string): (Entry & { year: number | null })[];
  MAX_ENTRIES: number;
  MAX_ENTRY_CHARS: number;
}

/** The worker's own functions, reached by returning them from the same
 * function body the host's blob would run. Nothing in the shipped file
 * exports them. */
function loadParser(self: Record<string, unknown> = {}): Parser {
  return new Function(
    'self',
    `${SOURCE}\nreturn { referenceSection, splitEntries, entryYear, parseReferences, MAX_ENTRIES, MAX_ENTRY_CHARS };`
  )(self) as Parser;
}

const parser = loadParser();
const { referenceSection, entryYear, parseReferences } = parser;
const splitEntries = (section: string) =>
  parser.splitEntries(section).map(({ label, text }) => ({ label, text }));

const ZHONG =
  '[48] A. Zhong and M. R. DeWeese, Beyond linear response:\nEquivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).';

describe('reference parser: the section', () => {
  it('follows the last heading', () => {
    const text =
      'Introduction\nWe follow the references below.\nReferences\n[1] A. B, Old, Phys. Rev. E 1, 1 (2000).\n';
    expect(referenceSection(text)).toBe('[1] A. B, Old, Phys. Rev. E 1, 1 (2000).\n');
    // Two heading lines: the last one is the list.
    const two = 'References\nsee the appendix\nBody text.\n7 REFERENCES\n[1] X. Y, Z (2001).';
    expect(referenceSection(two)).toBe('[1] X. Y, Z (2001).');
  });

  it('counts a heading run into the text, and not prose', () => {
    const text =
      'We thank the referees. References [1] A. Author, A title (2020). [2] B. Author, B (2021).';
    expect(referenceSection(text)?.startsWith('[1] A. Author')).toBe(true);
    const prose = 'as the references [1] show, and see References in the appendix';
    expect(referenceSection(prose)).toBeNull();
    // The sidecar form: a page is one line, a heading is never alone.
    const page = 'funding. 6 References: 1. Smith, J. A title (2001). 2. Jones, K. B (2002).';
    expect(referenceSection(page)?.startsWith('1. Smith')).toBe(true);
  });

  it('takes the first of a run of running headers', () => {
    const list = Array.from(
      { length: 6 },
      (_, n) => `[${n + 1}] Author ${n + 1}, Title (20${String(n + 1).padStart(2, '0')}).`
    );
    const text = `Body. References ${list.slice(0, 3).join(' ')}\nSMITH JONES References ${list
      .slice(3)
      .join(' ')}`;
    expect(referenceSection(text)?.startsWith('[1] Author 1')).toBe(true);
    // Far apart, the later heading is a new list.
    const far = `Body. References ${list[0]} ${'x '.repeat(7000)}\nAppendix. References Zed, A. Later (2010).`;
    expect(referenceSection(far)?.startsWith('Zed, A.')).toBe(true);
  });

  it('with no heading takes the longest numbered run', () => {
    // The shape of 2512.24540: no heading at all, and an appendix whose
    // equation numbers print "[1] (E10)" — a `[1]` that starts a shorter run.
    let text = 'As shown [1] and in [3, 4]. Equation [1] (E10) holds. ';
    for (let n = 1; n <= 6; n++) {
      text += `[${n}] Author ${n}, Title ${n} (20${String(n).padStart(2, '0')}). `;
    }
    const section = referenceSection(text);
    expect(section?.startsWith('[1] Author 1')).toBe(true);
    expect(splitEntries(section!)).toHaveLength(6);
    expect(referenceSection('[1] a [2] b [3] c [4] d')).toBeNull();
    expect(referenceSection('no references at all')).toBeNull();
  });
});

describe('reference parser: entries', () => {
  it('splits bracketed entries in order and joins their lines', () => {
    const section =
      '[1] A. One, Non-equi-\nlibrium work, Physical\n25 Review E 94, 052106 (2016). [2] B. Two, see [1], Phys. Rev. Lett. 1, 2 (1999). [3] C. Three, T\u0002hree (2001).';
    const entries = splitEntries(section);
    expect(entries.map((e) => e.label)).toEqual(['1', '2', '3']);
    // Hyphen rejoined, page number dropped.
    expect(entries[0]!.text).toBe(
      'A. One, Non-equilibrium work, Physical Review E 94, 052106 (2016).'
    );
    expect(entries[1]!.text).toBe('B. Two, see [1], Phys. Rev. Lett. 1, 2 (1999).');
    expect(entries[2]!.text).toBe('C. Three, Three (2001).');
  });

  it('splits numbered, parenthesized and superscript entries', () => {
    const dotted =
      '1. Smith, J. Alpha in 3. dimensions (2001).\n2. Jones, K. Beta (2002).\n3. Lee, M. Gamma (2003).';
    const entries = splitEntries(dotted);
    expect(entries).toHaveLength(3);
    expect(entries[0]!.text).toBe('Smith, J. Alpha in 3. dimensions (2001).');
    const parens = '(1) A. One, X (2001). (2) B. Two, Y (2002). (3) C. Three, Z (2003).';
    expect(splitEntries(parens)).toHaveLength(3);
    const bare =
      '1 A. One, X, Phys. Rev. 4 (2001).\n2 B. Two, Y (2002).\n3 C. Three, Z (2003).\n4 D. Four (2004).';
    const four = splitEntries(bare);
    expect(four).toHaveLength(4);
    expect(four[3]!.label).toBe('4');
  });

  it('splits author-year and alpha-labelled lists', () => {
    const section =
      'Agarwal, N., Kale, S., and Zimmert, J. (2022). Efficient methods. In COLT.\nAlon, N., Ben-Eliezer, O. (2021). Adversarial laws. J. ACM.\nBartlett, P. L. (1998). The sample complexity. IEEE.';
    const entries = splitEntries(section);
    expect(entries).toHaveLength(3);
    expect(entries[1]!.text.startsWith('Alon, N.')).toBe(true);
    expect(entries[0]!.label).toBeNull();

    const alpha =
      '[AB99] M. Anthony and P. Bartlett. Neural network learning, 1999. [ACSZ23a] I. Aden-Ali et al. The one-inclusion graph, 2023. [Vap98] V. Vapnik. Statistical learning theory, 1998.';
    expect(splitEntries(alpha).map((e) => e.label)).toEqual(['AB99', 'ACSZ23a', 'Vap98']);

    const fullNames =
      'Emmanuel Abbe and Enric Boix. SGD learning. In COLT, 2023. Raman Arora and Ofer Dekel. Online bandits, 2012. URL https://x.org/a. Peter Bartlett. Rademacher, 2002. 3, 7 Martin Anthony. Neural nets, 2009.';
    expect(splitEntries(fullNames).map((e) => e.text.slice(0, 6))).toEqual([
      'Emmanu',
      'Raman ',
      'Peter ',
      'Martin',
    ]);
    expect(splitEntries('just some prose without structure')).toEqual([]);
  });

  it('does not take an initial for the end of an entry', () => {
    const section =
      'Brown, C., Title two (2002). A.K. Alton, G.R. Araujo, Title one (2001). Clark, D., Title three (2003). Dunn, E., Title four (2004).';
    const entries = splitEntries(section);
    expect(entries.map((e) => e.text.slice(0, 5))).toEqual(['Brown', 'Clark', 'Dunn,']);
    expect(entries[0]!.text).toContain('A.K. Alton, G.R. Araujo');
  });

  it('caps an entry’s length and the number of entries', () => {
    const long = `[1] ${'word '.repeat(1000)}[2] B (2001). [3] C (2002).`;
    const entries = splitEntries(long);
    expect(entries[0]!.text.length).toBeLessThanOrEqual(parser.MAX_ENTRY_CHARS);
    expect(entries[0]!.text.length).toBeGreaterThan(parser.MAX_ENTRY_CHARS - 6);
    const many = Array.from({ length: 1100 }, (_, n) => `[${n + 1}] Author ${n + 1}.`).join(' ');
    expect(splitEntries(many)).toHaveLength(parser.MAX_ENTRIES);
  });
});

describe('reference parser: years', () => {
  it('come from parentheses, then the end, then anywhere', () => {
    expect(entryYear('Phys. Rev. Lett. 133, 057102 (2024).')).toBe(2024);
    expect(entryYear('Smith, J. (2019a). Title, 2021 edition.')).toBe(2019);
    expect(entryYear('A. B. Title. In COLT, pages 1-9, 2023.')).toBe(2023);
    expect(entryYear('Title, arXiv:1901.00002, vol 1998 x')).toBe(1998);
    expect(entryYear('pages 2690 and 108301')).toBeNull();
    expect(entryYear('arXiv:2409.18065')).toBeNull();
  });
});

describe('reference parser: whole documents', () => {
  it('reads labels, texts and years, the paper’s own entry included', () => {
    // Identifiers are the engine's to read, own ones left out; the parser
    // hands over every entry.
    const text = `Body. References [1] Self, arXiv:2512.24540 (2025). [2] B. Two, doi:10.1000/x (2020). ${ZHONG.replace('[48]', '[3]')}`;
    const refs = parseReferences(text);
    expect(refs.map((r) => [r.label, r.year])).toEqual([
      ['1', 2025],
      ['2', 2020],
      ['3', 2024],
    ]);
    expect(refs[2]!.text).toBe(
      'A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).'
    );
  });

  /** "Higher-order response theory in optimal stochastic thermodynamics"
   * (2512.24540): [48] cites a library paper by journal reference and title
   * only. These are the entries the engine's end-to-end test stores. */
  it('reads 2512.24540’s list, with no heading, into the entries the engine stores', () => {
    const text =
      'Body text cites [1, 2] and [3].\n' +
      '[1] A. Zhong and M. R. DeWeese, Beyond linear response:\n' +
      'Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024). ' +
      '[2] T. Schmiedl and U. Seifert, Optimal finite-time processes in stochastic thermodynamics, Phys. Rev. Lett. 98, 108301 (2007). ' +
      '[3] Self, Higher-order response theory in optimal stochastic thermodynamics (2025). ' +
      '[4] X. Y, Optimal transport, Springer (2024). ' +
      '[5] D. A. Sivak and G. E. Crooks, Thermodynamic geometry of minimum-dissipation driven barrier crossing, doi:10.1000/other (2016). ' +
      '[6] Someone, Beyond linear response: equivalence between thermodynamic geometry and optimal transport, doi:10.1000/other (2024).';
    expect(parseReferences(text)).toEqual([
      {
        label: '1',
        text: 'A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).',
        year: 2024,
      },
      {
        label: '2',
        text: 'T. Schmiedl and U. Seifert, Optimal finite-time processes in stochastic thermodynamics, Phys. Rev. Lett. 98, 108301 (2007).',
        year: 2007,
      },
      {
        label: '3',
        text: 'Self, Higher-order response theory in optimal stochastic thermodynamics (2025).',
        year: 2025,
      },
      { label: '4', text: 'X. Y, Optimal transport, Springer (2024).', year: 2024 },
      {
        label: '5',
        text: 'D. A. Sivak and G. E. Crooks, Thermodynamic geometry of minimum-dissipation driven barrier crossing, doi:10.1000/other (2016).',
        year: 2016,
      },
      {
        label: '6',
        text: 'Someone, Beyond linear response: equivalence between thermodynamic geometry and optimal transport, doi:10.1000/other (2024).',
        year: 2024,
      },
    ]);
  });

  it('reads a list glued to the acknowledgments across a page break', () => {
    const text =
      'Higher-order response theory. See [1] and [2].\n' +
      'Acknowledgments. We thank X. References [1] A. Zhong and M. R. DeWeese, Beyond linear response:\n' +
      'Equivalence between thermodynamic geometry and optimal transport,\n' +
      'Phys. Rev. Lett. 133, 057102 (2024). [2] B. Two, arXiv:2401.00003 (2024).';
    const refs = parseReferences(text);
    expect(refs).toHaveLength(2);
    expect(refs[0]!.label).toBe('1');
    expect(refs[0]!.year).toBe(2024);
    expect(refs[0]!.text).toContain(
      'Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett.'
    );
    expect(parseReferences('No list here.')).toEqual([]);
  });
});

describe('References as the reference provider', () => {
  /** A worker with a host that records `references.contribute` calls. */
  function provider() {
    const contributed: unknown[] = [];
    const fake = {
      onmessage: undefined as ((e: { data: unknown }) => void) | undefined,
      postMessage(message: Record<string, unknown>) {
        if (message.type === 'call') {
          if (message.api === 'references.contribute') contributed.push(message.args);
          fake.onmessage?.({
            data: { type: 'reply', requestId: message.requestId, ok: true, value: { stored: 1 } },
          });
        }
      },
    };
    new Function('self', SOURCE)(fake);
    const invoke = (value: unknown) =>
      new Promise<{ ok: boolean; error: string | undefined }>((resolve) => {
        const post = fake.postMessage.bind(fake);
        fake.postMessage = (message: Record<string, unknown>) => {
          if (message.type === 'done') {
            fake.postMessage = post;
            resolve({ ok: message.ok as boolean, error: message.error as string | undefined });
            return;
          }
          post(message);
        };
        fake.onmessage?.({ data: { type: 'invoke', requestId: 1, command: 'provide', value } });
      });
    return { contributed, invoke };
  }

  it('answers a document’s token with its entries before it is done', async () => {
    const { contributed, invoke } = provider();
    const done = await invoke({
      document: 'tok-1',
      text: 'Body. References [1] A. One, X (2001). [2] B. Two, Y.',
    });
    expect(done.ok).toBe(true);
    expect(contributed).toEqual([
      {
        document: 'tok-1',
        entries: [
          { label: '1', text: 'A. One, X (2001).', year: 2001 },
          { label: '2', text: 'B. Two, Y.' },
        ],
      },
    ]);
  });

  it('contributes an empty list for a document with none, and refuses a run by hand', async () => {
    const { contributed, invoke } = provider();
    expect((await invoke({ document: 'tok-2', text: 'No list.' })).ok).toBe(true);
    expect(contributed).toEqual([{ document: 'tok-2', entries: [] }]);
    const byHand = await invoke(undefined);
    expect(byHand.ok).toBe(false);
    expect(contributed).toHaveLength(1);
  });
});

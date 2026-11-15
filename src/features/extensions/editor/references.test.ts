/** A note's shared references section: one entry a work, however each
 * quoted paper printed it, read and rewritten as plain text. */
import { describe, expect, it } from 'vitest';
import {
  addReferences,
  bodyEnd,
  hasStrayDefinitions,
  libraryMatches,
  linkToLibrary,
  removeReferences,
  sameWork,
  stableId,
  tidyReferences,
  unusedReferences,
  type Work,
} from './references';

const SIMON: Work = {
  text: 'B. Simon, Almost periodic Schroedinger operators, Adv. Appl. Math. 3 (1982).',
  ids: [],
  title: 'Almost periodic Schroedinger operators',
  author: 'Simon',
  year: 1982,
};

describe('one entry a work', () => {
  it('shares one entry between two quotes citing the same work by different labels', () => {
    // Paper A prints it "[Sim82]", paper B "[37]": same work, one entry.
    const first = addReferences('> As in [Sim82].', [SIMON]);
    expect(first.ids).toEqual(['simon1982almost']);
    expect(first.text).toBe(
      [
        '> As in [Sim82].',
        '',
        '## References',
        '',
        '[^simon1982almost]: B. Simon, Almost periodic Schroedinger operators, Adv. Appl. Math. 3 (1982).',
        '',
      ].join('\n')
    );
    const printedElsewhere: Work = {
      text: '[37] Barry Simon. Almost periodic Schroedinger operators. Advances in Applied Mathematics, 1982.',
      title: 'Almost periodic Schroedinger operators',
      author: 'Simon',
      year: 1982,
    };
    const second = addReferences(first.text, [printedElsewhere]);
    expect(second.ids).toEqual(['simon1982almost']);
    // Nothing added: the entry is there already.
    expect(second.text).toBe(first.text);
  });

  it('matches by DOI or arXiv id first, whatever the text', () => {
    const note = addReferences('', [
      {
        text: 'L. Larsen, Blocks (2023).',
        ids: ['doi:10.1000/blocks'],
        author: 'Larsen',
        year: 2023,
      },
    ]);
    const again = addReferences(note.text, [
      { text: 'Larsen L. Some other wording. doi:10.1000/BLOCKS', author: 'Larsen', year: 2023 },
    ]);
    expect(again.ids).toEqual(note.ids);
  });

  it('never merges different works, even with the same printed label', () => {
    const one = addReferences('', [SIMON]);
    // Another Simon paper of the same year, printed "[Sim82]" in its paper.
    const other: Work = {
      text: '[Sim82] B. Simon, Schroedinger semigroups, Bull. AMS 7 (1982).',
      title: 'Schroedinger semigroups',
      author: 'Simon',
      year: 1982,
    };
    const two = addReferences(one.text, [other]);
    expect(two.ids).toEqual(['simon1982schroedinger']);
    expect(sameWork(other, `[Sim82] ${SIMON.text}`)).toBe(false);
    // Different DOIs are different works, whatever the titles say.
    expect(
      sameWork(
        { text: 'x doi:10.1000/a', title: SIMON.title ?? null, author: 'Simon', year: 1982 },
        `${SIMON.text} doi:10.1000/b`
      )
    ).toBe(false);
  });

  it('suffixes an id that names another work already', () => {
    const taken = new Set(['simon1982almost']);
    expect(stableId(SIMON, taken)).toBe('simon1982almosta');
    expect(stableId(SIMON, new Set([...taken, 'simon1982almosta']))).toBe('simon1982almostb');
    // Lower-case ASCII, accents off, a stop word skipped.
    expect(
      stableId(
        { text: '', author: 'Erdős', year: 1947, title: 'The probabilistic method' },
        new Set()
      )
    ).toBe('erdos1947probabilistic');
    expect(stableId({ text: 'x' }, new Set())).toBe('ref');
  });

  it('adds only the entries that are missing', () => {
    const one = addReferences('', [SIMON]);
    const larsen: Work = {
      text: 'L. Larsen, Blocks of random matrices (2023).',
      author: 'Larsen',
      year: 2023,
      title: 'Blocks of random matrices',
    };
    const two = addReferences(one.text, [SIMON, larsen]);
    expect(two.ids).toEqual(['simon1982almost', 'larsen2023blocks']);
    expect(two.text.match(/^\[\^/gm)).toHaveLength(2);
  });
});

describe('the section in the note', () => {
  it('keeps what is added at the end before the section', () => {
    const text = '# Notes\n\nSome text.\n\n## References\n\n[^a]: A.\n';
    expect(text.slice(0, bodyEnd(text))).toBe('# Notes\n\nSome text.');
    expect(bodyEnd('No section.')).toBe('No section.'.length);
  });

  it('moves the entries an earlier Notepad wrote under each quote, once', () => {
    const old = [
      '> As in [^Sim15]. [@rz26, p. 2]',
      '',
      '[^Sim15]: [Sim15] B. Simon, Spectral theory (2015).',
      '',
      '> And [^Lar23]. [@rz26, p. 3]',
      '',
      '[^Lar23]: [Lar23] L. Larsen, Blocks (2023).',
      '',
    ].join('\n');
    expect(hasStrayDefinitions(old)).toBe(true);
    const tidy = tidyReferences(old);
    expect(tidy).toBe(
      [
        '> As in [^Sim15]. [@rz26, p. 2]',
        '',
        '> And [^Lar23]. [@rz26, p. 3]',
        '',
        '## References',
        '',
        '[^Sim15]: [Sim15] B. Simon, Spectral theory (2015).',
        '[^Lar23]: [Lar23] L. Larsen, Blocks (2023).',
        '',
      ].join('\n')
    );
    // Idempotent: a tidy note stays as it is.
    expect(tidyReferences(tidy)).toBe(tidy);
    expect(hasStrayDefinitions(tidy)).toBe(false);
    // The next quote's write moves them too, and reuses an old entry.
    const next = addReferences(old, [
      {
        text: 'B. Simon, Spectral theory (2015).',
        title: 'Spectral theory and more',
        author: 'Simon',
        year: 2015,
      },
    ]);
    expect(next.text.startsWith(tidy.trimEnd())).toBe(true);
  });

  it('finds the entries nothing cites, and removes them only when asked', () => {
    const text = '> As in [^a]{label=A1}.\n\n## References\n\n[^a]: A.\n[^b]: B.\n';
    expect(unusedReferences(text)).toEqual(['b']);
    expect(removeReferences(text, ['b'])).toBe(
      '> As in [^a]{label=A1}.\n\n## References\n\n[^a]: A.\n'
    );
  });
});

describe('entries now in the library', () => {
  const ITEMS = [
    {
      key: 'larsen2023',
      title: 'Blocks of random matrices',
      detail: 'Larsen - 2023',
      doi: null,
      arxiv: null,
    },
    {
      key: 'simon15',
      title: 'Spectral theory',
      detail: 'Simon - 2015',
      doi: null,
      arxiv: '1501.01234',
    },
  ];
  const NOTE = [
    '> By [@han2016; @aahlz2024][^larsen2023blocks]{label=Lar23 at=1}, and [^sim]{label=Sim15}.',
    '',
    '## References',
    '',
    '[^larsen2023blocks]: L. Larsen, Blocks of random matrices, Ann. Probab. (2023).',
    '[^sim]: B. Simon, Spectral theory, arXiv:1501.01234 (2015).',
    '[^other]: K. Keller, Overlaps (1999).',
    '',
  ].join('\n');

  it('offers the entries whose work an item is, by identifier or title, author and year', () => {
    expect([...libraryMatches(NOTE, ITEMS)]).toEqual([
      ['larsen2023blocks', 'larsen2023'],
      ['sim', 'simon15'],
    ]);
  });

  it('links them: markers become citations in their printed place, entries go', () => {
    const linked = linkToLibrary(NOTE, libraryMatches(NOTE, ITEMS));
    expect(linked).toBe(
      [
        '> By [@han2016; @larsen2023; @aahlz2024], and [@simon15].',
        '',
        '## References',
        '',
        '[^other]: K. Keller, Overlaps (1999).',
        '',
      ].join('\n')
    );
  });
});

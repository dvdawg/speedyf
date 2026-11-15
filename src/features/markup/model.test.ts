import { describe, expect, it } from 'vitest';
import { quadsFromRects } from './geometry';
import type { CommentLabel } from './labels';
import {
  alsoText,
  anchorQuote,
  buildEntries,
  citationsAsText,
  citedKeys,
  groupBySection,
  kindCounts,
  longDate,
  mergeRepeats,
  parseMarkup,
  plainLetter,
  serializeMarkup,
  shortMathQuote,
  shortQuote,
  summarySkeleton,
  typoSuggestion,
  type Entry,
} from './model';
import type { Anchor, CommentKind, MarkupComment } from './types';

const H = 800;

function anchor(page: number, top: number, quote = 'the words'): Anchor {
  return { page, quads: quadsFromRects([[72, top, 100, 10]], H), rect: null, quote };
}

let n = 0;
function comment(over: Partial<MarkupComment> & { anchor: Anchor }): MarkupComment {
  n += 1;
  return {
    id: `c${n}`,
    kind: 'comment',
    body: '',
    replacement: null,
    quickmark: null,
    created: n,
    ...over,
  };
}

/** Sections by page: page 0 front matter, 1-2 "1 Intro", 3+ "2 Method". */
function label(a: Anchor): CommentLabel {
  const section =
    a.page === 0
      ? null
      : a.page < 3
        ? { title: '1 Intro', short: 'Sec. 1', page: 1, y: 700 }
        : { title: '2 Method', short: 'Sec. 2', page: 3, y: 700 };
  return {
    page: a.page + 1,
    section,
    statement: a.page === 3 ? 'Lemma 2.3' : null,
    equation: null,
  };
}

const keys: Record<string, string> = { jar97: 'Jar97', smi20: 'Smi20' };
const labelOf = (k: string) => keys[k] ?? null;

describe('storage', () => {
  it('round-trips a review', () => {
    const doc = {
      version: 1,
      summary: 'Good.',
      comments: [comment({ anchor: anchor(0, 100), body: 'Hi', kind: 'comment' })],
    };
    expect(parseMarkup(serializeMarkup(doc))).toEqual(doc);
  });

  it('reads what it can of a damaged file and nothing of garbage', () => {
    expect(parseMarkup(null).comments).toEqual([]);
    expect(parseMarkup('{not json').comments).toEqual([]);
    const parsed = parseMarkup(
      JSON.stringify({
        version: 7,
        summary: 3,
        comments: [
          { id: 'a', kind: 'severe', anchor: { page: 1, rect: { x: 1, y: 2, w: 3, h: 4 } } },
          { id: 'b', anchor: { page: -1, rect: { x: 1, y: 2, w: 3, h: 4 } } },
          { id: 'c', anchor: { page: 0, quads: [{ x1: 'no' }] } },
          'nonsense',
        ],
      })
    );
    expect(parsed.summary).toBe('');
    expect(parsed.comments).toHaveLength(1);
    expect(parsed.comments[0]).toMatchObject({
      id: 'a',
      kind: 'comment',
      body: '',
      quickmark: null,
    });
  });
});

describe('entries and groups', () => {
  const list = [
    comment({ anchor: anchor(3, 100), kind: 'comment' }),
    comment({ anchor: anchor(0, 300) }),
    comment({ anchor: anchor(1, 100), kind: 'comment' }),
    comment({ anchor: anchor(2, 100), kind: 'typo', replacement: 'the' }),
  ];
  const entries = buildEntries(list, label);

  it('numbers in reading order and groups by section in paper order', () => {
    expect(entries.map((e) => e.number)).toEqual([1, 2, 3, 4]);
    expect(entries.map((e) => e.label.page)).toEqual([1, 2, 3, 4]);
    expect(groupBySection(entries).map((g) => [g.title, g.entries.length])).toEqual([
      ['Front matter', 1],
      ['1 Intro', 2],
      ['2 Method', 1],
    ]);
  });

  it('counts each kind', () => {
    expect(kindCounts(list)).toEqual({
      comment: 3,
      question: 0,
      typo: 1,
    });
  });
});

describe('repeats', () => {
  it('folds the same QuickMark on the same words into the first', () => {
    const list = [
      comment({
        anchor: anchor(1, 100, 'Banach  space'),
        quickmark: 'define',
        body: 'Define this notation before use.',
      }),
      comment({
        anchor: anchor(4, 100, 'banach space'),
        quickmark: 'define',
        body: 'Define this notation before use.',
      }),
      comment({
        anchor: anchor(6, 100, 'Banach space'),
        quickmark: 'define',
        body: 'Define this notation before use. Also here.',
      }),
      comment({
        anchor: anchor(8, 100, 'Banach space'),
        quickmark: 'define',
        body: 'Define this notation before use.',
      }),
      comment({ anchor: anchor(9, 100, 'Banach space'), body: 'Define this notation before use.' }),
    ];
    const merged = mergeRepeats(buildEntries(list, label));
    expect(merged.map((m) => [m.entry.number, m.also.map((e) => e.number)])).toEqual([
      [1, [2, 4]],
      [3, []],
      [5, []],
    ]);
    expect(alsoText(merged[0]!)).toBe('also p. 5, p. 9');
    expect(alsoText(merged[1]!)).toBe('');
  });
});

describe('text', () => {
  it('cuts a long quote at a word', () => {
    expect(shortQuote('  short\n quote ')).toBe('short quote');
    const long = 'word '.repeat(60);
    const cut = shortQuote(long, 40);
    expect(cut.endsWith('...')).toBe(true);
    expect(cut.length).toBeLessThanOrEqual(43);
    expect(cut).not.toMatch(/wor\.\.\.$/);
  });

  it('reads citations as their labels and leaves unknown keys alone', () => {
    expect(citedKeys('See [@jar97] and [@smi20, p. 3; @jar97]. Not @x.')).toEqual([
      'jar97',
      'smi20',
    ]);
    expect(citationsAsText('See [@jar97].', labelOf)).toBe('See [Jar97].');
    expect(citationsAsText('[@smi20, Thm. 2; @jar97]', labelOf)).toBe('[Smi20, Thm. 2; Jar97]');
    expect(citationsAsText('[@nobody]', labelOf)).toBe('[@nobody]');
    expect(citationsAsText('[not a cite]', labelOf)).toBe('[not a cite]');
  });

  it('starts the Overall paragraph from the counts', () => {
    const skeleton = summarySkeleton([
      comment({ anchor: anchor(0, 1), kind: 'comment' }),
      comment({ anchor: anchor(0, 2), kind: 'comment' }),
      comment({ anchor: anchor(0, 3), kind: 'typo' }),
    ]);
    expect(skeleton).toContain('Summary: ');
    expect(skeleton).toContain('Main concerns:');
    expect(skeleton).toContain('3 notes: 2 comments and 1 typo.');
    expect(summarySkeleton([])).not.toContain('notes:');
  });

  it('says what a typo should read', () => {
    const a = anchor(0, 1, 'teh');
    expect(typoSuggestion(comment({ anchor: a, kind: 'typo', replacement: 'the' }))).toBe('the');
    expect(
      typoSuggestion(comment({ anchor: a, kind: 'typo', quickmark: 'typo', body: 'Typo.' }))
    ).toBe('');
    expect(typoSuggestion(comment({ anchor: a, kind: 'typo', body: 'should be "the"' }))).toBe(
      'should be "the"'
    );
  });

  it('writes dates out in full', () => {
    expect(longDate(new Date(2026, 10, 15))).toBe('November 15, 2026');
  });
});

describe('plainLetter', () => {
  const kinds: CommentKind[] = ['comment', 'comment'];
  const list = [
    comment({
      anchor: anchor(0, 100, 'We consider'),
      kind: kinds[1]!,
      body: 'Add motivation here.',
    }),
    comment({
      anchor: anchor(3, 100, 'by compactness'),
      kind: 'comment',
      quickmark: 'justify',
      body: 'Justify this step. Compare [@jar97].',
    }),
    comment({ anchor: anchor(3, 200, 'teh'), kind: 'typo', replacement: 'the', quickmark: 'typo' }),
    comment({
      anchor: anchor(5, 200, 'by compactness'),
      kind: 'comment',
      quickmark: 'justify',
      body: 'Justify this step. Compare [@jar97].',
    }),
  ];
  const entries: Entry[] = buildEntries(list, label);
  const letter = plainLetter({
    title: 'A Paper',
    date: 'November 15, 2026',
    summary: 'A fine paper; see [@smi20].',
    entries,
    label: labelOf,
    reference: (k) =>
      k === 'jar97' ? 'Jarvis. On things. 1997.' : k === 'smi20' ? 'Smith. Stuff. 2020.' : null,
  });

  it('is a letter grouped by section with the typos and references at the end', () => {
    expect(letter).toBe(
      [
        'Comments on A Paper',
        'November 15, 2026',
        '',
        'Overall',
        '',
        'A fine paper; see [Smi20].',
        '',
        'Front matter',
        '',
        '  1. p. 1 [Comment]',
        '     "We consider"',
        '     Add motivation here.',
        '',
        '2 Method',
        '',
        '  2. p. 4 - Lemma 2.3 [Comment] (also p. 6)',
        '     "by compactness"',
        '     Justify this step. Compare [Jar97].',
        '',
        'Typos and wording',
        '',
        '  3. p. 4: "teh" -> "the"',
        '',
        'References',
        '',
        '  [Smi20] Smith. Stuff. 2020.',
        '  [Jar97] Jarvis. On things. 1997.',
        '',
      ].join('\n')
    );
  });

  it('is ASCII', () => {
    expect(letter).toMatch(/^[\x20-\x7e\n]*$/);
  });
});

describe('quotes with their math as LaTeX', () => {
  it('reads quoteTex back, and leaves it out of reviews without it', () => {
    const quad = { x1: 0, y1: 10, x2: 5, y2: 10, x3: 0, y3: 0, x4: 5, y4: 0 };
    const json = (anchor: object) =>
      JSON.stringify({
        version: 1,
        summary: '',
        comments: [{ id: 'c_1', kind: 'comment', anchor, body: '', created: 1 }],
      });
    const withTex = parseMarkup(json({ page: 0, quads: [quad], quote: 'a', quoteTex: '$a$' }));
    expect(withTex.comments[0]!.anchor.quoteTex).toBe('$a$');
    const without = parseMarkup(json({ page: 0, quads: [quad], quote: 'a' }));
    expect('quoteTex' in without.comments[0]!.anchor).toBe(false);
  });

  it('never cuts a quote inside a formula', () => {
    const tex = 'we bound $\\sum_{i=1}^{n} x_i^2 \\le C$ and then the rest of it goes on';
    expect(shortMathQuote(tex, 45)).toBe('we bound $\\sum_{i=1}^{n} x_i^2 \\le C$ and...');
    expect(shortMathQuote(tex, 30)).toBe('we bound...');
    expect(shortMathQuote(tex, 12)).toBe('we bound...');
    expect(shortMathQuote('$$ a + b = c $$ then', 5)).toBe('$$ a + b = c $$...');
  });

  it('quotes the LaTeX in the letter when there is some', () => {
    expect(anchorQuote({ page: 0, quads: null, rect: null, quote: 'x2', quoteTex: '$x^2$' })).toBe(
      '$x^2$'
    );
    expect(anchorQuote({ page: 0, quads: null, rect: null, quote: 'x2' })).toBe('x2');
  });
});

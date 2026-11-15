import { describe, expect, it } from 'vitest';
import type { PaperRow } from '../../types/engine';
import {
  alphaLabels,
  atQuery,
  buildCiteIndex,
  insertCitation,
  pickCandidates,
  referenceLine,
} from './citations';

function row(over: Partial<PaperRow> & { id: string }): PaperRow {
  return {
    title: 'A title',
    authors: 'Smith',
    year: 2020,
    venue: null,
    citeKey: over.id,
    alphaLabel: 'Smi20',
    projectIds: [],
    missing: false,
    hasIdentifier: false,
    incompleteCount: 0,
    isPaper: true,
    candidateCount: 0,
    fileCount: 1,
    ...over,
  };
}

describe('labels', () => {
  it('tells repeats apart the way alpha.bst does', () => {
    const rows = [
      row({ id: 'b', year: 2020, title: 'Beta' }),
      row({ id: 'a', year: 2020, title: 'Alpha' }),
      row({ id: 'c', alphaLabel: 'Jar97', year: 1997 }),
      row({ id: 'd', alphaLabel: '', authors: 'Kim, Lee', year: 2003 }),
    ];
    const labels = alphaLabels(rows);
    expect(labels.get('a')).toBe('Smi20a');
    expect(labels.get('b')).toBe('Smi20b');
    expect(labels.get('c')).toBe('Jar97');
    expect(labels.get('d')).toBe('Kim03');
  });

  it('writes a reference line from what the item has', () => {
    expect(
      referenceLine(
        row({ id: 'x', authors: 'Smith, Jones', title: 'On things.', venue: 'Ann. Math.' })
      )
    ).toBe('Smith, Jones. On things. Ann. Math., 2020.');
    expect(referenceLine(row({ id: 'y', authors: '', title: '', year: null }))).toBe('y');
  });

  it('resolves old keys to the item that had them', () => {
    const index = buildCiteIndex([
      row({ id: 'x', citeKey: 'smith2020new', keyHistory: ['smith2020old'] }),
    ]);
    expect(index.byKey.get('smith2020old')?.key).toBe('smith2020new');
  });
});

describe('the @ picker', () => {
  it('opens on an @ after a space or bracket, not inside a word', () => {
    expect(atQuery('see @smi', 8)).toEqual({ at: 4, query: 'smi' });
    expect(atQuery('@', 1)).toEqual({ at: 0, query: '' });
    expect(atQuery('mail a@b', 8)).toBeNull();
  });

  it('ranks matches and writes a bracketed citation', () => {
    const index = buildCiteIndex([
      row({
        id: 'jarvis1997',
        title: 'Spectral gaps',
        alphaLabel: 'Jar97',
        year: 1997,
        authors: 'Jarvis',
      }),
      row({ id: 'smith2020', title: 'Banach spaces', authors: 'Smith' }),
    ]);
    expect(pickCandidates(index, 'banach').map((e) => e.key)).toEqual(['smith2020']);
    expect(pickCandidates(index, '').length).toBe(2);
    expect(insertCitation('see @ban more', 4, 8, 'smith2020')).toEqual({
      text: 'see [@smith2020] more',
      caret: 16,
    });
  });
});

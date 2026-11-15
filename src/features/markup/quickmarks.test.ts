import { describe, expect, it } from 'vitest';
import {
  applyQuickmark,
  matchQuickmarks,
  quickmarkById,
  QUICKMARKS,
  slashQuery,
} from './quickmarks';

describe('QuickMarks', () => {
  it('are the stock remarks, each with its kind', () => {
    expect(QUICKMARKS.map((q) => q.id)).toEqual([
      'gap',
      'justify',
      'hypothesis',
      'why',
      'cite',
      'related',
      'define',
      'unclear',
      'reorder',
      'motivate',
      'typo',
    ]);
    expect(quickmarkById('gap')).toMatchObject({ text: 'Gap in the argument', kind: 'proof' });
    expect(quickmarkById('cite')?.kind).toBe('literature');
    expect(quickmarkById('define')?.kind).toBe('exposition');
    expect(quickmarkById('nope')).toBeNull();
  });

  it('open on a slash at the very start, while the caret is in the word', () => {
    expect(slashQuery('/ju', 3)).toBe('ju');
    expect(slashQuery('/', 1)).toBe('');
    expect(slashQuery('/justify more', 10)).toBeNull();
    expect(slashQuery('a /ju', 5)).toBeNull();
    expect(slashQuery('', 0)).toBeNull();
  });

  it('match the id first, then anything mentioning the words', () => {
    expect(matchQuickmarks('').length).toBe(QUICKMARKS.length);
    expect(matchQuickmarks('re').map((q) => q.id)).toEqual(['related', 'reorder', 'unclear']);
    expect(matchQuickmarks('citation').map((q) => q.id)).toEqual(['cite']);
  });

  it('replace the slash word with their text, ready to type on', () => {
    const gap = quickmarkById('gap')!;
    expect(applyQuickmark('/ga', gap)).toEqual({ text: 'Gap in the argument. ', caret: 21 });
    const hypothesis = quickmarkById('hypothesis')!;
    expect(applyQuickmark('/hyp  Lemma 2 needs it', hypothesis).text).toBe(
      'Which hypothesis is used here? Lemma 2 needs it'
    );
  });
});

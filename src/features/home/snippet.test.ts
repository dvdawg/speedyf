import { describe, expect, it } from 'vitest';
import { splitSnippet } from './snippet';

const emphasized = (snippet: string, query: string) =>
  splitSnippet(snippet, query)
    .filter((part) => part.match)
    .map((part) => part.text);

const rejoined = (snippet: string, query: string) =>
  splitSnippet(snippet, query)
    .map((part) => part.text)
    .join('');

describe('splitSnippet', () => {
  it('marks the phrase and leaves the rest alone', () => {
    const parts = splitSnippet('we show variance collapse holds', 'variance collapse');
    expect(parts).toEqual([
      { text: 'we show ', match: false },
      { text: 'variance collapse', match: true },
      { text: ' holds', match: false },
    ]);
  });

  it('never loses or reorders a character', () => {
    // Whatever it marks, the snippet must still read exactly as it arrived.
    for (const query of ['variance', 'VARIANCE', 'nothing here', '', '   ']) {
      expect(rejoined('the variance of the variance', query)).toBe('the variance of the variance');
    }
  });

  it('ignores case, as the search itself does', () => {
    expect(emphasized('Variance Collapse under smoothing', 'variance collapse')).toEqual([
      'Variance Collapse',
    ]);
  });

  it('matches a phrase broken across a line, since the backend does', () => {
    expect(emphasized('we show variance\ncollapse holds', 'variance collapse')).toEqual([
      'variance\ncollapse',
    ]);
  });

  it('marks every occurrence, not just the first', () => {
    expect(emphasized('variance and more variance', 'variance')).toEqual(['variance', 'variance']);
  });

  it('treats regex characters as literal text', () => {
    // Typed into a search box these are characters, not syntax.
    expect(emphasized('the term f(x) appears', 'f(x)')).toEqual(['f(x)']);
    expect(emphasized('a .* b', '.*')).toEqual(['.*']);
    expect(emphasized('cost is O(n^2) here', 'O(n^2)')).toEqual(['O(n^2)']);
  });

  it('emphasizes nothing when the phrase is not there', () => {
    expect(splitSnippet('some text', 'absent')).toEqual([{ text: 'some text', match: false }]);
  });

  it('handles an empty snippet without inventing parts', () => {
    expect(splitSnippet('', 'query')).toEqual([{ text: '', match: false }]);
  });
});

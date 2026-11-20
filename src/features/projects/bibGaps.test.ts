import { describe, expect, it } from 'vitest';
import type { Paper } from '../../types/engine';
import { gapFix, gapLabel, gapsText } from './bibGaps';

const paper = (over: Partial<Paper>) =>
  ({ doi: null, arxiv: null, isPaper: true, lookedUpAt: null, ...over }) as Paper;

describe('gapLabel', () => {
  it('names a gap as the entry type names it', () => {
    expect(gapLabel('venue', 'article')).toBe('journal');
    expect(gapLabel('venue', 'inproceedings')).toBe('proceedings');
    expect(gapLabel('venue', null)).toBe('venue');
    expect(gapLabel('publisher', 'thesis')).toBe('school');
    expect(gapLabel('publisher', 'book')).toBe('publisher');
    expect(gapLabel('year', 'article')).toBe('year');
    expect(gapsText(['year', 'venue'])).toBe('no year, no venue');
  });
});

describe('gapFix', () => {
  it('offers the lookup that has not run, by identifier or else by title', () => {
    expect(gapFix(paper({ doi: '10.1/x' }), true)).toEqual({ kind: 'lookup' });
    expect(gapFix(paper({}), true)).toEqual({ kind: 'title' });
  });

  it('offers typing it in once a lookup has run, or when lookups are off', () => {
    expect(gapFix(paper({ doi: '10.1/x', lookedUpAt: '2026-11-19T00:00:00Z' }), true)).toEqual({
      kind: 'edit',
      lookedUp: true,
    });
    expect(gapFix(paper({ doi: '10.1/x' }), false)).toEqual({ kind: 'edit', lookedUp: false });
  });
});

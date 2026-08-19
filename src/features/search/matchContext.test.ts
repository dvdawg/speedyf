import { describe, expect, it } from 'vitest';
import { contextForMatch } from './matchContext';
import type { FormalEntry } from '../../types/engine';

const entry = (depth: number, label: string, page: number, charIndex: number): FormalEntry => ({
  depth,
  label,
  page,
  y: 0,
  charIndex,
});

const doc: FormalEntry[] = [
  entry(0, '1 Setup', 0, 10),
  entry(1, 'Definition 1.1', 0, 40),
  entry(1, 'Theorem 1.2', 0, 120),
  entry(0, '2 Results', 1, 5),
  entry(1, 'Theorem 2.1', 1, 60),
];

describe('contextForMatch', () => {
  it('attributes a hit to the environment it falls inside', () => {
    expect(contextForMatch(doc, 0, 130)).toEqual({
      section: '1 Setup',
      environment: 'Theorem 1.2',
    });
  });

  it('reports the section alone before the first environment in it', () => {
    expect(contextForMatch(doc, 0, 20)).toEqual({ section: '1 Setup', environment: null });
  });

  it('does not carry an environment across a section boundary', () => {
    // page 1 char 10 is past "2 Results" but before "Theorem 2.1"; the answer
    // is the new section, not the last theorem of the previous one
    expect(contextForMatch(doc, 1, 10)).toEqual({ section: '2 Results', environment: null });
  });

  it('crosses pages correctly', () => {
    expect(contextForMatch(doc, 1, 90)).toEqual({
      section: '2 Results',
      environment: 'Theorem 2.1',
    });
  });

  it('reports nothing for a hit before any structure', () => {
    expect(contextForMatch(doc, 0, 1)).toEqual({ section: null, environment: null });
  });

  it('reports nothing when the document has no index', () => {
    expect(contextForMatch([], 3, 50)).toEqual({ section: null, environment: null });
  });
});

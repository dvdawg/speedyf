/** Pasted DOIs and arXiv ids, read into the library's forms and turned into
 * citations of the items holding them. */
import { describe, expect, it } from 'vitest';
import {
  citeIdentifiers,
  identifierKey,
  normalizeArxiv,
  normalizeDoi,
  parseIdentifier,
  type Identifier,
} from './identifiers';

describe('identifiers', () => {
  it('reads every DOI form into the stored one', () => {
    for (const raw of [
      '10.1103/PhysRevLett.78.2690',
      'doi:10.1103/PhysRevLett.78.2690',
      'DOI: 10.1103/PhysRevLett.78.2690',
      'https://doi.org/10.1103/PhysRevLett.78.2690',
      'http://dx.doi.org/10.1103/PhysRevLett.78.2690',
      'doi.org/10.1103/PhysRevLett.78.2690',
    ]) {
      expect(normalizeDoi(raw)).toBe('10.1103/physrevlett.78.2690');
    }
    expect(normalizeDoi('https://doi.org/10.1000%2Fabc')).toBe('10.1000/abc');
    expect(normalizeDoi('11.1103/x')).toBeNull();
    expect(normalizeDoi('10.12/x')).toBeNull();
  });

  it('reads every arXiv form into the id without its version', () => {
    for (const raw of [
      '2401.01234',
      '2401.01234v3',
      'arXiv:2401.01234v2',
      'arxiv: 2401.01234',
      'https://arxiv.org/abs/2401.01234',
      'https://arxiv.org/abs/2401.01234v2',
      'https://arxiv.org/pdf/2401.01234v2',
      'https://arxiv.org/pdf/2401.01234.pdf',
      'arxiv.org/abs/2401.01234?context=cs',
    ]) {
      expect(normalizeArxiv(raw)).toBe('2401.01234');
    }
    expect(normalizeArxiv('math/0601001')).toBe('math/0601001');
    expect(normalizeArxiv('arXiv:hep-th/9901001v1')).toBe('hep-th/9901001');
    expect(normalizeArxiv('https://arxiv.org/abs/math.AG/0601001')).toBe('math.AG/0601001');
    expect(normalizeArxiv('0704.0001')).toBe('0704.0001');
    // Not ids: a month 13, before the new style began, a plain number.
    expect(normalizeArxiv('2413.01234')).toBeNull();
    expect(normalizeArxiv('0612.0001')).toBeNull();
    expect(normalizeArxiv('3.14159')).toBeNull();
    expect(normalizeArxiv('math/06010')).toBeNull();
  });

  it('takes a DOI first, so an arXiv DOI stays a DOI', () => {
    expect(parseIdentifier('10.48550/arXiv.2401.01234')).toEqual({
      kind: 'doi',
      id: '10.48550/arxiv.2401.01234',
    });
    expect(parseIdentifier('hello')).toBeNull();
  });

  const library: Record<string, string> = {
    'doi:10.1103/physrevlett.78.2690': 'jarzynski1997',
    'arxiv:2401.01234': 'zd24',
    'arxiv:math/0601001': 'perelman06',
    'doi:10.1000/a(b)': 'paren',
  };
  const resolve = (id: Identifier) => library[identifierKey(id)] ?? null;

  it('turns a paste that is one known identifier into a citation', () => {
    expect(citeIdentifiers('https://doi.org/10.1103/PhysRevLett.78.2690', resolve)).toBe(
      '@jarzynski1997'
    );
    expect(citeIdentifiers('arXiv:2401.01234v2', resolve)).toBe('@zd24');
    expect(citeIdentifiers('  https://arxiv.org/pdf/2401.01234v1\n', resolve)).toBe('  @zd24\n');
    expect(citeIdentifiers('math/0601001', resolve)).toBe('@perelman06');
    expect(citeIdentifiers('10.1000/a(b)', resolve)).toBe('@paren');
  });

  it('leaves an identifier no item holds, and text with none, alone', () => {
    expect(citeIdentifiers('10.9999/unknown', resolve)).toBeNull();
    expect(citeIdentifiers('arXiv:2402.00001', resolve)).toBeNull();
    expect(citeIdentifiers('just some words', resolve)).toBeNull();
  });

  it('replaces only the whole known identifiers in mixed text', () => {
    expect(
      citeIdentifiers(
        'Compare (arXiv:2401.01234), doi:10.9999/nope and 10.1103/physrevlett.78.2690. Also x2401.01234',
        resolve
      )
    ).toBe('Compare (@zd24), doi:10.9999/nope and @jarzynski1997. Also x2401.01234');
  });
});

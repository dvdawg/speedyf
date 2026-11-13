/**
 * How a year its references give and a value citing papers agreed on read
 * on screen.
 */
import { describe, expect, it } from 'vitest';
import { metaLine, sourceText, yearText } from './paperFields';
import { ROWS } from './testFixtures';

describe('yearText', () => {
  it('shows the year, else the floor with a plus, else nothing', () => {
    expect(yearText({ year: 2015, yearFloor: 2019 })).toBe('2015');
    expect(yearText({ year: null, yearFloor: 2019 })).toBe('2019+');
    expect(yearText({ year: null })).toBe('');
    expect(yearText({ year: null, yearFloor: null })).toBe('');
  });

  it('puts the floor in a list row line where the year goes', () => {
    const row = { ...ROWS[0]!, authors: 'Zhong & DeWeese', year: null, venue: null };
    expect(metaLine({ ...row, yearFloor: 2019 })).toBe('Zhong & DeWeese, 2019+');
    expect(metaLine(row)).toBe('Zhong & DeWeese');
  });
});

describe('sourceText', () => {
  it('counts the citing papers that vouched', () => {
    expect(sourceText('citations', 3)).toBe('from 3 citing papers');
    expect(sourceText('citations', 1)).toBe('from 1 citing paper');
    expect(sourceText('citations')).toBe('citing papers');
    expect(sourceText('crossref', 3)).toBe('Crossref');
  });
});

import { describe, expect, it } from 'vitest';
import { choiceLabel, optionLabel } from './printLabels';

describe('optionLabel', () => {
  it('renames the options CUPS spells badly', () => {
    expect(optionLabel('cupsPrintQuality', 'Quality')).toBe('Quality');
    expect(optionLabel('Duplex', '2-Sided Printing')).toBe('Two-sided');
    expect(optionLabel('PageSize', 'Media Size')).toBe('Paper');
  });

  it("falls back to the printer's own label, then the key", () => {
    expect(optionLabel('StapleLocation', 'Staple Location')).toBe('Staple Location');
    expect(optionLabel('OddThing', 'OddThing')).toBe('OddThing');
    expect(optionLabel('OddThing', '')).toBe('OddThing');
  });
});

describe('choiceLabel', () => {
  it('translates the values a person should never see', () => {
    expect(choiceLabel('Duplex', 'DuplexNoTumble')).toBe('On, long edge');
    expect(choiceLabel('Duplex', 'DuplexTumble')).toBe('On, short edge');
    expect(choiceLabel('Duplex', 'None')).toBe('Off');
    expect(choiceLabel('ColorModel', 'Gray')).toBe('Black and white');
    expect(choiceLabel('ColorModel', 'RGB')).toBe('Color');
  });

  it('leaves paper sizes alone', () => {
    expect(choiceLabel('PageSize', 'Letter')).toBe('Letter');
    expect(choiceLabel('PageSize', 'A4')).toBe('A4');
  });

  it('spaces out run-together names it does not know', () => {
    expect(choiceLabel('Whatever', 'PhotoGlossy')).toBe('Photo Glossy');
  });
});

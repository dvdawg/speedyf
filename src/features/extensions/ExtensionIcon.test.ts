import { describe, expect, it } from 'vitest';
import { initialsFor, isIconDataUri } from './ExtensionIcon';

describe('initialsFor', () => {
  it('takes the first letter of the first two words', () => {
    expect(initialsFor('Page Audit')).toBe('PA');
    expect(initialsFor('BibTeX Export')).toBe('BE');
  });

  it('falls back to the first two characters of a single word', () => {
    expect(initialsFor('Scratchpad')).toBe('SC');
    expect(initialsFor('Vim')).toBe('VI');
  });

  it('never returns more than two characters', () => {
    expect(initialsFor('One Two Three Four').length).toBe(2);
    expect(initialsFor('Supercalifragilistic').length).toBe(2);
  });

  it('treats separators as word breaks', () => {
    expect(initialsFor('page-audit')).toBe('PA');
    expect(initialsFor('page_audit')).toBe('PA');
  });

  /** Names are author-supplied, so leading punctuation and emoji are ordinary
   * input rather than an edge case worth crashing on. */
  it('skips leading punctuation when picking a word', () => {
    expect(initialsFor('★ Starred Notes')).toBe('SN');
    expect(initialsFor('  spaced  out  ')).toBe('SO');
  });

  it('answers something rather than nothing for an unusable name', () => {
    expect(initialsFor('')).toBe('?');
    expect(initialsFor('   ')).toBe('?');
    expect(initialsFor('!!!')).toBe('!!');
  });
});

/** The value reaches a stylesheet, so it is checked rather than trusted. The
 * backend builds it, but "the backend builds it" is an assumption that a future
 * change can quietly break. */
describe('isIconDataUri', () => {
  it('accepts the image types an icon can be', () => {
    expect(isIconDataUri('data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=')).toBe(true);
    expect(isIconDataUri('data:image/png;base64,iVBORw0KGgo=')).toBe(true);
    expect(isIconDataUri('data:image/jpeg;base64,/9j/4AAQ')).toBe(true);
  });

  it('refuses anything that is not one of ours', () => {
    // The old scheme-based URL, which paints nothing as a mask.
    expect(isIconDataUri('speedyf-ext://localhost/com.example.thing/icon.svg')).toBe(false);
    expect(isIconDataUri('data:text/html;base64,PHNjcmlwdD4=')).toBe(false);
    expect(isIconDataUri('https://example.com/icon.svg')).toBe(false);
  });

  it('refuses a value carrying characters that could end the url() token', () => {
    expect(isIconDataUri('data:image/png;base64,AAAA") ; background: url("evil')).toBe(false);
    expect(isIconDataUri("data:image/png;base64,AAAA') url('x")).toBe(false);
  });
});

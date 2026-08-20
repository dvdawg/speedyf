import { describe, expect, it } from 'vitest';
import { parseScriptMarkup, plainScriptText } from './scriptMarkup';

describe('parseScriptMarkup', () => {
  it('lifts a single-character superscript out of the prose', () => {
    expect(parseScriptMarkup('L^2 Optimality')).toEqual([
      { text: 'L', kind: 'normal' },
      { text: '2', kind: 'super' },
      { text: ' Optimality', kind: 'normal' },
    ]);
  });

  it('lowers a subscript', () => {
    expect(parseScriptMarkup('Canonicality of r_σ')).toEqual([
      { text: 'Canonicality of r', kind: 'normal' },
      { text: 'σ', kind: 'sub' },
    ]);
  });

  it('reads a braced multi-character script', () => {
    expect(parseScriptMarkup('α_{ext} across')).toEqual([
      { text: 'α', kind: 'normal' },
      { text: 'ext', kind: 'sub' },
      { text: ' across', kind: 'normal' },
    ]);
  });

  it('leaves a marker with nothing to lift alone', () => {
    expect(parseScriptMarkup('a trailing ^')).toEqual([{ text: 'a trailing ^', kind: 'normal' }]);
    expect(parseScriptMarkup('snake_')).toEqual([{ text: 'snake_', kind: 'normal' }]);
  });

  it('does not swallow a title on an unclosed brace', () => {
    expect(parseScriptMarkup('L^{2 Optimality')).toEqual([
      { text: 'L^{2 Optimality', kind: 'normal' },
    ]);
  });

  it('handles plain text without markup', () => {
    expect(parseScriptMarkup('1. Introduction')).toEqual([
      { text: '1. Introduction', kind: 'normal' },
    ]);
  });

  it('handles several scripts in one string', () => {
    expect(parseScriptMarkup('S^d and R_n')).toEqual([
      { text: 'S', kind: 'normal' },
      { text: 'd', kind: 'super' },
      { text: ' and R', kind: 'normal' },
      { text: 'n', kind: 'sub' },
    ]);
  });
});

describe('plainScriptText', () => {
  it('strips the markup for tooltips and search', () => {
    expect(plainScriptText('L^2 Optimality')).toBe('L2 Optimality');
    expect(plainScriptText('α_{ext} across')).toBe('αext across');
  });
});

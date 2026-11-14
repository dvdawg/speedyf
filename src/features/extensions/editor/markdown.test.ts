import { describe, expect, it } from 'vitest';
import {
  citationsIn,
  codeSource,
  inlineSpans,
  lineShape,
  locatorPage,
  mathSource,
  sourceOffset,
  splitBlocks,
} from './markdown';
import { applyOp, diffOp, invertOp, mapPos, transformOp } from './ops';
import { reuseBlocks } from './blocks';

describe('note blocks', () => {
  it('cut a note into lines and display blocks, and join back exactly', () => {
    const text = [
      '# Title',
      '',
      'A line with $x$.',
      '$$',
      'a = b',
      '$$',
      '$$ c = d $$',
      '```js',
      'let x = 1;',
      '```',
      '- item',
    ].join('\n');
    const blocks = splitBlocks(text);
    expect(blocks.map((b) => b.kind)).toEqual([
      'line',
      'line',
      'line',
      'math',
      'math',
      'code',
      'line',
    ]);
    expect(blocks.map((b) => b.raw).join('\n')).toBe(text);
    for (const block of blocks)
      expect(text.slice(block.start, block.start + block.raw.length)).toBe(block.raw);
    expect(mathSource(blocks[3]!.raw)).toBe('a = b');
    expect(mathSource(blocks[4]!.raw)).toBe('c = d');
    expect(codeSource(blocks[5]!.raw)).toBe('let x = 1;');
  });

  it('an unclosed display block runs to the end rather than losing text', () => {
    const blocks = splitBlocks('before\n$$\nx = 1');
    expect(blocks.map((b) => b.raw)).toEqual(['before', '$$\nx = 1']);
    expect(splitBlocks('')).toEqual([{ kind: 'line', raw: '', start: 0 }]);
  });

  it('reads line shapes', () => {
    expect(lineShape('## Sub')).toEqual({ kind: 'heading', level: 2, body: 'Sub' });
    expect(lineShape('  - a')).toMatchObject({ kind: 'bullet', indent: 2, body: 'a' });
    expect(lineShape('3. c')).toMatchObject({ kind: 'ordered', marker: '3.', body: 'c' });
    expect(lineShape('> > deep')).toMatchObject({ kind: 'quote', depth: 2, body: 'deep' });
    expect(lineShape('---')).toEqual({ kind: 'rule' });
    expect(lineShape('   ')).toEqual({ kind: 'blank' });
  });
});

describe('inline spans', () => {
  it('reads emphasis, code, math, links, images and escapes', () => {
    expect(
      inlineSpans('a *b* **c** `d` $e$ [f](https://x.org) ![g](assets/h.png) \\*i\\*')
    ).toEqual([
      { kind: 'text', text: 'a ' },
      { kind: 'em', children: [{ kind: 'text', text: 'b' }] },
      { kind: 'text', text: ' ' },
      { kind: 'strong', children: [{ kind: 'text', text: 'c' }] },
      { kind: 'text', text: ' ' },
      { kind: 'code', text: 'd' },
      { kind: 'text', text: ' ' },
      { kind: 'math', tex: 'e' },
      { kind: 'text', text: ' ' },
      { kind: 'link', href: 'https://x.org', children: [{ kind: 'text', text: 'f' }] },
      { kind: 'text', text: ' ' },
      { kind: 'image', src: 'assets/h.png', alt: 'g' },
      { kind: 'text', text: ' *i*' },
    ]);
    // Prices and snake_case are not markup.
    expect(inlineSpans('$5 and $10, snake_case_name')).toEqual([
      { kind: 'text', text: '$5 and $10, snake_case_name' },
    ]);
  });

  it('reads citations, bare and bracketed with a locator and a place', () => {
    const spans = inlineSpans(
      'As @attias2022 shows [@smith-1999, p. 12]{box=72,310,240,28}; mail@x.org is not one.'
    );
    const cites = spans.filter((s) => s.kind === 'cite');
    expect(cites).toEqual([
      {
        kind: 'cite',
        raw: '@attias2022',
        cite: { key: 'attias2022', locator: null, page: null, box: null },
      },
      {
        kind: 'cite',
        raw: '[@smith-1999, p. 12]{box=72,310,240,28}',
        cite: { key: 'smith-1999', locator: 'p. 12', page: 11, box: [72, 310, 240, 28] },
      },
    ]);
    expect(locatorPage('pp. 3-5')).toBe(2);
    expect(locatorPage('chap. 2')).toBeNull();
  });

  it('finds every citation with its offset', () => {
    const text = '# A\nSee @k1.\n> "q" [@k2, p. 2]';
    expect(citationsIn(text).map((c) => [c.cite.key, c.offset])).toEqual([
      ['k1', text.indexOf('@k1')],
      ['k2', text.indexOf('[@k2')],
    ]);
  });

  it('maps a click in drawn text back to the source', () => {
    expect(sourceOffset('**bold** text', 'bold text', 4)).toBe(6);
    expect(sourceOffset('## Head', 'Head', 0)).toBe(0);
  });
});

describe('edits', () => {
  it('apply, invert, diff and map like the core does', () => {
    const before = 'hello world';
    const op = { from: 0, to: 5, insert: 'HELLO' };
    const after = applyOp(before, op);
    expect(after).toBe('HELLO world');
    expect(applyOp(after, invertOp(op, before))).toBe(before);
    expect(diffOp('abcdef', 'abXYef')).toEqual({ from: 2, to: 4, insert: 'XY' });
    expect(diffOp('same', 'same')).toBeNull();
    // An insertion tie: before unless the position is later's.
    expect(mapPos({ from: 2, to: 2, insert: 'ab' }, 2)).toBe(2);
    expect(mapPos({ from: 2, to: 2, insert: 'ab' }, 2, true)).toBe(4);
    // A range ending where a replacement starts is not swallowed by it.
    expect(
      transformOp({ from: 0, to: 3, insert: '' }, { from: 3, to: 5, insert: 'Z' }, true)
    ).toEqual({
      from: 0,
      to: 3,
      insert: '',
    });
  });

  it('non-ASCII text is just text', () => {
    const text = 'Schrödinger \u{1d49c} α';
    const op = diffOp(text, `${text}!`)!;
    expect(applyOp(text, op)).toBe(`${text}!`);
    expect(splitBlocks(text)[0]!.raw).toBe(text);
  });
});

describe('a very long note', () => {
  const line = 'Some text with $x^2$ and @attias2022 in it, and **more** words to read.';
  const big = Array.from({ length: Math.ceil(1_048_576 / (line.length + 1)) }, (_, i) =>
    i % 40 === 0 ? `## Section ${i}` : line
  ).join('\n');

  it('is about a megabyte', () => {
    expect(big.length).toBeGreaterThan(1_000_000);
  });

  it('cuts, re-cuts after an edit reusing every untouched block, and diffs quickly', () => {
    const started = performance.now();
    const first = reuseBlocks([], big);
    const middle = Math.floor(big.length / 2);
    const edited = applyOp(big, { from: middle, to: middle, insert: 'x' });
    const second = reuseBlocks(first, edited);
    const op = diffOp(big, edited);
    const elapsed = performance.now() - started;
    expect(op).toEqual({ from: middle, to: middle, insert: 'x' });
    // One block is new; every other is an object the list already draws.
    const known = new Set(first);
    expect(second.filter((b) => !known.has(b))).toHaveLength(1);
    // Generous for a loaded CI machine; typically tens of milliseconds.
    expect(elapsed).toBeLessThan(1_500);
  });
});

/** The outline: which lines are headings, at what depth, reading as text. */
import { describe, expect, it } from 'vitest';
import { inlineSpans, splitBlocks } from './markdown';
import { headingLine, outlineOf, plainText } from './outline';

function outline(text: string) {
  const found = splitBlocks(text).flatMap((b) => {
    const h = headingLine(b.kind, b.raw);
    return h ? [{ ...h, offset: b.start }] : [];
  });
  return outlineOf(found);
}

describe('outline', () => {
  it('lists headings with their offsets, depths from the shallowest level', () => {
    const text = 'Intro\n## Methods\ntext\n### Setup\n#### Detail\n##### Deeper\n## Results';
    expect(outline(text)).toEqual([
      { depth: 0, text: 'Methods', offset: text.indexOf('## Methods') },
      { depth: 1, text: 'Setup', offset: text.indexOf('### Setup') },
      { depth: 2, text: 'Detail', offset: text.indexOf('#### Detail') },
      // A fourth level is left out.
      { depth: 0, text: 'Results', offset: text.indexOf('## Results') },
    ]);
  });

  it('is not fooled by a # in code, math or a quote', () => {
    const text = '```\n# not\n```\n> # quoted\n$$\n# x\n$$\n#nospace\n# Real';
    expect(outline(text)).toEqual([{ depth: 0, text: 'Real', offset: text.indexOf('# Real') }]);
  });

  it('reads inline Markdown as text and math as its TeX', () => {
    expect(plainText(inlineSpans('The *bound* on $x^2$ with `code` and [a link](u) @k'))).toBe(
      'The bound on x^2 with code and a link @k'
    );
    expect(outlineOf([{ level: 1, body: '**Bold** $\\alpha$', offset: 0 }])[0]!.text).toBe(
      'Bold \\alpha'
    );
  });
});

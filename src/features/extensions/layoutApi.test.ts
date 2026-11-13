/** The `document:layout` and `network` surface: who may call what, the
 * shapes arguments must have, and the copy formats of the math popover. */
import { describe, expect, it } from 'vitest';
import {
  apiRefusalReason,
  canCallApi,
  readFetchArgs,
  readRegionArg,
  readShowMathArgs,
  toGlyphs,
  toMathTags,
} from './api';
import {
  alignCopy,
  hasLineBreaks,
  latexCopy,
  markdownCopy,
  provenanceNote,
  renderSource,
} from './mathCopy';
import { placeMathPopover } from './mathPopoverStore';

describe('permissions', () => {
  it('gates glyphs and math tags behind document:layout', () => {
    for (const api of ['selection.glyphs', 'region.glyphs', 'document.mathTags']) {
      expect(canCallApi(['document:layout'], api)).toBe(true);
      expect(canCallApi(['document:read'], api)).toBe(false);
      expect(apiRefusalReason(['document:read'], api)).toBe(
        `"${api}" needs the "document:layout" permission (granted: document:read)`
      );
    }
  });

  it('gates identifiers behind document:read', () => {
    expect(canCallApi(['document:read'], 'document.identifiers')).toBe(true);
    expect(canCallApi(['document:layout'], 'document.identifiers')).toBe(false);
  });

  it('gates net.fetch behind network', () => {
    expect(canCallApi(['network'], 'net.fetch')).toBe(true);
    expect(canCallApi(['document:read', 'document:layout'], 'net.fetch')).toBe(false);
    expect(apiRefusalReason([], 'net.fetch')).toBe('"net.fetch" needs the "network" permission');
  });
});

describe('arguments', () => {
  it('reads a region', () => {
    expect(readRegionArg({ page: 2, bbox: [10, 20, 30, 40] }, 5)).toEqual({
      page: 2,
      bbox: [10, 20, 30, 40],
    });
    expect(() => readRegionArg({ page: 9, bbox: [0, 0, 1, 1] }, 5)).toThrow(/outside/);
    expect(() => readRegionArg({ page: 0, bbox: [0, 0, 1] }, 5)).toThrow(/bbox/);
    expect(() => readRegionArg({ page: 0, bbox: [0, 0, 0, 5] }, 5)).toThrow(/positive/);
    expect(() => readRegionArg({ page: 0, bbox: [0, 0, NaN, 5] }, 5)).toThrow(/bbox/);
  });

  it('reads net.fetch arguments, defaulting to text', () => {
    expect(readFetchArgs({ url: 'https://arxiv.org/x' })).toEqual({
      url: 'https://arxiv.org/x',
      kind: 'text',
      maxBytes: null,
    });
    expect(readFetchArgs({ url: 'https://arxiv.org/x', as: 'bytes', maxBytes: 1000 })).toEqual({
      url: 'https://arxiv.org/x',
      kind: 'bytes',
      maxBytes: 1000,
    });
    expect(() => readFetchArgs({})).toThrow(/url/);
    expect(() => readFetchArgs({ url: 'https://a.org', as: 'json' })).toThrow(/"as"/);
    expect(() => readFetchArgs({ url: 'https://a.org', maxBytes: -1 })).toThrow(/maxBytes/);
    expect(() => readFetchArgs({ url: 'https://a.org', maxBytes: 1.5 })).toThrow(/maxBytes/);
  });

  it('reads ui.showMath arguments strictly', () => {
    const good = { latex: 'x^2', display: true, source: 'arxiv', confidence: 'high' };
    expect(readShowMathArgs(good)).toEqual(good);
    expect(() => readShowMathArgs({ ...good, latex: ' ' })).toThrow(/latex/);
    expect(() => readShowMathArgs({ ...good, display: 'yes' })).toThrow(/display/);
    expect(() => readShowMathArgs({ ...good, source: 'web' })).toThrow(/source/);
    expect(() => readShowMathArgs({ ...good, confidence: 'certain' })).toThrow(/confidence/);
  });
});

describe('mapping', () => {
  it('copies glyphs field by field and nothing else', () => {
    const engineGlyph = {
      text: 'x',
      code: 120,
      font: 'CMMI10',
      size: 10,
      x: 1,
      y: 2,
      w: 3,
      h: 4,
      baseline: 5,
      bold: false,
      italic: true,
      ink: [1, 2.5, 3, 2] as const,
      internal: 'leak',
    };
    const out = toGlyphs(3, [engineGlyph]);
    expect(out.page).toBe(3);
    expect(out.glyphs[0]).toEqual({
      text: 'x',
      code: 120,
      font: 'CMMI10',
      size: 10,
      x: 1,
      y: 2,
      w: 3,
      h: 4,
      baseline: 5,
      bold: false,
      italic: true,
      ink: [1, 2.5, 3, 2],
    });
    // A rule, or a glyph PDFium cannot measure, has none.
    expect(toGlyphs(0, [{ ...engineGlyph, ink: null }]).glyphs[0]?.ink).toBeNull();
  });

  it('fills absent math tag fields with null', () => {
    expect(toMathTags([{ bbox: [1, 2, 3, 4], alt: 'a' }])).toEqual([
      { bbox: [1, 2, 3, 4], alt: 'a', actualText: null, mathml: null },
    ]);
  });
});

describe('copy formats', () => {
  it('copies raw LaTeX trimmed', () => {
    expect(latexCopy('  \\frac{a}{b} ')).toBe('\\frac{a}{b}');
  });

  it('copies Markdown inline or displayed', () => {
    expect(markdownCopy('x^2', false)).toBe('$x^2$');
    expect(markdownCopy('x^2', true)).toBe('$$x^2$$');
  });

  it('offers align only for line breaks', () => {
    expect(hasLineBreaks('a = b')).toBe(false);
    expect(hasLineBreaks('a &= b \\\\ c &= d')).toBe(true);
  });

  it('wraps bare rows in align', () => {
    expect(alignCopy('a &= b \\\\ c &= d')).toBe(
      '\\begin{align}\na &= b \\\\ c &= d\n\\end{align}'
    );
  });

  it('moves the rows of an aligned block across rather than nesting it', () => {
    expect(alignCopy('\\begin{aligned} a &= b \\\\ c &= d \\end{aligned}')).toBe(
      '\\begin{align}\na &= b \\\\ c &= d\n\\end{align}'
    );
    expect(alignCopy('\\begin{split} a &= b \\\\ &= c \\\\ \\end{split}')).toBe(
      '\\begin{align}\na &= b \\\\ &= c\n\\end{align}'
    );
  });

  it('renders bare rows and align blocks as aligned for KaTeX', () => {
    expect(renderSource('a \\\\ b')).toBe('\\begin{aligned}a \\\\ b\\end{aligned}');
    expect(renderSource('\\begin{align}a&=b\\\\c&=d\\end{align}')).toBe(
      '\\begin{aligned}a&=b\\\\c&=d\\end{aligned}'
    );
    expect(renderSource('\\frac{a}{b}')).toBe('\\frac{a}{b}');
  });

  it('names the source in plain words', () => {
    expect(provenanceNote('tags', 'high')).toBe('from PDF tags, high confidence');
    expect(provenanceNote('arxiv', 'high')).toBe('from arXiv source, high confidence');
    expect(provenanceNote('jats', 'medium')).toBe('from journal version, medium confidence');
    expect(provenanceNote('fonts', 'low')).toBe('from fonts, low confidence');
  });
});

describe('popover placement', () => {
  const viewport = { width: 1000, height: 800 };
  const size = { width: 400, height: 150 };

  it('sits under the anchor, centred, when there is room', () => {
    expect(
      placeMathPopover({ left: 400, top: 100, right: 600, bottom: 120 }, size, viewport)
    ).toEqual({ left: 300, top: 126 });
  });

  it('goes above an anchor near the bottom and stays inside the window', () => {
    const at = placeMathPopover({ left: 950, top: 700, right: 990, bottom: 720 }, size, viewport);
    expect(at.top).toBe(700 - 6 - 150);
    expect(at.left).toBe(1000 - 400 - 8);
  });
});

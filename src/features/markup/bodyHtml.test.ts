import { describe, expect, it } from 'vitest';
import { escapeHtml, inlineHtml, quoteHtml, renderBody } from './bodyHtml';

const math = (tex: string, display: boolean) => `<k${display ? ' d' : ''}>${tex}</k>`;
const cite = (key: string) => (key === 'jar97' ? 'Jar97' : null);

describe('comment bodies as HTML', () => {
  it("escapes the reviewer's text", () => {
    expect(escapeHtml('<b>"x" & y</b>')).toBe('&lt;b&gt;&quot;x&quot; &amp; y&lt;/b&gt;');
    expect(renderBody('<script>alert(1)</script>')).toBe(
      '<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>'
    );
  });

  it('typesets math, and shows its source until KaTeX is there', () => {
    expect(inlineHtml('take $x^2$ here', { math })).toBe(
      'take <span class="mk-math"><k>x^2</k></span> here'
    );
    expect(inlineHtml('take $x<2$', {})).toBe('take <span class="mk-math-raw">$x&lt;2$</span>');
    expect(renderBody('$$\\int f$$', { math })).toBe(
      '<div class="mk-display"><k d>\\int f</k></div>'
    );
  });

  it('sets $$...$$ inside a sentence as display math', () => {
    expect(renderBody('check $$\\sum k$$ holds, $x$ too', { math })).toBe(
      '<p>check <span class="mk-display"><k d>\\sum k</k></span> holds, ' +
        '<span class="mk-math"><k>x</k></span> too</p>'
    );
    expect(renderBody('a $$y$$ b')).toBe('<p>a <span class="mk-math-raw">$$y$$</span> b</p>');
  });

  it('reads citations as labels and marks unknown ones', () => {
    expect(inlineHtml('see [@jar97, p. 2].', { cite })).toBe(
      'see <span class="mk-cite">[Jar97, p. 2]</span>.'
    );
    expect(inlineHtml('[@who]', { cite })).toBe('<span class="mk-cite is-unknown">[@who]</span>');
  });

  it('keeps paragraphs, line breaks, lists and light emphasis', () => {
    expect(renderBody('One **bold** and *it* and `c`.\nNext line.\n\n- a\n- b\n  more')).toBe(
      '<p>One <strong>bold</strong> and <em>it</em> and <code>c</code>.<br>Next line.</p>' +
        '<ul><li>a</li><li>b more</li></ul>'
    );
    expect(renderBody('  \n\n ')).toBe('');
  });
});

describe('quotes as HTML', () => {
  it('typesets inline math and displays, and escapes the prose', () => {
    expect(quoteHtml('if <a> $x$ then\n\n$$ y = 1 $$ so a\\_b', { math })).toBe(
      'if &lt;a&gt; <span class="mk-math"><k>x</k></span> then ' +
        '<span class="mk-display"><k d>y = 1</k></span> so a_b'
    );
  });

  it('shows the source until KaTeX is there', () => {
    expect(quoteHtml('$x$')).toBe('<span class="mk-math-raw">$x$</span>');
  });
});

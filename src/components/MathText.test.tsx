/** @vitest-environment happy-dom */
import { describe, expect, it } from 'vitest';
import { render, waitFor } from '@solidjs/testing-library';
import MathText, { loadKatex, splitMath } from './MathText';

describe('splitMath', () => {
  it('leaves prices alone', () => {
    expect(splitMath('$5 and $10')).toEqual([{ kind: 'text', text: '$5 and $10' }]);
    expect(splitMath('costs $5.')).toEqual([{ kind: 'text', text: 'costs $5.' }]);
    expect(splitMath('from $ 3 to $ 4')).toEqual([{ kind: 'text', text: 'from $ 3 to $ 4' }]);
  });

  it('finds $...$ math', () => {
    expect(splitMath('Energy $x^2$ scaling')).toEqual([
      { kind: 'text', text: 'Energy ' },
      { kind: 'math', src: 'x^2', raw: '$x^2$', display: false },
      { kind: 'text', text: ' scaling' },
    ]);
  });

  it('does not close on a $ followed by a digit or preceded by a space', () => {
    expect(splitMath('$a$5')).toEqual([{ kind: 'text', text: '$a$5' }]);
    expect(splitMath('$5 and $x$')).toEqual([
      { kind: 'text', text: '$5 and ' },
      { kind: 'math', src: 'x', raw: '$x$', display: false },
    ]);
  });

  it('finds \\(...\\) and \\[...\\]', () => {
    expect(splitMath('\\(a\\) and \\[b\\]')).toEqual([
      { kind: 'math', src: 'a', raw: '\\(a\\)', display: false },
      { kind: 'text', text: ' and ' },
      { kind: 'math', src: 'b', raw: '\\[b\\]', display: true },
    ]);
  });

  it('keeps escaped dollars as text', () => {
    expect(splitMath('a \\$x\\$ b')).toEqual([{ kind: 'text', text: 'a $x$ b' }]);
    expect(splitMath('$\\$5$')).toEqual([
      { kind: 'math', src: '\\$5', raw: '$\\$5$', display: false },
    ]);
  });

  it('leaves unclosed delimiters and $$ as text', () => {
    expect(splitMath('half $x open')).toEqual([{ kind: 'text', text: 'half $x open' }]);
    expect(splitMath('\\(never closed')).toEqual([{ kind: 'text', text: '\\(never closed' }]);
    expect(splitMath('$$x$$')).toEqual([{ kind: 'text', text: '$$x$$' }]);
  });

  it('memoizes per string', () => {
    expect(splitMath('same $y$ text')).toBe(splitMath('same $y$ text'));
  });
});

describe('MathText', () => {
  it('shows the raw source until KaTeX arrives, then typesets', async () => {
    const { container } = render(() => <MathText text="$\geq 2$" />);
    expect(container.querySelector('.math-raw')!.textContent).toBe('$\\geq 2$');
    await waitFor(() => expect(container.querySelector('.katex')).not.toBeNull());
  });

  it('renders math with KaTeX', async () => {
    await loadKatex();
    const { container } = render(() => <MathText text="Area $\pi r^2$ here" class="t" />);
    const span = container.querySelector('.t')!;
    expect(span.querySelector('.math-run .katex')).not.toBeNull();
    expect(span.textContent).toContain('Area ');
    expect(span.textContent).toContain(' here');
  });

  it('escapes plain text instead of parsing it as HTML', async () => {
    await loadKatex();
    const evil = '<img src=x onerror="alert(1)"> and $x$';
    const { container } = render(() => <MathText text={evil} class="t" />);
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('.t')!.textContent).toContain('<img src=x onerror="alert(1)">');
  });

  it('does not let math source inject HTML either', async () => {
    await loadKatex();
    const { container } = render(() => <MathText text={'$\\text{<img src=x onerror=1>}$'} />);
    expect(container.querySelector('img')).toBeNull();
  });

  it('renders text with no math as plain text nodes', () => {
    const { container } = render(() => <MathText text="Plain title" />);
    expect(container.innerHTML).toBe('Plain title');
  });
});

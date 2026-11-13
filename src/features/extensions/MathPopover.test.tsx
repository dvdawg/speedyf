// @vitest-environment happy-dom
/** The popover `ui.showMath` fills: it shows the formula and the note, its
 * buttons copy the three formats on the click itself, an extension's error
 * appears in the same place, and Escape closes it. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import MathPopover from './MathPopover';
import {
  closeMath,
  mathPopover,
  openMathLoading,
  settleMath,
  showMath,
  showMathError,
} from './mathPopoverStore';

const anchor = { left: 100, top: 100, right: 300, bottom: 140 };
const writeText = vi.fn(async (_text: string) => {});

beforeEach(() => {
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText },
    configurable: true,
  });
});

afterEach(() => {
  cleanup();
  closeMath();
  writeText.mockClear();
});

async function clickCopy(label: string): Promise<string> {
  fireEvent.click(screen.getByRole('button', { name: label }));
  await Promise.resolve();
  return writeText.mock.calls.at(-1)![0];
}

describe('MathPopover', () => {
  it('shows nothing until something is asked', () => {
    render(() => <MathPopover />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows a loading line while the action runs, then closes if nothing is shown', () => {
    render(() => <MathPopover />);
    openMathLoading('com.example.math', anchor, 'Copy LaTeX');
    expect(screen.getByRole('status').textContent).toBe('Copy LaTeX...');
    settleMath('com.example.math');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('renders the formula with the note and copies the three formats', async () => {
    render(() => <MathPopover />);
    openMathLoading('com.example.math', anchor, 'Copy LaTeX');
    showMath('com.example.math', {
      latex: 'a &= b \\\\ c &= d',
      display: true,
      source: 'arxiv',
      confidence: 'high',
    });
    expect(screen.getByRole('dialog', { name: 'Formula' })).toBeTruthy();
    expect(screen.getByText('from arXiv source, high confidence')).toBeTruthy();
    expect(await clickCopy('LaTeX')).toBe('a &= b \\\\ c &= d');
    expect(await clickCopy('Markdown')).toBe('$$a &= b \\\\ c &= d$$');
    expect(await clickCopy('align')).toBe('\\begin{align}\na &= b \\\\ c &= d\n\\end{align}');
    // The anchor the action opened at is kept.
    expect(mathPopover()?.anchor).toEqual(anchor);
  });

  it('offers no align copy for a single line, and copies inline Markdown', async () => {
    render(() => <MathPopover />);
    showMath('com.example.math', {
      latex: 'x^2',
      display: false,
      source: 'fonts',
      confidence: 'low',
    });
    expect(screen.queryByRole('button', { name: 'align' })).toBeNull();
    expect(await clickCopy('Markdown')).toBe('$x^2$');
    expect(screen.getByText('from fonts, low confidence')).toBeTruthy();
  });

  it('shows an extension error in the same popover', () => {
    render(() => <MathPopover />);
    openMathLoading('com.example.math', anchor, 'Copy LaTeX');
    showMathError('com.example.math', 'This looks like a scan; there is no text to read.');
    expect(screen.getByRole('alert').textContent).toBe(
      'This looks like a scan; there is no text to read.'
    );
  });

  it('ignores an error from an extension it is not waiting on', () => {
    render(() => <MathPopover />);
    openMathLoading('com.example.math', anchor, 'Copy LaTeX');
    showMathError('com.example.other', 'nope');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('closes on Escape and on a click elsewhere', () => {
    render(() => <MathPopover />);
    showMath('com.example.math', {
      latex: 'x',
      display: false,
      source: 'tags',
      confidence: 'high',
    });
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();

    showMath('com.example.math', {
      latex: 'x',
      display: false,
      source: 'tags',
      confidence: 'high',
    });
    fireEvent.pointerDown(screen.getByRole('dialog'));
    expect(screen.queryByRole('dialog')).toBeTruthy();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

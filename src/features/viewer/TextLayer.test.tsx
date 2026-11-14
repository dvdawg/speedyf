// @vitest-environment happy-dom
import { afterEach, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import TextLayer from './TextLayer';
import type { TextRunDto } from '../../types/engine';

afterEach(() => {
  cleanup();
  document.getSelection()?.removeAllRanges();
});

const run = (text: string, y: number): TextRunDto => ({ text, start: 0, x: 60, y, w: 200, h: 10 });

const runs = [
  run('first paragraph one', 700),
  run('first paragraph two', 688),
  run('second paragraph one', 650),
  run('second paragraph two', 638),
];

it('selects the paragraph under a triple-click as a drag would', () => {
  const { container } = render(() => (
    <TextLayer runs={runs} pageHeightPt={800} zoom={1} page={0} />
  ));
  const spans = container.querySelectorAll('.text-layer span');
  fireEvent.mouseDown(spans[3]!, { button: 0, detail: 3 });
  // The DOM range runs from the first run of the paragraph to the end of
  // its last (happy-dom's containsNode is too loose to read it back through
  // selectedCopyRuns).
  const range = document.getSelection()!.getRangeAt(0);
  expect(range.startContainer).toBe(spans[2]!.firstChild);
  expect(range.startOffset).toBe(0);
  expect(range.endContainer).toBe(spans[3]!.firstChild);
  expect(range.endOffset).toBe('second paragraph two'.length);
  expect(document.getSelection()!.toString()).toBe('second paragraph onesecond paragraph two');
});

it('leaves single and double clicks to the browser', () => {
  const { container } = render(() => (
    <TextLayer runs={runs} pageHeightPt={800} zoom={1} page={0} />
  ));
  const span = container.querySelector('.text-layer span')!;
  fireEvent.mouseDown(span, { button: 0, detail: 2 });
  expect(document.getSelection()!.isCollapsed).toBe(true);
});

// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import TextLayer from './TextLayer';
import { selectedRunRects } from '../../lib/text/copySelection';
import type { TextRunDto } from '../../types/engine';

afterEach(() => {
  cleanup();
  document.getSelection()?.removeAllRanges();
});

const run = (text: string, y: number): TextRunDto => ({ text, start: 0, x: 60, y, w: 200, h: 10 });

// page 800pt tall at zoom 1, the layer at the client origin: a run's
// client y is 800 - its page y
const runs = [
  run('first paragraph one', 700),
  run('first paragraph two', 688),
  run('second paragraph one', 650),
  run('second paragraph two', 638),
];

function mount() {
  const { container } = render(() => (
    <TextLayer runs={runs} pageWidthPt={600} pageHeightPt={800} rotation={0} zoom={1} page={0} />
  ));
  return container.querySelectorAll('.text-layer span');
}

function selected() {
  const s = document.getSelection()!;
  return {
    anchor: s.anchorNode,
    anchorOffset: s.anchorOffset,
    focus: s.focusNode,
    focusOffset: s.focusOffset,
  };
}

it('selects the paragraph under a triple-click as a drag would', () => {
  const spans = mount();
  fireEvent.mouseDown(spans[3]!, { button: 0, detail: 3, clientX: 100, clientY: 157 });
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

it('selects the word under a double-click', () => {
  const spans = mount();
  // "first paragraph one" over 200pt from x 60; "paragraph" is characters 6..15
  fireEvent.mouseDown(spans[0]!, { button: 0, detail: 2, clientX: 160, clientY: 95 });
  expect(document.getSelection()!.toString()).toBe('paragraph');
});

it('drags from a press to the pointer, across runs', () => {
  const spans = mount();
  fireEvent.mouseDown(spans[0]!, { button: 0, detail: 1, clientX: 60, clientY: 95 });
  expect(document.getSelection()!.isCollapsed).toBe(true);
  fireEvent.mouseMove(window, { clientX: 400, clientY: 157 });
  fireEvent.mouseUp(window, { clientX: 400, clientY: 157 });
  expect(selected()).toEqual({
    anchor: spans[0]!.firstChild,
    anchorOffset: 0,
    focus: spans[3]!.firstChild,
    focusOffset: 'second paragraph two'.length,
  });
});

it('keeps the focus on the nearest line when the pointer is between them', () => {
  const spans = mount();
  fireEvent.mouseDown(spans[0]!, { button: 0, detail: 1, clientX: 60, clientY: 95 });
  // the gap between the paragraphs (client y 112..140), nearer the first,
  // and right of the text: the end of that line, not of the page
  fireEvent.mouseMove(window, { clientX: 500, clientY: 118 });
  fireEvent.mouseUp(window, {});
  expect(selected().focus).toBe(spans[1]!.firstChild);
  expect(selected().focusOffset).toBe('first paragraph two'.length);
  // and a drag back above the press reverses it
  fireEvent.mouseDown(spans[1]!, { button: 0, detail: 1, clientX: 260, clientY: 107 });
  fireEvent.mouseMove(window, { clientX: 60, clientY: 10 });
  fireEvent.mouseUp(window, {});
  expect(selected().anchor).toBe(spans[1]!.firstChild);
  expect(selected().focus).toBe(spans[0]!.firstChild);
  expect(selected().focusOffset).toBe(0);
});

it('extends the selection there is on a shift-click', () => {
  const spans = mount();
  fireEvent.mouseDown(spans[0]!, { button: 0, detail: 1, clientX: 60, clientY: 95 });
  fireEvent.mouseUp(window, {});
  fireEvent.mouseDown(spans[2]!, {
    button: 0,
    detail: 1,
    shiftKey: true,
    clientX: 260,
    clientY: 145,
  });
  fireEvent.mouseUp(window, {});
  expect(selected().anchor).toBe(spans[0]!.firstChild);
  expect(selected().focus).toBe(spans[2]!.firstChild);
});

it('treats a press that barely moves as a click', () => {
  const spans = mount();
  fireEvent.mouseDown(spans[0]!, { button: 0, detail: 1, clientX: 100, clientY: 95 });
  fireEvent.mouseMove(window, { clientX: 101, clientY: 96 });
  fireEvent.mouseUp(window, {});
  expect(document.getSelection()!.isCollapsed).toBe(true);
});

it('paints a selection as one band per line', () => {
  const spans = mount();
  fireEvent.mouseDown(spans[0]!, { button: 0, detail: 1, clientX: 60, clientY: 95 });
  fireEvent.mouseMove(window, { clientX: 400, clientY: 157 });
  fireEvent.mouseUp(window, {});
  document.dispatchEvent(new Event('selectionchange'));
  const bands = document.querySelectorAll('.text-selection div');
  expect(bands).toHaveLength(4);
  expect((bands[0] as HTMLElement).style.width).toBe('200px');
});

describe('display equations', () => {
  // prose, a display (two rows numbered (3)), prose; page 800pt tall, so a
  // run's client y is 800 - its page y
  const eqRuns: TextRunDto[] = [
    { text: 'we have', start: 0, x: 60, y: 700, w: 200, h: 10 },
    { text: 'a + b', start: 0, x: 140, y: 680, w: 30, h: 10 },
    { text: 'c', start: 0, x: 152, y: 668, w: 6, h: 10 },
    { text: '(3)', start: 0, x: 480, y: 674, w: 14, h: 10 },
    { text: 'and so on', start: 0, x: 60, y: 650, w: 200, h: 10 },
  ];
  // the engine's box, from the page's top-left
  const equations = [{ bbox: [138, 109, 34, 24] as [number, number, number, number], number: '3' }];

  function mountEq() {
    const { container } = render(() => (
      <TextLayer
        runs={eqRuns}
        pageWidthPt={600}
        pageHeightPt={800}
        rotation={0}
        zoom={1}
        page={0}
        equations={equations}
      />
    ));
    return container.querySelectorAll('.text-layer span');
  }

  it('takes a display whole when a drag ends in it', () => {
    const spans = mountEq();
    fireEvent.mouseDown(spans[0]!, { button: 0, detail: 1, clientX: 60, clientY: 95 });
    // onto the numerator
    fireEvent.mouseMove(window, { clientX: 150, clientY: 115 });
    fireEvent.mouseUp(window, {});
    expect(selected().focus).toBe(spans[3]!.firstChild);
    expect(selected().focusOffset).toBe(3);
  });

  it('selects a display by a drag inside it', () => {
    const spans = mountEq();
    fireEvent.mouseDown(spans[1]!, { button: 0, detail: 1, clientX: 150, clientY: 115 });
    fireEvent.mouseMove(window, { clientX: 155, clientY: 127 });
    fireEvent.mouseUp(window, {});
    expect(selected()).toEqual({
      anchor: spans[1]!.firstChild,
      anchorOffset: 0,
      focus: spans[3]!.firstChild,
      focusOffset: 3,
    });
    document.dispatchEvent(new Event('selectionchange'));
    expect(document.querySelectorAll('.text-selection div')).toHaveLength(1);
    // and where it is, for a comment's anchor, is the display's one box
    // (happy-dom's containsNode also counts the prose either side)
    const rects = selectedRunRects(document.getSelection()!, document.body);
    expect(rects.filter((r) => r.rect[1] < 700 && r.rect[1] > 650)).toEqual([
      { page: 0, rect: [138, 667, 356, 24] },
    ]);
  });

  it('places a caret on a click inside a display', () => {
    const spans = mountEq();
    fireEvent.mouseDown(spans[1]!, { button: 0, detail: 1, clientX: 150, clientY: 115 });
    fireEvent.mouseUp(window, {});
    expect(document.getSelection()!.isCollapsed).toBe(true);
  });
});

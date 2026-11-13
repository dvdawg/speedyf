// @vitest-environment happy-dom
/**
 * The page side of text hover previews: nothing drawn, the pointer tracked
 * over the page, a preview on hover through the shared popover machine, and
 * a jump on a plain click - but never during a drag, a selection, or with an
 * annotation tool out.
 */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { HoverSpan } from './hoverTypes';
import type * as hoverStoreModule from './hoverStore';

let spans: HoverSpan[] = [];
vi.mock('./hoverStore', async (importOriginal) => {
  const actual = await importOriginal<typeof hoverStoreModule>();
  return {
    ...actual,
    spansFor: () => Promise.resolve(spans),
    termsVersion: () => 0,
  };
});

import HoverSpanLayer from './HoverSpanLayer';
import { TabContext } from '../../app/TabContext';
import type { TabRecord } from '../../stores/tabsStore';
import { setActiveTool } from '../annotations/toolStore';

const figure: HoverSpan = {
  kind: 'figure',
  // "Fig. 3" broken across two lines: two rectangles, one span.
  rects: [
    [100, 600, 20, 10],
    [72, 588, 10, 10],
  ],
  label: 'Figure 3',
  target: { page: 4, x: 72, y: 500, region: [72, 300, 300, 200], continues: false, terms: [] },
};

// happy-dom lays nothing out: boxes come from the inline style the layer
// sets, which is exactly where the browser would put them.
const realRect = HTMLElement.prototype.getBoundingClientRect;
beforeEach(() => {
  HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
    const left = parseFloat(this.style.left) || 0;
    const top = parseFloat(this.style.top) || 0;
    const width = parseFloat(this.style.width) || 0;
    const height = parseFloat(this.style.height) || 0;
    return {
      left,
      top,
      right: left + width,
      bottom: top + height,
      width,
      height,
      x: left,
      y: top,
      toJSON: () => ({}),
    } as DOMRect;
  };
  spans = [figure];
  setActiveTool('select');
});
afterEach(() => {
  HTMLElement.prototype.getBoundingClientRect = realRect;
  cleanup();
  window.getSelection()?.removeAllRanges();
});

function mount() {
  const citationStore = {
    enter: vi.fn(),
    leave: vi.fn(),
    close: vi.fn(),
    navigateInternalTarget: vi.fn(),
  };
  const tab = { id: 'tab-1', citationStore } as unknown as TabRecord;
  const view = render(() => (
    <TabContext.Provider value={tab}>
      <div class="page-space">
        <HoverSpanLayer docId={1} src={0} pageHeightPt={792} zoom={1} />
      </div>
    </TabContext.Provider>
  ));
  const space = view.container.querySelector('.page-space') as HTMLElement;
  return { view, space, citationStore };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 260));

/** Client coordinates of a point inside the span's first rectangle:
 * y-up page space flipped to the layer's top-down CSS. */
const ON_SPAN = { clientX: 110, clientY: 792 - 605 };
const OFF_SPAN = { clientX: 400, clientY: 50 };

function pointer(
  target: Element,
  type: string,
  at: { clientX: number; clientY: number },
  extra = {}
) {
  target.dispatchEvent(
    new PointerEvent(type, { bubbles: true, button: 0, buttons: 0, ...at, ...extra })
  );
}

describe('HoverSpanLayer', () => {
  it('draws nothing visible: one labelled, focusable button per span', async () => {
    const { view } = mount();
    await settle();
    const buttons = view.container.querySelectorAll('.hover-span-target');
    expect(buttons).toHaveLength(2);
    expect(buttons[0]!.getAttribute('aria-label')).toBe('Preview Figure 3');
    expect(buttons[0]!.getAttribute('tabindex')).toBe('0');
    expect(buttons[1]!.getAttribute('tabindex')).toBe('-1');
    expect(buttons[1]!.getAttribute('aria-hidden')).toBe('true');
  });

  it('previews on hover and turns the cursor into a pointer', async () => {
    const { space, citationStore } = mount();
    await settle();
    pointer(space, 'pointermove', ON_SPAN);
    expect(citationStore.enter).toHaveBeenCalledTimes(1);
    expect(citationStore.enter.mock.calls[0]![2]).toEqual({ kind: 'span', span: figure });
    expect(space.classList.contains('is-over-hover-span')).toBe(true);
    pointer(space, 'pointermove', OFF_SPAN);
    expect(citationStore.leave).toHaveBeenCalledTimes(1);
    expect(space.classList.contains('is-over-hover-span')).toBe(false);
  });

  it('every rectangle of a broken-up span previews the same span', async () => {
    const { space, citationStore } = mount();
    await settle();
    pointer(space, 'pointermove', { clientX: 75, clientY: 792 - 592 });
    expect(citationStore.enter).toHaveBeenCalledTimes(1);
  });

  it('stays quiet during a drag, and with an annotation tool out', async () => {
    const { space, citationStore } = mount();
    await settle();
    pointer(space, 'pointermove', ON_SPAN, { buttons: 1 });
    expect(citationStore.enter).not.toHaveBeenCalled();
    setActiveTool('highlight');
    pointer(space, 'pointermove', ON_SPAN);
    expect(citationStore.enter).not.toHaveBeenCalled();
  });

  it('a plain click follows the reference; a drag or a selection does not', async () => {
    const { space, citationStore } = mount();
    await settle();
    pointer(space, 'pointerdown', ON_SPAN);
    space.dispatchEvent(new MouseEvent('click', { bubbles: true, button: 0, ...ON_SPAN }));
    expect(citationStore.navigateInternalTarget).toHaveBeenCalledWith({
      kind: 'internal',
      page: 4,
      x: 72,
      y: 500,
    });

    citationStore.navigateInternalTarget.mockClear();
    pointer(space, 'pointerdown', ON_SPAN);
    space.dispatchEvent(
      new MouseEvent('click', { bubbles: true, button: 0, clientX: 130, clientY: 300 })
    );
    expect(citationStore.navigateInternalTarget).not.toHaveBeenCalled();

    pointer(space, 'pointerdown', ON_SPAN);
    space.dispatchEvent(
      new MouseEvent('click', { bubbles: true, button: 0, shiftKey: true, ...ON_SPAN })
    );
    expect(citationStore.navigateInternalTarget).not.toHaveBeenCalled();
  });

  it('defers to a link annotation under the pointer', async () => {
    const { space, citationStore } = mount();
    await settle();
    const link = document.createElement('button');
    link.className = 'citation-hotspot';
    space.appendChild(link);
    pointer(link, 'pointermove', ON_SPAN);
    expect(citationStore.enter).not.toHaveBeenCalled();
  });

  it('keyboard focus previews at once', async () => {
    const { view, citationStore } = mount();
    await settle();
    const first = view.container.querySelector('.hover-span-target') as HTMLButtonElement;
    first.focus();
    expect(citationStore.enter).toHaveBeenCalledTimes(1);
    expect(citationStore.enter.mock.calls[0]![4]).toBe(true);
  });
});

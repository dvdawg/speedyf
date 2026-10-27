// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import TabStrip from './TabStrip';
import { activateTab, detachTab, reorderTab } from '../document/tabsController';
import { addTab, createTab, removeTab, setActiveId, tabsStore } from '../../stores/tabsStore';

vi.mock('../document/tabsController', () => ({
  activateTab: vi.fn(),
  closeTab: vi.fn(),
  goHome: vi.fn(),
  reorderTab: vi.fn(),
  detachTab: vi.fn(),
}));
vi.mock('../windows/windowController', () => ({ newWindow: vi.fn() }));
vi.mock('./TabPreview', () => ({ default: () => null }));

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(HTMLElement.prototype, 'scrollIntoView').mockImplementation(() => {});
  HTMLElement.prototype.setPointerCapture = vi.fn();
  HTMLElement.prototype.hasPointerCapture = () => false;
  HTMLElement.prototype.releasePointerCapture = vi.fn();
});
afterEach(() => {
  cleanup();
  for (const tab of [...tabsStore.state.tabs]) {
    tab.citationStore.dispose();
    removeTab(tab.id);
  }
  setActiveId(null);
  vi.restoreAllMocks();
});

function setup() {
  for (let i = 0; i < 3; i++) {
    const tab = createTab();
    tab.documentStore.initFromMeta({
      docId: i + 1,
      path: `/paper-${i}.pdf`,
      name: `Paper ${i}`,
      pageCount: 1,
      sizes: [[600, 800, 0, 0, 0]],
      estimatedSize: [600, 800],
    });
    addTab(tab);
  }
  setActiveId(tabsStore.state.tabs[0]!.id);
  const view = render(() => <TabStrip />);
  const tabs = view.getAllByRole('tab');
  const rect = (left: number, right: number) => ({
    left,
    right,
    top: 0,
    bottom: 40,
    width: right - left,
    height: 40,
    x: left,
    y: 0,
    toJSON: () => ({}),
  });
  vi.spyOn(view.getByRole('tablist'), 'getBoundingClientRect').mockReturnValue(rect(0, 800));
  vi.spyOn(
    view.container.querySelector('.tab-strip-tabs')!,
    'getBoundingClientRect'
  ).mockReturnValue(rect(40, 700));
  tabs.forEach((tab, i) =>
    vi.spyOn(tab, 'getBoundingClientRect').mockReturnValue(rect(40 + i * 200, 240 + i * 200))
  );
  return { view, tabs };
}

it('activates a click without detaching', () => {
  const { tabs } = setup();
  fireEvent.pointerDown(tabs[0]!, { pointerId: 1, button: 0, clientX: 100, clientY: 20 });
  fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 102, clientY: 21 });
  fireEvent.pointerUp(tabs[0]!, { pointerId: 1, clientX: 102, clientY: 21 });
  fireEvent.click(tabs[0]!);
  expect(activateTab).toHaveBeenCalled();
  expect(detachTab).not.toHaveBeenCalled();
  expect(document.querySelector('.tab-drag-preview')).toBeNull();
  expect(document.documentElement.classList.contains('is-dragging-tab')).toBe(false);
});

it('blocks browser selection from pointer-down until release, including over PDF text', () => {
  const { tabs } = setup();
  const pdfText = document.createElement('div');
  pdfText.className = 'text-layer';
  pdfText.textContent = 'Selectable PDF content';
  document.body.append(pdfText);
  const selection = window.getSelection()!;
  const range = document.createRange();
  range.selectNodeContents(pdfText);
  selection.addRange(range);
  try {
    const down = new PointerEvent('pointerdown', {
      bubbles: true,
      cancelable: true,
      pointerId: 1,
      button: 0,
      clientX: 100,
      clientY: 20,
    });
    tabs[0]!.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    expect(document.documentElement.classList.contains('is-dragging-tab')).toBe(true);
    expect(
      pdfText.dispatchEvent(new Event('selectstart', { bubbles: true, cancelable: true }))
    ).toBe(false);
    expect(pdfText.dispatchEvent(new Event('dragstart', { bubbles: true, cancelable: true }))).toBe(
      false
    );
    fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 300, clientY: 200 });
    expect(selection.rangeCount).toBe(0);
    fireEvent.pointerUp(tabs[0]!, { pointerId: 1, clientX: 300, clientY: 200 });
    expect(document.documentElement.classList.contains('is-dragging-tab')).toBe(false);
    expect(
      pdfText.dispatchEvent(new Event('selectstart', { bubbles: true, cancelable: true }))
    ).toBe(true);
    expect(pdfText.dispatchEvent(new Event('dragstart', { bubbles: true, cancelable: true }))).toBe(
      true
    );
  } finally {
    selection.removeAllRanges();
    pdfText.remove();
  }
});

it('shows the actual tab under the grab point and follows movement beyond the strip', () => {
  const { tabs } = setup();
  tabsStore.state.tabs[0]!.documentStore.apply({ type: 'setForm', field: 'name', value: 'Draft' });
  fireEvent.pointerDown(tabs[0]!, { pointerId: 1, button: 0, clientX: 100, clientY: 20 });
  fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 110, clientY: 20 });
  const ghost = document.querySelector<HTMLElement>('.tab-drag-preview')!;
  expect(ghost).not.toBeNull();
  expect(ghost.getAttribute('aria-hidden')).toBe('true');
  expect(ghost.style.left).toBe('50px');
  expect(ghost.style.top).toBe('0px');
  expect(ghost.querySelector('.tab-pill-name')!.textContent).toBe('Paper 0');
  expect(ghost.querySelector('.tab-pill.is-active')).not.toBeNull();
  expect(ghost.querySelector('.tab-pill-dirty')!.textContent).toBe('●');
  expect(ghost.querySelector<HTMLElement>('.tab-pill')!.style.width).toBe('200px');
  expect(ghost.querySelector('.tab-detach-hint')).toBeNull();
  fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 300, clientY: 200 });
  expect(ghost.style.left).toBe('240px');
  expect(ghost.style.top).toBe('180px');
  expect(ghost.querySelector('.tab-detach-hint')).not.toBeNull();
  fireEvent.pointerUp(tabs[0]!, { pointerId: 1, clientX: 300, clientY: 200 });
  expect(document.querySelector('.tab-drag-preview')).toBeNull();
});

it.each(['escape', 'pointercancel', 'lostpointercapture', 'blur', 'unmount'])(
  'removes the preview and restores selection after %s',
  (reason) => {
    const { view, tabs } = setup();
    fireEvent.pointerDown(tabs[0]!, { pointerId: 1, button: 0, clientX: 100, clientY: 20 });
    fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 300, clientY: 200 });
    expect(document.querySelector('.tab-drag-preview')).not.toBeNull();
    if (reason === 'escape') fireEvent.keyDown(document, { key: 'Escape' });
    else if (reason === 'blur') fireEvent.blur(window);
    else if (reason === 'unmount') view.unmount();
    else tabs[0]!.dispatchEvent(new PointerEvent(reason, { bubbles: true, pointerId: 1 }));
    expect(document.querySelector('.tab-drag-preview')).toBeNull();
    expect(document.documentElement.classList.contains('is-dragging-tab')).toBe(false);
    expect(document.dispatchEvent(new Event('selectstart', { cancelable: true }))).toBe(true);
    expect(detachTab).not.toHaveBeenCalled();
  }
);

it('reorders a tab at the end of the strip', () => {
  const { tabs } = setup();
  fireEvent.pointerDown(tabs[0]!, { pointerId: 1, button: 0, clientX: 100, clientY: 20 });
  fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 650, clientY: 20 });
  fireEvent.pointerUp(tabs[0]!, { pointerId: 1, clientX: 650, clientY: 20 });
  fireEvent.click(tabs[0]!);
  expect(reorderTab).toHaveBeenCalledWith(0, 2);
  expect(detachTab).not.toHaveBeenCalled();
  expect(activateTab).not.toHaveBeenCalled();
});

it('detaches outside the strip, and Escape cancels the same gesture', () => {
  const { tabs } = setup();
  const id = tabsStore.state.tabs[0]!.id;
  const down = () =>
    fireEvent.pointerDown(tabs[0]!, { pointerId: 1, button: 0, clientX: 100, clientY: 20 });
  const move = () => fireEvent.pointerMove(tabs[0]!, { pointerId: 1, clientX: 300, clientY: 200 });
  const up = () =>
    fireEvent.pointerUp(tabs[0]!, {
      pointerId: 1,
      clientX: 300,
      clientY: 200,
      screenX: 500,
      screenY: 400,
    });
  down();
  move();
  fireEvent.keyDown(document, { key: 'Escape' });
  up();
  expect(detachTab).not.toHaveBeenCalled();
  down();
  move();
  up();
  expect(detachTab).toHaveBeenCalledExactlyOnceWith(id, [380, 380]);
  expect(reorderTab).not.toHaveBeenCalled();
});

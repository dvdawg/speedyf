// @vitest-environment happy-dom
/**
 * The option rows under the toolbar (text style, Fill, Prepare form) never
 * wrap: like the toolbar they fold what does not fit into a trailing
 * overflow menu, least used first, and a control folded there still works.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import type { JSX } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import TextTools from '../features/annotations/TextTools';
import FillTools from '../features/sign/FillTools';
import PrepareTools from '../features/prepare/PrepareTools';
import { setActiveTool, toolState, updateToolStyle } from '../features/annotations/toolStore';
import { fillState, setFillMode } from '../features/sign/fillStore';
import { prepareState, setTool, stopPreparing } from '../features/prepare/prepareStore';

const tab = {
  id: 1,
  documentStore: {
    state: { loaded: true, docId: 1, selected: null, annotations: {}, pages: [] },
    apply: vi.fn(),
  },
  viewport: { state: { editingText: null } },
};

vi.mock('../stores/tabsStore', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  activeTab: () => tab,
}));
vi.mock('../features/prepare/prepareStore', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  preparing: () => true,
  ensureBase: async () => {},
  stopPreparing: vi.fn(),
}));

/** happy-dom lays nothing out: every group is 100 wide, the row as set. */
let rowWidth = 1000;
beforeEach(() => {
  rowWidth = 1000;
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (
    this: HTMLElement
  ) {
    return this.classList.contains('toolbar-row') ? rowWidth : 0;
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(
    () => ({ width: 100, height: 28, top: 0, left: 0, right: 100, bottom: 28 }) as DOMRect
  );
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  setActiveTool('select');
});

const shown = () =>
  [...document.querySelectorAll('.options-row > .tb-group')].map(
    (g) => g.getAttribute('aria-label') ?? ''
  );

/** The groups in the order they leave the row as it narrows, and the
 * widest row that shows the overflow button. */
function leavingOrder(row: () => JSX.Element, more: string) {
  const order: string[] = [];
  let all: string[] | null = null;
  let buttonFrom: number | null = null;
  for (let width = 1000; width >= 0; width -= 10) {
    rowWidth = width;
    render(row);
    const now = shown();
    all ??= now;
    if (buttonFrom === null && screen.queryByRole('button', { name: more })) buttonFrom = width;
    cleanup();
    for (const id of all) if (!now.includes(id) && !order.includes(id)) order.push(id);
  }
  return { order, buttonFrom };
}

async function openMenu(more: string) {
  const button = screen.getByRole('button', { name: more });
  button.focus();
  fireEvent.keyDown(button, { key: 'ArrowDown' });
  await Promise.resolve();
  return { button, menu: screen.getByRole('menu', { name: more }) };
}

describe('text style row', () => {
  beforeEach(() => setActiveTool('textbox'));

  it('lets the hint and caption go first, then folds opacity, font, colour, style, size', () => {
    const { order, buttonFrom } = leavingOrder(() => <TextTools />, 'More text options');
    expect(order).toEqual(['Hint', 'Text', 'Opacity', 'Font family', 'Color', 'Style', 'Size']);
    // Seven groups of 100: the hint and caption leave without a menu.
    expect(buttonFrom).toBe(490);
  });

  it('keeps folded fields usable inside the menu', async () => {
    rowWidth = 50;
    updateToolStyle('textbox', { bold: false, fontFamily: 'Helvetica' });
    render(() => <TextTools />);
    expect(shown()).toEqual([]);
    const { button, menu } = await openMenu('More text options');
    expect(menu.classList.contains('text-tool-menu')).toBe(true);

    // The first field takes focus; the arrows walk fields and rows alike.
    const font = screen.getByRole('combobox', { name: 'Text font' }) as HTMLSelectElement;
    expect(document.activeElement).toBe(font);
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('spinbutton'));
    fireEvent.keyDown(menu, { key: 'End' });
    expect(document.activeElement).toBe(screen.getByRole('slider', { name: 'Text opacity' }));

    fireEvent.change(font, { target: { value: 'Courier' } });
    expect(toolState.styles.textbox.fontFamily).toBe('Courier');
    fireEvent.input(screen.getByRole('slider', { name: 'Text opacity' }), {
      target: { value: '0.5' },
    });
    expect(toolState.styles.textbox.opacity).toBe(0.5);

    // The colour panel may take the window's focus: the menu stays for it.
    const color = screen.getByLabelText('Text color');
    fireEvent.pointerDown(color);
    fireEvent.click(color);
    window.dispatchEvent(new Event('blur'));
    expect(screen.getByRole('menu')).toBe(menu);
    fireEvent.input(color, { target: { value: '#336699' } });
    expect(toolState.styles.textbox.color).toBe('#336699');

    // A formatting row toggles and closes, handing focus back.
    fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Bold' }));
    expect(toolState.styles.textbox.bold).toBe(true);
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  it('closes on a plain window blur and on Escape', async () => {
    rowWidth = 50;
    render(() => <TextTools />);
    await openMenu('More text options');
    window.dispatchEvent(new Event('blur'));
    expect(screen.queryByRole('menu')).toBeNull();

    const { button, menu } = await openMenu('More text options');
    fireEvent.keyDown(menu, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(button);
  });
});

describe('Fill row', () => {
  beforeEach(() => {
    setActiveTool('fill');
    setFillMode('text');
  });

  it('keeps Done, folding the shapes, then the date, then the marks', () => {
    const { order, buttonFrom } = leavingOrder(() => <FillTools />, 'More fill options');
    expect(order).toEqual([
      'Hint',
      'Fill',
      'Place dots and lines',
      'Place the date',
      'Place text and marks',
    ]);
    expect(buttonFrom).toBe(390);
    rowWidth = 0;
    render(() => <FillTools />);
    expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy();
  });

  it('chooses a folded mark from the menu', async () => {
    rowWidth = 150;
    render(() => <FillTools />);
    expect(shown()).toEqual(['Done']);
    await openMenu('More fill options');
    const items = screen.getAllByRole('menuitemradio');
    expect(items.map((i) => i.getAttribute('aria-label'))).toEqual([
      'Type text',
      'Check mark',
      'Cross',
      'Dot',
      'Strike line',
      'Circle',
      'Date',
    ]);
    expect(items[0]!.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Dot' }));
    expect(fillState.mode).toBe('dot');
    expect(screen.queryByRole('menu')).toBeNull();
  });
});

describe('Prepare form row', () => {
  beforeEach(() => setTool('select'));

  it('keeps Done, folding choice fields, then signature and date, then text', () => {
    const { order, buttonFrom } = leavingOrder(() => <PrepareTools />, 'More form tools');
    expect(order).toEqual([
      'Hint',
      'Prepare form',
      'Draw choice fields',
      'Draw signature and date fields',
      'Draw text fields and checkboxes',
    ]);
    expect(buttonFrom).toBe(390);
  });

  it('leaves the mode keys to the menu while it is open', async () => {
    rowWidth = 150;
    render(() => <PrepareTools />);
    const { button, menu } = await openMenu('More form tools');
    const items = screen.getAllByRole('menuitemradio');
    expect(document.activeElement).toBe(items[0]);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);

    // Escape closes the menu only; the mode stays on.
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(stopPreparing).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(button);
    expect(menu.isConnected).toBe(false);

    await openMenu('More form tools');
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Dropdown' }));
    expect(prepareState.tool).toBe('combo');
  });
});

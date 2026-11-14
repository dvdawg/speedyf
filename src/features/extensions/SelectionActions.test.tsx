// @vitest-environment happy-dom
/** The button by a page selection: shown once the pointer is let go over a
 * selection in a project's paper, gone on collapse, scroll and Escape, and
 * never for an unfiled PDF; pressing it runs the action with the text and
 * leaves the selection where it was. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';

type Action = { id: string; localId: string; title: string; extensionId: string };
const [actions, setActions] = createSignal<Action[]>([]);
const runExtensionAction = vi.fn(async (_action: Action, _text: string) => {});
const settleSelectionNow = vi.fn();
const [tools, setTools] = createStore({ active: 'select' });

vi.mock('./extensionStore', () => ({
  selectionActions: () => actions(),
  runExtensionAction: (action: Action, text: string) => runExtensionAction(action, text),
  extensionBindings: () => [
    {
      extensionId: 'dev.speedyf.notepad',
      command: 'quote',
      chord: { key: 'e', mod: true, shift: true, alt: false },
    },
  ],
}));
vi.mock('../../stores/selectionStore', () => ({
  selectedText: () => 'Some selected text',
  settleSelectionNow: () => settleSelectionNow(),
}));
vi.mock('../annotations/toolStore', () => ({ toolState: tools }));

const { default: SelectionActions, shortLabels } = await import('./SelectionActions');

const QUOTE: Action = {
  id: 'dev.speedyf.notepad/quote',
  localId: 'quote',
  title: 'Quote to Notepad',
  extensionId: 'dev.speedyf.notepad',
};

afterEach(() => {
  cleanup();
  document.getSelection()?.removeAllRanges();
  setActions([]);
  setTools('active', 'select');
  runExtensionAction.mockClear();
  settleSelectionNow.mockClear();
});

/** A page's text layer, and the component. */
function mount() {
  render(() => (
    <>
      <div class="viewer">
        <div class="text-layer">
          <span data-testid="run">Some selected text on the page</span>
        </div>
      </div>
      <SelectionActions />
    </>
  ));
  return screen.getByTestId('run');
}

/** Drag over the first words, as a pointer would. */
async function select(run: HTMLElement) {
  fireEvent.pointerDown(run);
  const text = run.firstChild!;
  document.getSelection()!.setBaseAndExtent(text, 0, text, 18);
  fireEvent.pointerUp(run);
  await new Promise((resolve) => setTimeout(resolve, 5));
}

const button = () => screen.queryByRole('button', { name: 'Quote to Notepad' });

describe('SelectionActions', () => {
  it('shows a quiet Quote button after the pointer is let go over a selection', async () => {
    setActions([QUOTE]);
    const run = mount();
    fireEvent.pointerDown(run);
    const text = run.firstChild!;
    document.getSelection()!.setBaseAndExtent(text, 0, text, 18);
    // Not while the drag is still going.
    expect(button()).toBeNull();
    fireEvent.pointerUp(run);
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect(button()?.textContent).toBe('Quote');
    expect(button()?.title).toMatch(/^Quote to Notepad \(.+\)$/);
  });

  it('is never there for an unfiled PDF (no action offered)', async () => {
    const run = mount();
    await select(run);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('goes away on collapse, on scroll and on Escape', async () => {
    setActions([QUOTE]);
    const run = mount();

    await select(run);
    expect(button()).not.toBeNull();
    document.getSelection()!.collapseToEnd();
    document.dispatchEvent(new Event('selectionchange'));
    expect(button()).toBeNull();

    await select(run);
    expect(button()).not.toBeNull();
    document.dispatchEvent(new Event('scroll'));
    expect(button()).toBeNull();

    await select(run);
    expect(button()).not.toBeNull();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(button()).toBeNull();
  });

  it('stays hidden while a drawing tool is out', async () => {
    setActions([QUOTE]);
    setTools('active', 'highlight');
    const run = mount();
    await select(run);
    expect(button()).toBeNull();
  });

  it('runs the action with the text and keeps the selection', async () => {
    setActions([QUOTE]);
    const run = mount();
    await select(run);
    const quote = button()!;
    const down = fireEvent.pointerDown(quote);
    // Its default (taking focus, clearing the selection) is prevented.
    expect(down).toBe(false);
    fireEvent.click(quote);
    expect(settleSelectionNow).toHaveBeenCalled();
    expect(runExtensionAction).toHaveBeenCalledWith(QUOTE, 'Some selected text');
    const selection = document.getSelection()!;
    expect(selection.isCollapsed).toBe(false);
    expect(selection.toString()).toBe('Some selected text');
  });

  it('labels buttons by their first word when that tells them apart', () => {
    expect(shortLabels(['Quote to Notepad'])).toEqual(['Quote']);
    expect(shortLabels(['Quote to Notepad', 'Look up'])).toEqual(['Quote', 'Look']);
    expect(shortLabels(['Send to A', 'Send to B'])).toEqual(['Send to A', 'Send to B']);
  });
});

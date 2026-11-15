// @vitest-environment happy-dom
/** While Markup's review mode is on, the selection row offers Comment and
 * Fix before any extension's action, for any PDF; off, it does not. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';

const [on, setOn] = createSignal(false);
const beginComment = vi.fn();
const [tools] = createStore({ active: 'select' });

vi.mock('./extensionStore', () => ({
  selectionActions: () => [],
  runExtensionAction: vi.fn(),
  extensionBindings: () => [],
}));
vi.mock('../../stores/selectionStore', () => ({
  selectedText: () => 'Some selected text',
  settleSelectionNow: () => {},
}));
vi.mock('../annotations/toolStore', () => ({ toolState: tools }));
vi.mock('../markup/markupStore', () => ({
  reviewing: () => on(),
  beginComment: (mode: string) => beginComment(mode),
}));

const { default: SelectionActions } = await import('./SelectionActions');

afterEach(() => {
  cleanup();
  document.getSelection()?.removeAllRanges();
  setOn(false);
  beginComment.mockClear();
});

async function selectSome() {
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
  const run = screen.getByTestId('run');
  fireEvent.pointerDown(run);
  document.getSelection()!.setBaseAndExtent(run.firstChild!, 0, run.firstChild!, 18);
  fireEvent.pointerUp(run);
  await new Promise((resolve) => setTimeout(resolve, 5));
}

describe('Markup buttons by a selection', () => {
  it('are offered while reviewing and start a comment or a fix', async () => {
    setOn(true);
    await selectSome();
    fireEvent.click(screen.getByRole('button', { name: 'Comment' }));
    expect(beginComment).toHaveBeenCalledWith('comment');
    cleanup();
    await selectSome();
    fireEvent.click(screen.getByRole('button', { name: 'Fix' }));
    expect(beginComment).toHaveBeenCalledWith('typo');
  });

  it('are not there outside review mode', async () => {
    await selectSome();
    expect(screen.queryByRole('button', { name: 'Comment' })).toBeNull();
  });
});

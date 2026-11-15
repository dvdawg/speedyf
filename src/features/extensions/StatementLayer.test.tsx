// @vitest-environment happy-dom
/** Statement buttons appear only when an enabled extension contributes a
 * statement action, ask for the document's statements only then, show by
 * the label under the pointer, and hand the statement to the action. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import type * as PaperDocs from './paperDocs';

const [actions, setActions] = createSignal<
  { id: string; localId: string; title: string; extensionId: string }[]
>([]);
const runStatementAction = vi.fn(async (..._args: unknown[]) => {});
/** Page 7 of the file, 800pt tall: Proposition 2.2's label at y-up 580..590
 * (top-left 210..220), its statement down to y-up 500. */
const STATEMENT = {
  kind: 'proposition',
  word: 'Proposition',
  number: '2.2',
  title: 'Moments of two block overlaps',
  page: 7,
  labelRects: [[100, 580, 90, 10]],
  region: [97, 500, 400, 93],
  text: 'Proposition 2.2 (Moments of two block overlaps). Let P be.',
};
const docStatements = vi.fn(async (_docId: number) => [
  STATEMENT,
  { ...STATEMENT, kind: 'equation', page: 7, labelRects: [[400, 300, 20, 10]] },
  { ...STATEMENT, number: '9.9', page: 8 },
]);

vi.mock('./extensionStore', () => ({
  statementActions: () => actions(),
  runStatementAction: (...args: unknown[]) => runStatementAction(...args),
}));
vi.mock('./paperDocs', async () => {
  const real = await vi.importActual<typeof PaperDocs>('./paperDocs');
  return { ...real, docStatements: (docId: number) => docStatements(docId) };
});

const { default: StatementLayer } = await import('./StatementLayer');

const action = {
  id: 'dev.speedyf.notepad/statement',
  localId: 'statement',
  title: 'Send to Notepad',
  extensionId: 'dev.speedyf.notepad',
};

afterEach(() => {
  cleanup();
  setActions([]);
  docStatements.mockClear();
  runStatementAction.mockClear();
});

function mount(docId: number) {
  return render(() => (
    <div data-testid="space" style={{ position: 'relative', width: '600px', height: '800px' }}>
      <StatementLayer docId={docId} page={4} src={7} pageHeightPt={800} zoom={1} />
    </div>
  ));
}

const idle = () => new Promise((resolve) => setTimeout(resolve, 260));

describe('StatementLayer', () => {
  it('does nothing at all without a contributing extension', async () => {
    mount(1);
    await idle();
    expect(docStatements).not.toHaveBeenCalled();
    fireEvent.pointerMove(screen.getByTestId('space'), { clientX: 120, clientY: 215 });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('shows the button by a label and sends the whole statement', async () => {
    setActions([action]);
    mount(2);
    await idle();
    expect(docStatements).toHaveBeenCalledWith(2);
    const space = screen.getByTestId('space');
    // On the statement's body, away from its label: nothing.
    fireEvent.pointerMove(space, { clientX: 300, clientY: 280 });
    expect(screen.queryByRole('button')).toBeNull();
    fireEvent.pointerMove(space, { clientX: 120, clientY: 215 });
    const button = screen.getByRole('button', { name: 'Send to Notepad: Proposition 2.2' });
    fireEvent.click(button);
    expect(runStatementAction).toHaveBeenCalledTimes(1);
    const [calledAction, target] = runStatementAction.mock.calls[0] as [
      typeof action,
      { page: number; box: number[]; labelBox: number[]; statement: unknown },
    ];
    expect(calledAction).toEqual(action);
    expect(target.page).toBe(4);
    expect(target.labelBox).toEqual([100, 210, 90, 10]);
    // The extent, top-left: label to the statement's end.
    expect(target.box).toEqual([97, 207, 400, 93]);
    expect(target.statement).toEqual(STATEMENT);
    fireEvent.pointerLeave(space);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('removes the buttons when the extension is switched off', async () => {
    setActions([action]);
    mount(3);
    await idle();
    fireEvent.pointerMove(screen.getByTestId('space'), { clientX: 120, clientY: 215 });
    expect(screen.getByRole('button')).toBeTruthy();
    setActions([]);
    expect(screen.queryByRole('button')).toBeNull();
  });
});

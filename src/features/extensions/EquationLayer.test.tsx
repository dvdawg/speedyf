// @vitest-environment happy-dom
/** Equation buttons appear only when an enabled extension contributes an
 * equation action, ask the engine for the page's blocks only then, and hand
 * the block the user pointed at to the action. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';

const [actions, setActions] = createSignal<
  { id: string; localId: string; title: string; extensionId: string }[]
>([]);
const runEquationAction = vi.fn(async () => {});
const invoke = vi.fn(async (_command: string, _args?: unknown) => [
  { bbox: [100, 200, 300, 40], number: '3' },
]);

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));
vi.mock('./extensionStore', () => ({
  equationActions: () => actions(),
  runEquationAction: (...args: unknown[]) => runEquationAction(...(args as [])),
}));

const { default: EquationLayer } = await import('./EquationLayer');

const action = {
  id: 'dev.speedyf.mathlatex/copyEquation',
  localId: 'copyEquation',
  title: 'Copy LaTeX',
  extensionId: 'dev.speedyf.mathlatex',
};

afterEach(() => {
  cleanup();
  setActions([]);
  invoke.mockClear();
  runEquationAction.mockClear();
});

/** Mount the layer inside a page-space container, as PageView does. */
function mount(docId: number) {
  return render(() => (
    <div data-testid="space" style={{ position: 'relative', width: '600px', height: '800px' }}>
      <EquationLayer docId={docId} page={4} src={7} generation={0} zoom={1} />
    </div>
  ));
}

const idle = () => new Promise((resolve) => setTimeout(resolve, 260));

describe('EquationLayer', () => {
  it('does nothing at all without a contributing extension', async () => {
    mount(1);
    await idle();
    expect(invoke).not.toHaveBeenCalled();
    fireEvent.pointerMove(screen.getByTestId('space'), { clientX: 200, clientY: 220 });
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('shows the button on hover and invokes the action with the block', async () => {
    setActions([action]);
    mount(2);
    await idle();
    expect(invoke).toHaveBeenCalledWith('get_equation_blocks', { docId: 2, src: 7 });
    const space = screen.getByTestId('space');
    fireEvent.pointerMove(space, { clientX: 20, clientY: 20 });
    expect(screen.queryByRole('button')).toBeNull();
    fireEvent.pointerMove(space, { clientX: 200, clientY: 220 });
    const button = screen.getByRole('button', { name: 'Copy LaTeX for equation 3' });
    fireEvent.click(button);
    expect(runEquationAction).toHaveBeenCalledTimes(1);
    const [calledAction, target] = runEquationAction.mock.calls[0] as unknown as [
      typeof action,
      unknown,
    ];
    expect(calledAction).toEqual(action);
    expect(target).toEqual({ page: 4, bbox: [100, 200, 300, 40], number: '3' });
    fireEvent.pointerLeave(space);
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('removes the buttons when the extension is switched off', async () => {
    setActions([action]);
    mount(3);
    await idle();
    fireEvent.pointerMove(screen.getByTestId('space'), { clientX: 200, clientY: 220 });
    expect(screen.getByRole('button')).toBeTruthy();
    setActions([]);
    expect(screen.queryByRole('button')).toBeNull();
  });
});

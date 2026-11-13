// @vitest-environment happy-dom
/**
 * The left column's fixed rows carry the library counts, and a project is
 * made from an inline field rather than a dialog: Enter creates, Escape
 * walks away without creating anything.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));

const { default: ProjectsColumn } = await import('./ProjectsColumn');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { requestNewProject, newProjectRequested } = await import('./screenIntents');

beforeEach(async () => {
  projectsStore.setScope({ kind: 'all' });
  await projectsStore.initialize();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('ProjectsColumn', () => {
  it('lists the fixed scopes with their counts, then the projects in order', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    const names = screen.getAllByRole('option').map((row) => row.textContent);
    expect(names).toEqual([
      'All items3',
      'Not filed1',
      'Other PDFs1',
      'Noneq stat mech2',
      'Active matter1',
    ]);
    expect(
      screen.getByText('All items').closest('[role="option"]')!.getAttribute('aria-selected')
    ).toBe('true');
  });

  /** Projects are reordered with the pointer, not HTML5 drag and drop (see
   * pointerDrag.ts). The row under the pointer is whatever the stubbed
   * hit-test says; its rectangle decides before or after. */
  describe('reordering by drag', () => {
    const row = (name: string) => screen.getByText(name).closest('[role="option"]') as HTMLElement;
    const rect = (top: number, height: number) =>
      ({
        top,
        height,
        bottom: top + height,
        left: 0,
        right: 200,
        width: 200,
        x: 0,
        y: top,
      }) as DOMRect;
    let under: ReturnType<typeof vi.spyOn>;
    beforeEach(() => {
      under = vi.spyOn(document, 'elementFromPoint').mockReturnValue(null);
    });
    afterEach(async () => {
      // The click swallowed after a release is let go on a timer; let it run
      // before the next test clicks anything.
      await new Promise((resolve) => setTimeout(resolve, 0));
      under.mockRestore();
    });

    it('drops a project into the top half of another, before it', async () => {
      render(() => <ProjectsColumn onMoveFocus={() => {}} />);
      await screen.findByText('Noneq stat mech');
      const noneq = row('Noneq stat mech');
      const active = row('Active matter');
      noneq.getBoundingClientRect = () => rect(100, 28);
      under.mockReturnValue(noneq);

      fireEvent.pointerDown(active, { clientX: 20, clientY: 140, button: 0 });
      fireEvent.pointerMove(active, { clientX: 20, clientY: 105 });
      expect(document.querySelector('.drag-ghost')?.textContent).toBe('Active matter');
      expect(noneq.className).toContain('is-drop-before');
      expect(noneq.className).not.toContain('is-drop-after');
      expect(document.body.classList.contains('is-dragging')).toBe(true);

      fireEvent.pointerUp(active, { clientX: 20, clientY: 105 });
      expect(engine.libraryReorderProjects).toHaveBeenCalledWith(['p2', 'p1']);
      expect(document.querySelector('.drag-ghost')).toBeNull();
      expect(noneq.className).not.toContain('is-drop-before');
      expect(document.body.classList.contains('is-dragging')).toBe(false);
    });

    it('writes nothing when the drop leaves the order as it was', async () => {
      render(() => <ProjectsColumn onMoveFocus={() => {}} />);
      await screen.findByText('Noneq stat mech');
      const noneq = row('Noneq stat mech');
      const active = row('Active matter');
      noneq.getBoundingClientRect = () => rect(100, 28);
      under.mockReturnValue(noneq);
      // The bottom half of the row above: after it, which is where it is.
      fireEvent.pointerDown(active, { clientX: 20, clientY: 140, button: 0 });
      fireEvent.pointerMove(active, { clientX: 20, clientY: 125 });
      expect(noneq.className).toContain('is-drop-after');
      fireEvent.pointerUp(active, { clientX: 20, clientY: 125 });
      expect(engine.libraryReorderProjects).not.toHaveBeenCalled();
    });

    it('offers no slot on the row being dragged, and none from its menu button', async () => {
      render(() => <ProjectsColumn onMoveFocus={() => {}} />);
      await screen.findByText('Noneq stat mech');
      const active = row('Active matter');
      under.mockReturnValue(active);
      fireEvent.pointerDown(active, { clientX: 20, clientY: 140, button: 0 });
      fireEvent.pointerMove(active, { clientX: 20, clientY: 160 });
      expect(document.querySelector('.drag-ghost')).toBeTruthy();
      expect(active.className).not.toMatch(/is-drop-(before|after)/);
      fireEvent.pointerUp(active, { clientX: 20, clientY: 160 });
      expect(engine.libraryReorderProjects).not.toHaveBeenCalled();

      const menu = screen.getByLabelText('Actions for Active matter');
      fireEvent.pointerDown(menu, { clientX: 20, clientY: 140, button: 0 });
      fireEvent.pointerMove(menu, { clientX: 20, clientY: 200 });
      expect(document.querySelector('.drag-ghost')).toBeNull();
      fireEvent.pointerUp(menu, { clientX: 20, clientY: 200 });
    });
  });

  it('creates a project from the inline field on Enter and selects it', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    fireEvent.click(screen.getByText('New project'));
    const input = screen.getByLabelText('New project name') as HTMLInputElement;
    fireEvent.input(input, { target: { value: '  Thesis ' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(engine.libraryCreateProject).toHaveBeenCalledWith('Thesis');
    await vi.waitFor(() =>
      expect(projectsStore.state.scope).toEqual({ kind: 'project', id: 'p3' })
    );
    expect(screen.queryByLabelText('New project name')).toBeNull();
  });

  it("opens the same field for the palette's New Project..., before or after mounting", async () => {
    requestNewProject();
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    const input = (await screen.findByLabelText('New project name')) as HTMLInputElement;
    expect(newProjectRequested()).toBe(false);
    fireEvent.input(input, { target: { value: 'Thesis' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(engine.libraryCreateProject).toHaveBeenCalledWith('Thesis');
    await vi.waitFor(() => expect(screen.queryByLabelText('New project name')).toBeNull());

    requestNewProject();
    await screen.findByLabelText('New project name');
  });

  it('cancels the new project on Escape without creating anything', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    fireEvent.click(screen.getByText('New project'));
    const input = screen.getByLabelText('New project name');
    fireEvent.input(input, { target: { value: 'Thesis' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(engine.libraryCreateProject).not.toHaveBeenCalled();
    expect(screen.queryByLabelText('New project name')).toBeNull();
    expect(screen.getByText('New project')).toBeTruthy();
  });

  /** Most of a Downloads folder is not papers. Those files are not hidden,
   * they are one row down, and that row asks the engine for exactly them. */
  it('shows the other PDFs under their own row', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    fireEvent.click(screen.getByText('Other PDFs'));
    expect(projectsStore.state.scope).toEqual({ kind: 'others' });
    await vi.waitFor(() =>
      expect(engine.libraryPapers).toHaveBeenCalledWith({ others: true, query: '' })
    );
    await vi.waitFor(() => expect(projectsStore.state.rows.map((r) => r.id)).toEqual(['d']));
  });

  it('changes the scope from the keyboard', async () => {
    render(() => <ProjectsColumn onMoveFocus={() => {}} />);
    await screen.findByText('Noneq stat mech');
    fireEvent.keyDown(screen.getByRole('listbox'), { key: 'ArrowDown' });
    expect(projectsStore.state.scope).toEqual({ kind: 'unfiled' });
    fireEvent.keyDown(screen.getByRole('listbox'), { key: 'ArrowDown' });
    expect(projectsStore.state.scope).toEqual({ kind: 'others' });
    fireEvent.keyDown(screen.getByRole('listbox'), { key: 'ArrowDown' });
    expect(projectsStore.state.scope).toEqual({ kind: 'project', id: 'p1' });
  });
});

// @vitest-environment happy-dom
/**
 * A PDF dropped on the Projects screen is copied into the inbox and, on a
 * project row, filed there. The inbox is asked for once and kept; a copy
 * that fails is reported and does not stop the others.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
const openDialog = vi.fn(async (_opts?: unknown): Promise<string | null> => '/lib/inbox');
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialog(...(args as [])),
}));

const { addDroppedPdfs, chooseInbox, isPdf } = await import('./fileDrop');
const { projectsStore } = await import('./projectsStore');
const { engine } = await import('../../lib/transport/engine');
const { settings, updateSettings } = await import('../../stores/settings');

const NONEQ = { kind: 'project', id: 'p1', name: 'Noneq stat mech' } as const;

beforeEach(() => {
  updateSettings({ libraryInbox: null, libraryRoots: [] });
  projectsStore.clearError();
  openDialog.mockClear();
  openDialog.mockResolvedValue('/lib/inbox');
});

afterEach(() => vi.clearAllMocks());

describe('isPdf', () => {
  it('goes by the extension, whatever its case', () => {
    expect(isPdf('/d/a.pdf')).toBe(true);
    expect(isPdf('/d/B.PDF')).toBe(true);
    expect(isPdf('/d/notes.txt')).toBe(false);
    expect(isPdf('/d/pdf')).toBe(false);
  });
});

describe('chooseInbox', () => {
  it('answers from settings without asking', async () => {
    updateSettings({ libraryInbox: '/lib/chosen' });
    expect(await chooseInbox()).toBe('/lib/chosen');
    expect(openDialog).not.toHaveBeenCalled();
  });

  it('asks once, starting in the only library folder, and keeps the answer', async () => {
    updateSettings({ libraryRoots: ['/lib'] });
    expect(await chooseInbox()).toBe('/lib/inbox');
    expect(openDialog).toHaveBeenCalledTimes(1);
    expect(openDialog.mock.calls[0]![0]).toMatchObject({ directory: true, defaultPath: '/lib' });
    expect(settings.libraryInbox).toBe('/lib/inbox');
    expect(await chooseInbox()).toBe('/lib/inbox');
    expect(openDialog).toHaveBeenCalledTimes(1);
  });

  it('is null when the dialog is dismissed, and remembers nothing', async () => {
    openDialog.mockResolvedValueOnce(null);
    expect(await chooseInbox()).toBeNull();
    expect(settings.libraryInbox).toBeNull();
  });
});

describe('addDroppedPdfs', () => {
  it('copies each PDF into the inbox and files it in the project, skipping other files', async () => {
    updateSettings({ libraryInbox: '/lib/inbox' });
    await addDroppedPdfs(['/d/a.pdf', '/d/notes.txt', '/d/B.PDF'], NONEQ);
    expect(engine.libraryFileIntoInbox).toHaveBeenCalledTimes(2);
    expect(engine.libraryFileIntoInbox).toHaveBeenNthCalledWith(1, '/d/a.pdf', '/lib/inbox');
    expect(engine.libraryFileIntoInbox).toHaveBeenNthCalledWith(2, '/d/B.PDF', '/lib/inbox');
    // The fake answers every copy with paper a.
    expect(engine.librarySetMembership).toHaveBeenCalledTimes(2);
    expect(engine.librarySetMembership).toHaveBeenCalledWith('a', 'p1', true);
    expect(openDialog).not.toHaveBeenCalled();
  });

  it('copies without filing for a drop on the screen itself', async () => {
    updateSettings({ libraryInbox: '/lib/inbox' });
    await addDroppedPdfs(['/d/a.pdf'], { kind: 'screen' });
    expect(engine.libraryFileIntoInbox).toHaveBeenCalledWith('/d/a.pdf', '/lib/inbox');
    expect(engine.librarySetMembership).not.toHaveBeenCalled();
  });

  it('asks for the inbox when none is set, and copies nothing when refused', async () => {
    await addDroppedPdfs(['/d/a.pdf'], NONEQ);
    expect(openDialog).toHaveBeenCalledTimes(1);
    expect(engine.libraryFileIntoInbox).toHaveBeenCalledWith('/d/a.pdf', '/lib/inbox');

    updateSettings({ libraryInbox: null });
    openDialog.mockResolvedValueOnce(null);
    await addDroppedPdfs(['/d/b.pdf'], NONEQ);
    expect(engine.libraryFileIntoInbox).toHaveBeenCalledTimes(1);
  });

  it('asks nothing for a drop with no PDF in it', async () => {
    await addDroppedPdfs(['/d/notes.txt'], NONEQ);
    expect(openDialog).not.toHaveBeenCalled();
    expect(engine.libraryFileIntoInbox).not.toHaveBeenCalled();
  });

  it('reports a copy that fails and still files the rest', async () => {
    updateSettings({ libraryInbox: '/lib/inbox' });
    vi.mocked(engine.libraryFileIntoInbox).mockRejectedValueOnce({
      code: 'io',
      message: '/d/a.pdf is unreadable',
    });
    await addDroppedPdfs(['/d/a.pdf', '/d/b.pdf'], NONEQ);
    // Reported after the loop: the second file's membership write would
    // otherwise have wiped the bar on its way through the store.
    expect(projectsStore.state.error).toBe('/d/a.pdf is unreadable');
    expect(engine.libraryFileIntoInbox).toHaveBeenCalledTimes(2);
    expect(engine.librarySetMembership).toHaveBeenCalledTimes(1);

    vi.mocked(engine.libraryFileIntoInbox)
      .mockRejectedValueOnce({ code: 'io', message: 'a is unreadable' })
      .mockRejectedValueOnce(new Error('b is locked'));
    await addDroppedPdfs(['/d/a.pdf', '/d/b.pdf'], NONEQ);
    expect(projectsStore.state.error).toBe(
      '2 files could not be added: a is unreadable; b is locked'
    );
  });
});

// @vitest-environment happy-dom
/**
 * Linking a folder to a project: watched first when no library folder holds
 * it, then linked, which files what is there; unlinking stops new joins.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../lib/transport/engine', async () => {
  const { fakeEngine, isEngineError } = await import('./testFixtures');
  return { engine: fakeEngine(), isEngineError };
});
const openDialog = vi.fn(async (_opts?: unknown): Promise<string | null> => '/lit/FREYA_lit');
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialog(...(args as [])),
}));
const changeFolder = vi.fn(async () => {});
vi.mock('../citations/libraryStore', () => ({
  libraryStore: { changeFolder: (...args: unknown[]) => changeFolder(...(args as [])) },
}));

const { linkFolder, unlinkFolder, within } = await import('./projectFolder');
const { PROJECTS } = await import('./testFixtures');
const { engine } = await import('../../lib/transport/engine');
const { updateSettings } = await import('../../stores/settings');

const freya = { ...PROJECTS[0]!, name: 'FREYA' };

beforeEach(() => updateSettings({ libraryRoots: ['/Users/me/Downloads'] }));
afterEach(() => vi.clearAllMocks());

describe('within', () => {
  it('is the folder or inside it, never a sibling sharing its name', () => {
    expect(within('/lit/FREYA_lit', '/lit/FREYA_lit')).toBe(true);
    expect(within('/lit/FREYA_lit/a', '/lit/FREYA_lit/')).toBe(true);
    expect(within('/lit/FREYA_lit2', '/lit/FREYA_lit')).toBe(false);
  });
});

describe('linkFolder', () => {
  it('watches a folder outside the library, then links it', async () => {
    await linkFolder(freya);
    expect(changeFolder).toHaveBeenCalledWith('/lit/FREYA_lit', true);
    expect(engine.librarySetProjectFolder).toHaveBeenCalledWith('p1', '/lit/FREYA_lit');
    expect(changeFolder.mock.invocationCallOrder[0]!).toBeLessThan(
      vi.mocked(engine.librarySetProjectFolder).mock.invocationCallOrder[0]!
    );
  });

  it('links a folder the library already watches as it is', async () => {
    openDialog.mockResolvedValueOnce('/Users/me/Downloads/FREYA');
    await linkFolder(freya);
    expect(changeFolder).not.toHaveBeenCalled();
    expect(engine.librarySetProjectFolder).toHaveBeenCalledWith('p1', '/Users/me/Downloads/FREYA');
  });

  it('does nothing when the dialog is dismissed', async () => {
    openDialog.mockResolvedValueOnce(null);
    await linkFolder(freya);
    expect(engine.librarySetProjectFolder).not.toHaveBeenCalled();
  });
});

describe('unlinkFolder', () => {
  it('clears the link', async () => {
    await unlinkFolder({ ...freya, folder: '/lit/FREYA_lit' });
    expect(engine.librarySetProjectFolder).toHaveBeenCalledWith('p1', null);
  });
});

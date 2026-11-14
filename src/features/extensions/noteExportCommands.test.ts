/** Which notepad the export commands act on, and when they are offered. */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CommandCtx } from '../../app/commands';

const state = {
  projectPane: null as unknown,
  shownPane: null as unknown,
  readerPanes: [] as unknown[],
  item: null as { projectIds: string[] } | null,
  projectId: null as string | null,
};
vi.mock('./surfaces', () => ({
  projectPane: () => state.projectPane,
  shownPane: () => state.shownPane,
}));
vi.mock('./extensionStore', () => ({ readerPanes: () => state.readerPanes }));
vi.mock('./readerContext', () => ({
  readerItem: () => state.item,
  readerProjectId: () => state.projectId,
  askReaderProject: vi.fn(async () => 'p2'),
}));
const copyNoteLatex = vi.fn(async (_key: unknown) => true);
const exportNoteMarkdown = vi.fn(async (_key: unknown) => null);
vi.mock('./editor/noteExportActions', () => ({
  copyNoteLatex: (key: unknown) => copyNoteLatex(key),
  exportNoteMarkdown: (key: unknown) => exportNoteMarkdown(key),
}));

const { NOTE_EXPORT_COMMANDS, noteExportTarget } = await import('./noteExportCommands');
const { updateSettings } = await import('../../stores/settings');

const PANE = { extensionId: 'dev.speedyf.notepad', editor: { file: 'notepad.md' } };
const home: CommandCtx = { tab: null };
const reader = { tab: {} } as unknown as CommandCtx;
const command = (id: string) => NOTE_EXPORT_COMMANDS.find((c) => c.id === id)!;

beforeEach(() => {
  state.projectPane = null;
  state.shownPane = null;
  state.readerPanes = [];
  state.item = null;
  state.projectId = null;
  vi.clearAllMocks();
});

describe('notepad export commands', () => {
  it('act on the project pane on the Projects screen', () => {
    updateSettings({ launchScreen: 'projects' });
    expect(command('notepad.exportMarkdown').enabled(home)).toBe(false);
    state.projectPane = { view: PANE, projectId: 'p1' };
    expect(noteExportTarget(home)).toEqual({
      extensionId: 'dev.speedyf.notepad',
      projectId: 'p1',
      file: 'notepad.md',
    });
    command('notepad.copyLatex').run(home);
    expect(copyNoteLatex).toHaveBeenCalledWith({
      extensionId: 'dev.speedyf.notepad',
      projectId: 'p1',
      file: 'notepad.md',
    });
    updateSettings({ launchScreen: 'home' });
    expect(command('notepad.copyLatex').enabled(home)).toBe(false);
  });

  it('act on the notepad of the reader paper, only for a paper in a project', async () => {
    state.readerPanes = [PANE];
    expect(command('notepad.copyLatex').enabled(reader)).toBe(false);
    state.item = { projectIds: ['p1', 'p2'] };
    expect(command('notepad.copyLatex').enabled(reader)).toBe(true);
    // In several projects with none picked: asked, then exported.
    command('notepad.exportMarkdown').run(reader);
    await new Promise((r) => setTimeout(r, 0));
    expect(exportNoteMarkdown).toHaveBeenCalledWith({
      extensionId: 'dev.speedyf.notepad',
      projectId: 'p2',
      file: 'notepad.md',
    });
    state.projectId = 'p1';
    command('notepad.copyLatex').run(reader);
    expect(copyNoteLatex).toHaveBeenCalledWith({
      extensionId: 'dev.speedyf.notepad',
      projectId: 'p1',
      file: 'notepad.md',
    });
  });
});

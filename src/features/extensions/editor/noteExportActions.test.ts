// @vitest-environment happy-dom
/** Exporting a note: the file through the save dialog with its pictures
 * beside it, and LaTeX onto the clipboard inside the gesture. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { applyOp } from './ops';
import type { NoteKey, NoteTransport } from './noteSession';

const invoke = vi.fn(async (_cmd: string, _args?: Record<string, unknown>): Promise<unknown> => ({
  path: '/out/My Thesis.md',
  assets: 1,
  missing: [],
}));
const save = vi.fn(async (_opts: unknown): Promise<string | null> => '/out/My Thesis.md');
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {}, emit: async () => {} }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: (opts: unknown) => save(opts) }));
vi.mock('../../../lib/transport/engine', () => ({
  engine: { libraryProjects: async () => [{ id: 'p1', name: 'My: Thesis' }] },
}));

const host = await import('../editorHost');
const { copyNoteLatex, exportNoteMarkdown, exportSummary, fileNameOf } =
  await import('./noteExportActions');

const KEY: NoteKey = { extensionId: 'dev.speedyf.notepad', projectId: 'p1', file: 'notepad.md' };
const NOTE = '# Notes\n\nSee [@old2020, p. 12]{box=1,2,3,4}.\n\n![Region](assets/region-1.png)\n';

let opened = 0;
function core(text: string) {
  const transport: NoteTransport = {
    open: async () => {
      opened += 1;
      return { text, rev: 1 };
    },
    edit: async (_key, _base, op) => {
      text = applyOp(text, op);
      return 2;
    },
    close: async () => {},
  };
  return host.useNoteSessionsForTest(transport, 'test-window');
}

const writeText = vi.fn(async (_text: string) => {});
const write = vi.fn(async (_items: unknown[]) => {});

beforeEach(() => {
  opened = 0;
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText, write },
    configurable: true,
  });
  host.setCitations('dev.speedyf.notepad', 'p1', [
    {
      key: 'new2020',
      label: '[N20]',
      title: '',
      detail: '',
      pick: true,
      aliases: ['old2020'],
    },
  ]);
});

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe('export as Markdown', () => {
  it('saves under the project name, rewriting links and copying pictures', async () => {
    const sessions = core(NOTE);
    const held = sessions.acquire(KEY);
    await held.ready;
    const report = await exportNoteMarkdown(KEY);
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: 'My- Thesis.md' }));
    expect(invoke).toHaveBeenCalledWith('project_note_export', {
      extensionId: 'dev.speedyf.notepad',
      projectId: 'p1',
      path: '/out/My Thesis.md',
      text: '# Notes\n\nSee [@old2020, p. 12].\n\n![Region](My%20Thesis-assets/region-1.png)\n',
      folder: 'My Thesis-assets',
      assets: ['assets/region-1.png'],
    });
    expect(report?.assets).toBe(1);
    expect(held.notice()).toBe('Saved My Thesis.md with 1 picture');
  });

  it('does nothing when the dialog is cancelled', async () => {
    core(NOTE);
    save.mockResolvedValueOnce(null);
    expect(await exportNoteMarkdown(KEY)).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('says what it wrote', () => {
    expect(exportSummary({ path: '/a/b.md', assets: 0, missing: [] })).toBe('Saved b.md');
    expect(exportSummary({ path: 'C:\\a\\b.md', assets: 2, missing: ['x'] })).toBe(
      'Saved b.md with 2 pictures; 1 picture is missing'
    );
    expect(fileNameOf('  a/b:c  ')).toBe('a-b-c');
    expect(fileNameOf('???')).toBe('---');
    expect(fileNameOf('')).toBe('Notepad');
  });
});

describe('copy as LaTeX', () => {
  it('writes at once, inside the gesture, for a note open in a view', async () => {
    const sessions = core(NOTE);
    const held = sessions.acquire(KEY);
    await held.ready;
    const done = copyNoteLatex(KEY);
    // Before anything was awaited.
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText.mock.calls[0]![0]).toBe(
      '\\section{Notes}\n\nSee \\cite[p.~12]{new2020}.\n\n% figure: assets/region-1.png (Region)\n'
    );
    expect(await done).toBe(true);
    expect(held.notice()).toBe('Copied as LaTeX');
  });

  it('hands the clipboard a promise for a note no view has open', async () => {
    core(NOTE);
    class FakeItem {
      constructor(public items: Record<string, Promise<Blob>>) {}
    }
    vi.stubGlobal('ClipboardItem', FakeItem);
    const done = copyNoteLatex(Promise.resolve(KEY));
    expect(write).toHaveBeenCalledTimes(1);
    const item = (write.mock.calls[0]![0] as FakeItem[])[0]!;
    const blob = await item.items['text/plain']!;
    expect(await blob.text()).toContain('\\cite[p.~12]{new2020}');
    expect(await done).toBe(true);
    expect(opened).toBe(1);
    expect(writeText).not.toHaveBeenCalled();
  });

  it('says so when the clipboard refuses', async () => {
    const sessions = core(NOTE);
    const held = sessions.acquire(KEY);
    await held.ready;
    writeText.mockRejectedValueOnce(new Error('NotAllowedError'));
    expect(await copyNoteLatex(KEY)).toBe(false);
    expect(held.notice()).toMatch(/clipboard refused/);
  });
});

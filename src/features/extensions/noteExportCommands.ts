/** "Export Notepad as Markdown..." and "Copy Notepad as LaTeX" in the
 * palette: for the project's notepad on the Projects screen, or, in the
 * reader, for the notepad of the project the paper is in (the drawer's
 * pane, open or not).
 *
 * Which note: on the Projects screen, the project pane of the project in
 * scope; in the reader, the pane offered for the paper, in the project the
 * reader works in (asked once, as reader actions ask, when the paper is in
 * several and none is picked yet). */
import type { Command, CommandCtx } from '../../app/commands';
import { settings } from '../../stores/settings';
import type { NoteKey } from './editor/noteSession';
import { copyNoteLatex, exportNoteMarkdown } from './editor/noteExportActions';
import { readerPanes } from './extensionStore';
import { askReaderProject, readerItem, readerProjectId } from './readerContext';
import { projectPane, shownPane } from './surfaces';

/** The note the commands act on: its key, with `projectId` null while the
 * reader has still to ask which project. Null when there is none. */
export function noteExportTarget(
  ctx: CommandCtx
): (Omit<NoteKey, 'projectId'> & { projectId: string | null }) | null {
  if (ctx.tab === null) {
    if (settings.launchScreen !== 'projects') return null;
    const pane = projectPane();
    return pane
      ? {
          extensionId: pane.view.extensionId,
          projectId: pane.projectId,
          file: pane.view.editor.file,
        }
      : null;
  }
  if ((readerItem()?.projectIds.length ?? 0) === 0) return null;
  const pane = shownPane() ?? readerPanes()[0];
  if (!pane) return null;
  return { extensionId: pane.extensionId, projectId: readerProjectId(), file: pane.editor.file };
}

/** The note's key, asking which project when the reader has to. */
async function settle(
  target: NonNullable<ReturnType<typeof noteExportTarget>>
): Promise<NoteKey | null> {
  const projectId = target.projectId ?? (await askReaderProject());
  return projectId ? { ...target, projectId } : null;
}

const REASON = 'needs a project notepad: a project on the Projects screen, or a paper in a project';

export const NOTE_EXPORT_COMMANDS: Command[] = [
  {
    id: 'notepad.exportMarkdown',
    title: 'Export Notepad as Markdown...',
    group: 'File',
    keywords: ['notes', 'markdown', 'md', 'save', 'pandoc', 'project'],
    enabled: (ctx) => noteExportTarget(ctx) !== null,
    disabledReason: REASON,
    run: (ctx) => {
      const target = noteExportTarget(ctx);
      if (!target) return;
      void settle(target).then((key) => (key ? exportNoteMarkdown(key) : null));
    },
  },
  {
    id: 'notepad.copyLatex',
    title: 'Copy Notepad as LaTeX',
    group: 'File',
    keywords: ['notes', 'latex', 'tex', 'clipboard', 'cite', 'paper', 'project'],
    enabled: (ctx) => noteExportTarget(ctx) !== null,
    disabledReason: REASON,
    run: (ctx) => {
      const target = noteExportTarget(ctx);
      if (!target) return;
      // Known now, the write happens inside this key press or click.
      const projectId = target.projectId;
      void copyNoteLatex(projectId ? { ...target, projectId } : settle(target));
    },
  },
];

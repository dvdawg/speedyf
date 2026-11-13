/** Filing a PDF from outside the library: the inbox it is copied into, and
 * the drop from the desktop that asks for it.
 *
 * projects.md §6, "Filing": copy into `settings.libraryInbox`, a library
 * folder chosen once, then let the engine scan the copy and make the record;
 * for a drop on a project, add the membership. Never a move, never a delete.
 * The Paper panel's Add to library takes the same path. */
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { engine, isEngineError } from '../../lib/transport/engine';
import { settings, updateSettings } from '../../stores/settings';
import type { ProjectsDropTarget } from './dnd';
import { projectsStore } from './projectsStore';

const describe = (error: unknown) =>
  isEngineError(error) ? error.message : error instanceof Error ? error.message : String(error);

/** The library folder copies go into: the one in settings, else the one the
 * user picks now, which is then kept. With nothing chosen yet the dialog
 * starts in the only library folder when there is exactly one. Null when
 * the dialog was dismissed. */
export async function chooseInbox(): Promise<string | null> {
  const inbox = settings.libraryInbox;
  if (inbox) return inbox;
  const only = settings.libraryRoots.length === 1 ? settings.libraryRoots[0] : undefined;
  const picked = await openDialog({
    directory: true,
    multiple: false,
    title: 'Choose the library folder to copy into',
    ...(only ? { defaultPath: only } : {}),
  });
  if (typeof picked !== 'string') return null;
  updateSettings({ libraryInbox: picked });
  return picked;
}

export const isPdf = (path: string): boolean => path.toLowerCase().endsWith('.pdf');

/** Copy each dropped PDF into the inbox and, for a project, file the copy
 * there. One at a time: each copy is a scan and a change event in every
 * window. A copy that fails does not stop the rest; the failures are
 * reported together at the end, in the screen's error bar, because every
 * store write that succeeds clears that bar on its way through. */
export async function addDroppedPdfs(paths: string[], target: ProjectsDropTarget): Promise<void> {
  const pdfs = paths.filter(isPdf);
  if (pdfs.length === 0) return;
  const inbox = await chooseInbox();
  if (!inbox) return;
  const failures: string[] = [];
  for (const path of pdfs) {
    try {
      const paper = await engine.libraryFileIntoInbox(path, inbox);
      if (target.kind === 'project') await projectsStore.setMembership(paper.id, target.id, true);
    } catch (error) {
      failures.push(describe(error));
    }
  }
  if (failures.length === 1) projectsStore.reportError(failures[0]);
  else if (failures.length > 1) {
    projectsStore.reportError(
      `${failures.length} files could not be added: ${failures.join('; ')}`
    );
  }
}

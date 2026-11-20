/** A project's linked folder: the PDFs in it, subfolders included, join the
 * project as the library scan finds them, without being opened. Linking
 * makes the folder a watched library folder when no library folder holds
 * it yet, and files what is already there at once. It only ever adds; a
 * file that leaves the folder keeps its place in the project. Drops on the
 * project's row are copied into it (fileDrop.ts). */
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { engine } from '../../lib/transport/engine';
import { settings } from '../../stores/settings';
import { flashStatus } from '../../stores/statusMessage';
import type { Project } from '../../types/engine';
import { libraryStore } from '../citations/libraryStore';
import { projectsStore } from './projectsStore';

const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** Whether `path` is `root` or inside it. */
export function within(path: string, root: string): boolean {
  const trimmed = root.replace(/[\\/]+$/, '');
  return path === trimmed || path.startsWith(`${trimmed}/`) || path.startsWith(`${trimmed}\\`);
}

export async function linkFolder(project: Project): Promise<void> {
  let folder: string | null = null;
  try {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: `Link a folder to "${project.name}"`,
    });
    folder = typeof picked === 'string' ? picked : null;
  } catch (error) {
    projectsStore.reportError(error);
  }
  if (!folder) return;
  // Watched first, so what the scan finds there joins as it is read.
  if (!settings.libraryRoots.some((root) => within(folder, root))) {
    await libraryStore.changeFolder(folder, true);
  }
  try {
    const joined = await engine.librarySetProjectFolder(project.id, folder);
    flashStatus(
      joined > 0
        ? `Linked ${baseName(folder)}: ${joined} ${joined === 1 ? 'item' : 'items'} joined`
        : `Linked ${baseName(folder)}`
    );
    await projectsStore.refresh();
  } catch (error) {
    projectsStore.reportError(error);
  }
}

/** Stops new files joining. The folder stays a library folder and every
 * item stays in the project. */
export async function unlinkFolder(project: Project): Promise<void> {
  try {
    await engine.librarySetProjectFolder(project.id, null);
    await projectsStore.refresh();
  } catch (error) {
    projectsStore.reportError(error);
  }
}

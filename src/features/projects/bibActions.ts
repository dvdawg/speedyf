/** The .bib actions a project offers, from its menu in the left column and
 * from the list header, so the two surfaces cannot drift.
 *
 * Both go through the native save dialog. An export is a one-off write and a
 * synced path is a file SpeedyF will keep overwriting, but to the user each
 * is "choose a file to write", and a save dialog is the one that warns before
 * overwriting something that exists. */
import { save } from '@tauri-apps/plugin-dialog';
import type { Project } from '../../types/engine';
import { projectsStore } from './projectsStore';

async function chooseBibPath(project: Project, title: string): Promise<string | null> {
  try {
    const picked = await save({
      title,
      defaultPath: project.bibPath ?? `${project.name}.bib`,
      filters: [{ name: 'BibTeX', extensions: ['bib'] }],
    });
    return typeof picked === 'string' ? picked : null;
  } catch (error) {
    projectsStore.reportError(error);
    return null;
  }
}

export async function exportBib(project: Project): Promise<void> {
  const path = await chooseBibPath(project, `Export “${project.name}” as BibTeX`);
  if (path) await projectsStore.exportBib(project.id, path);
}

export async function syncBib(project: Project): Promise<void> {
  const path = await chooseBibPath(project, `Keep a .bib for “${project.name}”`);
  if (path) await projectsStore.setProjectBib(project.id, path);
}

export async function stopSyncingBib(project: Project): Promise<void> {
  await projectsStore.setProjectBib(project.id, null);
}

/** A path is shown by its file name; the full path goes in the title. */
export function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

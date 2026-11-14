/** The .bib actions a project offers, from its menu in the left column and
 * from the list header, so the two surfaces cannot drift.
 *
 * Both go through the native save dialog. An export is a one-off write and a
 * synced path is a file SpeedyF will keep overwriting, but to the user each
 * is "choose a file to write", and a save dialog is the one that warns before
 * overwriting something that exists. */
import { open as openDialog, save } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { chooseInbox } from './fileDrop';
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
  const path = await chooseBibPath(project, `Export "${project.name}" as BibTeX`);
  if (path) await projectsStore.exportBib(project.id, path);
}

/** Export bundle (.zip)...: the project's .bib and each item's main PDF in
 * one archive, for a co-author or a submission system. */
export async function exportBundle(project: Project): Promise<void> {
  let path: string | null = null;
  try {
    const picked = await save({
      title: `Export "${project.name}" as a bundle`,
      defaultPath: `${project.name}.zip`,
      filters: [{ name: 'Zip archive', extensions: ['zip'] }],
    });
    path = typeof picked === 'string' ? picked : null;
  } catch (error) {
    projectsStore.reportError(error);
  }
  if (path) await projectsStore.exportBundle(project.id, path);
}

/** What an import made (`library_import_bundle`). */
export interface BundleImport {
  projectId: string;
  projectName: string;
  items: number;
  files: number;
  failed: string[];
}

/** "Imported Thesis: 23 items and 2 project files; 1 PDF could not be filed". */
export function importSummary(report: BundleImport): string {
  const items = report.items === 1 ? '1 item' : `${report.items} items`;
  let text = `Imported ${report.projectName}: ${items}`;
  if (report.files > 0) {
    text += ` and ${report.files === 1 ? '1 project file' : `${report.files} project files`}`;
  }
  if (report.failed.length > 0) {
    text += `; ${report.failed.length === 1 ? '1 PDF' : `${report.failed.length} PDFs`} could not be filed`;
  }
  return text;
}

/** Import Project Bundle...: a bundle someone exported (projects.md sec. 13)
 * becomes a project here. Its PDFs are filed through the inbox, as a drop
 * would be, and its notepad comes with it. */
export async function importBundle(): Promise<void> {
  let path: string | null = null;
  try {
    const picked = await openDialog({
      title: 'Import a project bundle',
      multiple: false,
      directory: false,
      filters: [{ name: 'Zip archive', extensions: ['zip'] }],
    });
    path = typeof picked === 'string' ? picked : null;
  } catch (error) {
    projectsStore.reportError(error);
  }
  if (!path) return;
  const inbox = await chooseInbox();
  if (!inbox) return;
  try {
    const report = await invoke<BundleImport>('library_import_bundle', { path, inboxDir: inbox });
    projectsStore.showNotice(importSummary(report));
    projectsStore.setScope({ kind: 'project', id: report.projectId });
  } catch (error) {
    projectsStore.reportError(error);
  }
}

export async function syncBib(project: Project): Promise<void> {
  const path = await chooseBibPath(project, `Keep a .bib for "${project.name}"`);
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

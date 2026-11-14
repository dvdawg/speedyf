/** Opening a paper from the Projects screen: which file, then the same
 * `openInNewTabOrFocus` every other entry point uses, so a paper already open
 * in a tab (or another window) is focused rather than opened twice. */
import { engine } from '../../lib/transport/engine';
import type { Paper } from '../../types/engine';
import { openInNewTabOrFocus } from '../document/tabsController';
import { projectsStore } from './projectsStore';
import { noteOrigin } from '../extensions/readerContext';

/** The first file still on disk, else the first file: opening a missing one
 * fails with a real error, which beats silently doing nothing. */
export function pathFor(paper: Paper): string | null {
  const file = paper.files.find((f) => !f.missing) ?? paper.files[0];
  return file?.path ?? null;
}

/** The detail column's paper is used when it is the one asked for, which it
 * is for Enter and double-click; otherwise one read. */
export async function pathForPaper(id: string): Promise<string | null> {
  const selected = projectsStore.state.selected;
  if (selected?.id === id) return pathFor(selected);
  try {
    return pathFor(await engine.libraryPaper(id));
  } catch (error) {
    projectsStore.reportError(error);
    return null;
  }
}

/** Open from the Projects screen. Under a project, the paper is
 * remembered as opened from it: reader actions on it (a quote to its
 * notepad) go to that project's. */
export async function openPaper(id: string): Promise<boolean> {
  const path = await pathForPaper(id);
  if (!path) return false;
  const scope = projectsStore.state.scope;
  if (scope.kind === 'project') noteOrigin(path, scope.id);
  return openInNewTabOrFocus(path);
}

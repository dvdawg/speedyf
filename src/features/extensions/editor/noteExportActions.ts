/** Export a note: as a Markdown file through the save dialog, with its
 * pictures copied beside it, or as LaTeX onto the clipboard. Reached from
 * the editor head's Export list and from the palette.
 *
 * The clipboard is only writable inside the user's gesture (EXTENSIONS.md,
 * "Clipboard timing"). A note open in a view has its text in this window
 * already, so the LaTeX is written at once; otherwise the text is fetched
 * and the write is handed the promise of it (a `ClipboardItem`), which the
 * platform lets finish within the gesture. */
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { engine } from '../../../lib/transport/engine';
import { citationIndex, noteSessions } from '../editorHost';
import type { NoteKey } from './noteSession';
import { exportFolderName, exportMarkdown, noteToLatex } from './noteExport';

/** What `project_note_export` wrote. */
export interface NoteExportReport {
  path: string;
  assets: number;
  missing: string[];
}

/** How long a result stays in the editor's head. */
const SAID_MS = 4000;

/** The note's text when a view holds it open in this window, else null. */
function heldText(key: NoteKey): string | null {
  const session = noteSessions().get(key);
  return session && session.status() === 'ready' ? session.text() : null;
}

/** The note's text, opening it for the moment when no view has it. */
async function readText(key: NoteKey): Promise<string> {
  const held = heldText(key);
  if (held !== null) return held;
  const all = noteSessions();
  const session = all.acquire(key);
  try {
    await session.ready;
    if (session.status() !== 'ready') throw new Error(session.reason() ?? 'the note is not open');
    return session.text();
  } finally {
    all.release(session);
  }
}

/** Tell the views of the note, when there are any. */
function say(key: NoteKey, message: string, ms?: number): void {
  noteSessions().get(key)?.say(message, ms);
}

/** A project's name as a file name: no character a file system refuses. */
export function fileNameOf(name: string): string {
  const cleaned = name
    .replace(/[\\/:*?"<>|]/g, '-')
    .replace(/\s+/g, ' ')
    .trim();
  return cleaned || 'Notepad';
}

async function projectName(projectId: string): Promise<string> {
  try {
    const projects = await engine.libraryProjects();
    return projects.find((p) => p.id === projectId)?.name ?? 'Notepad';
  } catch {
    return 'Notepad';
  }
}

/** "Saved Thesis.md with 3 pictures; 1 picture is missing". */
export function exportSummary(report: NoteExportReport): string {
  const name = report.path.split(/[\\/]/).pop() ?? report.path;
  let text = `Saved ${name}`;
  if (report.assets > 0) {
    text += ` with ${report.assets === 1 ? '1 picture' : `${report.assets} pictures`}`;
  }
  if (report.missing.length > 0) {
    const n = report.missing.length;
    text += `; ${n === 1 ? '1 picture is' : `${n} pictures are`} missing`;
  }
  return text;
}

/** Export Notepad as Markdown...: choose a file (the project's name by
 * default), write the note there with its pictures in `<name>-assets/`. */
export async function exportNoteMarkdown(key: NoteKey): Promise<NoteExportReport | null> {
  try {
    const name = fileNameOf(await projectName(key.projectId));
    const picked = await save({
      title: 'Export notepad as Markdown',
      defaultPath: `${name}.md`,
      filters: [{ name: 'Markdown', extensions: ['md'] }],
    });
    if (typeof picked !== 'string') return null;
    const folder = exportFolderName(picked);
    const out = exportMarkdown(await readText(key), folder);
    const report = await invoke<NoteExportReport>('project_note_export', {
      extensionId: key.extensionId,
      projectId: key.projectId,
      path: picked,
      text: out.text,
      folder,
      assets: out.assets,
    });
    say(key, exportSummary(report), SAID_MS);
    return report;
  } catch (err) {
    say(key, `Could not export: ${err instanceof Error ? err.message : String(err)}`);
    return null;
  }
}

/** The note as LaTeX, its cite keys as the items have them now. */
export function latexOf(key: NoteKey, text: string): string {
  const index = citationIndex(key.extensionId, key.projectId);
  return noteToLatex(text, { key: (k) => index.byKey.get(k)?.key ?? k });
}

/** Copy Notepad as LaTeX. `key` is a promise when the note is still to be
 * decided (the reader asks which project); resolving null cancels. */
export function copyNoteLatex(key: NoteKey | Promise<NoteKey | null>): Promise<boolean> {
  const done = (target: NoteKey | null, ok: boolean) => {
    if (target) {
      say(
        target,
        ok
          ? 'Copied as LaTeX'
          : 'The clipboard refused the write: it is only open for a moment after a click. Try again.',
        ok ? SAID_MS : undefined
      );
    }
    return ok;
  };
  if (!(key instanceof Promise)) {
    const text = heldText(key);
    if (text !== null) {
      // Written now, inside the gesture.
      return navigator.clipboard.writeText(latexOf(key, text)).then(
        () => done(key, true),
        () => done(key, false)
      );
    }
  }
  let target: NoteKey | null = null;
  const latex = (async () => {
    target = await key;
    if (!target) throw new Error('cancelled');
    return latexOf(target, await readText(target));
  })();
  const written =
    typeof ClipboardItem !== 'undefined' && typeof navigator.clipboard?.write === 'function'
      ? navigator.clipboard.write([
          new ClipboardItem({
            'text/plain': latex.then((t) => new Blob([t], { type: 'text/plain' })),
          }),
        ])
      : latex.then((t) => navigator.clipboard.writeText(t));
  return written.then(
    () => done(target, true),
    () => {
      // Cancelled is not a failure to tell.
      return latex.then(
        () => done(target, false),
        () => false
      );
    }
  );
}

/** Undo for library actions, as the window sees it (projects.md sec. 14).
 *
 * The engine keeps the journal; this keeps its two ends in a signal for
 * the palette and Cmd+Z, and says each new step in the status bar:
 * "Removed from pac_learning. Undo". A step is new when its number is
 * higher than any seen before, so a redone step (which keeps its number)
 * and a change from a scan (which makes none) say nothing. Only the
 * window with focus says it: every window hears every change. */
import { createSignal } from 'solid-js';
import { engine, isEngineError } from '../../lib/transport/engine';
import { flashStatus } from '../../stores/statusMessage';
import type { LibraryUndoState } from '../../types/engine';

const [state, setState] = createSignal<LibraryUndoState>({ undo: null, redo: null });

/** The next undo and redo, as last read. */
export const libraryUndoState = state;

let seen = 0;
let readSeq = 0;
let installed: Promise<() => void> | null = null;

const focused = () =>
  typeof document === 'undefined' || (document.visibilityState !== 'hidden' && document.hasFocus());

const message = (error: unknown) => (isEngineError(error) ? error.message : String(error));

/** Re-read the journal's ends. With `announce`, a step newer than any
 * seen is said in the status bar, with Undo beside it. */
export async function refreshLibraryUndo(announce = true): Promise<void> {
  const mine = ++readSeq;
  let next: LibraryUndoState;
  try {
    next = await engine.libraryUndoState();
  } catch {
    return;
  }
  if (mine !== readSeq) return;
  setState(next);
  const top = next.undo;
  if (!top || top.seq <= seen) return;
  seen = top.seq;
  if (announce && focused()) {
    flashStatus(top.description, { action: { label: 'Undo', run: () => void undoLibrary() } });
  }
}

/** Follow the library's changes from now on. Idempotent; the steps made
 * before it are known but not announced. */
export function installLibraryUndo(): Promise<() => void> {
  if (!installed) {
    installed = (async () => {
      try {
        const unlisten = await engine.onLibraryChanged(() => void refreshLibraryUndo());
        await refreshLibraryUndo(false);
        return unlisten;
      } catch {
        // No library to follow (a window without the engine): no undo line.
        return () => {};
      }
    })();
  }
  return installed;
}

/** Put back the last library action and say which. */
export async function undoLibrary(): Promise<void> {
  try {
    const description = await engine.libraryUndo();
    flashStatus(`Undid: ${description}`);
  } catch (error) {
    flashStatus(message(error));
  }
  await refreshLibraryUndo(false);
}

/** Do again what the last undo put back. */
export async function redoLibrary(): Promise<void> {
  try {
    const description = await engine.libraryRedo();
    flashStatus(`Redid: ${description}`);
  } catch (error) {
    flashStatus(message(error));
  }
  await refreshLibraryUndo(false);
}

/** For tests: forget what was seen and installed. */
export function resetLibraryUndoForTests(): void {
  seen = 0;
  readSeq += 1;
  installed = null;
  setState({ undo: null, redo: null });
}

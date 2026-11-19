/**
 * Typed transport to the desktop integration in Rust
 * (`src-tauri/src/desktop.rs` and the window lifecycle in `windows.rs`):
 * the Settings window, quitting through every window's close guard, the
 * title bar's document state, the system's recent documents, and showing a
 * file in Finder or Explorer.
 *
 * Every call is best effort and never throws: none of these is a reason for
 * what the user asked for to fail, and outside a webview (the unit tests)
 * there is nothing to call.
 */
import { invoke } from '@tauri-apps/api/core';

const quietly = <T>(promise: Promise<T>, fallback: T): Promise<T> => promise.catch(() => fallback);

export const desktop = {
  /** Opens the Settings window, or raises the one already open. */
  openSettings: (): Promise<void> => quietly(invoke('open_settings'), undefined),
  /** Shows this window: it was built hidden and has now painted. */
  revealWindow: (): Promise<void> => quietly(invoke('reveal_window'), undefined),
  /** Asks every window to close, each through its own unsaved-changes guard. */
  requestQuit: (): Promise<void> => quietly(invoke('request_quit'), undefined),
  isQuitting: (): Promise<boolean> => quietly(invoke<boolean>('is_quitting'), false),
  cancelQuit: (): Promise<void> => quietly(invoke('cancel_quit'), undefined),
  /** The edited dot and proxy icon of this window's title bar (macOS). */
  setDocumentState: (edited: boolean, path: string | null): Promise<void> =>
    quietly(invoke('set_document_state', { edited, path }), undefined),
  /** The system's recent documents: the Dock menu's list on macOS. */
  noteRecentDocument: (path: string): Promise<void> =>
    quietly(invoke('note_recent_document', { path }), undefined),
  clearRecentDocuments: (): Promise<void> => quietly(invoke('clear_recent_documents'), undefined),
  /** Shows `path` selected in Finder or Explorer. Rejects with a message
   * when the file is gone, which the caller shows. */
  revealInFileManager: (path: string): Promise<void> => invoke('reveal_in_file_manager', { path }),
};

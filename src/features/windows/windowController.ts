import { createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { windowApi } from '../../lib/transport/windows';
import { showError } from '../../stores/modalStore';

export const [windowTransferPending, setWindowTransferPending] = createSignal(false);

export async function newWindow(): Promise<void> {
  try {
    await invoke('new_window');
  } catch (error) {
    showError(`Could not create a window: ${String(error)}`);
  }
}

/** Opens `path` in a new window (Cmd+Enter in quick open, Cmd- or Alt-click
 * on a file's Open). A window that already shows it is raised instead. */
export async function openInNewWindow(path: string): Promise<void> {
  try {
    await windowApi.openInNewWindow(path);
  } catch (error) {
    showError(`Could not open a new window: ${String(error)}`);
  }
}

/** The modifier that asks for a new window: Cmd on macOS, Ctrl elsewhere,
 * and Alt on both. */
export function wantsNewWindow(e: {
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}): boolean {
  return e.metaKey || e.ctrlKey || e.altKey;
}

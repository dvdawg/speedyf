import { createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { showError } from '../../stores/modalStore';

export const [windowTransferPending, setWindowTransferPending] = createSignal(false);

export async function newWindow(): Promise<void> {
  try {
    await invoke('new_window');
  } catch (error) {
    showError(`Could not create a window: ${String(error)}`);
  }
}

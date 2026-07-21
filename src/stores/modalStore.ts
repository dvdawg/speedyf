/** Tiny in-app modal orchestration (password prompt, unsaved-changes guard,
 * note editing). Native dialogs handle file pick/save; these cover flows the
 * dialog plugin has no primitive for. */
import { createStore } from 'solid-js/store';

export type UnsavedChoice = 'save' | 'discard' | 'cancel';

interface ModalState {
  kind: null | 'password' | 'unsaved' | 'error';
  message: string;
}

const [modal, setModal] = createStore<ModalState>({ kind: null, message: '' });
export { modal };

let passwordResolve: ((v: string | null) => void) | null = null;
let unsavedResolve: ((v: UnsavedChoice) => void) | null = null;

export function askPassword(message: string): Promise<string | null> {
  setModal({ kind: 'password', message });
  return new Promise((resolve) => {
    passwordResolve = resolve;
  });
}

export function resolvePassword(value: string | null) {
  setModal({ kind: null, message: '' });
  passwordResolve?.(value);
  passwordResolve = null;
}

export function askUnsaved(message: string): Promise<UnsavedChoice> {
  setModal({ kind: 'unsaved', message });
  return new Promise((resolve) => {
    unsavedResolve = resolve;
  });
}

export function resolveUnsaved(choice: UnsavedChoice) {
  setModal({ kind: null, message: '' });
  unsavedResolve?.(choice);
  unsavedResolve = null;
}

export function showError(message: string) {
  setModal({ kind: 'error', message });
}

export function dismissError() {
  if (modal.kind === 'error') setModal({ kind: null, message: '' });
}

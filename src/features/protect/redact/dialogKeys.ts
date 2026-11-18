/** Keyboard handling shared by the protect dialogs: Escape closes, Tab stays
 * inside, focus moves in on open and back out on close - as the other modals
 * do. */
import { onCleanup, onMount } from 'solid-js';

export function useDialogKeys(
  dialog: () => HTMLElement | undefined,
  close: () => void
): (e: KeyboardEvent) => void {
  onMount(() => {
    const restore = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    queueMicrotask(() =>
      dialog()?.querySelector<HTMLElement>('input:not(:disabled), button:not(:disabled)')?.focus()
    );
    onCleanup(() => {
      if (restore?.isConnected) restore.focus();
    });
  });
  return (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
      return;
    }
    const el = dialog();
    if (e.key !== 'Tab' || !el) return;
    const controls = [
      ...el.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)'),
    ];
    const first = controls[0];
    const last = controls[controls.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };
}

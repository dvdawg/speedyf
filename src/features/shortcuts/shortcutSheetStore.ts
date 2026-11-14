/** Whether the keyboard shortcut sheet is showing. Its own module, beside the
 * palette's, so the keymap and the command registry can both reach it without
 * importing the sheet itself. */
import { createSignal } from 'solid-js';

const [isOpen, setIsOpen] = createSignal(false);
export { isOpen as shortcutSheetOpen };

export function toggleShortcutSheet(): void {
  setIsOpen((open) => !open);
}

export function closeShortcutSheet(): void {
  setIsOpen(false);
}

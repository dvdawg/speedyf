/** Whether the quick-open switcher is up. Its own signal rather than a mode of
 * the command palette: the two answer different questions (what to do, what
 * to read) and keep separate lists, inputs and data. */
import { createSignal } from 'solid-js';

const [isOpen, setIsOpen] = createSignal(false);
export { isOpen as quickOpenOpen };

export function openQuickOpen(): void {
  setIsOpen(true);
}

export function closeQuickOpen(): void {
  setIsOpen(false);
}

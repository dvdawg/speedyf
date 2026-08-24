/** The text currently selected in a document, reconstructed.
 *
 * Kept here rather than read on demand because the two things that want it —
 * the command palette and, through it, extension selection actions — both run
 * at a moment when the DOM selection may already be gone. Opening the palette
 * moves focus to its input, and reading `document.getSelection()` after that
 * answers about the input, not the page.
 *
 * The text is the *smart-copied* reconstruction, not `selection.toString()`. A
 * raw PDF selection comes back as one fragment per positioned run, with broken
 * lines, ligature glyphs and words split at line ends; handing an extension
 * that instead of what a copy would produce would make every extension deal
 * with the same mess.
 */
import { createSignal } from 'solid-js';

const [selectedText, setSelectedTextSignal] = createSignal<string>('');

export { selectedText };

/** True when there is something an action could be performed on. */
export function hasSelection(): boolean {
  return selectedText().length > 0;
}

/** Settle time before a selection is reconstructed.
 *
 * `selectionchange` fires continuously while dragging, and rebuilding the text
 * walks every span in the scroller — cheap once, wasteful sixty times a second.
 * Waiting for the drag to stop costs nothing: nothing reads this until the user
 * reaches for a menu.
 */
const SETTLE_MS = 150;

let timer: ReturnType<typeof setTimeout> | undefined;

/** Called by the viewer as the selection changes. `compute` is deferred, so a
 * selection that is still being dragged is never reconstructed. */
export function noteSelectionChanged(compute: () => string | null): void {
  clearTimeout(timer);
  timer = setTimeout(() => {
    setSelectedTextSignal(compute() ?? '');
  }, SETTLE_MS);
}

/** Called when the selection is known to be gone — a collapse, or a document
 * closing — so the palette does not offer actions on stale text. */
export function clearSelection(): void {
  clearTimeout(timer);
  setSelectedTextSignal('');
}

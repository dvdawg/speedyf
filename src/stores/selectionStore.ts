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
    // `compute` reads the *live* Selection, which may have collapsed during the
    // settle window — a click lands, focus moves, the drag ends somewhere
    // empty. An empty answer means "there is nothing to read now", never
    // "what you highlighted a moment ago is void", so the last real capture
    // stands. Only a genuine new selection replaces it.
    const text = compute();
    if (text && text.length > 0) setSelectedTextSignal(text);
  }, SETTLE_MS);
}

/** Forget the selection entirely.
 *
 * Deliberately NOT called when the selection merely collapses. Moving focus —
 * opening the command palette, clicking a button — collapses it, and clearing
 * on that defeats the entire purpose of this store: the text would be gone by
 * the moment anything wanted to act on it. What is held here is "the last thing
 * you highlighted", which stays true until you highlight something else.
 *
 * Called when that stops being meaningful: a different document becomes
 * active, or the one it came from goes away.
 */
export function clearSelection(): void {
  clearTimeout(timer);
  setSelectedTextSignal('');
}

/** Pointer-event drags for the Projects screen.
 *
 * Not HTML5 drag and drop. The app keeps Tauri's native file drop on so a
 * PDF from Finder arrives with its path, and with that on, WKWebView hands
 * every drag — a row pulled toward a project included — to the native
 * handler, which answers with the "Drop PDF" overlay and never a `drop`. So
 * the screen's two drags are built from pointer events, which the native
 * handler does not see: nothing is `draggable`, and WebKit never starts a
 * drag of its own.
 *
 * One helper, so a row drag and a project drag cannot disagree on the
 * threshold, the ghost, Escape, or the click the browser fires after the
 * release. The caller says what is under a point and what to do with it;
 * this knows when a press has become a drag and when it is over.
 *
 * Limits: the pointer must stay a pointer, so there is no drag out of the
 * window and nothing is handed to the desktop; and the list under the
 * pointer does not scroll on its own near an edge. */

/** Movement before a press counts as a drag, in CSS pixels. Below it the
 * release is a click, which the row handles as it always did. */
export const DRAG_THRESHOLD = 5;

/** Controls inside a row keep their own pointer: a press on the menu button
 * opens the menu, a press in the rename field places the caret. */
const INTERACTIVE = 'button, input, select, textarea, a, [contenteditable]';

/** A press on a row is cancelled at pointerdown so WebKit never begins a
 * text selection under the drag (it would, and paint it as the pointer
 * passed over rows). Cancelling pointerdown leaves the click alone, so
 * selecting a row by clicking it still works; what it may take with it is
 * the focus the press would have given the row, so that is done by hand. */
function takePress(down: PointerEvent, source: HTMLElement | null): void {
  down.preventDefault();
  if (source?.hasAttribute('tabindex')) source.focus({ preventScroll: true });
}

export interface PointerDragOptions<T> {
  /** What the ghost under the pointer says. */
  label: string;
  /** What is under a point, or null where nothing takes the drop. */
  targetAt: (x: number, y: number) => T | null;
  /** Each move's answer, null included; the caller highlights. Called with
   * null once more when the drag ends. */
  onHover: (target: T | null) => void;
  /** The release over a target. */
  onDrop: (target: T) => void;
}

/** Swallow the click the browser fires after the release that ended a drag:
 * the row underneath would take it for a selection. The click is dispatched
 * in the same task as the pointerup, so a listener that outlives that task
 * is one nobody wanted. */
function swallowNextClick(): void {
  const swallow = (e: Event) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener('click', swallow, { capture: true, once: true });
  setTimeout(() => window.removeEventListener('click', swallow, true), 0);
}

/** Call from a row's pointerdown. Returns at once; the drag, if the press
 * becomes one, runs on window listeners until the release. */
export function beginPointerDrag<T>(down: PointerEvent, options: PointerDragOptions<T>): void {
  if (down.button !== 0) return;
  if ((down.target as Element | null)?.closest(INTERACTIVE)) return;
  const source = down.currentTarget as HTMLElement | null;
  takePress(down, source);
  const pointerId = down.pointerId;
  const startX = down.clientX;
  const startY = down.clientY;
  let dragging = false;
  let ghost: HTMLDivElement | null = null;
  let current: T | null = null;

  const setHover = (next: T | null) => {
    current = next;
    options.onHover(next);
  };

  const start = () => {
    dragging = true;
    // The class turns selection off everywhere and sets the cursor; any
    // selection that exists already would otherwise be dragged as text.
    document.body.classList.add('is-dragging');
    window.getSelection()?.removeAllRanges();
    // Capture keeps the moves coming past the edge of the row and of the
    // window; hit-testing below does not depend on it.
    try {
      source?.setPointerCapture(pointerId);
    } catch {
      // A pointer the browser no longer tracks; the moves still bubble.
    }
    ghost = document.createElement('div');
    ghost.className = 'drag-ghost';
    ghost.setAttribute('aria-hidden', 'true');
    ghost.textContent = options.label;
    document.body.appendChild(ghost);
  };

  const place = (x: number, y: number) => {
    if (ghost) ghost.style.transform = `translate(${x + 14}px, ${y + 12}px)`;
  };

  /** Listeners the press installs. The drag may never start, in which case
   * the release removes them and the click goes through. */
  const cleanup = () => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onLost);
    window.removeEventListener('blur', onLost);
    window.removeEventListener('keydown', onKey, true);
  };

  /** The end of a drag that did start, by drop, Escape or a lost pointer. */
  const finish = () => {
    cleanup();
    dragging = false;
    setHover(null);
    ghost?.remove();
    ghost = null;
    document.body.classList.remove('is-dragging');
    try {
      source?.releasePointerCapture(pointerId);
    } catch {
      // Released already, or never captured.
    }
  };

  const onMove = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    if (!dragging) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_THRESHOLD) return;
      start();
    }
    place(e.clientX, e.clientY);
    setHover(options.targetAt(e.clientX, e.clientY));
  };

  const onUp = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    if (!dragging) {
      cleanup();
      return;
    }
    const dropped = current;
    finish();
    swallowNextClick();
    if (dropped !== null) options.onDrop(dropped);
  };

  /** Escape abandons the drag with the button still down. The release that
   * follows is still a release, and its click still lands on a row. */
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || !dragging) return;
    e.preventDefault();
    e.stopPropagation();
    finish();
    const afterRelease = (up: PointerEvent) => {
      if (up.pointerId !== pointerId) return;
      window.removeEventListener('pointerup', afterRelease, true);
      swallowNextClick();
    };
    window.addEventListener('pointerup', afterRelease, true);
  };

  /** The browser took the pointer (a gesture, a lost window): no release
   * is coming our way, and no click either. */
  const onLost = () => {
    if (dragging) finish();
    else cleanup();
  };

  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onLost);
  window.addEventListener('blur', onLost);
  window.addEventListener('keydown', onKey, true);
}

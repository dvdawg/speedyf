/** Whether the user is looking at this window.
 *
 * It matters because `set_active_document` is app-global and destructive: it
 * bumps the citation epoch, tears down the library hover session, and hands
 * the single text indexer to whichever document was named last. Two windows
 * both asserting their own active tab would take turns cancelling each
 * other's work for as long as both stayed open. So only the focused window
 * speaks, and re-asserts when focus comes back to it.
 */
import { createSignal } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';

// A window that has just been created is the one in front. Corrected below
// the moment Tauri says otherwise.
const [windowFocused, setWindowFocused] = createSignal(true);
export { windowFocused };

/**
 * Tracks this window's focus, calling `onFocus` each time it comes forward.
 * Returns a cleanup. Safe to call outside a webview (the unit tests): it
 * simply never reports a change.
 */
export function trackWindowFocus(onFocus: () => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;

  try {
    const win = getCurrentWindow();
    void win
      .isFocused()
      .then((focused) => {
        setWindowFocused(focused);
        // Every window starts out assuming it is in front, because a window
        // usually is when it opens. At launch that assumption is wrong for
        // all but one of a restored set, and a background window may already
        // have claimed the engine's foreground document by the time this
        // resolves. The window that really is in front takes it back.
        if (focused) onFocus();
      })
      .catch(() => undefined);
    void win
      .onFocusChanged(({ payload: focused }) => {
        setWindowFocused(focused);
        if (focused) onFocus();
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
  } catch {
    /* no webview around us — focus stays as it started */
  }

  return () => {
    disposed = true;
    unlisten?.();
  };
}

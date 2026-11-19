/** Keeps the browser from showing through the app.
 *
 * A webview still answers some keys and gestures the way a browser tab
 * does: reloading on F5 or Cmd+R (which would drop every open document),
 * zooming the whole interface on Cmd+Plus or a pinch over the toolbar, and
 * stepping its own history on a mouse's side buttons. None of those mean
 * anything in a desktop app. Each is refused here, at the window, after the
 * app's own handlers have had the event: Cmd+Plus still zooms the PDF and a
 * pinch over the pages still zooms them (Viewer.tsx), because cancelling
 * the browser's default does not stop the event reaching them. */
import { IS_MAC } from '../lib/platform';

/** Whether a key press would reload the page or zoom the interface. Pure. */
export function isBrowserKey(
  e: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>,
  mac: boolean = IS_MAC
): boolean {
  const mod = mac ? e.metaKey : e.ctrlKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (key === 'F5' || key === 'BrowserRefresh') return true;
  if (key === 'BrowserBack' || key === 'BrowserForward') return true;
  if (mod && key === 'r') return true;
  if (mod && ['=', '+', '-', '_', '0'].includes(key)) return true;
  // Alt+Left/Right is the browser's Back/Forward off macOS (on macOS it is
  // word movement, which must stay).
  if (!mac && e.altKey && (key === 'ArrowLeft' || key === 'ArrowRight')) return true;
  return false;
}

export function installWebviewGuards(): () => void {
  const onKey = (e: KeyboardEvent) => {
    // A text field keeps Option/Alt+Arrow for its caret.
    if (isBrowserKey(e)) e.preventDefault();
  };
  // Ctrl+wheel (and a pinch, on engines that report one that way) zooms the
  // interface. Non-passive so it can be cancelled.
  const onWheel = (e: WheelEvent) => {
    if (e.ctrlKey) e.preventDefault();
  };
  // WebKit's own pinch events, which magnify the whole page.
  const onGesture = (e: Event) => e.preventDefault();
  // Mouse buttons 4 and 5 are Back and Forward to the webview.
  const onMouse = (e: MouseEvent) => {
    if (e.button === 3 || e.button === 4) e.preventDefault();
  };
  window.addEventListener('keydown', onKey);
  window.addEventListener('wheel', onWheel, { passive: false });
  window.addEventListener('gesturestart', onGesture);
  window.addEventListener('gesturechange', onGesture);
  window.addEventListener('mouseup', onMouse);
  window.addEventListener('mousedown', onMouse);
  return () => {
    window.removeEventListener('keydown', onKey);
    window.removeEventListener('wheel', onWheel);
    window.removeEventListener('gesturestart', onGesture);
    window.removeEventListener('gesturechange', onGesture);
    window.removeEventListener('mouseup', onMouse);
    window.removeEventListener('mousedown', onMouse);
  };
}

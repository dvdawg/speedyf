/** The macOS title bar folded into the tab strip.
 *
 * tauri.conf.json asks for an overlay title bar: the traffic lights sit on
 * the webview and the webview receives every click, so the empty parts of
 * the tab strip and toolbar have to move the window themselves, and a
 * double-click there has to do what the system title bar does (zoom,
 * minimize or nothing, per System Settings; titlebar.rs reads it).
 *
 * Only a press on an element marked `data-window-drag` itself counts - a
 * button, field or label inside the bar is never a handle. Windows and
 * Linux keep their native title bars and install none of this. */
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { platform } from './platform';

export const WINDOW_DRAG_ATTR = 'data-window-drag';

export function isWindowDragTarget(target: EventTarget | null): boolean {
  return (
    typeof HTMLElement !== 'undefined' &&
    target instanceof HTMLElement &&
    target.hasAttribute(WINDOW_DRAG_ATTR)
  );
}

interface TitlebarWindow {
  startDragging(): Promise<void>;
  isFullscreen(): Promise<boolean>;
  onResized(handler: () => void): Promise<() => void>;
}

/** Wire the drag regions and the full-screen flag. Answers the cleanup. */
export function installTitlebar(
  root: HTMLElement = document.documentElement,
  win: TitlebarWindow = getCurrentWindow(),
  zoom: () => Promise<unknown> = () => invoke('titlebar_double_click')
): () => void {
  // macOS zooms on the second release, and not at all if the pointer moved
  // in between - the same rule Tauri's own drag regions follow.
  let downX = 0;
  let downY = 0;
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || !isWindowDragTarget(e.target)) return;
    if (e.detail === 2) {
      downX = e.clientX;
      downY = e.clientY;
      return;
    }
    if (e.detail !== 1) return;
    e.preventDefault();
    void win.startDragging().catch(() => undefined);
  };
  const onMouseUp = (e: MouseEvent) => {
    if (
      e.button === 0 &&
      e.detail === 2 &&
      e.clientX === downX &&
      e.clientY === downY &&
      isWindowDragTarget(e.target)
    ) {
      void zoom().catch(() => undefined);
    }
  };
  document.addEventListener('mousedown', onMouseDown);
  document.addEventListener('mouseup', onMouseUp);

  // Full screen hides the traffic lights, and the strip gives their room back.
  let disposed = false;
  const sync = () =>
    void win
      .isFullscreen()
      .then((full) => {
        if (!disposed) root.toggleAttribute('data-fullscreen', full);
      })
      .catch(() => undefined);
  sync();
  let unlisten: (() => void) | undefined;
  void win
    .onResized(sync)
    .then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    })
    .catch(() => undefined);

  return () => {
    disposed = true;
    document.removeEventListener('mousedown', onMouseDown);
    document.removeEventListener('mouseup', onMouseUp);
    unlisten?.();
  };
}

/** The title bar wiring this platform needs: none outside macOS. */
export function installPlatformTitlebar(): () => void {
  return platform === 'macos' ? installTitlebar() : () => undefined;
}

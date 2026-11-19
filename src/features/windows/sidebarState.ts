/** Each window's sidebar across launches.
 *
 * A window's sidebar (open, width, panel) is saved beside its tabs, so a
 * relaunch puts every window's back as it was; the last one changed is also
 * kept on its own, for a brand-new window to start from (uiStore reads that
 * at startup). Saving is debounced: a drag of the resize handle changes the
 * width on every pointer move, and none of those needs to reach storage. */
import { createEffect, createRoot, on } from 'solid-js';
import { saveSidebar } from '../../stores/sessionStore';
import {
  applySidebar,
  saveLastUsedSidebar,
  sidebarState,
  type SavedSidebar,
  type SidebarState,
} from '../../stores/uiStore';

const SAVE_DELAY_MS = 300;

/** What a restore last put back. Saving that is bookkeeping for this window,
 * not a choice the user made, so it does not become the last-used sidebar a
 * new window copies: with several windows restored at once, whichever came
 * up last would otherwise win. */
let restored: SidebarState | null = null;
let flushPending: (() => void) | null = null;

function same(a: SidebarState, b: SidebarState | null): boolean {
  return b !== null && a.open === b.open && a.width === b.width && a.mode === b.mode;
}

/** Puts back the sidebar a window had at last quit. `windowWidth` is the
 * width the window is being given, when known, so the sidebar is clamped to
 * that rather than to the default frame it may still have. */
export function restoreSidebar(saved: SavedSidebar | undefined, windowWidth?: number): void {
  if (!saved) return;
  applySidebar(saved, windowWidth);
  restored = sidebarState();
}

/** Saves the pending change now: before the window closes. */
export function flushSidebar(): void {
  flushPending?.();
}

/** Keeps this window's saved sidebar current. Returns the stop function. */
export function trackSidebar(label: string): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const write = () => {
    clearTimeout(timer);
    timer = undefined;
    flushPending = null;
    const state = sidebarState();
    saveSidebar(state, label);
    if (!same(state, restored)) {
      saveLastUsedSidebar(state);
      restored = null;
    }
  };
  // What the window starts with came from elsewhere (the last-used sidebar,
  // or a restore): it is saved for this window, but nobody chose it here.
  restored ??= sidebarState();
  return createRoot((dispose) => {
    createEffect(
      on(
        () => {
          const s = sidebarState();
          return [s.open, s.width, s.mode];
        },
        () => {
          clearTimeout(timer);
          timer = setTimeout(write, SAVE_DELAY_MS);
          flushPending = write;
        }
      )
    );
    return () => {
      clearTimeout(timer);
      flushPending = null;
      dispose();
    };
  });
}

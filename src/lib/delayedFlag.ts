/** A flag that turns on only after its source has held true for a while, and
 * off at once: placeholders for a wait long enough to notice, never a flash
 * for one that is not. */
import { type Accessor, createEffect, createMemo, createSignal, onCleanup } from 'solid-js';

/** How long a list may load before it shows placeholder rows (ms). */
export const SKELETON_DELAY = 150;

export function createDelayedFlag(source: () => boolean, ms = SKELETON_DELAY): Accessor<boolean> {
  const want = createMemo(source);
  const [on, setOn] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    clearTimeout(timer);
    if (!want()) {
      setOn(false);
      return;
    }
    timer = setTimeout(() => setOn(true), ms);
  });
  onCleanup(() => clearTimeout(timer));
  return on;
}

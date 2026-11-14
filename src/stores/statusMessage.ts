/** A quiet line in the status bar's centre: "Copied cite key", "Removed
 * from pac_learning. Undo".
 *
 * One at a time, the newest winning. It hides by itself after a few
 * seconds, and stays while the pointer is over it, so its action can be
 * reached. Never a toast: the status bar is where passing remarks go. */
import { createSignal } from 'solid-js';

export interface StatusAction {
  label: string;
  run: () => void;
}

export interface StatusMessage {
  /** Rises with each message, so a view can tell two equal texts apart. */
  id: number;
  text: string;
  action?: StatusAction | undefined;
}

/** How long a message with an action stays: long enough to reach it. */
export const ACTION_MS = 6_000;
/** A plain confirmation says less, so it goes sooner. */
export const PLAIN_MS = 3_000;
/** Once the pointer leaves, a moment more before it goes. */
const AFTER_HOVER_MS = 1_500;

const [current, setCurrent] = createSignal<StatusMessage | null>(null);
let timer: ReturnType<typeof setTimeout> | undefined;
let seq = 0;
let held = false;

/** The message showing now, or null. */
export const statusMessage = current;

function hideAfter(ms: number): void {
  clearTimeout(timer);
  if (held) return;
  timer = setTimeout(() => setCurrent(null), ms);
}

/** Show `text` (and its action as a text button) in place of whatever
 * the line said. */
export function flashStatus(
  text: string,
  options: { action?: StatusAction; ms?: number } = {}
): void {
  seq += 1;
  setCurrent({ id: seq, text, action: options.action });
  hideAfter(options.ms ?? (options.action ? ACTION_MS : PLAIN_MS));
}

/** The pointer is over the message (true) or left it (false). */
export function holdStatus(hold: boolean): void {
  held = hold;
  if (hold) clearTimeout(timer);
  else if (current()) hideAfter(AFTER_HOVER_MS);
}

export function dismissStatus(): void {
  clearTimeout(timer);
  held = false;
  setCurrent(null);
}

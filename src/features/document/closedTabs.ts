/** Recently closed tabs, for Reopen Closed Tab.
 *
 * Per window: a module instance lives in one webview, so each window keeps
 * its own stack. Never persisted; a relaunch starts empty. Only a real close
 * pushes here. A tab detached or moved to another window is still open, just
 * elsewhere, so it never lands on this stack. */
import { createSignal } from 'solid-js';
import type { FitMode, Rotation } from '../../types/model';

export const CLOSED_TABS_LIMIT = 20;

/** A closed document and where its reader was in it. */
export interface ClosedTab {
  path: string;
  page: number;
  /** How far down the page the top of the view was, as a fraction of its
   * height (the same contract as the recents' reading position). */
  fraction: number;
  zoom: number;
  fitMode: FitMode;
  viewRotation: Rotation;
}

const stack: ClosedTab[] = [];
const [count, setCount] = createSignal(0);

/** How many closed tabs could be reopened. Reactive. */
export const closedTabCount = count;

export function pushClosedTab(entry: ClosedTab): void {
  stack.push(entry);
  if (stack.length > CLOSED_TABS_LIMIT) stack.splice(0, stack.length - CLOSED_TABS_LIMIT);
  setCount(stack.length);
}

/** The most recently closed tab, taken off the stack. */
export function popClosedTab(): ClosedTab | null {
  const entry = stack.pop() ?? null;
  setCount(stack.length);
  return entry;
}

/** For tests. */
export function clearClosedTabs(): void {
  stack.length = 0;
  setCount(0);
}

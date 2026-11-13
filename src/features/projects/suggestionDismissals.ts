/** The project suggestions the user said Not now to, this session: per item
 * and project. Shared by the item's suggestion line and the palette's Add to
 * Suggested Project, so the palette never offers what the line hid. */
import { createSignal } from 'solid-js';

const [dismissed, setDismissed] = createSignal<ReadonlySet<string>>(new Set());
const pair = (paperId: string, projectId: string) => `${paperId}:${projectId}`;

export function dismissSuggestion(paperId: string, projectId: string): void {
  setDismissed((pairs) => new Set([...pairs, pair(paperId, projectId)]));
}

/** Whether the user said Not now to this project for this item. */
export function suggestionDismissed(paperId: string, projectId: string): boolean {
  return dismissed().has(pair(paperId, projectId));
}

/** Forget every Not now; for tests. */
export function resetDismissedSuggestions(): void {
  setDismissed(new Set<string>());
}

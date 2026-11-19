/** File > Open Recent's entries, from the recent list. Pure. */
import type { RecentEntry } from '../stores/recentStore';

/** As many as a menu shows before it becomes a list to scroll. */
export const RECENT_MENU_LIMIT = 10;

export interface RecentMenuEntry {
  text: string;
  path: string;
}

function folderOf(path: string): string | null {
  const parts = path.split(/[\\/]/).filter((p) => p.length > 0);
  return parts.length >= 2 ? parts[parts.length - 2]! : null;
}

/** The most recent files by name. Two files of one name (papers arrive as
 * `main.pdf` in bulk) are told apart by their folder, as the window list
 * does. */
export function recentMenuEntries(entries: readonly RecentEntry[]): RecentMenuEntry[] {
  const shown = entries.slice(0, RECENT_MENU_LIMIT);
  const counts = new Map<string, number>();
  for (const entry of shown) counts.set(entry.name, (counts.get(entry.name) ?? 0) + 1);
  return shown.map((entry) => {
    const folder = (counts.get(entry.name) ?? 0) > 1 ? folderOf(entry.path) : null;
    return { text: folder ? `${entry.name} - ${folder}` : entry.name, path: entry.path };
  });
}

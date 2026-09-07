/** The pre-multi-window session, kept only long enough to migrate it.
 *
 * Open tabs used to be persisted here, in localStorage. They are not any
 * more: localStorage is shared by every window in the app, so two windows
 * writing this one key means the last to write wins and the other window's
 * tabs are simply gone. The session lives in the window registry in Rust now
 * (`src-tauri/src/windows.rs`), which is per-window by construction and is
 * also what the other windows read.
 *
 * This module survives so that upgrading does not forget what was open: the
 * main window reads the old key once, restores from it, and removes it. It
 * can be deleted once every install has launched at least once.
 *
 * As before, only file paths, their order, and which was active are ever
 * remembered — never docId (meaningless across restarts), zoom, or scroll.
 */

export interface LegacySession {
  tabs: { path: string }[];
  activePath: string | null;
}

const KEY = 'speedyf-session';

/** Reads the old session and removes it, so the migration happens once. */
export function takeLegacySession(): LegacySession | null {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(KEY);
    if (raw !== null) localStorage.removeItem(KEY);
  } catch {
    return null; // storage unavailable — nothing to migrate
  }
  if (raw === null) return null;

  try {
    const parsed = JSON.parse(raw) as Partial<LegacySession>;
    if (!Array.isArray(parsed.tabs)) return null;
    const tabs = parsed.tabs.filter(
      (t): t is { path: string } =>
        typeof t === 'object' && t !== null && typeof t.path === 'string'
    );
    if (tabs.length === 0) return null;
    return {
      tabs,
      activePath: typeof parsed.activePath === 'string' ? parsed.activePath : null,
    };
  } catch {
    return null; // corrupted session is worth no more than an absent one
  }
}

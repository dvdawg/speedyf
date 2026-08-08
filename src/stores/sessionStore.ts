/** Open-tabs session, persisted to localStorage so they can be restored on
 * relaunch. Only remembers file paths + order + which was active — never
 * docId (meaningless across restarts), zoom, or scroll (deferred to v1.1). */

export interface SessionFile {
  version: 1;
  tabs: { path: string }[];
  activePath: string | null;
}

const KEY = 'speedyf-session';

export function loadSession(): SessionFile {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<SessionFile>;
      if (Array.isArray(parsed.tabs)) {
        return {
          version: 1,
          tabs: parsed.tabs.filter(
            (t): t is { path: string } =>
              typeof t === 'object' && t !== null && typeof t.path === 'string'
          ),
          activePath: typeof parsed.activePath === 'string' ? parsed.activePath : null,
        };
      }
    }
  } catch {
    /* corrupted session falls back to empty */
  }
  return { version: 1, tabs: [], activePath: null };
}

export function saveSession(session: SessionFile): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(session));
  } catch {
    /* storage unavailable (private mode) — session stays unrestorable */
  }
}

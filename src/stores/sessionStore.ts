/** Open-tabs sessions, persisted to localStorage so they can be restored on
 * relaunch — one per window, each under its own key, so two windows never
 * overwrite each other. Only remembers file paths + order + which was active
 * — never docId (meaningless across restarts), zoom, or scroll (deferred to
 * v1.1). */

export interface SessionFile {
  version: 1;
  tabs: { path: string }[];
  activePath: string | null;
}

const KEY = 'speedyf-session';

function readSession(key: string): SessionFile {
  try {
    const raw = localStorage.getItem(key);
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

export function saveSession(session: SessionFile, label = 'main'): void {
  try {
    localStorage.setItem(sessionKey(label), JSON.stringify(session));
  } catch {
    /* storage unavailable (private mode) — session stays unrestorable */
  }
}

function sessionKey(label: string): string {
  return label === 'main' ? KEY : `${KEY}:${label}`;
}

/** The main window's own session from the last run. */
export function loadSession(): SessionFile {
  return readSession(KEY);
}

/** Past this a restore that has gone wrong could not spawn windows forever,
 * and the windows would be unmanageable anyway. */
export const MAX_RESTORED_WINDOWS = 11;

/** Every window that was open at last quit, for a cold launch: the main
 * window's tabs, and one entry per other window that should be reopened as a
 * window of its own.
 *
 * Each live window writes its own key, so no window ever overwrites
 * another's. The other windows' keys are removed here — the windows reopened
 * from them get new labels and write fresh keys. A file listed by two windows
 * is stale bookkeeping (one document is never open twice), so it stays with
 * the first. When the main window itself was closed while others stayed
 * open, it takes over the first of theirs rather than greeting the user with
 * nothing. */
export function takeWindowSessions(): { main: SessionFile; others: SessionFile[] } {
  let main = readSession(KEY);
  const others: SessionFile[] = [];
  try {
    const keys: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key?.startsWith(`${KEY}:`)) keys.push(key);
    }
    // Creation order — `speedyf-2` before `speedyf-10` — so windows come back
    // in the order they were opened.
    const order = (key: string) => Number(/(\d+)$/.exec(key)?.[1] ?? 0);
    keys.sort((a, b) => order(a) - order(b));
    const seen = new Set(main.tabs.map((t) => t.path));
    for (const key of keys) {
      const other = readSession(key);
      const tabs = other.tabs.filter((t) => !seen.has(t.path));
      for (const tab of tabs) seen.add(tab.path);
      if (tabs.length === 0) continue;
      const activePath = tabs.some((t) => t.path === other.activePath) ? other.activePath : null;
      others.push({ version: 1, tabs, activePath });
    }
    for (const key of keys) localStorage.removeItem(key);
  } catch {
    /* storage unavailable */
  }
  if (main.tabs.length === 0 && others.length > 0) {
    main = others.shift()!;
    saveSession(main);
  }
  return { main, others: others.slice(0, MAX_RESTORED_WINDOWS) };
}

export function removeWindowSession(label: string): void {
  try {
    localStorage.removeItem(sessionKey(label));
  } catch {
    /* storage unavailable */
  }
}

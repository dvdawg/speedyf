/** Open-tabs session, persisted to localStorage so they can be restored on
 * relaunch. Only remembers file paths + order + which was active — never
 * docId (meaningless across restarts), zoom, or scroll (deferred to v1.1). */

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

/** Each live window writes its own key. At the next cold launch, collect
 * surviving windows into the main window without racing other writers. */
export function loadSession(): SessionFile {
  const main = readSession(KEY);
  try {
    const keys: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key?.startsWith(`${KEY}:`)) keys.push(key);
    }
    for (const key of keys) {
      const other = readSession(key);
      for (const tab of other.tabs) {
        if (!main.tabs.some((t) => t.path === tab.path)) main.tabs.push(tab);
      }
      main.activePath ??= other.activePath;
    }
    saveSession(main);
    for (const key of keys) localStorage.removeItem(key);
  } catch {
    /* storage unavailable */
  }
  return main;
}

export function removeWindowSession(label: string): void {
  try {
    localStorage.removeItem(sessionKey(label));
  } catch {
    /* storage unavailable */
  }
}

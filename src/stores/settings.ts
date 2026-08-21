/** User settings persisted to localStorage. */
import { createStore } from 'solid-js/store';
import { engine } from '../lib/transport/engine';

export type ThemePref = 'system' | 'light' | 'dark';

interface Settings {
  theme: ThemePref;
  lowMemory: boolean;
  /** folders in the library; was a single optional root */
  libraryRoots: string[];
  /** Hosts the user chose not to be asked about again before following a
   * document's link out to the browser. Exact hosts only — see lib/links. */
  trustedLinkHosts: string[];
}

const KEY = 'speedyf-settings';

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Settings>;
      return {
        theme: parsed.theme === 'light' || parsed.theme === 'dark' ? parsed.theme : 'system',
        lowMemory: parsed.lowMemory === true,
        libraryRoots: Array.isArray(parsed.libraryRoots)
          ? parsed.libraryRoots.filter((root): root is string => typeof root === 'string')
          : // carry a single-root setting forward rather than making the user
            // pick their folder again
            typeof (parsed as { libraryRoot?: unknown }).libraryRoot === 'string'
            ? [(parsed as { libraryRoot: string }).libraryRoot]
            : [],
        trustedLinkHosts: Array.isArray(parsed.trustedLinkHosts)
          ? parsed.trustedLinkHosts.filter((host): host is string => typeof host === 'string')
          : [],
      };
    }
  } catch {
    /* corrupted settings fall back to defaults */
  }
  return { theme: 'system', lowMemory: false, libraryRoots: [], trustedLinkHosts: [] };
}

const [settings, setSettingsStore] = createStore<Settings>(load());

export { settings };

export function updateSettings(patch: Partial<Settings>) {
  setSettingsStore(patch);
  try {
    localStorage.setItem(
      KEY,
      JSON.stringify({
        theme: settings.theme,
        lowMemory: settings.lowMemory,
        libraryRoots: settings.libraryRoots,
        trustedLinkHosts: settings.trustedLinkHosts,
      })
    );
  } catch {
    /* storage unavailable (private mode) — settings stay session-only */
  }
}

/** Low memory is the one setting with an effect beyond the store — the engine
 * has to shrink its cache budgets too. Both surfaces that expose the toggle
 * (StatusBar, home Settings panel) go through here so they can't drift. */
export function applyLowMemory(value: boolean) {
  updateSettings({ lowMemory: value });
  void engine.setLowMemory(value);
}

/** Resolve the effective theme, honoring the OS preference in system mode. */
export function effectiveTheme(): 'light' | 'dark' {
  if (settings.theme !== 'system') return settings.theme;
  return typeof window !== 'undefined' &&
    window.matchMedia?.('(prefers-color-scheme: dark)').matches
    ? 'dark'
    : 'light';
}

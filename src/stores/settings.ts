/** User settings persisted to localStorage. */
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { engine } from '../lib/transport/engine';

export type ThemePref = 'system' | 'light' | 'dark';

interface Settings {
  theme: ThemePref;
  /** Render pages as dark paper with light ink while the dark theme is in
   * effect. Off by default: it changes how a document *looks*, which is the
   * author's call before it is ours. */
  invertPages: boolean;
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
        invertPages: parsed.invertPages === true,
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
  return {
    theme: 'system',
    invertPages: false,
    lowMemory: false,
    libraryRoots: [],
    trustedLinkHosts: [],
  };
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
        invertPages: settings.invertPages,
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

/** The OS preference, as a signal.
 *
 * It has to be reactive rather than read on demand: page rasters are inverted
 * by the *engine*, so a system appearance change while `theme` is `system` has
 * to reach the render URLs and re-request them. A plain matchMedia read would
 * leave every mounted page showing the previous appearance until something
 * else happened to invalidate it. */
const [systemDark, setSystemDark] = createSignal(
  typeof window !== 'undefined'
    ? (window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false)
    : false
);

if (typeof window !== 'undefined' && window.matchMedia) {
  const mq = window.matchMedia('(prefers-color-scheme: dark)');
  // No teardown on purpose: the store lives as long as the window does.
  mq.addEventListener('change', (e) => setSystemDark(e.matches));
}

/** Resolve the effective theme, honoring the OS preference in system mode. */
export function effectiveTheme(): 'light' | 'dark' {
  if (settings.theme !== 'system') return settings.theme;
  return systemDark() ? 'dark' : 'light';
}

/** Whether rendered pages should come back inverted right now.
 *
 * The setting is a standing preference; it only takes effect under the dark
 * theme, so one switch follows the system into and out of dark mode instead of
 * stranding dark pages on a light shell. */
export function pagesInverted(): boolean {
  return settings.invertPages && effectiveTheme() === 'dark';
}

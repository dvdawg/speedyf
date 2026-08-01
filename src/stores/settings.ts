/** User settings persisted to localStorage. */
import { createStore } from 'solid-js/store';

export type ThemePref = 'system' | 'light' | 'dark';

interface Settings {
  theme: ThemePref;
  lowMemory: boolean;
  libraryRoot: string | null;
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
        libraryRoot: typeof parsed.libraryRoot === 'string' ? parsed.libraryRoot : null,
      };
    }
  } catch {
    /* corrupted settings fall back to defaults */
  }
  return { theme: 'system', lowMemory: false, libraryRoot: null };
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
        libraryRoot: settings.libraryRoot,
      })
    );
  } catch {
    /* storage unavailable (private mode) — settings stay session-only */
  }
}

/** Resolve the effective theme, honoring the OS preference in system mode. */
export function effectiveTheme(): 'light' | 'dark' {
  if (settings.theme !== 'system') return settings.theme;
  return typeof window !== 'undefined' &&
    window.matchMedia?.('(prefers-color-scheme: dark)').matches
    ? 'dark'
    : 'light';
}

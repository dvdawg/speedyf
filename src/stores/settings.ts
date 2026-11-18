/** User settings persisted to localStorage. */
import { createSignal } from 'solid-js';
import { createStore } from 'solid-js/store';
import { engine } from '../lib/transport/engine';

export type ThemePref = 'system' | 'light' | 'dark';
export type LaunchScreen = 'home' | 'projects';
/** SleepyF, the night mode: off, on, or on while the system is dark. */
export type SleepyMode = 'off' | 'on' | 'system';

interface Settings {
  theme: ThemePref;
  /** Render pages as dark paper with light ink while the dark theme is in
   * effect. Off by default: it changes how a document *looks*, which is the
   * author's call before it is ours. */
  invertPages: boolean;
  /** SleepyF: the dark theme, warmer and quieter, with pages dimmed and
   * warmed. Off by default. */
  sleepyf: SleepyMode;
  /** How far SleepyF dims and warms the pages, 0 to 100. */
  sleepyWarmth: number;
  /** A faint tint over fillable form fields, and an outline on required
   * ones. On by default: an unmarked form hides which boxes take typing. */
  highlightFields: boolean;
  lowMemory: boolean;
  /** folders in the library; was a single optional root */
  libraryRoots: string[];
  /** Hosts the user chose not to be asked about again before following a
   * document's link out to the browser. Exact hosts only — see lib/links. */
  trustedLinkHosts: string[];
  /** Which screen shows when no document is open. See projects.md §5. */
  launchScreen: LaunchScreen;
  /** The library folder a PDF from outside the library is copied into when
   * it is added to a project. Null until the user picks one. */
  libraryInbox: string | null;
  /** Whether SpeedyF may fetch paper metadata from Crossref, arXiv and
   * OpenAlex. Off by default: it is the only network access SpeedyF has. */
  onlineLookup: boolean;
  /** Optional contact address sent in the User-Agent of lookups, which
   * Crossref rewards with better service. Empty by default. */
  lookupContact: string;
  /** Optional Semantic Scholar API key: a better rate limit, nothing else.
   * Empty by default. */
  s2Key: string;
  /** Whether certificate signing may ask a time-stamping server (RFC 3161)
   * to countersign the time. Off by default: it goes online, and only to
   * `timestampUrl`. Turned on from the signing dialog. */
  timestampOnline: boolean;
  /** The time-stamping server, typed by the user. Empty by default. */
  timestampUrl: string;
}

const KEY = 'speedyf-settings';
const DEFAULT_WARMTH = 50;

function sleepyMode(parsed: Partial<Settings> & { dimPages?: unknown }): SleepyMode {
  if (parsed.sleepyf === 'on' || parsed.sleepyf === 'system' || parsed.sleepyf === 'off') {
    return parsed.sleepyf;
  }
  // a schedule by the clock, which following the system replaced
  if ((parsed.sleepyf as unknown) === 'scheduled') return 'system';
  // the dim pages switch SleepyF grew out of
  return parsed.dimPages === true ? 'on' : 'off';
}

function load(): Settings {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Settings>;
      return {
        theme: parsed.theme === 'light' || parsed.theme === 'dark' ? parsed.theme : 'system',
        invertPages: parsed.invertPages === true,
        sleepyf: sleepyMode(parsed),
        sleepyWarmth:
          typeof parsed.sleepyWarmth === 'number' && Number.isFinite(parsed.sleepyWarmth)
            ? Math.min(100, Math.max(0, Math.round(parsed.sleepyWarmth)))
            : DEFAULT_WARMTH,
        highlightFields: parsed.highlightFields !== false,
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
        launchScreen: parsed.launchScreen === 'projects' ? 'projects' : 'home',
        libraryInbox: typeof parsed.libraryInbox === 'string' ? parsed.libraryInbox : null,
        onlineLookup: parsed.onlineLookup === true,
        lookupContact: typeof parsed.lookupContact === 'string' ? parsed.lookupContact : '',
        s2Key: typeof parsed.s2Key === 'string' ? parsed.s2Key : '',
        timestampOnline: parsed.timestampOnline === true,
        timestampUrl: typeof parsed.timestampUrl === 'string' ? parsed.timestampUrl : '',
      };
    }
  } catch {
    /* corrupted settings fall back to defaults */
  }
  return {
    theme: 'system',
    invertPages: false,
    sleepyf: 'off',
    sleepyWarmth: DEFAULT_WARMTH,
    highlightFields: true,
    lowMemory: false,
    libraryRoots: [],
    trustedLinkHosts: [],
    launchScreen: 'home',
    libraryInbox: null,
    onlineLookup: false,
    lookupContact: '',
    s2Key: '',
    timestampOnline: false,
    timestampUrl: '',
  };
}

const [settings, setSettingsStore] = createStore<Settings>(load());

export { settings };

export function updateSettings(patch: Partial<Settings>) {
  setSettingsStore(patch);
  // a new mode drops a hand toggle made under the old one
  if (patch.sleepyf !== undefined) setOverride(null);
  try {
    localStorage.setItem(
      KEY,
      JSON.stringify({
        theme: settings.theme,
        invertPages: settings.invertPages,
        sleepyf: settings.sleepyf,
        sleepyWarmth: settings.sleepyWarmth,
        highlightFields: settings.highlightFields,
        lowMemory: settings.lowMemory,
        libraryRoots: settings.libraryRoots,
        trustedLinkHosts: settings.trustedLinkHosts,
        launchScreen: settings.launchScreen,
        libraryInbox: settings.libraryInbox,
        onlineLookup: settings.onlineLookup,
        lookupContact: settings.lookupContact,
        s2Key: settings.s2Key,
        timestampOnline: settings.timestampOnline,
        timestampUrl: settings.timestampUrl,
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

/** A switch made by hand while SleepyF follows the system: it holds until
 * the system appearance next changes, then the system takes over again. */
const [override, setOverride] = createSignal<{ active: boolean; systemDark: boolean } | null>(null);

/** Whether SleepyF is in effect right now. */
export function sleepyActive(): boolean {
  if (settings.sleepyf === 'on') return true;
  if (settings.sleepyf === 'off') return false;
  const dark = systemDark();
  const held = override();
  return held && held.systemDark === dark ? held.active : dark;
}

/** Turns SleepyF over: on and off outright, or, while it follows the
 * system, the other way until the system appearance next changes. */
export function toggleSleepy(): void {
  if (settings.sleepyf !== 'system') {
    updateSettings({ sleepyf: settings.sleepyf === 'on' ? 'off' : 'on' });
    return;
  }
  setOverride({ active: !sleepyActive(), systemDark: systemDark() });
}

/** Resolve the effective theme, honoring the OS preference in system mode.
 * SleepyF is always dark. */
export function effectiveTheme(): 'light' | 'dark' {
  if (sleepyActive()) return 'dark';
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

/** Tells the stylesheet how pages look: the theme, whether the engine
 * inverts them, and whether SleepyF dims them and by how much. */
export function applyPageAppearance(root: HTMLElement): void {
  root.dataset.theme = effectiveTheme();
  // Pages come back inverted from the engine; the shell has to repaint the
  // paper it draws behind and around them to match.
  root.dataset.invertPages = pagesInverted() ? 'on' : 'off';
  // SleepyF is the shell's own: a filter over the rasters and a warmer set
  // of dark tokens (global.css).
  root.dataset.sleepy = sleepyActive() ? 'on' : 'off';
  root.style?.setProperty('--sleepy-warmth', String(settings.sleepyWarmth / 100));
}

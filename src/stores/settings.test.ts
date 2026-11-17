import { afterEach, describe, expect, it } from 'vitest';
import {
  applyPageAppearance,
  effectiveTheme,
  pagesInverted,
  settings,
  sleepyActive,
  toggleSleepy,
  updateSettings,
} from './settings';

afterEach(() =>
  updateSettings({
    theme: 'system',
    invertPages: false,
    sleepyf: 'off',
    sleepyWarmth: 50,
  })
);

describe('invertPages', () => {
  it('is off until it is asked for', () => {
    expect(settings.invertPages).toBe(false);
    expect(pagesInverted()).toBe(false);
  });

  it('takes effect only under the dark theme', () => {
    updateSettings({ invertPages: true, theme: 'light' });
    expect(pagesInverted()).toBe(false);

    updateSettings({ theme: 'dark' });
    expect(effectiveTheme()).toBe('dark');
    expect(pagesInverted()).toBe(true);
  });

  it('stays a standing preference across a trip through the light theme', () => {
    updateSettings({ invertPages: true, theme: 'dark' });
    updateSettings({ theme: 'light' });
    expect(pagesInverted()).toBe(false);

    // Not cleared on the way past: the setting says what to do *when* dark, so
    // going back to dark has to bring the inverted pages back with it.
    expect(settings.invertPages).toBe(true);
    updateSettings({ theme: 'dark' });
    expect(pagesInverted()).toBe(true);
  });

  it('is separable from the theme', () => {
    updateSettings({ theme: 'dark', invertPages: false });
    expect(effectiveTheme()).toBe('dark');
    expect(pagesInverted()).toBe(false);
  });
});

describe('SleepyF', () => {
  const appearance = (): Record<string, string | undefined> => {
    const props: Record<string, string> = {};
    const root = {
      dataset: {} as DOMStringMap,
      style: { setProperty: (k: string, v: string) => (props[k] = v) },
    } as unknown as HTMLElement;
    applyPageAppearance(root);
    return { ...root.dataset, warmth: props['--sleepy-warmth'] };
  };

  it('is off until it is asked for', () => {
    expect(settings.sleepyf).toBe('off');
    expect(sleepyActive()).toBe(false);
    expect(appearance().sleepy).toBe('off');
  });

  it('is always dark, whatever the theme', () => {
    updateSettings({ theme: 'light', sleepyf: 'on' });
    expect(effectiveTheme()).toBe('dark');
    expect(appearance()).toMatchObject({ theme: 'dark', sleepy: 'on', warmth: '0.5' });
    updateSettings({ sleepyf: 'off' });
    expect(effectiveTheme()).toBe('light');
  });

  it('dims inverted pages too', () => {
    updateSettings({ sleepyf: 'on', invertPages: true, theme: 'light' });
    expect(appearance()).toMatchObject({ invertPages: 'on', sleepy: 'on' });
  });

  it('passes its warmth to the stylesheet', () => {
    updateSettings({ sleepyf: 'on', sleepyWarmth: 80 });
    expect(appearance().warmth).toBe('0.8');
  });

  it('toggles outright when not scheduled', () => {
    toggleSleepy();
    expect(settings.sleepyf).toBe('on');
    toggleSleepy();
    expect(settings.sleepyf).toBe('off');
  });

  it('follows the system, and holds a hand toggle without changing the mode', () => {
    // the test environment's system appearance is light
    updateSettings({ sleepyf: 'system' });
    expect(sleepyActive()).toBe(false);
    toggleSleepy();
    expect(sleepyActive()).toBe(true);
    expect(settings.sleepyf).toBe('system');
    toggleSleepy();
    expect(sleepyActive()).toBe(false);
  });

  it('is saved with the other settings', () => {
    const saved: Record<string, string> = {};
    const real = globalThis.localStorage;
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      value: { setItem: (k: string, v: string) => (saved[k] = v), getItem: () => null },
    });
    try {
      updateSettings({ sleepyf: 'system', sleepyWarmth: 70 });
      const stored = JSON.parse(saved['speedyf-settings']!);
      expect(stored).toMatchObject({ sleepyf: 'system', sleepyWarmth: 70 });
    } finally {
      Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: real });
    }
  });
});

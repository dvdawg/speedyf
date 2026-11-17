import { afterEach, describe, expect, it } from 'vitest';
import {
  applyPageAppearance,
  effectiveTheme,
  inSleepyWindow,
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
    sleepyFrom: '22:00',
    sleepyTo: '07:00',
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

  it('holds a hand toggle while scheduled, without changing the mode', () => {
    // a window that is never in: the schedule says off
    updateSettings({ sleepyf: 'scheduled', sleepyFrom: '03:00', sleepyTo: '03:00' });
    expect(sleepyActive()).toBe(false);
    toggleSleepy();
    expect(sleepyActive()).toBe(true);
    expect(settings.sleepyf).toBe('scheduled');
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
      updateSettings({ sleepyf: 'scheduled', sleepyWarmth: 70 });
      const stored = JSON.parse(saved['speedyf-settings']!);
      expect(stored).toMatchObject({ sleepyf: 'scheduled', sleepyWarmth: 70 });
      expect(stored.sleepyFrom).toBe('22:00');
      expect(stored.sleepyTo).toBe('07:00');
    } finally {
      Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: real });
    }
  });
});

describe('inSleepyWindow', () => {
  const at = (h: number, m = 0) => h * 60 + m;

  it('runs past midnight when it ends earlier than it starts', () => {
    expect(inSleepyWindow('22:00', '07:00', at(23))).toBe(true);
    expect(inSleepyWindow('22:00', '07:00', at(3))).toBe(true);
    expect(inSleepyWindow('22:00', '07:00', at(7))).toBe(false);
    expect(inSleepyWindow('22:00', '07:00', at(12))).toBe(false);
    expect(inSleepyWindow('22:00', '07:00', at(22))).toBe(true);
  });

  it('runs within a day otherwise, and is never in when empty', () => {
    expect(inSleepyWindow('13:00', '15:30', at(14))).toBe(true);
    expect(inSleepyWindow('13:00', '15:30', at(15, 30))).toBe(false);
    expect(inSleepyWindow('13:00', '13:00', at(13))).toBe(false);
  });
});

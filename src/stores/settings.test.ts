import { afterEach, describe, expect, it } from 'vitest';
import {
  applyPageAppearance,
  effectiveTheme,
  pagesDimmed,
  pagesInverted,
  settings,
  updateSettings,
} from './settings';

afterEach(() => updateSettings({ theme: 'system', invertPages: false, dimPages: false }));

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

describe('dimPages', () => {
  const appearance = () => {
    const root = { dataset: {} as DOMStringMap } as HTMLElement;
    applyPageAppearance(root);
    return root.dataset;
  };

  it('is off until it is asked for', () => {
    expect(settings.dimPages).toBe(false);
    expect(pagesDimmed()).toBe(false);
  });

  it('dims only under the dark theme', () => {
    updateSettings({ dimPages: true, theme: 'light' });
    expect(pagesDimmed()).toBe(false);
    expect(appearance().dimPages).toBe('off');

    updateSettings({ theme: 'dark' });
    expect(pagesDimmed()).toBe(true);
    expect(appearance()).toMatchObject({ theme: 'dark', dimPages: 'on', invertPages: 'off' });
  });

  it('gives way to inversion', () => {
    updateSettings({ dimPages: true, invertPages: true, theme: 'dark' });
    expect(pagesDimmed()).toBe(false);
    expect(appearance()).toMatchObject({ invertPages: 'on', dimPages: 'off' });
    // Still standing: turning inversion off brings the dimming back.
    updateSettings({ invertPages: false });
    expect(appearance().dimPages).toBe('on');
  });

  it('is saved with the other settings', () => {
    const saved: Record<string, string> = {};
    const real = globalThis.localStorage;
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      value: { setItem: (k: string, v: string) => (saved[k] = v), getItem: () => null },
    });
    try {
      updateSettings({ dimPages: true });
      expect(JSON.parse(saved['speedyf-settings']!).dimPages).toBe(true);
    } finally {
      Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: real });
    }
  });
});

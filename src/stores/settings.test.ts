import { afterEach, describe, expect, it } from 'vitest';
import { effectiveTheme, pagesInverted, settings, updateSettings } from './settings';

afterEach(() => updateSettings({ theme: 'system', invertPages: false }));

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

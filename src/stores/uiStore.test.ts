import { describe, expect, it } from 'vitest';
import { DEFAULT_SIDEBAR_WIDTH, clampSidebarWidth, extensionPanelId } from './uiStore';

describe('clampSidebarWidth', () => {
  const roomy = 1600;

  it('keeps a reasonable width as asked', () => {
    expect(clampSidebarWidth(300, roomy)).toBe(300);
  });

  it('rounds to whole pixels, since a drag reports fractions', () => {
    expect(clampSidebarWidth(300.4, roomy)).toBe(300);
  });

  it('refuses to go narrower than the mode buttons need', () => {
    expect(clampSidebarWidth(20, roomy)).toBe(150);
    expect(clampSidebarWidth(-500, roomy)).toBe(150);
  });

  it('has an absolute ceiling however wide the window', () => {
    expect(clampSidebarWidth(5000, 4000)).toBe(560);
  });

  /** The document is the point of the app; a panel that can squeeze it to
   * nothing is a way to get the window into a state you cannot read out of. */
  it('never takes more than half a narrow window', () => {
    expect(clampSidebarWidth(900, 1000)).toBe(500);
    expect(clampSidebarWidth(400, 600)).toBe(300);
  });

  /** A width saved on an external display must not swallow a laptop screen. */
  it('clamps a width restored into a smaller window', () => {
    const onBigScreen = clampSidebarWidth(520, 2560);
    expect(onBigScreen).toBe(520);
    expect(clampSidebarWidth(onBigScreen, 900)).toBe(450);
  });

  /** The minimum wins over the fraction, or a very narrow window would produce
   * a sidebar too small to contain its own controls. */
  it('keeps the minimum even when half the window is less than it', () => {
    expect(clampSidebarWidth(200, 200)).toBe(150);
  });

  it('falls back to the default for a nonsense value', () => {
    expect(clampSidebarWidth(Number.NaN, roomy)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(clampSidebarWidth(Number.POSITIVE_INFINITY, roomy)).toBe(DEFAULT_SIDEBAR_WIDTH);
  });
});

describe('extensionPanelId', () => {
  it('reads the panel id out of an extension mode', () => {
    expect(extensionPanelId('ext:com.example.thing/refs')).toBe('com.example.thing/refs');
  });

  it('is null for a built-in mode', () => {
    expect(extensionPanelId('pages')).toBe(null);
    expect(extensionPanelId('notes')).toBe(null);
  });
});

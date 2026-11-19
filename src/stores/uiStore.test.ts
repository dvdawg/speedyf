import { describe, expect, it } from 'vitest';
import {
  DEFAULT_SIDEBAR,
  DEFAULT_SIDEBAR_WIDTH,
  clampSidebarWidth,
  extensionPanelId,
  isSidebarMode,
  resolveSidebar,
  sanitizeSidebar,
} from './uiStore';

describe('DEFAULT_SIDEBAR_WIDTH', () => {
  /** Measured from the CSS: `.icon-btn` is 28px wide (16px icon, 5px padding
   * and a 1px border each side), `.sidebar-mode-toggle` separates buttons by
   * 2px inside 6px of padding, and `.sidebar` carries a 1px right border in
   * its border-box width. Six built-ins — Pages, Outline, Formal, Figures,
   * Notes, Paper — must sit on one row by default. */
  it('fits the six built-in mode buttons on one row', () => {
    const buttons = 6;
    const needed = buttons * 28 + (buttons - 1) * 2 + 2 * 6 + 1;
    expect(DEFAULT_SIDEBAR_WIDTH).toBe(needed);
  });
});

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
    expect(extensionPanelId('paper')).toBe(null);
  });
});

describe('saved sidebar', () => {
  it('accepts every built-in panel and a well-formed extension panel', () => {
    for (const mode of ['pages', 'outline', 'formal', 'figures', 'notes', 'paper']) {
      expect(isSidebarMode(mode)).toBe(true);
    }
    expect(isSidebarMode('ext:com.example.thing/refs')).toBe(true);
  });

  it('refuses a panel this version does not have', () => {
    expect(isSidebarMode('thumbnails')).toBe(false);
    expect(isSidebarMode('ext:')).toBe(false);
    expect(isSidebarMode(3)).toBe(false);
  });

  it('keeps the good fields of a damaged record and drops the rest', () => {
    expect(sanitizeSidebar({ open: 'yes', width: 240, mode: 'gone' })).toEqual({ width: 240 });
    expect(sanitizeSidebar({ open: false, width: Number.NaN, mode: 'notes' })).toEqual({
      open: false,
      mode: 'notes',
    });
    expect(sanitizeSidebar({ width: -5 })).toBeUndefined();
    expect(sanitizeSidebar(null)).toBeUndefined();
    expect(sanitizeSidebar('wide')).toBeUndefined();
  });

  it('fills what an older record lacks from the fallback', () => {
    const fallback = { open: true, width: 300, mode: 'outline' as const };
    expect(resolveSidebar({ open: false }, fallback, 1600)).toEqual({
      open: false,
      width: 300,
      mode: 'outline',
    });
    expect(resolveSidebar(undefined, fallback, 1600)).toEqual(fallback);
  });

  it('clamps a width wider than the window it comes back into', () => {
    expect(resolveSidebar({ width: 900 }, DEFAULT_SIDEBAR, 1000).width).toBe(500);
    expect(resolveSidebar({ width: 5000 }, DEFAULT_SIDEBAR, 4000).width).toBe(560);
  });
});

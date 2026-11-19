import { describe, expect, it } from 'vitest';
import { fitToScreens, type WorkArea } from './windowState';
import type { WindowBounds } from '../../stores/sessionStore';

const laptop: WorkArea = { x: 0, y: 25, width: 1512, height: 920 };
/** An external display to the laptop's left. */
const external: WorkArea = { x: -2560, y: 0, width: 2560, height: 1415 };

const frame = (x: number, y: number, width = 1280, height = 840): WindowBounds => ({
  x,
  y,
  width,
  height,
  maximized: false,
});

describe('fitToScreens', () => {
  it('leaves a window that is on a screen where it was', () => {
    expect(fitToScreens(frame(100, 60), [laptop])).toEqual(frame(100, 60));
    // Negative coordinates are a real screen to the left, not off-screen.
    expect(fitToScreens(frame(-2000, 100), [laptop, external])).toEqual(frame(-2000, 100));
  });

  it('keeps a window hanging off an edge, as long as its title bar can be reached', () => {
    expect(fitToScreens(frame(1300, 60), [laptop])).toEqual(frame(1300, 60));
  });

  it('centres a window from an unplugged monitor on the primary screen', () => {
    const fitted = fitToScreens(frame(-2000, 100), [laptop]);
    expect(fitted).toEqual({
      x: Math.round((1512 - 1280) / 2),
      y: Math.round(25 + (920 - 840) / 2),
      width: 1280,
      height: 840,
      maximized: false,
    });
  });

  it('shrinks a window larger than the screen it lands on', () => {
    const fitted = fitToScreens(frame(-2400, 50, 2400, 1300), [laptop]);
    expect(fitted.width).toBe(1512);
    expect(fitted.height).toBe(920);
    expect(fitted.x).toBe(0);
    expect(fitted.y).toBe(25);
  });

  it('never leaves the title bar above the top of the screen', () => {
    expect(fitToScreens(frame(100, 10), [laptop]).y).toBe(25);
  });

  it('brings back a window with only a sliver of title bar showing', () => {
    const fitted = fitToScreens(frame(1500, 60), [laptop]);
    expect(fitted.x).toBe(Math.round((1512 - 1280) / 2));
  });

  it('keeps maximized, and leaves the frame alone when the screens are unknown', () => {
    const maximized = { ...frame(-2000, 100), maximized: true };
    expect(fitToScreens(maximized, [laptop]).maximized).toBe(true);
    expect(fitToScreens(maximized, [])).toEqual(maximized);
  });
});

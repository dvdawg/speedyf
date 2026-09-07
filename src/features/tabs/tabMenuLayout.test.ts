import { describe, expect, it } from 'vitest';
import { MENU_WIDTH, tabMenuHeight, tabMenuPosition } from './tabMenuLayout';

const viewport = { width: 1000, height: 700 };
const size = { width: MENU_WIDTH, height: 200 };

describe('tabMenuPosition', () => {
  it('opens from the pointer when there is room', () => {
    expect(tabMenuPosition({ x: 300, y: 40 }, size, viewport)).toEqual({ left: 300, top: 40 });
  });

  it('slides back inside the right edge', () => {
    const { left } = tabMenuPosition({ x: 980, y: 40 }, size, viewport);
    expect(left).toBe(viewport.width - MENU_WIDTH - 8);
  });

  it('flips above the pointer near the bottom edge, never under it', () => {
    const at = { x: 300, y: 640 };
    const { top } = tabMenuPosition(at, size, viewport);
    // Sliding up would leave the menu covering the cursor, with a close-tabs
    // item sitting under it.
    expect(top).toBe(at.y - size.height);
    expect(top + size.height).toBeLessThanOrEqual(at.y);
  });

  it('pins to the top margin when it fits neither way', () => {
    const tall = { width: MENU_WIDTH, height: 900 };
    expect(tabMenuPosition({ x: 10, y: 400 }, tall, viewport).top).toBe(8);
  });

  it('grows by one row per item and one rule per separator', () => {
    const oneRow = tabMenuHeight(1, 0);
    expect(tabMenuHeight(2, 0) - oneRow).toBe(28);
    expect(tabMenuHeight(1, 1) - oneRow).toBe(9);
  });
});

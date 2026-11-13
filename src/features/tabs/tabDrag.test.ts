import { expect, it } from 'vitest';
import { isOutsideTabStrip, isTabDrag } from './tabDrag';

it('allows a small click movement and a forgiving strip edge', () => {
  expect(isTabDrag({ x: 100, y: 20 }, 102, 22)).toBe(false);
  expect(isTabDrag({ x: 100, y: 20 }, 110, 20)).toBe(true);
  const strip = { left: 0, right: 900, top: 0, bottom: 40 };
  expect(isOutsideTabStrip(strip, 500, 50)).toBe(false);
  expect(isOutsideTabStrip(strip, 500, 100)).toBe(true);
  expect(isOutsideTabStrip(strip, -50, 20)).toBe(true);
  expect(isOutsideTabStrip(strip, 950, 20)).toBe(true);
  expect(isOutsideTabStrip(strip, 500, -50)).toBe(true);
});

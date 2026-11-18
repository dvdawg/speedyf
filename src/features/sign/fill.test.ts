import { describe, expect, it } from 'vitest';
import type { Blank } from '../../types/engine';
import {
  blankAt,
  formatDate,
  freeText,
  markPath,
  markRect,
  nearbyFontSize,
  nextStop,
  snapText,
  typingStops,
} from './fill';

const blank = (
  kind: Blank['kind'],
  x: number,
  y: number,
  w: number,
  h: number,
  cells = 0
): Blank => ({
  rect: { x, y, w, h },
  kind,
  fontSizePt: 10,
  cells,
});

/** Where the saved text's first baseline lands: one font size under the top
 * of its box (engine save.rs). */
const baselineOf = (r: { y: number; h: number }, size: number) => r.y + r.h - size;

describe('snapping text onto a blank', () => {
  it('stands the writing on the rule of a line blank', () => {
    const s = snapText(blank('line', 100, 500, 200, 14));
    expect(s.fontSizePt).toBe(10);
    const baseline = baselineOf(s.rect, 10);
    expect(baseline).toBeGreaterThan(500);
    expect(baseline).toBeLessThan(502.5);
    expect(s.rect.x).toBeCloseTo(102);
    expect(s.combCells).toBeUndefined();
  });

  it('centres the writing in a box', () => {
    const s = snapText(blank('box', 100, 400, 120, 30));
    const baseline = baselineOf(s.rect, 10);
    // Cap height centred: baseline about a third of an em under the middle.
    expect(baseline + 3.5).toBeCloseTo(415);
    expect(s.rect.x).toBeGreaterThan(100);
    expect(s.rect.x + s.rect.w).toBeLessThan(220);
  });

  it('spreads a comb across its cells', () => {
    const s = snapText(blank('comb', 100, 300, 126, 16, 9));
    expect(s.combCells).toBe(9);
    expect(s.rect.x).toBe(100);
    expect(s.rect.w).toBe(126);
  });
});

describe('finding the blank a click means', () => {
  const blanks = [
    blank('line', 100, 500, 200, 14),
    blank('box', 50, 300, 300, 100),
    blank('check', 60, 350, 10, 10),
  ];

  it('takes the blank the click is in, the tightest when they nest', () => {
    expect(blankAt(blanks, { x: 150, y: 505 })).toBe(0);
    expect(blankAt(blanks, { x: 65, y: 355 })).toBe(2);
    expect(blankAt(blanks, { x: 200, y: 320 })).toBe(1);
  });

  it('forgives a near miss but not a far one', () => {
    expect(blankAt(blanks, { x: 150, y: 497 })).toBe(0);
    expect(blankAt(blanks, { x: 150, y: 480 })).toBeNull();
  });

  it('can be limited to some kinds', () => {
    expect(blankAt(blanks, { x: 65, y: 355 }, ['box'])).toBe(1);
  });
});

describe('Tab order', () => {
  const blanks = [
    blank('line', 0, 700, 100, 14),
    blank('check', 0, 650, 10, 10),
    blank('box', 0, 600, 100, 20),
    blank('comb', 0, 550, 100, 16, 5),
  ];

  it('visits every blank that takes typing, skipping checkboxes', () => {
    expect(typingStops(blanks)).toEqual([0, 2, 3]);
    expect(nextStop(blanks, 0, 1)).toBe(2);
    expect(nextStop(blanks, 2, -1)).toBe(0);
  });

  it('leaves the page at either end', () => {
    expect(nextStop(blanks, 3, 1)).toBeNull();
    expect(nextStop(blanks, 0, -1)).toBeNull();
    expect(nextStop(blanks, null, 1)).toBe(0);
    expect(nextStop(blanks, null, -1)).toBe(3);
    expect(nextStop([], null, 1)).toBeNull();
  });
});

describe('free text and marks', () => {
  const runs = [
    { x: 72, y: 698, w: 60, h: 9 },
    { x: 72, y: 400, w: 200, h: 20 },
  ];

  it('takes its size from the text on the same line, clamped', () => {
    expect(nearbyFontSize(runs, { x: 200, y: 702 })).toBe(9);
    expect(nearbyFontSize(runs, { x: 300, y: 410 })).toBe(12);
    expect(nearbyFontSize(runs, { x: 300, y: 100 })).toBe(10);
  });

  it('puts a free box so the caret is where the click was', () => {
    const r = freeText({ x: 200, y: 500 }, 10, 612);
    const baseline = r.y + r.h - 10;
    expect(baseline + 3.5).toBeCloseTo(500);
    expect(r.x).toBe(200);
  });

  it('centres a mark in a small box', () => {
    const r = markRect('check', { x: 61, y: 351 }, 10, { x: 60, y: 350, w: 10, h: 10 });
    expect(r.x + r.w / 2).toBeCloseTo(65);
    expect(r.y + r.h / 2).toBeCloseTo(355);
    expect(r.w).toBeCloseTo(8);
  });

  it('sizes a free mark from the text around it', () => {
    expect(markRect('cross', { x: 100, y: 100 }, 10)).toEqual({ x: 95, y: 95, w: 10, h: 10 });
    const line = markRect('line', { x: 100, y: 100 }, 10);
    expect(line.w).toBeGreaterThan(line.h * 4);
    const circle = markRect('circle', { x: 100, y: 100 }, 10);
    expect(circle.w).toBeGreaterThan(circle.h);
  });

  it('draws every mark', () => {
    for (const m of ['check', 'cross', 'dot', 'line', 'circle'] as const) {
      expect(markPath(m, { x: 10, y: 10, w: 12, h: 12 }, 792)).toMatch(/^M /);
    }
  });
});

describe('the date stamp', () => {
  const day = new Date(2026, 10, 7);

  it('writes the date in the chosen format', () => {
    expect(formatDate(day, 'us')).toBe('11/07/2026');
    expect(formatDate(day, 'iso')).toBe('2026-11-07');
    expect(formatDate(day, 'dmy')).toBe('07/11/2026');
  });
});

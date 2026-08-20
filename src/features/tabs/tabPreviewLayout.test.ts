import { describe, expect, it } from 'vitest';
import { tabPreviewPosition } from './tabPreviewLayout';

const size = { width: 240, height: 320 };
const viewport = { width: 1280, height: 800 };

describe('tabPreviewPosition', () => {
  it('centers the card under its tab', () => {
    const { left, top } = tabPreviewPosition({ left: 500, right: 620, bottom: 36 }, size, viewport);
    expect(left).toBe(560 - 120);
    expect(top).toBe(42);
  });

  it('pulls a card back inside the right edge', () => {
    // The last tab in a full strip: centered would hang off the window.
    const { left } = tabPreviewPosition({ left: 1200, right: 1270, bottom: 36 }, size, viewport);
    expect(left).toBe(viewport.width - size.width - 8);
  });

  it('pulls a card back inside the left edge', () => {
    const { left } = tabPreviewPosition({ left: 0, right: 40, bottom: 36 }, size, viewport);
    expect(left).toBe(8);
  });

  it('keeps a tall card inside a short window', () => {
    const { top } = tabPreviewPosition(
      { left: 500, right: 620, bottom: 36 },
      { width: 240, height: 700 },
      { width: 1280, height: 500 }
    );
    expect(top).toBe(8);
  });

  it('pins to the left when the card is wider than the window', () => {
    const { left } = tabPreviewPosition(
      { left: 100, right: 200, bottom: 36 },
      { width: 900, height: 300 },
      { width: 600, height: 800 }
    );
    expect(left).toBe(8);
  });
});

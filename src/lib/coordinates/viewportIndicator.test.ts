import { describe, expect, it } from 'vitest';
import {
  cornerDragScale,
  pageViewportFraction,
  scrollForPageFraction,
  viewportOriginFraction,
} from './viewportIndicator';
import { layoutPages } from './layout';
import type { Rotation } from '../../types/model';

const P = (w: number, h: number, rotation: Rotation = 0) => ({ widthPt: w, heightPt: h, rotation });

// Three 600x800 pages at zoom 1: tops 16, 832, 1648; each 600 wide, centred
// in a 1000px container so lefts are 200.
const opts = { gap: 16, padding: 16, containerW: 1000 };
const layout = layoutPages([P(600, 800), P(600, 800), P(600, 800)], 1, opts);

const view = (scrollTop: number, scrollLeft = 0, containerW = 1000, containerH = 400) => ({
  scrollTop,
  scrollLeft,
  containerW,
  containerH,
});

describe('pageViewportFraction', () => {
  it('reports the top slice of a page scrolled to its own top edge', () => {
    const f = pageViewportFraction(layout, 0, view(16));
    expect(f).toEqual({ x: 0, y: 0, w: 1, h: 0.5 });
  });

  it('tracks a partial slice in the middle of a page', () => {
    // viewport covers content y 216..616; page 0 spans 16..816
    const f = pageViewportFraction(layout, 0, view(216))!;
    expect(f.y).toBeCloseTo(0.25);
    expect(f.h).toBeCloseTo(0.5);
  });

  it('clips to the page box when the viewport overhangs both edges', () => {
    // a tall viewport swallowing page 0 whole still reports the full page
    const f = pageViewportFraction(layout, 0, view(0, 0, 1000, 2000));
    expect(f).toEqual({ x: 0, y: 0, w: 1, h: 1 });
  });

  it('returns a slice for each page when the viewport straddles two', () => {
    // page 0 ends at 816, page 1 starts at 832
    const v = view(700, 0, 1000, 400); // covers 700..1100
    const a = pageViewportFraction(layout, 0, v)!;
    const b = pageViewportFraction(layout, 1, v)!;
    expect(a.y).toBeCloseTo(0.855);
    expect(a.h).toBeCloseTo(0.145); // 816-700 = 116 of 800
    expect(b.y).toBe(0);
    expect(b.h).toBeCloseTo(0.335); // 1100-832 = 268 of 800
  });

  it('returns null for a page entirely off screen', () => {
    expect(pageViewportFraction(layout, 2, view(0, 0, 1000, 400))).toBeNull();
  });

  it('returns null for an out-of-range page index', () => {
    expect(pageViewportFraction(layout, 9, view(0))).toBeNull();
  });

  it('reports a horizontal slice when zoomed in past the container width', () => {
    // page 0 spans x 200..800; a 300px-wide window at scrollLeft 350
    const f = pageViewportFraction(layout, 0, view(16, 350, 300, 400))!;
    expect(f.x).toBeCloseTo(0.25); // (350-200)/600
    expect(f.w).toBeCloseTo(0.5); // 300/600
  });
});

describe('viewportOriginFraction', () => {
  it('matches the clipped fraction when the origin is inside the page', () => {
    const v = view(216);
    const origin = viewportOriginFraction(layout, 0, v)!;
    const clipped = pageViewportFraction(layout, 0, v)!;
    expect(origin.y).toBeCloseTo(clipped.y);
  });

  it('goes negative for a page the viewport has not reached yet', () => {
    // viewport at 700 covers 700..1100; page 1 starts at 832, so the origin is
    // above it. The clipped fraction says 0 and would teleport a drag.
    const v = view(700);
    expect(pageViewportFraction(layout, 1, v)!.y).toBe(0);
    expect(viewportOriginFraction(layout, 1, v)!.y).toBeCloseTo(-0.165);
  });

  it('round-trips through scrollForPageFraction for a straddling view', () => {
    const v = view(700);
    const origin = viewportOriginFraction(layout, 1, v)!;
    const pos = scrollForPageFraction(layout, 1, origin.x, origin.y)!;
    expect(pos.top).toBeCloseTo(v.scrollTop);
    expect(pos.left).toBeCloseTo(v.scrollLeft);
  });

  it('returns null for an out-of-range page index', () => {
    expect(viewportOriginFraction(layout, 9, view(0))).toBeNull();
  });
});

describe('scrollForPageFraction', () => {
  it('round-trips with pageViewportFraction', () => {
    const v = view(216, 350, 300, 400);
    const f = pageViewportFraction(layout, 0, v)!;
    const pos = scrollForPageFraction(layout, 0, f.x, f.y)!;
    expect(pos.top).toBeCloseTo(v.scrollTop);
    expect(pos.left).toBeCloseTo(v.scrollLeft);
  });

  it('maps the page origin to the page box origin', () => {
    expect(scrollForPageFraction(layout, 1, 0, 0)).toEqual({ top: 832, left: 200 });
  });

  it('carries past the page edge rather than clamping', () => {
    // dragging the box below the page bottom should scroll into the next page
    const pos = scrollForPageFraction(layout, 0, 0, 1.5)!;
    expect(pos.top).toBe(16 + 1200);
  });

  it('returns null for an out-of-range page index', () => {
    expect(scrollForPageFraction(layout, 9, 0, 0)).toBeNull();
  });
});

describe('cornerDragScale', () => {
  it('is a no-op for a zero drag', () => {
    expect(cornerDragScale(100, 50, 0, 0)).toBe(1);
  });

  it('grows the box when dragged outward', () => {
    expect(cornerDragScale(100, 100, 10, 10)).toBeCloseTo(1.1);
  });

  it('shrinks the box when dragged inward', () => {
    expect(cornerDragScale(100, 100, -10, -10)).toBeCloseTo(0.9);
  });

  it('tracks the dragged axis 1:1 on a wide, short fit-width band', () => {
    // 148x20 box dragged 10px down: the corner must end up 10px lower, so the
    // box has to become 30px tall — a scale of 1.5, not a token nudge.
    const scale = cornerDragScale(148, 20, 0, 10);
    expect(scale).toBeCloseTo(1.5);
    expect(20 * scale - 20).toBeCloseTo(10);
  });

  it('tracks the horizontal axis 1:1 on a tall, narrow box', () => {
    const scale = cornerDragScale(20, 148, 10, 0);
    expect(20 * scale - 20).toBeCloseTo(10);
  });

  it('follows the axis with the larger relative movement', () => {
    // 5px on the 20px axis outweighs 20px on the 148px axis
    expect(cornerDragScale(148, 20, 20, 5)).toBeCloseTo(1.25);
  });

  it('floors the scale instead of inverting when dragged past the origin', () => {
    expect(cornerDragScale(100, 100, -500, -500)).toBe(0.05);
  });

  it('survives a degenerate zero-size box', () => {
    expect(cornerDragScale(0, 0, 10, 10)).toBe(1);
  });
});

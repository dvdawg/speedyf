import { describe, expect, it } from 'vitest';
import {
  GESTURE_GAP_MS,
  MARGIN_X,
  PUSH_PX,
  columnAt,
  edgeState,
  fitZoom,
  keyStep,
  pageFlow,
  scrollRange,
  segmentAt,
  wheelStep,
  type FlowSegment,
} from './columnFlow';
import { MAX_ZOOM } from '../../../stores/viewportStore';
import type { ColumnLayout } from '../../../types/engine';

const upright = { widthPt: 612, heightPt: 792, rotation: 0 as const };
const title: FlowSegment = { kind: 'block', rect: [54, 690, 504, 50] };
const left: FlowSegment = { kind: 'column', rect: [54, 72, 240, 600] };
const right: FlowSegment = { kind: 'column', rect: [318, 72, 240, 600] };
const page = [title, left, right];

describe('segmentAt', () => {
  it('finds the column under a point', () => {
    expect(segmentAt(page, 100, 400)).toBe(1);
    expect(segmentAt(page, 400, 400)).toBe(2);
    expect(segmentAt(page, 300, 710)).toBe(0);
  });

  it('takes the first column in reading order when the target names only a height', () => {
    expect(segmentAt(page, null, 400)).toBe(1);
  });

  it('sends a point in the gutter to the nearer column', () => {
    expect(segmentAt(page, 310, 400)).toBe(2);
    expect(segmentAt(page, 300, 400)).toBe(1);
  });

  it('sends a point between bands on to what is read next', () => {
    expect(segmentAt(page, 100, 680)).toBe(1);
  });
});

describe('columnAt', () => {
  it('answers for columns only, so a double-click on a title does nothing', () => {
    expect(columnAt(page, 100, 400)).toBe(1);
    expect(columnAt(page, 300, 710)).toBe(-1);
  });
});

describe('fitZoom', () => {
  it('fits the column width with a margin either side', () => {
    const z = fitZoom(left, upright, 600, null);
    expect(240 * z).toBeCloseTo(600 - 2 * MARGIN_X);
  });

  it('stops at the zoom limit for a very narrow column', () => {
    const sliver: FlowSegment = { kind: 'column', rect: [100, 100, 20, 600] };
    expect(fitZoom(sliver, upright, 1600, null)).toBe(MAX_ZOOM);
  });

  it('keeps the zoom between columns of nearly the same width', () => {
    const z = fitZoom(left, upright, 600, null);
    const nearly: FlowSegment = { kind: 'column', rect: [318, 72, 236, 600] };
    expect(fitZoom(nearly, upright, 600, z)).toBe(z);
    // A full-width block is a different fit.
    expect(fitZoom(title, upright, 600, z)).toBeLessThan(z);
  });

  it('measures a turned page across its displayed width', () => {
    const turned = { widthPt: 612, heightPt: 792, rotation: 90 as const };
    const whole: FlowSegment = { kind: 'page', rect: [0, 0, 612, 792] };
    expect(fitZoom(whole, turned, 820, null)).toBeCloseTo((820 - 2 * MARGIN_X) / 792);
  });
});

describe('scrollRange', () => {
  it('has one stop for a segment shorter than the view', () => {
    const r = scrollRange({ x: 0, y: 1000, w: 500, h: 200 }, 700);
    expect(r.lo).toBe(r.hi);
  });

  it('spans a tall segment top to bottom', () => {
    const r = scrollRange({ x: 0, y: 1000, w: 500, h: 2000 }, 700);
    expect(r.hi - r.lo).toBeGreaterThan(1000);
  });
});

describe('pageFlow', () => {
  const layout: ColumnLayout = {
    src: 0,
    kind: 'columns',
    columns: 2,
    bands: 1,
    segments: [
      { kind: 'column', rect: left.rect, band: 0, column: 0 },
      { kind: 'column', rect: right.rect, band: 0, column: 1 },
    ],
    source: 'text',
    reason: '',
  };

  it('reads a page with no confident reading as the whole page', () => {
    const none = { ...layout, kind: 'none' as const, segments: [] };
    expect(pageFlow(none, upright, 612, 792).segments).toEqual([
      { kind: 'page', rect: [0, 0, 612, 792] },
    ]);
  });

  it('reads a turned page as the whole page: columns are read upright', () => {
    const turned = { ...upright, rotation: 180 as const };
    expect(pageFlow(layout, turned, 612, 792).kind).toBe('none');
  });

  it('keeps the engine reading order', () => {
    expect(pageFlow(layout, upright, 612, 792).segments.map((s) => s.rect[0])).toEqual([54, 318]);
  });
});

describe('wheelStep', () => {
  it('scrolls within the segment and stops at its end', () => {
    const s = edgeState();
    expect(wheelStep(s, 0, 50, 100, 100, 400)).toEqual({ top: 150, advance: 0 });
    expect(wheelStep(s, 16, 500, 150, 100, 400)).toEqual({ top: 400, advance: 0 });
  });

  it('does not let the swipe that reached the end, or its momentum, move on', () => {
    const s = edgeState();
    wheelStep(s, 0, 600, 100, 100, 400);
    let t = 16;
    for (let i = 0; i < 60; i++, t += 16) {
      expect(wheelStep(s, t, 40, 400, 100, 400).advance).toBe(0);
    }
  });

  it('moves on when a fresh gesture pushes past the end', () => {
    const s = edgeState();
    wheelStep(s, 0, 600, 100, 100, 400);
    const later = GESTURE_GAP_MS + 50;
    expect(wheelStep(s, later, PUSH_PX / 2, 400, 100, 400).advance).toBe(0);
    expect(wheelStep(s, later + 16, PUSH_PX / 2, 400, 100, 400).advance).toBe(1);
    // ...once: the rest of that gesture cannot skip a second segment.
    expect(wheelStep(s, later + 32, PUSH_PX * 3, 100, 100, 400).advance).toBe(0);
  });

  it('works backwards the same way', () => {
    const s = edgeState();
    expect(wheelStep(s, 0, -300, 250, 100, 400)).toEqual({ top: 100, advance: 0 });
    expect(wheelStep(s, 16, -300, 100, 100, 400).advance).toBe(0);
    expect(wheelStep(s, 500, -PUSH_PX, 100, 100, 400).advance).toBe(-1);
  });

  it('starts over when the direction reverses', () => {
    const s = edgeState();
    wheelStep(s, 0, 600, 100, 100, 400);
    expect(wheelStep(s, 10, -30, 400, 100, 400)).toEqual({ top: 370, advance: 0 });
  });
});

describe('keyStep', () => {
  it('scrolls, then moves on at the end on a fresh press but not on repeats', () => {
    expect(keyStep(40, false, 100, 100, 400)).toEqual({ top: 140, advance: 0 });
    expect(keyStep(40, true, 400, 100, 400).advance).toBe(0);
    expect(keyStep(40, false, 400, 100, 400).advance).toBe(1);
    expect(keyStep(-40, false, 100, 100, 400).advance).toBe(-1);
  });
});

import { describe, expect, it } from 'vitest';
import {
  anchorBounds,
  anchorTopFromTop,
  compareReading,
  markerRects,
  numberComments,
  quadsFromRects,
  rectFromBox,
  rectFromViewBox,
  revealRects,
} from './geometry';
import type { Anchor, MarkupComment } from './types';

const H = 800;

function textAnchor(page: number, x: number, top: number, w = 40, h = 10): Anchor {
  return { page, quads: quadsFromRects([[x, top, w, h]], H), rect: null, quote: 'words' };
}

function comment(id: string, anchor: Anchor, created = 0): MarkupComment {
  return {
    id,
    kind: 'comment',
    anchor,
    body: '',
    replacement: null,
    quickmark: null,
    created,
  };
}

describe('quadsFromRects', () => {
  it('joins the runs of one line and flips the page y-up', () => {
    const quads = quadsFromRects(
      [
        [100, 50, 30, 10],
        [132, 50.5, 40, 10],
        [72, 62, 200, 10],
      ],
      H
    );
    expect(quads).toHaveLength(2);
    expect(quads[0]).toEqual({
      x1: 100,
      y1: 750,
      x2: 172,
      y2: 750,
      x3: 100,
      y3: 739.5,
      x4: 172,
      y4: 739.5,
    });
    expect(quads[1]!.x1).toBe(72);
    expect(quads[1]!.y1).toBe(738);
  });

  it('drops empty rectangles', () => {
    expect(quadsFromRects([[0, 0, 0, 10]], H)).toEqual([]);
  });
});

describe('anchors', () => {
  it('turns a dragged box into a y-up rect', () => {
    expect(rectFromBox([10, 20, 100, 50], H)).toEqual({ x: 10, y: 730, w: 100, h: 50 });
  });

  it('measures the top of an anchor from the top of the page', () => {
    expect(anchorTopFromTop(textAnchor(0, 72, 120), H)).toBe(120);
    const region: Anchor = {
      page: 0,
      quads: null,
      rect: { x: 0, y: 600, w: 10, h: 50 },
      quote: '',
    };
    expect(anchorTopFromTop(region, H)).toBe(150);
    expect(anchorBounds(region)).toEqual(region.rect);
  });

  it('gives the revealer one box per line', () => {
    const a: Anchor = {
      page: 0,
      quads: quadsFromRects(
        [
          [10, 10, 20, 10],
          [10, 30, 20, 10],
        ],
        H
      ),
      rect: null,
      quote: '',
    };
    expect(revealRects(a)).toEqual([
      [10, 780, 20, 10],
      [10, 760, 20, 10],
    ]);
  });
});

describe('reading order', () => {
  it('numbers by page, then down the page, then across a line', () => {
    const list = [
      comment('late-page', textAnchor(3, 72, 100)),
      comment('right', textAnchor(1, 300, 401)),
      comment('top', textAnchor(1, 72, 90)),
      comment('left', textAnchor(1, 72, 400)),
    ];
    expect(numberComments(list).map((e) => `${e.number}:${e.comment.id}`)).toEqual([
      '1:top',
      '2:left',
      '3:right',
      '4:late-page',
    ]);
  });

  it('keeps two comments on the same words in the order they were made', () => {
    const a = textAnchor(0, 72, 100);
    const list = [comment('second', a, 20), comment('first', a, 10)];
    expect(numberComments(list).map((e) => e.comment.id)).toEqual(['first', 'second']);
    expect(compareReading(a, a)).toBe(0);
  });
});

describe('markerRects', () => {
  it('puts a number just after the phrase, riding on its top edge', () => {
    const a = textAnchor(0, 72, 100, 40, 10);
    const r = markerRects([{ number: 3, anchor: a }]).get(3)!;
    expect(r.x).toBeCloseTo(112.6, 5);
    // Mostly above the line's top (y 700), a little into it.
    expect(r.y).toBeLessThan(700);
    expect(r.y + r.h).toBeGreaterThan(700 + r.h / 2);
    expect(r.w).toBeLessThan(r.h + 0.01);
  });

  it('widens for longer numbers and moves a marker off one it would touch', () => {
    const a = textAnchor(0, 72, 100, 40, 10);
    const rects = markerRects([
      { number: 9, anchor: a },
      { number: 12, anchor: a },
    ]);
    const first = rects.get(9)!;
    const second = rects.get(12)!;
    expect(second.w).toBeGreaterThan(first.w);
    expect(second.x).toBeGreaterThanOrEqual(first.x + first.w);
  });

  it('marks a region at its top-right corner', () => {
    const region: Anchor = {
      page: 0,
      quads: null,
      rect: { x: 100, y: 300, w: 200, h: 100 },
      quote: '',
    };
    const r = markerRects([{ number: 1, anchor: region }]).get(1)!;
    expect(r.x + r.w).toBeGreaterThan(300);
    expect(r.y + r.h).toBeGreaterThan(400);
    expect(r.x).toBeLessThan(300);
  });
});

describe('rectFromViewBox', () => {
  const page = { widthPt: 600, heightPt: 800 };
  const box = [100, 50, 40, 20] as const;
  it('is the plain flip on an unturned page', () => {
    expect(rectFromViewBox(box, { ...page, rotation: 0 })).toEqual(rectFromBox(box, 800));
  });

  it("finds the page's own place of a box dragged on a turned page", () => {
    // Turned a quarter clockwise, the page shows 800 wide and 600 tall; its
    // own top-left corner is at the shown top-right.
    expect(rectFromViewBox([760, 0, 40, 20], { ...page, rotation: 90 })).toEqual({
      x: 0,
      y: 760,
      w: 20,
      h: 40,
    });
    expect(rectFromViewBox([0, 0, 40, 20], { ...page, rotation: 180 })).toEqual({
      x: 560,
      y: 0,
      w: 40,
      h: 20,
    });
    expect(rectFromViewBox([0, 0, 40, 20], { ...page, rotation: 270 })).toEqual({
      x: 580,
      y: 760,
      w: 20,
      h: 40,
    });
  });
});

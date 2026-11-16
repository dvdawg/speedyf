/** Filling the canvas: a small graph is spread to use a large canvas,
 * within its caps (no more than MAX_SPREAD, no skew past MAX_SKEW); a
 * large one is left as it was; Fit never zooms past 1; and positions kept
 * in layout units come back where they were under any spread. Plus the
 * layout's lean toward the canvas's shape for a few nodes that would
 * otherwise stack into a column. */
import { describe, expect, it } from 'vitest';
import {
  boundsOf,
  fitTransform,
  MAX_SKEW,
  MAX_SPREAD,
  NO_SPREAD,
  spreadFor,
  spreadPoint,
  unspreadPoint,
} from './graphFit';
import type { Bounds, Reach, Size, Spread } from './graphFit';
import { LABEL_GAP, LABEL_HEIGHT, labelWidth, layoutGraph } from './graphLayout';
import type { LayoutEdge, LayoutNode, NodeBox } from './graphLayout';

/** The view's padding and smallest zoom. */
const PAD = { top: 44, right: 32, bottom: 28, left: 32 };
const MIN_ZOOM = 0.25;
/** The canvas from the report: a large window, a five-node graph. */
const BIG: Size = { width: 1700, height: 1300 };
const roomOf = (canvas: Size): Size => ({
  width: canvas.width - PAD.left - PAD.right,
  height: canvas.height - PAD.top - PAD.bottom,
});

const RADIUS = 5;
const labelled = (label = 'Zho25'): NodeBox => ({
  left: RADIUS + 4,
  right: RADIUS + LABEL_GAP + labelWidth(label),
  height: LABEL_HEIGHT,
});
/** A longer label, as a disambiguated or multi-author one. */
const WIDE: NodeBox = { left: 8, right: 48, height: LABEL_HEIGHT };
const GHOST: NodeBox = { left: 8, right: 8, height: LABEL_HEIGHT };

const reachOf = (boxes: NodeBox[]): Reach => ({
  left: Math.max(...boxes.map((b) => b.left)),
  right: Math.max(...boxes.map((b) => b.right)),
  top: LABEL_HEIGHT / 2,
  bottom: LABEL_HEIGHT / 2,
});

/** How much of the room (the canvas less padding) the fitted drawing
 * covers, across and down, with and without the spread: what the view
 * does, step by step. */
function occupancy(nodes: LayoutNode[], edges: LayoutEdge[], canvas: Size, spreading: boolean) {
  const room = roomOf(canvas);
  const layout = layoutGraph(nodes, edges, spreading ? { aspect: room.width / room.height } : {});
  const boxes = nodes.map((n) => n.box!);
  const s = spreading ? spreadFor(boundsOf(layout.points), reachOf(boxes), room) : NO_SPREAD;
  const placed = layout.points.map((p) => spreadPoint(p, s));
  const extent: Bounds = {
    minX: Math.min(...placed.map((p, i) => p.x - boxes[i]!.left)),
    maxX: Math.max(...placed.map((p, i) => p.x + boxes[i]!.right)),
    minY: Math.min(...placed.map((p, i) => p.y - boxes[i]!.height / 2)),
    maxY: Math.max(...placed.map((p, i) => p.y + boxes[i]!.height / 2)),
  };
  const t = fitTransform(extent, canvas, PAD, MIN_ZOOM);
  const across = ((extent.maxX - extent.minX) * t.k) / room.width;
  const down = ((extent.maxY - extent.minY) * t.k) / room.height;
  return { spread: s, k: t.k, across, down, area: across * down, layout, placed };
}

const chain = (ids: string[]): LayoutEdge[] =>
  ids.slice(1).map((id, i) => ({ from: id, to: ids[i]! }));

/** Small graphs like the report's, and the ones around it. */
const SMALL: Record<string, [LayoutNode[], LayoutEdge[]]> = {
  // One labelled item and four unlabelled missing works it cites.
  'item and four ghosts': [
    [
      { id: 'z', year: 2025, box: labelled() },
      ...['a', 'b', 'c', 'd'].map((id) => ({ id, box: GHOST })),
    ],
    ['a', 'b', 'c', 'd'].map((to) => ({ from: 'z', to })),
  ],
  // Five items of one year: no time pull, and wide label boxes stack.
  'five of one year': [
    ['a', 'b', 'c', 'd', 'e'].map((id) => ({ id, year: 2020, box: WIDE })),
    [
      { from: 'b', to: 'a' },
      { from: 'c', to: 'a' },
      { from: 'd', to: 'a' },
      { from: 'e', to: 'b' },
    ],
  ],
  'a chain of five years': [
    ['a', 'b', 'c', 'd', 'e'].map((id, i) => ({ id, year: 2000 + i, box: labelled('Abc00') })),
    chain(['a', 'b', 'c', 'd', 'e']),
  ],
  'a pair': [
    [
      { id: 'a', year: 2000, box: labelled('Old00') },
      { id: 'b', year: 2001, box: labelled('New01') },
    ],
    [{ from: 'b', to: 'a' }],
  ],
};

/** Hundreds of nodes around three hubs, as testFixtures' bigGraph. */
function large(count: number): [LayoutNode[], LayoutEdge[]] {
  const nodes: LayoutNode[] = [];
  const edges: LayoutEdge[] = [];
  for (let i = 0; i < count; i++) {
    nodes.push({ id: `n${i}`, year: 2000 + (i % 20), box: labelled('Aut04a') });
    if (i >= 3) {
      edges.push({ from: `n${i}`, to: 'n0' });
      if (i % 2 === 0) edges.push({ from: `n${i}`, to: 'n1' });
      if (i % 3 === 0) edges.push({ from: `n${i}`, to: 'n2' });
    }
  }
  return [nodes, edges];
}

describe('spreadFor', () => {
  const reach: Reach = { left: 9, right: 45, top: 7, bottom: 7 };

  it('fills a large room with a small cluster, up to the cap', () => {
    // 100 x 100 of centres in a 1636 x 1228 room: both axes hit the cap.
    const s = spreadFor({ minX: 0, minY: 0, maxX: 100, maxY: 100 }, reach, roomOf(BIG));
    expect(s).toEqual({ x: MAX_SPREAD, y: MAX_SPREAD });
    // Just short of the cap, exactly filling.
    const t = spreadFor({ minX: 0, minY: 0, maxX: 500, maxY: 500 }, reach, roomOf(BIG));
    expect(500 * t.x + reach.left + reach.right).toBeCloseTo(roomOf(BIG).width, 6);
    expect(500 * t.y + reach.top + reach.bottom).toBeCloseTo(roomOf(BIG).height, 6);
  });

  it('caps the skew between the axes', () => {
    // A column 40 wide and 330 tall, the report's shape.
    const s = spreadFor({ minX: 0, minY: 0, maxX: 40, maxY: 330 }, reach, roomOf(BIG));
    expect(s.x / s.y).toBeLessThanOrEqual(MAX_SKEW + 1e-9);
    expect(s.y / s.x).toBeLessThanOrEqual(MAX_SKEW + 1e-9);
    expect(Math.max(s.x, s.y)).toBeLessThanOrEqual(MAX_SPREAD);
    // A row that already fills the width may still grow down, but only so far.
    const row = spreadFor({ minX: 0, minY: 0, maxX: 2000, maxY: 50 }, reach, roomOf(BIG));
    expect(row).toEqual({ x: 1, y: MAX_SKEW });
  });

  it('leaves a graph that fills the room already as it is', () => {
    expect(spreadFor({ minX: 0, minY: 0, maxX: 3000, maxY: 2000 }, reach, roomOf(BIG))).toEqual(
      NO_SPREAD
    );
    expect(spreadFor(null, reach, roomOf(BIG))).toEqual(NO_SPREAD);
    // One node, or one row: nothing to skew.
    expect(spreadFor({ minX: 5, minY: 5, maxX: 5, maxY: 5 }, reach, roomOf(BIG))).toEqual(
      NO_SPREAD
    );
    const flat = spreadFor({ minX: 0, minY: 0, maxX: 100, maxY: 0 }, reach, roomOf(BIG));
    expect(flat.x).toBe(flat.y);
  });
});

describe('positions in layout units', () => {
  it('round-trip through any spread', () => {
    const spreads: Spread[] = [NO_SPREAD, { x: 4, y: 1.6 }, { x: 1.2, y: 3 }];
    const p = { x: 123.25, y: -47.5 };
    for (const s of spreads) {
      const back = unspreadPoint(spreadPoint(p, s), s);
      expect(back.x).toBeCloseTo(p.x, 9);
      expect(back.y).toBeCloseTo(p.y, 9);
    }
  });

  it('keep a dragged node with the rest when the canvas changes', () => {
    // A drag under one spread, kept unspread, drawn under another: it sits
    // in the same place relative to the layout's own points.
    const [nodes, edges] = SMALL['five of one year']!;
    const layout = layoutGraph(nodes, edges, { aspect: 1.3 });
    const reach = reachOf(nodes.map((n) => n.box!));
    const before = spreadFor(boundsOf(layout.points), reach, roomOf(BIG));
    const after = spreadFor(boundsOf(layout.points), reach, roomOf({ width: 900, height: 700 }));
    expect(after).not.toEqual(before);
    const anchor = layout.points[0]!;
    const onScreen = spreadPoint({ x: anchor.x + 30, y: anchor.y - 20 }, before);
    const kept = unspreadPoint({ x: onScreen.x + 60, y: onScreen.y + 15 }, before);
    const redrawn = spreadPoint(kept, after);
    const anchorRedrawn = spreadPoint(anchor, after);
    expect((redrawn.x - anchorRedrawn.x) / after.x).toBeCloseTo(30 + 60 / before.x, 9);
    expect((redrawn.y - anchorRedrawn.y) / after.y).toBeCloseTo(-20 + 15 / before.y, 9);
  });
});

describe('fitTransform', () => {
  it('centres the extent and never zooms past 1', () => {
    const t = fitTransform({ minX: 0, minY: 0, maxX: 100, maxY: 50 }, BIG, PAD, MIN_ZOOM);
    expect(t.k).toBe(1);
    expect(t.x + 50).toBeCloseTo(PAD.left + roomOf(BIG).width / 2, 9);
    expect(t.y + 25).toBeCloseTo(PAD.top + roomOf(BIG).height / 2, 9);
    const big = fitTransform({ minX: 0, minY: 0, maxX: 6000, maxY: 100 }, BIG, PAD, MIN_ZOOM);
    expect(big.k).toBeCloseTo(roomOf(BIG).width / 6000, 9);
  });
});

describe('a small graph in a big canvas', () => {
  for (const [name, [nodes, edges]] of Object.entries(SMALL)) {
    it(`${name}: fills more of it, within the caps`, () => {
      const before = occupancy(nodes, edges, BIG, false);
      const after = occupancy(nodes, edges, BIG, true);
      expect(after.k).toBe(1);
      expect(after.area).toBeGreaterThan(before.area * 2);
      const { x, y } = after.spread;
      expect(Math.max(x, y)).toBeLessThanOrEqual(MAX_SPREAD);
      expect(Math.max(x / y, y / x)).toBeLessThanOrEqual(MAX_SKEW + 1e-9);
      // Filling: either an axis reaches the room's edge, or the spread is
      // at its cap and the drawing stops short on purpose.
      const filled = Math.max(after.across, after.down) > 0.97;
      const capped =
        Math.max(x, y) >= MAX_SPREAD - 1e-9 || Math.min(x, y) * MAX_SKEW <= Math.max(x, y) + 1e-9;
      expect(filled || capped).toBe(true);
      // Edges at most MAX_SPREAD times what the layout made them.
      for (const e of edges) {
        const i = nodes.findIndex((n) => n.id === e.from);
        const j = nodes.findIndex((n) => n.id === e.to);
        const laid = Math.hypot(
          after.layout.points[i]!.x - after.layout.points[j]!.x,
          after.layout.points[i]!.y - after.layout.points[j]!.y
        );
        const drawn = Math.hypot(
          after.placed[i]!.x - after.placed[j]!.x,
          after.placed[i]!.y - after.placed[j]!.y
        );
        expect(drawn).toBeLessThanOrEqual(laid * MAX_SPREAD + 1e-6);
      }
    });
  }

  it('is no longer a column: five of one year comes out wider than tall', () => {
    const [nodes, edges] = SMALL['five of one year']!;
    const room = roomOf(BIG);
    const plain = boundsOf(layoutGraph(nodes, edges).points)!;
    const leaning = boundsOf(
      layoutGraph(nodes, edges, { aspect: room.width / room.height }).points
    )!;
    const ratio = (b: Bounds) => (b.maxX - b.minX) / (b.maxY - b.minY);
    expect(ratio(plain)).toBeLessThan(1);
    expect(ratio(leaning)).toBeGreaterThan(1);
  });
});

describe('a large graph', () => {
  it('is left as it was where it overflows: no spread, no lean, zoomed out by Fit', () => {
    const [nodes, edges] = large(300);
    const canvas = { width: 1100, height: 750 };
    const room = roomOf(canvas);
    const plain = layoutGraph(nodes, edges);
    const leaning = layoutGraph(nodes, edges, { aspect: room.width / room.height });
    expect(leaning.points).toEqual(plain.points);
    const after = occupancy(nodes, edges, canvas, true);
    expect(after.spread).toEqual(NO_SPREAD);
    expect(after.k).toBeLessThan(1);
  });
});

/** The layout's promises: linked papers sit closer than unlinked ones, no
 * two node-and-label boxes overlap at zoom 1, separate components are
 * packed side by side without overlapping, time leans left to right
 * without being enforced, crossings only go down, and the same graph lays
 * out the same way every time. Plus the greedy label placement, the alpha labels
 * and their per-view collision suffixes. */
import { describe, expect, it } from 'vitest';
import {
  countCrossings,
  LABEL_GAP,
  LABEL_HEIGHT,
  labelWidth,
  layoutGraph,
  nodeLabel,
  placeLabels,
  uniqueLabels,
  untangle,
} from './graphLayout';
import type { GraphLayout, LayoutEdge, LayoutNode, Point } from './graphLayout';

const RADIUS = 5;
/** A node drawn as the view draws it: a dot and an alpha label. */
const node = (id: string, label = 'ABC+24a', year: number | null = null): LayoutNode => ({
  id,
  year,
  box: {
    left: RADIUS + 4,
    right: RADIUS + LABEL_GAP + labelWidth(label),
    height: LABEL_HEIGHT,
  },
});

const boxOf = (layout: GraphLayout, nodes: LayoutNode[], i: number) => {
  const p = layout.points[i]!;
  const b = nodes[i]!.box!;
  return {
    left: p.x - b.left,
    right: p.x + b.right,
    top: p.y - b.height / 2,
    bottom: p.y + b.height / 2,
  };
};
type Box = ReturnType<typeof boxOf>;
const overlap = (a: Box, b: Box) =>
  a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

function expectNoOverlaps(layout: GraphLayout, nodes: LayoutNode[]) {
  for (let i = 0; i < nodes.length; i++) {
    const a = boxOf(layout, nodes, i);
    expect(a.left).toBeGreaterThanOrEqual(0);
    expect(a.top).toBeGreaterThanOrEqual(0);
    expect(a.right).toBeLessThanOrEqual(layout.width + 1e-6);
    expect(a.bottom).toBeLessThanOrEqual(layout.height + 1e-6);
    for (let j = i + 1; j < nodes.length; j++) {
      expect(overlap(a, boxOf(layout, nodes, j)), `${nodes[i]!.id} / ${nodes[j]!.id}`).toBe(false);
    }
  }
}

/** Twelve papers: three hubs, each cited by some of nine others. */
function twelve(): { nodes: LayoutNode[]; edges: LayoutEdge[] } {
  const nodes = Array.from({ length: 12 }, (_, i) => node(`n${i}`));
  const edges: LayoutEdge[] = [];
  for (let i = 3; i < 12; i++) {
    edges.push({ from: `n${i}`, to: 'n0' });
    if (i % 2 === 0) edges.push({ from: `n${i}`, to: 'n1' });
    if (i % 3 === 0) edges.push({ from: `n${i}`, to: 'n2' });
  }
  return { nodes, edges };
}

const dist = (layout: GraphLayout, i: number, j: number) =>
  Math.hypot(layout.points[i]!.x - layout.points[j]!.x, layout.points[i]!.y - layout.points[j]!.y);

describe('layoutGraph', () => {
  it('keeps every node and label clear of the others at zoom 1', () => {
    const { nodes, edges } = twelve();
    const layout = layoutGraph(nodes, edges);
    expectNoOverlaps(layout, nodes);
    // And it is compact: a 640 × 480 canvas holds it at zoom 1.
    expect(layout.width).toBeLessThan(560);
    expect(layout.height).toBeLessThan(400);
  });

  it('puts linked papers closer than unlinked ones', () => {
    // A path a–b–c–d–e–f: neighbours sit closer than the ends.
    const nodes = ['a', 'b', 'c', 'd', 'e', 'f'].map((id) => node(id));
    const edges = [0, 1, 2, 3, 4].map((i) => ({ from: nodes[i + 1]!.id, to: nodes[i]!.id }));
    const layout = layoutGraph(nodes, edges);
    for (let i = 0; i < 5; i++) expect(dist(layout, i, i + 1)).toBeLessThan(dist(layout, 0, 5));
    expect(dist(layout, 0, 1)).toBeLessThan(dist(layout, 0, 3));
  });

  it('packs separate components side by side without overlap', () => {
    // Three triangles and a pair, nothing between them.
    const nodes = Array.from({ length: 11 }, (_, i) => node(`n${i}`));
    const tri = (a: number) => [
      { from: `n${a}`, to: `n${a + 1}` },
      { from: `n${a + 1}`, to: `n${a + 2}` },
      { from: `n${a + 2}`, to: `n${a}` },
    ];
    const edges = [...tri(0), ...tri(3), ...tri(6), { from: 'n9', to: 'n10' }];
    const layout = layoutGraph(nodes, edges);
    expectNoOverlaps(layout, nodes);
    // Packed, not flung: the whole fits in a modest box.
    expect(layout.width).toBeLessThan(700);
    expect(layout.height).toBeLessThan(400);
    // Each component's members are nearer each other than to the others'.
    expect(dist(layout, 0, 1)).toBeLessThan(dist(layout, 0, 9));
  });

  it('is deterministic, and finite for a large graph', () => {
    const nodes = Array.from({ length: 150 }, (_, i) => node(`n${i}`, `X${i}`));
    const edges = nodes.slice(1).map((n, i) => ({ from: n.id, to: nodes[(i * 3) % 40]!.id }));
    const first = layoutGraph(nodes, edges);
    expect(layoutGraph(nodes, edges)).toEqual(first);
    for (const p of first.points) expect(Number.isFinite(p.x) && Number.isFinite(p.y)).toBe(true);
    expectNoOverlaps(first, nodes);
  });

  it('ignores self-citations, duplicates and edges to unknown ids', () => {
    const nodes = [node('a'), node('b')];
    const plain = layoutGraph(nodes, [{ from: 'b', to: 'a' }]);
    const noisy = layoutGraph(nodes, [
      { from: 'b', to: 'a' },
      { from: 'a', to: 'a' },
      { from: 'b', to: 'zz' },
      { from: 'a', to: 'b' },
    ]);
    expect(noisy).toEqual(plain);
  });

  it('handles an empty set and a single paper', () => {
    expect(layoutGraph([], [])).toEqual({
      points: [],
      width: 0,
      height: 0,
      crossings: { before: 0, after: 0 },
    });
    const lone = layoutGraph([{ id: 'a', box: { left: 4, right: 40, height: 14 } }], []);
    expect(lone.points).toHaveLength(1);
    expect(lone.width).toBeCloseTo(44 + 12);
    expect(lone.height).toBeCloseTo(14 + 8);
  });
});

describe('time bias', () => {
  const at = (layout: GraphLayout, nodes: LayoutNode[], id: string) =>
    layout.points[nodes.findIndex((n) => n.id === id)]!;

  it('reads oldest to newest when the links agree with time', () => {
    // Four years, three papers each; each paper cites one of the year before.
    const years = [1997, 2003, 2008, 2015];
    const nodes = years.flatMap((year, y) =>
      [0, 1, 2].map((k) => node(`y${y}k${k}`, 'Abc99a', year))
    );
    const edges = years
      .slice(1)
      .flatMap((_, y) =>
        [0, 1, 2].map((k) => ({ from: `y${y + 1}k${k}`, to: `y${y}k${(k + y) % 3}` }))
      );
    const layout = layoutGraph(nodes, edges);
    const meanX = years.map(
      (_, y) => [0, 1, 2].reduce((sum, k) => sum + at(layout, nodes, `y${y}k${k}`).x, 0) / 3
    );
    for (let y = 1; y < years.length; y++) expect(meanX[y]).toBeGreaterThan(meanX[y - 1]!);
    expectNoOverlaps(layout, nodes);
    expect(layoutGraph(nodes, edges)).toEqual(layout);
  });

  it('is a bias, not a rule: links can pull a paper out of order', () => {
    // An old clique, a new paper citing all of it, and a chain of newer
    // papers running away from one corner of the clique.
    const nodes = [
      ...['a', 'b', 'c', 'd'].map((id) => node(id, 'Old00', 2000)),
      node('x', 'New20', 2020),
      node('p', 'Mid05', 2005),
      node('q', 'Mid10', 2010),
      node('r', 'Mid15', 2015),
      node('y', 'New19', 2019),
    ];
    const clique = [
      ['a', 'b'],
      ['a', 'c'],
      ['a', 'd'],
      ['b', 'c'],
      ['b', 'd'],
      ['c', 'd'],
    ].map(([from, to]) => ({ from: from!, to: to! }));
    const edges = [
      ...clique,
      ...['a', 'b', 'c', 'd'].map((to) => ({ from: 'x', to })),
      { from: 'p', to: 'a' },
      { from: 'q', to: 'p' },
      { from: 'r', to: 'q' },
      { from: 'y', to: 'r' },
    ];
    const layout = layoutGraph(nodes, edges);
    // x is the newest, yet sits left of y, which only its chain holds.
    expect(at(layout, nodes, 'x').x).toBeLessThan(at(layout, nodes, 'y').x);
    expectNoOverlaps(layout, nodes);
  });
});

describe('crossings', () => {
  const small = { left: 9, right: 30, height: 14 };
  const pts = (list: [number, number][]): Point[] => list.map(([x, y]) => ({ x, y }));

  it('counts proper crossings between edges with no shared end', () => {
    const square = pts([
      [0, 0],
      [100, 100],
      [0, 100],
      [100, 0],
    ]);
    expect(
      countCrossings(square, [
        [0, 1],
        [2, 3],
      ])
    ).toBe(1);
    expect(
      countCrossings(square, [
        [0, 2],
        [1, 3],
      ])
    ).toBe(0);
    // Sharing an end is not a crossing.
    expect(
      countCrossings(square, [
        [0, 1],
        [0, 3],
      ])
    ).toBe(0);
  });

  it('untangles a twisted ladder to no crossings', () => {
    // A 2 × 4 ladder (planar) with two rungs' ends swapped top for bottom.
    const points = pts([
      [0, 0],
      [100, 80],
      [200, 0],
      [300, 80],
      [0, 80],
      [100, 0],
      [200, 80],
      [300, 0],
    ]);
    const edges: [number, number][] = [
      [0, 1],
      [1, 2],
      [2, 3],
      [4, 5],
      [5, 6],
      [6, 7],
      [0, 4],
      [1, 5],
      [2, 6],
      [3, 7],
    ];
    const boxes = points.map(() => small);
    const before = countCrossings(points, edges);
    const result = untangle(points, edges, boxes, 60);
    expect(result.before).toBe(before);
    expect(before).toBeGreaterThan(0);
    expect(result.after).toBe(0);
    expect(countCrossings(result.points, edges)).toBe(0);
    expect(untangle(points, edges, boxes, 60)).toEqual(result);
  });

  it('never adds crossings, and keeps boxes clear', () => {
    // Two chains zig-zagging through each other.
    const points = pts(
      Array.from({ length: 10 }, (_, i): [number, number] => [
        Math.floor(i / 2) * 90,
        (i + Math.floor(i / 2)) % 2 === 0 ? 0 : 70,
      ])
    );
    const edges: [number, number][] = [];
    for (let i = 0; i + 2 < 10; i++) edges.push([i, i + 2]);
    const boxes = points.map(() => small);
    const result = untangle(points, edges, boxes, 50);
    expect(result.after).toBeLessThanOrEqual(result.before);
    expect(countCrossings(result.points, edges)).toBe(result.after);
  });

  it('lays a planar graph out without crossings', () => {
    // A wheel: a hub and a ring of seven, drawable with nothing crossing.
    // (Not every planar graph comes out planar: a force layout of the
    // cube settles into its two-crossing projection, which no single move
    // undoes.)
    const nodes = Array.from({ length: 8 }, (_, i) => node(`v${i}`, 'Whe08', 2000 + (i % 4)));
    const pairs = [
      ...[1, 2, 3, 4, 5, 6, 7].map((i) => [0, i]),
      ...[1, 2, 3, 4, 5, 6, 7].map((i) => [i, (i % 7) + 1]),
    ];
    const edges = pairs.map(([a, b]) => ({ from: `v${a}`, to: `v${b}` }));
    const layout = layoutGraph(nodes, edges);
    expect(layout.crossings.after).toBeLessThanOrEqual(layout.crossings.before);
    expect(layout.crossings.after).toBe(0);
    expect(countCrossings(layout.points, pairs as [number, number][])).toBe(0);
    expectNoOverlaps(layout, nodes);
  });
});

describe('placeLabels', () => {
  const box = (id: string, x: number, force = false) => ({
    id,
    x,
    y: 0,
    width: 50,
    height: 12,
    force,
  });

  it('skips a label that overlaps one already placed', () => {
    expect([...placeLabels([box('a', 0), box('b', 20), box('c', 60)])]).toEqual(['a', 'c']);
  });

  it('always shows forced labels, and others make way for them', () => {
    const shown = placeLabels([box('a', 0), box('b', 20, true), box('c', 30, true)]);
    expect(shown.has('b')).toBe(true);
    expect(shown.has('c')).toBe(true);
    expect(shown.has('a')).toBe(false);
  });
});

describe('nodeLabel and uniqueLabels', () => {
  const row = (id: string, alphaLabel: string, year: number | null, title = id) => ({
    id,
    alphaLabel,
    year,
    title,
  });

  it('is the engine’s alpha label, or a stand-in', () => {
    expect(nodeLabel({ alphaLabel: 'Jar97' })).toBe('Jar97');
    expect(nodeLabel({ alphaLabel: ' ' })).toBe('Anon');
  });

  it('leaves unique labels alone', () => {
    const labels = uniqueLabels([row('a', 'Jar97', 1997), row('b', 'ZD24', 2024)]);
    expect([...labels]).toEqual([
      ['a', 'Jar97'],
      ['b', 'ZD24'],
    ]);
  });

  it('suffixes repeats a, b, c… by year then title', () => {
    const labels = uniqueLabels([
      row('x', 'ZD24', 2024, 'Zebra dynamics'),
      row('y', 'ZD24', 2024, 'Active matter'),
      row('z', 'ZD24', 2024, 'Middle'),
      row('u', 'Kub', null, 'B'),
      row('v', 'Kub', null, 'A'),
      row('w', 'Jar97', 1997),
    ]);
    expect(labels.get('y')).toBe('ZD24a');
    expect(labels.get('z')).toBe('ZD24b');
    expect(labels.get('x')).toBe('ZD24c');
    expect(labels.get('v')).toBe('Kuba');
    expect(labels.get('u')).toBe('Kubb');
    expect(labels.get('w')).toBe('Jar97');
  });

  it('orders by year first, no year last, and runs past z', () => {
    const many = Array.from({ length: 28 }, (_, i) => row(`r${i}`, 'ABC+', 2000 + i));
    many.push(row('none', 'ABC+', null));
    const labels = uniqueLabels(many);
    expect(labels.get('r0')).toBe('ABC+a');
    expect(labels.get('r25')).toBe('ABC+z');
    expect(labels.get('r26')).toBe('ABC+aa');
    expect(labels.get('r27')).toBe('ABC+ab');
    expect(labels.get('none')).toBe('ABC+ac');
  });
});

/** Where the citation graph's nodes go, and which labels fit. Pure, so the
 * view only draws.
 *
 * A force-directed layout, sized around the labels. Every node carries a
 * box: its dot, and its label to the right of it. Within a connected
 * component, all nodes push each other apart, every citation is a spring,
 * and a gentle pull toward the component's centre keeps it together. Time
 * is a bias, not a rule: each dated node is pulled weakly toward an x set
 * by its year's rank among the component's years (ranks, not years, so one
 * old outlier does not squash the rest), so the drawing reads roughly
 * oldest to newest left to right while links can still pull a paper out of
 * order. Undated nodes feel no time pull. Once
 * the drawing has mostly settled, a collision step pushes apart any two
 * boxes that overlap (plus padding), so at zoom 1 every label can be shown
 * without touching another or another's dot. Components are laid out one
 * by one and packed side by side on shelves, largest first, rather than
 * flung apart by repulsion.
 *
 * Nothing here is random: each component starts on a golden-angle spiral
 * in breadth-first order from its best-linked node, stretched toward the
 * time targets, so the same graph lays
 * out the same way on every reload and in every test. The round budget
 * shrinks for large components, so a few hundred nodes still take a few
 * tens of milliseconds.
 *
 * A node may be pinned: it starts at its component's origin and no force,
 * collision or untangling move takes it anywhere else, so the rest arrange
 * around it. The focus of a neighbourhood graph is pinned, which keeps it
 * in the middle of its drawing.
 *
 * Positions come back in layout space: pixels at zoom 1, with every box
 * inside [0, width] × [0, height]. The view frames that. */
import type { PaperRow } from '../../types/engine';

export interface Point {
  x: number;
  y: number;
}

export interface LayoutEdge {
  from: string;
  to: string;
}

/** How far a node's drawing reaches from its centre, in pixels at zoom 1:
 * `left` of it (the dot), `right` of it (the dot, a gap and the label),
 * and its full `height`. */
export interface NodeBox {
  left: number;
  right: number;
  height: number;
}

export interface LayoutNode {
  id: string;
  /** Pulls the node gently toward its year's place, oldest left. */
  year?: number | null;
  box?: NodeBox;
  /** Stays at its component's origin; the others move around it. */
  pinned?: boolean;
}

export interface GraphLayout {
  /** One per node, in the order given: the dot's centre. */
  points: Point[];
  /** The extent of every box, padding included. */
  width: number;
  height: number;
  /** Edge crossings after the forces, and after untangling; equal when
   * the untangling stage was skipped. */
  crossings: { before: number; after: number };
}

export interface LayoutOptions {
  /** Rounds of the simulation for a component of up to BUDGET_NODES. */
  iterations?: number;
}

/** The label font is 11px monospace; this is a little over its advance,
 * so an estimate never undershoots. Shared with the view. */
export const LABEL_CHAR = 7;
export const LABEL_HEIGHT = 14;
/** Between a dot's rim and its label. */
export const LABEL_GAP = 5;

/** A label's width at zoom 1, estimated from its length. */
export const labelWidth = (text: string) => text.length * LABEL_CHAR;

export const DEFAULT_ITERATIONS = 300;
/** Components larger than this get proportionally fewer rounds. */
const BUDGET_NODES = 60;
const MIN_ROUNDS = 40;
const DEFAULT_BOX: NodeBox = { left: 8, right: 8, height: LABEL_HEIGHT };
/** Clear space kept between two boxes, across and down. */
export const PAD_X = 12;
export const PAD_Y = 8;
/** Between packed components. */
const COMPONENT_GAP = 36;
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));
const GRAVITY = 0.06;
/** Components up to this size are laid out from STARTS seeds, and the one
 * with the fewest crossings is kept. */
const MULTI_START_NODES = 24;
const STARTS = 4;
/** The time pull, against an edge spring's 1 at its ideal length: enough
 * to read left to right, too weak to tear a cluster apart. */
const TIME = 0.2;
/** The span of the time targets per √node, in ideal edge lengths. */
const TIME_SPREAD = 1.3;
/** The collision step joins in for the last share of the rounds. */
const COLLIDE_FROM = 0.5;
const SETTLE_SWEEPS = 200;
/** Shelves aim for a drawing about this much wider than tall. */
const ASPECT = 1.6;
/** Room over the ideal shelf width, so components of similar size pair up. */
const SHELF_SLACK = 1.3;

/** Edges naming an id not in the list, and self-citations, are ignored;
 * a citation running both ways counts once. */
export function layoutGraph(
  nodes: LayoutNode[],
  edges: LayoutEdge[],
  options: LayoutOptions = {}
): GraphLayout {
  const n = nodes.length;
  if (n === 0) return { points: [], width: 0, height: 0, crossings: { before: 0, after: 0 } };
  const index = new Map<string, number>();
  nodes.forEach((node, i) => index.set(node.id, i));
  const boxes = nodes.map((node) => node.box ?? DEFAULT_BOX);
  const years = nodes.map((node) => node.year ?? null);
  const pinned = nodes.map((node) => node.pinned === true);

  const adjacent: number[][] = nodes.map(() => []);
  const links: [number, number][] = [];
  const seen = new Set<number>();
  for (const edge of edges) {
    const a = index.get(edge.from);
    const b = index.get(edge.to);
    if (a === undefined || b === undefined || a === b) continue;
    const key = a < b ? a * n + b : b * n + a;
    if (seen.has(key)) continue;
    seen.add(key);
    adjacent[a]!.push(b);
    adjacent[b]!.push(a);
    links.push(a < b ? [a, b] : [b, a]);
  }
  links.sort((p, q) => p[0] - q[0] || p[1] - q[1]);

  const xs = new Float64Array(n);
  const ys = new Float64Array(n);
  const rounds = options.iterations ?? DEFAULT_ITERATIONS;
  const crossings = { before: 0, after: 0 };
  const parts = components(adjacent).map((members) => {
    const inside = new Set(members);
    const local = links.filter(([a]) => inside.has(a));
    const slice = idealLength(members, boxes);
    // Small components try a few starts and keep the least tangled; each
    // start is a different turn of the same spiral, so still repeatable.
    const starts = members.length <= MULTI_START_NODES && local.length > 2 ? STARTS : 1;
    let best: { xs: Float64Array; ys: Float64Array; before: number; after: number } | null = null;
    for (let start = 0; start < starts; start++) {
      simulate(members, links, boxes, years, xs, ys, rounds, start, pinned);
      const result = untangleIn(members, local, boxes, xs, ys, slice, pinned);
      if (!best || result.after < best.after) {
        best = { xs: xs.slice(), ys: ys.slice(), ...result };
        if (result.after === 0) break;
      }
    }
    for (const i of members) {
      xs[i] = best!.xs[i]!;
      ys[i] = best!.ys[i]!;
    }
    crossings.before += best!.before;
    crossings.after += best!.after;
    return { members, bounds: extent(members, boxes, xs, ys) };
  });

  // Shelf packing, largest first; equal sizes keep their order of discovery.
  parts.sort((a, b) => b.members.length - a.members.length || a.members[0]! - b.members[0]!);
  const area = parts.reduce((sum, p) => sum + p.bounds.width * p.bounds.height, 0);
  const widest = Math.max(...parts.map((p) => p.bounds.width));
  const shelfWidth = Math.max(widest, Math.sqrt(area * ASPECT) * SHELF_SLACK);
  let x = 0;
  let y = 0;
  let shelfHeight = 0;
  let width = 0;
  for (const part of parts) {
    const { bounds } = part;
    if (x > 0 && x + bounds.width > shelfWidth) {
      y += shelfHeight + COMPONENT_GAP;
      x = 0;
      shelfHeight = 0;
    }
    for (const i of part.members) {
      xs[i] = xs[i]! - bounds.minX + x;
      ys[i] = ys[i]! - bounds.minY + y;
    }
    x += bounds.width + COMPONENT_GAP;
    width = Math.max(width, x - COMPONENT_GAP);
    shelfHeight = Math.max(shelfHeight, bounds.height);
  }
  const points: Point[] = nodes.map((_, i) => ({ x: xs[i]!, y: ys[i]! }));
  return { points, width, height: y + shelfHeight, crossings };
}

/** Connected components, each in breadth-first order from its best-linked
 * node (lowest index on ties), components in order of their lowest index. */
function components(adjacent: number[][]): number[][] {
  const n = adjacent.length;
  const part = new Int32Array(n).fill(-1);
  const groups: number[][] = [];
  for (let i = 0; i < n; i++) {
    if (part[i] !== -1) continue;
    const members: number[] = [];
    const stack = [i];
    part[i] = groups.length;
    while (stack.length > 0) {
      const v = stack.pop()!;
      members.push(v);
      for (const w of adjacent[v]!) {
        if (part[w] === -1) {
          part[w] = groups.length;
          stack.push(w);
        }
      }
    }
    members.sort((a, b) => a - b);
    let root = members[0]!;
    for (const v of members) if (adjacent[v]!.length > adjacent[root]!.length) root = v;
    const order = [root];
    const visited = new Set([root]);
    for (let head = 0; head < order.length; head++) {
      const next = adjacent[order[head]!]!.slice().sort((a, b) => a - b);
      for (const w of next) {
        if (!visited.has(w)) {
          visited.add(w);
          order.push(w);
        }
      }
    }
    groups.push(order);
  }
  return groups;
}

/** The ideal edge length: a typical box and its padding. */
function idealLength(members: number[], boxes: NodeBox[]): number {
  let typical = 0;
  for (const i of members) typical += boxes[i]!.left + boxes[i]!.right;
  return Math.max(40, typical / members.length + 2 * PAD_X);
}

/** Fruchterman–Reingold with gravity, then box collisions. In place. */
function simulate(
  members: number[],
  links: [number, number][],
  boxes: NodeBox[],
  years: (number | null)[],
  xs: Float64Array,
  ys: Float64Array,
  budget: number,
  start = 0,
  pinned: readonly boolean[] = []
): void {
  const m = members.length;
  // The ideal edge length: a typical box and its padding.
  const ideal = idealLength(members, boxes);
  // Time targets by rank among the years present, spread evenly, so one
  // old outlier does not squash the rest; undated nodes have none.
  const distinct = [...new Set(members.map((i) => years[i]).filter((y) => y !== null))] as number[];
  distinct.sort((a, b) => a - b);
  const span = ideal * TIME_SPREAD * Math.sqrt(m);
  const rank = new Map(distinct.map((year, r) => [year, r]));
  const target = (i: number): number | null => {
    const year = years[i];
    if (year === null || year === undefined || distinct.length < 2) return null;
    return (rank.get(year)! / (distinct.length - 1) - 0.5) * span;
  };
  const targets = new Map(members.map((i) => [i, target(i)] as const));
  const turn = (start * Math.PI * 2) / STARTS;
  const flip = start % 2 === 0 ? 1 : -1;
  // Later starts take the spiral's places in a shuffled order, from a
  // fixed-seed generator: different, but the same on every run.
  const place = members.map((_, k) => k);
  if (start > 0) {
    let seed = 2654435761 * start;
    for (let k = m - 1; k > 0; k--) {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      const j = seed % (k + 1);
      [place[k], place[j]] = [place[j]!, place[k]!];
    }
  }
  members.forEach((i, index) => {
    const k = place[index]!;
    const r = ideal * 0.6 * Math.sqrt(k + 0.5);
    const angle = k * GOLDEN_ANGLE + turn;
    const t = targets.get(i);
    xs[i] = t === null || t === undefined ? r * Math.cos(angle) : t + 0.3 * r * Math.cos(angle);
    ys[i] = flip * r * Math.sin(angle);
    if (pinned[i]) {
      xs[i] = 0;
      ys[i] = 0;
    }
  });
  if (m === 1) return;
  const inside = new Set(members);
  const local = links.filter(([a]) => inside.has(a));
  const rounds =
    m <= BUDGET_NODES ? budget : Math.max(MIN_ROUNDS, Math.round((budget * BUDGET_NODES) / m));
  const dx = new Float64Array(xs.length);
  const dy = new Float64Array(ys.length);
  const hottest = ideal * 2;
  for (let round = 0; round < rounds; round++) {
    for (const i of members) {
      dx[i] = 0;
      dy[i] = 0;
    }
    // Repulsion, every pair.
    for (let p = 0; p < m; p++) {
      const i = members[p]!;
      for (let q = p + 1; q < m; q++) {
        const j = members[q]!;
        let ex = xs[i]! - xs[j]!;
        let ey = ys[i]! - ys[j]!;
        let d = Math.hypot(ex, ey);
        if (d < 0.01) {
          // Coincident: a direction from the pair's indices, not from chance.
          const angle = (i * 7 + j * 13) * GOLDEN_ANGLE;
          ex = Math.cos(angle);
          ey = Math.sin(angle);
          d = 1;
        }
        const f = (ideal * ideal) / d / d;
        dx[i] = dx[i]! + ex * f;
        dy[i] = dy[i]! + ey * f;
        dx[j] = dx[j]! - ex * f;
        dy[j] = dy[j]! - ey * f;
      }
    }
    // Springs along citations.
    for (const [a, b] of local) {
      const ex = xs[a]! - xs[b]!;
      const ey = ys[a]! - ys[b]!;
      const d = Math.hypot(ex, ey);
      if (d < 0.01) continue;
      const f = d / ideal;
      dx[a] = dx[a]! - ex * f;
      dy[a] = dy[a]! - ey * f;
      dx[b] = dx[b]! + ex * f;
      dy[b] = dy[b]! + ey * f;
    }
    // A gentle pull to the centre.
    let cx = 0;
    let cy = 0;
    for (const i of members) {
      cx += xs[i]!;
      cy += ys[i]!;
    }
    cx /= m;
    cy /= m;
    const temperature = hottest * (1 - round / rounds) + 0.5;
    for (const i of members) {
      if (pinned[i]) continue;
      const t = targets.get(i);
      const time = t === null || t === undefined ? 0 : (xs[i]! - cx - t) * TIME;
      const fx = dx[i]! - (xs[i]! - cx) * GRAVITY * Math.sqrt(m) - time;
      const fy = dy[i]! - (ys[i]! - cy) * GRAVITY * Math.sqrt(m);
      const length = Math.hypot(fx, fy);
      if (length < 1e-9) continue;
      const step = Math.min(length, temperature);
      xs[i] = xs[i]! + (fx / length) * step;
      ys[i] = ys[i]! + (fy / length) * step;
    }
    if (round >= rounds * COLLIDE_FROM) collide(members, boxes, xs, ys, pinned);
  }
  for (let sweep = 0; sweep < SETTLE_SWEEPS; sweep++) {
    if (!collide(members, boxes, xs, ys, pinned)) break;
  }
}

/** One sweep: every overlapping pair of padded boxes is pushed apart along
 * the axis that needs less, half each, or all of it by the one that is not
 * pinned. True when anything moved. */
function collide(
  members: number[],
  boxes: NodeBox[],
  xs: Float64Array,
  ys: Float64Array,
  pinned: readonly boolean[] = []
): boolean {
  let moved = false;
  const m = members.length;
  for (let p = 0; p < m; p++) {
    const i = members[p]!;
    const a = boxes[i]!;
    for (let q = p + 1; q < m; q++) {
      const j = members[q]!;
      const b = boxes[j]!;
      const ox =
        Math.min(xs[i]! + a.right, xs[j]! + b.right) -
        Math.max(xs[i]! - a.left, xs[j]! - b.left) +
        PAD_X;
      const oy =
        Math.min(ys[i]! + a.height / 2, ys[j]! + b.height / 2) -
        Math.max(ys[i]! - a.height / 2, ys[j]! - b.height / 2) +
        PAD_Y;
      if (ox <= 0 || oy <= 0) continue;
      if (pinned[i] && pinned[j]) continue;
      moved = true;
      // Each side's share of the push: half, or none for a pinned node.
      const si = pinned[i] ? 0 : pinned[j] ? 1 : 0.5;
      const sj = 1 - si;
      if (ox <= oy) {
        const ci = xs[i]! + (a.right - a.left) / 2;
        const cj = xs[j]! + (b.right - b.left) / 2;
        const sign = ci < cj || (ci === cj && i < j) ? -1 : 1;
        xs[i] = xs[i]! + sign * (ox + 0.01) * si;
        xs[j] = xs[j]! - sign * (ox + 0.01) * sj;
      } else {
        const sign = ys[i]! < ys[j]! || (ys[i] === ys[j] && i < j) ? -1 : 1;
        ys[i] = ys[i]! + sign * (oy + 0.01) * si;
        ys[j] = ys[j]! - sign * (oy + 0.01) * sj;
      }
    }
  }
  return moved;
}

/** The members' boxes' extent, with half the padding on every side. */
function extent(members: number[], boxes: NodeBox[], xs: Float64Array, ys: Float64Array) {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const i of members) {
    const box = boxes[i]!;
    minX = Math.min(minX, xs[i]! - box.left - PAD_X / 2);
    maxX = Math.max(maxX, xs[i]! + box.right + PAD_X / 2);
    minY = Math.min(minY, ys[i]! - box.height / 2 - PAD_Y / 2);
    maxY = Math.max(maxY, ys[i]! + box.height / 2 + PAD_Y / 2);
  }
  return { minX, minY, width: maxX - minX, height: maxY - minY };
}

/** A label to place: its box in screen pixels and whether it shows even
 * over another. */
export interface LabelBox {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  force?: boolean;
}

/** Greedy placement in the order given: a label shows unless its box
 * overlaps one already placed. Forced labels always show, and are placed
 * first so the others make way for them. */
export function placeLabels(boxes: LabelBox[]): Set<string> {
  const placed: LabelBox[] = [];
  const shown = new Set<string>();
  const overlaps = (a: LabelBox, b: LabelBox) =>
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
  const ordered = [...boxes.filter((b) => b.force), ...boxes.filter((b) => !b.force)];
  for (const box of ordered) {
    if (shown.has(box.id)) continue;
    if (!box.force && placed.some((p) => overlaps(box, p))) continue;
    placed.push(box);
    shown.add(box.id);
  }
  return shown;
}

/** The engine's alpha label ("Jar97", "ZD24", "ABC+19"), or a stand-in for
 * a row that has none. */
export function nodeLabel(row: Pick<PaperRow, 'alphaLabel'>): string {
  return row.alphaLabel.trim() || 'Anon';
}

/** 0 → "a", 25 → "z", 26 → "aa", … */
function suffix(n: number): string {
  let out = '';
  let k = n;
  for (;;) {
    out = String.fromCharCode(97 + (k % 26)) + out;
    if (k < 26) return out;
    k = Math.floor(k / 26) - 1;
  }
}

/** Labels unique within one view, the way alpha.bst makes them: a label
 * that repeats gets a, b, c… in order of year (no year last), then title,
 * then id. Labels that occur once are left alone. */
export function uniqueLabels(
  rows: readonly Pick<PaperRow, 'id' | 'alphaLabel' | 'year' | 'title'>[]
): Map<string, string> {
  const groups = new Map<string, Pick<PaperRow, 'id' | 'year' | 'title'>[]>();
  for (const row of rows) {
    const base = nodeLabel(row);
    const group = groups.get(base);
    if (group) group.push(row);
    else groups.set(base, [row]);
  }
  const out = new Map<string, string>();
  const cmp = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
  for (const [base, group] of groups) {
    if (group.length === 1) {
      out.set(group[0]!.id, base);
      continue;
    }
    const ordered = group.slice().sort((a, b) => {
      if (a.year !== b.year) {
        if (a.year === null) return 1;
        if (b.year === null) return -1;
        return a.year - b.year;
      }
      return cmp(a.title, b.title) || cmp(a.id, b.id);
    });
    ordered.forEach((row, i) => out.set(row.id, `${base}${suffix(i)}`));
  }
  return out;
}

/* ---- crossing reduction ---- */

/** Past this many edges in a component the untangling stage is skipped:
 * each count is quadratic in the edges. */
export const UNTANGLE_MAX_EDGES = 600;
const UNTANGLE_PASSES = 30;
/** Components up to this size also try relocating a node anywhere. */
const RELOCATE_NODES = 80;
/** Where a node may go around its neighbours' barycentre, in half-slices,
 * nearest first. */
const NUDGES: [number, number][] = [[0, 0]];
for (let ring = 1; ring <= 2; ring++) {
  for (let gx = -ring; gx <= ring; gx++) {
    for (let gy = -ring; gy <= ring; gy++) {
      if (Math.max(Math.abs(gx), Math.abs(gy)) === ring) NUDGES.push([gx, gy]);
    }
  }
}
/** Crossing tests one untangling may spend, so a large graph stays cheap
 * and the result does not depend on the machine's speed. */
const UNTANGLE_BUDGET = 2_000_000;

const orient = (ax: number, ay: number, bx: number, by: number, cx: number, cy: number) =>
  Math.sign((bx - ax) * (cy - ay) - (by - ay) * (cx - ax));

/** Whether segments p–q and r–s cross at a point inside both. */
function crosses(
  px: number,
  py: number,
  qx: number,
  qy: number,
  rx: number,
  ry: number,
  sx: number,
  sy: number
): boolean {
  const o1 = orient(px, py, qx, qy, rx, ry);
  const o2 = orient(px, py, qx, qy, sx, sy);
  const o3 = orient(rx, ry, sx, sy, px, py);
  const o4 = orient(rx, ry, sx, sy, qx, qy);
  return o1 * o2 < 0 && o3 * o4 < 0;
}

/** Straight-segment crossings between edges that share no endpoint.
 * Edges are index pairs into `points`. */
export function countCrossings(
  points: readonly Point[],
  edges: readonly [number, number][]
): number {
  let count = 0;
  for (let e = 0; e < edges.length; e++) {
    const [a, b] = edges[e]!;
    for (let f = e + 1; f < edges.length; f++) {
      const [c, d] = edges[f]!;
      if (a === c || a === d || b === c || b === d) continue;
      const p = points[a]!;
      const q = points[b]!;
      const r = points[c]!;
      const s = points[d]!;
      if (crosses(p.x, p.y, q.x, q.y, r.x, r.y, s.x, s.y)) count++;
    }
  }
  return count;
}

/** Moves nodes to reduce edge crossings, without letting boxes overlap,
 * and returns the counts before and after. Pure: the points are copied.
 * `slice` is how far apart in x two nodes may be to trade places. */
export function untangle(
  points: readonly Point[],
  edges: readonly [number, number][],
  boxes: readonly NodeBox[],
  slice: number
): { points: Point[]; before: number; after: number } {
  const xs = Float64Array.from(points, (p) => p.x);
  const ys = Float64Array.from(points, (p) => p.y);
  const members = points.map((_, i) => i);
  const { before, after } = untangleIn(members, [...edges], [...boxes], xs, ys, slice);
  return { points: members.map((i) => ({ x: xs[i]!, y: ys[i]! })), before, after };
}

/** The untangling stage, in place over one component. Three moves, each
 * kept only when it lowers the crossing count and leaves every padded box
 * clear: a barycentric sweep (nodes in x-slices take each other's heights
 * in the order of their neighbours' mean height, left to right and back),
 * swapping two nodes close in x, and moving a node to its neighbours'
 * barycentre when that is not far in x (time stays a bias). Deterministic,
 * and bounded by passes and by a budget of crossing tests. */
function untangleIn(
  members: number[],
  links: [number, number][],
  boxes: NodeBox[],
  xs: Float64Array,
  ys: Float64Array,
  slice: number,
  pinned: readonly boolean[] = []
): { before: number; after: number } {
  const E = links.length;
  let spent = 0;
  const total = (): number => {
    spent += (E * E) / 2;
    let count = 0;
    for (let e = 0; e < E; e++) {
      const [a, b] = links[e]!;
      for (let f = e + 1; f < E; f++) {
        const [c, d] = links[f]!;
        if (a === c || a === d || b === c || b === d) continue;
        if (crosses(xs[a]!, ys[a]!, xs[b]!, ys[b]!, xs[c]!, ys[c]!, xs[d]!, ys[d]!)) count++;
      }
    }
    return count;
  };
  const before = E < 2 ? 0 : total();
  if (before === 0 || E > UNTANGLE_MAX_EDGES) return { before, after: before };

  const incident = new Map<number, number[]>(members.map((i) => [i, []]));
  links.forEach(([a, b], e) => {
    incident.get(a)!.push(e);
    incident.get(b)!.push(e);
  });
  const neighbours = new Map<number, number[]>(members.map((i) => [i, []]));
  for (const [a, b] of links) {
    neighbours.get(a)!.push(b);
    neighbours.get(b)!.push(a);
  }
  /** Crossings involving any edge at the given nodes, each pair once. */
  const local = (nodes: number[]): number => {
    const mine = new Set<number>();
    for (const v of nodes) for (const e of incident.get(v)!) mine.add(e);
    spent += mine.size * E;
    let count = 0;
    for (const e of mine) {
      const [a, b] = links[e]!;
      for (let f = 0; f < E; f++) {
        if (f === e || (mine.has(f) && f < e)) continue;
        const [c, d] = links[f]!;
        if (a === c || a === d || b === c || b === d) continue;
        if (crosses(xs[a]!, ys[a]!, xs[b]!, ys[b]!, xs[c]!, ys[c]!, xs[d]!, ys[d]!)) count++;
      }
    }
    return count;
  };
  const clearOf = (i: number): boolean => {
    const a = boxes[i]!;
    for (const j of members) {
      if (j === i) continue;
      const b = boxes[j]!;
      const ox =
        Math.min(xs[i]! + a.right, xs[j]! + b.right) - Math.max(xs[i]! - a.left, xs[j]! - b.left);
      const oy =
        Math.min(ys[i]! + a.height / 2, ys[j]! + b.height / 2) -
        Math.max(ys[i]! - a.height / 2, ys[j]! - b.height / 2);
      if (ox + PAD_X > 1e-6 && oy + PAD_Y > 1e-6) return false;
    }
    return true;
  };
  const allClear = () => members.every(clearOf);

  let current = before;
  /** Whether edge e crosses any other. */
  const crossing = (e: number): boolean => {
    const [a, b] = links[e]!;
    spent += E;
    for (let f = 0; f < E; f++) {
      const [c, d] = links[f]!;
      if (f === e || a === c || a === d || b === c || b === d) continue;
      if (crosses(xs[a]!, ys[a]!, xs[b]!, ys[b]!, xs[c]!, ys[c]!, xs[d]!, ys[d]!)) return true;
    }
    return false;
  };
  /** Tries a set of moves; keeps them if they help and fit. */
  const attempt = (
    nodes: number[],
    apply: () => void,
    undo: () => void,
    whole = false
  ): boolean => {
    if (nodes.some((i) => pinned[i])) return false;
    const was = whole ? current : local(nodes);
    apply();
    const fits = whole ? allClear() : nodes.every(clearOf);
    const now = fits ? (whole ? total() : local(nodes)) : Infinity;
    if (now < was) {
      current = whole ? now : current - (was - now);
      return true;
    }
    undo();
    return false;
  };

  const byX = members.slice().sort((a, b) => xs[a]! - xs[b]! || a - b);
  for (let pass = 0; pass < UNTANGLE_PASSES && current > 0; pass++) {
    if (spent > UNTANGLE_BUDGET) break;
    let improved = false;

    // (a) Barycentric sweeps over x-slices, left to right, then back.
    const lo = xs[byX[0]!]!;
    const slices = new Map<number, number[]>();
    for (const i of byX) {
      const key = Math.floor((xs[i]! - lo) / slice);
      const list = slices.get(key);
      if (list) list.push(i);
      else slices.set(key, [i]);
    }
    const keys = [...slices.keys()].sort((a, b) => a - b);
    for (const order of [keys, keys.slice().reverse()]) {
      for (const key of order) {
        const group = slices.get(key)!.filter((i) => !pinned[i]);
        if (group.length < 2) continue;
        const saved = group.map((i) => ys[i]!);
        const slots = saved.slice().sort((a, b) => a - b);
        const bary = (i: number) => {
          const near = neighbours.get(i)!;
          if (near.length === 0) return ys[i]!;
          let sum = 0;
          for (const j of near) sum += ys[j]!;
          return sum / near.length;
        };
        const ranked = group.map((i) => ({ i, b: bary(i) })).sort((p, q) => p.b - q.b || p.i - q.i);
        if (ranked.every((r, k) => ys[r.i] === slots[k])) continue;
        const ok = attempt(
          group,
          () => ranked.forEach((r, k) => (ys[r.i] = slots[k]!)),
          () => group.forEach((i, k) => (ys[i] = saved[k]!)),
          true
        );
        if (ok) improved = true;
        if (current === 0 || spent > UNTANGLE_BUDGET) break;
      }
    }

    // (b) Swaps of nodes close in x, and moves to the barycentre.
    for (let p = 0; p < byX.length && current > 0 && spent <= UNTANGLE_BUDGET; p++) {
      const i = byX[p]!;
      for (let q = p + 1; q < byX.length; q++) {
        const j = byX[q]!;
        if (xs[j]! - xs[i]! >= slice) break;
        if (incident.get(i)!.length === 0 && incident.get(j)!.length === 0) continue;
        const [xi, yi, xj, yj] = [xs[i]!, ys[i]!, xs[j]!, ys[j]!];
        const ok = attempt(
          [i, j],
          () => {
            xs[i] = xj;
            ys[i] = yj;
            xs[j] = xi;
            ys[j] = yi;
          },
          () => {
            xs[i] = xi;
            ys[i] = yi;
            xs[j] = xj;
            ys[j] = yj;
          }
        );
        if (ok) improved = true;
        if (current === 0) break;
      }
      const near = neighbours.get(i)!;
      if (near.length === 0 || current === 0) continue;
      let bx = 0;
      let by = 0;
      for (const j of near) {
        bx += xs[j]!;
        by += ys[j]!;
      }
      bx /= near.length;
      by /= near.length;
      // The barycentre first, then a small grid around it; the first spot
      // that helps and fits wins. Never far in x, so time stays a bias.
      const [xi, yi] = [xs[i]!, ys[i]!];
      for (const [gx, gy] of NUDGES) {
        const tx = bx + gx * slice * 0.5;
        const ty = by + gy * slice * 0.5;
        if (Math.abs(tx - xi) >= slice * 1.5) continue;
        const ok = attempt(
          [i],
          () => {
            xs[i] = tx;
            ys[i] = ty;
          },
          () => {
            xs[i] = xi;
            ys[i] = yi;
          }
        );
        if (ok) {
          improved = true;
          break;
        }
      }
    }
    // (c) In a small component, a node on a crossing edge may jump beside
    // any other node (not far in x): the best spot that helps and fits.
    if (members.length <= RELOCATE_NODES) {
      for (const i of byX) {
        if (current === 0 || spent > UNTANGLE_BUDGET) break;
        if (pinned[i] || !incident.get(i)!.some(crossing)) continue;
        const [xi, yi] = [xs[i]!, ys[i]!];
        const was = local([i]);
        const step = boxes[i]!.height + PAD_Y + 2;
        let bestSpot: [number, number] | null = null;
        let bestCount = was;
        for (const j of members) {
          if (j === i) continue;
          for (const [ox, oy] of [
            [0, -step],
            [0, step],
            [-slice, 0],
            [slice, 0],
          ] as const) {
            const tx = xs[j]! + ox;
            const ty = ys[j]! + oy;
            if (Math.abs(tx - xi) >= slice * 2) continue;
            xs[i] = tx;
            ys[i] = ty;
            if (clearOf(i)) {
              const now = local([i]);
              if (now < bestCount) {
                bestCount = now;
                bestSpot = [tx, ty];
              }
            }
          }
        }
        if (bestSpot) {
          xs[i] = bestSpot[0];
          ys[i] = bestSpot[1];
          current -= was - bestCount;
          improved = true;
        } else {
          xs[i] = xi;
          ys[i] = yi;
        }
      }
    }
    if (!improved) break;
    byX.sort((a, b) => xs[a]! - xs[b]! || a - b);
  }
  return { before, after: current };
}

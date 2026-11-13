/** The citation graph of the middle column's scope: who cites whom among
 * the items of a project, or of the whole library under All items.
 *
 * One engine read gives the nodes and the edges between them (projects.md
 * §8); graphLayout.ts says where they go (a force layout spaced for the
 * labels, roughly oldest to newest left to right, untangled to cut edge
 * crossings); this draws them as inline SVG and handles the pointer and the
 * keyboard. Only items with a citation inside the set are drawn; the rest
 * are counted in one chip that leads to the list.
 *
 * Quiet by default, emphatic on request. At rest the edges are faint,
 * barely curved and without arrowheads, and every item is labelled by its
 * alpha label ("ZD24a") when the graph is small enough to label whole; in a
 * larger one only the most cited are. Hovering shows a small card and
 * a ring, nothing else. Emphasis follows the selection, from a click, the
 * keyboard, or the list: the selected item's citations come forward with
 * arrowheads, what it cites solid and what cites it dashed, its neighbours
 * are labelled, and the rest recedes. A click on the background lets the
 * emphasis go and keeps the store's selection, so the detail column stays.
 * Nothing lights up during a pan or a drag but the dragged node.
 *
 * Zoom spreads the positions, not the drawing: dots, strokes and labels
 * keep their size, so zooming in makes room and more labels fit. Zoom
 * buttons, Fit and the first arrival animate briefly, unless reduced
 * motion is asked for. A dragged node stays where it was put, across
 * sessions (graphPositions.ts), until Reset layout.
 *
 * The data is read when the view mounts, when the scope changes, and on
 * every library:changed, sequenced like the store's reads so a slow earlier
 * answer never overwrites a later one.
 *
 * Pointer handling is pointer events, like the rest of the screen
 * (pointerDrag.ts says why): a press that moves under the threshold is a
 * click; past it, on a node it moves the node, on the background it pans.
 *
 * Two extras, both off until asked for. Missing works (the Missing toggle)
 * are references several items cite that the library lacks: ghost nodes,
 * a dashed outline and no fill, unlabelled until hovered or picked, joined
 * to their citers by edges in the ordinary rest style. They take part in
 * the layout and its untangling but are never persisted. A focus (Show in
 * graph, from the detail column) narrows the graph to one item's
 * neighbourhood, one or two citation steps out, with the item pinned in
 * the middle; its dragged positions are kept under their own key, and
 * Escape or × goes back to the scope's graph.
 *
 * Hover, labels and emphasis touch only the nodes they change: each is a
 * selector or a keyed store read per node, and the layout never reruns for
 * any of them. */
import {
  createEffect,
  createMemo,
  createSelector,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  untrack,
} from 'solid-js';
import { createStore, reconcile, unwrap } from 'solid-js/store';
import { engine } from '../../lib/transport/engine';
import type { Graph, GraphEdge, MissingWork, Paper, PaperRow } from '../../types/engine';
import GraphHoverCard, { GhostHoverCard } from './GraphHoverCard';
import {
  LABEL_GAP,
  LABEL_HEIGHT,
  labelWidth,
  layoutGraph,
  nodeLabel,
  placeLabels,
  uniqueLabels,
} from './graphLayout';
import type { LabelBox, NodeBox, Point } from './graphLayout';
import { clearPositions, loadPositions, positionsKey, savePositions } from './graphPositions';
import { registerGraphControls } from './screenIntents';
import { ghostId, ghostLabel, referenceText } from './MissingWorks';
import { openPaper } from './openPaper';
import { IconClose } from '../../components/icons';
import MathText, { plainMath } from '../../components/MathText';
import { DRAG_THRESHOLD } from './pointerDrag';
import { MISSING_MAX, MISSING_MIN, projectsStore } from './projectsStore';

/** Room around the drawing when it is fitted, in pixels: the notes and the
 * controls sit along the top, the chip and the time hint along the bottom.
 * Label widths are part of what is fitted, so this is clear space. */
const PAD = { top: 44, right: 32, bottom: 36, left: 32 };
/** What the canvas is taken to be before it has been measured, or where
 * nothing can measure it. */
const FALLBACK = { width: 640, height: 480 };
const MIN_ZOOM = 0.25;
const MAX_ZOOM = 6;
/** Node radii by cited-by count within the view, on a square-root scale. */
const RADIUS_MIN = 3.5;
const RADIUS_MAX = 7.5;
/** A missing work's dot: small, whoever cites it. */
const GHOST_RADIUS = 4;
/** Every node answers clicks within this radius, however small its dot is
 * drawn. Ghosts have no fill, and an unfilled circle only takes clicks on
 * its outline, which made missing works nearly impossible to hit. */
const HIT_RADIUS = 12;
/** How far an edge bows from the straight line, as a share of its length;
 * a citation running both ways bows further so the pair separates. */
const BOW = 0.06;
const BOW_MUTUAL = 0.22;
/** The arrowhead's length, so an emphasised edge ends at the rim. */
const ARROW = 6;
/** Labels at rest in a large graph: the most cited this many. */
const TOP_LABELS = 8;
/** Up to this many connected items, every label is a candidate at rest:
 * the layout leaves room for all of them at zoom 1. */
export const LABEL_ALL = 40;
const CARD_WIDTH = 272;
const CARD_GAP = 10;
const CARD_MARGIN = 8;
/** The hover card waits this long to appear, and this long to go. */
export const SHOW_DELAY = 250;
export const HIDE_DELAY = 120;
/** Past this many edges the rest state gets fainter still. */
const DENSE_EDGES = 60;
const ZOOM_STEP = 1.5;
const TWEEN_MS = 200;
const SETTLE_MS = 400;

interface Transform {
  k: number;
  x: number;
  y: number;
}

const IDENTITY: Transform = { k: 1, x: 0, y: 0 };

const clamp = (value: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, value));
const lerp = (a: number, b: number, t: number) => a + (b - a) * t;

let instances = 0;

function reducedMotion(): boolean {
  try {
    return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
  } catch {
    return false;
  }
}

/** Calls `step` with an eased 0..1 over `ms`, one frame at a time; at once
 * with 1 under reduced motion or where there are no frames. Returns a
 * cancel. */
function tween(ms: number, step: (t: number) => void, done?: () => void): () => void {
  if (reducedMotion() || typeof requestAnimationFrame !== 'function') {
    step(1);
    done?.();
    return () => {};
  }
  let frame = 0;
  let start: number | null = null;
  const tick = (now: number) => {
    if (start === null) start = now;
    const t = Math.min(1, (now - start) / ms);
    step(1 - (1 - t) ** 3);
    if (t < 1) frame = requestAnimationFrame(tick);
    else done?.();
  };
  frame = requestAnimationFrame(tick);
  return () => cancelAnimationFrame(frame);
}

/** A press that may become a drag. Below the threshold the release is a
 * click, which the element handles as usual; past it the element captures
 * the pointer and `move` gets each position until the release. Escape,
 * a lost pointer and a blurred window all end it. */
function press(
  down: PointerEvent,
  element: Element,
  handlers: { move: (x: number, y: number) => void; end: (dragged: boolean) => void }
): void {
  if (down.button !== 0) return;
  const pointerId = down.pointerId;
  const startX = down.clientX;
  const startY = down.clientY;
  let dragging = false;
  const cleanup = () => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onLost);
    window.removeEventListener('blur', onLost);
    window.removeEventListener('keydown', onKey, true);
  };
  const finish = () => {
    cleanup();
    try {
      element.releasePointerCapture(pointerId);
    } catch {
      // Never captured, or released already.
    }
    handlers.end(dragging);
  };
  const onMove = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    if (!dragging) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_THRESHOLD) return;
      dragging = true;
      try {
        element.setPointerCapture(pointerId);
      } catch {
        // A pointer the browser no longer tracks; the moves still bubble.
      }
    }
    handlers.move(e.clientX, e.clientY);
  };
  const onUp = (e: PointerEvent) => {
    if (e.pointerId === pointerId) finish();
  };
  const onLost = () => finish();
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || !dragging) return;
    e.preventDefault();
    e.stopPropagation();
    finish();
  };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onLost);
  window.addEventListener('blur', onLost);
  window.addEventListener('keydown', onKey, true);
}

const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

export default function GraphView(props: { onMoveFocus?: (dir: -1 | 1) => void }) {
  const store = projectsStore;
  const uid = ++instances;
  const outArrow = `graph-arrow-out-${uid}`;
  const inArrow = `graph-arrow-in-${uid}`;
  const cardDomId = `graph-card-${uid}`;

  const [graph, setGraph] = createSignal<Graph | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [size, setSize] = createSignal(FALLBACK);
  const [view, setView] = createSignal<Transform>(IDENTITY);
  /** The node under the pointer or the keyboard focus. */
  const [hover, setHover] = createSignal<string | null>(null);
  const [cardId, setCardId] = createSignal<string | null>(null);
  const [dragging, setDragging] = createSignal<string | null>(null);
  const [panning, setPanning] = createSignal(false);
  /** 0 → 1 as the first layout of a scope arrives from a compact start. */
  const [settle, setSettle] = createSignal(1);
  /** The selection whose emphasis a background click let go. */
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  /** Where the user put a node, in layout space (pixels at zoom 1). */
  const [moved, setMoved] = createStore<Record<string, Point>>({});
  /** Full records read for the hover card, for the life of the view. */
  const [papers, setPapers] = createSignal<ReadonlyMap<string, Paper | null>>(new Map());
  let canvas: HTMLDivElement | undefined;
  let svg: SVGSVGElement | undefined;
  let cardEl: HTMLDivElement | undefined;

  /* ---- data ---- */
  const projectId = () => store.graphProjectId();
  const focus = () => store.state.focus;
  /** Where dragged positions are kept: per scope, and per focus item. */
  const scopeKey = () => positionsKey(store.state.scope, focus());

  let seq = 0;
  /** The next answer is the scope's first: it settles in and is fitted. */
  let arriving = true;
  /** `quiet` is a background refresh (the library changed): the graph on
   * screen stays put and nothing flips into a loading state, so a backlog
   * of small changes does not make the view jump. */
  const load = async (quiet = false) => {
    const mine = ++seq;
    const id = projectId();
    const f = focus();
    if (!quiet || !untrack(graph)) setLoading(true);
    try {
      const result = await (f
        ? engine.libraryGraph(id, { paperId: f.paperId, depth: f.depth })
        : engine.libraryGraph(id));
      if (mine !== seq) return;
      setGraph(result);
      setError(null);
      if (arriving) {
        arriving = false;
        arrive();
      }
    } catch (e) {
      if (mine !== seq) return;
      setError(store.reportError(e));
    } finally {
      if (mine === seq) setLoading(false);
    }
  };

  let requested = new Set<string>();
  let paperGen = 0;
  const fetchPaper = (id: string) => {
    if (requested.has(id)) return;
    requested.add(id);
    const gen = paperGen;
    const keep = (paper: Paper | null) => {
      if (gen !== paperGen || disposed) return;
      setPapers((map) => new Map(map).set(id, paper));
    };
    engine.libraryPaper(id).then(keep, () => keep(null));
  };

  createEffect(
    on([scopeKey, () => focus()?.depth], ([key]) => {
      stopViewTween();
      stopSettle();
      arriving = true;
      setMoved(reconcile(loadPositions(key)));
      setDismissed(null);
      dropCard();
      void load();
    })
  );
  // A new selection is emphasised even if the last one was let go.
  createEffect(
    on([() => store.state.selectedId, () => store.state.ghostKey], () => setDismissed(null), {
      defer: true,
    })
  );

  /** The records may have changed: forget them, and the graph is read
   * again. An open card asks for its record afresh. */
  // Changes arrive in bursts (the References extension rebuilding every
  // reference list sends one per document). Re-read at most every
  // REFRESH_EVERY_MS, trailing, so the graph settles instead of twitching.
  const REFRESH_EVERY_MS = 2000;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let lastRefresh = 0;
  function refreshQuietly() {
    refreshTimer = undefined;
    lastRefresh = Date.now();
    requested = new Set();
    paperGen += 1;
    setPapers(new Map());
    const open = untrack(cardId);
    if (open) fetchPaper(open);
    void load(true);
  }
  function onLibraryChanged() {
    if (refreshTimer !== undefined) return;
    const wait = Math.max(0, lastRefresh + REFRESH_EVERY_MS - Date.now());
    refreshTimer = setTimeout(refreshQuietly, wait);
  }
  onCleanup(() => clearTimeout(refreshTimer));

  let disposed = false;
  let unlisten: (() => void) | null = null;
  onMount(() => {
    void engine.onLibraryChanged(onLibraryChanged).then((off) => {
      if (disposed) off();
      else unlisten = off;
    });
    if (!canvas) return;
    const measure = () => {
      const width = canvas?.clientWidth ?? 0;
      const height = canvas?.clientHeight ?? 0;
      if (width > 0 && height > 0) setSize({ width, height });
    };
    measure();
    if (typeof ResizeObserver === 'function') {
      const observer = new ResizeObserver(measure);
      observer.observe(canvas);
      onCleanup(() => observer.disconnect());
    }
  });
  onCleanup(() => {
    disposed = true;
    seq += 1;
    unlisten?.();
    stopViewTween();
    stopSettle();
    clearTimers();
  });

  /* ---- derived ---- */
  const allNodes = () => graph()?.nodes ?? [];
  const edges = createMemo<GraphEdge[]>(() => {
    const known = new Set(allNodes().map((n) => n.id));
    return (graph()?.edges ?? []).filter(
      (e) => known.has(e.from) && known.has(e.to) && e.from !== e.to
    );
  });
  /** Missing works with a citer among the nodes, while asked for. */
  const ghosts = createMemo<MissingWork[]>(() => {
    if (!store.state.missing.on) return [];
    const known = new Set(allNodes().map((n) => n.id));
    return store.state.missingWorks.filter((w) => w.citedBy.some((id) => known.has(id)));
  });
  const ghostOf = createMemo(() => new Map(ghosts().map((w) => [ghostId(w.key), w] as const)));
  const ghostEdges = createMemo<GraphEdge[]>(() => {
    const known = new Set(allNodes().map((n) => n.id));
    return ghosts().flatMap((w) =>
      w.citedBy.filter((id) => known.has(id)).map((id) => ({ from: id, to: ghostId(w.key) }))
    );
  });
  const allEdges = createMemo(() => [...edges(), ...ghostEdges()]);
  const shown = createMemo(() => {
    const linked = new Set<string>();
    for (const e of allEdges()) {
      linked.add(e.from);
      linked.add(e.to);
    }
    // The focus is drawn even with nothing around it: it is the point.
    const f = focus()?.paperId;
    if (f) linked.add(f);
    return allNodes().filter((n) => linked.has(n.id));
  });
  const isolatedCount = () => allNodes().length - shown().length;
  const rowOf = createMemo(() => new Map(shown().map((n) => [n.id, n] as const)));
  /** Layout order: the items, then the ghosts. */
  const indexOf = createMemo(() => {
    const map = new Map(shown().map((n, i) => [n.id, i] as const));
    ghosts().forEach((w, i) => map.set(ghostId(w.key), shown().length + i));
    return map;
  });
  const labelText = createMemo(() => uniqueLabels(shown()));
  // The missing works' cited-by lists name items as the graph does.
  createEffect(() => {
    const texts = labelText();
    const labels: Record<string, string> = {};
    for (const n of allNodes()) labels[n.id] = texts.get(n.id) ?? nodeLabel(n);
    store.setGraphLabels(labels);
  });
  /** What a node's label says: its alpha label, or a ghost's count. */
  const labelFor = (id: string): string => {
    const ghost = ghostOf().get(id);
    if (ghost) return ghostLabel(ghost);
    return labelText().get(id) ?? '';
  };
  const degrees = createMemo(() => {
    const into = new Map<string, number>();
    const out = new Map<string, number>();
    for (const e of edges()) {
      into.set(e.to, (into.get(e.to) ?? 0) + 1);
      out.set(e.from, (out.get(e.from) ?? 0) + 1);
    }
    return { into, out };
  });
  const maxIn = createMemo(() => Math.max(0, ...degrees().into.values()));
  const neighbours = createMemo(() => {
    const map = new Map<string, Set<string>>();
    const add = (a: string, b: string) => {
      if (!map.has(a)) map.set(a, new Set());
      map.get(a)!.add(b);
    };
    for (const e of allEdges()) {
      add(e.from, e.to);
      add(e.to, e.from);
    }
    return map;
  });
  const mutual = createMemo(() => {
    const set = new Set(edges().map((e) => `${e.from}\u0000${e.to}`));
    return (e: GraphEdge) => set.has(`${e.to}\u0000${e.from}`);
  });

  const radius = (id: string) => {
    if (ghostOf().has(id)) return GHOST_RADIUS;
    const max = maxIn();
    if (max === 0) return RADIUS_MIN;
    return RADIUS_MIN + (RADIUS_MAX - RADIUS_MIN) * Math.sqrt((degrees().into.get(id) ?? 0) / max);
  };
  /** What a node's drawing covers around its centre: the halo to the left,
   * the dot, the gap and the label to the right. */
  const boxOf = (id: string): NodeBox => ({
    left: radius(id) + 4,
    // A ghost's label shows only on hover, over whatever is there; at rest
    // it is a dot, and the layout spaces it as one.
    right: radius(id) + (ghostOf().has(id) ? 4 : LABEL_GAP + labelWidth(labelText().get(id) ?? '')),
    height: LABEL_HEIGHT,
  });
  const layout = createMemo(() => {
    const centre = focus()?.paperId;
    return layoutGraph(
      [
        ...shown().map((n) => ({
          id: n.id,
          year: n.year,
          box: boxOf(n.id),
          pinned: n.id === centre,
        })),
        // No time pull without a year: where it sits is its citers' doing.
        ...ghosts().map((w) => ({ id: ghostId(w.key), year: w.year, box: boxOf(ghostId(w.key)) })),
      ],
      allEdges()
    );
  });
  /** Every drawn id: the items, then the ghosts. */
  const drawnIds = createMemo(() => [
    ...shown().map((n) => n.id),
    ...ghosts().map((w) => ghostId(w.key)),
  ]);
  const yearCount = createMemo(
    () => new Set(shown().flatMap((n) => (n.year === null ? [] : [n.year]))).size
  );
  const positionOf = (id: string): Point =>
    moved[id] ?? layout().points[indexOf().get(id) ?? 0] ?? { x: 0, y: 0 };
  /** Layout space before zoom and pan; `s` < 1 draws the arrival's
   * compact start, gathered toward the middle of the drawing. */
  const toPixels = (u: Point, s = 1): Point => {
    if (s >= 1) return u;
    const { width, height } = layout();
    const f = 0.3 + 0.7 * s;
    return { x: width / 2 + (u.x - width / 2) * f, y: height / 2 + (u.y - height / 2) * f };
  };
  /** Where each node rests, before zoom: what labels and Fit go by. */
  const rest = createMemo(() => {
    const map = new Map<string, Point>();
    for (const id of drawnIds()) map.set(id, toPixels(positionOf(id)));
    return map;
  });
  /** A memo of the zoom alone, so a pan (which changes only x and y)
   * moves one group transform and recomputes nothing per node. */
  const zoom = createMemo(() => view().k);
  /** Where each node is drawn, inside the panned group. */
  const screen = createMemo(() => {
    const s = settle();
    const k = zoom();
    const resting = rest();
    const map = new Map<string, Point>();
    for (const id of drawnIds()) {
      const p = s >= 1 ? resting.get(id)! : toPixels(positionOf(id), s);
      map.set(id, { x: p.x * k, y: p.y * k });
    }
    return map;
  });

  /** From the citing node's rim toward the cited node's, bowed to the
   * left of its direction; the reverse of a mutual pair bows the other
   * way because its direction is reversed. */
  const edgePath = (e: GraphEdge, arrow: boolean): string => {
    const a = screen().get(e.from);
    const b = screen().get(e.to);
    if (!a || !b) return '';
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const length = Math.hypot(dx, dy);
    if (length < 1) return '';
    const ux = dx / length;
    const uy = dy / length;
    const bow = mutual()(e) ? BOW_MUTUAL : BOW;
    const cx = (a.x + b.x) / 2 - uy * length * bow;
    const cy = (a.y + b.y) / 2 + ux * length * bow;
    const toward = (from: Point, r: number) => {
      const tx = cx - from.x;
      const ty = cy - from.y;
      const t = Math.hypot(tx, ty) || 1;
      return { x: from.x + (tx / t) * r, y: from.y + (ty / t) * r };
    };
    const start = toward(a, radius(e.from) + 1);
    const end = toward(b, radius(e.to) + (arrow ? ARROW + 1 : 1.5));
    return `M${start.x} ${start.y}Q${cx} ${cy} ${end.x} ${end.y}`;
  };

  /* ---- emphasis, hover, labels ---- */
  const isSelected = createSelector(() => store.state.selectedId);
  /** The picked missing work, as its node id, when it is drawn. */
  const ghostPicked = createMemo(() => {
    const key = store.state.ghostKey;
    const id = key ? ghostId(key) : null;
    return id && ghostOf().has(id) ? id : null;
  });
  const isGhostPicked = createSelector(ghostPicked);
  const isCentre = createSelector(() => focus()?.paperId ?? null);
  /** What the emphasis follows: a picked missing work, else the selection. */
  const focusId = createMemo(() => {
    const ghost = ghostPicked();
    if (ghost) return ghost === dismissed() ? null : ghost;
    const id = store.state.selectedId;
    if (!id || id === dismissed() || !rowOf().has(id)) return null;
    return id;
  });
  /** The hovered node, except while a pan or a drag is under way. */
  const hot = createMemo(() => (dragging() !== null || panning() ? null : hover()));
  const isHot = createSelector(hot);
  const isDragged = createSelector(dragging);
  const hasCard = createSelector(cardId);

  const [neighbourFlag, setNeighbourFlag] = createStore<Record<string, true>>({});
  createEffect(() => {
    const f = focusId();
    const flags: Record<string, true> = {};
    if (f) for (const id of neighbours().get(f) ?? []) flags[id] = true;
    setNeighbourFlag(reconcile(flags));
  });
  const focusEdges = createMemo(() => {
    const f = focusId();
    if (!f) return [];
    return allEdges()
      .filter((e) => e.from === f || e.to === f)
      .map((edge) => ({ edge, out: edge.from === f }));
  });

  /** Cited ones first, most cited first, by position in the set on ties. */
  const ranked = createMemo(() => {
    const into = degrees().into;
    const index = indexOf();
    return shown()
      .map((n) => n.id)
      .filter((id) => (into.get(id) ?? 0) > 0)
      .sort((a, b) => into.get(b)! - into.get(a)! || index.get(a)! - index.get(b)!);
  });
  const labelled = createMemo(() => {
    const k = zoom();
    const resting = rest();
    const texts = labelText();
    const boxes: LabelBox[] = [];
    const add = (id: string, force: boolean) => {
      const p = resting.get(id);
      if (!p) return;
      boxes.push({
        id,
        x: p.x * k + radius(id) + LABEL_GAP,
        y: p.y * k - LABEL_HEIGHT / 2,
        width: labelWidth(ghostOf().has(id) ? labelFor(id) : (texts.get(id) ?? '')),
        height: LABEL_HEIGHT,
        force,
      });
    };
    const h = hot();
    if (h) add(h, true);
    const selected = store.state.selectedId;
    if (selected) add(selected, true);
    const ghost = ghostPicked();
    if (ghost) add(ghost, true);
    const f = focusId();
    // Ghost neighbours stay unlabelled: only the hovered or picked one speaks.
    if (f) for (const id of neighbours().get(f) ?? []) if (!ghostOf().has(id)) add(id, true);
    const n = shown().length;
    if (n <= LABEL_ALL) {
      // Small enough to label whole: the most cited first, then the rest.
      for (const id of ranked()) add(id, false);
      for (const node of shown()) add(node.id, false);
      return placeLabels(boxes);
    }
    // Zooming in raises the quota, until everything that fits is shown.
    const quota = k <= 1 ? TOP_LABELS : TOP_LABELS + Math.round(n * Math.min(1, (k - 1) / 1.5));
    const order = ranked();
    for (const id of order.slice(0, quota)) add(id, false);
    // The uncited get a label in a graph small enough to label whole, or
    // once zoomed in; at rest in a large one they would be an arbitrary few.
    if ((n <= TOP_LABELS || k > 1) && quota > order.length) {
      for (const node of shown().slice(0, quota)) add(node.id, false);
    }
    return placeLabels(boxes);
  });
  const [labelOn, setLabelOn] = createStore<Record<string, true>>({});
  createEffect(() => {
    const flags: Record<string, true> = {};
    for (const id of labelled()) flags[id] = true;
    setLabelOn(reconcile(flags));
  });

  /* ---- hover card ---- */
  let showTimer: ReturnType<typeof setTimeout> | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  const clearTimers = () => {
    clearTimeout(showTimer);
    clearTimeout(hideTimer);
  };
  const showCard = (id: string) => {
    setCardId(id);
    if (!ghostOf().has(id)) fetchPaper(id);
  };
  const want = (id: string) => {
    if (dragging() !== null || panning()) return;
    clearTimers();
    // Already showing one: swap in place, no second wait.
    if (cardId() !== null) showCard(id);
    else showTimer = setTimeout(() => showCard(id), SHOW_DELAY);
  };
  const unwant = () => {
    clearTimeout(showTimer);
    if (cardId() === null) return;
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => setCardId(null), HIDE_DELAY);
  };
  function dropCard() {
    clearTimers();
    setCardId(null);
  }

  const [cardHeight, setCardHeight] = createSignal(96);
  createEffect(() => {
    cardId();
    papers();
    if (cardEl) setCardHeight(cardEl.offsetHeight || 96);
  });
  /** Beside the node, right if it fits and left if not, and kept inside
   * the canvas vertically. */
  const cardBox = createMemo(() => {
    const id = cardId();
    if (!id) return null;
    const row = rowOf().get(id) ?? null;
    const ghost = ghostOf().get(id) ?? null;
    const p = screen().get(id);
    if ((!row && !ghost) || !p) return null;
    const t = view();
    const { width: W, height: H } = size();
    const width = Math.min(CARD_WIDTH, W - 2 * CARD_MARGIN);
    const x = t.x + p.x;
    const y = t.y + p.y;
    const reach = radius(id) + CARD_GAP;
    let side: 'left' | 'right' = 'right';
    let left = x + reach;
    if (left + width > W - CARD_MARGIN && x - reach - width >= CARD_MARGIN) {
      side = 'left';
      left = x - reach - width;
    }
    left = clamp(left, CARD_MARGIN, Math.max(CARD_MARGIN, W - width - CARD_MARGIN));
    const top = clamp(y - 16, CARD_MARGIN, Math.max(CARD_MARGIN, H - cardHeight() - CARD_MARGIN));
    return { id, row, ghost, left, top, width, side };
  });

  /* ---- view ---- */
  let stopViewAnimation: (() => void) | null = null;
  let viewTarget: Transform | null = null;
  function stopViewTween() {
    stopViewAnimation?.();
    stopViewAnimation = null;
    viewTarget = null;
  }
  const animateView = (target: Transform) => {
    stopViewTween();
    const from = view();
    viewTarget = target;
    stopViewAnimation = tween(
      TWEEN_MS,
      (t) =>
        setView({
          k: lerp(from.k, target.k, t),
          x: lerp(from.x, target.x, t),
          y: lerp(from.y, target.y, t),
        }),
      () => {
        viewTarget = null;
        stopViewAnimation = null;
      }
    );
  };
  let stopSettleAnimation: (() => void) | null = null;
  function stopSettle() {
    stopSettleAnimation?.();
    stopSettleAnimation = null;
    setSettle(1);
  }
  /** Everything at rest, labels included, framed with the padding,
   * centred. */
  const fitTransform = (): Transform => {
    const resting = rest();
    if (resting.size === 0) return IDENTITY;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const [id, p] of resting) {
      const box = boxOf(id);
      minX = Math.min(minX, p.x - box.left);
      maxX = Math.max(maxX, p.x + box.right);
      minY = Math.min(minY, p.y - box.height / 2);
      maxY = Math.max(maxY, p.y + box.height / 2);
    }
    // Around a focus the frame is symmetric about it, so it sits in the
    // middle of the canvas.
    const centre = focus() ? resting.get(focus()!.paperId) : undefined;
    if (centre) {
      const hx = Math.max(centre.x - minX, maxX - centre.x);
      const hy = Math.max(centre.y - minY, maxY - centre.y);
      minX = centre.x - hx;
      maxX = centre.x + hx;
      minY = centre.y - hy;
      maxY = centre.y + hy;
    }
    const width = Math.max(1, maxX - minX);
    const height = Math.max(1, maxY - minY);
    const availX = Math.max(1, size().width - PAD.left - PAD.right);
    const availY = Math.max(1, size().height - PAD.top - PAD.bottom);
    // Never past 1: the layout is spaced for its labels at 1, and the
    // label quota counts zoom from there.
    const k = clamp(Math.min(availX / width, availY / height), MIN_ZOOM, 1);
    return {
      k,
      x: PAD.left + (availX - width * k) / 2 - minX * k,
      y: PAD.top + (availY - height * k) / 2 - minY * k,
    };
  };
  const fit = () => animateView(fitTransform());
  function arrive() {
    setView(fitTransform());
    stopSettleAnimation?.();
    if (reducedMotion()) {
      setSettle(1);
      return;
    }
    setSettle(0);
    stopSettleAnimation = tween(SETTLE_MS, setSettle, () => {
      stopSettleAnimation = null;
    });
  }
  const zoomBy = (factor: number) => {
    const t = viewTarget ?? view();
    const cx = size().width / 2;
    const cy = size().height / 2;
    const k = clamp(t.k * factor, MIN_ZOOM, MAX_ZOOM);
    const ratio = k / t.k;
    animateView({ k, x: cx - (cx - t.x) * ratio, y: cy - (cy - t.y) * ratio });
  };
  const hasMoved = () => Object.keys(moved).length > 0;
  const persist = () => {
    savePositions(scopeKey(), { ...unwrap(moved) }, new Set(allNodes().map((n) => n.id)));
  };
  const resetLayout = () => {
    clearPositions(scopeKey());
    setMoved(reconcile({}));
    fit();
  };
  // The palette's Fit Graph and Reset Graph Layout reach the graph on
  // screen through here.
  onCleanup(registerGraphControls({ fit, resetLayout, hasMoved }));

  /* ---- notes ---- */
  const externalNote = () => {
    const n = graph()?.external ?? 0;
    if (n === 0) return null;
    const where = projectId() === null ? 'the library' : 'this project';
    return `${plural(n, 'citation')} lead${n === 1 ? 's' : ''} outside ${where}`;
  };
  const unindexedNote = () => {
    const n = graph()?.unindexed ?? 0;
    if (n === 0) return null;
    return `${plural(n, 'item')} not indexed yet, ${n === 1 ? 'its' : 'their'} citations are not known`;
  };
  const emptyNote = () =>
    allNodes().length > 0 && edges().length === 0 ? 'No citations between these items yet' : null;
  const missingNote = () => {
    if (!store.state.missing.on) return null;
    const n = ghosts().length;
    return n === 0
      ? `Nothing missing is cited by $\\geq ${store.state.missing.min}$`
      : `${plural(n, 'missing work')} cited by $\\geq ${store.state.missing.min}$`;
  };
  const notes = createMemo(() =>
    [emptyNote(), externalNote(), unindexedNote(), missingNote()].filter(
      (t): t is string => t !== null
    )
  );

  const ariaLabel = (node: PaperRow) => {
    const parts = [`${labelText().get(node.id) ?? nodeLabel(node)}: ${node.title}`];
    if (node.missing) parts.push('file missing');
    if (node.projectIds.length > 1) parts.push(`in ${node.projectIds.length} projects`);
    return parts.join(', ');
  };

  /* ---- pointer ---- */
  /** Set by a drag's release, cleared a tick later: the click the browser
   * fires after that release must not select, and no click may follow. */
  let justDragged = false;
  const markDragged = () => {
    justDragged = true;
    setTimeout(() => {
      justDragged = false;
    }, 0);
  };
  /** True while a press focuses its node by hand: that focus is the
   * pointer's, which has its own hover, not the keyboard's. */
  let pointerFocusing = false;

  const onNodePointerDown = (node: { id: string }, e: PointerEvent) => {
    const element = e.currentTarget as SVGGElement;
    // Cancelled so WebKit begins no text selection under the drag; the
    // focus the press would have given is given by hand (pointerDrag.ts).
    e.preventDefault();
    pointerFocusing = true;
    element.focus?.({ preventScroll: true });
    pointerFocusing = false;
    let lastX = e.clientX;
    let lastY = e.clientY;
    press(e, element, {
      move: (x, y) => {
        if (dragging() === null) {
          setDragging(node.id);
          setHover(null);
          dropCard();
          stopViewTween();
          stopSettle();
        }
        const k = view().k;
        const u = positionOf(node.id);
        setMoved(node.id, { x: u.x + (x - lastX) / k, y: u.y + (y - lastY) / k });
        lastX = x;
        lastY = y;
      },
      end: (dragged) => {
        if (!dragged) return;
        markDragged();
        setDragging(null);
        persist();
      },
    });
  };

  const isBackground = (target: EventTarget | null) =>
    target === svg || (target instanceof Element && target.classList.contains('graph-backdrop'));

  const onBackgroundPointerDown = (e: PointerEvent) => {
    if (!svg || !isBackground(e.target)) return;
    e.preventDefault();
    stopViewTween();
    const start = view();
    const startX = e.clientX;
    const startY = e.clientY;
    press(e, svg, {
      move: (x, y) => {
        if (!panning()) {
          setPanning(true);
          setHover(null);
          dropCard();
        }
        setView({ k: start.k, x: start.x + (x - startX), y: start.y + (y - startY) });
      },
      end: (dragged) => {
        setPanning(false);
        // A click on nothing lets the emphasis go; the selection stays.
        if (!dragged) setDismissed(ghostPicked() ?? store.state.selectedId);
      },
    });
  };

  const onWheel = (e: WheelEvent) => {
    if (!svg) return;
    e.preventDefault();
    stopViewTween();
    const rect = svg.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const delta = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    const t = view();
    const k = clamp(t.k * Math.exp(-delta * 0.0015), MIN_ZOOM, MAX_ZOOM);
    if (k === t.k) return;
    const ratio = k / t.k;
    setView({ k, x: mx - (mx - t.x) * ratio, y: my - (my - t.y) * ratio });
  };

  const choose = (id: string) => {
    setDismissed(null);
    const ghost = ghostOf().get(id);
    if (ghost) store.selectGhost(ghost.key);
    else store.select(id);
  };
  const onNodeClick = (node: { id: string }) => {
    if (justDragged) return;
    choose(node.id);
  };
  const onNodeDblClick = (node: PaperRow) => {
    if (justDragged) return;
    void openPaper(node.id);
  };
  const onNodeKeyDown = (node: { id: string }, e: KeyboardEvent) => {
    switch (e.key) {
      case 'Enter':
        e.preventDefault();
        choose(node.id);
        break;
      case ' ':
        e.preventDefault();
        if (!ghostOf().has(node.id)) void openPaper(node.id);
        break;
      case 'ArrowLeft':
        e.preventDefault();
        props.onMoveFocus?.(-1);
        break;
      case 'ArrowRight':
        e.preventDefault();
        props.onMoveFocus?.(1);
        break;
    }
  };
  const onNodeEnter = (node: { id: string }) => {
    if (dragging() !== null || panning()) return;
    setHover(node.id);
    want(node.id);
  };
  const onNodeLeave = (node: { id: string }) => {
    setHover((h) => (h === node.id ? null : h));
    unwant();
  };
  const onNodeFocus = (node: { id: string }) => {
    if (pointerFocusing) return;
    setHover(node.id);
    want(node.id);
  };

  /** The focus item's name in the header: its label in this graph. */
  const focusLabel = () => {
    const id = focus()?.paperId;
    if (!id) return '';
    const row = allNodes().find((n) => n.id === id) ?? store.state.rows.find((r) => r.id === id);
    return labelText().get(id) ?? (row ? nodeLabel(row) : (store.state.selected?.citeKey ?? id));
  };
  const onViewKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || e.defaultPrevented || !focus()) return;
    e.preventDefault();
    store.leaveFocus();
  };

  return (
    <div class="graph-view" onKeyDown={onViewKeyDown}>
      <Show when={focus()}>
        {(f) => (
          <div class="graph-focus-bar">
            <span class="graph-focus-title">
              Around <span class="graph-focus-label">{focusLabel()}</span>
            </span>
            <div class="projects-launch-toggle graph-depth" role="radiogroup" aria-label="Steps">
              <For each={[1, 2] as const}>
                {(depth) => (
                  <button
                    type="button"
                    role="radio"
                    aria-checked={f().depth === depth}
                    classList={{ 'is-active': f().depth === depth }}
                    title={
                      depth === 1
                        ? 'What it cites and what cites it'
                        : 'And what those cite, and what cites them'
                    }
                    onClick={() => store.setFocusDepth(depth)}
                  >
                    {depth === 1 ? '1 step' : '2 steps'}
                  </button>
                )}
              </For>
            </div>
            <button
              type="button"
              class="graph-focus-leave"
              aria-label="Leave focus"
              title="Back to the whole graph (Escape)"
              onClick={() => store.leaveFocus()}
            >
              <IconClose />
            </button>
          </div>
        )}
      </Show>
      {/* Always one line, always there, so the canvas below never moves
          as notes come and go. The full text is in the tooltip. */}
      <div class="graph-notes" title={plainMath(notes().join('\n'))}>
        <span class="graph-note">
          <MathText text={error() ? '' : notes().join('; ')} />
        </span>
        <Show when={store.state.missing.on && ghosts().length > 0}>
          <button
            type="button"
            class="paper-verdict-link graph-note-link"
            onClick={() => store.showMissingList(true)}
          >
            List missing ({ghosts().length})
          </button>
        </Show>
      </div>
      <Show when={error()}>
        {(text) => (
          <div class="graph-error" role="alert">
            <span>{text()}</span>
            <button type="button" class="secondary-btn" onClick={() => void load()}>
              Retry
            </button>
          </div>
        )}
      </Show>
      <div
        ref={canvas}
        class="graph-canvas"
        classList={{
          'is-loading': loading(),
          'is-empty': !loading() && shown().length === 0,
          'has-focus': focusId() !== null,
          'has-ghosts': ghosts().length > 0,
          'is-dense': edges().length > DENSE_EDGES,
          'is-panning': panning(),
          'is-dragging': dragging() !== null,
        }}
      >
        <Show when={loading() && !graph()}>
          <svg class="graph-skeleton" viewBox="0 0 200 120" aria-busy="true" aria-label="Loading">
            <line x1="40" y1="70" x2="100" y2="50" />
            <line x1="40" y1="70" x2="160" y2="80" />
            <line x1="100" y1="50" x2="160" y2="80" />
            <circle cx="40" cy="70" r="7" />
            <circle cx="100" cy="50" r="5" />
            <circle cx="160" cy="80" r="5" />
          </svg>
        </Show>
        <Show when={graph() && shown().length > 0}>
          <svg
            ref={svg}
            class="graph-svg"
            role="group"
            aria-label="Citation graph"
            width={size().width}
            height={size().height}
            onPointerDown={onBackgroundPointerDown}
            onDblClick={(e) => {
              if (isBackground(e.target)) fit();
            }}
            onWheel={onWheel}
          >
            <defs>
              <marker
                id={outArrow}
                class="graph-arrow is-out"
                viewBox="0 0 10 10"
                refX="10"
                refY="5"
                markerWidth={ARROW}
                markerHeight={ARROW}
                markerUnits="userSpaceOnUse"
                orient="auto"
              >
                <path d="M0 0L10 5L0 10z" />
              </marker>
              <marker
                id={inArrow}
                class="graph-arrow is-in"
                viewBox="0 0 10 10"
                refX="10"
                refY="5"
                markerWidth={ARROW}
                markerHeight={ARROW}
                markerUnits="userSpaceOnUse"
                orient="auto"
              >
                <path d="M0 0L10 5L0 10z" />
              </marker>
            </defs>
            <rect class="graph-backdrop" width="100%" height="100%" />
            <g transform={`translate(${view().x} ${view().y})`}>
              <g class="graph-edges">
                <For each={edges()}>
                  {(edge) => (
                    <path
                      class="graph-edge"
                      d={edgePath(edge, false)}
                      data-from={edge.from}
                      data-to={edge.to}
                    />
                  )}
                </For>
                <For each={ghostEdges()}>
                  {(edge) => (
                    <path
                      class="graph-edge is-ghost"
                      d={edgePath(edge, false)}
                      data-from={edge.from}
                      data-to={edge.to}
                    />
                  )}
                </For>
              </g>
              <g class="graph-focus-edges">
                <For each={focusEdges()}>
                  {(lit) => (
                    <path
                      class="graph-edge-lit"
                      classList={{ 'is-out': lit.out, 'is-in': !lit.out }}
                      d={edgePath(lit.edge, true)}
                      marker-end={`url(#${lit.out ? outArrow : inArrow})`}
                      data-from={lit.edge.from}
                      data-to={lit.edge.to}
                    />
                  )}
                </For>
              </g>
              <g class="graph-nodes">
                <For each={shown()}>
                  {(node) => {
                    const r = () => radius(node.id);
                    const at = () => screen().get(node.id) ?? { x: 0, y: 0 };
                    return (
                      <g
                        class="graph-node"
                        classList={{
                          'is-selected': isSelected(node.id) && ghostPicked() === null,
                          'is-centre': isCentre(node.id),
                          'is-missing': node.missing,
                          'is-shared': node.projectIds.length > 1,
                          'is-hover': isHot(node.id),
                          'is-neighbour': neighbourFlag[node.id] === true,
                          'is-label-on': labelOn[node.id] === true,
                          'is-dragged': isDragged(node.id),
                        }}
                        role="button"
                        tabindex={0}
                        aria-label={ariaLabel(node)}
                        aria-pressed={isSelected(node.id) && ghostPicked() === null}
                        aria-describedby={hasCard(node.id) ? cardDomId : undefined}
                        data-id={node.id}
                        transform={`translate(${at().x} ${at().y})`}
                        onPointerDown={(e) => onNodePointerDown(node, e)}
                        onPointerEnter={() => onNodeEnter(node)}
                        onPointerLeave={() => onNodeLeave(node)}
                        onFocus={() => onNodeFocus(node)}
                        onBlur={() => onNodeLeave(node)}
                        onClick={() => onNodeClick(node)}
                        onDblClick={() => onNodeDblClick(node)}
                        onKeyDown={(e) => onNodeKeyDown(node, e)}
                      >
                        <circle class="graph-node-hit" r={Math.max(r() + 8, HIT_RADIUS)} />
                        <circle class="graph-node-halo" r={r() + 4} />
                        <Show when={node.projectIds.length > 1}>
                          <circle class="graph-node-ring" r={r() + 2.5} />
                        </Show>
                        <circle class="graph-node-dot" r={r()} />
                        <text class="graph-node-label" x={r() + LABEL_GAP} dy="0.35em">
                          {labelText().get(node.id) ?? nodeLabel(node)}
                        </text>
                      </g>
                    );
                  }}
                </For>
                <For each={ghosts()}>
                  {(work) => {
                    const id = ghostId(work.key);
                    const at = () => screen().get(id) ?? { x: 0, y: 0 };
                    return (
                      <g
                        class="graph-node is-ghost"
                        classList={{
                          'is-selected': isGhostPicked(id),
                          'is-hover': isHot(id),
                          'is-neighbour': neighbourFlag[id] === true,
                          'is-label-on': labelOn[id] === true,
                          'is-dragged': isDragged(id),
                        }}
                        role="button"
                        tabindex={0}
                        aria-label={`Missing: ${referenceText(work)}, cited by ${work.citedBy.length}`}
                        aria-pressed={isGhostPicked(id)}
                        aria-describedby={hasCard(id) ? cardDomId : undefined}
                        data-id={id}
                        data-ghost={work.key}
                        transform={`translate(${at().x} ${at().y})`}
                        onPointerDown={(e) => onNodePointerDown({ id }, e)}
                        onPointerEnter={() => onNodeEnter({ id })}
                        onPointerLeave={() => onNodeLeave({ id })}
                        onFocus={() => onNodeFocus({ id })}
                        onBlur={() => onNodeLeave({ id })}
                        onClick={() => onNodeClick({ id })}
                        onKeyDown={(e) => onNodeKeyDown({ id }, e)}
                      >
                        <circle class="graph-node-hit" r={HIT_RADIUS} />
                        <circle class="graph-node-halo" r={GHOST_RADIUS + 4} />
                        <circle class="graph-node-dot" r={GHOST_RADIUS} />
                        <text class="graph-node-label" x={GHOST_RADIUS + LABEL_GAP} dy="0.35em">
                          {ghostLabel(work)}
                        </text>
                      </g>
                    );
                  }}
                </For>
              </g>
            </g>
          </svg>
          <Show when={yearCount() >= 2}>
            <div class="graph-time-hint is-start" aria-hidden="true">
              <MathText text="$\leftarrow$ older" />
            </div>
            <div class="graph-time-hint is-end" aria-hidden="true">
              <MathText text="newer $\rightarrow$" />
            </div>
          </Show>
          <Show when={focusId()}>
            <div class="graph-legend" aria-hidden="true">
              <span class="graph-legend-row">
                <svg width="30" height="10" viewBox="0 0 30 10">
                  <circle class="graph-legend-dot" cx="4" cy="5" r="3" />
                  <path class="graph-legend-out" d="M9 5H24" />
                  <path class="graph-legend-head is-out" d="M23 2L29 5L23 8z" />
                </svg>
                cites
              </span>
              <span class="graph-legend-row">
                <svg width="30" height="10" viewBox="0 0 30 10">
                  <circle class="graph-legend-dot" cx="4" cy="5" r="3" />
                  <path class="graph-legend-in" d="M14 5H29" />
                  <path class="graph-legend-head is-in" d="M15 2L9 5L15 8z" />
                </svg>
                cited by
              </span>
            </div>
          </Show>
          <div class="graph-controls" role="toolbar" aria-label="Graph view">
            <button
              type="button"
              class="graph-control"
              aria-label="Zoom in"
              title="Zoom in"
              onClick={() => zoomBy(ZOOM_STEP)}
            >
              <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                <path d="M6 2v8M2 6h8" />
              </svg>
            </button>
            <button
              type="button"
              class="graph-control"
              aria-label="Zoom out"
              title="Zoom out"
              onClick={() => zoomBy(1 / ZOOM_STEP)}
            >
              <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                <path d="M2 6h8" />
              </svg>
            </button>
            <button
              type="button"
              class="graph-control is-text"
              aria-label="Fit to view"
              title="Fit everything in view (or double-click the background)"
              onClick={fit}
            >
              Fit
            </button>
            <button
              type="button"
              class="graph-control is-text"
              title="Put dragged items back where the layout places them"
              disabled={!hasMoved()}
              onClick={resetLayout}
            >
              Reset layout
            </button>
            <button
              type="button"
              class="graph-control is-text"
              aria-pressed={store.state.missing.on}
              classList={{ 'is-on': store.state.missing.on }}
              title="Show works several items here cite that are not in the library"
              onClick={() => store.setMissing(!store.state.missing.on)}
            >
              Missing
            </button>
            <Show when={store.state.missing.on}>
              <span class="graph-control graph-stepper" role="group" aria-label="Cited by at least">
                <button
                  type="button"
                  aria-label="Fewer citers"
                  disabled={store.state.missing.min <= MISSING_MIN}
                  onClick={() => store.setMissingMin(store.state.missing.min - 1)}
                >
                  -
                </button>
                <span class="graph-stepper-value">
                  <MathText text={`cited by $\\geq ${store.state.missing.min}$`} />
                </span>
                <button
                  type="button"
                  aria-label="More citers"
                  disabled={store.state.missing.min >= MISSING_MAX}
                  onClick={() => store.setMissingMin(store.state.missing.min + 1)}
                >
                  +
                </button>
              </span>
            </Show>
          </div>
          <Show when={cardBox()}>
            {(box) => (
              <>
                <Show when={box().row}>
                  {(row) => (
                    <GraphHoverCard
                      ref={(el) => (cardEl = el)}
                      id={cardDomId}
                      row={row()}
                      label={labelText().get(row().id) ?? nodeLabel(row())}
                      paper={papers().get(row().id)}
                      cites={degrees().out.get(row().id) ?? 0}
                      citedBy={degrees().into.get(row().id) ?? 0}
                      left={box().left}
                      top={box().top}
                      width={box().width}
                      side={box().side}
                    />
                  )}
                </Show>
                <Show when={box().ghost}>
                  {(work) => (
                    <GhostHoverCard
                      ref={(el) => (cardEl = el)}
                      id={cardDomId}
                      work={work()}
                      citers={work().citedBy.map((id) => labelText().get(id) ?? id)}
                      left={box().left}
                      top={box().top}
                      width={box().width}
                      side={box().side}
                    />
                  )}
                </Show>
              </>
            )}
          </Show>
        </Show>
        <Show when={graph() && !error() && isolatedCount() > 0}>
          <button
            type="button"
            class="graph-chip"
            title="Show every item in the list"
            onClick={() => store.setView('list')}
          >
            {plural(isolatedCount(), 'item')} with no citations here
          </button>
        </Show>
        <Show when={!loading() && !error() && allNodes().length === 0}>
          <div class="paper-empty">
            <p class="library-hint">No items</p>
          </div>
        </Show>
      </div>
    </div>
  );
}

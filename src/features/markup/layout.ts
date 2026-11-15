/** Laying the exported pages out, given measured heights: the notes in a
 * page's margin, as near as they can be to what they are about, and (only
 * when a page has more to say than its margin holds) the pages of notes
 * continued at the end. Pure; all in points (CSS px == pt). */

/** The margin strip's frame, in points: notes are set `left` in from the
 * page edge, `right` short of the strip's, between `top` and the page
 * height less `bottom`, `gap` apart. */
export const MARGIN = { left: 12, right: 10, top: 12, bottom: 12, gap: 4 };

export interface Band {
  top: number;
  bottom: number;
  gap: number;
}

/** Tops for notes in a margin band, in the order given (reading order),
 * none overlapping, all inside the band: each as close as it can be to
 * where it would like to be (`desired`, level with its anchor), the total
 * squared displacement least - so notes that collide share the move, one
 * up and one down, rather than all being pushed below.
 *
 * Isotonic regression (pool adjacent violators): with `o_i` the room the
 * notes before `i` need, the tops are `o_i + z_i` for a non-decreasing `z`
 * closest to `desired_i - o_i`, clamped into the band. When the notes do
 * not fit at all they are stacked from the top and run past the bottom. */
export function placeNotes(
  desired: readonly number[],
  heights: readonly number[],
  band: Band
): number[] {
  const n = desired.length;
  if (n === 0) return [];
  const offset: number[] = [];
  let acc = 0;
  for (let i = 0; i < n; i++) {
    offset.push(acc);
    acc += heights[i]! + band.gap;
  }
  const total = acc - band.gap;
  const lo = band.top;
  const hi = band.bottom - total;
  // Pools of equal z: [sum, count].
  const pools: { sum: number; count: number }[] = [];
  for (let i = 0; i < n; i++) {
    pools.push({ sum: desired[i]! - offset[i]!, count: 1 });
    while (pools.length > 1) {
      const b = pools[pools.length - 1]!;
      const a = pools[pools.length - 2]!;
      if (a.sum / a.count <= b.sum / b.count) break;
      pools.splice(pools.length - 2, 2, { sum: a.sum + b.sum, count: a.count + b.count });
    }
  }
  const z: number[] = [];
  for (const pool of pools) {
    const value = pool.sum / pool.count;
    for (let k = 0; k < pool.count; k++) z.push(value);
  }
  return z.map((v, i) => {
    const clamped = hi < lo ? lo : Math.min(Math.max(v, lo), hi);
    return Math.round((clamped + offset[i]!) * 100) / 100;
  });
}

/* ---- pages of continued notes ---- */

export interface FlowBlock {
  height: number;
  /** A heading: never the last thing on a page. */
  keepWithNext?: boolean;
  /** A table row: when it starts a page, its table's head (this tall) is
   * drawn above it again. */
  headerHeight?: number;
}

export interface FlowPage {
  /** Indices of the blocks on the page, in order. */
  blocks: number[];
  /** The first block is a table row continued from the page before. */
  repeatHeader: boolean;
}

/** Fill pages `room` points tall with the blocks in order. A block taller
 * than a page gets one to itself (and is cut at its foot). */
export function paginate(blocks: readonly FlowBlock[], room: number): FlowPage[] {
  const pages: FlowPage[] = [];
  let current: FlowPage = { blocks: [], repeatHeader: false };
  let used = 0;
  const newPage = () => {
    if (current.blocks.length > 0) pages.push(current);
    current = { blocks: [], repeatHeader: false };
    used = 0;
  };
  const cost = (i: number, atTop: boolean) => {
    const b = blocks[i]!;
    return b.height + (atTop && b.headerHeight ? b.headerHeight : 0);
  };

  for (let i = 0; i < blocks.length; i++) {
    const atTop = current.blocks.length === 0;
    let need = cost(i, atTop);
    // A heading takes the next block along with it.
    if (blocks[i]!.keepWithNext && i + 1 < blocks.length) need += cost(i + 1, false);
    if (!atTop && used + need > room) {
      newPage();
    }
    if (current.blocks.length === 0 && blocks[i]!.headerHeight) {
      // A row opening a page: is it a continuation (its table started on an
      // earlier page)? The table's first row follows its head directly, and
      // the head is its own block, so only a row not preceded by one is.
      const prev = blocks[i - 1];
      current.repeatHeader = prev !== undefined && prev.headerHeight !== undefined;
    }
    used += cost(i, current.blocks.length === 0);
    current.blocks.push(i);
  }
  newPage();
  return pages;
}

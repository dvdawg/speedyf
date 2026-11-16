/** A page's display equations, as the engine finds them
 * (`get_equation_blocks`), asked for once per page. Used by the equation
 * buttons, the text layer (a selection takes a display whole) and Markup
 * (a comment's quote reads its displays). */
import { invoke } from '@tauri-apps/api/core';

export interface EquationBlock {
  /** `[x, y, w, h]`, page points from the displayed page's top-left. */
  bbox: [number, number, number, number];
  number: string | null;
}

/** Blocks already asked for, per document page and generation, so a page that
 * scrolls back into view does not ask again. Bounded: a long session reading
 * many documents must not keep every page's answer. */
const cache = new Map<string, Promise<EquationBlock[]>>();
const CACHE_LIMIT = 512;

export function equationBlocksFor(
  docId: number,
  src: number,
  generation: number
): Promise<EquationBlock[]> {
  const key = `${docId}:${src}:${generation}`;
  let hit = cache.get(key);
  if (!hit) {
    hit = invoke<EquationBlock[]>('get_equation_blocks', { docId, src }).catch(() => []);
    if (cache.size >= CACHE_LIMIT) cache.clear();
    cache.set(key, hit);
  }
  return hit;
}

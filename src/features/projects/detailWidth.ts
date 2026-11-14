/** The Projects detail column's width: dragged by its left edge, the middle
 * column taking up the difference, and remembered (like the reader drawer's,
 * extensions/surfaces.ts). The left column is fixed, and the middle one
 * always keeps room for its rows. */
import { createSignal } from 'solid-js';

const WIDTH_KEY = 'speedyf-projects-detail-width';
export const DEFAULT_DETAIL_WIDTH = 400;
export const MIN_DETAIL_WIDTH = 280;
/** The projects column, which does not resize. */
const LEFT_COLUMN = 220;
/** What the middle column keeps at least. */
const MIN_MIDDLE = 300;

export function clampDetailWidth(desired: number, windowWidth: number): number {
  if (!Number.isFinite(desired)) return DEFAULT_DETAIL_WIDTH;
  const ceiling = Math.max(MIN_DETAIL_WIDTH, windowWidth - LEFT_COLUMN - MIN_MIDDLE);
  return Math.round(Math.min(Math.max(desired, MIN_DETAIL_WIDTH), ceiling));
}

const windowWidth = () => (typeof window === 'undefined' ? 1280 : window.innerWidth);

function storedWidth(): number {
  try {
    const raw = localStorage.getItem(WIDTH_KEY);
    if (raw === null) return DEFAULT_DETAIL_WIDTH;
    return clampDetailWidth(Number(raw), windowWidth());
  } catch {
    return DEFAULT_DETAIL_WIDTH;
  }
}

const [width, setWidthSignal] = createSignal(storedWidth());

/** The width now. */
export function detailWidth(): number {
  return width();
}

export function setDetailWidth(desired: number): void {
  const next = clampDetailWidth(desired, windowWidth());
  setWidthSignal(next);
  try {
    localStorage.setItem(WIDTH_KEY, String(next));
  } catch {
    /* session-only */
  }
}

/** For tests: read the width from storage again. */
export function reloadDetailWidthForTest(): void {
  setWidthSignal(storedWidth());
}

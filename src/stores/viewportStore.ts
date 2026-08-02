/** Viewport state: zoom, fit mode, view rotation, scroll, current page.
 * Lightweight values only — layout geometry is derived in components. */
import { createStore } from 'solid-js/store';
import type { FitMode, Rotation } from '../types/model';

export interface ViewportState {
  zoom: number;
  fitMode: FitMode;
  /** whole-view rotation applied on top of per-page rotation */
  viewRotation: Rotation;
  scrollTop: number;
  scrollLeft: number;
  containerW: number;
  containerH: number;
  dpr: number;
  currentPage: number;
  sidebarOpen: boolean;
  searchOpen: boolean;
  formPanelOpen: boolean;
  editMode: boolean;
  /** bumped to request programmatic page navigation or exact view restore */
  scrollRequest: ScrollRequest | null;
}

export type ScrollRequest =
  | { kind: 'page'; page: number; offsetCss?: number; seq: number }
  | { kind: 'position'; top: number; left: number; seq: number };

export const MIN_ZOOM = 0.2;
export const MAX_ZOOM = 6;
export const PAGE_GAP = 16;
export const VIEW_PADDING = 24;

const [viewport, setViewport] = createStore<ViewportState>({
  zoom: 1,
  fitMode: 'fit-width',
  viewRotation: 0,
  scrollTop: 0,
  scrollLeft: 0,
  containerW: 800,
  containerH: 600,
  dpr: typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1,
  currentPage: 0,
  sidebarOpen: true,
  searchOpen: false,
  formPanelOpen: false,
  editMode: false,
  scrollRequest: null,
});

export { viewport, setViewport };

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

let scrollSeq = 0;
export function requestScrollToPage(page: number, offsetCss?: number) {
  setViewport('scrollRequest', {
    kind: 'page',
    page,
    seq: ++scrollSeq,
    ...(offsetCss !== undefined ? { offsetCss } : {}),
  });
}

/** Restore an exact reading position, used by internal-reference history. */
export function requestScrollToPosition(top: number, left: number) {
  setViewport('scrollRequest', {
    kind: 'position',
    top: Math.max(0, top),
    left: Math.max(0, left),
    seq: ++scrollSeq,
  });
}

export function rotateView() {
  setViewport('viewRotation', ((viewport.viewRotation + 90) % 360) as Rotation);
}

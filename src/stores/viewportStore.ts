/** Viewport state: zoom, fit mode, view rotation, scroll, current page.
 * Lightweight values only — layout geometry is derived in components.
 * One instance per open tab (see tabsStore.ts); `createViewportStore()` is
 * the factory, with a temporary module-level singleton kept below until the
 * tab-registry wiring lands. */
import { createStore, type SetStoreFunction } from 'solid-js/store';
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
  searchOpen: boolean;
  formPanelOpen: boolean;
  editMode: boolean;
  /** bumped to request programmatic page navigation or exact view restore */
  scrollRequest: ScrollRequest | null;
}

export type ScrollRequest =
  | {
      kind: 'page';
      page: number;
      offsetCss?: number;
      /** 0..1 down the page, resolved against the layout at apply time so a
       * restore is correct whatever zoom the document settles at */
      fraction?: number;
      /** wait for the initial fit before applying (see Viewer) */
      settle?: boolean;
      seq: number;
    }
  | { kind: 'position'; top: number; left: number; seq: number };

export const MIN_ZOOM = 0.2;
export const MAX_ZOOM = 6;
export const PAGE_GAP = 16;
export const VIEW_PADDING = 24;

export function clampZoom(z: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

export interface ViewportStore {
  state: ViewportState;
  setState: SetStoreFunction<ViewportState>;
  requestScrollToPage(page: number, offsetCss?: number): void;
  /** Restores a remembered reading position once the initial fit has settled. */
  requestRestorePosition(page: number, fraction: number): void;
  requestScrollToPosition(top: number, left: number): void;
  rotateView(): void;
}

function emptyState(): ViewportState {
  return {
    zoom: 1,
    fitMode: 'fit-width',
    viewRotation: 0,
    scrollTop: 0,
    scrollLeft: 0,
    containerW: 800,
    containerH: 600,
    dpr: typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1,
    currentPage: 0,
    searchOpen: false,
    formPanelOpen: false,
    editMode: false,
    scrollRequest: null,
  };
}

export function createViewportStore(): ViewportStore {
  const [state, setState] = createStore<ViewportState>(emptyState());
  let scrollSeq = 0;

  return {
    // getter, not a data property — see the note in tabsStore.ts
    get state() {
      return state;
    },
    setState,

    requestScrollToPage(page: number, offsetCss?: number) {
      setState('scrollRequest', {
        kind: 'page',
        page,
        seq: ++scrollSeq,
        ...(offsetCss !== undefined ? { offsetCss } : {}),
      });
    },

    requestRestorePosition(page: number, fraction: number) {
      setState('scrollRequest', {
        kind: 'page',
        page,
        fraction,
        settle: true,
        seq: ++scrollSeq,
      });
    },

    requestScrollToPosition(top: number, left: number) {
      setState('scrollRequest', {
        kind: 'position',
        top: Math.max(0, top),
        left: Math.max(0, left),
        seq: ++scrollSeq,
      });
    },

    rotateView() {
      setState('viewRotation', ((state.viewRotation + 90) % 360) as Rotation);
    },
  };
}

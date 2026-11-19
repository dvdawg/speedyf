/** Where a freshly opened tab's view starts: the page, the place on it, and
 * the zoom its reader left it at.
 *
 * Zoom is per file, beside the reading position (recentStore), not per tab
 * in the session: a document comes back at the zoom it was read at however
 * it is opened again (a relaunch, Open Recent, a double-click in Finder),
 * the way Acrobat's "restore last view settings" and Preview do it. */
import { clampZoom, type ViewportStore } from '../../stores/viewportStore';
import type { FitMode } from '../../types/model';

export interface ResumePoint {
  page: number;
  /** 0..1 down the page */
  fraction: number;
  zoom?: number;
  fitMode?: FitMode;
}

/** The zoom a saved view asks for, made safe: clamped into the range the
 * viewer supports, and nothing at all when it cannot be used (an explicit
 * percentage that is not a number, a fit mode this version does not know).
 * For a fit mode the zoom is only a first guess, recomputed for the window
 * as it is now; without one the viewer's default stands in until then. */
export function restoredZoom(
  point: Pick<ResumePoint, 'zoom' | 'fitMode'>
): { zoom?: number; fitMode: FitMode } | null {
  const zoom =
    typeof point.zoom === 'number' && Number.isFinite(point.zoom) && point.zoom > 0
      ? clampZoom(point.zoom)
      : undefined;
  switch (point.fitMode) {
    case 'fit-page':
    case 'fit-width':
      return zoom === undefined ? { fitMode: point.fitMode } : { zoom, fitMode: point.fitMode };
    case 'custom':
      return zoom === undefined ? null : { zoom, fitMode: 'custom' };
    default:
      return null;
  }
}

/** Puts a new tab's view where `resume` says, or at the top of the document.
 *
 * Order matters. The zoom goes in first, in the same store write as the page,
 * and only then is the scroll asked for: the request is resolved against the
 * layout when the viewer applies it, after the initial fit has settled, and
 * that layout depends on the zoom. Restoring the zoom after the scroll would
 * land the page somewhere else; restoring it at the same time as the first
 * render means the document never shows at the default zoom first. */
export function resumeView(
  vp: Pick<ViewportStore, 'setState' | 'requestRestorePosition' | 'requestScrollToPage'>,
  resume: ResumePoint | null,
  pageCount: number
): void {
  const zoom = resume ? restoredZoom(resume) : null;
  const page = resume && resume.page >= 0 && resume.page < pageCount ? resume.page : null;
  vp.setState({ currentPage: page ?? 0, scrollTop: 0, ...(zoom ?? {}) });
  if (resume && page !== null) vp.requestRestorePosition(page, resume.fraction);
  else vp.requestScrollToPage(0);
}

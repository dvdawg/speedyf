/** Builds pdfr:// URLs for rendered content. The webview never receives pixel
 * data through IPC — <img> elements pull binary PNGs straight from the custom
 * protocol, so bitmap bytes stay out of frontend state entirely. */
import type { Rotation } from '../../types/model';
import type { TileRect } from '../coordinates/coords';

export type RenderKind = 'page' | 'thumb' | 'tile' | 'preview';

export interface RenderUrlSpec {
  docId: number;
  srcIndex: number;
  rotation: Rotation;
  /** quantized device scale × 1000 (integer keeps cache keys exact) */
  scaleMilli: number;
  /** engine generation — bumping it invalidates every outstanding URL */
  generation: number;
  kind: RenderKind;
  tile?: TileRect;
}

/** Tauri custom-scheme origins differ per platform. */
export function protocolBase(scheme: string, isWindows: boolean): string {
  return isWindows ? `http://${scheme}.localhost` : `${scheme}://localhost`;
}

export function buildRenderUrl(spec: RenderUrlSpec, isWindows: boolean): string {
  let url =
    `${protocolBase('pdfr', isWindows)}/render` +
    `?doc=${spec.docId}&src=${spec.srcIndex}&rot=${spec.rotation}` +
    `&scale=${spec.scaleMilli}&gen=${spec.generation}&kind=${spec.kind}`;
  if ((spec.kind === 'tile' || spec.kind === 'preview') && spec.tile) {
    url += `&tx=${spec.tile.x}&ty=${spec.tile.y}&tw=${spec.tile.w}&th=${spec.tile.h}`;
  }
  return url;
}

/** How many times a blank raster is re-requested before it is reported as a
 * failed render. A render abandoned by an engine cancel answers 204, which an
 * <img> reports as a load error; since the URL is unchanged nothing would ever
 * ask again, so the frontend has to. */
export const RENDER_RETRIES = 2;

/** Same raster, cache-busted. `retry` is not part of the render query, so the
 * engine resolves this to the byte-identical cache key — it only stops the
 * webview from replaying the empty response it already has. */
export function retryUrl(url: string, attempt: number): string {
  return attempt > 0 ? `${url}&retry=${attempt}` : url;
}

let cachedIsWindows: boolean | null = null;

/** Runtime variant using the current platform (detected once). */
export function renderUrl(spec: RenderUrlSpec): string {
  if (cachedIsWindows === null) {
    cachedIsWindows =
      typeof navigator !== 'undefined' && /windows/i.test(navigator.userAgent ?? '');
  }
  return buildRenderUrl(spec, cachedIsWindows);
}

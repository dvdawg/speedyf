/** Pictures of what the user pointed at (`render:region`): a rectangle
 * dragged on a page (`region.capture`), or an equation they clicked
 * (`equations.snapshot`). The page is rendered by the engine at idle, the
 * region cut out of it here, and the extension gets PNG bytes and nothing
 * else - never the page, never the document. */
import { createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { activeTab } from '../../stores/tabsStore';
import type { EquationRef } from './editorHost';

export type Box = [number, number, number, number];

export interface CapturedRegion {
  /** Zero-based, as the reader sees pages. */
  page: number;
  /** Top-left page points on the page as displayed. */
  box: Box;
  /** PNG bytes. */
  png: number[];
}

/** Render scale for a captured region: about 2x, sharp on a Retina
 * screen, a few hundred kilobytes for a figure. */
const SCALE_MILLI = 2000;
/** A drag shorter than this either way is a click, not a region. */
export const MIN_REGION_PT = 4;

export interface CaptureRequest {
  prompt: string;
  finish: (region: { page: number; box: Box } | null) => void;
}

const [request, setRequest] = createSignal<CaptureRequest | null>(null);
export { request as captureRequest };

/** The rectangle between two points on a page, in page points, kept on the
 * page; null when it is too small to be a region. `scale` is CSS pixels per
 * point. */
export function regionFromDrag(
  start: { x: number; y: number },
  end: { x: number; y: number },
  scale: number,
  page: { widthPt: number; heightPt: number }
): Box | null {
  if (!(scale > 0)) return null;
  const clampX = (v: number) => Math.min(Math.max(v / scale, 0), page.widthPt);
  const clampY = (v: number) => Math.min(Math.max(v / scale, 0), page.heightPt);
  const x0 = clampX(Math.min(start.x, end.x));
  const x1 = clampX(Math.max(start.x, end.x));
  const y0 = clampY(Math.min(start.y, end.y));
  const y1 = clampY(Math.max(start.y, end.y));
  if (x1 - x0 < MIN_REGION_PT || y1 - y0 < MIN_REGION_PT) return null;
  const round = (v: number) => Math.round(v * 10) / 10;
  return [round(x0), round(y0), round(x1 - x0), round(y1 - y0)];
}

/** Cut `box` (page points) out of a page PNG rendered `widthPt` wide. */
async function crop(pagePng: ArrayBuffer, widthPt: number, box: Box): Promise<number[]> {
  const bitmap = await createImageBitmap(new Blob([pagePng], { type: 'image/png' }));
  const s = bitmap.width / widthPt;
  const [x, y, w, h] = box;
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(w * s));
  canvas.height = Math.max(1, Math.round(h * s));
  const context = canvas.getContext('2d');
  if (!context) throw new Error('cannot draw the region');
  context.fillStyle = '#ffffff';
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(bitmap, x * s, y * s, w * s, h * s, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
  if (!blob) throw new Error('cannot encode the region');
  return Array.from(new Uint8Array(await blob.arrayBuffer()));
}

/** A PNG of `box` on reader page `page` of the active document. */
async function renderRegion(page: number, box: Box): Promise<CapturedRegion> {
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded) throw new Error('no document is open');
  const src = doc.pages[page]?.srcIndex;
  const geom = tab.zoom.pagesGeom()[page];
  if (src === undefined || src === null || !geom) throw new Error(`page ${page} cannot be drawn`);
  if (geom.rotation !== 0) throw new Error('regions can be taken from unrotated pages only');
  const bytes = await invoke<ArrayBuffer>('extension_render_page', {
    docId: doc.docId,
    src,
    scaleMilli: SCALE_MILLI,
  });
  return { page, box, png: await crop(bytes, geom.widthPt, box) };
}

/** Let the user drag a rectangle on a page; its picture, or null when they
 * press Escape. One at a time: a second request cancels the first. */
export async function captureRegion(
  prompt: unknown,
  extensionName: string
): Promise<CapturedRegion | null> {
  if (!activeTab()?.documentStore.state.loaded) throw new Error('no document is open');
  request()?.finish(null);
  const text =
    typeof prompt === 'string' && prompt.trim()
      ? prompt.trim().slice(0, 120)
      : `${extensionName}: drag a rectangle on the page. Escape cancels.`;
  const picked = await new Promise<{ page: number; box: Box } | null>((resolve) => {
    setRequest({
      prompt: text,
      finish: (region) => {
        setRequest(null);
        resolve(region);
      },
    });
  });
  return picked ? renderRegion(picked.page, picked.box) : null;
}

/** The picture of an equation the user clicked. */
export async function snapshotRegion(ref: EquationRef): Promise<CapturedRegion> {
  if (activeTab()?.documentStore.state.docId !== ref.docId) {
    throw new Error('the equation is not on screen any more');
  }
  // A little air around the block, as the eye sees it.
  const [x, y, w, h] = ref.bbox;
  const pad = 3;
  return renderRegion(ref.page, [
    Math.max(0, x - pad),
    Math.max(0, y - pad),
    w + 2 * pad,
    h + 2 * pad,
  ]);
}

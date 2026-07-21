/** Page-editing actions shared by Toolbar and Sidebar. */
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { engine } from '../../lib/transport/engine';
import { documentStore } from '../document/documentStore';
import { viewport } from '../../stores/viewportStore';
import { showError } from '../../stores/modalStore';

let annotSeq = 0;
export function newAnnotId(): string {
  return `an-${Date.now().toString(36)}-${++annotSeq}`;
}

export async function addImageFromDialog() {
  const state = documentStore.state;
  if (!state.loaded) return;
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg'] }],
  });
  if (typeof picked !== 'string') return;
  try {
    const [nw, nh] = await engine.imageSize(picked);
    const page = state.pages[Math.min(viewport.currentPage, state.pages.length - 1)];
    if (!page) return;
    // place at ~40% of page width, centered
    const targetW = Math.min(page.widthPt * 0.4, nw * 0.75);
    const targetH = targetW * (nh / nw);
    const rect = {
      x: (page.widthPt - targetW) / 2,
      y: (page.heightPt - targetH) / 2,
      w: targetW,
      h: targetH,
    };
    const id = newAnnotId();
    documentStore.apply({
      type: 'addAnnot',
      annot: {
        id,
        pageId: page.id,
        kind: 'image',
        rect,
        color: '#000000',
        opacity: 1,
        sourcePath: picked,
        naturalW: nw,
        naturalH: nh,
      },
    });
    documentStore.setSelected({ pageId: page.id, annotId: id });
  } catch (e) {
    showError(`Could not load image: ${String(e)}`);
  }
}

export function addBlankPageAfter(index: number) {
  const state = documentStore.state;
  const ref = state.pages[Math.min(index, state.pages.length - 1)];
  documentStore.apply({
    type: 'addBlank',
    index: Math.min(index + 1, state.pages.length),
    widthPt: ref?.widthPt ?? 612,
    heightPt: ref?.heightPt ?? 792,
  });
}

/** Small downscaled preview of a local image for the annotation overlay
 * (binary IPC → blob URL; revoked when the document changes). */
const previewCache = new Map<string, string>();

export async function imagePreviewUrl(path: string): Promise<string> {
  const hit = previewCache.get(path);
  if (hit) return hit;
  const bytes = await invoke<ArrayBuffer>('image_preview', { path });
  const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
  previewCache.set(path, url);
  return url;
}

export function clearImagePreviews() {
  for (const url of previewCache.values()) URL.revokeObjectURL(url);
  previewCache.clear();
}

/** Page-editing actions shared by Toolbar and Sidebar. Takes the acting
 * tab's own documentStore/viewport explicitly since this is a plain module
 * (not a component), so it can't read them via TabContext. */
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { engine } from '../../lib/transport/engine';
import type { DocumentStore } from '../document/documentStore';
import type { ViewportStore } from '../../stores/viewportStore';
import { showError } from '../../stores/modalStore';

let annotSeq = 0;
export function newAnnotId(): string {
  return `an-${Date.now().toString(36)}-${++annotSeq}`;
}

export async function addImageFromDialog(documentStore: DocumentStore, vp: ViewportStore) {
  const state = documentStore.state;
  if (!state.loaded) return;
  const docId = state.docId;
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg'] }],
  });
  if (typeof picked !== 'string') return;
  try {
    const [nw, nh] = await engine.imageSize(picked);
    if (!state.loaded || state.docId !== docId) return;
    const page = state.pages[Math.min(vp.state.currentPage, state.pages.length - 1)];
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

export function addBlankPageAfter(documentStore: DocumentStore, index: number) {
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
interface PendingPreview {
  promise: Promise<string>;
}
const previewPending = new Map<string, PendingPreview>();
let previewEpoch = 0;

export async function imagePreviewUrl(path: string): Promise<string> {
  const hit = previewCache.get(path);
  if (hit) return hit;
  const pending = previewPending.get(path);
  if (pending) return pending.promise;

  const epoch = previewEpoch;
  const entry = {} as PendingPreview;
  entry.promise = (async () => {
    try {
      const bytes = await invoke<ArrayBuffer>('image_preview', { path });
      if (epoch !== previewEpoch) throw new Error('image preview request was invalidated');
      const raced = previewCache.get(path);
      if (raced) return raced;
      const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
      previewCache.set(path, url);
      return url;
    } finally {
      if (previewPending.get(path) === entry) previewPending.delete(path);
    }
  })();
  previewPending.set(path, entry);
  return entry.promise;
}

export function clearImagePreviews() {
  previewEpoch += 1;
  for (const url of previewCache.values()) URL.revokeObjectURL(url);
  previewCache.clear();
  previewPending.clear();
}

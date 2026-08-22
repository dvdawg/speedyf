/** Open / save / close orchestration for the active tab. Owns every native
 * file dialog and the post-save reload, keeping components free of workflow
 * logic. Multi-tab lifecycle (new tab / dedup-focus / close) lives in
 * tabsController.ts — this module only ever acts on the currently active
 * tab's own stores. */
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import { engine, isEngineError } from '../../lib/transport/engine';
import type { TabRecord } from '../../stores/tabsStore';
import { askUnsaved, showError } from '../../stores/modalStore';
import { recentStore } from '../../stores/recentStore';
import type { Annotation } from '../../types/model';

const SIZE_BATCH = 128;

export async function hydrateSizes(tab: TabRecord, pageCount: number) {
  const docId = tab.documentStore.state.docId;
  for (let from = 64; from < pageCount; from += SIZE_BATCH) {
    if (tab.documentStore.state.docId !== docId) return; // doc changed — stop
    try {
      const res = await engine.requestPageSizes(docId, from, SIZE_BATCH);
      if (tab.documentStore.state.docId !== docId) return;
      tab.documentStore.updateSizes(res.from, res.sizes);
    } catch {
      return; // doc closed mid-flight
    }
  }
}

/** Adopt the annotations already in the file, so they can be selected, moved
 * and deleted instead of being pixels the renderer happens to draw.
 *
 * Best effort: a document whose annotations cannot be read still opens and
 * still shows them, because PDFium draws them either way — they simply stay
 * uneditable, which is exactly how every document behaved before this existed.
 *
 * The engine hides whatever it hands over in its own render document, so the
 * page images have to be refetched or each annotation would appear twice: once
 * drawn into the raster, once by the editor on top of it. */
export async function hydrateAnnotations(tab: TabRecord, docId: number) {
  let pages;
  try {
    pages = await engine.getAnnotations(docId);
  } catch {
    return;
  }
  if (tab.documentStore.state.docId !== docId || pages.length === 0) return;

  tab.documentStore.hydrateAnnotations(
    pages.map((page) => ({
      src: page.src,
      annots: page.annots.map((a) => ({
        id: '', // replaced by the store, which owns id generation
        pageId: '',
        kind: a.kind as Annotation['kind'],
        rect: a.rect,
        color: a.color,
        opacity: a.opacity,
        srcAnnotIndex: a.index,
        ...(a.strokeWidth !== null ? { strokeWidth: a.strokeWidth } : {}),
        ...(a.quads !== null ? { quads: a.quads } : {}),
        ...(a.strokes !== null ? { strokes: a.strokes } : {}),
        ...(a.text !== null ? { text: a.text } : {}),
        ...(a.fontSizePt !== null ? { fontSizePt: a.fontSizePt } : {}),
      })),
    }))
  );

  try {
    tab.documentStore.setGeneration(await engine.bumpGeneration(docId));
  } catch {
    // The annotations are editable regardless; the raster just catches up on
    // the next reason to re-render.
  }
}

/** Returns false when the user cancels. */
export async function guardUnsaved(tab: TabRecord): Promise<boolean> {
  if (!tab.documentStore.state.loaded || !tab.documentStore.state.dirty) return true;
  const choice = await askUnsaved(
    `"${tab.documentStore.state.name}" has unsaved changes. Save them before continuing?`
  );
  if (choice === 'cancel') return false;
  if (choice === 'save') return saveDocument(tab, false);
  return true; // discard
}

/** Save (or Save As). Returns true on success. */
export async function saveDocument(tab: TabRecord, saveAs: boolean): Promise<boolean> {
  const state = tab.documentStore.state;
  if (!state.loaded || state.saving) return false;
  if (!saveAs && !state.dirty) return true;
  if (state.pages.length > 65_535) {
    showError(
      `This document currently has ${state.pages.length.toLocaleString()} pages. ` +
        'PDF output is limited to 65,535 pages; delete pages below that limit before saving.'
    );
    return false;
  }
  const docId = state.docId;

  let dest = state.path;
  if (saveAs || !dest) {
    const picked = await saveDialog({
      defaultPath: state.path ?? state.name ?? 'document.pdf',
      filters: [{ name: 'PDF documents', extensions: ['pdf'] }],
    });
    if (!picked) return false;
    if (!state.loaded || state.docId !== docId) return false;
    dest = picked.toLowerCase().endsWith('.pdf') ? picked : `${picked}.pdf`;
  }

  if (!state.loaded || state.docId !== docId) return false;
  tab.documentStore.setSaving(true);
  const rememberPage = tab.viewport.state.currentPage;
  try {
    const plan = tab.documentStore.buildEditPlan();
    await engine.saveDocument(docId, plan, dest);
    // A discard/open can replace the active document while the native save
    // is running. The file was saved, but its completion must not overwrite
    // the newer session's model or navigation.
    if (!state.loaded || state.docId !== docId) return true;
    tab.documentStore.markSaved();
    // Reload from disk so the saved file becomes the new baseline (edits are
    // baked in; undo history restarts clean).
    await reloadTabFromDisk(tab, dest);
    tab.viewport.requestScrollToPage(
      Math.min(rememberPage, tab.documentStore.state.pages.length - 1)
    );
    return true;
  } catch (e) {
    showError(
      isEngineError(e)
        ? `Save failed: ${e.message}. The original file was not modified.`
        : `Save failed: ${String(e)}`
    );
    return false;
  } finally {
    if (tab.documentStore.state.docId === docId) tab.documentStore.setSaving(false);
  }
}

/** Reopens `dest` from disk into `tab`'s own stores in place, without
 * creating a new tab (used after Save/Save As, and only there — routing a
 * post-save reload through tabsController.openInNewTabOrFocus would spawn a
 * duplicate tab for the same file on every save). */
export async function reloadTabFromDisk(tab: TabRecord, dest: string): Promise<void> {
  const meta = await engine.open(dest);
  tab.documentStore.initFromMeta(meta);
  tab.searchStore.resetForDocument(meta.docId, meta.pageCount);
  tab.citationStore.syncDocument(meta.docId);
  tab.structureStore.syncDocument(meta.docId);
  recentStore.recordOpen({ path: dest, name: meta.name });
}

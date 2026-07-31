/** Open / save / close orchestration. Owns every native file dialog and the
 * post-save reload, keeping components free of workflow logic. */
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { engine, isEngineError } from '../../lib/transport/engine';
import { documentStore } from './documentStore';
import { askPassword, askUnsaved, showError } from '../../stores/modalStore';
import { requestScrollToPage, setViewport, viewport } from '../../stores/viewportStore';
import { searchStore } from '../search/searchStore';
import { clearImagePreviews } from '../editor/editorActions';

const SIZE_BATCH = 128;
let openRequestSeq = 0;

async function hydrateSizes(docId: number, pageCount: number) {
  for (let from = 64; from < pageCount; from += SIZE_BATCH) {
    if (documentStore.state.docId !== docId) return; // doc changed — stop
    try {
      const res = await engine.requestPageSizes(docId, from, SIZE_BATCH);
      if (documentStore.state.docId !== docId) return;
      documentStore.updateSizes(res.from, res.sizes);
    } catch {
      return; // doc closed mid-flight
    }
  }
}

/** Returns false when the user cancels. */
export async function guardUnsaved(): Promise<boolean> {
  if (!documentStore.state.loaded || !documentStore.state.dirty) return true;
  const choice = await askUnsaved(
    `"${documentStore.state.name}" has unsaved changes. Save them before continuing?`
  );
  if (choice === 'cancel') return false;
  if (choice === 'save') return saveDocument(false);
  return true; // discard
}

export async function openFromDialog() {
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: 'PDF documents', extensions: ['pdf'] }],
  });
  if (typeof picked === 'string') await openPath(picked);
}

export async function openPath(path: string, opts?: { skipGuard?: boolean }) {
  if (!opts?.skipGuard && !(await guardUnsaved())) return;
  const requestSeq = ++openRequestSeq;
  const previous = documentStore.state.loaded ? documentStore.state.docId : null;

  let password: string | undefined;
  for (;;) {
    try {
      const meta = await engine.open(path, password);
      if (requestSeq !== openRequestSeq) {
        void engine.close(meta.docId);
        return;
      }
      if (previous !== null) void engine.close(previous);
      clearImagePreviews();
      searchStore.resetForDocument(meta.docId, meta.pageCount);
      documentStore.initFromMeta(meta);
      setViewport({ currentPage: 0, scrollTop: 0 });
      requestScrollToPage(0);
      void engine.startIndexing(meta.docId);
      void hydrateSizes(meta.docId, meta.pageCount);
      return;
    } catch (e) {
      if (requestSeq !== openRequestSeq) return;
      if (isEngineError(e) && e.code === 'password') {
        const entered = await askPassword(
          password === undefined
            ? 'This PDF is password-protected. Enter the password to open it.'
            : 'Incorrect password. Try again.'
        );
        if (entered === null) return;
        password = entered;
        continue;
      }
      showError(isEngineError(e) ? e.message : `Could not open ${path}: ${String(e)}`);
      return;
    }
  }
}

/** Save (or Save As). Returns true on success. */
export async function saveDocument(saveAs: boolean): Promise<boolean> {
  const state = documentStore.state;
  if (!state.loaded || state.saving) return false;
  if (!saveAs && !state.dirty) return true;
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
  documentStore.setSaving(true);
  const rememberPage = viewport.currentPage;
  try {
    const plan = documentStore.buildEditPlan();
    await engine.saveDocument(docId, plan, dest);
    // A discard/open can replace the active document while the native save
    // is running. The file was saved, but its completion must not overwrite
    // the newer session's model or navigation.
    if (!state.loaded || state.docId !== docId) return true;
    documentStore.markSaved();
    // Reload from disk so the saved file becomes the new baseline (edits are
    // baked in; undo history restarts clean).
    await openPath(dest, { skipGuard: true });
    requestScrollToPage(Math.min(rememberPage, documentStore.state.pages.length - 1));
    return true;
  } catch (e) {
    showError(
      isEngineError(e)
        ? `Save failed: ${e.message}. The original file was not modified.`
        : `Save failed: ${String(e)}`
    );
    return false;
  } finally {
    if (documentStore.state.docId === docId) documentStore.setSaving(false);
  }
}

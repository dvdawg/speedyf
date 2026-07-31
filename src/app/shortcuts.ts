/** Platform-aware keyboard shortcuts. Text inputs keep their native behavior:
 * while typing, only the file-level meta combos (open/save/search) fire. */
import { documentStore } from '../features/document/documentStore';
import { openFromDialog, saveDocument } from '../features/document/controller';
import { requestScrollToPage, setViewport, viewport } from '../stores/viewportStore';
import { setZoomAnchored, zoomStep, applyFit } from '../features/viewer/zoomController';
import { setActiveTool, toolState } from '../features/annotations/toolStore';
import { modal } from '../stores/modalStore';

const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return (
    t instanceof HTMLInputElement ||
    t instanceof HTMLTextAreaElement ||
    t instanceof HTMLSelectElement ||
    t.isContentEditable
  );
}

export function installShortcuts(): () => void {
  const handler = (e: KeyboardEvent) => {
    if (modal.kind !== null) return; // modals own the keyboard
    const meta = isMac ? e.metaKey : e.ctrlKey;
    const typing = isTypingTarget(e.target);
    const key = e.key;

    if (meta) {
      const k = key.toLowerCase();
      if (k === 'o') {
        e.preventDefault();
        void openFromDialog();
        return;
      }
      if (k === 's') {
        e.preventDefault();
        void saveDocument(e.shiftKey);
        return;
      }
      if (k === 'f') {
        e.preventDefault();
        setViewport('searchOpen', true);
        return;
      }
      if (typing) return; // browser handles text-editing undo/redo & the rest
      if (k === 'z') {
        e.preventDefault();
        if (e.shiftKey) documentStore.redo();
        else documentStore.undo();
        return;
      }
      if (!isMac && k === 'y') {
        e.preventDefault();
        documentStore.redo();
        return;
      }
      if (key === '=' || key === '+') {
        e.preventDefault();
        zoomStep(1);
        return;
      }
      if (key === '-') {
        e.preventDefault();
        zoomStep(-1);
        return;
      }
      if (key === '0') {
        e.preventDefault();
        applyFit('fit-page');
        return;
      }
      if (key === '1') {
        e.preventDefault();
        setZoomAnchored(1);
        return;
      }
      return;
    }

    if (typing) return;

    if (key === 'Escape') {
      if (viewport.searchOpen) {
        setViewport('searchOpen', false);
        return;
      }
      if (documentStore.state.selected) {
        documentStore.setSelected(null);
        return;
      }
      if (toolState.active !== 'select') {
        setActiveTool('select');
        return;
      }
      if (viewport.formPanelOpen) setViewport('formPanelOpen', false);
      return;
    }
    if (key === 'Delete' || key === 'Backspace') {
      const sel = documentStore.state.selected;
      if (sel) {
        e.preventDefault();
        documentStore.apply({ type: 'deleteAnnot', pageId: sel.pageId, id: sel.annotId });
      }
      return;
    }
    if (key === 'PageDown') {
      if (!documentStore.state.loaded) return;
      e.preventDefault();
      requestScrollToPage(Math.min(viewport.currentPage + 1, documentStore.state.pages.length - 1));
      return;
    }
    if (key === 'PageUp') {
      if (!documentStore.state.loaded) return;
      e.preventDefault();
      requestScrollToPage(Math.max(viewport.currentPage - 1, 0));
    }
  };

  window.addEventListener('keydown', handler);
  return () => window.removeEventListener('keydown', handler);
}

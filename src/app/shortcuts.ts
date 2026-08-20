/** Platform-aware keyboard shortcuts. Text inputs keep their native behavior:
 * while typing, only the file-level meta combos (open/save/search/close-tab)
 * fire. */
import { saveDocument } from '../features/document/controller';
import { closeTab, cycleTab, jumpToTab, openFromDialog } from '../features/document/tabsController';
import { activeTab } from '../stores/tabsStore';
import { setActiveTool, toolState } from '../features/annotations/toolStore';
import { modal } from '../stores/modalStore';
import { beginPrint } from '../features/print/printStore';

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

    // Ctrl+Tab / Ctrl+Shift+Tab cycle tabs regardless of platform (on mac,
    // Ctrl+Tab doesn't overlap the Cmd-based combos below) — same tier as
    // close-tab, so it isn't swallowed by focused inputs either.
    if (e.ctrlKey && key === 'Tab') {
      e.preventDefault();
      cycleTab(e.shiftKey ? -1 : 1);
      return;
    }

    if (meta) {
      const k = key.toLowerCase();
      if (k === 'o') {
        e.preventDefault();
        void openFromDialog();
        return;
      }
      if (k === 'p') {
        // Before the typing guard, and always prevented: otherwise the
        // webview opens its own print dialog for the viewer's DOM.
        e.preventDefault();
        const tab = activeTab();
        if (tab) void beginPrint(tab);
        return;
      }
      if (k === 's') {
        e.preventDefault();
        const tab = activeTab();
        if (tab) void saveDocument(tab, e.shiftKey);
        return;
      }
      if (k === 'w') {
        e.preventDefault();
        const tab = activeTab();
        if (tab) void closeTab(tab.id);
        return;
      }
      if (k === 'f') {
        e.preventDefault();
        activeTab()?.viewport.setState('searchOpen', true);
        return;
      }
      if (typing) return; // browser handles text-editing undo/redo & the rest
      if (k === 'z') {
        e.preventDefault();
        const tab = activeTab();
        if (tab) {
          if (e.shiftKey) tab.documentStore.redo();
          else tab.documentStore.undo();
        }
        return;
      }
      if (!isMac && k === 'y') {
        e.preventDefault();
        activeTab()?.documentStore.redo();
        return;
      }
      if (key === ']' && e.shiftKey) {
        e.preventDefault();
        cycleTab(1);
        return;
      }
      if (key === '[' && e.shiftKey) {
        e.preventDefault();
        cycleTab(-1);
        return;
      }
      if (/^[1-9]$/.test(key)) {
        e.preventDefault();
        jumpToTab(Number(key) - 1);
        return;
      }
      if (key === '=' || key === '+') {
        e.preventDefault();
        activeTab()?.zoom.zoomStep(1);
        return;
      }
      if (key === '-') {
        e.preventDefault();
        activeTab()?.zoom.zoomStep(-1);
        return;
      }
      if (key === '0') {
        e.preventDefault();
        // Cmd+1-9 now jumps to a tab, so "zoom to 100%" moved here (mirrors
        // Cmd+0's existing fit-page binding).
        if (e.shiftKey) activeTab()?.zoom.setZoomAnchored(1);
        else activeTab()?.zoom.applyFit('fit-page');
        return;
      }
      return;
    }

    if (typing) return;

    if (key === 'Escape') {
      const tab = activeTab();
      if (!tab) return;
      if (tab.viewport.state.searchOpen) {
        tab.viewport.setState('searchOpen', false);
        return;
      }
      if (tab.documentStore.state.selected) {
        tab.documentStore.setSelected(null);
        return;
      }
      if (toolState.active !== 'select') {
        setActiveTool('select');
        return;
      }
      if (tab.viewport.state.formPanelOpen) tab.viewport.setState('formPanelOpen', false);
      return;
    }
    if (key === 'Delete' || key === 'Backspace') {
      const tab = activeTab();
      const sel = tab?.documentStore.state.selected;
      if (tab && sel) {
        e.preventDefault();
        tab.documentStore.apply({ type: 'deleteAnnot', pageId: sel.pageId, id: sel.annotId });
      }
      return;
    }
    if (key === 'PageDown') {
      const tab = activeTab();
      if (!tab?.documentStore.state.loaded) return;
      e.preventDefault();
      tab.viewport.requestScrollToPage(
        Math.min(tab.viewport.state.currentPage + 1, tab.documentStore.state.pages.length - 1)
      );
      return;
    }
    if (key === 'PageUp') {
      const tab = activeTab();
      if (!tab?.documentStore.state.loaded) return;
      e.preventDefault();
      tab.viewport.requestScrollToPage(Math.max(tab.viewport.state.currentPage - 1, 0));
    }
  };

  window.addEventListener('keydown', handler);
  return () => window.removeEventListener('keydown', handler);
}

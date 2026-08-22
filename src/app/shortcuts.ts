/** Platform-aware keyboard shortcuts.
 *
 * The actions themselves live in `commands.ts`; this file is only the keymap.
 * That split is what lets the same action be reached by two different chords
 * (⌃Tab and ⌘⇧] both cycle tabs) and, later, by a command palette.
 *
 * Text inputs keep their native behavior: while typing, only bindings marked
 * `whileTyping` fire — the file-level combos (open/save/print/close-tab/find)
 * and tab cycling.
 */
import { jumpToTab } from '../features/document/tabsController';
import { activeTab } from '../stores/tabsStore';
import { setActiveTool, toolState } from '../features/annotations/toolStore';
import { modal } from '../stores/modalStore';
import { openPalette, paletteOpen } from '../features/palette/paletteStore';
import { runCommand, type CommandCtx } from './commands';

const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

interface Binding {
  /** `KeyboardEvent.key`; single letters are compared lower-cased. */
  key: string;
  /** ⌘ on macOS, Ctrl elsewhere. */
  mod?: boolean;
  /** `undefined` matches with or without Shift. */
  shift?: boolean;
  id: string;
  /** Fires even while a text field has focus. */
  whileTyping?: boolean;
  /** Swallow the key even when the command is disabled. Every ⌘ combo does:
   * letting one through means the webview answers it (⌘P opening its own
   * print dialog for the viewer's DOM is the case that bites). */
  preventAlways?: boolean;
}

const BINDINGS: readonly Binding[] = [
  { key: 'o', mod: true, id: 'file.open', whileTyping: true, preventAlways: true },
  { key: 'p', mod: true, shift: false, id: 'file.print', whileTyping: true, preventAlways: true },
  { key: 's', mod: true, shift: false, id: 'file.save', whileTyping: true, preventAlways: true },
  { key: 's', mod: true, shift: true, id: 'file.saveAs', whileTyping: true, preventAlways: true },
  { key: 'w', mod: true, id: 'file.closeTab', whileTyping: true, preventAlways: true },
  {
    key: 'f',
    mod: true,
    id: 'search.findInDocument',
    whileTyping: true,
    preventAlways: true,
  },

  // Below here the typing guard applies: the browser owns text-editing
  // undo/redo and the rest while an input has focus.
  { key: 'z', mod: true, shift: false, id: 'edit.undo', preventAlways: true },
  { key: 'z', mod: true, shift: true, id: 'edit.redo', preventAlways: true },
  { key: ']', mod: true, shift: true, id: 'nav.nextTab', preventAlways: true },
  { key: '[', mod: true, shift: true, id: 'nav.previousTab', preventAlways: true },
  { key: '=', mod: true, id: 'view.zoomIn', preventAlways: true },
  { key: '+', mod: true, id: 'view.zoomIn', preventAlways: true },
  { key: '-', mod: true, id: 'view.zoomOut', preventAlways: true },
  // ⌘1-9 took the digits for tabs, so "zoom to 100%" sits on ⌘⇧0 beside
  // ⌘0's fit-page.
  { key: '0', mod: true, shift: false, id: 'view.fitPage', preventAlways: true },
  { key: '0', mod: true, shift: true, id: 'view.actualSize', preventAlways: true },

  // Unmodified keys only swallow the event when they actually act.
  { key: 'Delete', id: 'edit.deleteSelection' },
  { key: 'Backspace', id: 'edit.deleteSelection' },
  { key: 'PageDown', id: 'nav.nextPage' },
  { key: 'PageUp', id: 'nav.previousPage' },
];

// Windows and Linux keep Ctrl+Y as a second redo.
const REDO_Y: Binding = { key: 'y', mod: true, id: 'edit.redo', preventAlways: true };

function isTypingTarget(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return (
    t instanceof HTMLInputElement ||
    t instanceof HTMLTextAreaElement ||
    t instanceof HTMLSelectElement ||
    t.isContentEditable
  );
}

function matches(binding: Binding, event: KeyboardEvent, mod: boolean): boolean {
  const key = binding.key.length === 1 ? event.key.toLowerCase() : event.key;
  if (key !== binding.key) return false;
  if ((binding.mod ?? false) !== mod) return false;
  if (binding.shift !== undefined && binding.shift !== event.shiftKey) return false;
  return true;
}

/** Escape unwinds one layer at a time, innermost first, so a single press
 * never dismisses more than the user was looking at. Too sequential to be a
 * command: what it does depends entirely on what is currently open. */
function handleEscape(): void {
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
}

export function installShortcuts(): () => void {
  const handler = (e: KeyboardEvent) => {
    if (modal.kind !== null) return; // modals own the keyboard
    // The palette owns the keyboard too, or its arrow keys would also scroll
    // the document underneath it. It handles its own Escape.
    if (paletteOpen()) return;
    const mod = isMac ? e.metaKey : e.ctrlKey;
    const typing = isTypingTarget(e.target);
    const ctx: CommandCtx = { tab: activeTab() };

    // ⌃Tab cycles tabs on every platform, so it is matched before the ⌘/Ctrl
    // split below — on Windows and Linux `mod` *is* Ctrl, and folding this
    // into the table would make it unreachable there. Same tier as close-tab:
    // a focused input must not swallow it.
    if (e.ctrlKey && e.key === 'Tab') {
      e.preventDefault();
      runCommand(e.shiftKey ? 'nav.previousTab' : 'nav.nextTab', ctx);
      return;
    }

    if (mod && e.shiftKey && e.key.toLowerCase() === 'p') {
      e.preventDefault();
      openPalette();
      return;
    }

    const bindings = isMac ? BINDINGS : [...BINDINGS, REDO_Y];
    for (const binding of bindings) {
      if (!matches(binding, e, mod)) continue;
      if (typing && !binding.whileTyping) return;
      const ran = runCommand(binding.id, ctx);
      if (ran || binding.preventAlways) e.preventDefault();
      return;
    }

    // ⌘1-9 jumps to a tab by position. Parameterized rather than nine
    // near-identical commands, which a palette would only have to hide again.
    if (mod && !typing && /^[1-9]$/.test(e.key)) {
      e.preventDefault();
      jumpToTab(Number(e.key) - 1);
      return;
    }
    if (mod) return;
    if (typing) return;

    if (e.key === 'Escape') handleEscape();
  };

  window.addEventListener('keydown', handler);
  return () => window.removeEventListener('keydown', handler);
}

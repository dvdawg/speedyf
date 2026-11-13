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
import { windowTransferPending } from '../features/windows/windowController';
import { jumpToTab } from '../features/document/tabsController';
import { activeTab } from '../stores/tabsStore';
import { setActiveTool, toolState } from '../features/annotations/toolStore';
import { modal } from '../stores/modalStore';
import { openPalette, paletteOpen } from '../features/palette/paletteStore';
import { quickOpenOpen } from '../features/palette/quickOpenStore';
import { runCommand, type CommandCtx } from './commands';
import { canonicalChord, chordMatches } from '../features/extensions/capabilities';
import { extensionBindings, runBoundCommand } from '../features/extensions/extensionStore';
import { columnModeFor } from '../features/viewer/columns/columnMode';

const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

interface Binding {
  /** `KeyboardEvent.key`; single letters are compared lower-cased. */
  key: string;
  /** ⌘ on macOS, Ctrl elsewhere. */
  mod?: boolean;
  /** `undefined` matches with or without Shift. */
  shift?: boolean;
  /** Option on macOS, Alt elsewhere. Matched on the physical key
   * (`KeyboardEvent.code`), since Option turns C into a cedilla in `key`. */
  alt?: boolean;
  id: string;
  /** Fires even while a text field has focus. */
  whileTyping?: boolean;
  /** Swallow the key even when the command is disabled. Every ⌘ combo does:
   * letting one through means the webview answers it (⌘P opening its own
   * print dialog for the viewer's DOM is the case that bites). */
  preventAlways?: boolean;
}

const BINDINGS: readonly Binding[] = [
  {
    key: 'n',
    mod: true,
    shift: false,
    id: 'file.newWindow',
    whileTyping: true,
    preventAlways: true,
  },
  { key: 'o', mod: true, shift: false, id: 'file.open', whileTyping: true, preventAlways: true },
  // Quick open sits on the shifted Open: Cmd+P is print, which the webview
  // would answer with its own dialog for the viewer's DOM if we let it go.
  {
    key: 'o',
    mod: true,
    shift: true,
    id: 'library.quickOpen',
    whileTyping: true,
    preventAlways: true,
  },
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
  // Back/Forward through reading positions, on the browser's own chords. The
  // shifted forms above stay with tab cycling.
  { key: '[', mod: true, shift: false, id: 'nav.back', preventAlways: true },
  { key: ']', mod: true, shift: false, id: 'nav.forward', preventAlways: true },
  { key: '=', mod: true, id: 'view.zoomIn', preventAlways: true },
  { key: '+', mod: true, id: 'view.zoomIn', preventAlways: true },
  { key: '-', mod: true, id: 'view.zoomOut', preventAlways: true },
  // ⌘1-9 took the digits for tabs, so "zoom to 100%" sits on ⌘⇧0 beside
  // ⌘0's fit-page.
  // Column zoom joins the fits on 0, with Option; ahead of fit-page, which
  // does not look at Alt and would otherwise take Ctrl+Alt+0 elsewhere.
  { key: '0', mod: true, shift: false, alt: true, id: 'view.zoomToColumn', preventAlways: true },
  { key: '0', mod: true, shift: false, id: 'view.fitPage', preventAlways: true },
  { key: '0', mod: true, shift: true, id: 'view.actualSize', preventAlways: true },
  // Library. ⌘⇧C sits beside the webview's own ⌘C — copy, but the key — and
  // ⌘⇧L is the group's initial. Both claim only their shifted form, so an
  // extension can still take the plain key the way Scratchpad takes ⌘K.
  { key: 'c', mod: true, shift: true, id: 'library.copyCiteKey', preventAlways: true },
  // The key's sibling: Option instead of Shift, for a whole \cite{} of the
  // Projects selection (or the open item).
  {
    key: 'c',
    mod: true,
    shift: false,
    alt: true,
    id: 'library.copyCiteSelection',
    preventAlways: true,
  },
  { key: 'l', mod: true, shift: true, id: 'library.showPaperPanel', preventAlways: true },

  // Unmodified keys only swallow the event when they actually act.
  { key: 'Delete', id: 'edit.deleteSelection' },
  { key: 'Backspace', id: 'edit.deleteSelection' },
  { key: 'PageDown', id: 'nav.nextPage' },
  { key: 'PageUp', id: 'nav.previousPage' },
];

// Windows and Linux keep Ctrl+Y as a second redo.
const REDO_Y: Binding = { key: 'y', mod: true, id: 'edit.redo', preventAlways: true };

// ...and Alt+Left/Right as a second Back/Forward, as their browsers do. Not on
// macOS, where Option+Arrow is word movement and Cmd+[ is the convention.
const NAV_ALT: readonly Binding[] = [
  { key: 'ArrowLeft', alt: true, id: 'nav.back' },
  { key: 'ArrowRight', alt: true, id: 'nav.forward' },
];

/** Every chord SpeedyF binds for itself.
 *
 * Handed to the extension host so a contributed binding can be refused before
 * it ever reaches the keyboard. A binding whose `shift` is undefined matches
 * with or without it, so both forms are reserved — an extension taking ⌘⇧O
 * would otherwise shadow half of a built-in.
 *
 * The chords handled outside the table are listed explicitly. They are the
 * easiest ones to forget, and each is load-bearing: ⌘⇧P opens the palette,
 * ⌃Tab cycles tabs, ⌘1-9 jumps to one, and Escape unwinds whatever is open.
 */
export function reservedChords(): ReadonlySet<string> {
  const reserved = new Set<string>();
  const add = (key: string, mod: boolean, shift: boolean, alt = false) =>
    reserved.add(
      canonicalChord({ key: key.length === 1 ? key.toLowerCase() : key, mod, shift, alt })
    );

  for (const binding of [...BINDINGS, REDO_Y, ...NAV_ALT]) {
    const mod = binding.mod ?? false;
    const alt = binding.alt ?? false;
    if (binding.shift === undefined) {
      add(binding.key, mod, false, alt);
      add(binding.key, mod, true, alt);
    } else {
      add(binding.key, mod, binding.shift, alt);
    }
  }

  add('p', true, true); // command palette
  add('Tab', false, false); // ⌃Tab is matched on ctrlKey directly
  add('Tab', false, true);
  add('Escape', false, false);
  for (let digit = 1; digit <= 9; digit++) add(String(digit), true, false);
  return reserved;
}

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
  if (binding.alt) {
    if (!event.altKey) return false;
    const physical =
      (/^[a-z]$/.test(binding.key) && event.code === `Key${binding.key.toUpperCase()}`) ||
      (/^[0-9]$/.test(binding.key) && event.code === `Digit${binding.key}`);
    if (!physical && event.key.toLowerCase() !== binding.key.toLowerCase()) return false;
  } else {
    const key = binding.key.length === 1 ? event.key.toLowerCase() : event.key;
    if (key !== binding.key) return false;
  }
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
  if (tab.viewport.state.formPanelOpen) {
    tab.viewport.setState('formPanelOpen', false);
    return;
  }
  // Last: column zoom is the view itself, the outermost layer.
  columnModeFor(tab.id)?.exit();
}

export function installShortcuts(): () => void {
  const handler = (e: KeyboardEvent) => {
    if (windowTransferPending()) {
      e.preventDefault();
      return;
    }
    if (modal.kind !== null) return; // modals own the keyboard
    // The palette (and quick open) owns the keyboard too, or its arrow keys would also scroll
    // the document underneath it. It handles its own Escape.
    if (paletteOpen() || quickOpenOpen()) return;
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

    const bindings = isMac ? BINDINGS : [...BINDINGS, REDO_Y, ...NAV_ALT];
    for (const binding of bindings) {
      if (!matches(binding, e, mod)) continue;
      if (typing && !binding.whileTyping) return;
      const ran = runCommand(binding.id, ctx);
      if (ran || binding.preventAlways) e.preventDefault();
      return;
    }

    // Extension bindings come last, and only for keys SpeedyF did not claim —
    // `reservedChords()` already refused anything that would shadow a built-in,
    // so reaching here means the chord is genuinely free. They never fire while
    // a text field has focus: an extension must not eat characters as you type.
    if (!typing) {
      for (const binding of extensionBindings()) {
        if (!chordMatches(binding.chord, e, mod)) continue;
        e.preventDefault();
        void runBoundCommand(binding.extensionId, binding.command);
        return;
      }
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

/** Everything SpeedyF can be asked to do, as data.
 *
 * One registry, two readers: `shortcuts.ts` binds keys to entries here, and a
 * command palette can search the same list. Before this existed each action
 * was written inline in the key handler, which meant the keyboard was the only
 * way to reach any of them — and that most of the app (every annotation tool,
 * every page operation, every panel) could not be reached that way at all.
 *
 * A command is deliberately *not* a key binding. Which keys reach it, and
 * whether they survive a focused text field, are dispatch concerns that live
 * with the keymap; the same command is reached by two different chords in more
 * than one case.
 */
import { saveDocument } from '../features/document/controller';
import { closeTab, cycleTab, goHome, openFromDialog } from '../features/document/tabsController';
import { addBlankPageAfter } from '../features/editor/editorActions';
import { setActiveTool, type Tool } from '../features/annotations/toolStore';
import { beginPrint } from '../features/print/printStore';
import { settings, updateSettings } from '../stores/settings';
import { setUi, ui, type SidebarMode } from '../stores/uiStore';
import type { TabRecord } from '../stores/tabsStore';

export type CommandGroup =
  'File' | 'Edit' | 'Tools' | 'Page' | 'View' | 'Panels' | 'Settings' | 'Search' | 'Navigate';

/** What a command is allowed to know. Everything else is reachable through the
 * tab, so this stays one field rather than a snapshot that can drift. */
export interface CommandCtx {
  tab: TabRecord | null;
}

export interface Command {
  id: string;
  title: string;
  group: CommandGroup;
  /** Matched by a palette search, never displayed. */
  keywords?: readonly string[];
  /** Platform-neutral spec such as `mod+shift+s`, rendered by
   * `formatShortcut`. Display only — `shortcuts.ts` owns the real binding. */
  shortcut?: string;
  enabled(ctx: CommandCtx): boolean;
  run(ctx: CommandCtx): void;
  /** Why this is unavailable, when the generic reason would be wrong. */
  disabledReason?: string;
}

const IS_MAC = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);

const MAC_PARTS: Record<string, string> = {
  mod: '⌘',
  ctrl: '⌃',
  shift: '⇧',
  alt: '⌥',
  plus: '+',
  minus: '−',
  backspace: '⌫',
  pagedown: 'PgDn',
  pageup: 'PgUp',
  tab: 'Tab',
};

const PC_PARTS: Record<string, string> = {
  mod: 'Ctrl',
  ctrl: 'Ctrl',
  shift: 'Shift',
  alt: 'Alt',
  plus: '+',
  minus: '−',
  backspace: 'Backspace',
  pagedown: 'PgDn',
  pageup: 'PgUp',
  tab: 'Tab',
};

/** Render a neutral shortcut spec for the current platform.
 *
 * macOS stacks the glyphs (`⌘⇧S`); everywhere else the convention is a joined
 * word form (`Ctrl+Shift+S`). Storing the spec neutrally is what keeps a
 * Windows build from advertising a ⌘ that does not exist on the keyboard.
 */
export function formatShortcut(spec: string, isMac: boolean = IS_MAC): string {
  const table = isMac ? MAC_PARTS : PC_PARTS;
  const parts = spec.split('+').map((part) => table[part] ?? part.toUpperCase());
  return isMac ? parts.join('') : parts.join('+');
}

/** What to tell someone about a command they cannot run right now.
 *
 * Generic by default: forty per-command strings would be forty pieces of copy
 * to keep true, for a state most of them never reach.
 */
export function describeDisabled(command: Command, ctx: CommandCtx): string {
  if (command.disabledReason) return command.disabledReason;
  if (!ctx.tab || !loaded(ctx)) return 'no document open';
  if (command.group === 'Edit' && state(ctx)?.selected == null) return 'nothing selected';
  return 'unavailable here';
}

const state = (ctx: CommandCtx) => ctx.tab?.documentStore.state ?? null;
const loaded = (ctx: CommandCtx) => state(ctx)?.loaded === true;

/** The page the viewport is currently on, clamped — pages can be deleted out
 * from under a stale index. */
function currentPage(ctx: CommandCtx) {
  const tab = ctx.tab;
  const s = state(ctx);
  if (!tab || !s || s.pages.length === 0) return null;
  return s.pages[Math.min(tab.viewport.state.currentPage, s.pages.length - 1)] ?? null;
}

function tool(id: string, title: string, value: Tool, keywords: string[]): Command {
  return {
    id,
    title,
    group: 'Tools',
    keywords,
    enabled: loaded,
    run: () => setActiveTool(value),
  };
}

function panel(id: string, title: string, mode: SidebarMode, keywords: string[]): Command {
  return {
    id,
    title,
    group: 'Panels',
    keywords,
    enabled: (ctx) => ctx.tab !== null,
    run: () => setUi({ sidebarOpen: true, sidebarMode: mode }),
  };
}

function rotate(id: string, title: string, delta: 90 | 270): Command {
  return {
    id,
    title,
    group: 'Page',
    keywords: ['turn', 'orient'],
    enabled: (ctx) => loaded(ctx) && currentPage(ctx) !== null,
    run: (ctx) => {
      const page = currentPage(ctx);
      if (page) ctx.tab?.documentStore.apply({ type: 'rotate', pageId: page.id, delta });
    },
  };
}

export const COMMANDS: readonly Command[] = [
  // ---- File ----------------------------------------------------------
  {
    id: 'file.open',
    title: 'Open PDF…',
    group: 'File',
    shortcut: 'mod+o',
    enabled: () => true,
    run: () => void openFromDialog(),
  },
  {
    id: 'file.save',
    title: 'Save',
    group: 'File',
    shortcut: 'mod+s',
    enabled: loaded,
    run: (ctx) => {
      if (ctx.tab) void saveDocument(ctx.tab, false);
    },
  },
  {
    id: 'file.saveAs',
    title: 'Save As…',
    group: 'File',
    shortcut: 'mod+shift+s',
    enabled: loaded,
    run: (ctx) => {
      if (ctx.tab) void saveDocument(ctx.tab, true);
    },
  },
  {
    id: 'file.print',
    title: 'Print…',
    group: 'File',
    shortcut: 'mod+p',
    enabled: loaded,
    run: (ctx) => {
      if (ctx.tab) void beginPrint(ctx.tab);
    },
  },
  {
    id: 'file.closeTab',
    title: 'Close Tab',
    group: 'File',
    shortcut: 'mod+w',
    enabled: (ctx) => ctx.tab !== null,
    run: (ctx) => {
      if (ctx.tab) void closeTab(ctx.tab.id);
    },
  },
  {
    id: 'file.goHome',
    title: 'Go to Home Screen',
    group: 'File',
    keywords: ['start', 'library', 'recents'],
    enabled: () => true,
    run: () => goHome(),
  },

  // ---- Edit ----------------------------------------------------------
  {
    id: 'edit.undo',
    title: 'Undo',
    group: 'Edit',
    shortcut: 'mod+z',
    enabled: loaded,
    run: (ctx) => ctx.tab?.documentStore.undo(),
  },
  {
    id: 'edit.redo',
    title: 'Redo',
    group: 'Edit',
    shortcut: 'mod+shift+z',
    enabled: loaded,
    run: (ctx) => ctx.tab?.documentStore.redo(),
  },
  {
    id: 'edit.deleteSelection',
    title: 'Delete Selected Annotation',
    group: 'Edit',
    shortcut: 'backspace',
    enabled: (ctx) => state(ctx)?.selected != null,
    run: (ctx) => {
      const selected = state(ctx)?.selected;
      if (selected) {
        ctx.tab?.documentStore.apply({
          type: 'deleteAnnot',
          pageId: selected.pageId,
          id: selected.annotId,
        });
      }
    },
  },

  // ---- Tools ---------------------------------------------------------
  tool('tools.select', 'Select Tool', 'select', ['pointer', 'arrow']),
  tool('tools.highlight', 'Highlight Tool', 'highlight', ['marker', 'annotate']),
  tool('tools.ink', 'Ink Tool', 'ink', ['pen', 'draw', 'freehand']),
  tool('tools.rect', 'Rectangle Tool', 'rect', ['box', 'square', 'shape']),
  tool('tools.textbox', 'Text Box Tool', 'textbox', ['type', 'label']),
  tool('tools.note', 'Note Tool', 'note', ['comment', 'sticky']),

  // ---- Page ----------------------------------------------------------
  rotate('page.rotateRight', 'Rotate Page Right', 90),
  rotate('page.rotateLeft', 'Rotate Page Left', 270),
  {
    id: 'page.duplicate',
    title: 'Duplicate Page',
    group: 'Page',
    keywords: ['copy'],
    enabled: (ctx) => loaded(ctx) && currentPage(ctx) !== null,
    run: (ctx) => {
      const page = currentPage(ctx);
      if (page) ctx.tab?.documentStore.apply({ type: 'duplicate', pageId: page.id });
    },
  },
  {
    id: 'page.delete',
    title: 'Delete Page',
    group: 'Page',
    keywords: ['remove'],
    // The last page cannot go: a document with no pages is not a document.
    enabled: (ctx) => loaded(ctx) && (state(ctx)?.pages.length ?? 0) > 1,
    disabledReason: 'the document has only one page',
    run: (ctx) => {
      const page = currentPage(ctx);
      if (page) ctx.tab?.documentStore.apply({ type: 'delete', pageId: page.id });
    },
  },
  {
    id: 'page.addBlank',
    title: 'Insert Blank Page',
    group: 'Page',
    keywords: ['new', 'empty', 'add'],
    enabled: loaded,
    run: (ctx) => {
      if (ctx.tab) addBlankPageAfter(ctx.tab.documentStore, ctx.tab.viewport.state.currentPage);
    },
  },

  // ---- View ----------------------------------------------------------
  {
    id: 'view.zoomIn',
    title: 'Zoom In',
    group: 'View',
    shortcut: 'mod+plus',
    enabled: loaded,
    run: (ctx) => ctx.tab?.zoom.zoomStep(1),
  },
  {
    id: 'view.zoomOut',
    title: 'Zoom Out',
    group: 'View',
    shortcut: 'mod+minus',
    enabled: loaded,
    run: (ctx) => ctx.tab?.zoom.zoomStep(-1),
  },
  {
    id: 'view.fitPage',
    title: 'Fit Page',
    group: 'View',
    shortcut: 'mod+0',
    enabled: loaded,
    run: (ctx) => ctx.tab?.zoom.applyFit('fit-page'),
  },
  {
    id: 'view.fitWidth',
    title: 'Fit Width',
    group: 'View',
    enabled: loaded,
    run: (ctx) => ctx.tab?.zoom.applyFit('fit-width'),
  },
  {
    id: 'view.actualSize',
    title: 'Actual Size (100%)',
    group: 'View',
    shortcut: 'mod+shift+0',
    keywords: ['reset', 'zoom'],
    enabled: loaded,
    run: (ctx) => ctx.tab?.zoom.setZoomAnchored(1),
  },
  {
    id: 'view.toggleSidebar',
    title: 'Toggle Sidebar',
    group: 'View',
    keywords: ['hide', 'show', 'panel'],
    enabled: (ctx) => ctx.tab !== null,
    run: () => setUi('sidebarOpen', !ui.sidebarOpen),
  },

  // ---- Panels --------------------------------------------------------
  panel('panel.pages', 'Page Thumbnails', 'pages', ['thumbnails', 'sidebar']),
  panel('panel.outline', 'Table of Contents', 'outline', ['toc', 'outline', 'sections']),
  panel('panel.formal', 'Theorems & Definitions', 'formal', ['lemma', 'proof', 'environments']),
  panel('panel.figures', 'Figures & Tables', 'figures', ['captions', 'plots']),
  // Markdown export lives on this panel's own button, so the command that
  // reaches it is the panel itself rather than a duplicate of its logic.
  panel('panel.notes', 'Notes & Annotations', 'notes', ['export', 'markdown', 'comments']),

  // ---- Settings ------------------------------------------------------
  {
    id: 'settings.themeLight',
    title: 'Theme: Light',
    group: 'Settings',
    keywords: ['appearance'],
    enabled: () => settings.theme !== 'light',
    disabledReason: 'already active',
    run: () => updateSettings({ theme: 'light' }),
  },
  {
    id: 'settings.themeDark',
    title: 'Theme: Dark',
    group: 'Settings',
    keywords: ['appearance', 'night'],
    enabled: () => settings.theme !== 'dark',
    disabledReason: 'already active',
    run: () => updateSettings({ theme: 'dark' }),
  },
  {
    id: 'settings.themeSystem',
    title: 'Theme: Match System',
    group: 'Settings',
    keywords: ['appearance', 'auto'],
    enabled: () => settings.theme !== 'system',
    disabledReason: 'already active',
    run: () => updateSettings({ theme: 'system' }),
  },
  {
    id: 'settings.toggleLowMemory',
    title: 'Toggle Low-Memory Mode',
    group: 'Settings',
    keywords: ['cache', 'performance', 'ram'],
    enabled: () => true,
    run: () => updateSettings({ lowMemory: !settings.lowMemory }),
  },

  // ---- Search --------------------------------------------------------
  {
    id: 'search.findInDocument',
    title: 'Find in Document…',
    group: 'Search',
    shortcut: 'mod+f',
    keywords: ['search'],
    enabled: loaded,
    run: (ctx) => ctx.tab?.viewport.setState('searchOpen', true),
  },
  {
    id: 'search.library',
    title: 'Search Library…',
    group: 'Search',
    // The library lives on the home screen; this is the route to it rather
    // than a second copy of that panel.
    keywords: ['find', 'papers', 'everything'],
    enabled: () => true,
    run: () => goHome(),
  },

  // ---- Navigate ------------------------------------------------------
  {
    id: 'nav.nextPage',
    title: 'Next Page',
    group: 'Navigate',
    shortcut: 'pagedown',
    enabled: loaded,
    run: (ctx) => {
      const tab = ctx.tab;
      const s = state(ctx);
      if (!tab || !s) return;
      tab.viewport.requestScrollToPage(
        Math.min(tab.viewport.state.currentPage + 1, s.pages.length - 1)
      );
    },
  },
  {
    id: 'nav.previousPage',
    title: 'Previous Page',
    group: 'Navigate',
    shortcut: 'pageup',
    enabled: loaded,
    run: (ctx) => {
      const tab = ctx.tab;
      if (!tab) return;
      tab.viewport.requestScrollToPage(Math.max(tab.viewport.state.currentPage - 1, 0));
    },
  },
  {
    id: 'nav.nextTab',
    title: 'Next Tab',
    group: 'Navigate',
    shortcut: 'ctrl+tab',
    enabled: () => true,
    run: () => cycleTab(1),
  },
  {
    id: 'nav.previousTab',
    title: 'Previous Tab',
    group: 'Navigate',
    shortcut: 'ctrl+shift+tab',
    enabled: () => true,
    run: () => cycleTab(-1),
  },
];

const BY_ID = new Map(COMMANDS.map((command) => [command.id, command]));

export function commandById(id: string): Command | undefined {
  return BY_ID.get(id);
}

/** Run a command by id, honouring its own `enabled` check.
 *
 * Returns whether it ran, so a caller that must swallow the key regardless
 * (⌘P, which the webview would otherwise answer with its own print dialog)
 * can tell the two cases apart.
 */
export function runCommand(id: string, ctx: CommandCtx): boolean {
  const command = BY_ID.get(id);
  if (!command || !command.enabled(ctx)) return false;
  command.run(ctx);
  return true;
}

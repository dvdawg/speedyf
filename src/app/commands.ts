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
import { newWindow } from '../features/windows/windowController';
import { saveDocument } from '../features/document/controller';
import {
  closeOtherTabs,
  closeTab,
  closeTabsToTheRight,
  cycleTab,
  detachTab,
  goHome,
  openFromDialog,
} from '../features/document/tabsController';
import { addBlankPageAfter } from '../features/editor/editorActions';
import { setActiveTool, type Tool } from '../features/annotations/toolStore';
import { beginPrint } from '../features/print/printStore';
import { settings, updateSettings } from '../stores/settings';
import { forgetContributedText } from '../features/extensions/extensionStore';
import {
  activePaper,
  activePaperKnown,
  refreshActivePaper,
} from '../features/projects/activePaper';
import { engine } from '../lib/transport/engine';
import { openQuickOpen } from '../features/palette/quickOpenStore';
import { citeSelection, copyCite, copyListSelection } from '../features/projects/citeSelection';
import { setUi, ui, type SidebarMode } from '../stores/uiStore';
import { tabsStore, type TabRecord } from '../stores/tabsStore';
import type { Paper } from '../types/engine';

/** Listed rather than only typed so a test can check every group is used —
 * an empty heading is a palette section nobody can reach. */
export const COMMAND_GROUPS = [
  'File',
  'Edit',
  'Tools',
  'Page',
  'View',
  'Panels',
  'Library',
  'Settings',
  'Search',
  'Navigate',
] as const;

export type CommandGroup = (typeof COMMAND_GROUPS)[number];

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

export { formatShortcut } from './shortcutLabel';

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

/** Open the Paper panel, where the cite key and the BibTeX entry are visible
 * with Copy buttons of their own. The fallback for every Library command that
 * could not finish: there is no status-bar API for ad hoc text, and the worst
 * outcome of a copy is a silent one. */
function showPaperPanel(): void {
  setUi({ sidebarOpen: true, sidebarMode: 'paper' });
}

const hasIdentifier = (paper: Paper | null) =>
  paper !== null && (paper.doi !== null || paper.arxiv !== null);

/** What a lookup can be run for: a record the scan (or the user) takes for a
 * paper, with something to send. A manual in the library has neither. */
const lookupable = (paper: Paper | null) => hasIdentifier(paper) && paper!.isPaper;

/** The active document's paper record: the cached one when the Paper panel
 * has fetched it, otherwise a round trip. Null when the file is outside the
 * library or the engine could not say. */
async function paperForActiveTab(): Promise<Paper | null> {
  if (activePaperKnown()) return activePaper();
  try {
    return await refreshActivePaper();
  } catch {
    return null;
  }
}

/** Copy something derived from the active paper.
 *
 * A clipboard write only succeeds inside the user gesture that asked for it,
 * and WebKit's window for that is shorter than a round trip to the engine
 * (EXTENSIONS.md, "Clipboard timing"). So the common path is synchronous: the
 * Paper panel keeps the record cached and the key is written at once. When
 * nothing is cached — the panel has never been shown for this document — or
 * when the text itself needs the engine, as BibTeX does, the write lands after
 * an await and may be refused. That case, and a file with no record at all,
 * both end in the Paper panel, where the same text can be copied by hand.
 */
async function copyFromPaper(text: (paper: Paper) => string | Promise<string>): Promise<void> {
  const paper = await paperForActiveTab();
  if (!paper) {
    showPaperPanel();
    return;
  }
  let value: string;
  try {
    value = await text(paper);
  } catch {
    showPaperPanel();
    return;
  }
  try {
    await navigator.clipboard.writeText(value);
  } catch {
    showPaperPanel();
  }
}

/** \cite{} for the Projects list's selection when the list is showing
 * and has one, else for the open document's item. The selection's keys are
 * in memory, so that write lands inside the key press; the open item's goes
 * through `paperForActiveTab` as Copy Cite Key does. Keys are pinned after
 * the write (citeSelection.ts). */
async function copyCiteSelection(ctx: CommandCtx): Promise<void> {
  if (ctx.tab === null && copyListSelection()) return;
  const paper = await paperForActiveTab();
  if (!paper || !(await copyCite([paper]))) showPaperPanel();
}

async function lookUpActivePaper(): Promise<void> {
  const paper = await paperForActiveTab();
  if (!paper || !lookupable(paper)) {
    showPaperPanel();
    return;
  }
  try {
    // The result arrives as a change event; nothing to show here.
    await engine.libraryLookup(paper.id, 'id');
  } catch {
    showPaperPanel();
  }
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
  {
    id: 'file.newWindow',
    title: 'New Window',
    group: 'File',
    shortcut: 'mod+n',
    keywords: ['window', 'new'],
    enabled: () => true,
    run: () => void newWindow(),
  },
  {
    id: 'file.detachTab',
    title: 'Move Tab to New Window',
    group: 'File',
    keywords: ['detach', 'tear off', 'window'],
    enabled: (ctx) => loaded(ctx) && !ctx.tab?.opening && !state(ctx)?.saving,
    run: (ctx) => {
      if (ctx.tab) void detachTab(ctx.tab.id);
    },
  },
  // ---- File ----------------------------------------------------------
  {
    id: 'file.open',
    title: 'Open PDF...',
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
    title: 'Save As...',
    group: 'File',
    shortcut: 'mod+shift+s',
    enabled: loaded,
    run: (ctx) => {
      if (ctx.tab) void saveDocument(ctx.tab, true);
    },
  },
  {
    id: 'file.print',
    title: 'Print...',
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
    id: 'file.closeOtherTabs',
    title: 'Close Other Tabs',
    group: 'File',
    keywords: ['tidy', 'clean up', 'rest'],
    enabled: (ctx) => ctx.tab !== null && tabsStore.state.tabs.length > 1,
    disabledReason: 'there are no other tabs',
    run: (ctx) => {
      if (ctx.tab) void closeOtherTabs(ctx.tab.id);
    },
  },
  {
    id: 'file.closeTabsToTheRight',
    title: 'Close Tabs to the Right',
    group: 'File',
    keywords: ['tidy', 'clean up', 'after'],
    enabled: (ctx) => {
      if (!ctx.tab) return false;
      const index = tabsStore.state.tabs.findIndex((t) => t.id === ctx.tab!.id);
      return index >= 0 && index < tabsStore.state.tabs.length - 1;
    },
    disabledReason: 'this is the last tab',
    run: (ctx) => {
      if (ctx.tab) void closeTabsToTheRight(ctx.tab.id);
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
  tool('tools.bubble', 'Shade Bubbles', 'bubble', [
    'checkbox',
    'radio',
    'quiz',
    'multiple choice',
    'fill in',
  ]),

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

  // ---- Library -------------------------------------------------------
  {
    id: 'library.quickOpen',
    title: 'Open Item...',
    group: 'Library',
    shortcut: 'mod+shift+o',
    keywords: ['quick', 'open', 'switch', 'find', 'paper', 'cite key', 'author', 'title', 'go to'],
    enabled: () => true,
    run: openQuickOpen,
  },
  {
    id: 'library.copyCiteKey',
    title: 'Copy Cite Key',
    group: 'Library',
    shortcut: 'mod+shift+c',
    keywords: ['citation', 'bibtex', 'key', 'clipboard', 'latex'],
    enabled: loaded,
    run: () => void copyFromPaper((paper) => paper.citeKey),
  },
  {
    id: 'library.copyCiteSelection',
    title: 'Copy \\cite for Selection',
    group: 'Library',
    shortcut: 'mod+alt+c',
    keywords: ['citation', 'cite', 'key', 'keys', 'clipboard', 'latex', 'selected', 'items'],
    enabled: (ctx) => (ctx.tab === null && citeSelection().length > 0) || loaded(ctx),
    disabledReason: 'needs items selected on the Projects screen, or an open document',
    run: (ctx) => void copyCiteSelection(ctx),
  },
  {
    id: 'library.copyBibtex',
    title: 'Copy BibTeX',
    group: 'Library',
    keywords: ['citation', 'entry', 'clipboard', 'latex', 'reference'],
    enabled: loaded,
    run: () => void copyFromPaper((paper) => engine.libraryBibtex([paper.id])),
  },
  {
    id: 'library.showPaperPanel',
    title: 'Show Library Panel',
    group: 'Library',
    shortcut: 'mod+shift+l',
    keywords: ['library', 'projects', 'cite key', 'bibtex', 'sidebar', 'metadata'],
    enabled: (ctx) => ctx.tab !== null,
    run: showPaperPanel,
  },
  {
    id: 'library.lookup',
    title: 'Look Up Metadata',
    group: 'Library',
    keywords: ['crossref', 'arxiv', 'doi', 'online', 'fetch', 'authors', 'year'],
    // Optimistic while the record is unknown: refusing would hide the command
    // from everyone who has not opened the Paper panel yet, and the run ends
    // in that panel anyway when the file turns out to be no paper or to have
    // no identifier.
    enabled: (ctx) =>
      settings.onlineLookup && loaded(ctx) && (!activePaperKnown() || lookupable(activePaper())),
    disabledReason: 'needs an open paper with a DOI or arXiv id, and online lookup on in Settings',
    run: () => void lookUpActivePaper(),
  },
  {
    id: 'library.goToProjects',
    title: 'Go to Projects',
    group: 'Library',
    keywords: ['launch', 'screen', 'papers', 'library', 'bib'],
    enabled: () => true,
    run: () => {
      updateSettings({ launchScreen: 'projects' });
      goHome();
    },
  },
  {
    id: 'library.switchLaunchScreen',
    title: 'Switch Launch Screen',
    group: 'Library',
    keywords: ['home', 'projects', 'toggle', 'start', 'recents'],
    enabled: () => true,
    run: () => {
      updateSettings({ launchScreen: settings.launchScreen === 'projects' ? 'home' : 'projects' });
      goHome();
    },
  },

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
    id: 'tools.forgetContributedText',
    title: 'Forget Text Contributed By Extensions',
    group: 'Tools',
    keywords: ['ocr', 'sidecar', 'recognised', 'discard'],
    enabled: loaded,
    run: (ctx) => {
      const path = state(ctx)?.path;
      if (path) void forgetContributedText(path);
    },
  },
  {
    id: 'settings.toggleInvertPages',
    title: 'Toggle Inverted Page Colors',
    group: 'Settings',
    keywords: ['dark', 'night', 'invert', 'appearance', 'contrast'],
    enabled: () => true,
    run: () => updateSettings({ invertPages: !settings.invertPages }),
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
    title: 'Find in Document...',
    group: 'Search',
    shortcut: 'mod+f',
    keywords: ['search'],
    enabled: loaded,
    run: (ctx) => ctx.tab?.viewport.setState('searchOpen', true),
  },
  {
    id: 'search.library',
    title: 'Search Library...',
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

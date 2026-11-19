/** Right-click menus over a document: on a page (and on selected text), and
 * on a page's thumbnail. Each entry is something the app already does
 * elsewhere - a command from the registry, the highlight tool's mark, the
 * Find panel, an extension's selection action - reached from where the
 * pointer is. */
import { commandEntry } from '../../app/commandMenu';
import { SEPARATOR, type MenuEntry } from '../../lib/nativeMenu';
import { selectedRunRects } from '../../lib/text/copySelection';
import type { TabRecord } from '../../stores/tabsStore';
import { selectedText, settleSelectionNow } from '../../stores/selectionStore';
import { allows } from '../protect/permissions';
import { toolState } from '../annotations/toolStore';
import { newAnnotId, addBlankPageAfter } from '../editor/editorActions';
import { runExtensionAction, selectionActions } from '../extensions/extensionStore';
import { jumpToPage } from './navHistory';
import type { PdfQuad, PdfRect } from '../../types/model';

/** Longest selection named in "Find ..." before it is cut short. */
const FIND_LABEL_CHARS = 28;

/** One highlight per page from the selected runs' rectangles (page points,
 * y up - the space annotations live in). Pure. */
export function highlightsFrom(
  runs: readonly { page: number; rect: readonly [number, number, number, number] }[]
): { page: number; rect: PdfRect; quads: PdfQuad[] }[] {
  const byPage = new Map<number, { rect: PdfRect; quads: PdfQuad[] }>();
  for (const { page, rect } of runs) {
    const [x, y, w, h] = rect;
    if (w <= 0 || h <= 0) continue;
    const quad: PdfQuad = {
      p1: { x, y },
      p2: { x: x + w, y },
      p3: { x: x + w, y: y + h },
      p4: { x, y: y + h },
    };
    const entry = byPage.get(page);
    if (!entry) {
      byPage.set(page, { rect: { x, y, w, h }, quads: [quad] });
      continue;
    }
    const r = entry.rect;
    const left = Math.min(r.x, x);
    const bottom = Math.min(r.y, y);
    entry.rect = {
      x: left,
      y: bottom,
      w: Math.max(r.x + r.w, x + w) - left,
      h: Math.max(r.y + r.h, y + h) - bottom,
    };
    entry.quads.push(quad);
  }
  return [...byPage].map(([page, value]) => ({ page, ...value }));
}

function highlightSelection(tab: TabRecord, selection: Selection, scroller: HTMLElement): void {
  const store = tab.documentStore;
  const style = toolState.styles.highlight;
  for (const mark of highlightsFrom(selectedRunRects(selection, scroller))) {
    const page = store.state.pages[mark.page];
    if (!page) continue;
    store.apply({
      type: 'addAnnot',
      annot: {
        id: newAnnotId(),
        pageId: page.id,
        kind: 'highlight',
        rect: mark.rect,
        color: style.color,
        opacity: style.opacity,
        quads: mark.quads,
      },
    });
  }
  selection.removeAllRanges();
}

function shorten(text: string): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > FIND_LABEL_CHARS ? `${flat.slice(0, FIND_LABEL_CHARS - 3)}...` : flat;
}

/** The live selection, when it lies in this document's pages. */
function selectionIn(scroller: HTMLElement): Selection | null {
  const selection = document.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  return scroller.contains(selection.getRangeAt(0).commonAncestorContainer) ? selection : null;
}

/** The menu for a right-click on the pages of `tab`. `onFormField` adds what
 * applies to a form (a checkbox or a choice; text fields keep the system's
 * own Cut, Copy and Paste menu). */
export function pageMenuEntries(
  tab: TabRecord,
  scroller: HTMLElement,
  opts: { onFormField?: boolean } = {}
): MenuEntry[] {
  const ctx = { tab };
  const state = tab.documentStore.state;
  const selection = selectionIn(scroller);
  const entries: MenuEntry[] = [];
  if (selection) {
    settleSelectionNow();
    const text = selectedText() || selection.toString();
    entries.push(
      { kind: 'native', item: 'Copy' },
      {
        text: 'Highlight',
        enabled: state.loaded && allows(state.protection, 'annotate'),
        run: () => highlightSelection(tab, selection, scroller),
      }
    );
    if (text.trim()) {
      entries.push({
        text: `Find "${shorten(text)}"`,
        run: () => {
          tab.searchStore.setQuery(text.replace(/\s+/g, ' ').trim());
          tab.viewport.setState('searchOpen', true);
        },
      });
      const actions = selectionActions();
      if (actions.length > 0) {
        entries.push(SEPARATOR);
        for (const action of actions) {
          entries.push({ text: action.title, run: () => void runExtensionAction(action, text) });
        }
      }
    }
    entries.push(SEPARATOR);
  }
  entries.push(
    ...commandEntry('nav.back', ctx),
    ...commandEntry('nav.forward', ctx),
    SEPARATOR,
    ...commandEntry('tools.note', ctx, 'Add Note'),
    ...commandEntry('tools.highlight', ctx),
    ...(opts.onFormField ? commandEntry('forms.reset', ctx) : []),
    SEPARATOR,
    ...commandEntry('page.rotateRight', ctx),
    ...commandEntry('page.rotateLeft', ctx),
    SEPARATOR,
    ...commandEntry('view.fitPage', ctx),
    ...commandEntry('view.fitWidth', ctx),
    SEPARATOR,
    ...commandEntry('search.findInDocument', ctx),
    ...commandEntry('file.print', ctx)
  );
  return entries;
}

/** The menu for a right-click on the thumbnail of page `index`. The page
 * operations are the ones edit mode puts on each thumbnail. */
export function thumbnailMenuEntries(tab: TabRecord, index: number): MenuEntry[] {
  const store = tab.documentStore;
  const state = store.state;
  const page = state.pages[index];
  if (!page) return [];
  const assemble = state.loaded && allows(state.protection, 'assemble');
  return [
    { text: `Go to Page ${index + 1}`, run: () => jumpToPage(tab, index) },
    SEPARATOR,
    {
      text: 'Rotate Page Right',
      enabled: assemble,
      run: () => store.apply({ type: 'rotate', pageId: page.id, delta: 90 }),
    },
    {
      text: 'Rotate Page Left',
      enabled: assemble,
      run: () => store.apply({ type: 'rotate', pageId: page.id, delta: 270 }),
    },
    {
      text: 'Duplicate Page',
      enabled: assemble,
      run: () => store.apply({ type: 'duplicate', pageId: page.id }),
    },
    {
      text: 'Insert Blank Page After',
      enabled: assemble,
      run: () => addBlankPageAfter(store, index),
    },
    SEPARATOR,
    {
      text: 'Delete Page',
      // The last page cannot go: a document with no pages is not a document.
      enabled: assemble && state.pages.length > 1,
      run: () => store.apply({ type: 'delete', pageId: page.id }),
    },
  ];
}

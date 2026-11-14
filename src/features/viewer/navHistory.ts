/** Browser-style Back/Forward for reading positions.
 *
 * Every deliberate jump (a followed link, an outline row, a search hit, an
 * extension's navigate call, a typed page number) first records where the
 * reader was, so Back returns to exactly that spot: same page, same offset
 * within it, same zoom, same horizontal scroll. Plain scrolling never records
 * anything.
 *
 * Histories are per tab. An entry can name a *different* tab: when a jump
 * opens or focuses another document (a citation opening the cited paper, a
 * Paper panel row, quick open), the source position is pushed onto the
 * target tab's Back stack, so Back there returns to the source tab and
 * Forward from the source re-enters the target.
 *
 * Kept apart from the document's undo/redo: returning to a reading position
 * must never touch the document itself. */
import { createSignal } from 'solid-js';
import { pageIndexAt } from '../../lib/coordinates/layout';
import { VIEW_PADDING, type ViewportStore } from '../../stores/viewportStore';
import { activeTab, setActiveId, tabsStore, type TabRecord } from '../../stores/tabsStore';
import type { FitMode, PageId } from '../../types/model';

export interface NavEntry {
  tabId: string;
  /** The document the position belongs to: its path, or the engine id for a
   * document that has none. A tab reused for another file drops its entries. */
  docKey: string;
  /** Display index when captured; `pageId` and `src` re-find the page after a
   * reorder. */
  page: number;
  pageId: PageId | null;
  src: number | null;
  /** How far down the page the top of the view was, as a fraction of the
   * page's height (may sit slightly outside 0..1 inside the inter-page gap). */
  fraction: number;
  zoom: number;
  fitMode: FitMode;
  scrollLeft: number;
}

/** Where a jump is headed: a page plus a CSS offset down from its top (the
 * viewer's `requestScrollToPage` contract), or an absolute scroll top. */
export type JumpTarget = { page: number; offsetCss?: number | undefined } | { top: number };

export interface JumpOptions {
  /** Skip recording unless the jump moves at least this many CSS px. The
   * default is half a viewport, so a jump to something already on screen
   * does not leave a Back step that goes nowhere visible. */
  minDistance?: number;
}

export const NAV_HISTORY_LIMIT = 100;

interface Stacks {
  back: NavEntry[];
  forward: NavEntry[];
}

const stacks = new Map<string, Stacks>();
const [version, setVersion] = createSignal(0);
const changed = () => setVersion((v) => v + 1);

/** How a cross-tab entry brings its tab forward. tabsController installs its
 * own `activateTab`, which also tells the engine and the session; the default
 * keeps this module usable (and testable) without it. */
let activate: (id: string) => void = (id) => setActiveId(id);
export function setNavTabActivator(fn: (id: string) => void): void {
  activate = fn;
}

function stacksFor(tabId: string): Stacks {
  let s = stacks.get(tabId);
  if (!s) {
    s = { back: [], forward: [] };
    stacks.set(tabId, s);
  }
  return s;
}

function pushCapped(list: NavEntry[], entry: NavEntry): void {
  list.push(entry);
  if (list.length > NAV_HISTORY_LIMIT) list.splice(0, list.length - NAV_HISTORY_LIMIT);
}

function docKeyOf(tab: TabRecord): string {
  const state = tab.documentStore.state;
  return state.path ?? `doc:${state.docId}`;
}

function findTab(id: string): TabRecord | null {
  return tabsStore.state.tabs.find((t) => t.id === id) ?? null;
}

/** The tab owning a viewport, for callers (jumpToAnchor, the citation store)
 * that are handed the viewport rather than the tab. */
export function tabForViewport(vp: ViewportStore): TabRecord | null {
  return tabsStore.state.tabs.find((t) => t.viewport.state === vp.state) ?? null;
}

function valid(entry: NavEntry): boolean {
  const tab = findTab(entry.tabId);
  return !!tab && tab.documentStore.state.loaded && docKeyOf(tab) === entry.docKey;
}

/** The view as it stands, in terms that survive a zoom change or a reorder. */
export function captureView(tab: TabRecord): NavEntry | null {
  const doc = tab.documentStore.state;
  if (!doc.loaded || doc.pages.length === 0) return null;
  const vp = tab.viewport.state;
  const layout = tab.zoom.layoutFor(vp.zoom);
  const page = Math.min(pageIndexAt(layout, vp.scrollTop + VIEW_PADDING), doc.pages.length - 1);
  const top = layout.tops[page];
  const height = layout.heights[page];
  if (top === undefined || !height) return null;
  return {
    tabId: tab.id,
    docKey: docKeyOf(tab),
    page,
    pageId: doc.pages[page]?.id ?? null,
    src: doc.pages[page]?.srcIndex ?? null,
    fraction: (vp.scrollTop - top + VIEW_PADDING) / height,
    zoom: vp.zoom,
    fitMode: vp.fitMode,
    scrollLeft: vp.scrollLeft,
  };
}

/** The page an entry names in the tab as it is now. */
function resolvePage(tab: TabRecord, entry: NavEntry): number {
  const pages = tab.documentStore.state.pages;
  if (entry.pageId !== null && pages[entry.page]?.id === entry.pageId) return entry.page;
  if (entry.pageId !== null) {
    const byId = pages.findIndex((p) => p.id === entry.pageId);
    if (byId >= 0) return byId;
  }
  if (entry.src !== null) {
    const bySrc = pages.findIndex((p) => p.srcIndex === entry.src);
    if (bySrc >= 0) return bySrc;
  }
  return Math.max(0, Math.min(entry.page, pages.length - 1));
}

/** Scroll top a target would land on at the current zoom. */
function targetTop(tab: TabRecord, target: JumpTarget): number | null {
  if ('top' in target) return Math.max(0, target.top);
  const layout = tab.zoom.layoutFor(tab.viewport.state.zoom);
  const top = layout.tops[target.page];
  if (top === undefined) return null;
  return Math.max(0, top - VIEW_PADDING + (target.offsetCss ?? 0));
}

/** Scroll top an entry stands for, when it is in this tab at the current zoom. */
function entryTop(tab: TabRecord, entry: NavEntry): number | null {
  if (entry.tabId !== tab.id || entry.docKey !== docKeyOf(tab)) return null;
  if (Math.abs(entry.zoom - tab.viewport.state.zoom) > 1e-6) return null;
  const layout = tab.zoom.layoutFor(tab.viewport.state.zoom);
  const page = resolvePage(tab, entry);
  const top = layout.tops[page];
  const height = layout.heights[page];
  if (top === undefined || height === undefined) return null;
  return top - VIEW_PADDING + entry.fraction * height;
}

/** Record the current position before a deliberate jump to `target`.
 * Returns whether an entry was pushed. A recorded jump truncates the Forward
 * stack, as a new navigation does in a browser. */
export function recordJump(tab: TabRecord, target?: JumpTarget, opts?: JumpOptions): boolean {
  const vp = tab.viewport.state;
  const threshold = Math.max(opts?.minDistance ?? 0, vp.containerH / 2);
  if (target) {
    const to = targetTop(tab, target);
    if (to !== null && Math.abs(to - vp.scrollTop) < threshold) return false;
  }
  const here = captureView(tab);
  if (!here) return false;
  const s = stacksFor(tab.id);
  // Two jumps fired before the first one has scrolled (two outline rows
  // clicked in quick succession) both start from the same spot: one Back
  // step, not two.
  const last = s.back[s.back.length - 1];
  const lastTop = last ? entryTop(tab, last) : null;
  const duplicate = lastTop !== null && Math.abs(lastTop - vp.scrollTop) < vp.containerH / 2;
  if (!duplicate) pushCapped(s.back, here);
  s.forward.length = 0;
  changed();
  return true;
}

/** `recordJump` for a caller that only holds the viewport. A viewport that
 * belongs to no registered tab (a test double, a print preview) records
 * nothing. */
export function recordJumpForViewport(
  vp: ViewportStore,
  target?: JumpTarget,
  opts?: JumpOptions
): boolean {
  const tab = tabForViewport(vp);
  return tab ? recordJump(tab, target, opts) : false;
}

/** Record, then scroll: the common shape of a deliberate page jump. */
export function jumpToPage(
  tab: TabRecord,
  page: number,
  offsetCss?: number,
  opts?: JumpOptions
): void {
  recordJump(tab, { page, offsetCss }, opts);
  tab.viewport.requestScrollToPage(page, offsetCss);
}

/** A jump that landed in another tab: Back from `targetId` returns to `from`. */
export function noteTabSwitch(from: NavEntry | null, targetId: string): void {
  if (!from || from.tabId === targetId) return;
  const s = stacksFor(targetId);
  pushCapped(s.back, from);
  s.forward.length = 0;
  changed();
}

/** Put a tab's view back where an entry says. */
export function restoreView(tab: TabRecord, entry: NavEntry): void {
  const page = resolvePage(tab, entry);
  const vp = tab.viewport;
  if (Math.abs(vp.state.zoom - entry.zoom) > 1e-6) {
    tab.zoom.setZoomAnchored(entry.zoom, undefined, entry.fitMode);
  } else if (vp.state.fitMode !== entry.fitMode) {
    vp.setState('fitMode', entry.fitMode);
  }
  vp.requestRestoreView(page, entry.fraction, entry.scrollLeft);
}

function hasValid(list: readonly NavEntry[] | undefined): boolean {
  return !!list && list.some(valid);
}

export function canGoBack(tab: TabRecord | null = activeTab()): boolean {
  version();
  return !!tab && hasValid(stacks.get(tab.id)?.back);
}

export function canGoForward(tab: TabRecord | null = activeTab()): boolean {
  version();
  return !!tab && hasValid(stacks.get(tab.id)?.forward);
}

function move(tab: TabRecord | null, dir: 'back' | 'forward'): boolean {
  if (!tab) return false;
  const s = stacks.get(tab.id);
  if (!s) return false;
  const from = dir === 'back' ? s.back : s.forward;
  let entry: NavEntry | undefined;
  while ((entry = from.pop()) !== undefined) {
    if (valid(entry)) break;
  }
  if (!entry) {
    changed();
    return false;
  }
  const target = findTab(entry.tabId)!;
  // Where we are now becomes the step in the opposite direction, kept with
  // the tab being returned to so that tab's Forward (or Back) re-enters here.
  const here = captureView(tab);
  if (here) {
    const dest = stacksFor(target.id);
    pushCapped(dir === 'back' ? dest.forward : dest.back, here);
  }
  if (target.id !== tab.id) activate(target.id);
  restoreView(target, entry);
  changed();
  return true;
}

export function goBack(tab: TabRecord | null = activeTab()): boolean {
  return move(tab, 'back');
}

export function goForward(tab: TabRecord | null = activeTab()): boolean {
  return move(tab, 'forward');
}

/** The mouse's own Back (button 3) and Forward (button 4). Returns whether the
 * button is one of them, so the caller can keep the webview from acting on it. */
export function navigateForMouseButton(button: number, tab: TabRecord | null = activeTab()) {
  if (button === 3) {
    goBack(tab);
    return true;
  }
  if (button === 4) {
    goForward(tab);
    return true;
  }
  return false;
}

/** Drop a closed tab's history. Entries elsewhere that name it are skipped
 * lazily, the next time Back or Forward reaches them. */
export function forgetTab(tabId: string): void {
  if (stacks.delete(tabId)) changed();
}

/** Test hook. */
export function resetNavHistory(): void {
  stacks.clear();
  changed();
}

/** Test hook: the stack depths for a tab. */
export function navDepth(tabId: string): { back: number; forward: number } {
  const s = stacks.get(tabId);
  return { back: s?.back.length ?? 0, forward: s?.forward.length ?? 0 };
}

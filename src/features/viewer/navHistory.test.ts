import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { addTab, createTab, removeTab, setActiveId, tabsStore } from '../../stores/tabsStore';
import type { TabRecord } from '../../stores/tabsStore';
import { VIEW_PADDING } from '../../stores/viewportStore';
import { jumpToAnchor } from '../outline/jumpToAnchor';
import {
  NAV_HISTORY_LIMIT,
  canGoBack,
  canGoForward,
  captureView,
  forgetTab,
  goBack,
  goForward,
  jumpToPage,
  navDepth,
  navigateForMouseButton,
  noteTabSwitch,
  recordJump,
  resetNavHistory,
} from './navHistory';

const PAGES = 20;
const CONTAINER_H = 800;

/** A loaded tab of identical 612x792 pages, laid out at `zoom`. */
function openTab(path: string, zoom = 1): TabRecord {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: tabsStore.state.tabs.length + 1,
    path,
    name: path,
    pageCount: PAGES,
    sizes: Array.from({ length: PAGES }, () => [612, 792, 0, 0, 0]),
    estimatedSize: [612, 792],
  });
  tab.viewport.setState({ zoom, containerW: 1000, containerH: CONTAINER_H, fitMode: 'custom' });
  addTab(tab);
  setActiveId(tab.id);
  return tab;
}

function layout(tab: TabRecord) {
  return tab.zoom.layoutFor(tab.viewport.state.zoom);
}

/** What a user scrolling does: moves the view, records nothing. */
function scrollTo(tab: TabRecord, top: number, left = 0) {
  tab.viewport.setState({ scrollTop: top, scrollLeft: left });
}

/** Stand-in for the Viewer: applies the pending scroll request the way its
 * effect does, against the layout at the zoom in force. */
function land(tab: TabRecord) {
  const req = tab.viewport.takeScrollRequest();
  if (!req) return;
  if (req.kind === 'position') {
    scrollTo(tab, req.top, req.left);
    return;
  }
  const l = layout(tab);
  const top = Math.max(
    0,
    l.tops[req.page]! -
      VIEW_PADDING +
      (req.fraction ?? 0) * l.heights[req.page]! +
      (req.offsetCss ?? 0)
  );
  scrollTo(tab, top, req.left ?? tab.viewport.state.scrollLeft);
}

beforeEach(() => {
  for (const t of [...tabsStore.state.tabs]) removeTab(t.id);
  setActiveId(null);
  resetNavHistory();
});
afterEach(() => resetNavHistory());

describe('recording jumps', () => {
  it('pushes the current position before a link jump', () => {
    const tab = openTab('/a.pdf');
    scrollTo(tab, 1234, 7);
    tab.citationStore.navigateInternalTarget({ kind: 'internal', page: 12, x: null, y: 400 });
    expect(navDepth(tab.id)).toEqual({ back: 1, forward: 0 });
    expect(canGoBack(tab)).toBe(true);
  });

  it('pushes before an outline jump', () => {
    const tab = openTab('/a.pdf');
    scrollTo(tab, 300);
    jumpToAnchor(tab.viewport, tab.documentStore.state, 15, 500);
    expect(navDepth(tab.id).back).toBe(1);
    expect(tab.viewport.state.scrollRequest).not.toBeNull();
  });

  it('never records plain scrolling', () => {
    const tab = openTab('/a.pdf');
    for (const top of [0, 900, 4000, 12000, 50]) scrollTo(tab, top);
    expect(navDepth(tab.id)).toEqual({ back: 0, forward: 0 });
    expect(canGoBack(tab)).toBe(false);
  });

  it('does not record a jump to somewhere already on screen', () => {
    const tab = openTab('/a.pdf');
    const l = layout(tab);
    // Current view starts at page 3's top; the target is 100px further down.
    scrollTo(tab, l.tops[3]! - VIEW_PADDING);
    jumpToPage(tab, 3, 100);
    expect(navDepth(tab.id).back).toBe(0);
    // ...but a page further on is a real jump.
    jumpToPage(tab, 5);
    expect(navDepth(tab.id).back).toBe(1);
  });

  it('honours a larger minimum distance (search stepping)', () => {
    const tab = openTab('/a.pdf');
    const l = layout(tab);
    scrollTo(tab, l.tops[3]! - VIEW_PADDING);
    expect(recordJump(tab, { page: 4 }, { minDistance: l.heights[3]! * 2 })).toBe(false);
    expect(recordJump(tab, { page: 9 }, { minDistance: l.heights[3]! * 2 })).toBe(true);
  });

  it('caps the history', () => {
    const tab = openTab('/a.pdf');
    const l = layout(tab);
    for (let i = 0; i < NAV_HISTORY_LIMIT + 30; i++) {
      jumpToPage(tab, i % 2 === 0 ? 15 : 2);
      land(tab);
    }
    expect(navDepth(tab.id).back).toBe(NAV_HISTORY_LIMIT);
    expect(l.tops.length).toBe(PAGES);
  });
});

describe('back and forward', () => {
  it('restores the exact position and zoom, and comes forward again', () => {
    const tab = openTab('/a.pdf', 1.25);
    scrollTo(tab, 2345.5, 42);
    tab.citationStore.navigateInternalTarget({ kind: 'internal', page: 14, x: null, y: 300 });
    land(tab);
    const target = tab.viewport.state.scrollTop;
    expect(target).not.toBe(2345.5);

    // Zoom changes after the jump; Back must undo that too.
    tab.zoom.setZoomAnchored(2);
    expect(goBack(tab)).toBe(true);
    land(tab);
    expect(tab.viewport.state.zoom).toBe(1.25);
    expect(tab.viewport.state.scrollTop).toBeCloseTo(2345.5, 6);
    expect(tab.viewport.state.scrollLeft).toBe(42);
    expect(canGoBack(tab)).toBe(false);
    expect(canGoForward(tab)).toBe(true);

    expect(goForward(tab)).toBe(true);
    land(tab);
    expect(tab.viewport.state.zoom).toBe(2);
    expect(canGoForward(tab)).toBe(false);
    expect(canGoBack(tab)).toBe(true);
  });

  it('finds the page again after the pages were reordered', () => {
    const tab = openTab('/a.pdf');
    const l = layout(tab);
    const startTop = l.tops[6]! - VIEW_PADDING + 120;
    scrollTo(tab, startTop);
    const before = captureView(tab)!;
    jumpToPage(tab, 18);
    land(tab);
    // Move the page that was being read to the front.
    const pageId = tab.documentStore.state.pages[6]!.id;
    tab.documentStore.apply({ type: 'reorder', pageId, toIndex: 0 });
    expect(tab.documentStore.state.pages[0]!.srcIndex).toBe(before.src);
    goBack(tab);
    land(tab);
    expect(tab.viewport.state.scrollTop).toBeCloseTo(layout(tab).tops[0]! - VIEW_PADDING + 120, 6);
  });

  it('truncates the forward stack when a new jump follows a Back', () => {
    const tab = openTab('/a.pdf');
    jumpToPage(tab, 5);
    land(tab);
    jumpToPage(tab, 10);
    land(tab);
    goBack(tab);
    land(tab);
    expect(navDepth(tab.id)).toEqual({ back: 1, forward: 1 });
    jumpToPage(tab, 17);
    land(tab);
    expect(navDepth(tab.id)).toEqual({ back: 2, forward: 0 });
    expect(canGoForward(tab)).toBe(false);
  });

  it('is a no-op with nothing to go back to', () => {
    const tab = openTab('/a.pdf');
    expect(goBack(tab)).toBe(false);
    expect(goForward(tab)).toBe(false);
    expect(goBack(null)).toBe(false);
  });
});

describe('across tabs', () => {
  it('returns to the source tab at its exact position, and forward re-enters the target', () => {
    const source = openTab('/source.pdf');
    scrollTo(source, 3210, 5);
    const from = captureView(source);
    // A citation opens the cited paper in a tab of its own.
    const target = openTab('/cited.pdf');
    noteTabSwitch(from, target.id);
    scrollTo(target, 900);

    expect(canGoBack(target)).toBe(true);
    expect(goBack(target)).toBe(true);
    expect(tabsStore.state.activeId).toBe(source.id);
    land(source);
    expect(source.viewport.state.scrollTop).toBeCloseTo(3210, 6);
    expect(source.viewport.state.scrollLeft).toBe(5);

    expect(canGoForward(source)).toBe(true);
    expect(goForward(source)).toBe(true);
    expect(tabsStore.state.activeId).toBe(target.id);
    land(target);
    expect(target.viewport.state.scrollTop).toBeCloseTo(900, 6);
    expect(canGoBack(target)).toBe(true);
  });

  it('skips entries for a tab that has been closed', () => {
    const source = openTab('/source.pdf');
    const target = openTab('/cited.pdf');
    noteTabSwitch(captureView(source), target.id);
    removeTab(source.id);
    forgetTab(source.id);
    expect(canGoBack(target)).toBe(false);
    expect(goBack(target)).toBe(false);
    expect(tabsStore.state.activeId).toBe(target.id);
  });

  it('records nothing when the open stayed in the same tab', () => {
    const tab = openTab('/a.pdf');
    noteTabSwitch(captureView(tab), tab.id);
    expect(navDepth(tab.id).back).toBe(0);
  });
});

describe('mouse buttons', () => {
  it('steps back on button 3 and forward on button 4, and ignores the rest', () => {
    const tab = openTab('/a.pdf');
    jumpToPage(tab, 12);
    land(tab);
    const there = tab.viewport.state.scrollTop;
    expect(navigateForMouseButton(0, tab)).toBe(false);
    expect(navigateForMouseButton(1, tab)).toBe(false);
    expect(navigateForMouseButton(3, tab)).toBe(true);
    land(tab);
    expect(tab.viewport.state.scrollTop).toBe(0);
    expect(navigateForMouseButton(4, tab)).toBe(true);
    land(tab);
    expect(tab.viewport.state.scrollTop).toBeCloseTo(there, 6);
  });
});

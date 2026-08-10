import { beforeEach, describe, expect, it } from 'vitest';
import { createMemo, createRoot } from 'solid-js';
import { unwrap } from 'solid-js/store';
import {
  activeTab,
  addTab,
  createTab,
  findTabByPath,
  removeTab,
  reorderTabs,
  setActiveId,
  setTabOpening,
  tabsStore,
} from './tabsStore';

// tabsStore is a module-level singleton (one tab registry for the whole app),
// so each test resets it back to empty rather than constructing a fresh
// instance the way documentStore.test.ts does with its factory.
beforeEach(() => {
  for (const t of [...tabsStore.state.tabs]) removeTab(t.id);
  setActiveId(null);
});

describe('addTab / removeTab', () => {
  it('appends tabs in order and removes by id', () => {
    const a = createTab();
    const b = createTab();
    addTab(a);
    addTab(b);
    expect(tabsStore.state.tabs.map((t) => t.id)).toEqual([a.id, b.id]);

    removeTab(a.id);
    expect(tabsStore.state.tabs.map((t) => t.id)).toEqual([b.id]);
  });

  it('is a no-op removing an id that is not present', () => {
    const a = createTab();
    addTab(a);
    removeTab('not-a-real-id');
    expect(tabsStore.state.tabs).toHaveLength(1);
  });
});

describe('activeTab / setActiveId', () => {
  it('resolves the tab matching activeId, or null', () => {
    const a = createTab();
    addTab(a);
    expect(activeTab()).toBeNull();

    setActiveId(a.id);
    expect(activeTab()?.id).toBe(a.id);

    setActiveId(null);
    expect(activeTab()).toBeNull();
  });

  it('clears to null when the last tab closes (close-last-tab-returns-home)', () => {
    const a = createTab();
    addTab(a);
    setActiveId(a.id);
    removeTab(a.id);
    // removeTab alone doesn't touch activeId — that's tabsController's job —
    // but activeTab() must reflect that the id no longer resolves.
    expect(activeTab()).toBeNull();
    setActiveId(null);
    expect(tabsStore.state.activeId).toBeNull();
  });
});

describe('findTabByPath', () => {
  it('finds the tab whose loaded document matches the given path', () => {
    const a = createTab();
    const b = createTab();
    addTab(a);
    addTab(b);
    expect(findTabByPath('/tmp/a.pdf')).toBeNull();

    a.documentStore.initFromMeta({
      docId: 1,
      path: '/tmp/a.pdf',
      name: 'a.pdf',
      pageCount: 1,
      sizes: [[612, 792, 0, 0, 0]],
      estimatedSize: [612, 792],
    });
    expect(findTabByPath('/tmp/a.pdf')?.id).toBe(a.id);
    expect(findTabByPath('/tmp/b.pdf')).toBeNull();
  });
});

describe('setTabOpening', () => {
  it('updates only the matching tab, reactively', () => {
    const a = createTab();
    const b = createTab();
    addTab(a);
    addTab(b);
    setTabOpening(a.id, true);
    expect(tabsStore.state.tabs.find((t) => t.id === a.id)?.opening).toBe(true);
    expect(tabsStore.state.tabs.find((t) => t.id === b.id)?.opening).toBe(false);

    setTabOpening(a.id, false);
    expect(tabsStore.state.tabs.find((t) => t.id === a.id)?.opening).toBe(false);
  });
});

// A TabRecord holds live sub-stores, and Solid's store setter runs unwrap()
// over whatever you hand it — which walks plain objects and swaps any nested
// store proxy for its raw target IN PLACE. That used to strip
// `viewport.state` / `documentStore.state` the moment a tab was registered,
// leaving every holder of a direct reference (zoomController and citationStore
// both capture their sibling stores at createTab() time) reading dead,
// untracked objects.
describe('registering a tab preserves its sub-stores', () => {
  it('leaves the sub-store state proxies reactive', () => {
    const a = createTab();
    addTab(a);
    expect(unwrap(a.viewport.state)).not.toBe(a.viewport.state);
    expect(unwrap(a.documentStore.state)).not.toBe(a.documentStore.state);
  });

  it('keeps zoomController geometry reactive to view rotation', () => {
    const a = createTab();
    addTab(a);
    setActiveId(a.id);
    a.documentStore.initFromMeta({
      docId: 1,
      path: '/tmp/a.pdf',
      name: 'a.pdf',
      pageCount: 1,
      sizes: [[600, 800, 0, 0, 0]],
      estimatedSize: [600, 800],
    });

    /* eslint-disable solid/reactivity -- assertions read the memos directly,
       which is the point: they must recompute after rotateView(). */
    createRoot((dispose) => {
      // Mirrors ViewerContent: geoms/layout memos over the controller the tab
      // was built with. If these stop tracking, "Rotate view 90°" changes
      // viewRotation but nothing re-lays-out or re-renders.
      const geoms = createMemo(() => a.zoom.pagesGeom());
      const layout = createMemo(() => a.zoom.layoutFor(a.viewport.state.zoom, geoms()));
      expect(geoms()[0]!.rotation).toBe(0);
      expect([layout().widths[0], layout().heights[0]]).toEqual([600, 800]);

      a.viewport.rotateView();

      expect(a.viewport.state.viewRotation).toBe(90);
      expect(geoms()[0]!.rotation).toBe(90);
      expect([layout().widths[0], layout().heights[0]]).toEqual([800, 600]);
      dispose();
    });
    /* eslint-enable solid/reactivity */
  });

  it('keeps zoomController geometry reactive to page-list edits', () => {
    const a = createTab();
    addTab(a);
    a.documentStore.initFromMeta({
      docId: 1,
      path: '/tmp/a.pdf',
      name: 'a.pdf',
      pageCount: 3,
      sizes: [
        [600, 800, 0, 0, 0],
        [600, 800, 0, 0, 0],
        [600, 800, 0, 0, 0],
      ],
      estimatedSize: [600, 800],
    });

    /* eslint-disable solid/reactivity -- see above */
    createRoot((dispose) => {
      const geoms = createMemo(() => a.zoom.pagesGeom());
      expect(geoms()).toHaveLength(3);

      a.documentStore.apply({ type: 'delete', pageId: a.documentStore.state.pages[0]!.id });

      expect(geoms()).toHaveLength(2);
      dispose();
    });
    /* eslint-enable solid/reactivity */
  });
});

describe('reorderTabs', () => {
  it('moves a tab from one index to another', () => {
    const a = createTab();
    const b = createTab();
    const c = createTab();
    addTab(a);
    addTab(b);
    addTab(c);

    reorderTabs(0, 2);
    expect(tabsStore.state.tabs.map((t) => t.id)).toEqual([b.id, c.id, a.id]);
  });

  it('is a no-op when fromIndex is out of range', () => {
    const a = createTab();
    addTab(a);
    reorderTabs(5, 0);
    expect(tabsStore.state.tabs.map((t) => t.id)).toEqual([a.id]);
  });
});

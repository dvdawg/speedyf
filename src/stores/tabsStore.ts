/** Tab registry: one TabRecord per open document, each holding its own
 * independent document/viewport/zoom/citation/search stores. Only tab
 * lifecycle (add/remove/reorder/activate) lives here — actual open/close/
 * save orchestration lives in features/document/tabsController.ts. */
import { createStore } from 'solid-js/store';
import { createDocumentStore, type DocumentStore } from '../features/document/documentStore';
import { createViewportStore, type ViewportStore } from './viewportStore';
import { createZoomController, type ZoomController } from '../features/viewer/zoomController';
import { createCitationStore, type CitationStore } from '../features/citations/linkStore';
import { createSearchStore, type SearchStore } from '../features/search/searchStore';

export interface TabRecord {
  id: string;
  documentStore: DocumentStore;
  viewport: ViewportStore;
  zoom: ZoomController;
  citationStore: CitationStore;
  searchStore: SearchStore;
  /** true from creation until the open resolves (success or failure) */
  opening: boolean;
}

let tabSeq = 0;

/** Builds a fresh, empty tab (no document loaded yet) with its own
 * independent set of per-document stores. */
export function createTab(): TabRecord {
  const documentStore = createDocumentStore();
  const viewport = createViewportStore();
  const zoom = createZoomController(documentStore, viewport);
  const citationStore = createCitationStore(documentStore, viewport);
  const searchStore = createSearchStore();
  return {
    id: `tab-${++tabSeq}`,
    documentStore,
    viewport,
    zoom,
    citationStore,
    searchStore,
    opening: false,
  };
}

const [state, setState] = createStore<{ tabs: TabRecord[]; activeId: string | null }>({
  tabs: [],
  activeId: null,
});

export const tabsStore = { state };

export function activeTab(): TabRecord | null {
  return state.tabs.find((t) => t.id === state.activeId) ?? null;
}

export function findTabByPath(path: string): TabRecord | null {
  return state.tabs.find((t) => t.documentStore.state.path === path) ?? null;
}

// --- Low-level mutators, used only by tabsController.ts --------------------

export function addTab(record: TabRecord): void {
  setState('tabs', (tabs) => [...tabs, record]);
}

export function removeTab(id: string): void {
  setState('tabs', (tabs) => tabs.filter((t) => t.id !== id));
}

export function setActiveId(id: string | null): void {
  setState('activeId', id);
}

export function setTabOpening(id: string, opening: boolean): void {
  setState('tabs', (t) => t.id === id, 'opening', opening);
}

export function reorderTabs(fromIndex: number, toIndex: number): void {
  setState('tabs', (tabs) => {
    const next = [...tabs];
    const [moved] = next.splice(fromIndex, 1);
    if (!moved) return tabs;
    next.splice(toIndex, 0, moved);
    return next;
  });
}

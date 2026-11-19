/** Redaction UI state: the Redact panel (search-and-redact), the overlay text
 * new marks carry, and applying marks. Marks themselves live in the document
 * model as `redact` annotations; applying is a model edit too (`setScrub`),
 * carried out - irreversibly - by the next save. */
import { createStore, produce } from 'solid-js/store';
import { engine } from '../../../lib/transport/engine';
import { activeTab, type TabRecord } from '../../../stores/tabsStore';
import { askConfirm, showError } from '../../../stores/modalStore';
import { selectedGeometry, settleSelectionNow } from '../../../stores/selectionStore';
import type { DocumentStore } from '../../document/documentStore';
import { toolState } from '../../annotations/toolStore';
import type { FindQuery, RedactMatch, RedactPattern } from '../types';
import { addMarks, markCount, markFromRects, type MarkShape, type MarkStyle } from './redactMarks';

export const PATTERNS: { kind: RedactPattern; label: string }[] = [
  { kind: 'email', label: 'Email addresses' },
  { kind: 'phone', label: 'Phone numbers' },
  { kind: 'ssn', label: 'US Social Security numbers' },
  { kind: 'card', label: 'Credit card numbers' },
  { kind: 'date', label: 'Dates' },
];

interface RedactState {
  panelOpen: boolean;
  query: FindQuery;
  running: boolean;
  /** null until a search has run */
  results: RedactMatch[] | null;
  chosen: boolean[];
  /** Overlay text new marks carry; empty for none. */
  overlay: string;
}

const [redactState, setRedactState] = createStore<RedactState>({
  panelOpen: false,
  query: { phrase: '', caseSensitive: false, wholeWords: false, patterns: [] },
  running: false,
  results: null,
  chosen: [],
  overlay: '',
});

export { redactState };

export function openRedactPanel(open = true) {
  setRedactState('panelOpen', open);
}

export function setOverlay(text: string) {
  setRedactState('overlay', text);
}

export function setQuery(patch: Partial<FindQuery>) {
  setRedactState(
    produce((s) => {
      Object.assign(s.query, patch);
    })
  );
}

export function togglePattern(kind: RedactPattern, on: boolean) {
  setRedactState(
    produce((s) => {
      s.query.patterns = s.query.patterns.filter((p) => p !== kind);
      if (on) s.query.patterns.push(kind);
    })
  );
}

export function setChosen(index: number, on: boolean) {
  setRedactState('chosen', index, on);
}

export function setAllChosen(on: boolean) {
  setRedactState('chosen', (c) => c.map(() => on));
}

/** The style new marks get: the Redact tool's colour, the panel's overlay. */
export function markStyle(): MarkStyle {
  return { color: toolState.styles.redact.color, overlay: redactState.overlay };
}

export async function runFind(tab: TabRecord | null = activeTab()) {
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded) return;
  const q = redactState.query;
  if (!q.phrase.trim() && q.patterns.length === 0) {
    setRedactState({ results: null, chosen: [] });
    return;
  }
  setRedactState('running', true);
  try {
    const results = await engine.findRedactions(doc.docId, {
      ...q,
      patterns: [...q.patterns],
    });
    setRedactState({ results, chosen: results.map(() => true), running: false });
  } catch (e) {
    setRedactState('running', false);
    showError(`Search failed: ${String(e)}`);
  }
}

/** Display-space `[x, y, w, h]` rectangles (y up) as a mark. */
function shapeOf(rects: readonly [number, number, number, number][]): MarkShape | null {
  return markFromRects(rects.map(([x, y, w, h]) => ({ x, y, w, h })));
}

/** Mark the chosen search results. Returns how many marks were added. */
export function markChosen(store: DocumentStore): number {
  const results = redactState.results ?? [];
  const marks: { pageId: string; shape: MarkShape }[] = [];
  results.forEach((m, i) => {
    if (!redactState.chosen[i]) return;
    // Every page showing this source page (a duplicated page included).
    for (const page of store.state.pages.filter((p) => p.srcIndex === m.page)) {
      const shape = shapeOf(m.rects);
      if (shape) marks.push({ pageId: page.id, shape });
    }
  });
  const n = addMarks(store, marks, markStyle());
  if (n > 0) setRedactState({ results: null, chosen: [] });
  return n;
}

/** Mark the current text selection. Returns false when there is none. */
export function markSelection(tab: TabRecord | null = activeTab()): boolean {
  if (!tab) return false;
  settleSelectionNow();
  const geometry = selectedGeometry();
  const doc = tab.documentStore.state;
  if (!geometry || geometry.docId !== doc.docId) return false;
  const groups = [{ page: geometry.page, rects: geometry.rects }];
  if (geometry.start) groups.push(geometry.start);
  const marks: { pageId: string; shape: MarkShape }[] = [];
  for (const g of groups) {
    const page = doc.pages[g.page];
    if (!page) continue;
    // Selection rectangles run from the page's top-left; marks are y-up.
    const shape = markFromRects(
      g.rects.map(([x, top, w, h]) => ({ x, y: page.heightPt - top - h, w, h }))
    );
    if (shape) marks.push({ pageId: page.id, shape });
  }
  if (marks.length === 0) return false;
  addMarks(tab.documentStore, marks, markStyle());
  document.getSelection()?.removeAllRanges();
  return true;
}

/** Arm applying every mark at the next save, after a confirmation. */
export async function applyRedactions(tab: TabRecord | null = activeTab()): Promise<boolean> {
  const store = tab?.documentStore;
  if (!store?.state.loaded) return false;
  const count = markCount(store);
  if (count === 0) {
    showError(
      'There are no redaction marks to apply. Mark text or areas with the Redact tool first.'
    );
    return false;
  }
  let signed = false;
  try {
    signed = (await engine.scanHiddenInfo(store.state.docId)).signed;
  } catch {
    // The warning is best effort; the confirmation still stands.
  }
  const message =
    `Apply ${count} redaction mark${count === 1 ? '' : 's'}? When you next save, the text, ` +
    'images and drawings under them are permanently removed and the file is rewritten without ' +
    'its earlier versions. Until you save, Undo takes this back; after saving it cannot be undone.' +
    (signed ? ' This document is digitally signed: its signatures will be removed.' : '');
  if (!(await askConfirm(message, 'Apply redactions', { destructive: true }))) return false;
  store.apply({ type: 'setScrub', scrub: { ...store.state.scrub, applyRedactions: true } });
  return true;
}

/** Take back an armed apply before saving. */
export function cancelApply(store: DocumentStore | undefined = activeTab()?.documentStore) {
  if (!store?.state.scrub.applyRedactions) return;
  store.apply({ type: 'setScrub', scrub: { ...store.state.scrub, applyRedactions: false } });
}

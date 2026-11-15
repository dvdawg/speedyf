/** "Papers you may have missed": items in the reviewer's own library the
 * citation graph ties to this paper (shared references, cited together)
 * that the paper does not cite. Offline - the library's own graph, read
 * from the references SpeedyF has parsed for its items - so it needs the
 * paper to be an item of the library. One click turns a suggestion into a
 * literature comment citing it, anchored at the paper's reference list. */
import { engine } from '../../lib/transport/engine';
import type { TabRecord } from '../../stores/tabsStore';
import type { RelatedItem } from '../../types/engine';
import { flattenOutline } from '../outline/structureStore';
import { quadsFromRects } from './geometry';
import type { Anchor } from './types';

export interface Suggestion {
  id: string;
  key: string;
  title: string;
  detail: string;
  reason: string;
}

/** Related items less the ones the paper cites (and itself). */
export function missingFrom(
  paperId: string,
  related: readonly RelatedItem[],
  citedIds: ReadonlySet<string>
): Suggestion[] {
  return related
    .filter((r) => r.row.id !== paperId && !citedIds.has(r.row.id))
    .map((r) => ({
      id: r.row.id,
      key: r.row.citeKey,
      title: r.row.title,
      detail: [r.row.authors, r.row.year].filter(Boolean).join(' - '),
      reason: r.reason,
    }));
}

export async function loadSuggestions(paperId: string): Promise<Suggestion[]> {
  const [related, links] = await Promise.all([
    engine.libraryRelatedItems(paperId),
    engine.libraryCitationLinks(paperId).catch(() => ({ cites: [], citedBy: [], unresolved: 0 })),
  ]);
  return missingFrom(paperId, related, new Set(links.cites.map((row) => row.id)));
}

const HEADING = /^\s*(?:\d+\.?\s*)?(references|bibliography|literature cited)\s*$/i;

/** Where a literature suggestion is anchored: the heading of the paper's
 * reference list when it can be found (the outline first, then the text of
 * the last pages), else the first line of the first page. */
export async function referencesAnchor(tab: TabRecord): Promise<Anchor | null> {
  const doc = tab.documentStore.state;
  if (!doc.loaded) return null;
  const docId = doc.docId;
  const fromOutline = flattenOutline(tab.structureStore.state.outline).find(
    ({ node }) => node.page !== null && HEADING.test(node.title)
  )?.node.page;
  const last = Math.max(0, ...doc.pages.map((p) => p.srcIndex ?? 0));
  const candidates =
    fromOutline !== undefined && fromOutline !== null
      ? [fromOutline]
      : Array.from({ length: Math.min(12, last + 1) }, (_, i) => last - i);
  for (const page of [...candidates, 0]) {
    const layout = await engine.getTextLayout(docId, page).catch(() => null);
    if (!layout || layout.runs.length === 0) continue;
    const run =
      layout.runs.find((r) => HEADING.test(r.text)) ?? (page === 0 ? layout.runs[0] : undefined);
    if (!run) continue;
    const height = doc.pages.find((p) => p.srcIndex === page)?.heightPt;
    if (!height) continue;
    const quads = quadsFromRects([[run.x, height - run.y - run.h, run.w, run.h]], height);
    return { page, quads, rect: null, quote: run.text.trim() };
  }
  return null;
}

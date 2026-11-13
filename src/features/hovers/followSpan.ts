import type { TabRecord } from '../../stores/tabsStore';
import type { HoverSpan } from './hoverTypes';

/** Go to a span's target - or one of a term's definitions - through the
 * same path a followed link takes, so the view left behind goes on the
 * Back stack. */
export function followSpan(tab: TabRecord, span: HoverSpan, entry = 0): void {
  const term = span.kind === 'term' ? span.target.terms[entry] : undefined;
  const page = term ? term.page : span.target.page;
  const rect = term?.rect ?? null;
  const y = rect ? rect[1] + rect[3] : term ? null : span.target.y;
  const x = rect ? rect[0] : term ? null : span.target.x;
  tab.citationStore.navigateInternalTarget({ kind: 'internal', page, x, y });
  tab.citationStore.close();
}

/** The reader's page number for a page in the file, or null when the page
 * was deleted from the document. */
export function readerPage(tab: TabRecord, src: number): number | null {
  const index = tab.documentStore.state.pages.findIndex((page) => page.srcIndex === src);
  return index < 0 ? null : index + 1;
}

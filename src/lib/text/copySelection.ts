/** Bridge from a live DOM selection to the geometry-aware rebuild in
 * `smartCopy`. Kept apart from that module so the heuristics stay pure and
 * testable, and only this thin layer touches the DOM. */
import { smartCopyText, type CopyRun } from './smartCopy';

/** Each selectable span reports the run it draws. A WeakMap keeps this from
 * pinning DOM nodes that page virtualization has already discarded.
 *
 * Registration is one-shot because both inputs are fixed for the life of the
 * span: `<For>` keys its rows by value, so neither the run nor the page index
 * of an already-created span can change under it. */
const runForSpan = new WeakMap<Element, CopyRun>();

export function registerCopyRun(element: Element, run: CopyRun): void {
  runForSpan.set(element, run);
}

/** Collect the selected runs, trimming the two partially-selected spans at
 * the ends so copying half a word does not paste the whole one. */
export function selectedCopyRuns(selection: Selection, root: ParentNode): CopyRun[] {
  if (selection.isCollapsed || selection.rangeCount === 0) return [];
  const range = selection.getRangeAt(0);
  const startsInText = range.startContainer.nodeType === Node.TEXT_NODE;
  const endsInText = range.endContainer.nodeType === Node.TEXT_NODE;
  const runs: CopyRun[] = [];

  for (const span of root.querySelectorAll('.text-layer span')) {
    if (!selection.containsNode(span, true)) continue;
    const run = runForSpan.get(span);
    if (!run) continue;
    const from = startsInText && span.contains(range.startContainer) ? range.startOffset : 0;
    const to = endsInText && span.contains(range.endContainer) ? range.endOffset : run.text.length;
    const text = run.text.slice(from, to);
    if (text.length > 0) runs.push({ ...run, text });
  }
  return runs;
}

/** Where the selection lies, page by page: one rectangle per selected run in
 * the run's own space (display page points, y-up), narrowed in proportion
 * for the two partially selected runs at the ends. For readers of the
 * selection's geometry rather than its text (`selection.glyphs`). */
export function selectedRunRects(
  selection: Selection,
  root: ParentNode
): { page: number; rect: [number, number, number, number] }[] {
  if (selection.isCollapsed || selection.rangeCount === 0) return [];
  const range = selection.getRangeAt(0);
  const startsInText = range.startContainer.nodeType === Node.TEXT_NODE;
  const endsInText = range.endContainer.nodeType === Node.TEXT_NODE;
  const out: { page: number; rect: [number, number, number, number] }[] = [];
  for (const span of root.querySelectorAll('.text-layer span')) {
    if (!selection.containsNode(span, true)) continue;
    const run = runForSpan.get(span);
    if (!run || run.text.length === 0) continue;
    const from = startsInText && span.contains(range.startContainer) ? range.startOffset : 0;
    const to = endsInText && span.contains(range.endContainer) ? range.endOffset : run.text.length;
    if (to <= from) continue;
    const left = run.x + (run.w * from) / run.text.length;
    const right = run.x + (run.w * to) / run.text.length;
    out.push({ page: run.page, rect: [left, run.y, right - left, run.h] });
  }
  return out;
}

/** The text a copy of `selection` should actually put on the clipboard, or
 * null to leave the browser's own handling alone. */
export function smartCopyForSelection(selection: Selection, root: ParentNode): string | null {
  const runs = selectedCopyRuns(selection, root);
  if (runs.length === 0) return null;
  const text = smartCopyText(runs);
  return text.length > 0 ? text : null;
}

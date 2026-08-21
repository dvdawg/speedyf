/** Shaping the edit plan for a print job.
 *
 * Both transforms below are pure and produce a *new* plan — the document's own
 * plan is never altered, because the same document keeps being edited while a
 * print dialog is open. */
import type { EditPlan } from '../document/documentStore';

/** The document without any markup on it.
 *
 * Strips highlights, ink, rectangles and notes, and also the text boxes and
 * stamped images, since all of them are markup a reader added rather than the
 * document as its author wrote it. Printing a clean copy is a normal thing to
 * want, and printing one with someone's highlighter on it is not.
 *
 * Clearing the plan's own lists only removes markup added *this session*.
 * Annotations that were already in the file ride along inside the imported
 * page, so they have to be named for removal explicitly — otherwise the
 * highlights you made last week would print through a toggle that says they
 * will not. `ownedByPage` carries those indices, one entry per plan page. */
export function withoutAnnotations(plan: EditPlan, ownedByPage: number[][] = []): EditPlan {
  return {
    ...plan,
    pages: plan.pages.map((page, index) => ({
      ...page,
      annots: [],
      texts: [],
      images: [],
      dropSrcAnnots: ownedByPage[index] ?? page.dropSrcAnnots,
    })),
  };
}

/** Expand a CUPS page-range list into zero-based page indices, in order and
 * without repeats. Returns null when the range is unusable, so callers fall
 * back to the whole document rather than silently printing nothing. */
export function pagesInRange(range: string, pageCount: number): number[] | null {
  const wanted = new Set<number>();
  for (const part of range.split(',')) {
    const bounds = part.split('-');
    if (bounds.length > 2 || bounds.some((b) => !/^\d+$/.test(b))) return null;
    const from = Number(bounds[0]);
    const to = Number(bounds[bounds.length - 1]);
    if (from < 1 || to < from || to > pageCount) return null;
    for (let page = from; page <= to; page += 1) wanted.add(page - 1);
  }
  if (wanted.size === 0) return null;
  return [...wanted].sort((a, b) => a - b);
}

/** The plan cut down to a page range, for exporting a selection as a PDF.
 *
 * Printing does not need this — CUPS applies the range itself — but a saved
 * file has no printer to do the cutting, so the pages have to actually go. */
export function limitToRange(plan: EditPlan, range: string): EditPlan {
  const pages = pagesInRange(range, plan.pages.length);
  if (!pages) return plan;
  return { ...plan, pages: pages.map((index) => plan.pages[index]!) };
}

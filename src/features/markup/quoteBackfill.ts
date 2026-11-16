/** Quotes kept before their math could be read: re-read in the background
 * for the review on screen, one comment at a time, when the browser is
 * idle. Each is taken apart again from the page under its quads
 * (`piecesUnderQuads`) and read as a new comment's quote is - its plain
 * text rebuilt clean (margin numbers gone, lines joined, in reading order)
 * and its math typeset. A comment is tried once a session; one whose
 * reading is wrong can be read again from its card. */
import { untrack } from 'solid-js';
import { unwrap } from 'solid-js/store';
import { plausibleQuoteTex } from './quoteTexCheck';
import type { MarkupComment, Quad } from './types';

/** What a backfill needs of the store, handed in so this module stays
 * free of it (and testable without it). */
export interface BackfillHost {
  comments: (path: string) => readonly MarkupComment[] | null;
  reread: (path: string, comment: MarkupComment) => Promise<void>;
}

/** Comments tried this session, by document and id. */
const tried = new Set<string>();
/** Documents whose backfill is running. */
const running = new Set<string>();

/** Whether a kept comment's quote still wants reading: words, not a typo,
 * and no trustworthy reading of their math yet. */
export function wantsReading(comment: MarkupComment): boolean {
  const a = comment.anchor;
  if (comment.kind === 'typo' || !a.quads?.length || a.rect || !a.quote.trim()) return false;
  return !(a.quoteTex && plausibleQuoteTex(a.quoteTex, a.quote));
}

/** Resolve once the browser has a moment. */
function idle(): Promise<void> {
  return new Promise<void>((resolve) => {
    if (typeof window !== 'undefined' && typeof window.requestIdleCallback === 'function') {
      window.requestIdleCallback(() => resolve(), { timeout: 2_000 });
    } else {
      setTimeout(resolve, 50);
    }
  });
}

/** Re-read, one at a time and at idle, every comment of `path` that wants
 * it and has not been tried this session. Resolves when done. */
export async function backfillQuotes(path: string, host: BackfillHost): Promise<void> {
  if (running.has(path)) return;
  running.add(path);
  try {
    for (;;) {
      const next = (untrack(() => host.comments(path)) ?? []).find(
        (c) => !tried.has(`${path}\u0000${c.id}`) && wantsReading(c)
      );
      if (!next) return;
      tried.add(`${path}\u0000${next.id}`);
      await idle();
      await host.reread(path, unwrap(next));
    }
  } finally {
    running.delete(path);
  }
}

/** For tests: forget what was tried. */
export function resetBackfillForTest(): void {
  tried.clear();
  running.clear();
}

/** A quad's box, `[x, y, w, h]`, page points y up. */
export function quadBox(q: Quad): [number, number, number, number] {
  const left = Math.min(q.x1, q.x2, q.x3, q.x4);
  const right = Math.max(q.x1, q.x2, q.x3, q.x4);
  const bottom = Math.min(q.y1, q.y2, q.y3, q.y4);
  const top = Math.max(q.y1, q.y2, q.y3, q.y4);
  return [left, bottom, right - left, top - bottom];
}

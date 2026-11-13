/** The paper record behind the active tab, cached for the Library commands.
 *
 * A command's clipboard write has to happen inside the user gesture that ran
 * it, and WebKit's window for that is shorter than a round trip to the engine
 * (see "Clipboard timing" in EXTENSIONS.md). So the lookup is done ahead of
 * time: the Paper panel asks the engine for the record when a document is
 * shown and whenever the library changes, and leaves the answer here for
 * `Copy Cite Key` and `Copy BibTeX` to read synchronously. Nothing in this
 * module is reactive on purpose — it is a side table for commands, not a
 * store a view should render from.
 *
 * Keyed by path rather than by tab: the panel lives inside a tab's subtree and
 * only knows its own document, while a command only knows which tab is in
 * front. The path is what both agree on. */
import { engine } from '../../lib/transport/engine';
import { activeTab } from '../../stores/tabsStore';
import type { Paper } from '../../types/engine';

/** `null` means the engine said the file is outside the library; a path that
 * is absent has simply not been asked about yet. */
const papers = new Map<string, Paper | null>();

function activePath(): string | null {
  const doc = activeTab()?.documentStore.state;
  return doc?.loaded && doc.path ? doc.path : null;
}

/** The active document's paper, if the panel has already fetched it. Returns
 * null both when the file is outside the library and when nothing is known
 * yet; `activePaperKnown` tells those apart. */
export function activePaper(): Paper | null {
  const path = activePath();
  return path ? (papers.get(path) ?? null) : null;
}

/** Whether `activePaper()`'s null is an answer rather than an absence. */
export function activePaperKnown(): boolean {
  const path = activePath();
  return path !== null && papers.has(path);
}

/** Called by the Paper panel with each answer it gets. */
export function rememberPaper(path: string, paper: Paper | null): void {
  papers.set(path, paper);
}

/** Every cached record may be stale after `library:changed` — a membership
 * toggled, a lookup that rewrote the cite key — so the table is emptied and
 * the panel refills it. */
export function forgetPapers(): void {
  papers.clear();
}

/** The slow path, for a command that runs before the panel has ever been
 * shown: fetch the record now. The write that follows may land outside the
 * gesture window, and the caller decides what to do about that. */
export async function refreshActivePaper(): Promise<Paper | null> {
  const path = activePath();
  if (!path) return null;
  const paper = await engine.libraryPaperForPath(path);
  // Only worth keeping if the user is still on the same document.
  if (activePath() === path) papers.set(path, paper);
  return paper;
}

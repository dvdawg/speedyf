/** Placing a search hit inside the document's structure.
 *
 * The formal-environment index is a flat list in document order, each entry
 * carrying the page and the character index it starts at. A search match
 * carries indices into that same character stream, so attributing a hit to the
 * environment containing it is an ordered scan — no geometry, no rect
 * resolution, nothing that has to wait on the renderer. */
import type { FormalEntry } from '../../types/engine';

export interface MatchContext {
  /** enclosing section, as printed ("2 Results") */
  section: string | null;
  /** enclosing environment, as printed ("Theorem 3.1") */
  environment: string | null;
}

const NONE: MatchContext = { section: null, environment: null };

/** Entries must be in document order, which is how the engine emits them. */
export function contextForMatch(
  entries: readonly FormalEntry[],
  page: number,
  charIndex: number
): MatchContext {
  let section: string | null = null;
  let environment: string | null = null;

  for (const entry of entries) {
    // Ordered, so the first entry past the hit ends the search.
    if (entry.page > page || (entry.page === page && entry.charIndex > charIndex)) break;
    if (entry.depth === 0) {
      section = entry.label;
      // A new section closes whatever environment preceded it; a hit between
      // the heading and the first theorem is in the section, not in the last
      // theorem of the previous one.
      environment = null;
    } else {
      environment = entry.label;
    }
  }

  return section === null && environment === null ? NONE : { section, environment };
}

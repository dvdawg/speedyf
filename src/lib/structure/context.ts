/** Where a point in the document sits within its own structure.
 *
 * The engine emits the formal index as a flat list in document order, each
 * entry carrying its page, its y, and the character index it starts at.
 * Everything here is an ordered scan over that list — the only difference
 * between callers is what "before" means:
 *
 *   a search match knows a character index   -> `contextForMatch`
 *   a scroll position knows a y coordinate   -> `contextForPosition`
 *
 * Neither needs geometry resolved or rects fetched. */
import type { FormalEntry, OutlineNode } from '../../types/engine';

export interface StructureContext {
  /** enclosing section, as printed ("2 Results") */
  section: string | null;
  /** enclosing subsection ("2.1 Setup") */
  subsection: string | null;
  /** enclosing environment, as printed ("Theorem 3.1") */
  environment: string | null;
}

export const NO_CONTEXT: StructureContext = {
  section: null,
  subsection: null,
  environment: null,
};

/** The entries themselves, for callers that need to navigate to them. Looking
 * them up again by label would be ambiguous — two sections can share a title. */
export interface StructureAnchors {
  section: FormalEntry | null;
  subsection: FormalEntry | null;
  environment: FormalEntry | null;
}

/** Heading levels the breadcrumb shows. */
const LEVELS = 2;

/** Entries must be in document order, which is how the engine emits them. */
function scan(
  entries: readonly FormalEntry[],
  atOrBefore: (entry: FormalEntry) => boolean
): StructureAnchors {
  const headings: (FormalEntry | null)[] = new Array(LEVELS).fill(null);
  let environment: FormalEntry | null = null;
  for (const entry of entries) {
    if (!atOrBefore(entry)) break; // ordered, so the first miss ends it
    if (entry.heading) {
      const level = Math.min(entry.depth, LEVELS - 1);
      headings[level] = entry;
      // A heading closes everything nested under it: a point between a section
      // heading and its first theorem is in the section, not still inside the
      // last theorem of the section before.
      for (let deeper = level + 1; deeper < LEVELS; deeper += 1) headings[deeper] = null;
      environment = null;
    } else {
      environment = entry;
    }
  }
  return { section: headings[0] ?? null, subsection: headings[1] ?? null, environment };
}

function labels(anchors: StructureAnchors): StructureContext {
  return {
    section: anchors.section?.label ?? null,
    subsection: anchors.subsection?.label ?? null,
    environment: anchors.environment?.label ?? null,
  };
}

export function contextForMatch(
  entries: readonly FormalEntry[],
  page: number,
  charIndex: number
): StructureContext {
  return labels(
    scan(entries, (e) => e.page < page || (e.page === page && e.charIndex <= charIndex))
  );
}

/** `y` is page space (points, y-up), so an entry *earlier* in the document has
 * the *larger* y on the same page. */
export function anchorsForPosition(
  entries: readonly FormalEntry[],
  page: number,
  y: number
): StructureAnchors {
  return scan(entries, (e) => e.page < page || (e.page === page && e.y >= y));
}

export function contextForPosition(
  entries: readonly FormalEntry[],
  page: number,
  y: number
): StructureContext {
  return labels(anchorsForPosition(entries, page, y));
}

export function hasAnchors(anchors: StructureAnchors): boolean {
  return (
    anchors.section !== null || anchors.subsection !== null || anchors.environment !== null
  );
}

/** The bookmark tree flattened into the same shape, preserving its nesting as
 * section / subsection.
 *
 * This — not the merged structure index — is what the breadcrumb reads. The
 * index deliberately omits any heading with no environments under it, since a
 * list built to find theorems should not be padded with sections that hold
 * none. A breadcrumb needs the opposite: every heading, so that scrolling into
 * a section without theorems stops naming the previous one.
 *
 * Bookmarks without a destination page are skipped, and one without a y is
 * treated as sitting at the very top of its page. */
export function outlineAsEntries(nodes: readonly OutlineNode[]): FormalEntry[] {
  const out: FormalEntry[] = [];
  const walk = (list: readonly OutlineNode[], depth: number) => {
    for (const node of list) {
      if (node.page !== null) {
        out.push({
          heading: true,
          // Deeper bookmarks still group, they just stop indenting further.
          depth: Math.min(depth, LEVELS - 1),
          label: node.title.trim() || 'Untitled',
          page: node.page,
          y: node.y ?? Number.POSITIVE_INFINITY,
          charIndex: 0,
        });
      }
      walk(node.children, depth + 1);
    }
  };
  walk(nodes, 0);
  // Bookmarks are usually authored in document order, but nothing guarantees
  // it, and the scan depends on it. Equal y values (two bookmarks on the same
  // page with no coordinates) must compare equal rather than NaN.
  return out.sort((a, b) => a.page - b.page || (a.y === b.y ? 0 : b.y - a.y));
}

/** Where down the viewport the "you are here" point sits.
 *
 * Not the top edge. Headings can be close together, so probing the top edge
 * reports whatever you have scrolled *past*. This matches the fraction the
 * viewer already uses to decide the current page, so the breadcrumb and the
 * toolbar's page number never disagree. */
export const READING_FOCUS = 0.4;

/** Content-space scroll offset -> page-space y (points, y-up), which is the
 * space anchors are reported in. */
export function pageSpaceY(
  contentY: number,
  pageTopCss: number,
  zoom: number,
  pageHeightPt: number
): number {
  return pageHeightPt - (contentY - pageTopCss) / zoom;
}

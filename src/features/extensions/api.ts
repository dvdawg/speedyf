/** The read API extensions bind to.
 *
 * These shapes are the commitment. Everything else in SpeedyF can be
 * rearranged freely; what is returned from here cannot, because extensions
 * living in other repositories are compiled against it.
 *
 * So this is deliberately *not* the engine's DTOs forwarded. Each shape is a
 * narrower, plainer version, and the mapping between them is the seam that lets
 * the engine change without breaking anything installed. Three rules were
 * applied while choosing them:
 *
 * - **Omit what belongs to another permission.** No filesystem paths and no
 *   internal document ids, however convenient. `document:read` is not
 *   `filesystem:read` and must not become a way to spell it.
 * - **Omit geometry until something needs it.** Page text arrives as text, not
 *   as positioned runs. Coordinates are the hardest thing to keep stable across
 *   an engine change, and no v2 extension has a use for them.
 * - **Normalise oddities rather than pass them on.** The engine's citation
 *   scheme is `arXiv`; extensions see `arxiv`. An author should not have to
 *   learn our internal spelling.
 *
 * The mapping functions are pure and separately tested, because a silent change
 * in one of them is a silent break in every extension.
 */
import type {
  CitationId,
  Figure,
  LibrarySearchResult,
  OutlineNode,
  PageLinks,
  PageTextLayout,
  ResolvedCitation,
} from '../../types/engine';

// ---------- the shapes ----------

export interface ApiDocumentInfo {
  /** File name, without its directory. Never the full path. */
  name: string;
  pageCount: number;
  /** Zero-based, the page the viewport is currently on. */
  currentPage: number;
}

export interface ApiOutlineNode {
  title: string;
  /** Zero-based, or null when the entry has no destination. */
  page: number | null;
  children: ApiOutlineNode[];
}

export interface ApiFigure {
  /** As printed: "Figure 3", "Table 1". */
  label: string;
  caption: string;
  page: number;
}

export type ApiCitationKind = 'doi' | 'arxiv';

export interface ApiCitation {
  kind: ApiCitationKind;
  /** The identifier itself: a DOI, or an arXiv id. */
  value: string;
  page: number;
}

export interface ApiResolvedCitation {
  inLibrary: boolean;
  title: string | null;
  fileName: string | null;
  pageCount: number | null;
}

export interface ApiLibraryHit {
  name: string;
  title: string | null;
  matches: number;
}

// ---------- mapping ----------

export function toDocumentInfo(
  name: string,
  pageCount: number,
  currentPage: number
): ApiDocumentInfo {
  return { name, pageCount, currentPage };
}

/** Outline nodes keep their nesting. Flattening would be simpler to consume but
 * throws away structure an extension cannot rebuild, and a lossy mapping is the
 * one kind that cannot be widened later. */
export function toOutline(nodes: readonly OutlineNode[]): ApiOutlineNode[] {
  return nodes.map((node) => ({
    title: node.title,
    page: node.page,
    children: toOutline(node.children),
  }));
}

export function toFigures(figures: readonly Figure[]): ApiFigure[] {
  return figures.map((figure) => ({
    label: figure.label,
    caption: figure.title,
    page: figure.page,
  }));
}

/** Page text, joined from the layout runs.
 *
 * Runs arrive in reading order, so joining with spaces reconstructs the line
 * well enough for the things extensions do with it — matching a reference,
 * pulling a title. A run already ending in whitespace is not padded again.
 */
export function toPageText(layout: PageTextLayout | null): string {
  if (!layout) return '';
  let out = '';
  for (const run of layout.runs) {
    if (out.length > 0 && !/\s$/.test(out) && !/^\s/.test(run.text)) out += ' ';
    out += run.text;
  }
  return out;
}

export function toCitationKind(scheme: CitationId['scheme']): ApiCitationKind {
  return scheme === 'doi' ? 'doi' : 'arxiv';
}

/** Citations on one page, taken from its links.
 *
 * Links without a citation are dropped rather than reported as citations with
 * empty fields — an extension asking for citations wants citations. Duplicates
 * on the same page are collapsed: the same reference cited twice in a paragraph
 * is one citation, and reporting it twice makes every consumer dedupe.
 */
export function toCitations(links: PageLinks): ApiCitation[] {
  const seen = new Set<string>();
  const out: ApiCitation[] = [];
  for (const link of links.links) {
    if (link.target.kind !== 'uri' || !link.target.citation) continue;
    const kind = toCitationKind(link.target.citation.scheme);
    const value = link.target.citation.value;
    const key = `${kind}:${value}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ kind, value, page: links.src });
  }
  return out;
}

/** A resolution result. `null` — the engine's "not in your library" — becomes an
 * explicit `inLibrary: false` rather than a null the caller has to interpret. */
export function toResolvedCitation(resolved: ResolvedCitation | null): ApiResolvedCitation {
  if (!resolved) {
    return { inLibrary: false, title: null, fileName: null, pageCount: null };
  }
  return {
    inLibrary: true,
    title: resolved.title,
    fileName: resolved.fileName,
    pageCount: resolved.pageCount,
  };
}

/** Library hits, without paths or match geometry. */
export function toLibraryHits(result: LibrarySearchResult): ApiLibraryHit[] {
  return result.documents.map((hit) => ({
    name: hit.name,
    title: hit.title,
    matches: hit.totalMatches,
  }));
}

// ---------- the catalogue ----------

/** Which permission each API needs. The single place that answers "may this
 * extension call this", so a new API cannot be added without deciding. */
export const API_PERMISSIONS: Readonly<Record<string, string>> = {
  'document.info': 'document:read',
  'document.pageText': 'document:read',
  'document.outline': 'document:read',
  'document.figures': 'document:read',
  'citations.list': 'citations:read',
  'citations.resolve': 'citations:read',
  'library.search': 'library:read',
  'render.page': 'render:read',
  'document.contributeText': 'document:contributeText',
  'document.contributedText': 'document:contributeText',
  'clipboard.write': 'clipboard:write',
  'storage.get': 'storage',
  'storage.set': 'storage',
  'storage.remove': 'storage',
  // ui.render needs no permission: an extension may only draw into a panel it
  // declared, and drawing is not access to anything.
};

/** Whether `permissions` allow `api`. Unknown APIs are refused, so a typo in an
 * extension is an error rather than an accidental grant. */
export function canCallApi(permissions: readonly string[], api: string): boolean {
  const needed = API_PERMISSIONS[api];
  return needed !== undefined && permissions.includes(needed);
}

export function apiRefusalReason(permissions: readonly string[], api: string): string {
  const needed = API_PERMISSIONS[api];
  if (needed === undefined) return `"${api}" is not an API this version of SpeedyF offers`;
  return `"${api}" needs the "${needed}" permission${
    permissions.length > 0 ? ` (granted: ${permissions.join(', ')})` : ''
  }`;
}

/** Read a zero-based page argument, refusing anything that is not one.
 *
 * Extension arguments are untrusted: a float, a negative, or a page past the
 * end must be a clear error rather than an out-of-range read the engine has to
 * defend against.
 */
export function readPageArg(args: unknown, pageCount: number): number {
  const page = (args as { page?: unknown } | undefined)?.page;
  if (typeof page !== 'number' || !Number.isInteger(page)) {
    throw new Error('expected an integer "page"');
  }
  if (page < 0 || page >= pageCount) {
    throw new Error(`page ${page} is outside this document (0..${pageCount - 1})`);
  }
  return page;
}

/** Read a citation argument in the extension-facing shape, back into the
 * engine's. */
export function readCitationArg(args: unknown): CitationId {
  const kind = (args as { kind?: unknown } | undefined)?.kind;
  const value = (args as { value?: unknown } | undefined)?.value;
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error('expected a non-empty "value"');
  }
  if (kind === 'doi') return { scheme: 'doi', value };
  if (kind === 'arxiv') return { scheme: 'arXiv', value };
  throw new Error('expected "kind" to be "doi" or "arxiv"');
}

export function readQueryArg(args: unknown): string {
  const query = (args as { query?: unknown } | undefined)?.query;
  if (typeof query !== 'string' || query.trim().length === 0) {
    throw new Error('expected a non-empty "query"');
  }
  return query;
}

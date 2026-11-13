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
 * - **Omit geometry until something needs it.** Held until something did.
 *   Reading a bibliography needs it: the only thing tying a body-text citation
 *   to the entry it refers to is that the `[12]` jumps to a *place* on the
 *   references page, and the only thing tying that place to a title is where
 *   the words sit. Both are coordinates, and no amount of flat text recovers
 *   them. So geometry is exposed — but as one axis, in one space, in the two
 *   places that need it (`document.links`, `document.pageRuns`), rather than
 *   as the engine's rects forwarded wholesale. All four edges come with
 *   it: `x` and `w` because a two-column bibliography is unreadable without
 *   them, `h` because pointing at where something *is* on a page means drawing
 *   a shape around it. This is now the hardest part of the contract to keep
 *   stable across an engine change, and that cost was accepted knowingly.
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
  Paper,
  PaperList,
  PaperMeta,
  Project,
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
  /** The paper's cite key, when the library has a record for the file. What
   * a BibTeX extension is after; the rest of the record is `library.paper`. */
  citeKey: string | null;
}

/** An outbound link the document carries.
 *
 * Reported whether or not SpeedyF recognises an identifier in it, because a
 * reference list is full of works cited by URL alone — a JMLR paper, a book on
 * Google Books, a Semantic Scholar entry. `citation` is filled in for the two
 * schemes SpeedyF can resolve against your library and is null otherwise, which
 * is a statement about what can be matched, not about what counts as a
 * reference.
 */
export interface ApiLink {
  uri: string;
  /** Lowercase host, or null when the link has none (a `mailto:`). */
  host: string | null;
  page: number;
  citation: ApiCitation | null;
  /** Where the link sits on the page, in the space described by `ApiBox`. */
  box: ApiBox;
}

/** A rectangle on a page, in points, y-up from the bottom-left corner, at the
 * page's display size.
 *
 * One space for every coordinate an extension can see — link boxes, text runs,
 * and jump destinations all use it, so they can be compared directly. It is the
 * space SpeedyF renders in, which means a `y` here is a `y` on screen and an
 * extension never has to know which way round the file stored it.
 */
export interface ApiBox {
  x: number;
  /** The *top* edge. Higher is further up the page. */
  y: number;
  w: number;
  h: number;
}

/** An internal link: a place on the page that jumps to another place in the
 * same document.
 *
 * The thing a bibliography is read with. A `[12]` in body text is one of these,
 * and its destination is the entry it refers to — which is how a reference can
 * be tied to where it is genuinely cited rather than merely where it is listed.
 *
 * `toY` is null when the document's destination fixes only a page ("go to page
 * 14", with no position on it). Reported anyway, because knowing the page is
 * still worth something and silently dropping the jump would look like the
 * citation does not exist.
 */
export interface ApiJump {
  /** The zero-based page the jump starts on, and where on it. */
  page: number;
  box: ApiBox;
  /** The zero-based page it lands on. */
  toPage: number;
  /** Where on that page it lands, or null when the destination has no
   * position. A two-column page needs both: `toY` alone cannot say which of
   * the two entries printed at that height is the one meant. */
  toX: number | null;
  toY: number | null;
}

/** A run of text and where it sits.
 *
 * `document.pageText` flattens these into a string, which is the right shape
 * for searching and the wrong one for anything that needs to know what belongs
 * with what: two bibliography entries are one string with no boundary in it,
 * and two columns interleave into nonsense. Runs keep the boundary, because a
 * position is the only evidence of it in the file.
 */
export interface ApiTextRun {
  text: string;
  box: ApiBox;
}

export interface ApiLibraryHit {
  name: string;
  title: string | null;
  matches: number;
}

/** One paper as the Projects screen lists it: display strings, resolved. */
export interface ApiPaper {
  id: string;
  title: string;
  /** Already joined for display, the way the paper list shows it. */
  authors: string;
  year: number | null;
  venue: string | null;
  citeKey: string;
  projectIds: string[];
  /** Whether the scan (or the user) takes the file for a paper. A project
   * may hold a manual filed there on purpose; its key means nothing. */
  isPaper: boolean;
}

export interface ApiProject {
  id: string;
  name: string;
  paperCount: number;
}

/** A paper's file, without its path. The name tells an extension which copy
 * it is looking at — preprint or published — and `missing` whether the file
 * is still there; where it is remains the user's business. */
export interface ApiPaperFile {
  name: string;
  missing: boolean;
}

/** The full record, with provenance on every field, so an extension can tell
 * a title the user typed from one a heuristic guessed. */
export interface ApiPaperDetail {
  id: string;
  files: ApiPaperFile[];
  doi: string | null;
  arxiv: string | null;
  meta: PaperMeta;
  citeKey: string;
  citeKeyPinned: boolean;
  projectIds: string[];
  addedAt: string;
  updatedAt: string;
  incomplete: string[];
  isPaper: boolean;
}

/** One character as typeset, for reading mathematics back out of a page.
 *
 * Its own coordinate space on purpose, as the contract for `document:layout`
 * fixed it: page points from the *top-left* corner of the page as displayed,
 * y growing downwards, the way a typesetter thinks about a formula. `ApiBox`
 * stays y-up for everything older. */
export interface ApiGlyph {
  /** The character, from the PDF's ToUnicode map; '' when it has none. */
  text: string;
  /** The character code. The raw code in the font when PDFium could not map
   * it to Unicode (exactly the case a TeX font decoder needs); otherwise the
   * Unicode scalar, since PDFium does not expose raw codes for mapped text. */
  code: number;
  /** Base font name, subset prefix stripped: 'CMMI10', 'CMSY10'. '__rule'
   * for a thin horizontal rule (a fraction bar, a radical's overbar). */
  font: string;
  /** Font size in points, as printed (matrix-scaled). */
  size: number;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Baseline, page points from the top. */
  baseline: number;
  bold: boolean;
  italic: boolean;
}

export interface ApiGlyphs {
  page: number;
  glyphs: ApiGlyph[];
}

/** A tagged-PDF Formula element. */
export interface ApiMathTag {
  /** `[x, y, w, h]`, top-left origin, like a glyph. */
  bbox: [number, number, number, number];
  alt: string | null;
  actualText: string | null;
  mathml: string | null;
}

export interface ApiIdentifiers {
  doi: string | null;
  arxiv: string | null;
  arxivVersion: number | null;
}

export interface ApiFetchResult {
  status: number;
  contentType: string | null;
  /** A string for `as: 'text'`, bytes as plain numbers for `as: 'bytes'`. */
  body: string | number[];
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

/** Outbound links on one page, deduplicated by destination *and place*.
 *
 * The same URL printed twice in the same spot is one link — the engine finds a
 * bibliography URL twice, once as an annotation and once read out of the page
 * text, and reporting both would make every consumer dedupe. The same URL in
 * two different spots is two links, because now that a link carries a position
 * the second one is the only record that something is printed there. Collapsing
 * by URL alone silently moved the second occurrence to the first one's
 * coordinates, which put a reference in the wrong entry.
 *
 * Rounded to the point when comparing: the two detections of one link agree to
 * within a fraction of a point, and nothing legible is that close together. */
export function toLinks(links: PageLinks): ApiLink[] {
  const seen = new Set<string>();
  const out: ApiLink[] = [];
  for (const link of links.links) {
    if (link.target.kind !== 'uri') continue;
    const uri = link.target.uri;
    const box = toBox(link.rect);
    const key = `${Math.round(box.x)}:${Math.round(box.y)}:${uri}`;
    if (seen.has(key)) continue;
    seen.add(key);
    let host: string | null = null;
    try {
      host = new URL(uri).hostname.toLowerCase() || null;
    } catch {
      // A link the document carries need not be a URL SpeedyF can parse; it is
      // still reported, just without a host to group it by.
    }
    out.push({
      uri,
      host,
      page: links.src,
      citation: link.target.citation
        ? {
            kind: toCitationKind(link.target.citation.scheme),
            value: link.target.citation.value,
            page: links.src,
          }
        : null,
      box,
    });
  }
  return out;
}

/** The engine's `[x, y, w, h]` — y-up, bottom edge — as a box with a *top*
 * edge, which is what reading order wants: sorting entries down a page is then
 * descending `y`, and an entry's text is what lies below its heading. */
export function toBox(rect: readonly [number, number, number, number]): ApiBox {
  const [x, y, w, h] = rect;
  return { x, y: y + h, w, h };
}

/** Internal links on one page.
 *
 * Not merged into `document.links`: that API means "somewhere this document
 * points outside itself", every consumer written against it assumes a `uri`,
 * and quietly adding entries without one would break them. A jump is a
 * different kind of thing and gets its own call.
 *
 * Kept undeduplicated. Two `[12]`s in one paragraph are two citations of the
 * same work in the same place, and a caller counting citation sites wants both;
 * one that does not can collapse them, which it cannot do the other way round.
 */
export function toJumps(links: PageLinks): ApiJump[] {
  const out: ApiJump[] = [];
  for (const link of links.links) {
    if (link.target.kind !== 'internal') continue;
    out.push({
      page: links.src,
      box: toBox(link.rect),
      toPage: link.target.page,
      toX: link.target.x,
      toY: link.target.y,
    });
  }
  return out;
}

/** Page text as positioned runs, in reading order.
 *
 * Empty and whitespace-only runs are dropped: they carry no text and their
 * boxes are degenerate, so they only add noise to anything grouping by
 * position. */
export function toPageRuns(layout: PageTextLayout | null): ApiTextRun[] {
  if (!layout) return [];
  const out: ApiTextRun[] = [];
  for (const run of layout.runs) {
    if (run.text.trim().length === 0) continue;
    out.push({ text: run.text, box: { x: run.x, y: run.y + run.h, w: run.w, h: run.h } });
  }
  return out;
}

/** A resolution result. `null` — the engine's "not in your library" — becomes an
 * explicit `inLibrary: false` rather than a null the caller has to interpret.
 * The cite key is a second lookup the caller makes by path; it is null when
 * the file has no paper record, and always null for a file not in the library. */
export function toResolvedCitation(
  resolved: ResolvedCitation | null,
  citeKey: string | null = resolved?.citeKey ?? null
): ApiResolvedCitation {
  if (!resolved) {
    return { inLibrary: false, title: null, fileName: null, pageCount: null, citeKey: null };
  }
  return {
    inLibrary: true,
    title: resolved.title,
    fileName: resolved.fileName,
    pageCount: resolved.pageCount,
    citeKey,
  };
}

/** The paper list as rows. Already free of paths; the list is the engine's
 * display shape, and the mapping is spelled out field by field anyway so a
 * field added to the engine's row does not leak through on its own. */
export function toPapers(list: PaperList): { papers: ApiPaper[] } {
  return {
    papers: list.rows.map((row) => ({
      id: row.id,
      title: row.title,
      authors: row.authors,
      year: row.year,
      venue: row.venue,
      citeKey: row.citeKey,
      projectIds: [...row.projectIds],
      isPaper: row.isPaper,
    })),
  };
}

/** Projects without their synced `.bib` path, which is a filesystem location. */
export function toProjects(projects: readonly Project[]): ApiProject[] {
  return projects.map((project) => ({
    id: project.id,
    name: project.name,
    paperCount: project.paperCount,
  }));
}

/** The full record with its files reduced to name and presence. */
export function toPaper(paper: Paper): ApiPaperDetail {
  return {
    id: paper.id,
    files: paper.files.map((file) => ({ name: file.name, missing: file.missing })),
    doi: paper.doi,
    arxiv: paper.arxiv,
    meta: paper.meta,
    citeKey: paper.citeKey,
    citeKeyPinned: paper.citeKeyPinned,
    projectIds: [...paper.projectIds],
    addedAt: paper.addedAt,
    updatedAt: paper.updatedAt,
    incomplete: [...paper.incomplete],
    isPaper: paper.isPaper,
  };
}

/** Glyphs as the engine reports them, copied field by field so a field the
 * engine adds later does not leak into the contract on its own. */
export function toGlyphs(
  page: number,
  glyphs: readonly {
    text: string;
    code: number;
    font: string;
    size: number;
    x: number;
    y: number;
    w: number;
    h: number;
    baseline: number;
    bold: boolean;
    italic: boolean;
  }[]
): ApiGlyphs {
  return {
    page,
    glyphs: glyphs.map((g) => ({
      text: g.text,
      code: g.code,
      font: g.font,
      size: g.size,
      x: g.x,
      y: g.y,
      w: g.w,
      h: g.h,
      baseline: g.baseline,
      bold: g.bold,
      italic: g.italic,
    })),
  };
}

export function toMathTags(
  tags: readonly {
    bbox: readonly [number, number, number, number];
    alt?: string | null;
    actualText?: string | null;
    mathml?: string | null;
  }[]
): ApiMathTag[] {
  return tags.map((tag) => ({
    bbox: [tag.bbox[0], tag.bbox[1], tag.bbox[2], tag.bbox[3]],
    alt: tag.alt ?? null,
    actualText: tag.actualText ?? null,
    mathml: tag.mathml ?? null,
  }));
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
  'document.links': 'document:read',
  'document.jumps': 'document:read',
  'document.pageRuns': 'document:read',
  'citations.resolve': 'citations:read',
  'library.search': 'library:read',
  // The paper records behind the Projects screen. Read-only, and without the
  // one thing a record is organised around on disk: where its files are.
  'library.papers': 'library:read',
  'library.projects': 'library:read',
  'library.paper': 'library:read',
  // Navigation, so it sits under the same permission as running SpeedyF's own
  // Navigate commands. `commands.run` cannot express it: a command takes no
  // arguments, and "next page" is not what a panel row needs.
  'navigate.toPage': 'commands:run',
  'navigate.toPlace': 'commands:run',
  'render.page': 'render:read',
  'document.contributeText': 'document:contributeText',
  'document.contributedText': 'document:contributeText',
  // A reference provider's answer for a library document it was handed. The
  // only library write an extension has, and it can only answer what it was
  // asked: the document is a token SpeedyF issued, never a path.
  'references.contribute': 'library:contributeReferences',
  'clipboard.write': 'clipboard:write',
  'storage.get': 'storage',
  'storage.set': 'storage',
  'storage.remove': 'storage',
  // Glyph geometry and math tags: what a formula reader needs, still only of
  // the document on screen.
  'selection.glyphs': 'document:layout',
  'region.glyphs': 'document:layout',
  'document.mathTags': 'document:layout',
  'document.identifiers': 'document:read',
  // Outbound HTTPS, made by the core to the hosts the manifest lists.
  'net.fetch': 'network',
  // ui.render and ui.showMath need no permission: an extension may only draw
  // into a panel it declared, or show a formula in the popover SpeedyF owns,
  // and drawing is not access to anything.
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

/** One reference as a provider returns it. */
export interface ApiReferenceEntry {
  label?: string;
  text: string;
  year?: number;
}

/** No paper has more references; a provider claiming more is misreading. */
export const MAX_REFERENCE_ENTRIES = 1000;
/** An entry's text is cut here; a real entry is shorter. */
export const MAX_REFERENCE_TEXT = 1500;
const MAX_REFERENCE_LABEL = 32;

/** Read `references.contribute` arguments: the document token, and the
 * entries — each an object with a non-empty string `text`, cut at 1500
 * characters, and optionally a short string `label` and a plausible integer
 * `year`. A malformed entry, or a malformed field of one, is dropped on its
 * own rather than costing the provider the whole list; past 1000 entries the
 * rest are ignored. */
export function readReferenceEntries(args: unknown): {
  document: string;
  entries: ApiReferenceEntry[];
} {
  const { document, entries } = (args ?? {}) as { document?: unknown; entries?: unknown };
  if (typeof document !== 'string' || document.length === 0) {
    throw new Error('references.contribute needs the "document" it was handed');
  }
  if (!Array.isArray(entries)) throw new Error('references.contribute needs an "entries" array');
  const out: ApiReferenceEntry[] = [];
  for (const raw of entries) {
    if (out.length >= MAX_REFERENCE_ENTRIES) break;
    if (typeof raw !== 'object' || raw === null) continue;
    const { label, text, year } = raw as { label?: unknown; text?: unknown; year?: unknown };
    if (typeof text !== 'string') continue;
    const trimmed = text.trim().slice(0, MAX_REFERENCE_TEXT);
    if (trimmed.length === 0) continue;
    const entry: ApiReferenceEntry = { text: trimmed };
    if (
      typeof label === 'string' &&
      label.trim().length > 0 &&
      label.length <= MAX_REFERENCE_LABEL
    ) {
      entry.label = label.trim();
    }
    if (typeof year === 'number' && Number.isInteger(year) && year >= 1400 && year <= 2100) {
      entry.year = year;
    }
    out.push(entry);
  }
  return { document, entries: out };
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

/** Read a position on a page, in the space `ApiBox` describes.
 *
 * A page is a coarse place to be sent. "Cited on page 4" is only useful if it
 * lands on the sentence doing the citing, and a reader who has to find it
 * again has been given a page number, not a destination. */
export function readPlaceArg(
  args: unknown,
  pageCount: number
): { page: number; boxes: ApiBox[]; y: number } {
  const page = readPageArg(args, pageCount);
  const record = (args as Record<string, unknown> | undefined) ?? {};
  const y = record.y;
  if (typeof y !== 'number' || !Number.isFinite(y)) {
    throw new Error('expected a finite "y"');
  }
  // `boxes` is optional and plural. A caller that knows only a height gets
  // taken to it; one that knows the shape of what it is pointing at can have
  // it marked — and that shape is often several boxes, because one citation is
  // often several links.
  const boxes: ApiBox[] = [];
  if (record.boxes !== undefined) {
    if (!Array.isArray(record.boxes)) throw new Error('expected "boxes" to be an array');
    for (const value of record.boxes) {
      const box = value as Record<string, unknown> | null;
      const sides = ['x', 'y', 'w', 'h'].map((key) => box?.[key]);
      if (!sides.every((side) => typeof side === 'number' && Number.isFinite(side))) {
        throw new Error('expected each box to have finite x, y, w and h');
      }
      const [x, top, w, h] = sides as [number, number, number, number];
      boxes.push({ x, y: top, w, h });
    }
  }
  return { page, boxes, y };
}

/** Read a record id. Only its shape is checked here; whether a paper with it
 * exists is the engine's answer, and an unknown id is its error. */
export function readIdArg(args: unknown): string {
  const id = (args as { id?: unknown } | undefined)?.id;
  if (typeof id !== 'string' || id.trim().length === 0) {
    throw new Error('expected a non-empty "id"');
  }
  return id;
}

export function readQueryArg(args: unknown): string {
  const query = (args as { query?: unknown } | undefined)?.query;
  if (typeof query !== 'string' || query.trim().length === 0) {
    throw new Error('expected a non-empty "query"');
  }
  return query;
}

/** Read a `{ page, bbox }` region: a page index and `[x, y, w, h]` in page
 * points, top-left origin, with a positive size. */
export function readRegionArg(
  args: unknown,
  pageCount: number
): { page: number; bbox: [number, number, number, number] } {
  const page = readPageArg(args, pageCount);
  const bbox = (args as { bbox?: unknown } | undefined)?.bbox;
  if (
    !Array.isArray(bbox) ||
    bbox.length !== 4 ||
    !bbox.every((n) => typeof n === 'number' && Number.isFinite(n))
  ) {
    throw new Error('expected "bbox" to be [x, y, w, h] in page points');
  }
  const [x, y, w, h] = bbox as [number, number, number, number];
  if (w <= 0 || h <= 0) throw new Error('expected "bbox" to have a positive width and height');
  return { page, bbox: [x, y, w, h] };
}

/** The most `net.fetch` URL text accepted; anything longer is not a URL a
 * service expects. */
const MAX_URL = 4096;

/** Read `net.fetch` arguments. The host and scheme are the core's to judge;
 * this only checks the shape. */
export function readFetchArgs(args: unknown): {
  url: string;
  kind: 'text' | 'bytes';
  maxBytes: number | null;
} {
  const record = (args ?? {}) as { url?: unknown; as?: unknown; maxBytes?: unknown };
  if (typeof record.url !== 'string' || record.url.length === 0 || record.url.length > MAX_URL) {
    throw new Error('net.fetch needs a "url" string');
  }
  const kind = record.as === undefined ? 'text' : record.as;
  if (kind !== 'text' && kind !== 'bytes') {
    throw new Error('net.fetch: "as" must be "text" or "bytes"');
  }
  let maxBytes: number | null = null;
  if (record.maxBytes !== undefined) {
    if (
      typeof record.maxBytes !== 'number' ||
      !Number.isInteger(record.maxBytes) ||
      record.maxBytes <= 0
    ) {
      throw new Error('net.fetch: "maxBytes" must be a positive integer');
    }
    maxBytes = record.maxBytes;
  }
  return { url: record.url, kind, maxBytes };
}

export type MathSource = 'tags' | 'arxiv' | 'jats' | 'fonts';
export type MathConfidence = 'high' | 'medium' | 'low';

export interface ShowMathArgs {
  latex: string;
  display: boolean;
  source: MathSource;
  confidence: MathConfidence;
}

/** Longest LaTeX `ui.showMath` takes; an equation page is far shorter. */
export const MAX_SHOW_MATH = 20_000;

export function readShowMathArgs(args: unknown): ShowMathArgs {
  const record = (args ?? {}) as Record<string, unknown>;
  const { latex, display, source, confidence } = record;
  if (typeof latex !== 'string' || latex.trim().length === 0) {
    throw new Error('ui.showMath needs a non-empty "latex" string');
  }
  if (latex.length > MAX_SHOW_MATH) {
    throw new Error(`ui.showMath is limited to ${MAX_SHOW_MATH} characters of LaTeX`);
  }
  if (typeof display !== 'boolean') throw new Error('ui.showMath needs a boolean "display"');
  if (source !== 'tags' && source !== 'arxiv' && source !== 'jats' && source !== 'fonts') {
    throw new Error('ui.showMath: "source" must be "tags", "arxiv", "jats" or "fonts"');
  }
  if (confidence !== 'high' && confidence !== 'medium' && confidence !== 'low') {
    throw new Error('ui.showMath: "confidence" must be "high", "medium" or "low"');
  }
  return { latex, display, source, confidence };
}

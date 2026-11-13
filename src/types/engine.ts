/** DTOs mirrored from the Rust engine (serde camelCase). */
import type { PdfPoint, PdfQuad, PdfRect, TextFormat } from './model';

export interface DocMeta {
  docId: number;
  path: string;
  name: string;
  pageCount: number;
  /** [w, h, cropX, cropY, baseRotation] — display sizes; base rotation is
   * folded into them engine-side, so cropX/cropY/baseRotation arrive as 0. */
  sizes: [number, number, number, number, number][];
  estimatedSize: [number, number];
}

export interface TextRunDto {
  text: string;
  start: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** One document that matched a library-wide search. Carries the file, not a
 * docId — nothing is open when a library search runs. */
export interface LibraryHit {
  path: string;
  name: string;
  title: string | null;
  totalMatches: number;
  pages: PageMatches[];
}

export interface LibrarySearchResult {
  documents: LibraryHit[];
  truncated: boolean;
}

export interface Printer {
  name: string;
  isDefault: boolean;
}

/** One thing a printer can vary, as the printer itself reports it. */
export interface PrintOption {
  /** what CUPS calls it: "Duplex" */
  key: string;
  /** what a person calls it: "2-Sided Printing" */
  label: string;
  choices: string[];
  default: string;
}

export interface PrintJob {
  printer: string;
  copies: number;
  /** null prints everything */
  range: string | null;
  /** chosen values as [key, value] */
  options: [string, string][];
}

/** A figure, table or algorithm, with a crop of the artwork above its caption. */
export interface Figure {
  label: string;
  /** caption text after the label, with LaTeX scripts restored */
  title: string;
  page: number;
  y: number;
  tile: { x: number; y: number; w: number; h: number };
  scaleMilli: number;
}

export interface PageTextLayout {
  src: number;
  runs: TextRunDto[];
  charCount: number;
}

export type CitationId = { scheme: 'doi'; value: string } | { scheme: 'arXiv'; value: string };

export type LinkTarget =
  | { kind: 'internal'; page: number; x: number | null; y: number | null }
  | { kind: 'uri'; uri: string; citation: CitationId | null }
  | { kind: 'unknown' };

export interface PageLink {
  rect: [number, number, number, number];
  target: LinkTarget;
}

export interface PageLinks {
  src: number;
  links: PageLink[];
}

/** A fillable-looking glyph a page *draws* — a LaTeX `\bigcirc`, a printed
 * checkbox — as opposed to an AcroForm widget, which `FormFieldInfo` covers.
 * See src-tauri/src/engine/bubbles.rs. */
export interface Bubble {
  /** Index of the glyph in the page's character stream; stable per page, so
   * a shading can be keyed to the bubble it fills. */
  charIndex: number;
  /** Normalized PDF page space, y-up — the same space text runs use. */
  rect: { x: number; y: number; w: number; h: number };
  fontSizePt: number;
}

export interface PageBubbles {
  src: number;
  bubbles: Bubble[];
}

export interface PreviewSpec {
  docId: number;
  src: number;
  tile: { x: number; y: number; w: number; h: number };
  scaleMilli: number;
  text: string;
}

export interface ResolvedCitation {
  docId: number;
  title: string | null;
  pageCount: number;
  fileName: string;
  path: string;
  /** The cite key of the paper record holding the file, when there is one. */
  citeKey?: string | null;
  preview: PreviewSpec;
}

export interface LibraryStatus {
  /** every folder in the library; was a single optional root */
  roots: string[];
  indexed: number;
  total: number;
  scanning: boolean;
  /** documents whose full text is searchable, which lags metadata indexing */
  textIndexed: number;
  textTotal: number;
}

export interface MatchDto {
  start: number;
  len: number;
  snippet: string;
}

export interface PageMatches {
  src: number;
  matches: MatchDto[];
}

export interface SearchQueryResult {
  pages: PageMatches[];
  truncated: boolean;
  limit: number;
}

export interface FormFieldInfo {
  name: string;
  kind: string;
  value: string;
  page: number;
  readOnly: boolean;
  /** The widget's position in its page's annotation list. With `page`, the
   * only stable identity one *control* has — `name` is shared by every button
   * in a radio group, so it cannot say which one was clicked. */
  index: number;
  /** Normalized PDF page space, y-up. */
  rect: { x: number; y: number; w: number; h: number };
  /** The "on" state a checkbox or radio turns its field to; empty otherwise. */
  exportValue: string;
  /** Whether this particular widget is currently on. */
  checked: boolean;
}

/** A theorem, lemma, definition or caption recovered from a LaTeX PDF.
 * `kind` is the word as printed, which is not the LaTeX counter it was
 * anchored on — see src-tauri/src/engine/formal.rs. */
/** One row of the formal-environment list: a section heading (depth 0) or an
 * environment under it (depth 1), already in document order. */
/** One annotation read out of the file, ready to become an editable
 * `Annotation`. `index` is its position in the source page's annotation list —
 * the identity the save path uses to replace it. */
export interface StoredAnnotation extends TextFormat {
  index: number;
  kind: string;
  rect: PdfRect;
  color: string;
  opacity: number;
  strokeWidth: number | null;
  quads: PdfQuad[] | null;
  strokes: PdfPoint[][] | null;
  text: string | null;
  fontSizePt: number | null;
}

export interface PageAnnotations {
  src: number;
  annots: StoredAnnotation[];
}

export interface FormalEntry {
  /** true for a section/subsection heading, false for an environment. Depth
   * alone cannot tell them apart — an environment indents to however many
   * headings stand above it, so it can sit at any depth. */
  heading: boolean;
  depth: number;
  label: string;
  page: number;
  y: number;
  /** index of the anchor's first character in the page's character stream */
  charIndex: number;
}

export interface OutlineNode {
  title: string;
  page: number | null;
  /** destination y in page space, when the destination specifies one */
  y: number | null;
  children: OutlineNode[];
}

export interface SaveResult {
  path: string;
  bytes: number;
  durationMs: number;
}

/* ---------- projects and paper records (mirrors src-tauri/src/library/types.rs) ---------- */

/** Where a metadata value came from. A value never loses to a weaker source:
 * user > crossref > arxiv > openalex > pdf-info > first-page > filename. */
export type MetaSource =
  | 'user'
  | 'crossref'
  | 'inspire'
  | 'datacite'
  | 'arxiv'
  | 'semantic-scholar'
  | 'dblp'
  | 'openalex'
  | 'open-library'
  | 'xmp'
  | 'identifier'
  | 'pdf-info'
  | 'first-page'
  | 'filename';

export interface MetaField<T> {
  value: T;
  source: MetaSource;
}

export interface Author {
  family: string;
  given: string;
}

export type PaperKind = 'article' | 'inproceedings' | 'book' | 'thesis' | 'misc';

export interface PaperMeta {
  title: MetaField<string> | null;
  authors: MetaField<Author[]> | null;
  year: MetaField<number> | null;
  venue: MetaField<string> | null;
  volume: MetaField<string> | null;
  issue: MetaField<string> | null;
  pages: MetaField<string> | null;
  publisher: MetaField<string> | null;
  abstract: MetaField<string> | null;
  kind: MetaField<PaperKind> | null;
}

export interface PaperFile {
  path: string;
  name: string;
  size: number;
  mtimeMs: number;
  fingerprint: string | null;
  missing: boolean;
  /** When the file was first found missing (RFC 3339). */
  missingSince?: string | null;
  /** Set when this file is a byte-identical copy of another file of the
   * item: the path of the copy that stands for both in the list. */
  duplicateOf?: string | null;
}

/** The full record, for the detail column and the Paper panel. */
export interface Paper {
  id: string;
  files: PaperFile[];
  doi: string | null;
  arxiv: string | null;
  /** for books: the ISBN found on the first pages, digits only */
  isbn: string | null;
  meta: PaperMeta;
  citeKey: string;
  citeKeyPinned: boolean;
  projectIds: string[];
  addedAt: string;
  updatedAt: string;
  /** What a BibTeX entry needs and this paper lacks: 'authors', 'year',
   * 'title', 'venue'. Empty when the entry would be complete. */
  incomplete: string[];
  /** the user's override when there is one, else the scan's verdict */
  isPaper: boolean;
  /** why, in a few words: the scan's reason, or "marked by you" */
  paperReason: string;
  /** null means the scan decides */
  paperOverride: boolean | null;
  /** title-search matches parked for the user to pick from */
  candidates: LookupCandidate[];
  /** how many records were merged into this one; non-zero means it can be split */
  mergedCount: number;
  /** why records were merged into this one automatically ("same arXiv id
   * 2606.13614"); empty when none were */
  autoMerged: string[];
}

/** One row of the paper list: display strings, already resolved. */
export interface PaperRow {
  id: string;
  title: string;
  authors: string;
  year: number | null;
  venue: string | null;
  citeKey: string;
  /** BibTeX alpha-style label from every author and the year: "Jar97",
   * "ZD24", "ABC+19". Not unique; the graph suffixes repeats. */
  alphaLabel: string;
  projectIds: string[];
  /** every file is missing */
  missing: boolean;
  hasIdentifier: boolean;
  incompleteCount: number;
  isPaper: boolean;
  /** parked title-search matches waiting for the user */
  candidateCount: number;
  /** files of the item that are on disk */
  fileCount: number;
}

/** What Export bundle wrote (`library_export_bundle`). */
export interface BundleReport {
  path: string;
  bibName: string;
  /** Names inside the zip (`pdfs/<key>.pdf`), in project order. */
  pdfs: string[];
  /** Cite keys of the items whose main file is not on disk. */
  missing: string[];
}

export interface Project {
  id: string;
  name: string;
  order: number;
  paperCount: number;
  bibPath: string | null;
  incompleteCount: number;
  createdAt: string;
}

export interface PaperFilter {
  /** only papers in this project */
  project?: string | null;
  /** only papers in no project; ignored when `project` is set */
  unfiled?: boolean;
  /** only records that are not papers; ignored when `project` is set.
   * Without it, and without `project`, non-papers are left out. */
  others?: boolean;
  /** matched against title, authors, year and cite key */
  query?: string;
}

export interface PaperList {
  rows: PaperRow[];
  total: number;
}

/** A user edit. A field left out is untouched; `null` clears it. Every field
 * set here is recorded as `user` and never overwritten afterwards. */
export interface MetaPatch {
  title?: string | null;
  authors?: Author[] | null;
  year?: number | null;
  venue?: string | null;
  volume?: string | null;
  issue?: string | null;
  pages?: string | null;
  publisher?: string | null;
  abstract?: string | null;
  kind?: PaperKind | null;
  doi?: string | null;
  arxiv?: string | null;
  isbn?: string | null;
  /** setting a key pins it; `null` unpins and regenerates */
  citeKey?: string | null;
  /** true/false overrides the scan's verdict; null hands it back */
  isPaper?: boolean | null;
}

export interface LookupCandidate {
  source: MetaSource;
  title: string;
  authors: Author[];
  year: number | null;
  venue: string | null;
  volume: string | null;
  issue: string | null;
  pages: string | null;
  publisher: string | null;
  abstract: string | null;
  kind: PaperKind | null;
  doi: string | null;
  arxiv: string | null;
  isbn: string | null;
}

export type LookupResult =
  | { kind: 'applied'; paper: Paper }
  | { kind: 'candidates'; candidates: LookupCandidate[] }
  | { kind: 'nothing' };

export type LookupMode = 'id' | 'title';

export interface CitationLinks {
  cites: PaperRow[];
  citedBy: PaperRow[];
  /** identifiers this paper cites that match no paper in the library */
  unresolved: number;
}

export interface LibraryDataInfo {
  dir: string;
  isDefault: boolean;
}

/** One backup of library.json, newest first in a listing. */
export interface LibraryBackup {
  /** The file name in the backups folder; what a restore takes. */
  file: string;
  /** When it was written, RFC 3339 UTC. */
  at: string;
  /** Size on disk, compressed. */
  bytes: number;
  papers: number;
  projects: number;
}

/** How BibTeX is written: the clipboard, exports and synced .bib files. */
export interface BibtexOptions {
  includeAbstract: boolean;
  /** doi.org link, else the arXiv abstract page */
  includeUrl: boolean;
  /** the main file's path */
  includeFile: boolean;
  /** double braces around titles */
  protectTitle: boolean;
  abbreviateJournals: boolean;
}

/** Counts for the two fixed rows of the projects column. */
export interface LibraryCounts {
  all: number;
  unfiled: number;
  /** records the scan does not take for papers (and the user has not) */
  others: number;
}

/** What a whole-project lookup did. Papers without an identifier are
 * skipped, never searched by title. */
export interface LookupBatch {
  attempted: number;
  applied: number;
  skipped: number;
  failed: number;
  /** title searches that found something but nothing sure enough to apply */
  parked: number;
}

/** Whether an open PDF looks like a paper. The reader only offers to file
 * what does; a manual or a scan gets a quiet note instead. */
export type PapernessVerdict = 'paper' | 'likely' | 'unknown';

export interface Paperness {
  verdict: PapernessVerdict;
  /** why, in a few words; empty for 'unknown' */
  reason: string;
  /** what kind of item it looks like, when the signals say */
  kind: PaperKind | null;
  doi: string | null;
  arxiv: string | null;
  isbn: string | null;
}

/** One citation between two items: `from` cites `to`. */
export interface GraphEdge {
  from: string;
  to: string;
}

/** The citation graph of a project, or of the whole library. */
export interface Graph {
  nodes: PaperRow[];
  /** edges between nodes only; citations leaving the set are counted in `external` */
  edges: GraphEdge[];
  external: number;
  /** nodes whose text is not indexed yet, so their citations are unknown */
  unindexed: number;
}

/** Records the engine suspects are one work. */
export interface DuplicateGroup {
  paperIds: string[];
  /** why, in a few words */
  reason: string;
}

/** A work cited by items in the view but not in the library. */
export interface MissingWork {
  /** stable within one answer; the ghost node's id */
  key: string;
  /** the reference as printed */
  text: string | null;
  year: number | null;
  doi: string | null;
  arxiv: string | null;
  /** the items in the view that cite it, in list order */
  citedBy: string[];
}

/** A project the item might belong in, read off the citation graph. */
export interface ProjectSuggestion {
  projectId: string;
  name: string;
  /** higher is stronger; normalized by the project's size */
  score: number;
  /** members the item cites */
  cites: number;
  /** members citing the item */
  citedBy: number;
  /** works the item and at least one member both cite */
  sharedRefs: number;
  /** in plain words: "cites 2 papers in it, shares 4 references with it" */
  reason: string;
}

/** Another item the citation graph ties to one: works both cite, items
 * citing both. */
export interface RelatedItem {
  row: PaperRow;
  /** higher is more related; normalized so size alone does not win */
  score: number;
  /** works both cite */
  sharedRefs: number;
  /** items citing both */
  coCited: number;
  /** in plain words: "shares 6 references, cited together by 3 items" */
  reason: string;
}

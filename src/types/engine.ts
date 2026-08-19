/** DTOs mirrored from the Rust engine (serde camelCase). */

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
  preview: PreviewSpec;
}

export interface LibraryStatus {
  root: string | null;
  indexed: number;
  total: number;
  scanning: boolean;
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
}

/** A theorem, lemma, definition or caption recovered from a LaTeX PDF.
 * `kind` is the word as printed, which is not the LaTeX counter it was
 * anchored on — see src-tauri/src/engine/formal.rs. */
/** One row of the formal-environment list: a section heading (depth 0) or an
 * environment under it (depth 1), already in document order. */
export interface FormalEntry {
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

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

export interface SaveResult {
  path: string;
  bytes: number;
  durationMs: number;
}

/** Engine shapes for redaction and hidden-information removal
 * (src-tauri/src/engine/redact, engine/sanitize.rs). */

export type RedactPattern = 'email' | 'phone' | 'ssn' | 'card' | 'date';

export interface FindQuery {
  phrase: string;
  caseSensitive: boolean;
  wholeWords: boolean;
  patterns: RedactPattern[];
}

export interface RedactMatch {
  /** source page index */
  page: number;
  start: number;
  len: number;
  text: string;
  /** 'phrase' or the pattern kind */
  kind: string;
  /** display-space [x, y, w, h], y up, one per line */
  rects: [number, number, number, number][];
}

export type HiddenKey =
  | 'metadata'
  | 'attachments'
  | 'javascript'
  | 'hiddenLayers'
  | 'hiddenText'
  | 'comments'
  | 'forms'
  | 'links'
  | 'bookmarks'
  | 'thumbnails'
  | 'privateData'
  | 'revisions';

export interface HiddenItem {
  key: HiddenKey;
  count: number;
  detail: string;
}

export interface HiddenInfo {
  items: HiddenItem[];
  /** the document carries a digital signature */
  signed: boolean;
}

/** The Markup comment model: what is stored per PDF (`markup_save`) and
 * what everything else here reads. Owned by the frontend; the backend keeps
 * it as an opaque JSON string.
 *
 * Coordinates are the display-normalized page space the annotation tools
 * use (y-up, origin at the displayed crop box's bottom-left, points), on
 * the page of the FILE (`page` is the source page index), so a comment
 * stays put when pages are reordered on screen. */

export const MARKUP_VERSION = 1;

/** The built-in extension whose switch turns all of this on. */
export const MARKUP_EXTENSION_ID = 'dev.speedyf.markup';

export type CommentKind = 'proof' | 'exposition' | 'question' | 'literature' | 'typo';

export const COMMENT_KINDS: readonly CommentKind[] = [
  'proof',
  'exposition',
  'question',
  'literature',
  'typo',
];

/** What each kind reads as on a chip, in a filter and in the letter. */
export const KIND_LABEL: Record<CommentKind, string> = {
  proof: 'Proof',
  exposition: 'Exposition',
  question: 'Question',
  literature: 'Literature',
  typo: 'Typo',
};

/** A text quad. Points 1 and 2 are the top edge (left, right), 3 and 4 the
 * bottom edge (left, right): the order PDF QuadPoints use in practice. */
export interface Quad {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  x3: number;
  y3: number;
  x4: number;
  y4: number;
}

/** `x`, `y` are the bottom-left corner (y-up). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Anchor {
  /** Zero-based page of the file. */
  page: number;
  /** One quad per line of selected text; null for a region. */
  quads: Quad[] | null;
  /** The dragged box of a region comment; null for text. */
  rect: Rect | null;
  /** The text under it (empty for a region). */
  quote: string;
  /** The same words as Markdown with their math as LaTeX (`$...$`, displays
   * as `$$ ... $$`), when a math converter could read them: what the
   * drawer shows and the letter quotes. Absent in reviews written before
   * it, and when there was no math or no converter. */
  quoteTex?: string;
}

export interface MarkupComment {
  id: string;
  kind: CommentKind;
  anchor: Anchor;
  /** Markdown with `$math$` and `[@citekey]` citations. */
  body: string;
  /** A typo fix: strike the quote, insert this. */
  replacement: string | null;
  quickmark: string | null;
  /** Unix milliseconds. */
  created: number;
}

export interface MarkupDoc {
  version: number;
  /** The "Overall" paragraph, Markdown. */
  summary: string;
  comments: MarkupComment[];
}

export function emptyMarkup(): MarkupDoc {
  return { version: MARKUP_VERSION, summary: '', comments: [] };
}

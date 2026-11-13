/** Hover spans the engine finds in page text (`engine::hovers`): cross-
 * references ("Eq. (12)", "Fig. 3", "Theorem 2.1"), footnote marks, and
 * terms an extension contributed. Rectangles are `[x, y, w, h]` in y-up
 * page points, the space every page overlay draws in. */

export type HoverSpanKind =
  'equation' | 'figure' | 'table' | 'algorithm' | 'section' | 'theorem' | 'footnote' | 'term';

export type HoverRect = [number, number, number, number];

/** One definition a term occurrence points at. */
export interface HoverTermEntry {
  title: string;
  /** Plain text, or with `$...$` TeX. */
  body: string;
  /** The page in the file where it is defined. */
  page: number;
  rect: HoverRect | null;
}

export interface HoverTarget {
  /** The page in the file. */
  page: number;
  x: number | null;
  /** The top of the target, where a click lands. */
  y: number;
  /** What the preview shows, rendered from the page; null for a term. */
  region: HoverRect | null;
  /** A footnote that runs on onto the next page. */
  continues: boolean;
  terms: HoverTermEntry[];
}

export interface HoverSpan {
  kind: HoverSpanKind;
  rects: HoverRect[];
  /** For screen readers: "Equation 12", "Footnote 3", "FDT". */
  label: string;
  target: HoverTarget;
}

export interface PageHovers {
  src: number;
  spans: HoverSpan[];
}

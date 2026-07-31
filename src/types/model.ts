/** Shared document-model types. UI state stores ONLY this lightweight metadata —
 * never PDF bytes, bitmaps, or rendered images. */

export type Rotation = 0 | 90 | 180 | 270;

export type PageId = string;

/** A point/size in PDF points (1/72 in), in normalized page space:
 * origin at the bottom-left of the page crop box, y-up, unrotated. */
export interface PdfPoint {
  x: number;
  y: number;
}

export interface PdfRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Quad in PDF space (for text-markup annotations). */
export interface PdfQuad {
  p1: PdfPoint;
  p2: PdfPoint;
  p3: PdfPoint;
  p4: PdfPoint;
}

export interface PageEntry {
  id: PageId;
  /** Index into the ORIGINAL source document; null for pages added in-session. */
  srcIndex: number | null;
  /** Rotation baked into the PDF page dictionary (/Rotate). */
  baseRotation: Rotation;
  /** Additional rotation applied by the user in this session. */
  userRotation: Rotation;
  /** Crop-box size in points (unrotated). */
  widthPt: number;
  heightPt: number;
  /** Crop-box origin offset within raw PDF user space. */
  cropX: number;
  cropY: number;
  /** False while the size is an estimate awaiting lazy hydration. */
  sizeKnown: boolean;
}

export type AnnotationKind = 'highlight' | 'ink' | 'rect' | 'textbox' | 'note' | 'image';

export interface Annotation {
  id: string;
  pageId: PageId;
  kind: AnnotationKind;
  /** Bounding rect in normalized PDF page space. */
  rect: PdfRect;
  /** CSS hex color, e.g. "#ffd54a". */
  color: string;
  /** 0..1 */
  opacity: number;
  /** Points, for ink/rect strokes. */
  strokeWidth?: number;
  /** Highlight quads (PDF space). */
  quads?: PdfQuad[];
  /** Ink strokes: list of polylines in PDF space. */
  strokes?: PdfPoint[][];
  /** textbox / note contents. */
  text?: string;
  /** textbox font size in points. */
  fontSizePt?: number;
  /** image source path (absolute, chosen via native dialog). */
  sourcePath?: string;
  /** natural pixel size of the source image. */
  naturalW?: number;
  naturalH?: number;
}

export interface DocMeta {
  docId: number;
  path: string | null;
  name: string;
  pageCount: number;
  /** Sizes for the first N pages: [w, h, cropX, cropY, baseRotation][] */
  sizes: [number, number, number, number, number][];
  /** Estimated [w,h] applied to pages whose size is not yet known. */
  estimatedSize: [number, number];
}

export type FitMode = 'custom' | 'fit-page' | 'fit-width';

export interface SearchMatchRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface SearchMatch {
  srcIndex: number;
  start: number;
  len: number;
  snippet: string;
  rects: SearchMatchRect[];
}

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

export type AnnotationKind =
  'highlight' | 'ink' | 'rect' | 'textbox' | 'note' | 'image' | 'bubble' | 'signature';

/** The typed signature styles, one bundled handwriting font each. */
export type SignatureStyle = 'flowing' | 'script' | 'hand';

/** A visual signature's artwork in its own box of `w` x `h` units, y down.
 * The engine writes it as a Stamp and keeps this same JSON inside it, so a
 * saved signature reopens as itself (src-tauri/src/engine/signature.rs). */
export type SignatureArt =
  | {
      type: 'drawn';
      w: number;
      h: number;
      /** smoothed polylines, box units */
      strokes: PdfPoint[][];
      /** pen width, box units */
      width: number;
    }
  | {
      type: 'typed';
      w: number;
      h: number;
      text: string;
      style: SignatureStyle;
      /** the name's glyph outlines as SVG path data, box units */
      d: string;
    }
  | {
      type: 'image';
      w: number;
      h: number;
      /** base64 PNG with a transparent background */
      png: string;
    };

export type TextFontFamily = 'Helvetica' | 'Times-Roman' | 'Courier';

export interface TextFormat {
  fontFamily: TextFontFamily;
  bold: boolean;
  italic: boolean;
  underline: boolean;
}

export interface Annotation extends Partial<TextFormat> {
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
  /** New text boxes fit their contents until explicitly resized. */
  autoSize?: boolean;
  /** image source path (absolute, chosen via native dialog). */
  sourcePath?: string;
  /** natural pixel size of the source image. */
  naturalW?: number;
  naturalH?: number;
  /** For a shaded bubble: the character index of the glyph it fills, so the
   * same bubble can be recognised again and un-shaded. */
  bubbleCharIndex?: number;
  /** A visual signature's artwork (`kind: 'signature'`). */
  signature?: SignatureArt;
  /** Set only on annotations read out of the file: this annotation's position
   * in PDFium's enumeration of its source page.
   *
   * It is the identity the save path uses to tell an annotation it should
   * replace from one it should leave exactly as it found it. Annotations drawn
   * this session have no source index, so they are always written fresh. */
  srcAnnotIndex?: number;
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

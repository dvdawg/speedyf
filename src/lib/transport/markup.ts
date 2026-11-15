/**
 * Typed transport for Markup (`dev.speedyf.markup`): comment storage per
 * document and the annotated-PDF export (`src-tauri/src/commands/markup.rs`),
 * plus the three calls the hidden print page makes while its sheets are
 * captured (`src-tauri/src/markup_capture.rs`).
 */
import { invoke } from '@tauri-apps/api/core';
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';

/** Display-normalized page space: y-up, origin at the displayed crop-box
 * bottom-left, pt (the annotation tools' space). */
export interface MarkupRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface MarkupQuad {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  x3: number;
  y3: number;
  x4: number;
  y4: number;
}

/** `label`: a comment's number by its phrase, drawn into the page (`rect` is
 * its pill, `contents` the number). */
export type MarkupMarkKind = 'highlight' | 'strikeout' | 'caret' | 'square' | 'label';

/** A native PDF annotation written on a source page. */
export interface MarkupMark {
  page: number;
  kind: MarkupMarkKind;
  quads: MarkupQuad[] | null;
  rect: MarkupRect | null;
  /** `#rrggbb` */
  color: string;
  /** Plain text, prefixed by the comment's number: "3. Justify this step: ..." */
  contents: string;
}

export type MarkupLinkTarget =
  /** `y`: pt from the target sheet's top. */
  | { kind: 'sheet'; sheet: number; y: number }
  /** `y`: display pt from the source page's top. */
  | { kind: 'page'; page: number; y: number };

export interface MarkupSheetLink {
  /** Sheet-relative, CSS px == pt, top-left origin. */
  rect: MarkupRect;
  target: MarkupLinkTarget;
}

export interface MarkupSheet {
  /** `margin`: placed in the widened strip of `page`. `feedback`: appended
   * after the last source page, in order. */
  kind: 'margin' | 'feedback';
  page?: number;
  widthPt: number;
  heightPt: number;
  /** Body fragment, math already rendered to HTML. */
  html: string;
  links?: MarkupSheetLink[];
}

export interface MarkupExportRequest {
  source: string;
  /** Never the source; the engine refuses it. */
  dest: string;
  /** Width added on the right of every page, display space. */
  marginPt: number;
  marks: MarkupMark[];
  sheets: MarkupSheet[];
  /** Extra CSS for the print page, on top of `features/markup/print.css`. */
  css?: string;
}

export interface MarkupExportResult {
  path: string;
  /** Pages in the written file. */
  pages: number;
  /** Internal links written (0 when the file could not take them). */
  links: number;
}

export const markupApi = {
  /** The stored comment JSON for the PDF at `path`, or null. */
  load: (path: string): Promise<string | null> => invoke('markup_load', { path }),
  /** Store `json` for the PDF at `path` (atomic). */
  save: (path: string, json: string): Promise<void> => invoke('markup_save', { path, json }),
  /** Write the annotated copy. macOS only while sheets are rendered by WebKit. */
  exportPdf: (request: MarkupExportRequest): Promise<MarkupExportResult> =>
    invoke('markup_export', { request }),
};

/** Development builds only: the app's markup self-test drives the real
 * export from the main window (`src-tauri/src/markup_selftest.rs`). */
export const markupSelftestApi = {
  ready: (): Promise<void> => emit('markup-selftest:ready'),
  onRun: (cb: (run: { source: string; dest: string }) => void): Promise<UnlistenFn> =>
    listen<{ source: string; dest: string }>('markup-selftest:run', (e) => cb(e.payload)),
  done: (report: { ok: boolean; message: string; detail?: unknown }): Promise<void> =>
    emit('markup-selftest:done', report),
};

/** One sheet as the print page renders it. */
export interface PrintSheet {
  widthPt: number;
  heightPt: number;
  html: string;
}

export interface PrintPayload {
  sheets: PrintSheet[];
  css: string;
}

/** The print page's side of a capture. Only a capture window has a job. */
export const markupPrintApi = {
  payload: (): Promise<PrintPayload> => invoke('markup_print_payload'),
  /** Capture sheet `index`, now shown at the page's top-left corner. */
  capture: (index: number): Promise<void> => invoke('markup_print_capture', { index }),
  finish: (error: string | null): Promise<void> => invoke('markup_print_finish', { error }),
};

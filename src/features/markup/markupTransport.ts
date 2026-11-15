/** The backend's Markup commands (see the shared contract): load and save
 * the review of a PDF, and write the annotated copy. Typed wrappers, one
 * place to mock in tests. */
import { invoke } from '@tauri-apps/api/core';
import type { Quad, Rect } from './types';

export type MarkKind = 'highlight' | 'strikeout' | 'caret' | 'square';

/** A native PDF annotation on a source page, display space. */
export interface ExportMark {
  page: number;
  kind: MarkKind;
  quads: Quad[] | null;
  rect: Rect | null;
  color: string;
  contents: string;
}

export type LinkTarget =
  { kind: 'sheet'; sheet: number; y: number } | { kind: 'page'; page: number; y: number };

/** A link on a sheet: sheet-relative, top-left origin, CSS px == pt. */
export interface SheetLink {
  rect: Rect;
  target: LinkTarget;
}

export interface ExportSheet {
  kind: 'margin' | 'feedback';
  /** The source page a margin sheet widens; absent for feedback. */
  page?: number;
  widthPt: number;
  heightPt: number;
  html: string;
  links: SheetLink[];
}

export interface MarkupExportRequest {
  source: string;
  dest: string;
  marginPt: number;
  marks: ExportMark[];
  sheets: ExportSheet[];
  css?: string;
}

export interface MarkupExportResult {
  path: string;
  pages: number;
}

export interface MarkupTransport {
  load(path: string): Promise<string | null>;
  save(path: string, json: string): Promise<void>;
  exportPdf(request: MarkupExportRequest): Promise<MarkupExportResult>;
}

export const markupTransport: MarkupTransport = {
  load: (path) => invoke<string | null>('markup_load', { path }),
  save: (path, json) => invoke<void>('markup_save', { path, json }),
  exportPdf: (request) => invoke<MarkupExportResult>('markup_export', { request }),
};

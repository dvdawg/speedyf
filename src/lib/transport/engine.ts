/**
 * Typed transport to the Rust engine. This is the ONLY module that talks to
 * Tauri invoke/events, so the rest of the app depends on `EngineApi` and can
 * be pointed at a different backend (or a mock) without touching features.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  DocMeta,
  CitationId,
  FormFieldInfo,
  LibraryStatus,
  PageLinks,
  PageTextLayout,
  PreviewSpec,
  ResolvedCitation,
  SaveResult,
  SearchQueryResult,
} from '../../types/engine';
import type { EditPlan } from '../../features/document/documentStore';

export interface SearchProgress {
  docId: number;
  indexed: number;
  total: number;
  truncated: boolean;
}

export interface EngineMetrics {
  rendered: number;
  skippedStale: number;
  cacheHits: number;
  cacheLookups: number;
  pageCacheBytes: number;
  pageCacheBudget: number;
  thumbCacheBytes: number;
  thumbCacheBudget: number;
  previewCacheBytes: number;
  previewCacheBudget: number;
  textBytes: number;
  textBudget: number;
  pagesIndexed: number;
  queueDepth: number;
}

export interface EngineApi {
  open(path: string, password?: string): Promise<DocMeta>;
  close(docId: number): Promise<void>;
  /** Marks which doc is foreground for citation-hover bookkeeping; opening a
   * background tab does not implicitly change this. */
  setActiveDocument(docId: number | null): Promise<void>;
  /** Plain filesystem stat (not a PDFium operation) for the home screen's
   * recent-files list. */
  fileMetadata(path: string): Promise<{ sizeBytes: number; modifiedMs: number }>;
  requestPageSizes(
    docId: number,
    from: number,
    count: number
  ): Promise<{ from: number; sizes: [number, number, number, number, number][] }>;
  getTextLayout(docId: number, src: number): Promise<PageTextLayout>;
  getPageLinks(docId: number, src: number): Promise<PageLinks>;
  getPreviewRect(
    docId: number,
    src: number,
    x: number | null,
    y: number | null
  ): Promise<PreviewSpec>;
  resolveCitation(id: CitationId): Promise<ResolvedCitation | null>;
  setLibraryRoot(path: string | null): Promise<void>;
  libraryStatus(): Promise<LibraryStatus>;
  startIndexing(docId: number): Promise<void>;
  searchQuery(docId: number, query: string, caseSensitive: boolean): Promise<SearchQueryResult>;
  getMatchRects(
    docId: number,
    src: number,
    start: number,
    len: number
  ): Promise<[number, number, number, number][]>;
  saveDocument(docId: number, plan: EditPlan, destPath: string): Promise<SaveResult>;
  getFormFields(docId: number): Promise<FormFieldInfo[]>;
  imageSize(path: string): Promise<[number, number]>;
  metrics(): Promise<EngineMetrics>;
  setLowMemory(enabled: boolean): Promise<void>;
  bumpGeneration(docId: number): Promise<number>;
  onSearchProgress(cb: (p: SearchProgress) => void): Promise<() => void>;
}

export const engine: EngineApi = {
  open: (path, password) => invoke('open_document', { path, password: password ?? null }),
  close: (docId) => invoke('close_document', { docId }),
  setActiveDocument: (docId) => invoke('set_active_document', { docId }),
  fileMetadata: (path) => invoke('file_metadata', { path }),
  requestPageSizes: (docId, from, count) => invoke('request_page_sizes', { docId, from, count }),
  getTextLayout: (docId, src) => invoke('get_text_layout', { docId, src }),
  getPageLinks: (docId, src) => invoke('get_page_links', { docId, src }),
  getPreviewRect: (docId, src, x, y) => invoke('get_preview_rect', { docId, src, x, y }),
  resolveCitation: (id) => invoke('resolve_citation', { id }),
  setLibraryRoot: (path) => invoke('set_library_root', { path }),
  libraryStatus: () => invoke('library_status'),
  startIndexing: (docId) => invoke('start_indexing', { docId }),
  searchQuery: (docId, query, caseSensitive) =>
    invoke('search_query', { docId, query, caseSensitive }),
  getMatchRects: (docId, src, start, len) => invoke('get_match_rects', { docId, src, start, len }),
  saveDocument: (docId, plan, destPath) => invoke('save_document', { docId, plan, destPath }),
  getFormFields: (docId) => invoke('get_form_fields', { docId }),
  imageSize: (path) => invoke('image_size', { path }),
  metrics: () => invoke('engine_metrics'),
  setLowMemory: (enabled) => invoke('set_low_memory', { enabled }),
  bumpGeneration: (docId) => invoke('bump_generation', { docId }),
  onSearchProgress: async (cb) => listen<SearchProgress>('search:progress', (e) => cb(e.payload)),
};

/** Error shape produced by the Rust AppError serializer. */
export interface EngineError {
  code:
    'not_found' | 'engine' | 'password' | 'malformed' | 'io' | 'stale' | 'unsupported' | 'internal';
  message: string;
}

export function isEngineError(e: unknown): e is EngineError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'code' in e &&
    'message' in e &&
    typeof (e as EngineError).message === 'string'
  );
}

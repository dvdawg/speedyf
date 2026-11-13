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
  Figure,
  LibrarySearchResult,
  Printer,
  PrintJob,
  PrintOption,
  FormalEntry,
  FormFieldInfo,
  LibraryStatus,
  OutlineNode,
  Bubble,
  PageAnnotations,
  PageBubbles,
  PageLinks,
  PageTextLayout,
  PreviewSpec,
  ResolvedCitation,
  SaveResult,
  SearchQueryResult,
} from '../../types/engine';
import type {
  CitationLinks,
  DuplicateGroup,
  Graph,
  LibraryCounts,
  LibraryDataInfo,
  LookupBatch,
  LookupCandidate,
  LookupMode,
  LookupResult,
  MetaPatch,
  MissingWork,
  Paper,
  PaperFilter,
  PaperList,
  Paperness,
  Project,
} from '../../types/engine';
import type { EditPlan } from '../../features/document/documentStore';

/** What the status bar shows while lookups run: whether any is in flight,
 * and for a batch, how far along it is (both 0 for a single lookup). */
export interface LookupActivity {
  active: boolean;
  done: number;
  total: number;
}

/** A graph around one item across the whole library: what it cites and what
 * cites it, `depth` steps out (1 or 2). Overrides the project. */
export interface GraphFocus {
  paperId: string;
  depth: 1 | 2;
}

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
  /** Bubbles and checkboxes the page draws as glyphs rather than declares as
   * form fields, for the shading tool. */
  getPageBubbles(docId: number, src: number): Promise<Bubble[]>;
  getPreviewRect(
    docId: number,
    src: number,
    x: number | null,
    y: number | null
  ): Promise<PreviewSpec>;
  resolveCitation(id: CitationId): Promise<ResolvedCitation | null>;
  /** Adds a folder to the library, or removes one. */
  changeLibraryRoot(path: string, add: boolean): Promise<void>;
  /** Searches every document whose text has been indexed. */
  librarySearch(query: string, caseSensitive: boolean): Promise<LibrarySearchResult>;
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
  getOutline(docId: number): Promise<OutlineNode[]>;
  /** Annotations already in the file, so they can be edited rather than
   * merely looked at. */
  getAnnotations(docId: number): Promise<PageAnnotations[]>;
  getFormalEnvs(docId: number): Promise<FormalEntry[]>;
  getFigures(docId: number): Promise<Figure[]>;
  imageSize(path: string): Promise<[number, number]>;
  metrics(): Promise<EngineMetrics>;
  setLowMemory(enabled: boolean): Promise<void>;
  /** Invalidates every outstanding render URL for the document and flags its
   * cached rasters for eviction. Only for changes that make cached pixels
   * wrong — it forces the webview to re-decode every mounted image. */
  bumpGeneration(docId: number): Promise<number>;
  /** Drops queued render work without invalidating URLs or caches. */
  cancelRenders(docId: number): Promise<void>;
  /** Opens a document link in the user's browser. Rejects anything outside
   * the engine's scheme allowlist, whatever the frontend passed. */
  openExternalUrl(url: string): Promise<void>;
  /** Writes the document, including unsaved edits, to a temp file for
   * printing and returns its path. The user's own file is untouched. */
  buildPrintPdf(docId: number, plan: EditPlan): Promise<string>;
  /** Deletes a prepared print job. Refuses any path the engine did not make. */
  discardPrintPdf(path: string): Promise<void>;
  /** Saves a prepared print job where the user chose — "print to PDF". */
  exportPrintPdf(path: string, destPath: string): Promise<void>;
  listPrinters(): Promise<Printer[]>;
  printerOptions(printer: string): Promise<PrintOption[]>;
  /** Hands a prepared job to the printer. Re-validates every field. */
  submitPrint(job: PrintJob, path: string): Promise<void>;
  onSearchProgress(cb: (p: SearchProgress) => void): Promise<() => void>;

  /* ---- projects and paper records (see projects.md) ---- */
  libraryPapers(filter: PaperFilter): Promise<PaperList>;
  libraryCounts(): Promise<LibraryCounts>;
  libraryPaper(id: string): Promise<Paper>;
  /** The paper whose file is at this path, or null when the path is not in
   * the library. */
  libraryPaperForPath(path: string): Promise<Paper | null>;
  libraryUpdatePaper(id: string, patch: MetaPatch): Promise<Paper>;
  libraryProjects(): Promise<Project[]>;
  libraryCreateProject(name: string): Promise<Project>;
  libraryRenameProject(id: string, name: string): Promise<void>;
  libraryDeleteProject(id: string): Promise<void>;
  libraryReorderProjects(ids: string[]): Promise<void>;
  librarySetMembership(paperId: string, projectId: string, member: boolean): Promise<void>;
  /** Opens the paper's first file as a library document for a first-page
   * render; the same shape a citation hover gets. Null when every file is
   * missing. */
  libraryPreview(paperId: string): Promise<ResolvedCitation | null>;
  /** Whether the open document looks like a paper, from its first page and
   * Info dictionary. */
  libraryClassifyDocument(docId: number): Promise<Paperness>;
  /** Copies a PDF from outside the library into `inboxDir`, scans the copy
   * and returns its paper. */
  libraryFileIntoInbox(path: string, inboxDir: string): Promise<Paper>;
  /** BibTeX for these papers, in this order. */
  libraryBibtex(paperIds: string[]): Promise<string>;
  libraryExportBib(projectId: string, path: string): Promise<void>;
  librarySetProjectBib(projectId: string, path: string | null): Promise<void>;
  libraryLookup(paperId: string, mode: LookupMode): Promise<LookupResult>;
  libraryApplyCandidate(paperId: string, candidate: LookupCandidate): Promise<Paper>;
  /** Looks up every paper in the project that has an identifier, by
   * identifier only, applying each result. Sequential and cache-first. */
  libraryLookupProject(projectId: string): Promise<LookupBatch>;
  /** Tells the engine whether online lookup is allowed and whom to identify
   * as. Called at startup and whenever the setting changes, like low memory. */
  librarySetLookup(enabled: boolean, contact: string, s2Key: string): Promise<void>;
  /** Looks up every paper in the library: by identifier where there is one,
   * by title otherwise. Sequential and cache-first; non-papers are skipped. */
  libraryLookupAll(): Promise<LookupBatch>;
  /** Drops the parked title-search candidates on a paper. */
  libraryDiscardCandidates(paperId: string): Promise<void>;
  libraryCitationLinks(paperId: string): Promise<CitationLinks>;
  /** The citation graph of a project (its members) or, with null, of every
   * item in the library. One call; edges only between the returned nodes. */
  libraryGraph(projectId: string | null, focus?: GraphFocus): Promise<Graph>;
  /** Works cited by at least `minCitedBy` items of the view (a project, the
   * library, or a focus neighbourhood) that are not in the library. */
  libraryMissingWorks(
    projectId: string | null,
    minCitedBy: number,
    focus?: GraphFocus
  ): Promise<MissingWork[]>;
  /** Groups of records the engine suspects are one work. */
  libraryDuplicates(): Promise<DuplicateGroup[]>;
  /** Merges `mergeIds` into `keepId`: files, memberships, identifiers, best
   * metadata. Returns the merged record. Files on disk are untouched. */
  libraryMergePapers(keepId: string, mergeIds: string[]): Promise<Paper>;
  /** Undoes every merge into this record, restoring the records as they were. */
  libraryUnmerge(paperId: string): Promise<Paper[]>;
  /** Records that these are not one work, so they are never suggested
   * (or joined through a third record) again. */
  libraryNotDuplicates(paperIds: string[]): Promise<void>;
  libraryDataDir(): Promise<LibraryDataInfo>;
  libraryMoveData(dir: string): Promise<LibraryDataInfo>;
  /** Fires after any write to papers or projects, in every window. */
  onLibraryChanged(cb: () => void): Promise<() => void>;
  /** Fires when an online metadata lookup starts (`true`) and when the last
   * one in flight finishes (`false`). The status bar shows an indicator for
   * as long as it is true, so the user can always tell the app is talking
   * to a server (projects.md §4). */
  onLookupActivity(cb: (status: LookupActivity) => void): Promise<() => void>;
}

export const engine: EngineApi = {
  open: (path, password) => invoke('open_document', { path, password: password ?? null }),
  close: (docId) => invoke('close_document', { docId }),
  setActiveDocument: (docId) => invoke('set_active_document', { docId }),
  fileMetadata: (path) => invoke('file_metadata', { path }),
  requestPageSizes: (docId, from, count) => invoke('request_page_sizes', { docId, from, count }),
  getTextLayout: (docId, src) => invoke('get_text_layout', { docId, src }),
  getPageLinks: (docId, src) => invoke('get_page_links', { docId, src }),
  getPageBubbles: async (docId, src) =>
    (await invoke<PageBubbles>('get_page_bubbles', { docId, src })).bubbles,
  getPreviewRect: (docId, src, x, y) => invoke('get_preview_rect', { docId, src, x, y }),
  resolveCitation: (id) => invoke('resolve_citation', { id }),
  changeLibraryRoot: (path, add) => invoke('change_library_root', { path, add }),
  librarySearch: (query, caseSensitive) => invoke('library_search', { query, caseSensitive }),
  libraryStatus: () => invoke('library_status'),
  startIndexing: (docId) => invoke('start_indexing', { docId }),
  searchQuery: (docId, query, caseSensitive) =>
    invoke('search_query', { docId, query, caseSensitive }),
  getMatchRects: (docId, src, start, len) => invoke('get_match_rects', { docId, src, start, len }),
  saveDocument: (docId, plan, destPath) => invoke('save_document', { docId, plan, destPath }),
  getFormFields: (docId) => invoke('get_form_fields', { docId }),
  getOutline: (docId) => invoke('get_outline', { docId }),
  getAnnotations: (docId) => invoke('get_annotations', { docId }),
  getFormalEnvs: (docId) => invoke('get_formal_envs', { docId }),
  getFigures: (docId) => invoke('get_figures', { docId }),
  imageSize: (path) => invoke('image_size', { path }),
  metrics: () => invoke('engine_metrics'),
  setLowMemory: (enabled) => invoke('set_low_memory', { enabled }),
  bumpGeneration: (docId) => invoke('bump_generation', { docId }),
  cancelRenders: (docId) => invoke('cancel_renders', { docId }),
  openExternalUrl: (url) => invoke('open_external_url', { url }),
  buildPrintPdf: (docId, plan) => invoke('build_print_pdf', { docId, plan }),
  discardPrintPdf: (path) => invoke('discard_print_pdf', { path }),
  exportPrintPdf: (path, destPath) => invoke('export_print_pdf', { path, destPath }),
  listPrinters: () => invoke('list_printers'),
  printerOptions: (printer) => invoke('printer_options', { printer }),
  submitPrint: (job, path) => invoke('submit_print', { job, path }),
  onSearchProgress: async (cb) => listen<SearchProgress>('search:progress', (e) => cb(e.payload)),

  libraryPapers: (filter) => invoke('library_papers', { filter }),
  libraryCounts: () => invoke('library_counts'),
  libraryPaper: (id) => invoke('library_paper', { id }),
  libraryPaperForPath: (path) => invoke('library_paper_for_path', { path }),
  libraryUpdatePaper: (id, patch) => invoke('library_update_paper', { id, patch }),
  libraryProjects: () => invoke('library_projects'),
  libraryCreateProject: (name) => invoke('library_create_project', { name }),
  libraryRenameProject: (id, name) => invoke('library_rename_project', { id, name }),
  libraryDeleteProject: (id) => invoke('library_delete_project', { id }),
  libraryReorderProjects: (ids) => invoke('library_reorder_projects', { ids }),
  librarySetMembership: (paperId, projectId, member) =>
    invoke('library_set_membership', { paperId, projectId, member }),
  libraryPreview: (paperId) => invoke('library_preview', { paperId }),
  libraryClassifyDocument: (docId) => invoke('library_classify_document', { docId }),
  libraryFileIntoInbox: (path, inboxDir) => invoke('library_file_into_inbox', { path, inboxDir }),
  libraryBibtex: (paperIds) => invoke('library_bibtex', { paperIds }),
  libraryExportBib: (projectId, path) => invoke('library_export_bib', { projectId, path }),
  librarySetProjectBib: (projectId, path) => invoke('library_set_project_bib', { projectId, path }),
  libraryLookup: (paperId, mode) => invoke('library_lookup', { paperId, mode }),
  libraryApplyCandidate: (paperId, candidate) =>
    invoke('library_apply_candidate', { paperId, candidate }),
  libraryLookupProject: (projectId) => invoke('library_lookup_project', { projectId }),
  librarySetLookup: (enabled, contact, s2Key) =>
    invoke('library_set_lookup', { enabled, contact, s2Key }),
  libraryLookupAll: () => invoke('library_lookup_all'),
  libraryDiscardCandidates: (paperId) => invoke('library_discard_candidates', { paperId }),
  libraryCitationLinks: (paperId) => invoke('library_citation_links', { paperId }),
  libraryGraph: (projectId, focus) =>
    invoke('library_graph', {
      projectId,
      focus: focus?.paperId ?? null,
      depth: focus?.depth ?? null,
    }),
  libraryMissingWorks: (projectId, minCitedBy, focus) =>
    invoke('library_missing_works', {
      projectId,
      minCitedBy,
      focus: focus?.paperId ?? null,
      depth: focus?.depth ?? null,
    }),
  libraryDuplicates: () => invoke('library_duplicates'),
  libraryMergePapers: (keepId, mergeIds) => invoke('library_merge_papers', { keepId, mergeIds }),
  libraryUnmerge: (paperId) => invoke('library_unmerge', { paperId }),
  libraryNotDuplicates: (paperIds) => invoke('library_not_duplicates', { paperIds }),
  libraryDataDir: () => invoke('library_data_dir'),
  libraryMoveData: (dir) => invoke('library_move_data', { dir }),
  onLibraryChanged: async (cb) => listen('library:changed', () => cb()),
  onLookupActivity: async (cb) =>
    listen<LookupActivity>('library:lookup', (e) =>
      cb({ active: e.payload.active, done: e.payload.done ?? 0, total: e.payload.total ?? 0 })
    ),
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

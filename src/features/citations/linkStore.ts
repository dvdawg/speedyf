/** Per-document citation hover/preview session and internal-reference
 * navigation history. One instance per open tab (see tabsStore.ts);
 * `createCitationStore()` is the factory, with a temporary module-level
 * singleton kept below until the tab-registry wiring lands.
 * Library-folder scan status lives in libraryStore.ts — genuinely global,
 * independent of any open document. */
import { createStore } from 'solid-js/store';
import { engine, isEngineError } from '../../lib/transport/engine';
import type {
  CitationId,
  LinkTarget,
  PageLinks,
  PreviewSpec,
  ResolvedCitation,
} from '../../types/engine';
import type { DocumentStore } from '../document/documentStore';
import type { ViewportStore } from '../../stores/viewportStore';
import { libraryStore } from './libraryStore';
import { citationKey, citationLabel } from './citationLabel';

export type HoverPhase = 'idle' | 'dwelling' | 'loading' | 'shown' | 'leaving';

export interface AnchorRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

type HoverableTarget = Exclude<LinkTarget, { kind: 'unknown' }>;

export interface HoverRequest {
  key: string;
  docId: number;
  sourceSrc: number;
  target: HoverableTarget;
  anchor: AnchorRect;
}

export type HoverPreview =
  | {
      kind: 'internal';
      preview: PreviewSpec;
      page: number;
      x: number | null;
      y: number | null;
    }
  | {
      kind: 'external';
      id: CitationId;
      uri: string;
      resolved: ResolvedCitation;
    }
  | {
      kind: 'external-unresolved';
      id: CitationId | null;
      uri: string;
      libraryRoot: string | null;
      libraryScanning: boolean;
    }
  | { kind: 'error'; message: string; label: string };

export interface HoverSnapshot<Request, Result> {
  phase: HoverPhase;
  request: Request | null;
  result: Result | null;
  error: string | null;
}

interface MachineOptions<Result> {
  dwellMs?: number;
  leaveMs?: number;
  cacheSize?: number;
  cacheResult?: (result: Result) => boolean;
}

/** Explicit dwell/loading/leaving machine. It serializes loads even when a
 * stale request is still running, guaranteeing at most one engine call. */
export class HoverMachine<Request extends { key: string }, Result> {
  private state: HoverSnapshot<Request, Result> = {
    phase: 'idle',
    request: null,
    result: null,
    error: null,
  };
  private readonly listeners = new Set<(state: HoverSnapshot<Request, Result>) => void>();
  private readonly cache = new Map<string, Result>();
  private readonly dwellMs: number;
  private readonly leaveMs: number;
  private readonly cacheSize: number;
  private readonly cacheResult: (result: Result) => boolean;
  private hoverSeq = 0;
  private inFlight = false;
  private queued: { seq: number; request: Request } | null = null;
  private dwellTimer: ReturnType<typeof setTimeout> | undefined;
  private leaveTimer: ReturnType<typeof setTimeout> | undefined;
  private phaseBeforeLeaving: HoverPhase = 'shown';

  constructor(
    private readonly loader: (request: Request) => Promise<Result>,
    options: MachineOptions<Result> = {}
  ) {
    this.dwellMs = options.dwellMs ?? 200;
    this.leaveMs = options.leaveMs ?? 120;
    this.cacheSize = options.cacheSize ?? 50;
    this.cacheResult = options.cacheResult ?? (() => true);
  }

  snapshot(): HoverSnapshot<Request, Result> {
    return this.state;
  }

  subscribe(listener: (state: HoverSnapshot<Request, Result>) => void): () => void {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }

  enter(request: Request, keyboard = false): void {
    this.clearTimers();
    const seq = ++this.hoverSeq;
    this.queued = null;
    const cached = this.cache.get(request.key);
    if (cached !== undefined) {
      this.cache.delete(request.key);
      this.cache.set(request.key, cached);
      this.update({ phase: 'shown', request, result: cached, error: null });
      return;
    }
    this.update({ phase: 'dwelling', request, result: null, error: null });
    this.dwellTimer = setTimeout(() => this.beginLoad(seq, request), keyboard ? 0 : this.dwellMs);
  }

  leave(): void {
    if (this.state.phase === 'idle' || !this.state.request) return;
    if (this.dwellTimer !== undefined) {
      clearTimeout(this.dwellTimer);
      this.dwellTimer = undefined;
    }
    this.phaseBeforeLeaving = this.state.phase;
    this.update({ ...this.state, phase: 'leaving' });
    if (this.leaveTimer !== undefined) clearTimeout(this.leaveTimer);
    this.leaveTimer = setTimeout(() => this.close(), this.leaveMs);
  }

  enterPopover(): void {
    if (this.leaveTimer !== undefined) {
      clearTimeout(this.leaveTimer);
      this.leaveTimer = undefined;
    }
    if (this.state.phase === 'leaving') {
      const phase = this.state.result || this.state.error ? 'shown' : this.phaseBeforeLeaving;
      this.update({ ...this.state, phase: phase === 'leaving' ? 'loading' : phase });
    }
  }

  close(): void {
    this.clearTimers();
    this.hoverSeq += 1;
    this.queued = null;
    this.update({ phase: 'idle', request: null, result: null, error: null });
  }

  reset(): void {
    this.close();
    this.cache.clear();
  }

  invalidateCache(prefix?: string): void {
    if (!prefix) {
      this.cache.clear();
      return;
    }
    for (const key of this.cache.keys()) {
      if (key.startsWith(prefix)) this.cache.delete(key);
    }
  }

  private beginLoad(seq: number, request: Request): void {
    this.dwellTimer = undefined;
    if (seq !== this.hoverSeq || this.state.request?.key !== request.key) return;
    this.update({ ...this.state, phase: 'loading', request, result: null, error: null });
    if (this.inFlight) {
      this.queued = { seq, request };
      return;
    }
    this.inFlight = true;
    void this.loader(request)
      .then((result) => {
        if (seq !== this.hoverSeq || this.state.request?.key !== request.key) return;
        if (this.cacheResult(result)) this.remember(request.key, result);
        if (this.state.phase === 'leaving') {
          this.update({ ...this.state, result, error: null });
        } else {
          this.update({ phase: 'shown', request, result, error: null });
        }
      })
      .catch((error: unknown) => {
        if (seq !== this.hoverSeq || this.state.request?.key !== request.key) return;
        const message = error instanceof Error ? error.message : String(error);
        if (this.state.phase === 'leaving') {
          this.update({ ...this.state, result: null, error: message });
        } else {
          this.update({ phase: 'shown', request, result: null, error: message });
        }
      })
      .finally(() => {
        this.inFlight = false;
        const queued = this.queued;
        this.queued = null;
        if (queued && queued.seq === this.hoverSeq) this.beginLoad(queued.seq, queued.request);
      });
  }

  private remember(key: string, result: Result): void {
    this.cache.delete(key);
    this.cache.set(key, result);
    while (this.cache.size > this.cacheSize) {
      const oldest = this.cache.keys().next().value as string | undefined;
      if (oldest === undefined) break;
      this.cache.delete(oldest);
    }
  }

  private clearTimers(): void {
    if (this.dwellTimer !== undefined) clearTimeout(this.dwellTimer);
    if (this.leaveTimer !== undefined) clearTimeout(this.leaveTimer);
    this.dwellTimer = undefined;
    this.leaveTimer = undefined;
  }

  private update(state: HoverSnapshot<Request, Result>): void {
    this.state = state;
    for (const listener of this.listeners) listener(state);
  }
}

export interface ReadingPosition {
  docId: number;
  scrollTop: number;
  scrollLeft: number;
}

/** Bounded LIFO view history for internal PDF jumps. Kept independent of
 * edit undo/redo because returning to a reading position must never mutate
 * the document. */
export class ReferenceNavigationHistory {
  private readonly entries: ReadingPosition[] = [];

  constructor(private readonly limit = 100) {}

  get depth(): number {
    return this.entries.length;
  }

  push(position: ReadingPosition): void {
    this.entries.push(position);
    if (this.entries.length > this.limit) this.entries.splice(0, this.entries.length - this.limit);
  }

  pop(docId: number): ReadingPosition | undefined {
    while (this.entries.length > 0) {
      const position = this.entries.pop()!;
      if (position.docId === docId) return position;
    }
    return undefined;
  }

  clear(): void {
    this.entries.length = 0;
  }
}

function targetKey(docId: number, target: HoverableTarget): string {
  if (target.kind === 'internal') {
    return `internal:${docId}:${target.page}:${target.x ?? ''}:${target.y ?? ''}`;
  }
  return target.citation
    ? `external:${citationKey(target.citation)}`
    : `external-uri:${target.uri}`;
}

function toAnchorRect(element: HTMLElement): AnchorRect {
  const rect = element.getBoundingClientRect();
  return {
    left: rect.left,
    top: rect.top,
    right: rect.right,
    bottom: rect.bottom,
    width: rect.width,
    height: rect.height,
  };
}

export interface CitationStore {
  state: { hover: HoverSnapshot<HoverRequest, HoverPreview>; navigationDepth: number };
  syncDocument(docId: number): void;
  linksForPage(docId: number, src: number): Promise<PageLinks>;
  enter(
    docId: number,
    sourceSrc: number,
    target: HoverableTarget,
    element: HTMLElement,
    keyboard?: boolean
  ): void;
  leave(): void;
  enterPopover(): void;
  close(): void;
  navigateInternalTarget(target: Extract<LinkTarget, { kind: 'internal' }>): void;
  goBackFromInternalTarget(): void;
  dispose(): void;
}

export function createCitationStore(doc: DocumentStore, vp: ViewportStore): CitationStore {
  const [state, setState] = createStore<{
    hover: HoverSnapshot<HoverRequest, HoverPreview>;
    navigationDepth: number;
  }>({
    hover: { phase: 'idle', request: null, result: null, error: null },
    navigationDepth: 0,
  });

  async function loadPreview(request: HoverRequest): Promise<HoverPreview> {
    if (request.target.kind === 'internal') {
      const target = request.target;
      const preview = await engine.getPreviewRect(request.docId, target.page, target.x, target.y);
      return { kind: 'internal', preview, page: target.page, x: target.x, y: target.y };
    }
    const { uri, citation } = request.target;
    if (!citation) {
      return {
        kind: 'external-unresolved',
        id: null,
        uri,
        libraryRoot: libraryStore.state.library.roots[0] ?? null,
        libraryScanning: libraryStore.state.library.scanning,
      };
    }
    try {
      const resolved = await engine.resolveCitation(citation);
      if (resolved) return { kind: 'external', id: citation, uri, resolved };
      return {
        kind: 'external-unresolved',
        id: citation,
        uri,
        libraryRoot: libraryStore.state.library.roots[0] ?? null,
        libraryScanning: libraryStore.state.library.scanning,
      };
    } catch (error) {
      return {
        kind: 'error',
        message: isEngineError(error) ? error.message : String(error),
        label: citationLabel(citation),
      };
    }
  }

  const machine = new HoverMachine<HoverRequest, HoverPreview>(loadPreview, {
    cacheSize: 50,
    cacheResult: (result) => result.kind !== 'error',
  });
  machine.subscribe((hover) => setState('hover', hover));
  const unsubscribeLibrary = libraryStore.onChange(() => machine.invalidateCache('external:'));

  const pageLinks = new Map<string, Promise<PageLinks>>();
  const PAGE_LINK_CACHE_ENTRIES = 256;
  let activeDocId = -1;
  const navigationHistory = new ReferenceNavigationHistory();

  function syncNavigationDepth(): void {
    setState('navigationDepth', navigationHistory.depth);
  }

  return {
    // getter, not a data property — see the note in tabsStore.ts
    get state() {
      return state;
    },

    syncDocument(docId: number): void {
      if (docId === activeDocId) return;
      activeDocId = docId;
      pageLinks.clear();
      machine.reset();
      navigationHistory.clear();
      syncNavigationDepth();
    },

    async linksForPage(docId: number, src: number): Promise<PageLinks> {
      const key = `${docId}:${src}`;
      const cached = pageLinks.get(key);
      if (cached) {
        pageLinks.delete(key);
        pageLinks.set(key, cached);
        return cached;
      }
      const pending = engine.getPageLinks(docId, src).catch(() => {
        pageLinks.delete(key);
        return { src, links: [] };
      });
      pageLinks.set(key, pending);
      while (pageLinks.size > PAGE_LINK_CACHE_ENTRIES) {
        const oldest = pageLinks.keys().next().value as string | undefined;
        if (oldest === undefined) break;
        pageLinks.delete(oldest);
      }
      return pending;
    },

    enter(
      docId: number,
      sourceSrc: number,
      target: HoverableTarget,
      element: HTMLElement,
      keyboard = false
    ): void {
      machine.enter(
        {
          key: targetKey(docId, target),
          docId,
          sourceSrc,
          target,
          anchor: toAnchorRect(element),
        },
        keyboard
      );
    },

    leave(): void {
      machine.leave();
    },

    enterPopover(): void {
      machine.enterPopover();
    },

    close(): void {
      machine.close();
    },

    navigateInternalTarget(target: Extract<LinkTarget, { kind: 'internal' }>): void {
      const pageIndex = doc.state.pages.findIndex((page) => page.srcIndex === target.page);
      if (pageIndex < 0) return;
      navigationHistory.push({
        docId: doc.state.docId,
        scrollTop: vp.state.scrollTop,
        scrollLeft: vp.state.scrollLeft,
      });
      syncNavigationDepth();
      const page = doc.state.pages[pageIndex];
      const offset =
        target.y === null || !page
          ? undefined
          : Math.max(0, (page.heightPt - target.y) * vp.state.zoom - vp.state.containerH * 0.2);
      vp.requestScrollToPage(pageIndex, offset);
    },

    goBackFromInternalTarget(): void {
      const position = navigationHistory.pop(doc.state.docId);
      syncNavigationDepth();
      if (position) vp.requestScrollToPosition(position.scrollTop, position.scrollLeft);
    },

    dispose(): void {
      unsubscribeLibrary();
    },
  };
}

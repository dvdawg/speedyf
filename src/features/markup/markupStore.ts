/** Markup's state for the window: whether review mode is on, each open
 * PDF's review (loaded with `markup_load`, saved a moment after every
 * change with `markup_save`), the comment being written, and what the
 * labels are worked out from.
 *
 * Review mode is window-level, like the reader's drawer: it stays on while
 * moving between documents, each showing its own comments. It works on any
 * PDF, in the library or not, and only while the Markup extension is
 * switched on. */
import { batch, createEffect, createRoot, createSignal, untrack } from 'solid-js';
import { createStore, produce, unwrap } from 'solid-js/store';
import { engine } from '../../lib/transport/engine';
import { activeTab } from '../../stores/tabsStore';
import { flashStatus } from '../../stores/statusMessage';
import { revealPlace } from '../../stores/revealStore';
import { selectedGeometry, selectedText, settleSelectionNow } from '../../stores/selectionStore';
import { extensionUsable } from '../extensions/extensionStore';
import { closeReaderPane } from '../extensions/surfaces';
import { docStatements } from '../extensions/paperDocs';
import { pickRegion } from '../extensions/regionCapture';
import { jumpToAnchor, layoutIndexOf } from '../outline/jumpToAnchor';
import { outlineAsEntries } from '../../lib/structure/context';
import { anchorStart, quadsFromRects, rectFromBox, revealRects } from './geometry';
import {
  EMPTY_CONTEXT,
  labelFor,
  proofStarts,
  type Heading,
  type LabelContext,
  type ProofStart,
  type StatementLike,
} from './labels';
import { buildEntries, newCommentId, parseMarkup, serializeMarkup, type Entry } from './model';
import { buildCiteIndex, EMPTY_INDEX, type CiteIndex } from './citations';
import { markupApi } from '../../lib/transport/markup';
import {
  emptyMarkup,
  MARKUP_EXTENSION_ID,
  type Anchor,
  type CommentKind,
  type MarkupComment,
  type MarkupDoc,
} from './types';

/** The backend's Markup calls; a fake in tests. */
export type MarkupTransport = Pick<typeof markupApi, 'load' | 'save' | 'exportPdf'>;

let transport: MarkupTransport = markupApi;

/** For tests: talk to a fake backend. */
export function setMarkupTransport(next: MarkupTransport): void {
  transport = next;
}

/* ---- review mode ---- */

const [on, setOn] = createSignal(false);

/** The Markup extension is installed, switched on and running clean. */
export function markupAvailable(): boolean {
  return extensionUsable(MARKUP_EXTENSION_ID);
}

/** Review mode is on (and still allowed). */
export function reviewing(): boolean {
  return on() && markupAvailable();
}

export function startReviewing(): void {
  if (!markupAvailable()) return;
  track();
  // One drawer at a time on the right: the review takes the place of an
  // extension pane.
  closeReaderPane();
  setOn(true);
}

export function stopReviewing(): void {
  flushSaves();
  batch(() => {
    setDraft(null);
    setEditing(null);
    setOn(false);
  });
}

export function toggleReviewing(): void {
  if (reviewing()) stopReviewing();
  else startReviewing();
}

/* ---- each document's review ---- */

export type ReviewStatus = 'loading' | 'ready' | 'failed';

interface ReviewEntry {
  status: ReviewStatus;
  doc: MarkupDoc;
}

const [reviews, setReviews] = createStore<Record<string, ReviewEntry>>({});

/** The active tab's document path, once it has loaded. */
export function activePath(): string | null {
  const doc = activeTab()?.documentStore.state;
  return doc?.loaded && doc.path ? doc.path : null;
}

export function reviewFor(path: string | null): ReviewEntry | null {
  return path ? (reviews[path] ?? null) : null;
}

/** Read a document's review from the backend, once. */
export function ensureLoaded(path: string): void {
  if (untrack(() => reviews[path])) return;
  setReviews(path, { status: 'loading', doc: emptyMarkup() });
  transport
    .load(path)
    .then((json) => setReviews(path, { status: 'ready', doc: parseMarkup(json) }))
    .catch(() => setReviews(path, 'status', 'failed'));
}

/** For tests: forget every review and the state around it. */
export function resetMarkupForTest(): void {
  for (const timer of saveTimers.values()) clearTimeout(timer);
  saveTimers.clear();
  batch(() => {
    for (const key of Object.keys(untrack(() => reviews))) setReviews(key, undefined!);
    setOn(false);
    setDraft(null);
    setEditing(null);
    setKindFilter(null);
    setCiteIndex(EMPTY_INDEX);
  });
}

/* ---- saving ---- */

const SAVE_DELAY_MS = 500;
const saveTimers = new Map<string, ReturnType<typeof setTimeout>>();
let saveFailed = false;

function writeNow(path: string): void {
  saveTimers.delete(path);
  const entry = untrack(() => reviews[path]);
  if (!entry || entry.status !== 'ready') return;
  transport.save(path, serializeMarkup(unwrap(entry.doc))).then(
    () => {
      saveFailed = false;
    },
    (err: unknown) => {
      if (saveFailed) return;
      saveFailed = true;
      flashStatus(`Could not save the review: ${err instanceof Error ? err.message : String(err)}`);
    }
  );
}

function scheduleSave(path: string): void {
  clearTimeout(saveTimers.get(path));
  saveTimers.set(
    path,
    setTimeout(() => writeNow(path), SAVE_DELAY_MS)
  );
}

/** Write every review waiting out its delay now. */
export function flushSaves(): void {
  for (const [path, timer] of [...saveTimers]) {
    clearTimeout(timer);
    writeNow(path);
  }
}

/** Change a document's review and save it a moment later. A review that
 * could not be read is never written over. */
function change(path: string, fn: (doc: MarkupDoc) => void): boolean {
  const entry = untrack(() => reviews[path]);
  if (!entry || entry.status !== 'ready') return false;
  setReviews(path, 'doc', produce(fn));
  scheduleSave(path);
  return true;
}

export function setSummary(path: string, summary: string): void {
  change(path, (doc) => {
    doc.summary = summary;
  });
}

export function updateComment(
  path: string,
  id: string,
  patch: Partial<Pick<MarkupComment, 'kind' | 'body' | 'replacement' | 'quickmark'>>
): void {
  change(path, (doc) => {
    const c = doc.comments.find((x) => x.id === id);
    if (c) Object.assign(c, patch);
  });
}

export function deleteComment(path: string, id: string): void {
  const entry = untrack(() => reviews[path]);
  const at = entry?.doc.comments.findIndex((c) => c.id === id) ?? -1;
  if (!entry || at < 0) return;
  const removed = structuredClone(unwrap(entry.doc.comments[at]!));
  if (editing() === id) setEditing(null);
  change(path, (doc) => {
    doc.comments.splice(at, 1);
  });
  flashStatus('Comment deleted', {
    action: {
      label: 'Undo',
      run: () =>
        change(path, (doc) => {
          if (!doc.comments.some((c) => c.id === removed.id)) doc.comments.push(removed);
        }),
    },
  });
}

export function addComment(path: string, comment: MarkupComment): boolean {
  return change(path, (doc) => {
    doc.comments.push(comment);
  });
}

/* ---- the comment being written ---- */

export type DraftMode = 'comment' | 'typo';

export interface Draft {
  path: string;
  mode: DraftMode;
  anchor: Anchor;
  kind: CommentKind;
  /** Text to start from (a suggestion's citation). */
  body?: string;
  quickmark?: string | null;
}

const [draft, setDraft] = createSignal<Draft | null>(null);
const [editing, setEditing] = createSignal<string | null>(null);
const [kindFilter, setKindFilter] = createSignal<CommentKind | null>(null);
/** The card last pointed at from the page or jumped to: flashed once. */
const [flashed, setFlashed] = createSignal<{ id: string; token: number } | null>(null);
let flashToken = 0;

export { draft, editing, kindFilter, setKindFilter, flashed };

export function editComment(id: string | null): void {
  if (id) setDraft(null);
  setEditing(id);
}

export function flashCard(id: string): void {
  setFlashed({ id, token: ++flashToken });
}

/** A selection on a page of the active document, live (not one only
 * remembered): what C and T act on. */
export function liveSelection(): boolean {
  if (typeof document === 'undefined') return false;
  const selection = document.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return false;
  const node = selection.getRangeAt(0).startContainer;
  const el = node instanceof Element ? node : node.parentElement;
  if (!el?.closest('.viewer .text-layer')) return false;
  return selection.toString().trim() !== '';
}

/** Start a comment (or a typo fix) on the selected text. */
export function beginComment(mode: DraftMode): boolean {
  if (!reviewing()) return false;
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  const path = activePath();
  if (!tab || !doc || !path) return false;
  settleSelectionNow();
  const geometry = selectedGeometry();
  const text = selectedText().replace(/\s+/g, ' ').trim();
  if (!geometry || geometry.docId !== doc.docId || !text) return false;
  const page = doc.pages[geometry.page];
  if (!page || page.srcIndex === null) return false;
  const quads = quadsFromRects(geometry.rects, page.heightPt);
  if (quads.length === 0) return false;
  ensureLoaded(path);
  document.getSelection()?.removeAllRanges();
  batch(() => {
    setEditing(null);
    setDraft({
      path,
      mode,
      anchor: { page: page.srcIndex!, quads, rect: null, quote: text },
      kind: mode === 'typo' ? 'typo' : 'exposition',
      quickmark: mode === 'typo' ? 'typo' : null,
    });
  });
  return true;
}

/** Drag a box on a page for a figure, an equation, a diagram. */
export async function beginRegion(): Promise<boolean> {
  if (!reviewing()) return false;
  const tab = activeTab();
  const path = activePath();
  if (!tab || !path) return false;
  const picked = await pickRegion(
    'Markup: drag a box round what the comment is about. Escape cancels.'
  );
  if (!picked) return false;
  const doc = tab.documentStore.state;
  const page = doc.pages[picked.page];
  const geom = tab.zoom.pagesGeom()[picked.page];
  if (!page || page.srcIndex === null || !geom) return false;
  if (geom.rotation !== 0) {
    flashStatus('Regions can be marked on unrotated pages only');
    return false;
  }
  ensureLoaded(path);
  batch(() => {
    setEditing(null);
    setDraft({
      path,
      mode: 'comment',
      anchor: {
        page: page.srcIndex!,
        quads: null,
        rect: rectFromBox(picked.box, page.heightPt),
        quote: '',
      },
      kind: 'exposition',
      quickmark: null,
    });
  });
  return true;
}

/** Start a comment at a given place with given words (a suggestion). */
export function beginCommentAt(
  path: string,
  anchor: Anchor,
  start: { kind: CommentKind; body: string; quickmark: string | null }
): void {
  ensureLoaded(path);
  batch(() => {
    setEditing(null);
    setDraft({ path, mode: 'comment', anchor, ...start });
  });
}

export interface DraftResult {
  body: string;
  kind: CommentKind;
  quickmark: string | null;
  /** For a typo fix: the words to put in their place ('' deletes them). */
  replacement: string | null;
}

/** Keep the comment being written. Nothing to say and nothing to fix
 * cancels it instead. Returns the new comment's id. */
export function commitDraft(result: DraftResult): string | null {
  const d = draft();
  if (!d) return null;
  const body = result.body.trim();
  if (d.mode === 'comment' && !body && !result.quickmark) {
    setDraft(null);
    return null;
  }
  const comment: MarkupComment = {
    id: newCommentId(),
    kind: result.kind,
    anchor: d.anchor,
    body,
    replacement: d.mode === 'typo' ? (result.replacement ?? '').trim() : null,
    quickmark: result.quickmark,
    created: Date.now(),
  };
  if (!addComment(d.path, comment)) {
    flashStatus('The review is still loading; try again in a moment');
    return null;
  }
  setDraft(null);
  return comment.id;
}

export function cancelDraft(): void {
  setDraft(null);
}

/* ---- going to a comment ---- */

/** Scroll the page to a comment and mark its words for a moment. */
export function revealComment(comment: MarkupComment): void {
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded) return;
  const index = layoutIndexOf(doc, comment.anchor.page);
  if (index === null) return;
  const height = doc.pages[index]?.heightPt ?? 0;
  // A little of what comes before, so the words are not flush at the top.
  const y = Math.min(height, anchorStart(comment.anchor).y + 48);
  jumpToAnchor(tab.viewport, doc, comment.anchor.page, y);
  revealPlace(doc.docId, index, revealRects(comment.anchor));
}

/* ---- what labels are worked out from ---- */

const [statements, setStatements] = createStore<Record<number, StatementLike[]>>({});
const [proofs, setProofs] = createStore<Record<string, ProofStart[]>>({});
/** What has been asked for, and when it will have arrived. */
const asked = new Map<string, Promise<void>>();

function ensureStatements(docId: number): Promise<void> {
  const key = `s${docId}`;
  let pending = asked.get(key);
  if (!pending) {
    pending = docStatements(docId).then((list) =>
      setStatements(
        docId,
        list.map((s) => ({
          kind: s.kind,
          word: s.word,
          number: s.number,
          page: s.page,
          region: s.region,
        }))
      )
    );
    asked.set(key, pending);
  }
  return pending;
}

function ensureProofs(docId: number, src: number): Promise<void> {
  const key = `${docId}:${src}`;
  let pending = asked.get(key);
  if (!pending) {
    pending = engine
      .getTextLayout(docId, src)
      .then((layout) => setProofs(key, proofStarts(src, layout.runs)))
      .catch(() => setProofs(key, []));
    asked.set(key, pending);
  }
  return pending;
}

/** Wait for everything the labels of these comments are worked out from:
 * what an export does before it writes them down. */
export async function settleLabelSources(comments: readonly MarkupComment[]): Promise<void> {
  const doc = activeTab()?.documentStore.state;
  if (!doc?.loaded) return;
  const waits: Promise<void>[] = [ensureStatements(doc.docId)];
  for (const p of proofPages(comments)) waits.push(ensureProofs(doc.docId, p));
  await Promise.all(waits);
}

function proofPages(comments: readonly MarkupComment[]): number[] {
  const pages = new Set<number>();
  for (const c of comments) {
    for (let p = Math.max(0, c.anchor.page - PROOF_LOOKBACK); p <= c.anchor.page; p++) pages.add(p);
  }
  return [...pages].sort((a, b) => a - b);
}

/** How far back a proof is looked for from a comment's page. */
const PROOF_LOOKBACK = 3;

/** The outline, statements and proof openings of the active document, for
 * the pages that have comments. Fetches what it lacks (once each) and
 * answers with what has arrived so far. */
export function labelContext(comments: readonly MarkupComment[]): LabelContext {
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded) return EMPTY_CONTEXT;
  const headings: Heading[] = outlineAsEntries(tab.structureStore.state.outline).map((e) => ({
    depth: e.depth,
    label: e.label,
    page: e.page,
    y: e.y,
  }));
  if (comments.length === 0) return { headings, statements: [], proofs: [] };
  void ensureStatements(doc.docId);
  const found: ProofStart[] = [];
  for (const p of proofPages(comments)) {
    void ensureProofs(doc.docId, p);
    found.push(...(proofs[`${doc.docId}:${p}`] ?? []));
  }
  return { headings, statements: statements[doc.docId] ?? [], proofs: found };
}

/* ---- the library, for citations ---- */

const [citeIndex, setCiteIndex] = createSignal<CiteIndex>(EMPTY_INDEX);
export { citeIndex };

let libraryAsked = false;

function loadLibrary(): void {
  void engine
    .libraryPapers({})
    .then((list) => setCiteIndex(buildCiteIndex(list.rows)))
    .catch(() => undefined);
}

/* ---- following the active document ---- */

let tracking = false;

/** Load each document's review as it comes on screen while reviewing. */
function track(): void {
  if (tracking) return;
  tracking = true;
  createRoot(() => {
    createEffect(() => {
      if (!reviewing()) return;
      const path = activePath();
      if (path) ensureLoaded(path);
      if (!libraryAsked) {
        libraryAsked = true;
        loadLibrary();
        void engine.onLibraryChanged(loadLibrary).catch(() => undefined);
      }
    });
    // A comment being written belongs to its document: leaving it drops it.
    createEffect(() => {
      const path = activePath();
      const d = untrack(draft);
      if (d && d.path !== path) setDraft(null);
    });
  });
}

/** Statements of a document, for tests. */
export function setLabelSourcesForTest(
  docId: number,
  list: StatementLike[],
  proofs: Record<number, ProofStart[]> = {}
): void {
  asked.set(`s${docId}`, Promise.resolve());
  setStatements(docId, list);
  for (const [page, starts] of Object.entries(proofs)) {
    asked.set(`${docId}:${page}`, Promise.resolve());
    setProofs(`${docId}:${page}`, starts);
  }
}

export function setCiteIndexForTest(index: CiteIndex): void {
  setCiteIndex(index);
}

/** A review's comments numbered in reading order and labelled, against the
 * active document. */
export function entriesOf(doc: MarkupDoc): Entry[] {
  const ctx = labelContext(doc.comments);
  return buildEntries(doc.comments, (anchor) => labelFor(anchor, ctx));
}

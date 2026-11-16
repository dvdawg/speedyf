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
import {
  selectedGeometry,
  selectedPieces,
  selectedText,
  settleSelectionNow,
} from '../../stores/selectionStore';
import { planQuote } from '../../lib/text/quoteParts';
import { readQuote } from './quoteReading';
import {
  convertEquation,
  registerSnapshot,
  extensionUsable,
  snapshotSelection,
  type ConvertMode,
} from '../extensions/extensionStore';
import { equationToken } from '../extensions/editorHost';
import { equationBlocksFor } from '../extensions/equationBlocks';
import { piecesUnderQuads } from '../../lib/text/piecesUnder';
import { quadBox } from './quoteBackfill';
import { plausibleQuoteTex, texParses } from './quoteTexCheck';
import { closeReaderPane } from '../extensions/surfaces';
import { docStatements } from '../extensions/paperDocs';
import { pickRegion } from '../extensions/regionCapture';
import { jumpToAnchor, layoutIndexOf } from '../outline/jumpToAnchor';
import { outlineAsEntries } from '../../lib/structure/context';
import { anchorStart, quadsFromRects, rectFromViewBox, revealRects } from './geometry';
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
  // Taken while the selection is still the one the text came from: what the
  // exact source of the passage is looked for by.
  const token = snapshotSelection(selectedText());
  // The quote's parts: prose, formulas, displays (`quoteParts`).
  const pieces = selectedPieces();
  const plan = mode === 'comment' && pieces.length > 0 ? planQuote(pieces) : null;
  document.getSelection()?.removeAllRanges();
  const quote = plan?.plain.replace(/\s+/g, ' ').trim() || text;
  const anchor: Anchor = { page: page.srcIndex!, quads, rect: null, quote };
  batch(() => {
    setEditing(null);
    setDraft({
      path,
      mode,
      anchor,
      kind: mode === 'typo' ? 'typo' : 'comment',
      quickmark: mode === 'typo' ? 'typo' : null,
    });
  });
  if (plan && (plan.inline.length > 0 || plan.displays.length > 0)) {
    const docId = doc.docId;
    setReading(anchor, true);
    void readQuote(
      plan,
      { docId, pageHeight: (p) => tab.documentStore.state.pages[p]?.heightPt },
      token,
      (tex) => {
        setReading(anchor, false);
        if (tex) setQuoteTex(path, anchor, tex);
      }
    ).finally(() => setReading(anchor, false));
  }
  return true;
}

/** Quotes whose math is being read: their cards show the plain text,
 * marked as still coming. */
const [reading, setReadingSet] = createSignal<ReadonlySet<Anchor>>(new Set());

function setReading(anchor: Anchor, on: boolean): void {
  const now = untrack(reading);
  if (now.has(anchor) === on) return;
  const next = new Set(now);
  if (on) next.add(anchor);
  else next.delete(anchor);
  setReadingSet(next);
}

/** Whether the math of `anchor`'s quote is still being read. */
export function quoteReading(anchor: Anchor): boolean {
  for (const a of reading()) if (a === anchor || a === unwrap(anchor)) return true;
  return false;
}

/** Start a comment on a display equation (its button on the page): the
 * equation's box is the anchor, "Equation (3)" the quote, and its LaTeX,
 * when a converter reads it, what the drawer shows. `page` is the reader's
 * page, `bbox` the equation's box from that page's top-left. */
export function beginEquationComment(target: {
  page: number;
  bbox: [number, number, number, number];
  number: string | null;
}): boolean {
  if (!reviewing()) return false;
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  const path = activePath();
  if (!tab || !doc || !path) return false;
  const page = doc.pages[target.page];
  if (!page || page.srcIndex === null) return false;
  const quads = quadsFromRects([target.bbox], page.heightPt);
  if (quads.length === 0) return false;
  ensureLoaded(path);
  const anchor: Anchor = {
    page: page.srcIndex,
    quads,
    rect: null,
    quote: target.number ? `Equation (${target.number})` : 'Equation',
  };
  batch(() => {
    setEditing(null);
    setDraft({ path, mode: 'comment', anchor, kind: 'comment', quickmark: null });
  });
  const ref = { docId: doc.docId, page: target.page, bbox: target.bbox, number: target.number };
  const token = equationToken(ref);
  setReading(anchor, true);
  void readQuoteMath(path, anchor, '', async (m) => {
    const answer = await convertEquation(token, ref, m);
    if (m === 'local') setReading(anchor, false);
    let latex = answer?.latex.replace(/\s+/g, ' ').trim() ?? '';
    if (latex && target.number && !latex.includes('\\tag')) latex += ` \\tag{${target.number}}`;
    return latex ? `$$ ${latex} $$` : null;
  }).finally(() => setReading(anchor, false));
  return true;
}

/** Read a quote's math as LaTeX, into its anchor: from the page itself at
 * once, then, when that found math, quietly from an exact source if the
 * converter has one (which goes online only when its extension is
 * enabled, the user's consent to it). Nothing happens without a converter,
 * and a reading is kept only when it can be trusted (`quoteTexCheck`). */
async function readQuoteMath(
  path: string,
  anchor: Anchor,
  /** the words the page itself gives, which a reading must agree with
   * (empty for an equation on its own) */
  plain: string,
  read: (mode: ConvertMode) => Promise<string | null>
): Promise<void> {
  const trusted = async (tex: string | null) =>
    tex !== null && plausibleQuoteTex(tex, plain) && (await texParses(tex));
  const local = await read('local').catch(() => null);
  if (!local) return;
  if (await trusted(local)) setQuoteTex(path, anchor, local);
  const exact = await read('exact').catch(() => null);
  if (exact && exact !== local && (await trusted(exact))) setQuoteTex(path, anchor, exact);
}

/** Put a quote's LaTeX on its anchor: the draft's, while it is being
 * written, else the kept comment's (the draft's anchor is the comment's). */
function setQuoteTex(path: string, anchor: Anchor, tex: string): void {
  const d = untrack(draft);
  if (d && d.anchor === anchor) {
    anchor.quoteTex = tex;
    setDraft({ ...d });
    return;
  }
  const entry = untrack(() => reviews[path]);
  if (!entry?.doc.comments.some((c) => unwrap(c.anchor) === anchor)) return;
  change(path, (doc) => {
    for (const c of doc.comments) if (unwrap(c.anchor) === anchor) c.anchor.quoteTex = tex;
  });
}

/** Change a kept comment's anchor, found by identity. */
function patchAnchor(
  path: string,
  anchor: Anchor,
  patch: { [K in keyof Anchor]?: Anchor[K] | undefined }
): void {
  const entry = untrack(() => reviews[path]);
  if (!entry?.doc.comments.some((c) => unwrap(c.anchor) === anchor)) return;
  change(path, (doc) => {
    for (const c of doc.comments) {
      if (unwrap(c.anchor) !== anchor) continue;
      for (const [key, value] of Object.entries(patch)) {
        if (value === undefined) delete (c.anchor as unknown as Record<string, unknown>)[key];
        else (c.anchor as unknown as Record<string, unknown>)[key] = value;
      }
    }
  });
}

/** Whether a kept comment quotes words whose math can be read: a text
 * anchor, not a typo fix (whose words are struck as printed). */
export function quotesText(comment: MarkupComment): boolean {
  return (
    comment.kind !== 'typo' &&
    !!comment.anchor.quads?.length &&
    !comment.anchor.rect &&
    comment.anchor.quote.trim() !== ''
  );
}

/** Read a kept comment's quote again from the page under its quads: its
 * plain text rebuilt clean (margin line numbers gone, lines joined, in
 * reading order) and its math read as a new comment's is. For quotes kept
 * before their math could be read, and for one whose reading is wrong.
 * Only while its document is the one on screen. */
export async function rereadQuote(path: string, comment: MarkupComment): Promise<void> {
  const tab = activeTab();
  const doc = tab?.documentStore.state;
  if (!tab || !doc?.loaded || doc.path !== path || !quotesText(comment)) return;
  const anchor = unwrap(comment.anchor);
  const pageIndex = doc.pages.findIndex((p) => p.srcIndex === anchor.page);
  const page = doc.pages[pageIndex];
  if (!page || pageIndex < 0) return;
  const height = page.heightPt;
  const docId = doc.docId;
  setReading(anchor, true);
  try {
    const [layout, blocks] = await Promise.all([
      engine.getTextLayout(docId, anchor.page),
      equationBlocksFor(docId, anchor.page, 0),
    ]);
    const rects = anchor.quads!.map(quadBox);
    const pieces = piecesUnderQuads(
      layout.runs,
      rects,
      blocks.map(({ bbox: [x, y, w, h], number }) => ({ x, y: height - y - h, w, h, number })),
      pageIndex,
      anchor.quote
    );
    if (pieces.length === 0) return;
    const plan = planQuote(pieces);
    const plain = plan.plain.replace(/\s+/g, ' ').trim();
    if (plain && plain !== anchor.quote) patchAnchor(path, anchor, { quote: plain });
    if (plan.inline.length === 0 && plan.displays.length === 0) {
      if (anchor.quoteTex) patchAnchor(path, anchor, { quoteTex: undefined });
      return;
    }
    const whole = registerSnapshot({
      docId,
      page: pageIndex,
      rects: rects.map(([x, y, w, h]) => [x, height - y - h, w, h]),
      text: plain,
    });
    let first = true;
    await readQuote(plan, { docId, pageHeight: (p) => doc.pages[p]?.heightPt }, whole, (tex) => {
      setReading(anchor, false);
      // the first answer replaces whatever reading there was; a later one
      // only ever improves it
      if (tex) patchAnchor(path, anchor, { quoteTex: tex });
      else if (first) patchAnchor(path, anchor, { quoteTex: undefined });
      first = false;
    });
  } catch {
    // the page could not be read: the quote stands as it was
  } finally {
    setReading(anchor, false);
  }
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
  ensureLoaded(path);
  batch(() => {
    setEditing(null);
    setDraft({
      path,
      mode: 'comment',
      anchor: {
        page: page.srcIndex!,
        quads: null,
        // The box was dragged on the page as turned on screen; the anchor
        // is in the page's own space, like a text selection's.
        rect: rectFromViewBox(picked.box, geom),
        quote: '',
      },
      kind: 'comment',
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

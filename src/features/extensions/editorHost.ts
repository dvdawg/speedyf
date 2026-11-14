/** The window's side of extension editors: the note sessions, wired to the
 * core; what each extension told its editors about citations; the editors
 * on screen, for `editor.insert`, `editor.replace` and `editor.reveal` to
 * reach; the placeholders and marks extensions put in notes while a send
 * is on its way (`editor.insert { pending }`, `editor.resolve`,
 * `editor.mark`, `editor.replaceMarked`), told to every window; and the
 * item lines extensions contribute to the Projects detail column. */
import { createSignal } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import {
  NoteSessions,
  keyString,
  type DecorationSnapshot,
  type DocEvent,
  type NoteKey,
  type NoteSession,
  type NoteTransport,
  type PendingShape,
} from './editor/noteSession';
import type { Op } from './editor/ops';
import { identifierKey, normalizeArxiv, normalizeDoi, type Identifier } from './editor/identifiers';

/** The event a window's placeholders and marks travel by to the others. */
const DECOR_EVENT = 'project-file:decorations';

/* ---- sessions ---- */

const tauriTransport: NoteTransport = {
  open: (key) =>
    invoke<{ text: string; rev: number }>('project_file_open', {
      extensionId: key.extensionId,
      projectId: key.projectId,
      file: key.file,
    }),
  edit: (key, baseRev, op, origin) =>
    invoke<number>('project_file_edit', {
      extensionId: key.extensionId,
      projectId: key.projectId,
      file: key.file,
      baseRev,
      from: op.from,
      to: op.to,
      insert: op.insert,
      origin,
    }),
  close: (key) =>
    invoke<void>('project_file_close', {
      extensionId: key.extensionId,
      projectId: key.projectId,
      file: key.file,
    }),
};

/** This window, as the origin of its edits. */
const ORIGIN = `w-${Math.random().toString(36).slice(2)}-${Date.now().toString(36)}`;

let sessions: NoteSessions | null = null;
let listening: Promise<void> | null = null;
const [docEventSeq, setDocEventSeq] = createSignal(0);

/** Bumped on every note event in any window: what the item lines watch to
 * recount. */
export { docEventSeq };

/** The window's note sessions. Started on first use. */
export function noteSessions(): NoteSessions {
  if (!sessions) sessions = new NoteSessions(tauriTransport, ORIGIN);
  listening ??= Promise.all([
    listen<DocEvent>('project-file:event', (event) => {
      sessions?.dispatch(event.payload);
      setDocEventSeq((n) => n + 1);
    }),
    listen<DecorationSnapshot>(DECOR_EVENT, (event) => {
      sessions?.get(event.payload.key)?.receiveDecorations(event.payload);
    }),
  ]).then(
    () => undefined,
    () => {
      listening = null;
    }
  );
  return sessions;
}

/** Resolves once this window hears note events: an edit made before then
 * would never hear itself confirmed. */
async function eventsHeard(): Promise<void> {
  noteSessions();
  await listening;
}

/** For tests: a window with its own transport, and where its decorations
 * go instead of to other windows. */
export function useNoteSessionsForTest(
  transport: NoteTransport,
  origin: string,
  send?: (snapshot: DecorationSnapshot) => void
): NoteSessions {
  sessions = new NoteSessions(transport, origin);
  listening = Promise.resolve();
  if (send) sendDecorations = send;
  return sessions;
}

/** Send whatever every note has unsent, then ask the core to save: the
 * window is closing. */
export async function flushNotes(): Promise<void> {
  await sessions?.flushAll();
  await invoke('project_files_flush').catch(() => {});
}

/* ---- citations ---- */

/** One cite key an extension told its editor about. */
export interface CitationEntry {
  key: string;
  /** How a citation of it reads: "[AS22]". A bracketed label takes a
   * locator inside its brackets: "[AS22, p. 12]". */
  label: string;
  title: string;
  /** "Attias, Smith - 2022": the picker's second line. */
  detail: string;
  /** Offered in the `@` picker (an item of the note's project). */
  pick: boolean;
  /** Other keys that mean this item (its key history). */
  aliases: string[];
  /** The item's DOI and arXiv id, when it has them: a pasted identifier
   * becomes a citation of the entry holding it. */
  doi?: string | null;
  arxiv?: string | null;
}

export interface CitationIndex {
  entries: CitationEntry[];
  byKey: Map<string, CitationEntry>;
  /** By `identifierKey`: `doi:10.1/x`, `arxiv:2401.01234`. */
  byId: Map<string, CitationEntry>;
}

const EMPTY: CitationIndex = { entries: [], byKey: new Map(), byId: new Map() };
const [citationStore, setCitationStore] = createStore<Record<string, CitationEntry[]>>({});

const scope = (extensionId: string, projectId: string) => `${extensionId}\u0000${projectId}`;

export function setCitations(extensionId: string, projectId: string, entries: CitationEntry[]) {
  setCitationStore(scope(extensionId, projectId), reconcile(entries));
}

const indexCache = new Map<string, { entries: CitationEntry[]; index: CitationIndex }>();

/** The index an editor resolves `@key` with: a key first, then any alias,
 * the first entry holding it winning. */
export function citationIndex(extensionId: string, projectId: string): CitationIndex {
  const id = scope(extensionId, projectId);
  const entries = citationStore[id];
  if (!entries) return EMPTY;
  const cached = indexCache.get(id);
  if (cached && cached.entries === entries) return cached.index;
  const byKey = new Map<string, CitationEntry>();
  for (const entry of entries) if (!byKey.has(entry.key)) byKey.set(entry.key, entry);
  for (const entry of entries) {
    for (const alias of entry.aliases) if (!byKey.has(alias)) byKey.set(alias, entry);
  }
  // The project's own items first: a paste names the one it offers.
  const byId = new Map<string, CitationEntry>();
  for (const entry of [...entries.filter((e) => e.pick), ...entries.filter((e) => !e.pick)]) {
    const doi = entry.doi ? normalizeDoi(entry.doi) : null;
    const arxiv = entry.arxiv ? normalizeArxiv(entry.arxiv) : null;
    for (const id of [
      doi ? identifierKey({ kind: 'doi', id: doi }) : null,
      arxiv ? identifierKey({ kind: 'arxiv', id: arxiv }) : null,
    ]) {
      if (id && !byId.has(id)) byId.set(id, entry);
    }
  }
  const index = { entries: [...entries], byKey, byId };
  indexCache.set(id, { entries, index });
  return index;
}

/** The cite key of the entry holding identifier `id`, or null. */
export function keyForIdentifier(index: CitationIndex, id: Identifier): string | null {
  return index.byId.get(identifierKey(id))?.key ?? null;
}

/** Read `editor.citations` entries; a malformed one is dropped. */
export function readCitationEntries(raw: unknown): CitationEntry[] {
  if (!Array.isArray(raw)) throw new Error('editor.citations needs an "entries" array');
  const out: CitationEntry[] = [];
  const str = (v: unknown, max: number) => (typeof v === 'string' ? v.slice(0, max) : '');
  for (const value of raw.slice(0, 20_000)) {
    if (typeof value !== 'object' || value === null) continue;
    const r = value as Record<string, unknown>;
    const key = str(r.key, 200).trim();
    if (!key) continue;
    out.push({
      key,
      label: str(r.label, 200) || key,
      title: str(r.title, 2000),
      detail: str(r.detail, 2000),
      pick: r.pick === true,
      aliases: Array.isArray(r.aliases)
        ? r.aliases.filter((a): a is string => typeof a === 'string' && a.length <= 200)
        : [],
      doi: typeof r.doi === 'string' ? normalizeDoi(r.doi.slice(0, 500)) : null,
      arxiv: typeof r.arxiv === 'string' ? normalizeArxiv(r.arxiv.slice(0, 200)) : null,
    });
  }
  return out;
}

/* ---- editors on screen ---- */

export interface EditorHandle {
  /** Insert at the caret when there is one in this editor: as a paragraph
   * of its own after the block being edited. Returns where the inserted
   * text starts, or null with no caret here. */
  insertAtCaret(text: string): number | null;
  /** Where a paragraph inserted at the caret would go (the end of the block
   * being edited), or null with no caret here. */
  caretBlockEnd(): number | null;
  /** Scroll to `offset`, or to decoration `decor` when it is drawn. */
  reveal(offset: number, decor?: string): void;
}

const editors = new Map<string, Set<EditorHandle>>();
const pendingReveals = new Map<string, { offset: number; decor: string | undefined }>();

/** An editor on screen for `key`; returns the unregister. */
export function registerEditor(key: NoteKey, handle: EditorHandle): () => void {
  const id = keyString(key);
  let set = editors.get(id);
  if (!set) editors.set(id, (set = new Set()));
  set.add(handle);
  const wanted = pendingReveals.get(id);
  if (wanted !== undefined) {
    pendingReveals.delete(id);
    queueMicrotask(() => handle.reveal(wanted.offset, wanted.decor));
  }
  return () => {
    set.delete(handle);
    if (set.size === 0) editors.delete(id);
  };
}

/** Whether an editor of `key` is on screen in this window. */
export function noteShown(key: NoteKey): boolean {
  return (editors.get(keyString(key))?.size ?? 0) > 0;
}

/** Scroll an editor of `key` to `offset` (or decoration `decor`), now or
 * when one mounts. */
export function revealInEditor(key: NoteKey, offset: number, decor?: string): void {
  const set = editors.get(keyString(key));
  const handle = set ? [...set][set.size - 1] : undefined;
  if (handle) handle.reveal(offset, decor);
  else pendingReveals.set(keyString(key), { offset, decor });
}

/** Text inserted at `at` as a block of its own: after a blank line (unless
 * at the start), and before one (or the end of the note, ending in a
 * newline). */
export function blockInsertOp(text: string, at: number, insert: string): Op {
  const body = insert.replace(/\n+$/, '');
  const before = text.slice(0, at);
  const after = text.slice(at);
  let lead = '';
  if (before.length > 0)
    lead = before.endsWith('\n\n') ? '' : before.endsWith('\n') ? '\n' : '\n\n';
  let trail = '\n\n';
  if (after === '' || (after.startsWith('\n') && !after.startsWith('\n\n'))) trail = '\n';
  else if (after.startsWith('\n\n')) trail = '';
  return { from: at, to: at, insert: `${lead}${body}${trail}` };
}

/** Text appended to a note: after a blank line, ending in a newline. */
export function appendOp(text: string, insert: string): Op {
  return blockInsertOp(text, text.length, insert);
}

/* ---- placeholders and marks ---- */

/** An extension's edit: no view made it, so every view redraws for it. */
const OUTSIDE = { outside: true };

/** How long a placeholder or mark lives without being resolved, cleared or
 * renewed. */
export const DECORATION_TTL_MS = 30_000;

/** Handles this window gave out, each holding its note's session open. */
interface Held {
  key: NoteKey;
  session: NoteSession;
  owner: string;
}
const held = new Map<string, Held>();
let nextHandle = 1;

/** How this window tells the others about its decorations. */
let sendDecorations: (snapshot: DecorationSnapshot) => void = (snapshot) => {
  try {
    void emit(DECOR_EVENT, snapshot).catch(() => {});
  } catch {
    /* no other windows to tell */
  }
};

function broadcast(session: NoteSession): void {
  sendDecorations(session.snapshot());
}

/** Join `key`'s session for as long as a handle lives. */
async function hold(key: NoteKey): Promise<NoteSession> {
  await eventsHeard();
  const all = noteSessions();
  const session = all.acquire(key);
  await session.ready;
  if (session.status() !== 'ready') {
    all.release(session);
    throw new Error(session.reason() ?? 'the note is not open');
  }
  session.onExpire = (d) => {
    if (held.delete(d.id)) all.release(session);
    broadcast(session);
  };
  return session;
}

function letGo(handle: string): void {
  const h = held.get(handle);
  if (!h) return;
  held.delete(handle);
  noteSessions().release(h.session);
}

/** A handle of `owner`'s, or undefined: another extension's is not
 * reachable. */
function heldBy(owner: string, handle: unknown): Held | undefined {
  if (typeof handle !== 'string') return undefined;
  const h = held.get(handle);
  return h && h.owner === owner ? h : undefined;
}

/** Track `[from, to)` under a new handle, unlabelled. */
function track(session: NoteSession, key: NoteKey, owner: string, from: number, to: number) {
  const id = `${session.origin}:${nextHandle++}`;
  const now = Date.now();
  session.putDecoration({
    type: 'mark',
    id,
    owner,
    origin: session.origin,
    created: now,
    expires: now + DECORATION_TTL_MS,
    from,
    to,
    label: null,
  });
  held.set(id, { key, session, owner });
  return id;
}

async function confirmed(session: NoteSession): Promise<void> {
  await session.flush();
  // Confirmed by its broadcast; never waited on forever.
  await Promise.race([session.settled(), new Promise((r) => setTimeout(r, 5_000))]);
}

/** Insert into a note: at the caret of an editor showing it in this window
 * when `at` is 'cursor' and there is one, else at the end. Returns where
 * the inserted text starts and a handle tracking it (for `markInNote`). */
export async function insertIntoNote(
  key: NoteKey,
  insert: string,
  at: 'cursor' | 'end',
  owner = ''
): Promise<{ offset: number; handle: string }> {
  const session = await hold(key);
  const body = insert.replace(/\n+$/, '');
  let start: number | null = null;
  if (at === 'cursor') {
    for (const handle of editors.get(keyString(key)) ?? []) {
      start = handle.insertAtCaret(insert);
      if (start !== null) break;
    }
  }
  if (start === null) {
    const text = session.text();
    const op = blockInsertOp(text, text.length, insert);
    session.edit(op, text.length, op.from + op.insert.length, OUTSIDE);
    start = op.from + op.insert.indexOf(body);
  }
  const handle = track(session, key, owner, start, start + body.length);
  broadcast(session);
  await confirmed(session);
  return { offset: start, handle };
}

/** A placeholder in a note for something on its way: nothing goes into the
 * text, only a place that moves with edits, drawn by every view. At the
 * caret's block when `at` is 'cursor' and an editor here has one, else at
 * the end. */
export async function insertPending(
  key: NoteKey,
  pending: PendingShape,
  at: 'cursor' | 'end',
  owner: string
): Promise<{ offset: number; handle: string }> {
  const session = await hold(key);
  let anchor = session.text().length;
  if (at === 'cursor') {
    for (const handle of editors.get(keyString(key)) ?? []) {
      const end = handle.caretBlockEnd();
      if (end !== null) {
        anchor = end;
        break;
      }
    }
  }
  const id = `${session.origin}:${nextHandle++}`;
  const now = Date.now();
  session.putDecoration({
    type: 'pending',
    id,
    owner,
    origin: session.origin,
    created: now,
    expires: now + DECORATION_TTL_MS,
    at: anchor,
    pending,
  });
  held.set(id, { key, session, owner });
  broadcast(session);
  return { offset: anchor, handle: id };
}

/** Put `text` where placeholder `handle` is, as one undoable edit, and
 * keep tracking it under the same handle. Null when the placeholder is
 * gone (expired or cancelled). */
export async function resolvePending(
  owner: string,
  handle: unknown,
  text: string
): Promise<{ offset: number } | null> {
  const h = heldBy(owner, handle);
  if (!h) return null;
  const { session } = h;
  const d = session.removeDecoration(handle as string);
  if (!d || d.type !== 'pending') {
    letGo(handle as string);
    broadcast(session);
    return null;
  }
  const now = session.text();
  const op = blockInsertOp(now, Math.min(d.at, now.length), text);
  const body = text.replace(/\n+$/, '');
  const start = op.from + op.insert.indexOf(body);
  session.edit(op, op.from, op.from + op.insert.length, OUTSIDE);
  const at = Date.now();
  session.putDecoration({
    type: 'mark',
    id: d.id,
    owner,
    origin: session.origin,
    created: at,
    expires: at + DECORATION_TTL_MS,
    from: start,
    to: start + body.length,
    label: null,
  });
  broadcast(session);
  await confirmed(session);
  return { offset: start };
}

/** Remove a placeholder or mark, and stop tracking it. */
export function cancelHandle(owner: string, handle: unknown): boolean {
  const h = heldBy(owner, handle);
  if (!h) return false;
  const gone = h.session.removeDecoration(handle as string);
  letGo(handle as string);
  broadcast(h.session);
  return gone !== undefined;
}

/** Show a status line under the text `handle` tracks (renewing its
 * lifetime). False when the text is gone or edited, or still pending. */
export function markHandle(owner: string, handle: unknown, label: string): boolean {
  const h = heldBy(owner, handle);
  const d = h?.session.decoration(handle as string);
  if (!h || !d || d.type !== 'mark') return false;
  const now = Date.now();
  h.session.putDecoration({ ...d, label, created: now, expires: now + DECORATION_TTL_MS });
  broadcast(h.session);
  return true;
}

/** Change the line under a mark, keeping its lifetime: a converter
 * saying which source it is trying. */
export function relabelHandle(owner: string, handle: string, label: string): void {
  const h = heldBy(owner, handle);
  const d = h?.session.decoration(handle);
  if (!h || !d || d.type !== 'mark' || d.label === null) return;
  h.session.putDecoration({ ...d, label });
  broadcast(h.session);
}

/** Track `[from, to)` of a note with a status line under it. */
export async function markRange(
  key: NoteKey,
  owner: string,
  from: number,
  to: number,
  label: string
): Promise<string> {
  const session = await hold(key);
  const length = session.text().length;
  if (from < 0 || to <= from || to > length) {
    noteSessions().release(session);
    throw new Error(`expected a range within the note (0 to ${length})`);
  }
  const handle = track(session, key, owner, from, to);
  markHandle(owner, handle, label);
  return handle;
}

/** Replace exactly what `handle` tracks with `text`, as one undoable
 * edit, when nobody has touched it since; the handle is used up. False
 * (and nothing touched) when it was edited, deleted or has expired. */
export async function replaceMarked(
  owner: string,
  handle: unknown,
  text: string
): Promise<boolean> {
  const h = heldBy(owner, handle);
  if (!h) return false;
  const { session } = h;
  const d = session.removeDecoration(handle as string);
  if (!d || d.type !== 'mark') {
    letGo(handle as string);
    broadcast(session);
    return false;
  }
  if (session.text().slice(d.from, d.to) !== text) {
    session.edit({ from: d.from, to: d.to, insert: text }, d.to, d.from + text.length, OUTSIDE);
  }
  broadcast(session);
  try {
    await confirmed(session);
  } finally {
    letGo(handle as string);
  }
  return true;
}

/** Where a handle's placeholder or text is now, for `editor.reveal`. */
export function handlePlace(
  owner: string,
  handle: unknown
): { key: NoteKey; offset: number } | null {
  const h = heldBy(owner, handle);
  const d = h?.session.decoration(handle as string);
  if (!h || !d) return null;
  return { key: h.key, offset: d.type === 'pending' ? d.at : d.from };
}

/** Everything an extension has pending or marked, gone: it was disabled
 * or stopped. */
export function clearDecorationsOf(owner: string): void {
  const touched = new Set<NoteSession>();
  for (const [handle, h] of [...held]) {
    if (h.owner !== owner) continue;
    h.session.removeDecoration(handle);
    letGo(handle);
    touched.add(h.session);
  }
  for (const session of touched) broadcast(session);
}

/** Replace the first exact occurrence of `find` in a note with `replace`,
 * as one edit through the note's session: undoable in this window, ordered
 * by the core with every other window's edits. False when `find` is not
 * there (the user changed it meanwhile): nothing is touched. */
export async function replaceInNote(key: NoteKey, find: string, replace: string): Promise<boolean> {
  await eventsHeard();
  const all = noteSessions();
  const session: NoteSession = all.acquire(key);
  try {
    await session.ready;
    if (session.status() !== 'ready') throw new Error(session.reason() ?? 'the note is not open');
    const text = session.text();
    const at = text.indexOf(find);
    if (at < 0) return false;
    if (find === replace) return true;
    session.edit(
      { from: at, to: at + find.length, insert: replace },
      at + find.length,
      at + replace.length,
      OUTSIDE
    );
    await confirmed(session);
    return true;
  } finally {
    all.release(session);
  }
}

/* ---- item lines ---- */

export interface ItemLine {
  extensionId: string;
  text: string;
  /** The extension's own command, invoked with `value` on a click. */
  command: string | null;
  value: unknown;
}

const [itemLineStore, setItemLineStore] = createStore<Record<string, Record<string, ItemLine>>>({});

export function setItemLine(itemId: string, line: ItemLine | null, extensionId: string): void {
  if (!itemLineStore[itemId]) setItemLineStore(itemId, {});
  setItemLineStore(itemId, extensionId, line ?? undefined!);
}

export function itemLines(itemId: string, enabled: (extensionId: string) => boolean): ItemLine[] {
  return Object.values(itemLineStore[itemId] ?? {}).filter(
    (line): line is ItemLine => !!line && enabled(line.extensionId)
  );
}

/* ---- equations the user clicked ---- */

export interface EquationRef {
  docId: number;
  page: number;
  bbox: [number, number, number, number];
  number: string | null;
}

const equationTokens = new Map<string, EquationRef>();
let nextEquation = 1;

/** An opaque token for an equation the user clicked, which an extension
 * hands back to `equations.convert` and `equations.snapshot`. */
export function equationToken(ref: EquationRef): string {
  const token = `eq-${nextEquation++}`;
  equationTokens.set(token, ref);
  if (equationTokens.size > 200) {
    const first = equationTokens.keys().next().value;
    if (first !== undefined) equationTokens.delete(first);
  }
  return token;
}

export function equationFor(token: unknown): EquationRef {
  const ref = typeof token === 'string' ? equationTokens.get(token) : undefined;
  if (!ref) throw new Error('expected the "equation" token an equation action was invoked with');
  return ref;
}

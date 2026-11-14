/** The window's side of extension editors: the note sessions, wired to the
 * core; what each extension told its editors about citations; the editors
 * on screen, for `editor.insert` and `editor.reveal` to reach; and the item
 * lines extensions contribute to the Projects detail column. */
import { createSignal } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  NoteSessions,
  keyString,
  type DocEvent,
  type NoteKey,
  type NoteSession,
  type NoteTransport,
} from './editor/noteSession';
import { applyOp, type Op } from './editor/ops';

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
  listening ??= listen<DocEvent>('project-file:event', (event) => {
    sessions?.dispatch(event.payload);
    setDocEventSeq((n) => n + 1);
  }).then(
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

/** For tests: a window with its own transport. */
export function useNoteSessionsForTest(transport: NoteTransport, origin: string): NoteSessions {
  sessions = new NoteSessions(transport, origin);
  listening = Promise.resolve();
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
  /** How a citation of it reads: "Attias 2022". */
  label: string;
  title: string;
  /** "Attias, Smith - 2022": the picker's second line. */
  detail: string;
  /** Offered in the `@` picker (an item of the note's project). */
  pick: boolean;
  /** Other keys that mean this item (its key history). */
  aliases: string[];
}

export interface CitationIndex {
  entries: CitationEntry[];
  byKey: Map<string, CitationEntry>;
}

const EMPTY: CitationIndex = { entries: [], byKey: new Map() };
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
  const index = { entries: [...entries], byKey };
  indexCache.set(id, { entries, index });
  return index;
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
    });
  }
  return out;
}

/* ---- editors on screen ---- */

export interface EditorHandle {
  /** Insert at the caret when there is one in this editor. */
  insertAtCaret(text: string): boolean;
  reveal(offset: number): void;
}

const editors = new Map<string, Set<EditorHandle>>();
const pendingReveals = new Map<string, number>();

/** An editor on screen for `key`; returns the unregister. */
export function registerEditor(key: NoteKey, handle: EditorHandle): () => void {
  const id = keyString(key);
  let set = editors.get(id);
  if (!set) editors.set(id, (set = new Set()));
  set.add(handle);
  const offset = pendingReveals.get(id);
  if (offset !== undefined) {
    pendingReveals.delete(id);
    queueMicrotask(() => handle.reveal(offset));
  }
  return () => {
    set.delete(handle);
    if (set.size === 0) editors.delete(id);
  };
}

/** Scroll an editor of `key` to `offset`, now or when one mounts. */
export function revealInEditor(key: NoteKey, offset: number): void {
  const set = editors.get(keyString(key));
  const handle = set ? [...set][set.size - 1] : undefined;
  if (handle) handle.reveal(offset);
  else pendingReveals.set(keyString(key), offset);
}

/** Text appended to a note: after a blank line, ending in a newline. */
export function appendOp(text: string, insert: string): Op {
  const body = insert.replace(/\n+$/, '');
  let lead = '';
  if (text.length > 0) lead = text.endsWith('\n\n') ? '' : text.endsWith('\n') ? '\n' : '\n\n';
  return { from: text.length, to: text.length, insert: `${lead}${body}\n` };
}

/** Insert into a note: at the caret of an editor showing it in this window
 * when `at` is 'cursor' and there is one, else at the end. Returns the
 * offset the insertion starts at. */
export async function insertIntoNote(
  key: NoteKey,
  insert: string,
  at: 'cursor' | 'end'
): Promise<number> {
  const id = keyString(key);
  if (at === 'cursor') {
    for (const handle of editors.get(id) ?? []) if (handle.insertAtCaret(insert)) return -1;
  }
  await eventsHeard();
  const all = noteSessions();
  const session: NoteSession = all.acquire(key);
  try {
    await session.ready;
    if (session.status() !== 'ready') throw new Error(session.reason() ?? 'the note is not open');
    const text = session.text();
    const op = appendOp(text, insert);
    const caret = applyOp(text, op).length;
    session.edit(op, text.length, caret);
    await session.flush();
    // Confirmed by its broadcast; never waited on forever.
    await Promise.race([session.settled(), new Promise((r) => setTimeout(r, 5_000))]);
    return op.from + (op.insert.length - op.insert.trimStart().length);
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

/** One window's copy of an open note, shared by every view of it in the
 * window (the reader's drawer, the Projects screen's view).
 *
 * The core holds the note (`projectdocs.rs`); this is a client of it. The
 * text a view shows is `text()`: what the core has confirmed, plus this
 * window's edits not yet confirmed. An edit is sent as one range replacement
 * against the revision last seen, at most one in flight; the core answers by
 * broadcasting it, as applied, to every window, and this window recognises
 * its own by `origin`. Another window's edit is applied to the confirmed
 * text and this window's pending change is moved past it, the same way the
 * core moves it, so both arrive at the same text.
 *
 * Undo and redo are the note's own history in this window, across blocks
 * and modes: each entry is an edit and its inverse, kept in step with edits
 * from elsewhere.
 *
 * Decorations are what a view draws in the note that is not in its text: a
 * placeholder where something being sent will go, and a status line under
 * a block being checked. They live here only, never in the file, and their
 * places map through every edit the way a caret does, so a crash leaves
 * nothing behind. */
import { createSignal, type Accessor } from 'solid-js';
import { applyOp, diffOp, invertOp, mapPos, transformOp, type Op } from './ops';

export interface NoteKey {
  extensionId: string;
  projectId: string;
  file: string;
}

export function keyString(key: NoteKey): string {
  return `${key.extensionId}\u0000${key.projectId}\u0000${key.file}`;
}

/** What the core broadcasts (`DocEvent` in `projectdocs.rs`). */
export type DocEvent =
  | { kind: 'edit'; key: NoteKey; rev: number; op: Op; origin: string; len: number }
  | { kind: 'reset'; key: NoteKey; rev: number; text: string }
  | { kind: 'closed'; key: NoteKey; reason: string }
  | { kind: 'notice'; key: NoteKey; message: string };

/** How a session reaches the core; the app's goes through Tauri, a test's
 * through a fake. */
export interface NoteTransport {
  open(key: NoteKey): Promise<{ text: string; rev: number }>;
  edit(key: NoteKey, baseRev: number, op: Op, origin: string): Promise<number>;
  close(key: NoteKey): Promise<void>;
}

interface HistoryEntry {
  op: Op;
  inverse: Op;
  caretBefore: number;
  caretAfter: number;
  at: number;
}

/** Typing within this long of the last edit, in one place, is one undo
 * step. */
const COALESCE_MS = 800;
const HISTORY_LIMIT = 500;
/** Confirmed edits kept, to place another window's decorations made
 * against an older revision. */
const OP_LOG_LIMIT = 200;

/** What a placeholder stands in for. */
export interface PendingShape {
  kind: 'equation' | 'image' | 'text';
  /** The expected content's proportions (an image's drawn region). */
  shape: { w: number; h: number } | null;
  /** The line under it, as it will read ("Equation (3), [@key, p. 4]"). */
  caption: string | null;
}

interface DecorationBase {
  id: string;
  /** The extension it belongs to. */
  owner: string;
  /** The window that made it: this one's origin, or another's. */
  origin: string;
  created: number;
  expires: number;
}

/** A placeholder at `at`, where its content will be inserted. */
export interface PendingDecoration extends DecorationBase {
  type: 'pending';
  at: number;
  pending: PendingShape;
}

/** A tracked range of the note, `[from, to)`: an inserted block. With a
 * `label`, a dim status line is drawn under it; without, it is only
 * tracked. An edit inside it (or over it) drops it. */
export interface MarkDecoration extends DecorationBase {
  type: 'mark';
  from: number;
  to: number;
  label: string | null;
}

export type Decoration = PendingDecoration | MarkDecoration;

/** Decorations as another window sends them: places in the confirmed text
 * at `rev`. */
export interface DecorationSnapshot {
  key: NoteKey;
  origin: string;
  rev: number;
  list: Decoration[];
}

/** Where a decoration lands once `op` is applied, or null when the edit
 * drops it (a mark edited inside or over). Text inserted exactly at a
 * placeholder goes before it, so two sends in a row keep their order; text
 * typed at a mark's edges stays outside it. */
export function mapDecoration(d: Decoration, op: Op): Decoration | null {
  if (d.type === 'pending') return { ...d, at: mapPos(op, d.at, true) };
  const overlaps = op.from < d.to && op.to > d.from;
  const inside = op.from === op.to && op.from > d.from && op.from < d.to;
  if (overlaps || inside) return null;
  return { ...d, from: mapPos(op, d.from, true), to: mapPos(op, d.to, false) };
}
/** How long local edits gather before they are sent. */
const SEND_DELAY_MS = 40;

export type SessionStatus = 'loading' | 'ready' | 'closed' | 'error';

export class NoteSession {
  readonly key: NoteKey;
  readonly text: Accessor<string>;
  readonly status: Accessor<SessionStatus>;
  /** Why it closed or failed, for the view to say. */
  readonly reason: Accessor<string | null>;
  /** A one-off message from the core ("changed on disk..."). */
  readonly notice: Accessor<string | null>;
  /** Bumped on every change not made through `edit` (another view's,
   * another window's, a reload), with the edit as it applies to `text()`:
   * views map their caret through it. `null` for a whole reload. */
  readonly external: Accessor<{ op: Op | null; seq: number }>;
  /** Placeholders and marks, at places in `text()`. */
  readonly decorations: Accessor<Decoration[]>;
  /** Called when a decoration of this window's expires, or an edit drops
   * it. */
  onExpire: ((d: Decoration) => void) | null = null;

  private setText: (text: string) => void;
  private setStatus: (status: SessionStatus) => void;
  private setReason: (reason: string | null) => void;
  private setNotice: (notice: string | null) => void;
  private setExternal: (value: { op: Op | null; seq: number }) => void;
  private setDecorations: (list: Decoration[]) => void;
  private expiryTimer: ReturnType<typeof setTimeout> | null = null;
  /** Confirmed edits by revision, newest last. */
  private opLog: { rev: number; op: Op }[] = [];
  /** Another window's decorations made against a revision not seen yet. */
  private early = new Map<string, DecorationSnapshot>();
  private seq = 0;
  private confirmed = '';
  private rev = 0;
  private inflight = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  /** Resolves when nothing is waiting to be sent or confirmed. */
  private idleWaiters: (() => void)[] = [];
  users = 0;
  readonly ready: Promise<void>;

  constructor(
    key: NoteKey,
    private readonly transport: NoteTransport,
    readonly origin: string,
    private readonly now: () => number = () => Date.now()
  ) {
    this.key = key;
    const [text, setText] = createSignal('');
    const [status, setStatus] = createSignal<SessionStatus>('loading');
    const [reason, setReason] = createSignal<string | null>(null);
    const [notice, setNotice] = createSignal<string | null>(null);
    const [external, setExternal] = createSignal<{ op: Op | null; seq: number }>({
      op: null,
      seq: 0,
    });
    this.text = text;
    this.setText = setText;
    this.status = status;
    this.setStatus = setStatus;
    this.reason = reason;
    this.setReason = setReason;
    this.notice = notice;
    this.setNotice = setNotice;
    this.external = external;
    this.setExternal = setExternal;
    const [decorations, setDecorations] = createSignal<Decoration[]>([]);
    this.decorations = decorations;
    this.setDecorations = setDecorations;
    this.ready = this.load();
  }

  private async load(): Promise<void> {
    try {
      const opened = await this.transport.open(this.key);
      this.confirmed = opened.text;
      this.rev = opened.rev;
      this.setText(opened.text);
      this.setStatus('ready');
    } catch (err) {
      this.setReason(err instanceof Error ? err.message : String(err));
      this.setStatus('error');
    }
  }

  get revision(): number {
    return this.rev;
  }

  /** Apply a local edit. `caret` is where the cursor was before and is after,
   * for undo to put it back. Typing next to the previous edit within a
   * moment joins its undo step. `outside`: made by no view (an extension's
   * insert), so every view hears of it as of another's. */
  edit(
    op: Op,
    caretBefore: number,
    caretAfter: number,
    opts?: { history?: boolean; outside?: boolean }
  ): void {
    if (this.status() !== 'ready') return;
    const before = this.text();
    const after = applyOp(before, op);
    if (after === before) return;
    if (opts?.history !== false) {
      const inverse = invertOp(op, before);
      const last = this.undoStack[this.undoStack.length - 1];
      const now = this.now();
      const adjacent =
        last !== undefined &&
        now - last.at < COALESCE_MS &&
        !/\n/.test(op.insert) &&
        // Typing on: this edit starts where the last one ended.
        ((op.from === op.to && op.from === last.op.from + last.op.insert.length) ||
          // Deleting back: this edit ends where the last one started.
          (op.insert === '' && op.to === last.op.from && last.op.insert === ''));
      if (adjacent) {
        // One step from the text before the last step to the text now,
        // worked out from the two edits rather than by comparing texts: a
        // keystroke in a long note must not cost a pass over all of it.
        if (op.insert !== '') {
          // Typing on: the last step inserted a little more.
          last.op = { from: last.op.from, to: last.op.to, insert: last.op.insert + op.insert };
          last.inverse = {
            from: last.op.from,
            to: last.op.from + last.op.insert.length,
            insert: last.inverse.insert,
          };
        } else {
          // Deleting back: the last step deleted a little more, from earlier.
          const removed = before.slice(op.from, op.to);
          last.op = { from: op.from, to: last.op.to, insert: '' };
          last.inverse = { from: op.from, to: op.from, insert: removed + last.inverse.insert };
        }
        last.caretAfter = caretAfter;
        last.at = now;
      } else {
        this.undoStack.push({ op, inverse, caretBefore, caretAfter, at: now });
        if (this.undoStack.length > HISTORY_LIMIT) this.undoStack.shift();
      }
      this.redoStack = [];
    }
    this.mapDecorations(op);
    this.setText(after);
    if (opts?.outside) this.setExternal({ op, seq: ++this.seq });
    this.scheduleSend();
  }

  canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  /** Undo the last step; returns where the caret goes, or null. */
  undo(): number | null {
    const entry = this.undoStack.pop();
    if (!entry || this.status() !== 'ready') return null;
    const before = this.text();
    this.mapDecorations(entry.inverse);
    this.setText(applyOp(before, entry.inverse));
    this.redoStack.push(entry);
    this.scheduleSend();
    return entry.caretBefore;
  }

  redo(): number | null {
    const entry = this.redoStack.pop();
    if (!entry || this.status() !== 'ready') return null;
    this.mapDecorations(entry.op);
    this.setText(applyOp(this.text(), entry.op));
    this.undoStack.push(entry);
    this.scheduleSend();
    return entry.caretAfter;
  }

  private scheduleSend(): void {
    if (this.inflight || this.timer !== null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.send();
    }, SEND_DELAY_MS);
  }

  /** Send now whatever is unsent (when nothing is in flight). */
  async flush(): Promise<void> {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    await this.send();
  }

  /** Resolves once every local edit has been confirmed by the core. */
  settled(): Promise<void> {
    if (!this.inflight && this.timer === null && this.text() === this.confirmed) {
      return Promise.resolve();
    }
    return new Promise((resolve) => this.idleWaiters.push(resolve));
  }

  private wakeIdle(): void {
    if (this.inflight || this.timer !== null || this.text() !== this.confirmed) return;
    const waiters = this.idleWaiters;
    this.idleWaiters = [];
    for (const wake of waiters) wake();
  }

  private async send(): Promise<void> {
    if (this.inflight || this.status() !== 'ready') {
      this.wakeIdle();
      return;
    }
    const op = diffOp(this.confirmed, this.text());
    if (!op) {
      this.wakeIdle();
      return;
    }
    this.inflight = true;
    try {
      await this.transport.edit(this.key, this.rev, op, this.origin);
      // Confirmation comes as the broadcast edit (`receive`), in order with
      // everyone else's; the reply itself carries nothing more.
    } catch (err) {
      this.inflight = false;
      this.setNotice(`Could not save an edit: ${err instanceof Error ? err.message : String(err)}`);
      this.wakeIdle();
    }
  }

  /** An event from the core about this note. */
  receive(event: DocEvent): void {
    switch (event.kind) {
      case 'edit': {
        if (event.rev <= this.rev) return;
        this.logOp(event.rev, event.op);
        if (event.origin === this.origin) {
          this.confirmed = applyOp(this.confirmed, event.op);
          this.rev = event.rev;
          this.adoptEarly();
          this.inflight = false;
          if (this.text() !== this.confirmed) this.scheduleSend();
          this.wakeIdle();
          return;
        }
        const pending = diffOp(this.confirmed, this.text());
        this.confirmed = applyOp(this.confirmed, event.op);
        this.rev = event.rev;
        let local = this.confirmed;
        // The remote edit as it applies to what this window shows: the core
        // put it before this window's pending change.
        let shown = event.op;
        if (pending) {
          local = applyOp(this.confirmed, transformOp(pending, event.op, true));
          shown = transformOp(event.op, pending, false);
        }
        this.mapHistory(shown);
        this.mapDecorations(shown);
        this.setText(local);
        this.setExternal({ op: shown, seq: ++this.seq });
        this.adoptEarly();
        if (!pending && this.confirmed.length !== event.len) void this.resync();
        return;
      }
      case 'reset':
        this.confirmed = event.text;
        this.rev = event.rev;
        this.inflight = false;
        this.undoStack = [];
        this.redoStack = [];
        this.opLog = [];
        // The text was replaced whole: a mark no longer means anything, and
        // a placeholder keeps its place as near as the new text allows.
        this.setDecorations(
          this.decorations().flatMap((d) => {
            if (d.type === 'pending') return [{ ...d, at: Math.min(d.at, event.text.length) }];
            this.dropped(d);
            return [];
          })
        );
        this.setText(event.text);
        this.setExternal({ op: null, seq: ++this.seq });
        this.wakeIdle();
        return;
      case 'closed':
        this.setReason(event.reason);
        this.setStatus('closed');
        this.wakeIdle();
        return;
      case 'notice':
        this.setNotice(event.message);
        return;
    }
  }

  dismissNotice(): void {
    this.setNotice(null);
  }

  /* ---- decorations ---- */

  decoration(id: string): Decoration | undefined {
    return this.decorations().find((d) => d.id === id);
  }

  /** Add or replace a decoration (by id). */
  putDecoration(d: Decoration): void {
    const list = this.decorations().filter((x) => x.id !== d.id);
    list.push(d);
    this.setDecorations(list);
    this.scheduleExpiry();
  }

  /** Remove a decoration; returns it, or undefined when it was gone. */
  removeDecoration(id: string): Decoration | undefined {
    const found = this.decoration(id);
    if (found) this.setDecorations(this.decorations().filter((d) => d.id !== id));
    return found;
  }

  /** Remove every decoration an extension made. */
  clearDecorations(owner: string): Decoration[] {
    const gone = this.decorations().filter((d) => d.owner === owner);
    if (gone.length > 0) this.setDecorations(this.decorations().filter((d) => d.owner !== owner));
    return gone;
  }

  /** This window's decorations as places in the confirmed text, for other
   * windows. */
  snapshot(): DecorationSnapshot {
    const pending = diffOp(this.confirmed, this.text());
    const back = pending ? invertOp(pending, this.confirmed) : null;
    const toConfirmed = (pos: number) => (back ? mapPos(back, pos, true) : pos);
    const list = this.decorations()
      .filter((d) => d.origin === this.origin)
      .map((d) =>
        d.type === 'pending'
          ? { ...d, at: toConfirmed(d.at) }
          : { ...d, from: toConfirmed(d.from), to: toConfirmed(d.to) }
      );
    return { key: this.key, origin: this.origin, rev: this.rev, list };
  }

  /** Another window's decorations: they replace what it showed here
   * before. */
  receiveDecorations(snapshot: DecorationSnapshot): void {
    if (snapshot.origin === this.origin) return;
    if (snapshot.rev > this.rev) {
      // Made against edits this window has not heard yet: wait for them.
      this.early.set(snapshot.origin, snapshot);
      return;
    }
    this.early.delete(snapshot.origin);
    const since = this.opLog.filter((e) => e.rev > snapshot.rev);
    const complete =
      snapshot.rev === this.rev || (since.length > 0 && since[0]!.rev === snapshot.rev + 1);
    const pending = diffOp(this.confirmed, this.text());
    const placed: Decoration[] = [];
    if (complete) {
      for (const raw of snapshot.list) {
        let d: Decoration | null = { ...raw, origin: snapshot.origin };
        for (const entry of since) d = d && mapDecoration(d, entry.op);
        if (d && pending) d = mapDecoration(d, pending);
        if (d) placed.push(d);
      }
    }
    this.setDecorations([
      ...this.decorations().filter((d) => d.origin !== snapshot.origin),
      ...placed,
    ]);
    this.scheduleExpiry();
  }

  private adoptEarly(): void {
    for (const snapshot of [...this.early.values()]) {
      if (snapshot.rev <= this.rev) this.receiveDecorations(snapshot);
    }
  }

  private logOp(rev: number, op: Op): void {
    this.opLog.push({ rev, op });
    if (this.opLog.length > OP_LOG_LIMIT) this.opLog.shift();
  }

  private mapDecorations(op: Op): void {
    const list = this.decorations();
    if (list.length === 0) return;
    const next: Decoration[] = [];
    for (const d of list) {
      const moved = mapDecoration(d, op);
      if (moved) next.push(moved);
      else this.dropped(d);
    }
    this.setDecorations(next);
  }

  /** Tell the owner, once the edit that dropped it is in. */
  private dropped(d: Decoration): void {
    if (d.origin === this.origin) queueMicrotask(() => this.onExpire?.(d));
  }

  /** Drop what has expired, and wake for the next to. */
  private scheduleExpiry(): void {
    if (this.expiryTimer !== null) clearTimeout(this.expiryTimer);
    this.expiryTimer = null;
    const list = this.decorations();
    if (list.length === 0) return;
    const now = this.now();
    const expired = list.filter((d) => d.expires <= now);
    if (expired.length > 0) {
      this.setDecorations(list.filter((d) => d.expires > now));
      for (const d of expired) if (d.origin === this.origin) this.onExpire?.(d);
    }
    const left = this.decorations();
    if (left.length === 0) return;
    const next = Math.min(...left.map((d) => d.expires));
    this.expiryTimer = setTimeout(() => this.scheduleExpiry(), Math.max(0, next - this.now()));
  }

  /** Keep undo steps pointing at the right text after someone else's edit. */
  private mapHistory(op: Op): void {
    const move = (entry: HistoryEntry) => {
      entry.op = transformOp(entry.op, op, true);
      entry.inverse = transformOp(entry.inverse, op, true);
      entry.caretBefore = mapPos(op, entry.caretBefore, true);
      entry.caretAfter = mapPos(op, entry.caretAfter, true);
    };
    this.undoStack.forEach(move);
    this.redoStack.forEach(move);
  }

  /** Start over from the core's copy: this window lost track of it. */
  private async resync(): Promise<void> {
    try {
      await this.transport.close(this.key);
      const opened = await this.transport.open(this.key);
      this.receive({ kind: 'reset', key: this.key, rev: opened.rev, text: opened.text });
    } catch {
      /* the next edit will report it */
    }
  }
}

/** Sessions by note, for one window. */
export class NoteSessions {
  private sessions = new Map<string, NoteSession>();

  constructor(
    private readonly transport: NoteTransport,
    private readonly origin: string
  ) {}

  /** Join a note's session, opening it with the core on first use. */
  acquire(key: NoteKey): NoteSession {
    const id = keyString(key);
    let session = this.sessions.get(id);
    if (!session || session.status() === 'closed' || session.status() === 'error') {
      session = new NoteSession(key, this.transport, this.origin);
      this.sessions.set(id, session);
    }
    session.users++;
    return session;
  }

  /** Leave it; the last view out sends what is left and closes it. */
  release(session: NoteSession): void {
    session.users = Math.max(0, session.users - 1);
    if (session.users > 0) return;
    const id = keyString(session.key);
    void (async () => {
      await session.flush();
      await session.settled();
      if (session.users > 0) return;
      if (this.sessions.get(id) === session) this.sessions.delete(id);
      if (session.status() === 'ready') await this.transport.close(session.key).catch(() => {});
    })();
  }

  get(key: NoteKey): NoteSession | undefined {
    return this.sessions.get(keyString(key));
  }

  /** Route a core event to its session. */
  dispatch(event: DocEvent): void {
    this.sessions.get(keyString(event.key))?.receive(event);
  }

  /** Send everything unsent now (a window closing). */
  async flushAll(): Promise<void> {
    await Promise.all([...this.sessions.values()].map((s) => s.flush()));
  }
}

/** Placeholders and marks: what an extension puts in a note while a send
 * is on its way (`editor.insert { pending }`, `editor.resolve`,
 * `editor.mark`, `editor.replaceMarked`). They live in the note's session,
 * never in its file, move with every edit, show in every window, and go
 * away on their own. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyOp } from './editor/ops';
import {
  NoteSessions,
  type DecorationSnapshot,
  type NoteKey,
  type NoteTransport,
} from './editor/noteSession';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

const host = await import('./editorHost');

const KEY: NoteKey = { extensionId: 'dev.speedyf.notepad', projectId: 'p1', file: 'notepad.md' };
const OWNER = 'dev.speedyf.notepad';
const EQUATION = { kind: 'equation' as const, shape: null, caption: 'Equation (3), [@k, p. 4]' };

/** A core holding one note, broadcasting each edit to every window as the
 * real one does; this window (the host's) and, optionally, another. */
function core(initial: string) {
  const state = { text: initial, rev: 1, edits: 0 };
  const windows: NoteSessions[] = [];
  const transport: NoteTransport = {
    open: async () => ({ text: state.text, rev: state.rev }),
    edit: async (_key, _base, op, origin) => {
      state.text = applyOp(state.text, op);
      state.rev += 1;
      state.edits += 1;
      const rev = state.rev;
      const len = state.text.length;
      queueMicrotask(() => {
        for (const w of windows) w.dispatch({ kind: 'edit', key: KEY, rev, op, origin, len });
      });
      return rev;
    },
    close: async () => {},
  };
  const sent: DecorationSnapshot[] = [];
  const other = new NoteSessions(transport, 'other-window');
  const here = host.useNoteSessionsForTest(transport, 'test-window', (snapshot) => {
    sent.push(snapshot);
    other.get(snapshot.key)?.receiveDecorations(snapshot);
  });
  windows.push(here, other);
  return { state, here, other, sent };
}

const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};

afterEach(() => {
  host.clearDecorationsOf(OWNER);
  vi.useRealTimers();
});

describe('a pending insert', () => {
  it('writes nothing to the file, and fills in at its place as one undoable edit', async () => {
    const { state, here } = core('intro\n');
    const view = here.acquire(KEY);
    await view.ready;
    const { handle, offset } = await host.insertPending(KEY, EQUATION, 'end', OWNER);
    expect(offset).toBe(6);
    await settle();
    expect(state.text).toBe('intro\n');
    expect(state.edits).toBe(0);
    expect(view.decorations()).toEqual([
      expect.objectContaining({ type: 'pending', id: handle, at: 6, pending: EQUATION }),
    ]);

    const done = await host.resolvePending(OWNER, handle, '$$ a $$\n\nEquation (3)');
    expect(done).toEqual({ offset: 7 });
    expect(state.text).toBe('intro\n\n$$ a $$\n\nEquation (3)\n');
    expect(state.edits).toBe(1);
    // The placeholder is gone; the text it became is tracked, undrawn.
    expect(view.decorations()).toEqual([
      expect.objectContaining({ type: 'mark', id: handle, from: 7, label: null }),
    ]);
    view.undo();
    await view.flush();
    await view.settled();
    expect(state.text).toBe('intro\n');
    here.release(view);
  });

  it('keeps its place through edits before and after it', async () => {
    const { state, here } = core('one\n\ntwo\n');
    const view = here.acquire(KEY);
    await view.ready;
    // A placeholder after "one" (an editor's caret block end, as 'cursor'
    // gives): edits before it move it, edits after it do not.
    const unregister = host.registerEditor(KEY, {
      insertAtCaret: () => null,
      caretBlockEnd: () => 3,
      reveal: () => {},
    });
    const { handle } = await host.insertPending(KEY, EQUATION, 'cursor', OWNER);
    unregister();
    view.edit({ from: 0, to: 0, insert: 'zero ' }, 0, 5);
    view.edit({ from: view.text().length, to: view.text().length, insert: 'three\n' }, 0, 0);
    expect(view.decoration(handle)).toMatchObject({ type: 'pending', at: 8 });
    // Typing exactly at the place goes before it.
    view.edit({ from: 8, to: 8, insert: '!' }, 8, 9);
    expect(view.decoration(handle)).toMatchObject({ at: 9 });
    await host.resolvePending(OWNER, handle, 'X');
    expect(state.text).toBe('zero one!\n\nX\n\ntwo\nthree\n');
    here.release(view);
  });

  it('is removed by cancel, and on its own after 30 s', async () => {
    const { state, here } = core('a\n');
    const view = here.acquire(KEY);
    await view.ready;
    const first = await host.insertPending(KEY, EQUATION, 'end', OWNER);
    expect(host.cancelHandle(OWNER, first.handle)).toBe(true);
    expect(view.decorations()).toEqual([]);
    // Another extension cannot reach a handle that is not its own.
    const second = await host.insertPending(KEY, EQUATION, 'end', OWNER);
    expect(host.cancelHandle('someone.else', second.handle)).toBe(false);
    vi.useFakeTimers();
    const third = await host.insertPending(KEY, { ...EQUATION, kind: 'text' }, 'end', OWNER);
    view.decorations(); // both still there
    expect(view.decorations()).toHaveLength(2);
    vi.advanceTimersByTime(host.DECORATION_TTL_MS + 1);
    expect(view.decorations()).toEqual([]);
    vi.useRealTimers();
    // Expired: resolving it inserts nothing; the caller puts it elsewhere.
    expect(await host.resolvePending(OWNER, third.handle, 'late')).toBeNull();
    expect(state.text).toBe('a\n');
    here.release(view);
  });
});

describe('marks', () => {
  it('show in another window, and leave with the mark', async () => {
    const { here, other } = core('intro\n');
    const view = here.acquire(KEY);
    const elsewhere = other.acquire(KEY);
    await Promise.all([view.ready, elsewhere.ready]);
    const { handle } = await host.insertPending(KEY, EQUATION, 'end', OWNER);
    expect(elsewhere.decorations()).toEqual([
      expect.objectContaining({ type: 'pending', at: 6, origin: 'test-window' }),
    ]);
    await host.resolvePending(OWNER, handle, '$$ a $$');
    await settle();
    expect(elsewhere.text()).toBe('intro\n\n$$ a $$\n');
    expect(host.markHandle(OWNER, handle, 'checking arXiv source')).toBe(true);
    const shown = (d: { type: string }) => d.type === 'mark';
    expect(view.decorations().filter(shown)).toEqual([
      expect.objectContaining({ from: 7, to: 14, label: 'checking arXiv source' }),
    ]);
    expect(elsewhere.decorations().filter(shown)).toEqual([
      expect.objectContaining({ from: 7, to: 14, label: 'checking arXiv source' }),
    ]);
    // An edit made in the other window before the mark moves it there too.
    elsewhere.edit({ from: 0, to: 0, insert: '# ' }, 0, 2);
    await elsewhere.flush();
    await settle();
    expect(view.decoration(handle)).toMatchObject({ from: 9, to: 16 });
    expect(elsewhere.decoration(handle)).toMatchObject({ from: 9, to: 16 });
    host.relabelHandle(OWNER, handle, 'checking journal source');
    expect(elsewhere.decoration(handle)).toMatchObject({ label: 'checking journal source' });
    expect(host.cancelHandle(OWNER, handle)).toBe(true);
    expect(elsewhere.decorations()).toEqual([]);
    here.release(view);
    other.release(elsewhere);
  });

  it('replace exactly the marked text, and are refused once the user edited inside', async () => {
    const { state, here } = core('');
    const view = here.acquire(KEY);
    await view.ready;
    const sent = await host.insertIntoNote(KEY, '$$ a $$', 'end', OWNER);
    expect(sent.offset).toBe(0);
    view.edit({ from: 7, to: 7, insert: '\nmore' }, 7, 12);
    expect(host.markHandle(OWNER, sent.handle, 'checking source')).toBe(true);
    expect(await host.replaceMarked(OWNER, sent.handle, '$$ b $$')).toBe(true);
    await view.flush();
    await settle();
    expect(state.text).toBe('$$ b $$\nmore\n');

    const again = await host.insertIntoNote(KEY, '$$ c $$', 'end', OWNER);
    host.markHandle(OWNER, again.handle, 'checking source');
    // The user changes the formula: the mark drops, the upgrade is refused.
    const at = state.text.indexOf('c');
    view.edit({ from: at, to: at + 1, insert: 'C' }, at, at + 1);
    expect(view.decoration(again.handle)).toBeUndefined();
    expect(await host.replaceMarked(OWNER, again.handle, '$$ d $$')).toBe(false);
    await view.flush();
    await settle();
    expect(state.text).toBe('$$ b $$\nmore\n\n$$ C $$\n');
    here.release(view);
  });

  it('all go when their extension is disabled', async () => {
    const { here, other } = core('x\n');
    const view = here.acquire(KEY);
    const elsewhere = other.acquire(KEY);
    await Promise.all([view.ready, elsewhere.ready]);
    await host.insertPending(KEY, EQUATION, 'end', OWNER);
    const sent = await host.insertIntoNote(KEY, 'y', 'end', OWNER);
    host.markHandle(OWNER, sent.handle, 'checking source');
    expect(view.decorations()).toHaveLength(2);
    host.clearDecorationsOf(OWNER);
    expect(view.decorations()).toEqual([]);
    expect(elsewhere.decorations()).toEqual([]);
    here.release(view);
    other.release(elsewhere);
  });
});

describe('a note whose references close it', () => {
  it('puts a quote before the references, even when entries were added meanwhile', async () => {
    const { state, here } = core('intro\n');
    const view = here.acquire(KEY);
    await view.ready;
    const { handle } = await host.insertPending(KEY, EQUATION, 'end', OWNER);
    // The send's references go in first: the section is made at the end.
    const ids = await host.addNoteReferences(KEY, [
      {
        text: 'L. Larsen, Blocks of random matrices (2023).',
        author: 'Larsen',
        year: 2023,
        title: 'Blocks of random matrices',
      },
    ]);
    expect(ids).toEqual(['larsen2023blocks']);
    await settle();
    await host.resolvePending(OWNER, handle, '> As in [^larsen2023blocks]{label=Lar23}.');
    expect(state.text).toBe(
      [
        'intro',
        '',
        '> As in [^larsen2023blocks]{label=Lar23}.',
        '',
        '## References',
        '',
        '[^larsen2023blocks]: L. Larsen, Blocks of random matrices (2023).',
        '',
      ].join('\n')
    );
    // The next quote at the end goes before them too.
    await host.insertIntoNote(KEY, '> Next.', 'end', OWNER);
    expect(state.text).toContain('{label=Lar23}.\n\n> Next.\n\n## References');
  });
});

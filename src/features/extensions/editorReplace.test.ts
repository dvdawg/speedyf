/** `editor.replace` goes through the note's session, like a keystroke: one
 * undoable edit, ordered by the core, of the first exact occurrence only,
 * and nothing at all when the text is not there. */
import { describe, expect, it, vi } from 'vitest';
import { applyOp } from './editor/ops';
import type { NoteKey, NoteTransport } from './editor/noteSession';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

const host = await import('./editorHost');

const KEY: NoteKey = { extensionId: 'dev.speedyf.notepad', projectId: 'p1', file: 'notepad.md' };

/** A core holding one note, broadcasting each edit back as the real one does. */
function core(initial: string) {
  const state = { text: initial, rev: 1, edits: 0 };
  const transport: NoteTransport = {
    open: async () => ({ text: state.text, rev: state.rev }),
    edit: async (_key, _base, op, origin) => {
      state.text = applyOp(state.text, op);
      state.rev += 1;
      state.edits += 1;
      const rev = state.rev;
      queueMicrotask(() =>
        sessions.dispatch({ kind: 'edit', key: KEY, rev, op, origin, len: state.text.length })
      );
      return rev;
    },
    close: async () => {},
  };
  const sessions = host.useNoteSessionsForTest(transport, 'test-window');
  return { state, sessions };
}

describe('editor.replace', () => {
  it('replaces the first occurrence through the session, undoably', async () => {
    const { state, sessions } = core('intro\n\n$$ a $$\n\nEq\n\n$$ a $$\n\nEq\n');
    // An editor on screen holds the session, as the drawer does.
    const view = sessions.acquire(KEY);
    await view.ready;
    expect(await host.replaceInNote(KEY, '$$ a $$\n\nEq', '$$ b $$\n\nEq')).toBe(true);
    expect(state.text).toBe('intro\n\n$$ b $$\n\nEq\n\n$$ a $$\n\nEq\n');
    expect(view.text()).toBe(state.text);
    expect(state.edits).toBe(1);
    // Undo in the editor puts the inserted text back.
    expect(view.canUndo()).toBe(true);
    view.undo();
    await view.flush();
    await view.settled();
    expect(state.text).toBe('intro\n\n$$ a $$\n\nEq\n\n$$ a $$\n\nEq\n');
    sessions.release(view);
  });

  it('answers false and touches nothing when the text is not there', async () => {
    const { state } = core('$$ a $$ edited\n');
    expect(await host.replaceInNote(KEY, '$$ a $$\n\nEq', '$$ b $$\n\nEq')).toBe(false);
    expect(state.text).toBe('$$ a $$ edited\n');
    expect(state.edits).toBe(0);
  });
});

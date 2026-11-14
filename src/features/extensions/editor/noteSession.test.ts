/** One note, many views: the window-side session against a fake core that
 * orders edits the way `projectdocs.rs` does. */
import { createRoot } from 'solid-js';
import { describe, expect, it } from 'vitest';
import { applyOp, transformOp, type Op } from './ops';
import {
  NoteSessions,
  type DocEvent,
  type NoteKey,
  type NoteSession,
  type NoteTransport,
} from './noteSession';

const KEY: NoteKey = { extensionId: 'dev.speedyf.notepad', projectId: 'p1', file: 'notepad.md' };

/** The core's open document, as `projectdocs.rs` keeps it, broadcasting to
 * every window. Delivery is queued, so a test decides when each window
 * hears what. */
class FakeCore {
  text = '';
  rev = 1;
  log: { rev: number; op: Op }[] = [];
  windows: NoteSessions[] = [];
  queue: DocEvent[] = [];

  transport(): NoteTransport {
    return {
      open: async () => ({ text: this.text, rev: this.rev }),
      edit: async (_key, baseRev, op, origin) => {
        let moved = op;
        for (const entry of this.log)
          if (entry.rev > baseRev) moved = transformOp(moved, entry.op, true);
        moved = {
          from: Math.min(moved.from, this.text.length),
          to: Math.min(Math.max(moved.from, moved.to), this.text.length),
          insert: moved.insert,
        };
        this.text = applyOp(this.text, moved);
        this.rev += 1;
        this.log.push({ rev: this.rev, op: moved });
        this.queue.push({
          kind: 'edit',
          key: KEY,
          rev: this.rev,
          op: moved,
          origin,
          len: this.text.length,
        });
        return this.rev;
      },
      close: async () => {},
    };
  }

  /** Deliver everything queued to every window, in order. */
  deliver(): void {
    while (this.queue.length > 0) {
      const event = this.queue.shift()!;
      for (const window of this.windows) window.dispatch(event);
    }
  }

  broadcast(event: DocEvent): void {
    for (const window of this.windows) window.dispatch(event);
  }
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 60));

async function settle(core: FakeCore, ...sessions: NoteSession[]) {
  for (let i = 0; i < 6; i++) {
    await tick();
    core.deliver();
  }
  await Promise.all(sessions.map((s) => s.settled()));
}

function windowOn(core: FakeCore, origin: string): NoteSessions {
  const sessions = new NoteSessions(core.transport(), origin);
  core.windows.push(sessions);
  return sessions;
}

const type = (s: NoteSession, at: number, text: string) =>
  s.edit({ from: at, to: at, insert: text }, at, at + text.length);

describe('note sessions', () => {
  it('two windows editing the same note at once both end with the same text', async () => {
    await createRoot(async (dispose) => {
      const core = new FakeCore();
      core.text = 'hello world';
      const a = windowOn(core, 'A').acquire(KEY);
      const b = windowOn(core, 'B').acquire(KEY);
      await Promise.all([a.ready, b.ready]);
      // Both type before either hears of the other.
      type(a, 11, '!');
      type(b, 0, '>> ');
      a.edit({ from: 0, to: 5, insert: 'HELLO' }, 5, 5);
      await settle(core, a, b);
      expect(core.text).toBe('>> HELLO world!');
      expect(a.text()).toBe(core.text);
      expect(b.text()).toBe(core.text);
      // Then each keeps typing on the other's text.
      type(b, b.text().length, ' (b)');
      type(a, 0, 'x');
      await settle(core, a, b);
      expect(a.text()).toBe(b.text());
      expect(a.text()).toBe(core.text);
      dispose();
    });
  });

  it('views in one window share one session and its undo', async () => {
    await createRoot(async (dispose) => {
      const core = new FakeCore();
      const window = windowOn(core, 'A');
      const drawer = window.acquire(KEY);
      const projectView = window.acquire(KEY);
      expect(projectView).toBe(drawer);
      await drawer.ready;
      type(drawer, 0, 'a');
      type(drawer, 1, 'b');
      drawer.edit({ from: 2, to: 2, insert: '\nline' }, 2, 7);
      expect(drawer.undo()).toBe(2);
      expect(drawer.text()).toBe('ab');
      // Typing next to the last edit is one step.
      expect(drawer.undo()).toBe(0);
      expect(drawer.text()).toBe('');
      expect(drawer.redo()).toBe(2);
      expect(drawer.text()).toBe('ab');
      await settle(core, drawer);
      expect(core.text).toBe('ab');
      dispose();
    });
  });

  it("undo after another window edited undoes only this window's edit", async () => {
    await createRoot(async (dispose) => {
      const core = new FakeCore();
      core.text = 'middle';
      const a = windowOn(core, 'A').acquire(KEY);
      const b = windowOn(core, 'B').acquire(KEY);
      await Promise.all([a.ready, b.ready]);
      type(a, 6, ' end');
      await settle(core, a, b);
      type(b, 0, 'start ');
      await settle(core, a, b);
      a.undo();
      await settle(core, a, b);
      expect(core.text).toBe('start middle');
      expect(b.text()).toBe('start middle');
      dispose();
    });
  });

  it('keeps non-ASCII text intact through edits from two windows', async () => {
    await createRoot(async (dispose) => {
      const core = new FakeCore();
      core.text = 'Schrödinger \u{1d49c}';
      const a = windowOn(core, 'A').acquire(KEY);
      const b = windowOn(core, 'B').acquire(KEY);
      await Promise.all([a.ready, b.ready]);
      type(a, a.text().length, ' α→β');
      type(b, 0, '中文 ');
      await settle(core, a, b);
      expect(core.text).toBe('中文 Schrödinger \u{1d49c} α→β');
      expect(a.text()).toBe(core.text);
      expect(b.text()).toBe(core.text);
      dispose();
    });
  });

  it('a reload from disk replaces the text; a deleted project closes the views', async () => {
    await createRoot(async (dispose) => {
      const core = new FakeCore();
      core.text = 'old';
      const a = windowOn(core, 'A').acquire(KEY);
      await a.ready;
      core.broadcast({ kind: 'reset', key: KEY, rev: 5, text: 'changed outside' });
      expect(a.text()).toBe('changed outside');
      expect(a.external().op).toBeNull();
      expect(a.canUndo()).toBe(false);
      core.broadcast({
        kind: 'notice',
        key: KEY,
        message: 'Changed on disk while you were editing',
      });
      expect(a.notice()).toContain('Changed on disk');
      core.broadcast({ kind: 'closed', key: KEY, reason: 'deleted' });
      expect(a.status()).toBe('closed');
      expect(a.reason()).toBe('deleted');
      // Nothing more is sent for a closed note.
      type(a, 0, 'late');
      expect(a.text()).toBe('changed outside');
      dispose();
    });
  });

  it('a megabyte note takes a burst of typing quickly', async () => {
    await createRoot(async (dispose) => {
      const core = new FakeCore();
      core.text = 'A line of a long note, with @key2020 and $x$.\n'.repeat(23_000);
      const a = windowOn(core, 'A').acquire(KEY);
      await a.ready;
      const started = performance.now();
      const at = Math.floor(core.text.length / 2);
      for (let i = 0; i < 100; i++) type(a, at + i, 'x');
      const elapsed = performance.now() - started;
      expect(elapsed).toBeLessThan(1_000);
      await settle(core, a);
      expect(core.text.length).toBe(a.text().length);
      expect(core.text.slice(at, at + 100)).toBe('x'.repeat(100));
      dispose();
    });
  });
});

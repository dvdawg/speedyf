// @vitest-environment happy-dom
/** The note editor on screen: rendered blocks, the one being edited as its
 * source, citations read through the key history (and a key nobody has
 * left as written), and the keys that split, join and undo. */
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyOp } from './ops';
import type { NoteKey, NoteTransport } from './noteSession';

const events: unknown[] = [];
vi.mock('../extensionStore', () => ({
  runEditorEvent: vi.fn(async (_id: string, _editor: unknown, value: unknown) => {
    events.push(value);
  }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('../../../lib/transport/engine', () => ({
  engine: { onLibraryChanged: async () => () => {} },
}));
vi.mock('../../../lib/links/followExternalLink', () => ({ followExternalLink: vi.fn() }));

const host = await import('../editorHost');
const { default: NoteEditor } = await import('./NoteEditor');

const KEY: NoteKey = { extensionId: 'dev.speedyf.notepad', projectId: 'p1', file: 'notepad.md' };

function core(initial: string) {
  const state = { text: initial, rev: 1 };
  const transport: NoteTransport = {
    open: async () => ({ text: state.text, rev: state.rev }),
    edit: async (_key, _base, op, origin) => {
      state.text = applyOp(state.text, op);
      state.rev += 1;
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

function mount(text: string) {
  const fake = core(text);
  host.setCitations('dev.speedyf.notepad', 'p1', [
    {
      key: 'attias2022quantum',
      label: 'Attias 2022',
      title: 'Quantum things',
      detail: 'Attias - 2022',
      pick: true,
      aliases: ['attias2022'],
    },
    {
      key: 'other2020',
      label: 'Other 2020',
      title: 'Other work',
      detail: 'Other - 2020',
      pick: false,
      aliases: [],
    },
  ]);
  const view = render(() => (
    <NoteEditor
      extensionId="dev.speedyf.notepad"
      projectId="p1"
      editor={{ file: 'notepad.md', command: 'editor' }}
    />
  ));
  return { ...fake, view };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 80));

afterEach(() => {
  cleanup();
  events.length = 0;
});

describe('the note editor', () => {
  it('renders blocks and reads citations through the key history', async () => {
    const { view } = mount(
      '# Notes\nAs @attias2022 showed [@attias2022quantum, p. 12]{box=1,2,3,4}, unlike @gone1999.'
    );
    await settle();
    const root = view.container;
    expect(root.querySelector('.np-h1')?.textContent).toBe('Notes');
    const cites = [...root.querySelectorAll('.np-cite')];
    expect(cites.map((c) => c.textContent)).toEqual([
      'Attias 2022',
      'Attias 2022, p. 12',
      '@gone1999',
    ]);
    // A key no item has stays as written, quietly, and is not rewritten.
    expect(cites[2]!.classList.contains('is-unresolved')).toBe(true);
    // The text itself is untouched.
    expect(host.noteSessions().get(KEY)!.text()).toContain('@attias2022 showed');
    // Asked the extension for its citations on open.
    expect(events[0]).toMatchObject({ event: 'citations', projectId: 'p1', file: 'notepad.md' });
  });

  it('opens a pinpoint citation through the extension, at its page and place', async () => {
    const { view } = mount('See [@attias2022quantum, p. 12]{box=1,2,3,4}.');
    await settle();
    fireEvent.click(view.container.querySelector('.np-cite')!);
    expect(events.at(-1)).toEqual({
      event: 'activate',
      projectId: 'p1',
      file: 'notepad.md',
      cite: { key: 'attias2022quantum', locator: 'p. 12', page: 11, box: [1, 2, 3, 4] },
    });
  });

  it('edits the clicked block as its source, splits on Enter and undoes', async () => {
    const { view, state } = mount('first line\nsecond');
    await settle();
    const blocks = view.container.querySelectorAll<HTMLElement>('.note-block');
    fireEvent.click(blocks[1]!);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    expect(field.value).toBe('second');
    // Type at the end.
    field.value = 'second line';
    field.setSelectionRange(11, 11);
    fireEvent.input(field);
    // Enter splits the line.
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(state.text).toBe('first line\nsecond line\n');
    const next = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    expect(next.value).toBe('');
    // Undo takes the split back, then the typing.
    fireEvent.keyDown(next, { key: 'z', metaKey: true, ctrlKey: true });
    await settle();
    expect(host.noteSessions().get(KEY)!.text()).toBe('first line\nsecond line');
  });

  it('does not split or re-cut while an input method is composing', async () => {
    const { view } = mount('abc');
    await settle();
    fireEvent.click(view.container.querySelector('.note-block')!);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    fireEvent.compositionStart(field);
    field.value = 'abcに';
    fireEvent.input(field);
    fireEvent.keyDown(field, { key: 'Enter', isComposing: true, keyCode: 229 });
    fireEvent.compositionEnd(field);
    await settle();
    expect(host.noteSessions().get(KEY)!.text()).toBe('abcに');
    expect(view.container.querySelectorAll('.note-field')).toHaveLength(1);
  });

  it('offers the project items when @ is typed and inserts the key', async () => {
    const { view } = mount('');
    await settle();
    fireEvent.click(view.container.querySelector('.note-placeholder')!);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    field.value = 'See @qua';
    field.setSelectionRange(8, 8);
    fireEvent.input(field);
    const rows = view.container.querySelectorAll('.note-picker-row');
    // Only the project's items, not every key the note can resolve.
    expect(rows).toHaveLength(1);
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(host.noteSessions().get(KEY)!.text()).toBe('See @attias2022quantum');
  });
});

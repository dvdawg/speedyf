// @vitest-environment happy-dom
/** The note editor's tools: a pasted DOI or arXiv id as a citation. */
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

function mount(text: string, placeholder?: string) {
  const fake = core(text);
  host.setCitations('dev.speedyf.notepad', 'p1', [
    {
      key: 'attias2022quantum',
      label: '[AS22]',
      title: 'Quantum things',
      detail: 'Attias - 2022',
      pick: true,
      aliases: ['attias2022'],
      doi: '10.1103/physrevlett.78.2690',
      arxiv: '2401.01234',
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
      placeholder={placeholder}
      header={<span class="note-editor-title">Notepad</span>}
    />
  ));
  return { ...fake, view };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 80));

afterEach(() => {
  cleanup();
  events.length = 0;
  vi.restoreAllMocks();
});

/** A paste of `text` into `field`; whether the platform was left to do it. */
function paste(field: HTMLTextAreaElement, text: string): boolean {
  const event = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent;
  Object.defineProperty(event, 'clipboardData', {
    value: { getData: (type: string) => (type === 'text/plain' ? text : '') },
  });
  return field.dispatchEvent(event);
}

async function editAt(view: { container: HTMLElement }, index = 0) {
  fireEvent.click(view.container.querySelectorAll('.note-block')[index]!);
  await settle();
  return view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
}

describe('pasting an identifier', () => {
  it('turns a pasted DOI an item holds into its citation, as one undoable edit', async () => {
    const { view, state } = mount('See ');
    await settle();
    const field = await editAt(view);
    field.setSelectionRange(4, 4);
    const native = paste(field, 'https://doi.org/10.1103/PhysRevLett.78.2690');
    expect(native).toBe(false);
    await settle();
    expect(state.text).toBe('See @attias2022quantum');
    expect(field.value).toBe('See @attias2022quantum');
    // Typing on is a step of its own.
    field.value = 'See @attias2022quantum.';
    field.setSelectionRange(23, 23);
    fireEvent.input(field);
    fireEvent.keyDown(field, { key: 'z', metaKey: true, ctrlKey: true });
    await settle();
    expect(host.noteSessions().get(KEY)!.text()).toBe('See @attias2022quantum');
    const again = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    fireEvent.keyDown(again, { key: 'z', metaKey: true, ctrlKey: true });
    await settle();
    expect(host.noteSessions().get(KEY)!.text()).toBe('See ');
  });

  it('reads arXiv ids and URLs, replacing the selection', async () => {
    const { view } = mount('Cite XXX here');
    await settle();
    for (const raw of ['arXiv:2401.01234v2', 'https://arxiv.org/abs/2401.01234', '2401.01234']) {
      const field = await editAt(view);
      const at = field.value.indexOf('XXX');
      field.setSelectionRange(at, at + 3);
      expect(paste(field, raw)).toBe(false);
      await settle();
      expect(host.noteSessions().get(KEY)!.text()).toBe('Cite @attias2022quantum here');
      fireEvent.keyDown(field, { key: 'z', metaKey: true, ctrlKey: true });
      await settle();
      expect(host.noteSessions().get(KEY)!.text()).toBe('Cite XXX here');
    }
  });

  it('leaves an unknown identifier and plain text to the platform', async () => {
    const { view, state } = mount('x');
    await settle();
    const field = await editAt(view);
    expect(paste(field, '10.9999/nobody')).toBe(true);
    expect(paste(field, 'arXiv:2402.00001')).toBe(true);
    expect(paste(field, 'some words')).toBe(true);
    await settle();
    expect(state.text).toBe('x');
  });

  it('replaces only the known identifiers in mixed text, in Source too', async () => {
    const { view, state } = mount('');
    await settle();
    fireEvent.click(view.container.querySelector('.note-source-toggle')!);
    await settle();
    const source = view.container.querySelector<HTMLTextAreaElement>('.note-source')!;
    source.setSelectionRange(0, 0);
    expect(paste(source, 'Both (arXiv:2401.01234) and doi:10.9999/x.')).toBe(false);
    await settle();
    expect(state.text).toBe('Both (@attias2022quantum) and doi:10.9999/x.');
  });
});

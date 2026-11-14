// @vitest-environment happy-dom
/** The note editor's tools: a pasted DOI or arXiv id as a citation, and
 * the outline of the note's headings. */
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

describe('the outline', () => {
  const NOTE = '# Intro\nwords\n## The *bound* on $x^2$\nmore\n### Detail\n#### Too deep\n# End';

  const rows = (root: HTMLElement) =>
    [...root.querySelectorAll<HTMLElement>('.note-head-menu-row')].map((r) => [
      r.textContent,
      r.style.paddingLeft,
    ]);

  const toggle = (root: HTMLElement) =>
    root.querySelector<HTMLButtonElement>('.note-outline button') ?? undefined;

  it('shows only once the note has two headings', async () => {
    const { view } = mount('# Only one\ntext');
    await settle();
    expect(toggle(view.container)).toBeUndefined();
  });

  it('lists the headings, indented by level, three levels at most, as text', async () => {
    const { view } = mount(NOTE);
    await settle();
    const button = toggle(view.container)!;
    expect(button.textContent).toBe('Outline');
    fireEvent.click(button);
    await settle();
    expect(rows(view.container)).toEqual([
      ['Intro', ''],
      ['The bound on x^2', '22px'],
      ['Detail', '36px'],
      ['End', ''],
    ]);
    const list = view.container.querySelector('.note-head-menu-list')!;
    expect(document.activeElement).toBe(list);
  });

  it('moves with the arrows, jumps on Enter with the caret at the heading', async () => {
    const { view } = mount(NOTE);
    await settle();
    fireEvent.click(toggle(view.container)!);
    await settle();
    const list = view.container.querySelector<HTMLElement>('.note-head-menu-list')!;
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    fireEvent.keyDown(list, { key: 'ArrowUp' });
    expect(list.querySelector('.is-current')?.textContent).toBe('The bound on x^2');
    fireEvent.keyDown(list, { key: 'Enter' });
    await settle();
    expect(view.container.querySelector('.note-head-menu-list')).toBeNull();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    expect(field.value).toBe('## The *bound* on $x^2$');
    expect(document.activeElement).toBe(field);
    expect(field.selectionStart).toBe(0);
  });

  it('jumps on a click, and Escape closes it back to the button', async () => {
    const { view } = mount(NOTE);
    await settle();
    const button = toggle(view.container)!;
    fireEvent.click(button);
    await settle();
    fireEvent.keyDown(view.container.querySelector('.note-head-menu-list')!, { key: 'Escape' });
    await settle();
    expect(view.container.querySelector('.note-head-menu-list')).toBeNull();
    expect(document.activeElement).toBe(button);
    fireEvent.click(button);
    await settle();
    fireEvent.click(view.container.querySelectorAll('.note-head-menu-row')[3]!);
    await settle();
    expect(view.container.querySelector<HTMLTextAreaElement>('.note-field')!.value).toBe('# End');
  });

  it('jumps in Source too', async () => {
    const { view } = mount(NOTE);
    await settle();
    fireEvent.click(view.container.querySelector('.note-source-toggle')!);
    await settle();
    fireEvent.click(toggle(view.container)!);
    await settle();
    fireEvent.click(view.container.querySelectorAll('.note-head-menu-row')[2]!);
    await settle();
    const source = view.container.querySelector<HTMLTextAreaElement>('.note-source')!;
    expect(source.selectionStart).toBe(NOTE.indexOf('### Detail'));
  });

  it('follows the note as it changes, the line being typed included', async () => {
    const { view } = mount(NOTE);
    await settle();
    const field = await editAt(view, 1);
    expect(field.value).toBe('words');
    field.value = '## Words';
    field.setSelectionRange(8, 8);
    fireEvent.input(field);
    fireEvent.click(toggle(view.container)!);
    await settle();
    expect(rows(view.container).map((r) => r[0])).toEqual([
      'Intro',
      'Words',
      'The bound on x^2',
      'Detail',
      'End',
    ]);
    // Removed by another view: the open list follows.
    const session = host.noteSessions().get(KEY)!;
    const at = session.text().indexOf('# End');
    session.edit({ from: at, to: at + 5, insert: 'end' }, at, at + 3, { outside: true });
    await settle();
    expect(rows(view.container).map((r) => r[0])).toEqual([
      'Intro',
      'Words',
      'The bound on x^2',
      'Detail',
    ]);
  });
});

describe('the Export list', () => {
  it('offers Markdown and LaTeX, and copies LaTeX inside the click', async () => {
    const writeText = vi.fn(async (_text: string) => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const { view } = mount('# Notes\nSee @attias2022 and 50%.');
    await settle();
    const button = view.container.querySelector<HTMLButtonElement>('.note-export button')!;
    expect(button.textContent).toBe('Export');
    fireEvent.click(button);
    await settle();
    const rows = [...view.container.querySelectorAll<HTMLElement>('.note-head-menu-row')];
    expect(rows.map((r) => r.textContent)).toEqual(['Markdown file...', 'Copy as LaTeX']);
    fireEvent.click(rows[1]!);
    expect(writeText).toHaveBeenCalledWith(
      '\\section{Notes}\n\nSee \\cite{attias2022quantum} and 50\\%.\n'
    );
    await settle();
    expect(view.container.querySelector('.note-editor-notice')?.textContent).toContain(
      'Copied as LaTeX'
    );
  });
});

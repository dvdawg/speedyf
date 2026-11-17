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
const { default: NoteEditor, liveMath } = await import('./NoteEditor');
const { noteDropAt } = await import('./noteDrop');
const { loadKatex } = await import('../../../components/MathText');

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

describe('the note editor', () => {
  it('renders blocks and reads citations through the key history', async () => {
    const { view } = mount(
      '# Notes\nAs @attias2022 showed [@attias2022quantum, p. 12]{box=1,2,3,4}, unlike @gone1999.'
    );
    await settle();
    const root = view.container;
    expect(root.querySelector('.np-h1')?.textContent).toBe('Notes');
    const cites = [...root.querySelectorAll('.np-cite')];
    expect(cites.map((c) => c.textContent)).toEqual(['[AS22]', '[AS22, p. 12]', '@gone1999']);
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

  it('keeps the edited line until a press elsewhere is over, then moves to the clicked line', async () => {
    const { view, state } = mount('first\nsecond\nthird');
    await settle();
    fireEvent.click(view.container.querySelectorAll<HTMLElement>('.note-block')[0]!);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    field.value = 'first, edited';
    field.setSelectionRange(13, 13);
    fireEvent.input(field);
    await settle();
    const scroll = view.container.querySelector<HTMLElement>('.note-editor-scroll')!;
    const third = view.container.querySelectorAll<HTMLElement>('.note-block')[2]!;
    // The press takes focus off the line, but the line stays as it is, so
    // nothing moves under the pointer.
    fireEvent.pointerDown(third, { button: 0 });
    fireEvent.blur(field);
    expect(view.container.querySelector('.note-field')).toBe(field);
    fireEvent.pointerUp(window);
    fireEvent.click(third);
    await settle();
    const next = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    expect(next.value).toBe('third');
    expect(document.activeElement).toBe(next);
    expect(state.text).toBe('first, edited\nsecond\nthird');
    expect(scroll.contains(next)).toBe(true);
  });

  it('renders consecutive quote lines as one quote, and continues it on Enter', async () => {
    const { view, state } = mount(
      'Before.\n\n> First paragraph.\n>\n> Second [@attias2022quantum, p. 12]{box=1,2,3,4}\n\nAfter.'
    );
    await settle();
    const root = view.container;
    const quotes = root.querySelectorAll('.np-quote');
    expect(quotes).toHaveLength(1);
    const lines = [...quotes[0]!.querySelectorAll('.np-quote-line')].map((l) => l.textContent);
    expect(lines).toEqual(['First paragraph.', 'Second [AS22, p. 12]']);
    expect(quotes[0]!.querySelectorAll('.np-quote-gap')).toHaveLength(1);
    // Clicked, the whole quote is edited as its source.
    fireEvent.click(quotes[0]!.closest('.note-block')!);
    await settle();
    const field = root.querySelector<HTMLTextAreaElement>('.note-field')!;
    expect(field.value).toBe(
      '> First paragraph.\n>\n> Second [@attias2022quantum, p. 12]{box=1,2,3,4}'
    );
    // Enter at the end of its first line continues the quote.
    field.setSelectionRange(18, 18);
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(state.text).toBe(
      'Before.\n\n> First paragraph.\n> \n>\n> Second [@attias2022quantum, p. 12]{box=1,2,3,4}\n\nAfter.'
    );
  });

  it("typesets a quote's inline math and its display lines inside the one quote", async () => {
    // Formulas are typeset as soon as they are drawn, here.
    vi.stubGlobal('IntersectionObserver', undefined);
    await loadKatex();
    try {
      const { view } = mount(
        [
          '> Let $X^m$ be a sample and $\\|P\\|_q$ its norm:',
          '>',
          '> $$ \\|P(A)\\|_{L_q} \\le 62400 d \\tag{3} $$',
          '>',
          '> [@attias2022quantum, p. 12]{box=1,2,3,4}',
        ].join('\n')
      );
      await settle();
      const quotes = view.container.querySelectorAll('.np-quote');
      expect(quotes).toHaveLength(1);
      const quote = quotes[0]!;
      const inline = [...quote.querySelectorAll('.np-math')];
      expect(inline).toHaveLength(2);
      expect(inline.every((m) => m.querySelector('.katex') !== null)).toBe(true);
      const display = quote.querySelectorAll('.np-math-block');
      expect(display).toHaveLength(1);
      expect(display[0]!.querySelector('.katex-display')).not.toBeNull();
      // Its number is typeset with it, not left as text.
      expect(display[0]!.textContent).toContain('(3)');
      expect(quote.textContent).not.toContain('$$');
      expect(quote.querySelector('.np-cite')?.textContent).toBe('[AS22, p. 12]');
    } finally {
      vi.unstubAllGlobals();
    }
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

  it('typesets math in the line being typed, just under it', async () => {
    const { view } = mount('');
    await settle();
    fireEvent.click(view.container.querySelector('.note-placeholder')!);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    field.value = 'the bound $x^2';
    fireEvent.input(field);
    // Not closed yet: nothing to typeset.
    expect(view.container.querySelector('.note-live-preview')).toBeNull();
    field.value = 'the bound $x^2$ holds';
    fireEvent.input(field);
    const preview = view.container.querySelector('.note-live-preview');
    expect(preview).not.toBeNull();
    expect(preview!.querySelector('.np-math')).not.toBeNull();
    // The line itself stays raw text.
    expect(field.value).toBe('the bound $x^2$ holds');
  });

  it('shows an empty note as a dim line, and a click anywhere writes at the end', async () => {
    const { view } = mount('', 'Notes for Thesis. Type @ to cite an item; drag items here.');
    await settle();
    const root = view.container;
    const line = root.querySelector('.note-placeholder')!;
    expect(line.textContent).toBe('Notes for Thesis. Type @ to cite an item; drag items here.');
    // In the flow of the text, not a field.
    expect(root.querySelector('textarea')).toBeNull();
    // A click on the page below it.
    fireEvent.click(root.querySelector('.note-editor-scroll')!);
    await settle();
    const field = root.querySelector<HTMLTextAreaElement>('.note-field')!;
    expect(document.activeElement).toBe(field);
    // The hint stays as the field's own until something is typed.
    expect(field.getAttribute('placeholder')).toBe(
      'Notes for Thesis. Type @ to cite an item; drag items here.'
    );
  });

  it('heads the page with its title and a quiet Source toggle', async () => {
    const { view } = mount('# Notes');
    await settle();
    const head = view.container.querySelector('.note-editor-head')!;
    expect(head.querySelector('.note-editor-title')?.textContent).toBe('Notepad');
    const toggle = head.querySelector<HTMLButtonElement>('.note-source-toggle')!;
    expect(toggle.textContent).toBe('Source');
    fireEvent.click(toggle);
    await settle();
    expect(view.container.querySelector('.note-source')).not.toBeNull();
    expect(toggle.textContent).toBe('Done');
  });

  it('takes dropped citations where they land, or at the end, and undoes them', async () => {
    const { view, state } = mount('first line\nsecond');
    await settle();
    const root = view.container;
    const blocks = root.querySelectorAll<HTMLElement>('.note-block');
    const under = vi.spyOn(document, 'elementFromPoint');

    // On the first line: no caret position from the point here, so its end.
    under.mockReturnValue(blocks[0]!);
    const target = noteDropAt(10, 10)!;
    expect(target).not.toBeNull();
    target.insertAt(10, 10, '@a; @b');
    await settle();
    expect(state.text).toBe('first line @a; @b\nsecond');

    // Below every line: the end of the note.
    under.mockReturnValue(root.querySelector('.note-editor-scroll')!);
    noteDropAt(10, 400)!.insertAt(10, 400, '@c');
    await settle();
    expect(state.text).toBe('first line @a; @b\nsecond @c');

    // One edit each, through the note's own history.
    const field = root.querySelector<HTMLTextAreaElement>('.note-field')!;
    fireEvent.keyDown(field, { key: 'z', metaKey: true, ctrlKey: true });
    await settle();
    expect(host.noteSessions().get(KEY)!.text()).toBe('first line @a; @b\nsecond');
  });

  it('is nothing to drop on once it is gone', async () => {
    const { view } = mount('x');
    await settle();
    const el = view.container.querySelector('.note-editor') as HTMLElement;
    vi.spyOn(document, 'elementFromPoint').mockReturnValue(el);
    expect(noteDropAt(1, 1)).not.toBeNull();
    cleanup();
    expect(noteDropAt(1, 1)).toBeNull();
  });
});

describe('placeholders and status lines', () => {
  const OWNER = 'dev.speedyf.notepad';
  const motion = (reduce: boolean) =>
    vi
      .spyOn(window, 'matchMedia')
      .mockImplementation(
        (query: string) =>
          ({ matches: reduce && query.includes('reduce'), media: query }) as MediaQueryList
      );

  afterEach(() => host.clearDecorationsOf(OWNER));

  it('draws a skeleton with its caption where the send will land, outside the text', async () => {
    motion(false);
    const { view, state } = mount('Intro.');
    await settle();
    await host.insertPending(
      KEY,
      { kind: 'equation', shape: null, caption: 'Equation (3), [@attias2022quantum, p. 4]' },
      'end',
      OWNER
    );
    await settle();
    const root = view.container;
    const pending = root.querySelector('.np-pending')!;
    expect(pending).not.toBeNull();
    const skeleton = pending.querySelector('.np-skeleton')!;
    expect(skeleton.classList.contains('is-equation')).toBe(true);
    expect(skeleton.classList.contains('is-animated')).toBe(true);
    expect(skeleton.querySelectorAll('.np-skeleton-bar')).toHaveLength(2);
    // The caption reads as it will once filled in.
    expect(pending.querySelector('.np-pending-caption')?.textContent).toBe(
      'Equation (3), [AS22, p. 4]'
    );
    // After the text, and not in it.
    expect(root.querySelector('.note-block')!.nextElementSibling).toBe(pending);
    expect(state.text).toBe('Intro.');
  });

  it('keeps an image placeholder in its drawn proportions, still under reduced motion', async () => {
    motion(true);
    const { view } = mount('Intro.');
    await settle();
    await host.insertPending(
      KEY,
      { kind: 'image', shape: { w: 300, h: 120 }, caption: null },
      'end',
      OWNER
    );
    await settle();
    const skeleton = view.container.querySelector<HTMLElement>('.np-skeleton')!;
    expect(skeleton.classList.contains('is-image')).toBe(true);
    expect(skeleton.classList.contains('is-animated')).toBe(false);
    expect(skeleton.style.aspectRatio).toBe('300 / 120');
    expect(view.container.querySelector('.np-pending-caption')).toBeNull();
  });

  it('shows a text placeholder only when the text is slow to come', async () => {
    motion(false);
    const { view } = mount('Intro.');
    await settle();
    const { handle } = await host.insertPending(
      KEY,
      { kind: 'text', shape: null, caption: null },
      'end',
      OWNER
    );
    await settle();
    expect(view.container.querySelector('.np-pending')).not.toBeNull();
    expect(view.container.querySelector('.np-skeleton')).toBeNull();
    await new Promise((r) => setTimeout(r, 120));
    expect(view.container.querySelectorAll('.np-skeleton-bar')).toHaveLength(3);
    await host.resolvePending(OWNER, handle, '> quoted');
    await settle();
    expect(view.container.querySelector('.np-pending')).toBeNull();
    expect(view.container.querySelector('.np-quote')?.textContent).toBe('quoted');
  });

  it('puts a dim checking line under a block while it is checked, and clears it', async () => {
    motion(false);
    const { view } = mount('Intro.');
    await settle();
    const sent = await host.insertIntoNote(KEY, '$$ a $$', 'end', OWNER);
    await settle();
    // Tracked, but nothing drawn until there is something to say.
    expect(view.container.querySelector('.np-checking')).toBeNull();
    host.markHandle(OWNER, sent.handle, 'checking arXiv source');
    await new Promise((r) => setTimeout(r, 450));
    const line = view.container.querySelector('.np-checking')!;
    expect(line.textContent).toBe('checking arXiv source...');
    expect(line.querySelector('.np-ellipsis')!.classList.contains('is-animated')).toBe(true);
    // Under the equation's block.
    expect(line.previousElementSibling?.querySelector('.np-math-block')).not.toBeNull();
    host.cancelHandle(OWNER, sent.handle);
    await settle();
    expect(view.container.querySelector('.np-checking')).toBeNull();
  });

  it('holds the ellipsis still under reduced motion', async () => {
    motion(true);
    const { view } = mount('Intro.');
    await settle();
    const sent = await host.insertIntoNote(KEY, 'x', 'end', OWNER);
    host.markHandle(OWNER, sent.handle, 'checking journal source');
    await new Promise((r) => setTimeout(r, 450));
    const ellipsis = view.container.querySelector('.np-ellipsis')!;
    expect(ellipsis.classList.contains('is-animated')).toBe(false);
  });
});

describe('liveMath', () => {
  it('previews closed inline math and display blocks, nothing else', () => {
    expect(liveMath('plain text')).toBeNull();
    expect(liveMath('costs $5 and $6')).toBeNull();
    expect(liveMath('open $x^2')).toBeNull();
    expect(liveMath('closed $x^2$ here')).toBe('line');
    expect(liveMath('$$')).toBeNull();
    expect(liveMath('$$ \\int f $$')).toBe('math');
    expect(liveMath('$$\n\\sum_i a_i')).toBe('math');
  });
});

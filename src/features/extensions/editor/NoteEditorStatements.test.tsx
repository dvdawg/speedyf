// @vitest-environment happy-dom
/** What the note knows about the papers it quotes: statement blocks drawn
 * as the paper's theorem environments, cross-references and citations read
 * as links with a preview on hover, a quoted paper's references with their
 * entries, and the `@` picker's second step, a paper's statements. */
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
const followExternalLink = vi.fn();
vi.mock('../../../lib/links/followExternalLink', () => ({
  followExternalLink: (url: string) => followExternalLink(url),
}));

const PROP = {
  kind: 'proposition',
  word: 'Proposition',
  number: '2.2',
  title: 'Moments of two block overlaps',
  page: 1,
  labelRects: [],
  region: [0, 0, 1, 1],
  text: '',
};
const EQ = { ...PROP, kind: 'equation', word: 'Equation', number: '3', title: null };
let releaseStatements: () => void = () => {};
const statementPlaces = vi.fn(
  (_item: string) =>
    new Promise((resolve) => {
      releaseStatements = () =>
        resolve([
          { statement: PROP, box: [72, 100, 468, 80] },
          { statement: EQ, box: [100, 300, 300, 30] },
        ]);
    })
);
const placePreview = vi.fn(async (..._args: unknown[]) => null);
vi.mock('../paperDocs', () => ({
  statementPlaces: (item: string) => statementPlaces(item),
  placePreview: (...args: unknown[]) => placePreview(...args),
}));

const host = await import('../editorHost');
const { default: NoteEditor } = await import('./NoteEditor');
const { previewShown, previewClose } = await import('./NotePreview');

const KEY: NoteKey = { extensionId: 'dev.speedyf.notepad', projectId: 'p1', file: 'notepad.md' };

function mount(text: string) {
  const state = { text, rev: 1 };
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
  host.setCitations('dev.speedyf.notepad', 'p1', [
    {
      key: 'rz26',
      label: '[RZ26]',
      title: 'Block overlaps',
      detail: 'R, Z - 2026',
      pick: true,
      aliases: [],
      item: 'rz-item',
    },
  ]);
  const view = render(() => (
    <NoteEditor
      extensionId="dev.speedyf.notepad"
      projectId="p1"
      editor={{ file: 'notepad.md', command: 'editor' }}
    />
  ));
  return { state, view };
}

const settle = (ms = 80) => new Promise((resolve) => setTimeout(resolve, ms));

afterEach(() => {
  cleanup();
  previewClose();
  events.length = 0;
  statementPlaces.mockClear();
  placePreview.mockClear();
  followExternalLink.mockClear();
});

const NOTE = [
  '> **Proposition 2.2** (Moments of two block overlaps). Let $P$ be as in [^Sim15] and [@rz26].',
  '>',
  '> [@rz26, p. 2]{box=72,100,468,80}',
  '',
  'By [Theorem 2.1](@rz26#p1){box=72,300,400,60} and [@rz26, Prop. 2.2]{box=72,100,468,80 page=2}.',
  '',
  '> A plain quote.',
  '',
  '[^Sim15]: [Sim15] B. Simon, Spectral theory, arXiv:1501.01234 (2015).',
].join('\n');

describe('statements and links in the note', () => {
  it('draws a statement as the theorem environment, without the quote rule', async () => {
    const { view } = mount(NOTE);
    await settle();
    const statement = view.container.querySelector('.np-statement')!;
    expect(statement).toBeTruthy();
    expect(statement.closest('.np-quote')).toBeNull();
    expect(statement.classList.contains('is-italic')).toBe(true);
    expect(statement.querySelector('.np-statement-label')!.textContent).toBe('Proposition 2.2');
    expect(statement.querySelector('.np-statement-title')!.textContent).toBe(
      ' (Moments of two block overlaps)'
    );
    // The page link last, on its own small line.
    expect(statement.querySelector('.np-statement-source')!.textContent).toBe('[RZ26, p. 2]');
    // A plain quote keeps its rule.
    expect(view.container.querySelector('.np-quote')!.textContent).toBe('A plain quote.');
  });

  it('reads a cross-reference as its words, previews its target and opens it', async () => {
    const { view } = mount(NOTE);
    await settle();
    const xref = view.container.querySelector<HTMLButtonElement>('.np-xref')!;
    expect(xref.textContent).toBe('Theorem 2.1');
    fireEvent.pointerEnter(xref);
    await settle(400);
    expect(previewShown()).toMatchObject({
      kind: 'place',
      itemId: 'rz-item',
      page: 0,
      box: [72, 300, 400, 60],
    });
    expect(placePreview).toHaveBeenCalledWith('rz-item', 0, [72, 300, 400, 60], 'theorem');
    expect(document.querySelector('[aria-label="Link preview"]')).toBeTruthy();
    fireEvent.click(xref);
    expect(events.at(-1)).toMatchObject({
      event: 'activate',
      cite: { key: 'rz26', locator: null, page: 0, box: [72, 300, 400, 60] },
    });
  });

  it('reads a statement citation with its locator and opens the statement', async () => {
    const { view } = mount(NOTE);
    await settle();
    const cites = [...view.container.querySelectorAll<HTMLButtonElement>('.np-cite')];
    const statement = cites.find((c) => c.textContent === '[RZ26, Prop. 2.2]')!;
    expect(statement).toBeTruthy();
    fireEvent.click(statement);
    expect(events.at(-1)).toMatchObject({
      cite: { key: 'rz26', locator: 'Prop. 2.2', page: 1, box: [72, 100, 468, 80] },
    });
  });

  it("shows a quoted paper's reference dim, its entry and link on hover", async () => {
    const { view } = mount(NOTE);
    await settle();
    const ref = view.container.querySelector<HTMLButtonElement>('.np-cite.is-reference')!;
    expect(ref.textContent).toBe('[Sim15]');
    fireEvent.pointerEnter(ref);
    await settle(400);
    expect(previewShown()).toEqual({
      kind: 'reference',
      label: '[Sim15]',
      text: 'B. Simon, Spectral theory, arXiv:1501.01234 (2015).',
    });
    const link = [...document.querySelectorAll<HTMLAnchorElement>('.note-preview a')].find(
      (a) => a.textContent === 'arXiv:1501.01234'
    )!;
    fireEvent.click(link);
    expect(followExternalLink).toHaveBeenCalledWith('https://arxiv.org/abs/1501.01234');
    // The entry line itself is a quiet reference entry.
    expect(view.container.querySelector('.np-ref-entry')!.textContent).toContain('[Sim15]');
  });
});

describe('citing a statement from the @ picker', () => {
  it('offers the paper statements after `@key,` and inserts the citation', async () => {
    const { view, state } = mount('');
    await settle();
    fireEvent.click(view.container.querySelector('.note-placeholder') ?? view.container);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    field.value = 'See @rz26, prop';
    field.setSelectionRange(15, 15);
    fireEvent.input(field);
    await settle(10);
    expect(view.container.querySelector('.is-statements')!.textContent).toContain(
      'Reading statements...'
    );
    expect(statementPlaces).toHaveBeenCalledWith('rz-item');
    releaseStatements();
    await settle();
    const rows = [...view.container.querySelectorAll('.is-statements .note-picker-row')];
    expect(rows.map((r) => r.querySelector('.note-picker-title')!.textContent)).toEqual([
      'Proposition 2.2',
    ]);
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(state.text).toBe('See [@rz26, Prop. 2.2]{box=72,100,468,80 page=2}');
  });

  it('offers them, nothing chosen, right after an item is picked', async () => {
    const { view, state } = mount('');
    await settle();
    fireEvent.click(view.container.querySelector('.note-placeholder') ?? view.container);
    await settle();
    const field = view.container.querySelector<HTMLTextAreaElement>('.note-field')!;
    field.value = 'See @rz';
    field.setSelectionRange(7, 7);
    fireEvent.input(field);
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle(10);
    releaseStatements();
    await settle();
    expect(state.text).toBe('See @rz26');
    const rows = [...view.container.querySelectorAll('.is-statements .note-picker-row')];
    expect(rows).toHaveLength(2);
    expect(rows.some((r) => r.classList.contains('is-current'))).toBe(false);
    fireEvent.keyDown(field, { key: 'ArrowDown' });
    fireEvent.keyDown(field, { key: 'ArrowDown' });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(state.text).toBe('See [@rz26, Eq. (3)]{box=100,300,300,30 page=2}');
  });
});

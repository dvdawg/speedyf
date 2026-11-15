// @vitest-environment happy-dom
/** The comment box: Enter keeps, Shift+Enter is a new line, Escape lets go;
 * "/" offers the QuickMarks and a pick sets the kind; "@" offers the
 * library and writes a bracketed citation; a typo fix takes the
 * replacement. */
import { cleanup, fireEvent, render, screen, within } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { buildCiteIndex } from './citations';
import type { PaperRow } from '../../types/engine';

const index = buildCiteIndex([
  {
    id: 'p1',
    title: 'Spectral gaps',
    authors: 'Jarvis',
    year: 1997,
    venue: null,
    citeKey: 'jarvis1997',
    alphaLabel: 'Jar97',
    projectIds: [],
    missing: false,
    hasIdentifier: false,
    incompleteCount: 0,
    isPaper: true,
    candidateCount: 0,
    fileCount: 1,
  } as PaperRow,
]);
vi.mock('./markupStore', () => ({ citeIndex: () => index }));
vi.mock('./mathHtml', () => ({
  loadMath: async () => {},
  mathRenderer: () => (tex: string) => `<i class="tex">${tex}</i>`,
}));

const { default: CommentEditor } = await import('./CommentEditor');

afterEach(cleanup);

function mount(mode: 'comment' | 'typo' = 'comment') {
  const onSave = vi.fn();
  const onCancel = vi.fn();
  render(() => (
    <CommentEditor
      mode={mode}
      quote="teh"
      initial={{
        body: '',
        kind: 'exposition',
        quickmark: null,
        replacement: mode === 'typo' ? '' : null,
      }}
      onSave={onSave}
      onCancel={onCancel}
    />
  ));
  return { onSave, onCancel };
}

function type(field: HTMLTextAreaElement | HTMLInputElement, value: string) {
  field.value = value;
  field.setSelectionRange(value.length, value.length);
  fireEvent.input(field);
}

describe('CommentEditor', () => {
  it('keeps the comment on Enter and starts a line on Shift+Enter', () => {
    const { onSave } = mount();
    const field = screen.getByRole('textbox', { name: 'Comment' }) as HTMLTextAreaElement;
    type(field, 'Needs a reference.');
    fireEvent.keyDown(field, { key: 'Enter', shiftKey: true });
    expect(onSave).not.toHaveBeenCalled();
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(onSave).toHaveBeenCalledWith({
      body: 'Needs a reference.',
      kind: 'exposition',
      quickmark: null,
      replacement: null,
    });
  });

  it('lets go on Escape', () => {
    const { onCancel, onSave } = mount();
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Comment' }), { key: 'Escape' });
    expect(onCancel).toHaveBeenCalled();
    expect(onSave).not.toHaveBeenCalled();
  });

  it('offers the QuickMarks on a slash and sets the kind from the pick', () => {
    const { onSave } = mount();
    const field = screen.getByRole('textbox', { name: 'Comment' }) as HTMLTextAreaElement;
    type(field, '/ga');
    const list = screen.getByRole('listbox', { name: 'QuickMarks' });
    expect(list.textContent).toContain('Gap in the argument');
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(field.value).toBe('Gap in the argument. ');
    expect(screen.queryByRole('listbox')).toBeNull();
    type(field, 'Gap in the argument. The bound fails for $n=1$.');
    expect(document.querySelector('.mk-preview')?.innerHTML).toContain('<i class="tex">n=1</i>');
    fireEvent.keyDown(field, { key: 'Enter' });
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'proof', quickmark: 'gap' })
    );
  });

  it('cites a library item from the @ picker', () => {
    mount();
    const field = screen.getByRole('textbox', { name: 'Comment' }) as HTMLTextAreaElement;
    type(field, 'Compare @spec');
    const row = within(screen.getByRole('listbox', { name: 'Cite an item' })).getByRole('option');
    expect(row.textContent).toContain('[Jar97]');
    fireEvent.mouseDown(row);
    expect(field.value).toBe('Compare [@jarvis1997]');
    expect(document.querySelector('.mk-preview .mk-cite')?.textContent).toBe('[Jar97]');
  });

  it('takes the replacement for a typo fix', () => {
    const { onSave } = mount('typo');
    const fix = screen.getByRole('textbox', { name: 'Replace with' }) as HTMLInputElement;
    expect(document.activeElement).toBe(fix);
    type(fix, 'the');
    fireEvent.keyDown(fix, { key: 'Enter' });
    expect(onSave).toHaveBeenCalledWith({
      body: '',
      kind: 'exposition',
      quickmark: 'typo',
      replacement: 'the',
    });
  });
});

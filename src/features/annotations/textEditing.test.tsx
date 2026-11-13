// @vitest-environment happy-dom
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TabContext } from '../../app/TabContext';
import Toolbar from '../../components/Toolbar';
import { addTab, createTab, removeTab, setActiveId, tabsStore } from '../../stores/tabsStore';
import AnnotationLayer from './AnnotationLayer';
import { setActiveTool, updateToolStyle } from './toolStore';
import { DEFAULT_TEXT_STYLE } from './textStyle';

beforeEach(() =>
  updateToolStyle('textbox', { ...DEFAULT_TEXT_STYLE, color: '#1a1a1a', opacity: 1 })
);

afterEach(() => {
  cleanup();
  setActiveTool('select');
  for (const tab of [...tabsStore.state.tabs]) removeTab(tab.id);
  setActiveId(null);
  vi.restoreAllMocks();
});

function mount() {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 1,
    path: '/tmp/text.pdf',
    name: 'text.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  addTab(tab);
  setActiveId(tab.id);
  const page = tab.documentStore.state.pages[0]!;
  const view = render(() => (
    <>
      <Toolbar />
      <TabContext.Provider value={tab}>
        <AnnotationLayer
          page={page}
          geom={{ widthPt: 612, heightPt: 792, rotation: 0 }}
          zoom={1}
          runs={[]}
        />
      </TabContext.Provider>
      <button>Outside the editor</button>
    </>
  ));
  const layer = view.container.querySelector('.annot-layer')!;
  vi.spyOn(layer, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 612, 792));
  setActiveTool('textbox');
  fireEvent.pointerDown(layer, { button: 0, clientX: 100, clientY: 100 });
  return {
    ...view,
    tab,
    page,
    annotation: () => tab.documentStore.state.annotations[page.id]?.[0],
  };
}

it('keeps text controls and an empty editing box available when changing its style', async () => {
  const view = mount();
  await Promise.resolve();
  const editor = view.getByRole('textbox', { name: 'Text box content' }) as HTMLTextAreaElement;
  expect(view.annotation()!.rect.w).toBeLessThanOrEqual(120);
  expect(view.annotation()!.rect.h).toBeLessThanOrEqual(24);
  const font = view.getByRole('combobox', { name: 'Text font' });
  fireEvent.pointerDown(font);
  (font as HTMLSelectElement).focus();
  fireEvent.change(font, { target: { value: 'Times-Roman' } });
  expect(view.annotation()).toBeDefined();
  expect(view.getByRole('textbox', { name: 'Text box content' })).toBe(editor);
  const size = view.getByRole('spinbutton', { name: 'Font size in points' });
  fireEvent.input(size, { target: { value: '24' } });
  fireEvent.click(view.getByRole('button', { name: 'Bold' }));
  fireEvent.click(view.getByRole('button', { name: 'Italic' }));
  fireEvent.click(view.getByRole('button', { name: 'Underline' }));
  fireEvent.input(view.getByLabelText('Text color'), { target: { value: '#2255aa' } });
  editor.focus();
  fireEvent.input(editor, { target: { value: 'Formatted text' } });
  expect(view.annotation()).toMatchObject({
    text: 'Formatted text',
    fontFamily: 'Times-Roman',
    fontSizePt: 24,
    bold: true,
    italic: true,
    underline: true,
    color: '#2255aa',
  });
  // Saving while the caret is still in the field must include the latest text.
  expect(view.tab.documentStore.buildEditPlan().pages[0]!.texts[0]).toMatchObject({
    text: 'Formatted text',
    fontFamily: 'Times-Roman',
    bold: true,
    italic: true,
    underline: true,
  });
  expect(view.annotation()!.rect.y + view.annotation()!.rect.h).toBeCloseTo(692);
  expect(view.annotation()!.rect.h).toBeGreaterThan(24);
  fireEvent.pointerDown(view.getByRole('button', { name: 'Outside the editor' }));
  expect(view.queryByRole('textbox', { name: 'Text box content' })).toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
  expect(view.tab.documentStore.state.selected).toBeNull();
  expect(view.container.querySelector('.annot-textbox.is-selected')).toBeNull();
  expect(view.container.querySelector('.annot-handle')).toBeNull();
});

it('keeps the caret, typed contents, and undo steps when formatting during typing', async () => {
  const view = mount();
  await Promise.resolve();
  const editor = view.getByRole('textbox', { name: 'Text box content' }) as HTMLTextAreaElement;
  fireEvent.input(editor, { target: { value: 'First' } });
  fireEvent.input(editor, { target: { value: 'First line' } });
  editor.setSelectionRange(2, 5);
  const bold = view.getByRole('button', { name: 'Bold' });
  expect(fireEvent.pointerDown(bold)).toBe(false);
  fireEvent.click(bold);
  expect(document.activeElement).toBe(editor);
  expect([editor.selectionStart, editor.selectionEnd]).toEqual([2, 5]);
  expect(editor.value).toBe('First line');
  fireEvent.input(editor, { target: { value: 'First line\nSecond line' } });
  expect(view.annotation()!.rect.h).toBeGreaterThan(30);
  view.tab.documentStore.undo();
  expect(editor.value).toBe('First line');
  expect(view.annotation()!.bold).toBe(true);
  view.tab.documentStore.undo();
  expect(view.annotation()!.bold).toBe(false);
  view.tab.documentStore.undo();
  expect(editor.value).toBe('');
  view.tab.documentStore.redo();
  expect(editor.value).toBe('First line');
});

it('discards an empty box only when leaving the editor and text controls', async () => {
  const view = mount();
  await Promise.resolve();
  const color = view.getByLabelText('Text color');
  fireEvent.pointerDown(color);
  (color as HTMLInputElement).focus();
  expect(view.annotation()).toBeDefined();
  fireEvent.pointerDown(view.getByRole('button', { name: 'Outside the editor' }));
  expect(view.annotation()).toBeUndefined();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
});

it('edits a previously selected box and retains a manually chosen width', async () => {
  const view = mount();
  await Promise.resolve();
  const editor = view.getByRole('textbox', { name: 'Text box content' });
  fireEvent.input(editor, { target: { value: 'Saved text' } });
  fireEvent.pointerDown(view.getByRole('button', { name: 'Outside the editor' }));
  const a = view.annotation()!;
  view.tab.documentStore.apply({
    type: 'patchAnnot',
    pageId: a.pageId,
    id: a.id,
    patch: { autoSize: false, rect: { ...a.rect, w: 180 }, fontFamily: 'Courier' },
  });
  const box = view.container.querySelector('.annot-textbox')!;
  fireEvent.pointerDown(box, { button: 0, clientX: 100, clientY: 100 });
  fireEvent.pointerUp(view.container.querySelector('.annot-layer')!, {
    clientX: 100,
    clientY: 100,
  });
  expect(view.tab.documentStore.state.selected?.annotId).toBe(a.id);
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
  fireEvent.dblClick(box);
  expect((view.getByRole('combobox', { name: 'Text font' }) as HTMLSelectElement).value).toBe(
    'Courier'
  );
  fireEvent.input(view.getByRole('spinbutton', { name: 'Font size in points' }), {
    target: { value: '36' },
  });
  expect(view.annotation()!.rect.w).toBe(180);
  expect(view.annotation()!.rect.h).toBeGreaterThan(36);
  // A temporarily empty size input cannot erase the selected font size.
  const size = view.getByRole('spinbutton', { name: 'Font size in points' });
  fireEvent.input(size, { target: { value: '' } });
  fireEvent.blur(size);
  expect(view.annotation()!.fontSizePt).toBe(36);
});

it('deselects a moved box on the next outside click without reopening formatting', async () => {
  const view = mount();
  await Promise.resolve();
  fireEvent.input(view.getByRole('textbox', { name: 'Text box content' }), {
    target: { value: 'Move me' },
  });
  const outside = view.getByRole('button', { name: 'Outside the editor' });
  fireEvent.pointerDown(outside);
  const before = { ...view.annotation()!.rect };
  const layer = view.container.querySelector('.annot-layer')!;
  fireEvent.pointerDown(view.container.querySelector('.annot-textbox')!, {
    button: 0,
    clientX: 100,
    clientY: 100,
  });
  fireEvent.pointerMove(layer, { clientX: 130, clientY: 140 });
  fireEvent.pointerUp(layer, { clientX: 130, clientY: 140 });
  expect(view.annotation()!.rect.x).toBeCloseTo(before.x + 30);
  expect(view.annotation()!.rect.y).toBeCloseTo(before.y - 40);
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
  const handle = view.container.querySelector('.handle-se')!;
  fireEvent.pointerDown(handle, { button: 0, clientX: 200, clientY: 160 });
  fireEvent.pointerUp(layer, { clientX: 230, clientY: 180 });
  expect(view.tab.documentStore.state.selected).not.toBeNull();
  expect(view.annotation()!.autoSize).toBe(false);
  fireEvent.pointerDown(outside);
  expect(view.tab.documentStore.state.selected).toBeNull();
  expect(view.container.querySelector('.annot-handle')).toBeNull();
  expect(view.annotation()!.text).toBe('Move me');
});

it('ends text editing on Escape and when switching tools or tabs', async () => {
  const view = mount();
  await Promise.resolve();
  fireEvent.input(view.getByRole('textbox', { name: 'Text box content' }), {
    target: { value: 'Keep me' },
  });
  fireEvent.keyDown(view.getByRole('textbox', { name: 'Text box content' }), { key: 'Escape' });
  expect(view.tab.documentStore.state.selected).toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
  fireEvent.dblClick(view.container.querySelector('.annot-textbox')!);
  expect(view.queryByRole('group', { name: 'Text style' })).not.toBeNull();
  setActiveTool('rect');
  expect(view.tab.documentStore.state.selected).toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
  setActiveTool('select');
  fireEvent.dblClick(view.container.querySelector('.annot-textbox')!);
  const other = createTab();
  addTab(other);
  setActiveId(other.id);
  setActiveId(view.tab.id);
  expect(view.tab.documentStore.state.selected).toBeNull();
  expect(view.queryByRole('textbox', { name: 'Text box content' })).toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
});

it('supports keyboard editing and clears selection when focus leaves the box', async () => {
  const view = mount();
  await Promise.resolve();
  fireEvent.input(view.getByRole('textbox', { name: 'Text box content' }), {
    target: { value: 'Keyboard text' },
  });
  const outside = view.getByRole('button', { name: 'Outside the editor' });
  outside.focus();
  expect(view.tab.documentStore.state.selected).toBeNull();
  const target = view.getByRole('button', { name: 'textbox annotation: Keyboard text' });
  target.focus();
  expect(view.tab.documentStore.state.selected).not.toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
  // Browsers synthesize a click when a focused button is activated with Enter.
  fireEvent.click(target, { detail: 0 });
  await Promise.resolve();
  expect(view.queryByRole('textbox', { name: 'Text box content' })).not.toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).not.toBeNull();
  const remove = view.getByRole('button', { name: 'Delete annotation' });
  fireEvent.pointerDown(remove);
  remove.focus();
  fireEvent.click(remove);
  expect(view.annotation()).toBeUndefined();
  expect(view.tab.viewport.state.editingText).toBeNull();
  expect(view.queryByRole('group', { name: 'Text style' })).toBeNull();
});

it('clears the formatting session when its page leaves the viewport', async () => {
  const view = mount();
  await Promise.resolve();
  expect(view.tab.viewport.state.editingText).not.toBeNull();
  view.unmount();
  expect(view.tab.viewport.state.editingText).toBeNull();
});

// @vitest-environment happy-dom
/**
 * The wiring between a widget on the page and the edit that reaches the file.
 *
 * `documentStore` already covers what an edit *is*, and the engine tests
 * cover what happens to it at save time. What neither can see is the part in
 * between: that a page shows its own widgets and nobody else's, that each
 * kind of field records the right edit, that an edited field covers the
 * raster's stale appearance, and that Tab, Enter and Escape move through the
 * form the way a form filler does.
 */
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FormFieldInfo } from '../../types/engine';
import type * as formStoreModule from './formStore';
import { testField as field } from './testField';

let fields: FormFieldInfo[] = [];
vi.mock('./formStore', async (importOriginal) => {
  const actual = await importOriginal<typeof formStoreModule>();
  return {
    ...actual,
    formFields: () => Promise.resolve(fields),
  };
});

import FormLayer from './FormLayer';
import { resetFormFocus } from './formStore';
import { TabContext } from '../../app/TabContext';
import { createDocumentStore } from '../document/documentStore';
import { createViewportStore } from '../../stores/viewportStore';
import type { TabRecord } from '../../stores/tabsStore';
import { updateSettings } from '../../stores/settings';

const meta = {
  docId: 1,
  path: '/tmp/form.pdf',
  name: 'form.pdf',
  pageCount: 2,
  sizes: [
    [300, 400, 0, 0, 0],
    [300, 400, 0, 0, 0],
  ] as [number, number, number, number, number][],
  estimatedSize: [300, 400] as [number, number],
};

let documentStore: ReturnType<typeof createDocumentStore>;
let viewport: ReturnType<typeof createViewportStore>;

const mount = (src: number, zoom = 1) => {
  documentStore = createDocumentStore();
  documentStore.initFromMeta(meta);
  viewport = createViewportStore();
  const tab = { id: 'tab-1', documentStore, viewport } as unknown as TabRecord;
  return render(() => (
    <TabContext.Provider value={tab}>
      <FormLayer docId={1} src={src} pageHeightPt={400} zoom={zoom} />
    </TabContext.Provider>
  ));
};

/** Let the fields resource settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

const text = (over: Partial<FormFieldInfo>) =>
  field({ kind: 'text', exportValue: '', rect: { x: 20, y: 300, w: 200, h: 20 }, ...over });

beforeEach(() => {
  fields = [];
  resetFormFocus();
  updateSettings({ highlightFields: true, invertPages: false, theme: 'light' });
});
afterEach(cleanup);

describe('FormLayer buttons', () => {
  it('shows only the widgets belonging to its own page', async () => {
    fields = [field({ index: 0, name: 'here' }), field({ index: 1, name: 'elsewhere', page: 1 })];
    const { findAllByRole } = mount(0);
    const boxes = await findAllByRole('checkbox');
    expect(boxes).toHaveLength(1);
    expect(boxes[0]!.getAttribute('aria-label')).toBe('here');
  });

  // A widget PDFium could not place has no hit area, so offering it would put
  // an invisible zero-sized target on the page.
  it('skips widgets with no rectangle', async () => {
    fields = [field({ index: 0, rect: { x: 0, y: 0, w: 0, h: 0 } })];
    const { queryAllByRole } = mount(0);
    await settle();
    expect(queryAllByRole('checkbox')).toHaveLength(0);
  });

  it('leaves read-only widgets of every kind off the page', async () => {
    fields = [
      field({ index: 0, readOnly: true }),
      text({ index: 1, name: 'locked', readOnly: true }),
    ];
    const { container } = mount(0);
    await settle();
    expect(container.querySelectorAll('.form-field')).toHaveLength(0);
  });

  it('records a click against the widget, not the field name', async () => {
    fields = [field({ index: 3 })];
    const { findByRole } = mount(0);
    fireEvent.click(await findByRole('checkbox'));
    expect(documentStore.state.formButtonEdits).toEqual({ '0:3': true });
  });

  it('toggles a checkbox back off, covering the mark the raster still draws', async () => {
    fields = [field({ index: 0, checked: true })];
    const { findByRole } = mount(0);
    const box = await findByRole('checkbox');
    expect(box.getAttribute('aria-checked')).toBe('true');
    const face = box.querySelector('.form-check-face')!;
    expect(face.classList.contains('is-covered')).toBe(false);
    fireEvent.click(box);
    expect(documentStore.state.formButtonEdits).toEqual({ '0:0': false });
    expect(face.classList.contains('is-covered')).toBe(true);
    expect(face.classList.contains('is-on')).toBe(false);
  });

  // The group's value lives on one shared parent field, so two selected
  // buttons is a state the file cannot represent — the overlay must not show
  // it even briefly.
  it('clears the rest of a radio group when one is selected', async () => {
    fields = [
      field({ kind: 'radio', name: 'choice', index: 1, exportValue: 'a', checked: true }),
      field({ kind: 'radio', name: 'choice', index: 2, exportValue: 'b' }),
    ];
    const { findAllByRole } = mount(0);
    const radios = await findAllByRole('radio');
    fireEvent.click(radios[1]!);
    expect(documentStore.state.formButtonEdits).toEqual({ '0:2': true, '0:1': false });
  });

  it('leaves a different radio group alone', async () => {
    fields = [
      field({ kind: 'radio', name: 'other', index: 1, exportValue: 'a', checked: true }),
      field({ kind: 'radio', name: 'choice', index: 2, exportValue: 'b' }),
    ];
    const { findAllByRole } = mount(0);
    fireEvent.click((await findAllByRole('radio'))[1]!);
    expect(documentStore.state.formButtonEdits).toEqual({ '0:2': true });
  });

  it('moves a radio selection with the arrow keys', async () => {
    fields = [
      field({ kind: 'radio', name: 'g', index: 1, checked: true }),
      field({ kind: 'radio', name: 'g', index: 2 }),
    ];
    const { findAllByRole } = mount(0);
    const radios = await findAllByRole('radio');
    radios[0]!.focus();
    fireEvent.keyDown(radios[0]!, { key: 'ArrowDown' });
    expect(documentStore.state.formButtonEdits).toEqual({ '0:2': true, '0:1': false });
    expect(document.activeElement).toBe(radios[1]);
  });
});

describe('FormLayer text fields', () => {
  it('draws at the field font size times the zoom, aligned as the field says', async () => {
    fields = [text({ name: 'name', fontSize: 10, align: 1, color: '#0000ff' })];
    const { container } = mount(0, 2);
    await settle();
    const box = container.querySelector<HTMLElement>('.form-text')!;
    expect(box.style.fontSize).toBe('20px');
    expect(box.style.textAlign).toBe('center');
    expect(box.style.width).toBe('400px');
    expect(box.style.getPropertyValue('--form-ink')).toBe('#0000ff');
  });

  it('shrinks an auto-sized field to fit what was typed', async () => {
    fields = [text({ name: 'name', fontSize: 0, rect: { x: 0, y: 0, w: 60, h: 20 } })];
    const { container, findByRole } = mount(0);
    const input = await findByRole('textbox');
    const box = container.querySelector<HTMLElement>('.form-text')!;
    const before = parseFloat(box.style.fontSize);
    fireEvent.input(input, { target: { value: 'A much longer answer than fits' } });
    expect(parseFloat(box.style.fontSize)).toBeLessThan(before);
  });

  it('covers the raster only once edited or focused', async () => {
    fields = [text({ name: 'name', value: 'From file' })];
    const { container, findByRole } = mount(0);
    const input = (await findByRole('textbox')) as HTMLInputElement;
    const box = container.querySelector('.form-text')!;
    expect(input.value).toBe('From file');
    expect(box.classList.contains('is-covered')).toBe(false);
    fireEvent.focus(input);
    expect(box.classList.contains('is-covered')).toBe(true);
    fireEvent.blur(input);
    expect(box.classList.contains('is-covered')).toBe(false);
    fireEvent.input(input, { target: { value: 'Typed' } });
    expect(box.classList.contains('is-covered')).toBe(true);
    expect(box.classList.contains('form-cover')).toBe(true);
  });

  it('records one undo step per visit to a field', async () => {
    fields = [text({ name: 'name' })];
    const { findByRole } = mount(0);
    const input = await findByRole('textbox');
    fireEvent.focus(input);
    fireEvent.input(input, { target: { value: 'A' } });
    fireEvent.input(input, { target: { value: 'Ad' } });
    fireEvent.input(input, { target: { value: 'Ada' } });
    expect(documentStore.buildEditPlan().form).toEqual([['name', 'Ada']]);
    documentStore.undo();
    expect(documentStore.state.formEdits).toEqual({});
  });

  it('honours max length, password and multiline flags', async () => {
    fields = [
      text({ name: 'pin', index: 0, password: true, maxLen: 4 }),
      text({ name: 'notes', index: 1, multiline: true, rect: { x: 0, y: 100, w: 200, h: 80 } }),
    ];
    const { container } = mount(0);
    await settle();
    const pin = container.querySelector<HTMLInputElement>('input.form-input')!;
    expect(pin.type).toBe('password');
    expect(pin.maxLength).toBe(4);
    expect(container.querySelector('textarea.form-input')).not.toBeNull();
  });

  it('spaces a comb field one character per cell', async () => {
    fields = [text({ name: 'ssn', comb: true, maxLen: 9, rect: { x: 0, y: 0, w: 180, h: 20 } })];
    const { container } = mount(0);
    await settle();
    const box = container.querySelector<HTMLElement>('.form-text')!;
    const size = parseFloat(box.style.fontSize);
    const spacing = parseFloat(box.style.letterSpacing);
    // Advance (0.6 em in the monospaced comb font) plus spacing is one cell.
    expect(size * 0.6 + spacing).toBeCloseTo(20, 5);
    expect(box.classList.contains('is-comb')).toBe(true);
  });

  it('marks required fields while highlighting is on', async () => {
    fields = [text({ name: 'name', required: true })];
    const { container } = mount(0);
    await settle();
    expect(container.querySelector('.form-layer')!.classList.contains('is-tinted')).toBe(true);
    expect(container.querySelector('.form-text')!.classList.contains('is-required')).toBe(true);
    updateSettings({ highlightFields: false });
    expect(container.querySelector('.form-layer')!.classList.contains('is-tinted')).toBe(false);
  });

  it('draws inverted ink on inverted pages', async () => {
    updateSettings({ theme: 'dark', invertPages: true });
    fields = [text({ name: 'name', color: '#000000' })];
    const { container } = mount(0);
    await settle();
    const ink = container
      .querySelector<HTMLElement>('.form-text')!
      .style.getPropertyValue('--form-ink');
    expect(ink).not.toBe('#000000');
  });
});

describe('FormLayer keyboard', () => {
  const two = () => [
    text({ name: 'first', index: 0, tabOrder: 0 }),
    text({ name: 'second', index: 1, tabOrder: 1, rect: { x: 20, y: 250, w: 200, h: 20 } }),
  ];

  it('Tab and Shift-Tab follow the form order, wrapping', async () => {
    fields = two();
    const { findAllByRole } = mount(0);
    const [a, b] = (await findAllByRole('textbox')) as HTMLInputElement[];
    a!.focus();
    fireEvent.keyDown(a!, { key: 'Tab' });
    expect(document.activeElement).toBe(b);
    fireEvent.keyDown(b!, { key: 'Tab' });
    expect(document.activeElement).toBe(a);
    fireEvent.keyDown(a!, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(b);
  });

  it('Enter leaves a single-line field for the next one', async () => {
    fields = two();
    const { findAllByRole } = mount(0);
    const [a, b] = await findAllByRole('textbox');
    a!.focus();
    fireEvent.keyDown(a!, { key: 'Enter' });
    expect(document.activeElement).toBe(b);
  });

  it('Escape leaves the field and goes no further', async () => {
    fields = two();
    const { findAllByRole } = mount(0);
    const [a] = await findAllByRole('textbox');
    const outer = vi.fn();
    window.addEventListener('keydown', outer);
    a!.focus();
    fireEvent.keyDown(a!, { key: 'Escape' });
    window.removeEventListener('keydown', outer);
    expect(document.activeElement).not.toBe(a);
    expect(outer).not.toHaveBeenCalled();
  });

  it('Tab to a field on a page not shown scrolls there and focuses it on arrival', async () => {
    fields = [
      text({ name: 'here', index: 0, tabOrder: 0 }),
      text({ name: 'there', index: 0, page: 1, tabOrder: 1 }),
    ];
    const { findByRole } = mount(0);
    const here = await findByRole('textbox');
    here.focus();
    fireEvent.keyDown(here, { key: 'Tab' });
    const request = viewport.takeScrollRequest();
    expect(request).toMatchObject({ kind: 'page', page: 1 });
  });
});

describe('FormLayer choice fields', () => {
  const options = [
    { label: 'California', export: 'CA' },
    { label: 'New York', export: 'NY' },
    { label: 'Texas', export: 'TX' },
  ];

  it('a combo picks an option by index', async () => {
    fields = [field({ kind: 'combo', name: 'state', options, selected: [1] })];
    const { container } = mount(0);
    await settle();
    const select = container.querySelector<HTMLSelectElement>('select.form-select')!;
    fireEvent.change(select, { target: { value: '2' } });
    expect(documentStore.state.formChoiceEdits).toEqual({ state: { selected: [2] } });
    expect(documentStore.buildEditPlan().formChoices).toEqual([{ field: 'state', selected: [2] }]);
  });

  it('an editable combo takes typed text, and an option typed out is that option', async () => {
    fields = [field({ kind: 'combo', name: 'state', options, editable: true })];
    const { findByRole } = mount(0);
    const input = await findByRole('combobox', { name: 'state' });
    fireEvent.focus(input);
    fireEvent.input(input, { target: { value: 'Oregon' } });
    expect(documentStore.state.formChoiceEdits.state).toEqual({ selected: [], text: 'Oregon' });
    fireEvent.input(input, { target: { value: 'Texas' } });
    expect(documentStore.state.formChoiceEdits.state).toEqual({ selected: [2] });
  });

  it('a list box selects one, or several with the modifier key', async () => {
    fields = [field({ kind: 'list', name: 'colors', options, multiSelect: true, selected: [0] })];
    const { findAllByRole } = mount(0);
    const rows = await findAllByRole('option');
    fireEvent.mouseDown(rows[1]!);
    expect(documentStore.state.formChoiceEdits.colors).toEqual({ selected: [1] });
    fireEvent.mouseDown(rows[2]!, { metaKey: true });
    expect(documentStore.state.formChoiceEdits.colors).toEqual({ selected: [1, 2] });
    fireEvent.mouseDown(rows[0]!, { shiftKey: true });
    expect(documentStore.state.formChoiceEdits.colors).toEqual({ selected: [0, 1, 2] });
  });

  it('a single-select list box moves with the arrow keys', async () => {
    fields = [field({ kind: 'list', name: 'size', options, selected: [0] })];
    const { findByRole } = mount(0);
    const box = await findByRole('listbox');
    fireEvent.keyDown(box, { key: 'ArrowDown' });
    expect(documentStore.state.formChoiceEdits.size).toEqual({ selected: [1] });
  });
});

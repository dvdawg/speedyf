// @vitest-environment happy-dom
/**
 * The wiring between a widget on the page and the edit that reaches the file.
 *
 * `documentStore` already covers what a button edit *is*, and the engine tests
 * cover what happens to it at save time. What neither can see is the part in
 * between: that a page shows its own widgets and nobody else's, that clicking
 * one records it by widget rather than by field name, and that selecting a
 * radio visibly clears the rest of its group instead of leaving two filled in
 * until the file is saved and reopened.
 */
import { render, fireEvent, cleanup } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FormFieldInfo } from '../../types/engine';
import type * as formStoreModule from './formStore';

const field = (over: Partial<FormFieldInfo>): FormFieldInfo => ({
  name: 'agree',
  kind: 'checkbox',
  value: '',
  page: 0,
  readOnly: false,
  index: 0,
  rect: { x: 100, y: 345, w: 14, h: 14 },
  exportValue: 'Yes',
  checked: false,
  ...over,
});

let fields: FormFieldInfo[] = [];
vi.mock('./formStore', async (importOriginal) => {
  const actual = await importOriginal<typeof formStoreModule>();
  return {
    ...actual,
    formFields: () => Promise.resolve(fields),
  };
});

import FormLayer from './FormLayer';
import { TabContext } from '../../app/TabContext';
import { createDocumentStore } from '../document/documentStore';
import type { TabRecord } from '../../stores/tabsStore';

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

const mount = (src: number) => {
  documentStore = createDocumentStore();
  documentStore.initFromMeta(meta);
  const tab = { id: 'tab-1', documentStore } as unknown as TabRecord;
  return render(() => (
    <TabContext.Provider value={tab}>
      <FormLayer docId={1} src={src} pageHeightPt={400} zoom={1} />
    </TabContext.Provider>
  ));
};

/** Let the fields resource settle. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  fields = [];
});
afterEach(cleanup);

describe('FormLayer', () => {
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

  it('leaves read-only widgets off the page', async () => {
    fields = [field({ index: 0, readOnly: true })];
    const { queryAllByRole } = mount(0);
    await settle();
    expect(queryAllByRole('checkbox')).toHaveLength(0);
  });

  it('records a click against the widget, not the field name', async () => {
    fields = [field({ index: 3 })];
    const { findByRole } = mount(0);
    fireEvent.click(await findByRole('checkbox'));
    expect(documentStore.state.formButtonEdits).toEqual({ '0:3': true });
  });

  it('toggles a checkbox back off', async () => {
    fields = [field({ index: 0, checked: true })];
    const { findByRole } = mount(0);
    const box = await findByRole('checkbox');
    expect(box.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(box);
    expect(documentStore.state.formButtonEdits).toEqual({ '0:0': false });
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
});

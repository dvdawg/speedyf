// @vitest-environment happy-dom
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TabContext } from '../../app/TabContext';
import { createTab } from '../../stores/tabsStore';
import type { FormInfo } from '../../types/engine';
import { testField as field } from '../forms/testField';
import type * as formStoreModule from '../forms/formStore';
import { settings, updateSettings } from '../../stores/settings';

let info: FormInfo = { fields: [], xfa: false };
vi.mock('../forms/formStore', async (importOriginal) => {
  const actual = await importOriginal<typeof formStoreModule>();
  return {
    ...actual,
    formInfo: () => Promise.resolve(info),
    formFields: () => Promise.resolve(info.fields),
  };
});

import FormPanel from './FormPanel';

const mount = () => {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 1,
    path: '/tmp/form.pdf',
    name: 'form.pdf',
    pageCount: 2,
    sizes: [
      [612, 792, 0, 0, 0],
      [612, 792, 0, 0, 0],
    ],
    estimatedSize: [612, 792],
  });
  const view = render(() => (
    <TabContext.Provider value={tab}>
      <FormPanel />
    </TabContext.Provider>
  ));
  return { tab, view };
};

const form = () => [
  field({
    kind: 'text',
    name: 'topmostSubform[0].Page1[0].f1_01[0]',
    tooltip: 'Your first name',
    required: true,
    tabOrder: 0,
    defaultValue: '',
    value: 'Filed',
  }),
  field({ kind: 'checkbox', name: 'agree', index: 1, tabOrder: 1, defaultChecked: true }),
  field({ kind: 'text', name: 'later', page: 1, index: 0, tabOrder: 2, required: true }),
  field({ kind: 'text', name: 'locked', index: 2, tabOrder: 3, readOnly: true }),
];

beforeEach(() => {
  info = { fields: form(), xfa: false };
  updateSettings({ highlightFields: true });
});
afterEach(cleanup);

describe('FormPanel', () => {
  it('groups fields by page and labels them by tooltip, else by name', async () => {
    const { view } = mount();
    const heads = await view.findAllByRole('heading', { level: 3 });
    expect(heads.map((h) => h.textContent)).toEqual(['Page 1', 'Page 2']);
    const rows = view.getAllByRole('listitem');
    expect(rows.map((r) => r.querySelector('.form-nav-label')!.textContent)).toEqual([
      'Your first name',
      'agree',
      'later',
    ]);
    // Read-only fields are not part of what the reader fills.
    expect(view.queryByText('locked')).toBeNull();
  });

  it('counts required fields left and follows edits', async () => {
    const { view, tab } = mount();
    expect((await view.findByText('1 required left')).textContent).toBe('1 required left');
    tab.documentStore.apply({
      type: 'setForm',
      field: 'topmostSubform[0].Page1[0].f1_01[0]',
      value: '',
    });
    expect(view.getByText('2 required left')).toBeTruthy();
    tab.documentStore.apply({ type: 'setForm', field: 'later', value: 'x' });
    tab.documentStore.apply({
      type: 'setForm',
      field: 'topmostSubform[0].Page1[0].f1_01[0]',
      value: 'A',
    });
    expect(view.getByText('3 fields')).toBeTruthy();
  });

  it('jumps to a field on a page not shown, for Back to return from', async () => {
    const { view, tab } = mount();
    const rows = await view.findAllByRole('listitem');
    fireEvent.click(rows[2]!);
    expect(tab.viewport.takeScrollRequest()).toMatchObject({ kind: 'page', page: 1 });
  });

  it('switches field highlighting', async () => {
    const { view } = mount();
    const toggle = (await view.findByLabelText('Highlight fields')) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.input(toggle, { target: { checked: false } });
    expect(settings.highlightFields).toBe(false);
  });

  it('resets every field to its default in one undo step', async () => {
    const { view, tab } = mount();
    tab.documentStore.apply({ type: 'setForm', field: 'later', value: 'typed' });
    fireEvent.click(await view.findByText('Reset form'));
    const s = tab.documentStore.state;
    expect(s.formEdits).toEqual({ 'topmostSubform[0].Page1[0].f1_01[0]': '', later: '' });
    expect(s.formButtonEdits).toEqual({ '0:1': true });
    expect(s.formEdits.locked).toBeUndefined();
    tab.documentStore.undo();
    expect(tab.documentStore.state.formEdits).toEqual({ later: 'typed' });
    expect(tab.documentStore.state.formButtonEdits).toEqual({});
  });

  it('says when a form is XFA only', async () => {
    info = { fields: [], xfa: true };
    const { view } = mount();
    expect(await view.findByText("This form uses XFA, which SpeedyF can't fill.")).toBeTruthy();
  });

  it('says plainly when there is no form', async () => {
    info = { fields: [], xfa: false };
    const { view } = mount();
    expect(
      await view.findByText('This document has no fillable form fields. Prepare form adds them.')
    ).toBeTruthy();
  });
});

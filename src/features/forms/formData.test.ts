/** Form data in and out: values collected for FDF/XFDF/JSON/CSV, and
 * imported ones applied as form edits. The file formats themselves are
 * tested in src-tauri/src/engine/forms/data.rs. */
import { describe, expect, it } from 'vitest';
import type { FormInfo } from '../../types/engine';
import { createDocumentStore, formButtonKey } from '../document/documentStore';
import { applyImported, collectData, valuesFromData } from './formData';
import { testField } from './testField';

const FIELDS = [
  testField({ name: 'name', kind: 'text', index: 0, value: 'Ada' }),
  testField({ name: 'agree', kind: 'checkbox', index: 1, exportValue: 'Yes' }),
  testField({ name: 'pick', kind: 'radio', index: 2, exportValue: 'A' }),
  testField({ name: 'pick', kind: 'radio', index: 3, exportValue: 'B', checked: true }),
  testField({
    name: 'langs',
    kind: 'list',
    index: 4,
    multiSelect: true,
    options: [
      { label: 'English', export: 'en' },
      { label: 'French', export: 'fr' },
    ],
    selected: [0],
  }),
  testField({ name: 'fixed', kind: 'text', index: 5, readOnly: true, value: 'x' }),
];

function store() {
  const s = createDocumentStore();
  s.initFromMeta({
    docId: 1,
    path: '/tmp/f.pdf',
    name: 'f.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  return s;
}

describe('collecting values', () => {
  it('takes edits over the file, buttons as export values, lists as selections', () => {
    const s = store();
    s.apply({ type: 'setForm', field: 'name', value: 'Grace' });
    s.apply({ type: 'setFormButton', page: 0, index: 1, checked: true });
    s.apply({ type: 'setFormChoice', field: 'langs', choice: { selected: [0, 1] } });
    expect(collectData(s.state, FIELDS)).toEqual([
      { name: 'name', kind: 'text', values: ['Grace'] },
      { name: 'agree', kind: 'checkbox', values: ['Yes'] },
      { name: 'pick', kind: 'radio', values: ['B'] },
      { name: 'langs', kind: 'list', values: ['en', 'fr'] },
      { name: 'fixed', kind: 'text', values: ['x'] },
    ]);
  });
});

describe('importing values', () => {
  it('maps names to edits, and says which matched nothing', () => {
    const { values, matched, unknown } = valuesFromData(FIELDS, [
      { name: 'name', values: ['Lovelace'] },
      { name: 'agree', values: ['On'] },
      { name: 'pick', values: ['A'] },
      { name: 'langs', values: ['fr;en'] },
      { name: 'fixed', values: ['changed'] },
      { name: 'nowhere', values: ['?'] },
    ]);
    expect(matched).toBe(4);
    expect(unknown).toEqual(['nowhere']);
    expect(values.texts).toEqual({ name: 'Lovelace' });
    expect(values.buttons).toEqual({
      [formButtonKey(0, 1)]: true,
      [formButtonKey(0, 2)]: true,
      [formButtonKey(0, 3)]: false,
    });
    expect(values.choices).toEqual({ langs: { selected: [0, 1] } });
  });

  it('turns a button off for Off or an empty value', () => {
    const { values } = valuesFromData(FIELDS, [{ name: 'pick', values: ['Off'] }]);
    expect(values.buttons[formButtonKey(0, 3)]).toBe(false);
  });

  it('applies as one undo step', () => {
    const s = store();
    const info: FormInfo = { fields: FIELDS, xfa: false };
    const n = applyImported({ documentStore: s }, info, [
      { name: 'name', values: ['Lovelace'] },
      { name: 'agree', values: ['Yes'] },
    ]);
    expect(n).toBe(2);
    expect(s.state.formEdits.name).toBe('Lovelace');
    expect(s.state.formButtonEdits[formButtonKey(0, 1)]).toBe(true);
    s.undo();
    expect(s.state.formEdits.name).toBeUndefined();
    expect(s.state.formButtonEdits[formButtonKey(0, 1)]).toBeUndefined();
  });
});

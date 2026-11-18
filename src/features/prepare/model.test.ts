/** Prepare form mode's model: the design read from a form, edits, Tab
 * order, snapping, and the plan the engine writes. */
import { describe, expect, it } from 'vitest';
import type { FormInfo } from '../../types/engine';
import type { PageEntry } from '../../types/model';
import { testField } from '../forms/testField';
import {
  addFields,
  blankAtPoint,
  blankUnder,
  changeOf,
  deleteFields,
  designFromForm,
  designPlan,
  duplicateFields,
  EMPTY_DESIGN,
  fieldFromProposal,
  geometricOrder,
  moveFields,
  moveInTabOrder,
  nameProblem,
  newField,
  patchFields,
  setTabOrder,
  snapToEdges,
  tabOrderOf,
  uniqueName,
  type FormDesign,
} from './model';
import { calcFor, choiceOf, formatFor, FORMAT_CHOICES } from './formats';

const page = (id: string, srcIndex: number | null): PageEntry => ({
  id,
  srcIndex,
  baseRotation: 0,
  userRotation: 0,
  widthPt: 612,
  heightPt: 792,
  cropX: 0,
  cropY: 0,
  sizeKnown: true,
});

const PAGES = [page('p1', 0), page('p2', 1)];

const INFO: FormInfo = {
  xfa: false,
  fields: [
    testField({
      name: 'name',
      kind: 'text',
      index: 0,
      tabOrder: 0,
      rect: { x: 72, y: 700, w: 200, h: 16 },
      tooltip: 'Your name',
      required: true,
      maxLen: 30,
      defaultValue: 'Ada',
    }),
    testField({ name: 'agree', kind: 'checkbox', index: 1, tabOrder: 1, defaultChecked: true }),
    testField({ name: 'go', kind: 'button', index: 2, tabOrder: 2 }),
    testField({
      name: 'state',
      kind: 'combo',
      index: 3,
      tabOrder: 3,
      page: 1,
      options: [
        { label: 'California', export: 'CA' },
        { label: 'Texas', export: 'TX' },
      ],
      defaultSelected: [1],
    }),
  ],
  behaviours: { name: { format: { kind: 'date', format: 'mm/dd/yyyy' } } },
};

describe('the design read from a form', () => {
  it('has one field per widget it can edit, keeping what the file says', () => {
    const d = designFromForm(INFO, PAGES);
    expect(d.fields.map((f) => f.name)).toEqual(['name', 'agree', 'state']);
    const name = d.fields[0]!;
    expect(name.pageId).toBe('p1');
    expect(name.tooltip).toBe('Your name');
    expect(name.required).toBe(true);
    expect(name.defaultValue).toBe('Ada');
    expect(name.format).toEqual({ kind: 'date', format: 'mm/dd/yyyy' });
    expect(name.origin?.base.maxLen).toBe(30);
    expect(d.fields[1]!.defaultValue).toBe('Yes');
    expect(d.fields[2]!.pageId).toBe('p2');
    expect(d.fields[2]!.defaultValue).toBe('TX');
    // Untouched, it changes nothing.
    expect(designPlan(d, PAGES)).toBeNull();
  });
});

describe('making fields', () => {
  it('names new fields uniquely, and groups radio buttons', () => {
    let d: FormDesign = EMPTY_DESIGN;
    const a = newField('text', 'p1', { x: 0, y: 0, w: 100, h: 18 }, d);
    d = addFields(d, [a]);
    const b = newField('text', 'p1', { x: 0, y: 30, w: 100, h: 18 }, d);
    expect(a.name).toBe('Text 1');
    expect(b.name).toBe('Text 2');
    const r1 = newField('radio', 'p1', { x: 0, y: 60, w: 12, h: 12 }, d);
    d = addFields(d, [b, r1]);
    const r2 = newField('radio', 'p1', { x: 20, y: 60, w: 12, h: 12 }, d);
    expect(r2.name).toBe(r1.name);
    expect(r2.exportValue).not.toBe(r1.exportValue);
    const date = newField('date', 'p1', { x: 0, y: 90, w: 80, h: 18 }, d, 'dd/mm/yyyy');
    expect(date.kind).toBe('text');
    expect(date.format).toEqual({ kind: 'date', format: 'dd/mm/yyyy' });
    expect(newField('multiline', 'p1', { x: 0, y: 0, w: 9, h: 9 }, d).multiline).toBe(true);
    expect(newField('combo', 'p1', { x: 0, y: 0, w: 9, h: 9 }, d).options).toHaveLength(2);
  });

  it('keeps names free of clashes', () => {
    expect(uniqueName('Name', new Set(['Name', 'Name 2']))).toBe('Name 3');
    expect(uniqueName('a.b', new Set())).toBe('a b');
    const d = designFromForm(INFO, PAGES);
    const id = d.fields[0]!.id;
    expect(nameProblem(d, id, 'agree')).toMatch(/Another field/);
    expect(nameProblem(d, id, '')).toMatch(/needs a name/);
    expect(nameProblem(d, id, 'x.y')).toMatch(/full stop/);
    expect(nameProblem(d, id, 'Full name')).toBeNull();
  });

  it('moves, patches, duplicates and deletes', () => {
    const d = designFromForm(INFO, PAGES);
    const [name, agree] = d.fields;
    const moved = moveFields(d, [name!.id], 5, -5);
    expect(moved.fields[0]!.rect).toMatchObject({ x: 77, y: 695 });
    expect(d.fields[0]!.rect.x).toBe(72);
    const patched = patchFields(moved, [name!.id], () => ({ tooltip: 'Legal name' }));
    expect(patched.fields[0]!.tooltip).toBe('Legal name');
    const dup = duplicateFields(patched, [name!.id]);
    expect(dup.ids).toHaveLength(1);
    const copy = dup.design.fields.find((f) => f.id === dup.ids[0])!;
    expect(copy.name).toBe('name 2');
    expect(copy.origin).toBeUndefined();
    const gone = deleteFields(dup.design, [agree!.id, copy.id]);
    expect(gone.fields.map((f) => f.name)).toEqual(['name', 'state']);
    // Only a widget from the file is remembered for deletion.
    expect(gone.deleted).toEqual([{ src: 0, name: 'agree', rect: agree!.rect }]);
  });
});

describe('Tab order', () => {
  const grid = () => {
    let d: FormDesign = EMPTY_DESIGN;
    const at = (x: number, y: number) => newField('text', 'p1', { x, y, w: 80, h: 16 }, d);
    const a = at(300, 700);
    d = addFields(d, [a]);
    const b = at(50, 702);
    d = addFields(d, [b]);
    const c = at(50, 600);
    d = addFields(d, [c]);
    return { d, a, b, c };
  };

  it('orders by rows and by columns', () => {
    const { d, a, b, c } = grid();
    expect(geometricOrder(d.fields, 'R')).toEqual([b.id, a.id, c.id]);
    expect(geometricOrder(d.fields, 'C')).toEqual([b.id, c.id, a.id]);
    const rows = setTabOrder(d, 'p1', 'R');
    expect(tabOrderOf(rows, 'p1')).toEqual([b.id, a.id, c.id]);
  });

  it('moves one field by hand, and new fields come last', () => {
    const { d, a, b, c } = grid();
    const moved = moveInTabOrder(d, 'p1', c.id, 0);
    expect(tabOrderOf(moved, 'p1')).toEqual([c.id, a.id, b.id]);
    expect(moved.tabs.p1!.mode).toBe('S');
    const extra = newField('text', 'p1', { x: 0, y: 0, w: 10, h: 10 }, moved);
    expect(tabOrderOf(addFields(moved, [extra]), 'p1')).toEqual([c.id, a.id, b.id, extra.id]);
  });
});

describe('the plan', () => {
  it('sends new fields whole, changed ones with what changed, and nothing else', () => {
    let d = designFromForm(INFO, PAGES);
    const [name, agree, state] = d.fields;
    d = patchFields(d, [name!.id], () => ({ tooltip: 'Legal name' }));
    d = patchFields(d, [agree!.id], (f) => ({ rect: { ...f.rect, w: 20 } }));
    const made = newField('text', 'p2', { x: 10, y: 10, w: 100, h: 18 }, d);
    d = addFields(d, [made]);
    const plan = designPlan(d, PAGES)!;
    expect(plan.fields.map((f) => f.name)).toEqual(['name', 'agree', 'Text 1']);
    const [pName, pAgree, pMade] = plan.fields;
    expect(pName!.origin).toEqual({ page: 0, name: 'name', rect: name!.rect });
    expect(pName).toMatchObject({ restyle: false, formatSet: false, calcSet: false });
    expect(pAgree).toMatchObject({ restyle: true });
    expect(pMade).toMatchObject({ page: 1, restyle: true, formatSet: true });
    expect(pMade!.origin).toBeUndefined();
    expect(changeOf(state!).changed).toBe(false);
  });

  it('carries deletions, Tab orders by reference, and flatten', () => {
    let d = designFromForm(INFO, PAGES);
    const [name, agree] = d.fields;
    d = deleteFields(d, [agree!.id]);
    const made = newField('text', 'p1', { x: 10, y: 10, w: 100, h: 18 }, d);
    d = addFields(d, [made]);
    d = moveInTabOrder(d, 'p1', made.id, 0);
    d = { ...d, flatten: true };
    const plan = designPlan(d, PAGES)!;
    expect(plan.deleted).toEqual([{ page: 0, name: 'agree', rect: agree!.rect }]);
    expect(plan.tabOrders).toEqual([
      {
        page: 0,
        mode: 'S',
        order: [{ key: made.id }, { widget: { page: 0, name: 'name', rect: name!.rect } }],
      },
    ]);
    expect(plan.flatten).toBe(true);
    // A deleted page takes its fields with it.
    expect(designPlan(d, [page('p2', 1)])!.fields).toHaveLength(0);
  });
});

describe('snapping and proposals', () => {
  const blanks = [
    { rect: { x: 100, y: 500, w: 200, h: 14 }, kind: 'line' },
    { rect: { x: 100, y: 450, w: 10, h: 10 }, kind: 'check' },
  ];

  it('finds the blank a drag or a click means', () => {
    expect(blankUnder({ x: 98, y: 499, w: 205, h: 16 }, blanks)?.kind).toBe('line');
    expect(blankUnder({ x: 0, y: 0, w: 50, h: 50 }, blanks)).toBeNull();
    expect(blankAtPoint(105, 455, blanks)?.kind).toBe('check');
    expect(blankAtPoint(5, 5, blanks)).toBeNull();
  });

  it('snaps near edges onto a blank', () => {
    const r = snapToEdges({ x: 102, y: 498, w: 50, h: 14 }, blanks);
    expect(r).toMatchObject({ x: 100, y: 500 });
    expect(snapToEdges({ x: 140, y: 300, w: 50, h: 14 }, blanks)).toMatchObject({ x: 140, y: 300 });
  });

  it('makes a field from a proposal, named uniquely', () => {
    const taken = new Set(['Name']);
    const f = fieldFromProposal(
      {
        rect: { x: 1, y: 2, w: 90, h: 14 },
        kind: 'text',
        name: 'Name',
        label: 'Name (as shown on your return)',
        fontSize: 9,
        maxLen: 9,
        comb: true,
        multiline: false,
      },
      'p1',
      taken
    );
    expect(f.name).toBe('Name 2');
    expect(f.tooltip).toBe('Name (as shown on your return)');
    expect(f).toMatchObject({ comb: true, maxLen: 9, fontSize: 9, border: false });
    expect(taken.has('Name 2')).toBe(true);
  });
});

describe('formats', () => {
  it('round-trips every choice', () => {
    for (const { value } of FORMAT_CHOICES) {
      expect(choiceOf(formatFor(value))).toBe(value);
    }
    expect(formatFor('date', 'yyyy-mm-dd')).toEqual({ kind: 'date', format: 'yyyy-mm-dd' });
    expect(calcFor('none', ['a'])).toBeNull();
    expect(calcFor('SUM', ['a', 'b'])).toEqual({ op: 'SUM', fields: ['a', 'b'] });
  });
});

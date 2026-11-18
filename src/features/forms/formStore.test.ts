// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from 'vitest';
import { createDocumentStore } from '../document/documentStore';
import {
  fieldLabel,
  focusWidget,
  nextStop,
  registerWidget,
  requiredCount,
  requiredLeft,
  resetFormFocus,
  tabStops,
} from './formStore';
import { autoFontSize, combSpacing, fieldFontSize, wrappedLines } from './fieldText';
import { testField as field } from './testField';

const store = () => {
  const s = createDocumentStore();
  s.initFromMeta({
    docId: 1,
    path: '/tmp/f.pdf',
    name: 'f.pdf',
    pageCount: 2,
    sizes: [
      [300, 400, 0, 0, 0],
      [300, 400, 0, 0, 0],
    ],
    estimatedSize: [300, 400],
  });
  return s;
};

beforeEach(resetFormFocus);

describe('form order', () => {
  const radios = [
    field({ kind: 'radio', name: 'g', index: 2, tabOrder: 2 }),
    field({ kind: 'radio', name: 'g', index: 3, tabOrder: 3 }),
  ];
  const form = [
    field({ kind: 'text', name: 'b', index: 1, tabOrder: 1 }),
    field({ kind: 'text', name: 'a', index: 0, tabOrder: 0 }),
    ...radios,
    field({ kind: 'text', name: 'ro', index: 4, tabOrder: 4, readOnly: true }),
    field({ kind: 'signature', name: 'sig', index: 5, tabOrder: 5 }),
    field({ kind: 'text', name: 'p2', page: 1, index: 0, tabOrder: 6 }),
  ];

  it('visits fillable fields by tab order, a radio group once', () => {
    expect(tabStops(form).map((f) => f.name)).toEqual(['a', 'b', 'g', 'p2']);
  });

  it('wraps at both ends and treats any radio of a group as its stop', () => {
    expect(nextStop(form, form[1]!, 1)!.name).toBe('b');
    expect(nextStop(form, radios[1]!, 1)!.name).toBe('p2');
    expect(nextStop(form, form[6]!, 1)!.name).toBe('a');
    expect(nextStop(form, form[1]!, -1)!.name).toBe('p2');
  });
});

describe('required fields', () => {
  it('counts each unanswered required field once, by name', () => {
    const s = store();
    const form = [
      field({ kind: 'text', name: 'name', required: true }),
      field({ kind: 'text', name: 'name', required: true, page: 1 }),
      field({ kind: 'radio', name: 'g', index: 1, required: true }),
      field({ kind: 'radio', name: 'g', index: 2, required: true }),
      field({
        kind: 'combo',
        name: 'c',
        index: 3,
        required: true,
        options: [{ label: 'X', export: 'x' }],
      }),
      field({ kind: 'text', name: 'optional', index: 4 }),
      field({ kind: 'text', name: 'locked', index: 5, required: true, readOnly: true }),
    ];
    expect(requiredCount(form)).toBe(3);
    expect(requiredLeft(s.state, form)).toBe(3);
    s.apply({ type: 'setForm', field: 'name', value: 'Ada' });
    s.apply({ type: 'setFormButton', page: 0, index: 2, checked: true });
    expect(requiredLeft(s.state, form)).toBe(1);
    s.apply({ type: 'setFormChoice', field: 'c', choice: { selected: [0] } });
    expect(requiredLeft(s.state, form)).toBe(0);
  });
});

describe('labels', () => {
  it('prefers the tooltip, else the last part of the name', () => {
    expect(fieldLabel(field({ tooltip: 'Your name', name: 'a.b' }))).toBe('Your name');
    expect(fieldLabel(field({ name: 'topmostSubform[0].Page1[0].f1_01[0]' }))).toBe('f1_01');
  });
});

describe('focus across pages', () => {
  it('focuses a widget at once when mounted, or when it mounts', () => {
    const target = field({ page: 1, index: 4 });
    expect(focusWidget(1, target)).toBe(false);
    const el = document.createElement('input');
    document.body.append(el);
    const off = registerWidget(1, 1, 4, el);
    return Promise.resolve().then(() => {
      expect(document.activeElement).toBe(el);
      expect(focusWidget(1, target)).toBe(true);
      off();
      el.remove();
    });
  });
});

describe('field text layout', () => {
  const measure = (text: string, size: number) => text.length * size * 0.5;

  it('auto-size grows with the box and shrinks to fit the text', () => {
    const short = autoFontSize(
      'Hi',
      { rect: { x: 0, y: 0, w: 200, h: 20 }, multiline: false },
      measure
    );
    const long = autoFontSize(
      'x'.repeat(80),
      { rect: { x: 0, y: 0, w: 200, h: 20 }, multiline: false },
      measure
    );
    expect(short).toBe(12);
    expect(long).toBeLessThan(short);
  });

  it('caps multiline auto-size at 12pt, as PDFium does', () => {
    expect(
      autoFontSize('', { rect: { x: 0, y: 0, w: 200, h: 300 }, multiline: true }, measure)
    ).toBe(12);
  });

  it('wraps words, and breaks a word wider than the box', () => {
    expect(wrappedLines('aa bb cc', 10, 30, measure)).toBe(2);
    expect(wrappedLines('aaaaaaaaaa', 10, 20, measure)).toBe(3);
    expect(wrappedLines('one\ntwo', 10, 200, measure)).toBe(2);
  });

  it('honours an explicit size, and fits a comb to its cells', () => {
    const base = { rect: { x: 0, y: 0, w: 90, h: 20 }, multiline: false, comb: false, maxLen: 0 };
    expect(fieldFontSize('', { ...base, fontSize: 9 })).toBe(9);
    const comb = { ...base, comb: true, maxLen: 9, fontSize: 0 };
    const size = fieldFontSize('', comb);
    const { letterSpacing, inset } = combSpacing(comb, size);
    expect(size * 0.6 + letterSpacing).toBeCloseTo(10);
    expect(inset).toBeCloseTo(letterSpacing / 2);
  });
});

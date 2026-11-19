// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { isEditableTarget, SEPARATOR, tidyMenu, type MenuEntry } from './nativeMenu';

const item = (text: string): MenuEntry => ({ text, run: () => undefined });

describe('tidyMenu', () => {
  it('drops separators that would separate nothing', () => {
    const tidy = tidyMenu([SEPARATOR, item('a'), SEPARATOR, SEPARATOR, item('b'), SEPARATOR]);
    expect(tidy.map((e) => ('text' in e ? e.text : e.kind))).toEqual(['a', 'separator', 'b']);
  });

  it('drops an empty submenu and the separator it leaves behind', () => {
    const tidy = tidyMenu([
      item('a'),
      SEPARATOR,
      { kind: 'submenu', text: 'More', items: [SEPARATOR] },
    ]);
    expect(tidy).toHaveLength(1);
  });
});

describe('isEditableTarget', () => {
  it('keeps the system text menu for text fields only', () => {
    const text = document.createElement('input');
    const box = document.createElement('input');
    box.type = 'checkbox';
    const area = document.createElement('textarea');
    const div = document.createElement('div');
    const span = document.createElement('span');
    div.append(span);
    expect(isEditableTarget(text)).toBe(true);
    expect(isEditableTarget(area)).toBe(true);
    expect(isEditableTarget(box)).toBe(false);
    expect(isEditableTarget(span)).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
  });
});

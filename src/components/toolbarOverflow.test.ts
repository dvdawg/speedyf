import { describe, expect, it } from 'vitest';
import { foldedGroups, splitShortcut, type OverflowGroup } from './toolbarOverflow';

const layout = (available: number) => ({ available, gap: 10, buttonWidth: 28 });
const groups: OverflowGroup[] = [
  { id: 'sidebar', width: 30, priority: Infinity },
  { id: 'file', width: 150, priority: 1 },
  { id: 'history', width: 60, priority: 2 },
  { id: 'navigate', width: 190, priority: 6 },
  { id: 'zoom', width: 200, priority: 5 },
  { id: 'tools', width: 270, priority: 3 },
  { id: 'panels', width: 200, priority: 4 },
];
// 1100 of groups and 6 gaps of 10.
const full = 1160;

describe('foldedGroups', () => {
  it('folds nothing while everything fits', () => {
    expect(foldedGroups(groups, layout(full))).toEqual([]);
    expect(foldedGroups(groups, layout(full + 200))).toEqual([]);
  });

  it('folds the lowest priority first, leaving room for the button', () => {
    // One pixel short is enough to fold the least important group.
    expect(foldedGroups(groups, layout(full - 1))).toEqual(['file']);
    // The button and its gap take 38: without file the row is 1000 wide.
    expect(foldedGroups(groups, layout(1038))).toEqual(['file']);
    expect(foldedGroups(groups, layout(1037))).toEqual(['file', 'history']);
    expect(foldedGroups(groups, layout(700))).toEqual(['file', 'history', 'tools']);
  });

  it('answers folded groups in toolbar order, whatever their priority', () => {
    expect(foldedGroups(groups, layout(420))).toEqual([
      'file',
      'history',
      'zoom',
      'tools',
      'panels',
    ]);
  });

  it('never folds a pinned group, even when nothing else is left', () => {
    expect(foldedGroups(groups, layout(10))).toEqual([
      'file',
      'history',
      'navigate',
      'zoom',
      'tools',
      'panels',
    ]);
  });

  it('folds the later of two equal priorities first', () => {
    const twins: OverflowGroup[] = [
      { id: 'a', width: 100, priority: 1 },
      { id: 'b', width: 100, priority: 1 },
    ];
    expect(foldedGroups(twins, layout(180))).toEqual(['b']);
  });
});

describe('splitShortcut', () => {
  it('sets a shortcut apart from the name', () => {
    expect(splitShortcut('Save (Cmd+S)')).toEqual({ name: 'Save', key: 'Cmd+S' });
    expect(splitShortcut('Previous page (Page Up)')).toEqual({
      name: 'Previous page',
      key: 'Page Up',
    });
    expect(splitShortcut('Select (Esc)')).toEqual({ name: 'Select', key: 'Esc' });
  });

  it('keeps what follows the shortcut in the name', () => {
    expect(splitShortcut('Print (Ctrl+P) - not allowed')).toEqual({
      name: 'Print - not allowed',
      key: 'Ctrl+P',
    });
  });

  it('leaves a parenthesis that is not a key alone', () => {
    const label = 'Edit mode (page reorder, delete, rotate)';
    expect(splitShortcut(label)).toEqual({ name: label, key: null });
    expect(splitShortcut('Fit page')).toEqual({ name: 'Fit page', key: null });
  });
});

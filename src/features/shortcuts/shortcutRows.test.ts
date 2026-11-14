import { describe, expect, it } from 'vitest';
import { filterShortcuts, shortcutSections, type ShortcutSources } from './shortcutRows';

const sources: ShortcutSources = {
  commands: [
    { id: 'file.save', title: 'Save', group: 'File', shortcut: 'mod+s' },
    { id: 'view.zoomIn', title: 'Zoom In', group: 'View', shortcut: 'mod+plus' },
    { id: 'view.fitWidth', title: 'Fit Width', group: 'View' },
    { id: 'nav.back', title: 'Go Back', group: 'Navigate', shortcut: 'mod+[' },
    { id: 'library.copyCiteKey', title: 'Copy Cite Key', group: 'Library' },
  ],
  bound: [
    { id: 'file.save', spec: 'mod+s' },
    { id: 'view.zoomIn', spec: 'mod+=' },
    { id: 'view.zoomIn', spec: 'mod+plus' },
    { id: 'nav.back', spec: 'alt+arrowleft' },
    { id: 'library.copyCiteKey', spec: 'mod+shift+c' },
  ],
  unlisted: [{ spec: 'escape', title: 'Close', area: 'View' }],
  extensions: [{ spec: 'mod+shift+e', title: 'Quote', extension: 'Notepad' }],
};

describe('shortcutSections', () => {
  it('groups by area in order, merging a command shortcut with its other chords', () => {
    const sections = shortcutSections(sources, true);
    expect(sections.map((s) => s.area)).toEqual([
      'Files',
      'Navigation',
      'View',
      'Library',
      'Extensions',
    ]);
    const view = sections.find((s) => s.area === 'View')!;
    expect(view.rows).toEqual([
      { title: 'Close', keys: ['Escape'] },
      { title: 'Zoom In', keys: ['Cmd+Plus', 'Cmd+='] },
    ]);
    expect(sections.find((s) => s.area === 'Navigation')!.rows[0]!.keys).toEqual([
      'Cmd+[',
      'Option+Left',
    ]);
    // A chord only the keymap knows still lists its command.
    expect(sections.find((s) => s.area === 'Library')!.rows[0]).toEqual({
      title: 'Copy Cite Key',
      keys: ['Cmd+Shift+C'],
    });
  });

  it('names keys for the platform, in ASCII', () => {
    const files = shortcutSections(sources, false).find((s) => s.area === 'Files')!;
    expect(files.rows[0]!.keys).toEqual(['Ctrl+S']);
    const extensions = shortcutSections(sources, false).find((s) => s.area === 'Extensions')!;
    expect(extensions.rows[0]).toEqual({
      title: 'Quote',
      keys: ['Ctrl+Shift+E'],
      extension: 'Notepad',
    });
  });
});

describe('filterShortcuts', () => {
  const sections = shortcutSections(sources, true);

  it('keeps rows matching every word, across title, keys and extension', () => {
    expect(filterShortcuts(sections, 'zoom').flatMap((s) => s.rows.map((r) => r.title))).toEqual([
      'Zoom In',
    ]);
    expect(filterShortcuts(sections, 'notepad')[0]!.area).toBe('Extensions');
    expect(
      filterShortcuts(sections, 'cmd+shift').flatMap((s) => s.rows.map((r) => r.title))
    ).toEqual(['Copy Cite Key', 'Quote']);
    expect(filterShortcuts(sections, 'zoom save')).toEqual([]);
  });

  it('keeps everything for an empty filter', () => {
    expect(filterShortcuts(sections, '  ')).toEqual(sections);
  });
});

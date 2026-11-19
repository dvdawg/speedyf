// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { recentMenuEntries, RECENT_MENU_LIMIT } from './recentMenu';
import { isBrowserKey } from './webviewGuards';
import { annotationMenuEntries } from '../features/annotations/annotationMenu';
import { highlightsFrom } from '../features/viewer/pageMenu';
import { revealLabel } from '../features/home/fileMenu';
import type { Annotation } from '../types/model';
import type { MenuEntry } from '../lib/nativeMenu';

const key = (k: string, mods: Partial<KeyboardEvent> = {}) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe('webview guards', () => {
  it('refuses reload and interface zoom, on each platform own modifier', () => {
    expect(isBrowserKey(key('F5'), true)).toBe(true);
    expect(isBrowserKey(key('r', { metaKey: true }), true)).toBe(true);
    expect(isBrowserKey(key('R', { ctrlKey: true, shiftKey: true }), false)).toBe(true);
    expect(isBrowserKey(key('=', { metaKey: true }), true)).toBe(true);
    expect(isBrowserKey(key('-', { ctrlKey: true }), false)).toBe(true);
  });

  it('lets ordinary typing and word movement through', () => {
    expect(isBrowserKey(key('r'), true)).toBe(false);
    expect(isBrowserKey(key('r', { ctrlKey: true }), true)).toBe(false);
    expect(isBrowserKey(key('ArrowLeft', { altKey: true }), true)).toBe(false);
    expect(isBrowserKey(key('ArrowLeft', { altKey: true }), false)).toBe(true);
  });
});

describe('Open Recent', () => {
  const entry = (path: string) => ({
    path,
    name: path.split('/').pop()!,
    lastOpened: 0,
    sizeBytes: null,
  });

  it('names files, telling two of one name apart by folder', () => {
    const entries = recentMenuEntries([
      entry('/papers/elsevier/main.pdf'),
      entry('/papers/springer/main.pdf'),
      entry('/papers/solo.pdf'),
    ]);
    expect(entries.map((e) => e.text)).toEqual([
      'main.pdf - elsevier',
      'main.pdf - springer',
      'solo.pdf',
    ]);
  });

  it('shows at most the menu limit', () => {
    const many = Array.from({ length: 15 }, (_, i) => entry(`/p/${i}.pdf`));
    expect(recentMenuEntries(many)).toHaveLength(RECENT_MENU_LIMIT);
  });
});

describe('annotation menu', () => {
  const annot = (kind: Annotation['kind'], color = '#ffd54a') =>
    ({
      id: 'a',
      pageId: 'p',
      kind,
      rect: { x: 0, y: 0, w: 1, h: 1 },
      color,
      opacity: 1,
    }) as Annotation;
  const actions = {
    editable: true,
    editText: vi.fn(),
    recolour: vi.fn(),
    remove: vi.fn(),
  };
  const texts = (entries: MenuEntry[]) =>
    entries.filter((e) => 'text' in e).map((e) => ('text' in e ? e.text : ''));

  it('offers editing for text, colour for marks, delete for all', () => {
    expect(texts(annotationMenuEntries(annot('textbox'), actions))).toEqual([
      'Edit Text',
      'Colour',
      'Delete',
    ]);
    expect(texts(annotationMenuEntries(annot('highlight'), actions))).toEqual(['Colour', 'Delete']);
    expect(texts(annotationMenuEntries(annot('signature'), actions))).toEqual(['Delete']);
  });

  it('checks the current colour and recolours through the action', () => {
    const colour = annotationMenuEntries(annot('highlight'), actions).find(
      (e) => e.kind === 'submenu'
    );
    if (colour?.kind !== 'submenu') throw new Error('no colour menu');
    const yellow = colour.items[0]!;
    const red = colour.items[2]!;
    expect('checked' in yellow && yellow.checked).toBe(true);
    if ('run' in red) red.run();
    expect(actions.recolour).toHaveBeenCalledWith('#e53935');
  });

  it('disables everything in a document that refuses annotation changes', () => {
    const entries = annotationMenuEntries(annot('textbox'), { ...actions, editable: false });
    for (const e of entries) if ('enabled' in e) expect(e.enabled).toBe(false);
  });
});

describe('highlight from a selection', () => {
  it('makes one highlight per page, a quad per run', () => {
    const marks = highlightsFrom([
      { page: 0, rect: [10, 100, 50, 12] },
      { page: 0, rect: [10, 86, 80, 12] },
      { page: 1, rect: [20, 700, 30, 10] },
      { page: 1, rect: [0, 0, 0, 10] },
    ]);
    expect(marks).toHaveLength(2);
    expect(marks[0]!.rect).toEqual({ x: 10, y: 86, w: 80, h: 26 });
    expect(marks[0]!.quads).toHaveLength(2);
    expect(marks[0]!.quads[0]).toEqual({
      p1: { x: 10, y: 100 },
      p2: { x: 60, y: 100 },
      p3: { x: 60, y: 112 },
      p4: { x: 10, y: 112 },
    });
    expect(marks[1]!.quads).toHaveLength(1);
  });
});

describe('file rows', () => {
  it('name the file manager of the platform', () => {
    expect(revealLabel(true, false)).toBe('Show in Finder');
    expect(revealLabel(false, true)).toBe('Show in Explorer');
    expect(revealLabel(false, false)).toBe('Open Containing Folder');
  });
});

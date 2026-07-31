import { describe, expect, it } from 'vitest';
import { createDocumentStore } from './documentStore';
import type { Annotation, DocMeta } from '../../types/model';

const meta: DocMeta = {
  docId: 1,
  path: '/tmp/sample.pdf',
  name: 'sample.pdf',
  pageCount: 3,
  sizes: [
    [600, 800, 0, 0, 0],
    [595, 842, 0, 0, 0],
    [612, 792, 0, 0, 90],
  ],
  estimatedSize: [600, 800],
};

const fresh = (opts?: { historyLimit?: number }) => {
  const store = createDocumentStore(opts);
  store.initFromMeta(meta);
  return store;
};

const annot = (pageId: string, over: Partial<Annotation> = {}): Annotation => ({
  id: over.id ?? 'a-test-1',
  pageId,
  kind: 'rect',
  rect: { x: 10, y: 10, w: 100, h: 50 },
  color: '#ff0000',
  opacity: 1,
  strokeWidth: 2,
  ...over,
});

describe('initFromMeta', () => {
  it('creates stable page entries with source indexes and base rotations', () => {
    const s = fresh();
    expect(s.state.pages).toHaveLength(3);
    expect(s.state.pages.map((p) => p.srcIndex)).toEqual([0, 1, 2]);
    expect(new Set(s.state.pages.map((p) => p.id)).size).toBe(3);
    expect(s.state.pages[2]!.baseRotation).toBe(90);
    expect(s.state.dirty).toBe(false);
  });

  it('marks pages beyond the provided sizes as estimated', () => {
    const bigMeta: DocMeta = { ...meta, pageCount: 5 };
    const s = createDocumentStore();
    s.initFromMeta(bigMeta);
    expect(s.state.pages).toHaveLength(5);
    expect(s.state.pages[4]!.sizeKnown).toBe(false);
    expect(s.state.pages[4]!.widthPt).toBe(600);
  });

  it('tracks the engine generation independently of edit history', () => {
    const s = fresh();
    s.setGeneration(7);
    expect(s.state.generation).toBe(7);
    expect(s.state.dirty).toBe(false);
    expect(s.state.historyDepth).toBe(0);
  });
});

describe('page operations', () => {
  it('reorders pages by stable id', () => {
    const s = fresh();
    const first = s.state.pages[0]!.id;
    s.apply({ type: 'reorder', pageId: first, toIndex: 2 });
    expect(s.state.pages.map((p) => p.srcIndex)).toEqual([1, 2, 0]);
    expect(s.state.dirty).toBe(true);
    s.undo();
    expect(s.state.pages.map((p) => p.srcIndex)).toEqual([0, 1, 2]);
    expect(s.state.dirty).toBe(false);
    s.redo();
    expect(s.state.pages.map((p) => p.srcIndex)).toEqual([1, 2, 0]);
  });

  it('deletes pages and restores their annotations on undo', () => {
    const s = fresh();
    const target = s.state.pages[1]!.id;
    s.apply({ type: 'addAnnot', annot: annot(target) });
    s.apply({ type: 'delete', pageId: target });
    expect(s.state.pages).toHaveLength(2);
    expect(s.state.annotations[target]).toBeUndefined();
    s.undo();
    expect(s.state.pages).toHaveLength(3);
    expect(s.state.pages[1]!.id).toBe(target);
    expect(s.state.annotations[target]).toHaveLength(1);
  });

  it('duplicates a page with a fresh id and copies annotations under new ids', () => {
    const s = fresh();
    const src = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: annot(src) });
    s.apply({ type: 'duplicate', pageId: src });
    expect(s.state.pages).toHaveLength(4);
    const dup = s.state.pages[1]!;
    expect(dup.id).not.toBe(src);
    expect(dup.srcIndex).toBe(0);
    expect(s.state.annotations[dup.id]).toHaveLength(1);
    expect(s.state.annotations[dup.id]![0]!.id).not.toBe('a-test-1');
    // redo after undo recreates the SAME id so later history stays valid
    s.undo();
    s.redo();
    expect(s.state.pages[1]!.id).toBe(dup.id);
  });

  it('rotates individual pages additively', () => {
    const s = fresh();
    const id = s.state.pages[0]!.id;
    s.apply({ type: 'rotate', pageId: id, delta: 90 });
    expect(s.state.pages[0]!.userRotation).toBe(90);
    s.apply({ type: 'rotate', pageId: id, delta: 270 });
    expect(s.state.pages[0]!.userRotation).toBe(0);
    s.undo();
    expect(s.state.pages[0]!.userRotation).toBe(90);
  });

  it('adds blank pages at an index', () => {
    const s = fresh();
    s.apply({ type: 'addBlank', index: 1, widthPt: 612, heightPt: 792 });
    expect(s.state.pages).toHaveLength(4);
    expect(s.state.pages[1]!.srcIndex).toBeNull();
    expect(s.state.pages[1]!.sizeKnown).toBe(true);
    s.undo();
    expect(s.state.pages).toHaveLength(3);
  });
});

describe('annotations', () => {
  it('creates, moves, and deletes annotations through undoable ops', () => {
    const s = fresh();
    const pid = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: annot(pid) });
    expect(s.state.annotations[pid]).toHaveLength(1);
    s.apply({
      type: 'patchAnnot',
      pageId: pid,
      id: 'a-test-1',
      patch: { rect: { x: 50, y: 60, w: 100, h: 50 } },
    });
    expect(s.state.annotations[pid]![0]!.rect.x).toBe(50);
    s.undo();
    expect(s.state.annotations[pid]![0]!.rect.x).toBe(10);
    s.apply({ type: 'deleteAnnot', pageId: pid, id: 'a-test-1' });
    expect(s.state.annotations[pid] ?? []).toHaveLength(0);
    s.undo();
    expect(s.state.annotations[pid]).toHaveLength(1);
  });
});

describe('form values', () => {
  it('records values with undo support', () => {
    const s = fresh();
    s.apply({ type: 'setForm', field: 'Name', value: 'Ada' });
    expect(s.state.formEdits['Name']).toBe('Ada');
    expect(s.state.dirty).toBe(true);
    s.undo();
    expect(s.state.formEdits['Name']).toBeUndefined();
  });
});

describe('dirty / savepoint semantics', () => {
  it('tracks the savepoint through undo and redo', () => {
    const s = fresh();
    const id = s.state.pages[0]!.id;
    s.apply({ type: 'rotate', pageId: id, delta: 90 });
    expect(s.state.dirty).toBe(true);
    s.markSaved();
    expect(s.state.dirty).toBe(false);
    s.undo();
    expect(s.state.dirty).toBe(true);
    s.redo();
    expect(s.state.dirty).toBe(false);
  });

  it('stays dirty forever once history diverges past the savepoint', () => {
    const s = fresh();
    const id = s.state.pages[0]!.id;
    s.apply({ type: 'rotate', pageId: id, delta: 90 });
    s.markSaved();
    s.undo();
    s.apply({ type: 'rotate', pageId: id, delta: 180 });
    expect(s.state.dirty).toBe(true);
    s.undo();
    expect(s.state.dirty).toBe(true);
  });
});

describe('history limit', () => {
  it('caps the undo depth', () => {
    const s = fresh({ historyLimit: 3 });
    const id = s.state.pages[0]!.id;
    for (let i = 0; i < 5; i++) s.apply({ type: 'rotate', pageId: id, delta: 90 });
    let undone = 0;
    while (s.undo()) undone++;
    expect(undone).toBe(3);
  });
});

describe('buildEditPlan', () => {
  it('serializes order, rotations, annotations, texts, and images', () => {
    const s = fresh();
    const [p0, p1, p2] = s.state.pages.map((p) => p.id) as [string, string, string];
    s.apply({ type: 'delete', pageId: p1 });
    s.apply({ type: 'rotate', pageId: p2, delta: 90 }); // save plan carries only the user delta
    s.apply({ type: 'addBlank', index: 2, widthPt: 500, heightPt: 500 });
    s.apply({ type: 'addAnnot', annot: annot(p0, { id: 'hl', kind: 'highlight', quads: [] }) });
    s.apply({
      type: 'addAnnot',
      annot: annot(p0, { id: 'tb', kind: 'textbox', text: 'Hello', fontSizePt: 14 }),
    });
    s.apply({
      type: 'addAnnot',
      annot: annot(p0, {
        id: 'im',
        kind: 'image',
        sourcePath: '/tmp/x.png',
        naturalW: 32,
        naturalH: 32,
      }),
    });
    s.apply({ type: 'setForm', field: 'Email', value: 'a@b.c' });

    const plan = s.buildEditPlan();
    expect(plan.pages).toHaveLength(3);
    expect(plan.pages.map((p) => p.srcIndex)).toEqual([0, 2, null]);
    expect(plan.pages[1]!.rotation).toBe(90);
    expect(plan.pages[2]!.widthPt).toBe(500);
    expect(plan.pages[0]!.annots).toHaveLength(1);
    expect(plan.pages[0]!.texts).toHaveLength(1);
    expect(plan.pages[0]!.texts[0]!.text).toBe('Hello');
    expect(plan.pages[0]!.images).toHaveLength(1);
    expect(plan.form).toEqual([['Email', 'a@b.c']]);
  });
});

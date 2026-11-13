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

it('does not coalesce typing across a savepoint or after undo', () => {
  const store = fresh();
  const pageId = store.state.pages[0]!.id;
  const a = annot(pageId, { kind: 'textbox', text: '' });
  store.apply({ type: 'addAnnot', annot: a });
  const group = Symbol('typing');
  const type = (text: string) =>
    store.apply({ type: 'patchAnnot', pageId, id: a.id, patch: { text } }, group);
  type('a');
  type('ab');
  store.markSaved();
  type('abc');
  store.undo();
  expect(store.state.annotations[pageId]![0]!.text).toBe('ab');
  expect(store.state.dirty).toBe(false);
  type('ab!');
  expect(store.state.dirty).toBe(true);
  store.undo();
  expect(store.state.annotations[pageId]![0]!.text).toBe('ab');
  store.undo();
  expect(store.state.annotations[pageId]![0]!.text).toBe('');
});

it('replaces a saved text appearance after a formatting-only edit', () => {
  const store = fresh();
  store.hydrateAnnotations([
    {
      src: 0,
      annots: [
        annot('', {
          kind: 'textbox',
          text: 'Existing text',
          fontFamily: 'Helvetica',
          bold: false,
          italic: false,
          underline: false,
          srcAnnotIndex: 4,
        }),
      ],
    },
  ]);
  const pageId = store.state.pages[0]!.id;
  const a = store.state.annotations[pageId]![0]!;
  expect(a.pageId).toBe(pageId);
  expect(store.buildEditPlan().pages[0]!.texts).toHaveLength(0);
  store.apply({
    type: 'patchAnnot',
    pageId,
    id: a.id,
    patch: { fontFamily: 'Times-Roman', bold: true, italic: true, underline: true },
  });
  expect(store.buildEditPlan().pages[0]).toMatchObject({
    dropSrcAnnots: [4],
    texts: [{ fontFamily: 'Times-Roman', bold: true, italic: true, underline: true }],
  });
  store.undo();
  expect(store.buildEditPlan().pages[0]!.texts).toHaveLength(0);
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

  it('hydrates every page sharing a source index in one size batch', () => {
    const s = createDocumentStore();
    s.initFromMeta({ ...meta, pageCount: 5 });
    const estimated = s.state.pages[4]!;
    s.apply({ type: 'duplicate', pageId: estimated.id });
    const duplicate = s.state.pages.find(
      (page) => page.srcIndex === 4 && page.id !== estimated.id
    )!;
    s.apply({ type: 'reorder', pageId: duplicate.id, toIndex: 0 });

    s.updateSizes(4, [[420, 640, 0, 0, 0]]);

    const copies = s.state.pages.filter((page) => page.srcIndex === 4);
    expect(copies).toHaveLength(2);
    expect(copies.every((page) => page.sizeKnown)).toBe(true);
    expect(copies.map((page) => [page.widthPt, page.heightPt])).toEqual([
      [420, 640],
      [420, 640],
    ]);
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

describe('shaded bubbles', () => {
  const shading = (pageId: string, charIndex: number): Annotation => ({
    id: `bub-${charIndex}`,
    pageId,
    kind: 'bubble',
    rect: { x: 72.61, y: 531.1, w: 10.63, h: 10.63 },
    color: '#1a1a1a',
    opacity: 1,
    bubbleCharIndex: charIndex,
  });

  // What Cmd-S actually sends. A shading that never reaches the plan is a
  // mark the reader can see on screen and will not find in the saved file.
  it('carries a shading into the edit plan and marks the document dirty', () => {
    const s = fresh();
    const pageId = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: shading(pageId, 733) });
    expect(s.state.dirty).toBe(true);

    const plan = s.buildEditPlan();
    expect(plan.pages[0]!.annots).toEqual([
      {
        kind: 'bubble',
        rect: { x: 72.61, y: 531.1, w: 10.63, h: 10.63 },
        color: '#1a1a1a',
        opacity: 1,
      },
    ]);
  });

  it('sends every shading on a page, not just the first', () => {
    const s = fresh();
    const pageId = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: shading(pageId, 733) });
    s.apply({ type: 'addAnnot', annot: shading(pageId, 739) });
    expect(s.buildEditPlan().pages[0]!.annots).toHaveLength(2);
  });

  it('drops a shading again when it is cleared', () => {
    const s = fresh();
    const pageId = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: shading(pageId, 733) });
    s.apply({ type: 'deleteAnnot', pageId, id: 'bub-733' });
    expect(s.buildEditPlan().pages[0]!.annots).toEqual([]);
  });
});

describe('form buttons', () => {
  it('records a widget by page and index, with undo support', () => {
    const s = fresh();
    s.apply({ type: 'setFormButton', page: 0, index: 3, checked: true });
    expect(s.state.formButtonEdits['0:3']).toBe(true);
    expect(s.state.dirty).toBe(true);
    s.undo();
    expect(s.state.formButtonEdits['0:3']).toBeUndefined();
  });

  // Two buttons in one radio group share a field name, so a name-keyed edit
  // would have the second overwrite the first instead of sitting beside it.
  it('keeps two widgets on the same page apart', () => {
    const s = fresh();
    s.apply({ type: 'setFormButton', page: 0, index: 1, checked: true });
    s.apply({ type: 'setFormButton', page: 0, index: 2, checked: false });
    expect(s.state.formButtonEdits).toEqual({ '0:1': true, '0:2': false });
  });

  it('carries button edits into the edit plan', () => {
    const s = fresh();
    s.apply({ type: 'setFormButton', page: 1, index: 4, checked: true });
    expect(s.buildEditPlan().formButtons).toEqual([{ page: 1, index: 4, checked: true }]);
  });

  it('sends no buttons when none were touched', () => {
    expect(fresh().buildEditPlan().formButtons).toEqual([]);
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

describe('hydrateAnnotations', () => {
  /** Two annotations as they arrive from the file, both on source page 0. */
  const fromFile = (store: ReturnType<typeof fresh>) => {
    store.hydrateAnnotations([
      {
        src: 0,
        annots: [
          { ...annot('', { kind: 'highlight', srcAnnotIndex: 0 }), id: '' },
          { ...annot('', { kind: 'rect', srcAnnotIndex: 1 }), id: '' },
        ],
      },
    ]);
    const pageId = store.state.pages[0]!.id;
    return { pageId, annots: store.state.annotations[pageId] ?? [] };
  };

  it('adopting a file’s annotations is not an unsaved change', () => {
    // Opening a paper someone else highlighted must not offer to save it.
    const s = fresh();
    const { annots } = fromFile(s);
    expect(annots).toHaveLength(2);
    expect(s.state.dirty).toBe(false);
    expect(s.canUndo()).toBe(false);
    expect(s.state.historyDepth).toBe(0);
  });

  it('gives every adopted annotation its own id', () => {
    const s = fresh();
    const { annots } = fromFile(s);
    expect(new Set(annots.map((a) => a.id)).size).toBe(2);
    expect(annots.every((a) => a.id !== '')).toBe(true);
  });

  it('leaves untouched annotations for the imported page to carry', () => {
    // They are already in the file; rewriting them would strip the author,
    // dates and appearance stream this model has no room for.
    const s = fresh();
    fromFile(s);
    const page = s.buildEditPlan().pages[0]!;
    expect(page.annots).toHaveLength(0);
    expect(page.dropSrcAnnots).toHaveLength(0);
  });

  it('drops and rewrites only the annotation that was edited', () => {
    const s = fresh();
    const { pageId, annots } = fromFile(s);
    const target = annots[1]!;
    s.apply({
      type: 'patchAnnot',
      pageId,
      id: target.id,
      patch: { rect: { x: 200, y: 200, w: 20, h: 20 } },
    });

    const page = s.buildEditPlan().pages[0]!;
    expect(page.dropSrcAnnots).toEqual([1]);
    expect(page.annots).toHaveLength(1);
    expect(page.annots[0]!.rect.x).toBe(200);
    // ...and editing one does count as an unsaved change.
    expect(s.state.dirty).toBe(true);
  });

  it('drops a deleted annotation without writing it back', () => {
    // The one case where the index is the only record left of it.
    const s = fresh();
    const { pageId, annots } = fromFile(s);
    s.apply({ type: 'deleteAnnot', pageId, id: annots[0]!.id });

    const page = s.buildEditPlan().pages[0]!;
    expect(page.dropSrcAnnots).toEqual([0]);
    expect(page.annots).toHaveLength(0);
  });

  it('treats an edit that restores the original as no edit at all', () => {
    const s = fresh();
    const { pageId, annots } = fromFile(s);
    const target = annots[0]!;
    const original = target.rect;
    s.apply({
      type: 'patchAnnot',
      pageId,
      id: target.id,
      patch: { rect: { x: 5, y: 5, w: 5, h: 5 } },
    });
    s.apply({ type: 'patchAnnot', pageId, id: target.id, patch: { rect: original } });

    const page = s.buildEditPlan().pages[0]!;
    expect(page.dropSrcAnnots).toHaveLength(0);
    expect(page.annots).toHaveLength(0);
  });

  it('reset forgets what was hydrated', () => {
    const s = fresh();
    fromFile(s);
    s.reset();
    s.initFromMeta(meta);
    expect(s.buildEditPlan().pages[0]!.dropSrcAnnots).toHaveLength(0);
  });
});

it('moves edits, source annotation ownership, savepoint and both history stacks through JSON', () => {
  const source = fresh();
  source.hydrateAnnotations([{ src: 0, annots: [annot('', { srcAnnotIndex: 3 })] }]);
  const pageId = source.state.pages[0]!.id;
  const id = source.state.annotations[pageId]![0]!.id;
  source.apply({ type: 'patchAnnot', pageId, id, patch: { color: '#00ff00', bold: true } });
  source.apply({ type: 'setForm', field: 'name', value: 'Still unsaved' });
  source.apply({ type: 'rotate', pageId, delta: 90 });
  source.undo();
  const target = createDocumentStore();
  target.restore(JSON.parse(JSON.stringify(source.snapshot())));
  expect(target.buildEditPlan()).toEqual(source.buildEditPlan());
  expect(target.ownedSrcAnnots()).toEqual(source.ownedSrcAnnots());
  expect(target.state.dirty).toBe(true);
  expect(target.redo()).toBe(true);
  expect(target.state.pages[0]!.userRotation).toBe(90);
  target.undo();
  target.undo();
  target.undo();
  expect(target.state.dirty).toBe(false);
  expect(target.state.annotations[pageId]![0]!.bold).toBeFalsy();
  expect(target.buildEditPlan().pages[0]!.dropSrcAnnots).toEqual([]);
  target.apply({ type: 'addBlank', index: 1, widthPt: 100, heightPt: 100 });
  expect(new Set(target.state.pages.map((p) => p.id)).size).toBe(target.state.pages.length);
  expect(source.state.formEdits.name).toBe('Still unsaved');
});

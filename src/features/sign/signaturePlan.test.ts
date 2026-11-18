/** How placed signatures reach the engine: as `signature` plan annotations
 * carrying their artwork, flattened into the page only when the document
 * says so, and - once read back from a file - left alone unless touched or
 * flattened. */
import { describe, expect, it } from 'vitest';
import { createDocumentStore } from '../document/documentStore';
import type { Annotation, SignatureArt } from '../../types/model';

const art: SignatureArt = {
  type: 'drawn',
  w: 100,
  h: 30,
  width: 2,
  strokes: [
    [
      { x: 2, y: 2 },
      { x: 98, y: 28 },
    ],
  ],
};

function store() {
  const s = createDocumentStore();
  s.initFromMeta({
    docId: 1,
    path: '/tmp/a.pdf',
    name: 'a.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  return s;
}

const signature = (pageId: string, over: Partial<Annotation> = {}): Annotation => ({
  id: 'sig-1',
  pageId,
  kind: 'signature',
  rect: { x: 50, y: 60, w: 100, h: 30 },
  color: '#1a1a1a',
  opacity: 1,
  signature: art,
  ...over,
});

describe('signatures in the edit plan', () => {
  it('sends a placed signature with its artwork, movable by default', () => {
    const s = store();
    const pageId = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: signature(pageId) });
    const [planned] = s.buildEditPlan().pages[0]!.annots;
    expect(planned).toMatchObject({ kind: 'signature', signature: art, flatten: false });
  });

  it('flattens on request, as an undoable edit that dirties the document', () => {
    const s = store();
    const pageId = s.state.pages[0]!.id;
    s.apply({ type: 'addAnnot', annot: signature(pageId) });
    s.markSaved();
    s.apply({ type: 'setFlattenSignatures', value: true });
    expect(s.state.dirty).toBe(true);
    expect(s.buildEditPlan().pages[0]!.annots[0]?.flatten).toBe(true);
    s.undo();
    expect(s.state.flattenSignatures).toBe(false);
    expect(s.state.dirty).toBe(false);
    expect(s.buildEditPlan().pages[0]!.annots[0]?.flatten).toBe(false);
    s.redo();
    expect(s.state.flattenSignatures).toBe(true);
  });

  it('leaves a signature read from the file alone until it is moved or flattened', () => {
    const s = store();
    s.hydrateAnnotations([{ src: 0, annots: [{ ...signature(''), id: '', srcAnnotIndex: 2 }] }]);
    let page = s.buildEditPlan().pages[0]!;
    expect(page.annots).toEqual([]);
    expect(page.dropSrcAnnots).toEqual([]);

    s.apply({ type: 'setFlattenSignatures', value: true });
    page = s.buildEditPlan().pages[0]!;
    expect(page.annots).toHaveLength(1);
    expect(page.annots[0]?.flatten).toBe(true);
    expect(page.dropSrcAnnots).toEqual([2]);

    s.undo();
    const pageId = s.state.pages[0]!.id;
    const hydrated = s.state.annotations[pageId]![0]!;
    s.apply({
      type: 'patchAnnot',
      pageId,
      id: hydrated.id,
      patch: { rect: { x: 200, y: 300, w: 100, h: 30 } },
    });
    page = s.buildEditPlan().pages[0]!;
    expect(page.annots[0]).toMatchObject({ kind: 'signature', flatten: false });
    expect(page.dropSrcAnnots).toEqual([2]);
  });
});

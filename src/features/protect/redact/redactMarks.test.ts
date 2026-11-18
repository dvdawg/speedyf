import { describe, expect, it } from 'vitest';
import type { TextRunDto } from '../../../types/engine';
import type { DocMeta } from '../../../types/model';
import { createDocumentStore } from '../../document/documentStore';
import {
  addMarks,
  inkOn,
  markBoxes,
  markCount,
  markFromDrag,
  markFromRects,
  snappedTextRects,
} from './redactMarks';

// "Name: JOHN end", 6 pt per glyph from x = 72.
const run: TextRunDto = {
  text: 'Name: JOHN end',
  start: 0,
  x: 72,
  y: 700,
  w: 84,
  h: 10,
  edges: Array.from({ length: 15 }, (_, i) => i * 6),
};

const meta: DocMeta = {
  docId: 1,
  path: '/tmp/sample.pdf',
  name: 'sample.pdf',
  pageCount: 2,
  sizes: [
    [612, 792, 0, 0, 0],
    [612, 792, 0, 0, 0],
  ],
  estimatedSize: [612, 792],
};

describe('snappedTextRects', () => {
  it('snaps a drag to whole glyphs', () => {
    // From the middle of ':' (x 100-106) to the middle of 'N' (x 126-132).
    const rects = snappedTextRects([run], { x: 104, y: 695, w: 25, h: 20 });
    // ':' (96-102) is not covered; ' ' two thirds; J, O, H fully; N half.
    expect(rects).toEqual([{ x: 102, y: 700, w: 30, h: 10 }]);
  });

  it('ignores runs the drag barely touches vertically', () => {
    expect(snappedTextRects([run], { x: 72, y: 708, w: 50, h: 10 })).toEqual([]);
  });
});

describe('markFromDrag', () => {
  it('marks text over text, and the area elsewhere or with Alt', () => {
    const text = markFromDrag([run], { x: 104, y: 695, w: 25, h: 20 }, false);
    expect(text?.quads).toHaveLength(1);
    const area = markFromDrag([run], { x: 104, y: 695, w: 25, h: 20 }, true);
    expect(area).toEqual({ rect: { x: 104, y: 695, w: 25, h: 20 } });
    expect(markFromDrag([run], { x: 300, y: 300, w: 40, h: 30 }, false)?.quads).toBeUndefined();
    expect(markFromDrag([], { x: 300, y: 300, w: 2, h: 2 }, false)).toBeNull();
  });
});

describe('marks in the document model', () => {
  it('adds search matches and plans them for save', () => {
    const store = createDocumentStore();
    store.initFromMeta(meta);
    const [p0, p1] = store.state.pages;
    const shape = markFromRects([
      { x: 10, y: 10, w: 30, h: 8 },
      { x: 10, y: 0, w: 20, h: 8 },
    ])!;
    addMarks(
      store,
      [
        { pageId: p0!.id, shape },
        { pageId: p1!.id, shape },
      ],
      { color: '#000000', overlay: 'Withheld' }
    );
    expect(markCount(store)).toBe(2);
    const plan = store.buildEditPlan();
    const annot = plan.pages[0]!.annots[0]!;
    expect(annot.kind).toBe('redact');
    expect(annot.text).toBe('Withheld');
    expect(annot.quads).toHaveLength(2);
    expect(plan.scrub).toBeUndefined();
    store.undo();
    expect(markCount(store)).toBe(1);
  });

  it('carries an armed apply into the plan, undoably', () => {
    const store = createDocumentStore();
    store.initFromMeta(meta);
    store.apply({ type: 'setScrub', scrub: { applyRedactions: true, removeHidden: null } });
    expect(store.buildEditPlan().scrub).toEqual({ applyRedactions: true, removeHidden: null });
    expect(store.state.dirty).toBe(true);
    store.undo();
    expect(store.buildEditPlan().scrub).toBeUndefined();
  });
});

describe('mark view', () => {
  it('draws one box per quad and readable overlay ink', () => {
    const shape = markFromRects([
      { x: 1, y: 2, w: 3, h: 4 },
      { x: 5, y: 6, w: 7, h: 8 },
    ])!;
    const boxes = markBoxes({
      id: 'a',
      pageId: 'p',
      kind: 'redact',
      color: '#000000',
      opacity: 1,
      ...shape,
    });
    expect(boxes).toEqual([
      { x: 1, y: 2, w: 3, h: 4 },
      { x: 5, y: 6, w: 7, h: 8 },
    ]);
    expect(inkOn('#000000')).toBe('white');
    expect(inkOn('#ffffff')).toBe('black');
  });
});

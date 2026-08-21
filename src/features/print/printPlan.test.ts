import { describe, expect, it } from 'vitest';
import { limitToRange, pagesInRange, withoutAnnotations } from './printPlan';
import type { EditPlan } from '../document/documentStore';

const rect = { x: 10, y: 10, w: 100, h: 20 };
const page = (srcIndex: number) => ({
  srcIndex,
  widthPt: 612,
  heightPt: 792,
  rotation: 0 as const,
  annots: [{ kind: 'highlight' as const, rect, color: '#ffff00', opacity: 0.4 }],
  texts: [{ rect, text: 'note', fontSizePt: 12, color: '#000000', opacity: 1 }],
  images: [],
  dropSrcAnnots: [],
});
const plan = (count: number): EditPlan => ({
  pages: Array.from({ length: count }, (_, i) => page(i)),
  form: [],
});

describe('withoutAnnotations', () => {
  it('strips everything the reader added', () => {
    const clean = withoutAnnotations(plan(2));
    expect(clean.pages.every((p) => p.annots.length === 0 && p.texts.length === 0)).toBe(true);
  });

  it('leaves the original plan alone', () => {
    // The document keeps being edited while the dialog is open.
    const original = plan(1);
    withoutAnnotations(original);
    expect(original.pages[0]!.annots).toHaveLength(1);
  });
});

describe('pagesInRange', () => {
  it('expands pages, spans and lists into zero-based indices', () => {
    expect(pagesInRange('1', 10)).toEqual([0]);
    expect(pagesInRange('2-4', 10)).toEqual([1, 2, 3]);
    expect(pagesInRange('1,4,7-9', 10)).toEqual([0, 3, 6, 7, 8]);
  });

  it('sorts and de-duplicates overlapping parts', () => {
    expect(pagesInRange('5,1-2,2', 10)).toEqual([0, 1, 4]);
  });

  it('refuses ranges it cannot honour', () => {
    for (const bad of ['0', '5-2', '1-99', 'a', '1;2', '', '1-']) {
      expect(pagesInRange(bad, 10), bad).toBeNull();
    }
  });
});

describe('limitToRange', () => {
  it('keeps only the requested pages, in order', () => {
    const limited = limitToRange(plan(6), '4,1-2');
    expect(limited.pages.map((p) => p.srcIndex)).toEqual([0, 1, 3]);
  });

  it('falls back to the whole document rather than exporting nothing', () => {
    expect(limitToRange(plan(3), '9-12').pages).toHaveLength(3);
  });
});

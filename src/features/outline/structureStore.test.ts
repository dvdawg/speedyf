import { describe, expect, it } from 'vitest';
import { flattenOutline } from './structureStore';
import type { OutlineNode } from '../../types/engine';

function node(title: string, page: number | null, children: OutlineNode[] = []): OutlineNode {
  return { title, page, y: null, children };
}

describe('flattenOutline', () => {
  it('keeps document order, parents before their children', () => {
    const tree = [node('One', 0, [node('One.A', 1), node('One.B', 2)]), node('Two', 3)];
    expect(flattenOutline(tree).map((row) => row.node.title)).toEqual([
      'One',
      'One.A',
      'One.B',
      'Two',
    ]);
  });

  it('reports how deep each row sits', () => {
    const tree = [node('One', 0, [node('One.A', 1, [node('One.A.i', 2)])])];
    expect(flattenOutline(tree).map((row) => row.depth)).toEqual([0, 1, 2]);
  });

  it('keeps destination-less containers, which the caller filters', () => {
    // A node with no page is a grouping row in the sidebar; dropping it here
    // would also drop the children hanging off it.
    const tree = [node('Part I', null, [node('Chapter 1', 4)])];
    expect(flattenOutline(tree).map((row) => row.node.title)).toEqual(['Part I', 'Chapter 1']);
  });

  it('is empty for a document with no outline', () => {
    expect(flattenOutline([])).toEqual([]);
  });
});

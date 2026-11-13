import { describe, expect, it, vi, beforeEach } from 'vitest';
import {
  clearSelection,
  hasSelection,
  noteSelectionChanged,
  selectedText,
  selectionGeometryFrom,
} from './selectionStore';

const settle = () => vi.advanceTimersByTime(200);

beforeEach(() => {
  vi.useFakeTimers();
  clearSelection();
});

describe('selection capture', () => {
  it('records the reconstructed text once the drag settles', () => {
    noteSelectionChanged(() => 'a highlighted sentence');
    expect(selectedText()).toBe('');
    settle();
    expect(selectedText()).toBe('a highlighted sentence');
    expect(hasSelection()).toBe(true);
  });

  it('only computes once for a burst of changes', () => {
    const compute = vi.fn(() => 'final');
    noteSelectionChanged(compute);
    noteSelectionChanged(compute);
    noteSelectionChanged(compute);
    settle();
    expect(compute).toHaveBeenCalledTimes(1);
  });

  /** The bug this exists to prevent: opening the command palette moves focus,
   * which collapses the selection. If that wiped the capture, every action on
   * a selection would receive nothing — which is exactly what happened. */
  it('survives the selection collapsing after it was captured', () => {
    noteSelectionChanged(() => 'kept text');
    settle();
    noteSelectionChanged(() => null);
    settle();
    expect(selectedText()).toBe('kept text');
  });

  it('survives a collapse during the settle window', () => {
    noteSelectionChanged(() => 'first');
    settle();
    // The live Selection went away before the timer fired.
    noteSelectionChanged(() => '');
    settle();
    expect(selectedText()).toBe('first');
  });

  it('is replaced by a genuine new selection', () => {
    noteSelectionChanged(() => 'first');
    settle();
    noteSelectionChanged(() => 'second');
    settle();
    expect(selectedText()).toBe('second');
  });

  it('is forgotten when explicitly cleared', () => {
    noteSelectionChanged(() => 'text');
    settle();
    clearSelection();
    expect(selectedText()).toBe('');
    expect(hasSelection()).toBe(false);
  });

  it('drops a pending capture when cleared before it settles', () => {
    noteSelectionChanged(() => 'never arrives');
    clearSelection();
    settle();
    expect(selectedText()).toBe('');
  });
});

describe('selectionGeometryFrom', () => {
  it('keeps the page holding most of the selection and turns its rects top-left', () => {
    const geometry = selectionGeometryFrom(
      3,
      [
        { page: 1, rect: [10, 700, 50, 12] },
        { page: 2, rect: [10, 680, 40, 12] },
        { page: 2, rect: [10, 660, 40, 12] },
      ],
      (page) => (page === 2 ? 792 : undefined)
    );
    expect(geometry).toEqual({
      docId: 3,
      page: 2,
      rects: [
        [10, 100, 40, 12],
        [10, 120, 40, 12],
      ],
    });
  });

  it('merges a long selection into lines to stay under the engine cap', () => {
    const runs = Array.from({ length: 100 }, (_, i) => ({
      page: 0,
      rect: [(i % 4) * 100, 700 - Math.floor(i / 4) * 14, 90, 12] as [
        number,
        number,
        number,
        number,
      ],
    }));
    const geometry = selectionGeometryFrom(1, runs, () => 800)!;
    expect(geometry.rects).toHaveLength(25);
    // The top line: four runs side by side, 800 - (700 + 12) from the top.
    expect(geometry.rects[0]).toEqual([0, 88, 390, 12]);
  });

  it('is null with nothing selected', () => {
    expect(selectionGeometryFrom(1, [], () => 792)).toBeNull();
  });
});

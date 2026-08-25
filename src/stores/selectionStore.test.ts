import { describe, expect, it, vi, beforeEach } from 'vitest';
import { clearSelection, hasSelection, noteSelectionChanged, selectedText } from './selectionStore';

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

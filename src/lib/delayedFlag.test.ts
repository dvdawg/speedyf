import { createRoot, createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createDelayedFlag, SKELETON_DELAY } from './delayedFlag';
import { rowsPending, type Scope } from '../features/projects/projectsStore';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function setup() {
  return createRoot((dispose) => {
    const [source, setSource] = createSignal(false);
    const flag = createDelayedFlag(source);
    return { flag, setSource, dispose };
  });
}

describe('createDelayedFlag', () => {
  it('waits out the delay before turning on', () => {
    const { flag, setSource, dispose } = setup();
    setSource(true);
    vi.advanceTimersByTime(SKELETON_DELAY - 1);
    expect(flag()).toBe(false);
    vi.advanceTimersByTime(1);
    expect(flag()).toBe(true);
    dispose();
  });

  it('never flashes for a wait shorter than the delay', () => {
    const { flag, setSource, dispose } = setup();
    setSource(true);
    vi.advanceTimersByTime(100);
    setSource(false);
    vi.advanceTimersByTime(500);
    expect(flag()).toBe(false);
    dispose();
  });

  it('turns off at once', () => {
    const { flag, setSource, dispose } = setup();
    setSource(true);
    vi.advanceTimersByTime(SKELETON_DELAY);
    setSource(false);
    expect(flag()).toBe(false);
    dispose();
  });

  it('waits at least 150 ms', () => {
    expect(SKELETON_DELAY).toBeGreaterThanOrEqual(150);
  });
});

describe('rowsPending', () => {
  const all: Scope = { kind: 'all' };
  const p1: Scope = { kind: 'project', id: 'p1' };
  const row = [{}];

  it('is idle when nothing is loading', () => {
    expect(rowsPending({ loadingRows: false, rows: [], rowsScope: null, scope: all })).toBe(false);
  });

  it('waits on an empty list or the rows of another scope', () => {
    expect(rowsPending({ loadingRows: true, rows: [], rowsScope: all, scope: all })).toBe(true);
    expect(rowsPending({ loadingRows: true, rows: row, rowsScope: all, scope: p1 })).toBe(true);
    expect(rowsPending({ loadingRows: true, rows: row, rowsScope: null, scope: all })).toBe(true);
  });

  it('keeps current rows through a refresh of the same scope', () => {
    expect(rowsPending({ loadingRows: true, rows: row, rowsScope: p1, scope: p1 })).toBe(false);
  });
});

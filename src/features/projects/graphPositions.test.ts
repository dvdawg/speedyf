/** Dragged positions survive the session: per scope, pruned to the papers
 * still there, bounded in count and in scopes, and harmless when storage
 * is broken. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  clearPositions,
  loadPositions,
  MAX_POSITIONS,
  MAX_SCOPES,
  savePositions,
  STORAGE_KEY,
} from './graphPositions';
import { memoryStorage } from './testFixtures';

let storage: Storage;
beforeEach(() => {
  storage = memoryStorage();
  vi.stubGlobal('localStorage', storage);
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('graphPositions', () => {
  it('round-trips per scope and clears one scope alone', () => {
    savePositions('p1', { a: { x: 0.25, y: 0.75 } });
    savePositions('all', { a: { x: 0.5, y: 0.5 } });
    expect(loadPositions('p1')).toEqual({ a: { x: 0.25, y: 0.75 } });
    expect(loadPositions('all')).toEqual({ a: { x: 0.5, y: 0.5 } });
    clearPositions('p1');
    expect(loadPositions('p1')).toEqual({});
    expect(loadPositions('all')).toEqual({ a: { x: 0.5, y: 0.5 } });
  });

  it('prunes papers no longer in the scope', () => {
    savePositions('p1', { a: { x: 0.1, y: 0.1 }, gone: { x: 0.2, y: 0.2 } }, new Set(['a']));
    expect(Object.keys(loadPositions('p1'))).toEqual(['a']);
  });

  it('keeps the most recent positions and the most recently saved scopes', () => {
    const many = Object.fromEntries(
      Array.from({ length: MAX_POSITIONS + 5 }, (_, i) => [`n${i}`, { x: 0.5, y: 0.5 }])
    );
    savePositions('big', many);
    const kept = Object.keys(loadPositions('big'));
    expect(kept).toHaveLength(MAX_POSITIONS);
    expect(kept).not.toContain('n0');
    expect(kept).toContain(`n${MAX_POSITIONS + 4}`);

    for (let i = 0; i < MAX_SCOPES; i++) savePositions(`s${i}`, { a: { x: 0, y: 0 } });
    expect(loadPositions('big')).toEqual({});
    expect(loadPositions('s0')).not.toEqual({});
    expect(Object.keys(JSON.parse(localStorage.getItem(STORAGE_KEY)!))).toHaveLength(MAX_SCOPES);
  });

  it('ignores malformed data and survives a storage that throws', () => {
    localStorage.setItem(STORAGE_KEY, '{"p1":{"used":1,"positions":{"a":[1,"x"],"b":[0.5,0.5]}}}');
    expect(loadPositions('p1')).toEqual({ b: { x: 0.5, y: 0.5 } });
    localStorage.setItem(STORAGE_KEY, 'not json');
    expect(loadPositions('p1')).toEqual({});

    vi.spyOn(storage, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    vi.spyOn(storage, 'setItem').mockImplementation(() => {
      throw new Error('full');
    });
    expect(() => savePositions('p1', { a: { x: 0, y: 0 } })).not.toThrow();
    expect(loadPositions('p1')).toEqual({});
    expect(() => clearPositions('p1')).not.toThrow();
  });
});

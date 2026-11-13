import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GeneratedOutline, OutlineNode } from '../../types/engine';

vi.mock('../../lib/transport/engine', () => ({
  engine: {
    getOutline: vi.fn(),
    getGeneratedOutline: vi.fn(),
    getFormalEnvs: vi.fn(),
  },
}));

import { engine } from '../../lib/transport/engine';
import { createStructureStore } from './structureStore';

const getOutline = vi.mocked(engine.getOutline);
const getGenerated = vi.mocked(engine.getGeneratedOutline);

function node(title: string, page: number): OutlineNode {
  return { title, page, y: 500, children: [] };
}

function slice(done: boolean, outline: OutlineNode[]): GeneratedOutline {
  return { done, outline, pagesDone: done ? 10 : 5, pageCount: 10 };
}

describe('generated outline', () => {
  beforeEach(() => {
    getOutline.mockReset();
    getGenerated.mockReset();
  });

  it('asks again until done and shows what the headings gave', async () => {
    getOutline.mockResolvedValue([]);
    getGenerated
      .mockResolvedValueOnce(slice(false, []))
      .mockResolvedValueOnce(slice(true, [node('1 Introduction', 0), node('2 Methods', 3)]));
    const store = createStructureStore();
    store.syncDocument(7);
    await vi.waitFor(() => expect(store.state.outline).toHaveLength(2));
    expect(store.state.outlineGenerated).toBe(true);
    expect(getGenerated).toHaveBeenCalledTimes(2);
  });

  it('leaves a real outline alone', async () => {
    getOutline.mockResolvedValue([node('Intro', 0), node('Methods', 2)]);
    const store = createStructureStore();
    store.syncDocument(7);
    await vi.waitFor(() => expect(store.state.outlineLoading).toBe(false));
    expect(getGenerated).not.toHaveBeenCalled();
    expect(store.state.outlineGenerated).toBe(false);
  });

  it('keeps the trivial outline when nothing confident is found', async () => {
    const title = [node('Paper title', 0)];
    getOutline.mockResolvedValue(title);
    getGenerated
      .mockResolvedValueOnce(slice(false, [node('A', 0), node('B', 1)]))
      .mockResolvedValueOnce(slice(true, []));
    const store = createStructureStore();
    store.syncDocument(7);
    await vi.waitFor(() => expect(getGenerated).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(store.state.outlineGenerated).toBe(false));
    expect(store.state.outline.map((n) => n.title)).toEqual(['Paper title']);
  });

  it('stops when another document takes the tab', async () => {
    getOutline.mockResolvedValueOnce([]).mockResolvedValueOnce([node('A', 0), node('B', 4)]);
    getGenerated.mockResolvedValue(slice(true, [node('Stale', 0), node('Stale 2', 1)]));
    const store = createStructureStore();
    store.syncDocument(1);
    store.syncDocument(2);
    await vi.waitFor(() => expect(store.state.outlineLoading).toBe(false));
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(store.state.outline.map((n) => n.title)).toEqual(['A', 'B']);
    expect(store.state.outlineGenerated).toBe(false);
  });
});

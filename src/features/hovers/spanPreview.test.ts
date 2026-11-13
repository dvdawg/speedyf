/** Spans share the link previews' hover machine, so there is only ever one
 * popover: a cross-reference or footnote previews its region of a page, a
 * term previews as text without asking the engine for anything. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { HoverSpan } from './hoverTypes';

const invoke = vi.fn(async (command: string, args: Record<string, unknown>) => {
  if (command === 'get_hover_preview') {
    return {
      docId: args.docId,
      src: args.src,
      tile: { x: 0, y: 0, w: 100, h: 40 },
      scaleMilli: 2000,
      text: '',
    };
  }
  throw new Error(`unexpected ${command}`);
});
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (c: string, a: Record<string, unknown>) => invoke(c, a),
}));

import { createCitationStore } from '../citations/linkStore';
import { createDocumentStore } from '../document/documentStore';
import type { ViewportStore } from '../../stores/viewportStore';

const span = (over: Partial<HoverSpan>): HoverSpan => ({
  kind: 'equation',
  rects: [[100, 600, 30, 10]],
  label: 'Equation 12',
  target: { page: 3, x: 72, y: 500, region: [72, 450, 400, 60], continues: false, terms: [] },
  ...over,
});

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

afterEach(() => {
  invoke.mockClear();
  vi.useRealTimers();
});

describe('span previews in the citation popover', () => {
  it('renders an equation from its region on the target page', async () => {
    vi.useFakeTimers();
    const store = createCitationStore(createDocumentStore(), {} as ViewportStore);
    const element = {
      getBoundingClientRect: () => ({
        left: 0,
        top: 0,
        right: 10,
        bottom: 10,
        width: 10,
        height: 10,
      }),
    } as HTMLElement;
    store.enter(1, 0, { kind: 'span', span: span({}) }, element, true);
    vi.advanceTimersByTime(0);
    await settle();
    expect(invoke).toHaveBeenCalledWith('get_hover_preview', {
      docId: 1,
      src: 3,
      kind: 'equation',
      region: [72, 450, 400, 60],
    });
    expect(store.state.hover.result?.kind).toBe('span');
  });

  it('shows a term as text, without the engine', async () => {
    vi.useFakeTimers();
    const store = createCitationStore(createDocumentStore(), {} as ViewportStore);
    const element = {
      getBoundingClientRect: () => ({
        left: 0,
        top: 0,
        right: 10,
        bottom: 10,
        width: 10,
        height: 10,
      }),
    } as HTMLElement;
    const term = span({
      kind: 'term',
      label: 'FDT',
      target: {
        page: 2,
        x: 72,
        y: 510,
        region: null,
        continues: false,
        terms: [
          {
            title: 'FDT',
            body: 'fluctuation-dissipation theorem',
            page: 2,
            rect: [72, 500, 100, 10],
          },
        ],
      },
    });
    store.enter(1, 0, { kind: 'span', span: term }, element, true);
    vi.advanceTimersByTime(0);
    await settle();
    expect(invoke).not.toHaveBeenCalled();
    expect(store.state.hover.result).toEqual({ kind: 'term', span: term });
  });
});

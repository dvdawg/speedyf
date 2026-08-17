import { describe, expect, it } from 'vitest';
import { createViewportStore } from './viewportStore';

describe('scroll requests', () => {
  it('consumes a page request exactly once', () => {
    const viewport = createViewportStore();
    viewport.requestScrollToPage(0, 120);

    expect(viewport.takeScrollRequest()).toEqual({
      kind: 'page',
      page: 0,
      offsetCss: 120,
      seq: 1,
    });
    expect(viewport.state.scrollRequest).toBeNull();

    // Unrelated viewport changes (especially zoom/layout updates) cannot
    // resurrect and replay the navigation command.
    viewport.setState('zoom', 2);
    expect(viewport.takeScrollRequest()).toBeNull();
  });

  it('returns the newest position request and clears it', () => {
    const viewport = createViewportStore();
    viewport.requestScrollToPage(2);
    viewport.requestScrollToPosition(-20, 45);

    expect(viewport.takeScrollRequest()).toEqual({
      kind: 'position',
      top: 0,
      left: 45,
      seq: 2,
    });
    expect(viewport.takeScrollRequest()).toBeNull();
  });
});

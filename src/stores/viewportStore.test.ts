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

  it("keeps a restore request's fraction, settle flag and horizontal scroll", () => {
    const viewport = createViewportStore();
    viewport.requestRestorePosition(4, 0.25);
    expect(viewport.takeScrollRequest()).toEqual({
      kind: 'page',
      page: 4,
      fraction: 0.25,
      settle: true,
      seq: 1,
    });
    viewport.requestRestoreView(2, 0.5, 30);
    expect(viewport.takeScrollRequest()).toEqual({
      kind: 'page',
      page: 2,
      fraction: 0.5,
      left: 30,
      seq: 2,
    });
  });
});

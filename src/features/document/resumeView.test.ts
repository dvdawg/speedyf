import { describe, expect, it } from 'vitest';
import { createViewportStore, type ViewportStore } from '../../stores/viewportStore';
import { restoredZoom, resumeView } from './resumeView';

/** A real viewport store that notes the view as it stood when the scroll was
 * asked for: the request is resolved against the layout of that zoom. */
function watched(): { vp: ViewportStore; asked: { zoom: number; fitMode: string }[] } {
  const vp = createViewportStore();
  const asked: { zoom: number; fitMode: string }[] = [];
  const restore = vp.requestRestorePosition.bind(vp);
  vp.requestRestorePosition = (page, fraction) => {
    asked.push({ zoom: vp.state.zoom, fitMode: vp.state.fitMode });
    restore(page, fraction);
  };
  return { vp, asked };
}

describe('resumeView', () => {
  it('sets the zoom before asking for the scroll', () => {
    const { vp, asked } = watched();
    resumeView(vp, { page: 4, fraction: 0.25, zoom: 1.75, fitMode: 'custom' }, 10);
    expect(asked).toEqual([{ zoom: 1.75, fitMode: 'custom' }]);
    expect(vp.state.currentPage).toBe(4);
    expect(vp.takeScrollRequest()).toMatchObject({
      kind: 'page',
      page: 4,
      fraction: 0.25,
      // Waits for the fit, which may still recompute the zoom.
      settle: true,
    });
  });

  it('starts a fit mode at the zoom it last worked out to', () => {
    const { vp, asked } = watched();
    resumeView(vp, { page: 1, fraction: 0, zoom: 1.3, fitMode: 'fit-page' }, 3);
    expect(asked).toEqual([{ zoom: 1.3, fitMode: 'fit-page' }]);
  });

  it('opens a position from an older launch at the default zoom', () => {
    const { vp, asked } = watched();
    const defaults = { zoom: vp.state.zoom, fitMode: vp.state.fitMode };
    resumeView(vp, { page: 2, fraction: 0.5 }, 3);
    expect(asked).toEqual([defaults]);
  });

  it('keeps the zoom but starts at the top when the page is gone', () => {
    const { vp } = watched();
    resumeView(vp, { page: 40, fraction: 0.5, zoom: 2, fitMode: 'custom' }, 3);
    expect(vp.state.zoom).toBe(2);
    expect(vp.state.currentPage).toBe(0);
    expect(vp.takeScrollRequest()).toMatchObject({ kind: 'page', page: 0 });
  });

  it('starts at the top at the default zoom with nothing to resume', () => {
    const { vp } = watched();
    resumeView(vp, null, 3);
    expect(vp.state.zoom).toBe(1);
    expect(vp.state.fitMode).toBe('fit-width');
    expect(vp.takeScrollRequest()).toMatchObject({ kind: 'page', page: 0 });
  });
});

describe('restoredZoom', () => {
  it('clamps into the supported range', () => {
    expect(restoredZoom({ zoom: 12, fitMode: 'custom' })).toEqual({ zoom: 6, fitMode: 'custom' });
    expect(restoredZoom({ zoom: 0.05, fitMode: 'custom' })).toEqual({
      zoom: 0.2,
      fitMode: 'custom',
    });
  });

  it('has nothing for an explicit zoom that is not a number', () => {
    expect(restoredZoom({ zoom: Number.POSITIVE_INFINITY, fitMode: 'custom' })).toBeNull();
    expect(restoredZoom({ zoom: -1, fitMode: 'custom' })).toBeNull();
    expect(restoredZoom({ fitMode: 'custom' })).toBeNull();
  });

  it('has nothing without a fit mode it knows', () => {
    expect(restoredZoom({ zoom: 1.5 })).toBeNull();
  });
});

import { describe, expect, it } from 'vitest';
import { buildRenderUrl, protocolBase, retryUrl } from './renderSource';

describe('protocolBase', () => {
  it('uses scheme://localhost on macOS/Linux and http://scheme.localhost on Windows', () => {
    expect(protocolBase('pdfr', false)).toBe('pdfr://localhost');
    expect(protocolBase('pdfr', true)).toBe('http://pdfr.localhost');
  });
});

describe('buildRenderUrl', () => {
  const spec = {
    docId: 3,
    srcIndex: 12,
    rotation: 90 as const,
    scaleMilli: 1500,
    generation: 7,
    kind: 'page' as const,
  };

  it('encodes a whole-page render request', () => {
    expect(buildRenderUrl(spec, false)).toBe(
      'pdfr://localhost/render?doc=3&src=12&rot=90&scale=1500&gen=7&kind=page'
    );
  });

  it('encodes tile regions', () => {
    const url = buildRenderUrl(
      { ...spec, kind: 'tile', tile: { x: 1024, y: 0, w: 1024, h: 512 } },
      false
    );
    expect(url).toBe(
      'pdfr://localhost/render?doc=3&src=12&rot=90&scale=1500&gen=7&kind=tile&tx=1024&ty=0&tw=1024&th=512'
    );
  });

  it('encodes thumbnails on the windows base', () => {
    const url = buildRenderUrl({ ...spec, kind: 'thumb', scaleMilli: 200 }, true);
    expect(url).toBe('http://pdfr.localhost/render?doc=3&src=12&rot=90&scale=200&gen=7&kind=thumb');
  });

  it('encodes a preview crop', () => {
    const url = buildRenderUrl(
      { ...spec, kind: 'preview', scaleMilli: 2000, tile: { x: 20, y: 40, w: 840, h: 600 } },
      false
    );
    expect(url).toBe(
      'pdfr://localhost/render?doc=3&src=12&rot=90&scale=2000&gen=7&kind=preview&tx=20&ty=40&tw=840&th=600'
    );
  });
});

describe('retryUrl', () => {
  const url = 'pdfr://localhost/render?doc=3&src=12&rot=90&scale=1500&gen=7&kind=page';

  it('returns the URL untouched for the first attempt', () => {
    expect(retryUrl(url, 0)).toBe(url);
  });

  it('appends a distinct cache-buster per attempt', () => {
    expect(retryUrl(url, 1)).toBe(`${url}&retry=1`);
    expect(retryUrl(url, 2)).toBe(`${url}&retry=2`);
  });

  it('leaves the render query itself intact so the engine cache key is unchanged', () => {
    // The Rust parser ignores unknown params; a retry must not become a
    // different raster. Guarded on the Rust side by
    // `cache_busting_retry_param_does_not_change_the_render_key`.
    const [base, query] = retryUrl(url, 1).split('?');
    expect(base).toBe('pdfr://localhost/render');
    const params = new URLSearchParams(query);
    params.delete('retry');
    expect(params.toString()).toBe(new URL(url).searchParams.toString());
  });
});

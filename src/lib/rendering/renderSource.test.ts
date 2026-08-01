import { describe, expect, it } from 'vitest';
import { buildRenderUrl, protocolBase } from './renderSource';

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

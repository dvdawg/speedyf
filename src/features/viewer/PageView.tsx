/** One mounted page: progressive raster (preview → full → tiles), text
 * selection layer, search highlights, and the annotation overlay.
 *
 * Overlays live inside an UNROTATED "page space" container that is rotated
 * into place with a single CSS transform, so overlay math never deals with
 * rotation — only the pointer-input path does (via lib/coordinates).
 */
import {
  createEffect,
  createMemo,
  createResource,
  createSignal,
  For,
  onCleanup,
  Show,
} from 'solid-js';
import type { Layout } from '../../lib/coordinates/layout';
import type { PageGeom } from '../../lib/coordinates/coords';
import { tileGrid } from '../../lib/coordinates/coords';
import { renderUrl } from '../../lib/rendering/renderSource';
import { documentStore } from '../document/documentStore';
import { viewport } from '../../stores/viewportStore';
import { engine } from '../../lib/transport/engine';
import { searchStore } from '../search/searchStore';
import TextLayer from './TextLayer';
import AnnotationLayer from '../annotations/AnnotationLayer';
import type { Rotation } from '../../types/model';

// Keep single PDFium jobs bounded. Normal letter pages remain whole-page at
// typical Retina zoom, while large/complex sheets switch to ~1Mpx tiles.
const TILE_THRESHOLD_PX = 4_000_000;
const TILE_SIZE = 1024;

interface Props {
  index: number;
  layout: Layout;
  geom: PageGeom;
  scaleMilli: number;
}

export default function PageView(props: Props) {
  const doc = documentStore.state;
  const page = () => doc.pages[props.index];
  const srcIndex = () => page()?.srcIndex ?? null;

  const cssW = () => props.layout.widths[props.index] ?? 0;
  const cssH = () => props.layout.heights[props.index] ?? 0;
  const top = () => props.layout.tops[props.index] ?? 0;
  const left = () => props.layout.lefts[props.index] ?? 0;

  // rotated (display) size in device pixels at the target render scale
  const rotW = () => (props.geom.rotation % 180 === 0 ? props.geom.widthPt : props.geom.heightPt);
  const rotH = () => (props.geom.rotation % 180 === 0 ? props.geom.heightPt : props.geom.widthPt);
  const devW = () => Math.max(1, Math.round((rotW() * props.scaleMilli) / 1000));
  const devH = () => Math.max(1, Math.round((rotH() * props.scaleMilli) / 1000));
  const tiled = () => devW() * devH() > TILE_THRESHOLD_PX;

  const urlFor = (
    kind: 'page' | 'thumb' | 'tile',
    scaleMilli: number,
    tile?: { x: number; y: number; w: number; h: number }
  ) => {
    const src = srcIndex();
    if (src === null) return null;
    return renderUrl({
      docId: doc.docId,
      srcIndex: src,
      rotation: props.geom.rotation as Rotation,
      scaleMilli,
      generation: doc.generation,
      kind,
      ...(tile ? { tile } : {}),
    });
  };

  /** Low-res preview for ordinary pages. Tiled sheets intentionally skip a
   * whole-page preview: a dense CAD preview can itself monopolize PDFium for
   * hundreds of milliseconds while useful visible tiles wait behind it. */
  const previewUrl = createMemo(() => (tiled() ? null : urlFor('page', 250)));

  /** whole-page raster URL (non-tiled mode) */
  const fullUrl = createMemo(() => {
    if (tiled() || srcIndex() === null) return null;
    return urlFor('page', props.scaleMilli);
  });

  const allTiles = createMemo(() => (tiled() ? tileGrid(devW(), devH(), TILE_SIZE) : []));

  /** Visible tiles only (plus margin) in both axes. `allTiles()` owns stable
   * object identities, so scrolling filters references instead of recreating
   * every tile DOM node. */
  const visibleTiles = createMemo(() => {
    if (!tiled()) return [];
    const scaleX = devW() / Math.max(1, cssW());
    const scaleY = devH() / Math.max(1, cssH());
    const margin = 768;
    const x0 = Math.max(0, (viewport.scrollLeft - left()) * scaleX - margin);
    const x1 = Math.min(
      devW(),
      (viewport.scrollLeft + viewport.containerW - left()) * scaleX + margin
    );
    const y0 = Math.max(0, (viewport.scrollTop - top()) * scaleY - margin);
    const y1 = Math.min(
      devH(),
      (viewport.scrollTop + viewport.containerH - top()) * scaleY + margin
    );
    if (x1 <= 0 || x0 >= devW() || y1 <= 0 || y0 >= devH()) return [];
    return allTiles().filter(
      (tile) => tile.x + tile.w >= x0 && tile.x <= x1 && tile.y + tile.h >= y0 && tile.y <= y1
    );
  });

  // Progressive swap: keep the last loaded raster on screen until the newer
  // one has actually decoded (the browser CSS-scales the old one meanwhile).
  const [displayed, setDisplayed] = createSignal<string | null>(null);
  const [renderError, setRenderError] = createSignal(false);
  createEffect(() => {
    const url = fullUrl();
    if (!url || displayed() === url) return;
    setRenderError(false);
    let cancelled = false;
    const img = new Image();
    img.decoding = 'async';
    img.src = url;
    img.onload = () => {
      if (!cancelled) setDisplayed(url);
    };
    img.onerror = () => {
      if (!cancelled) setRenderError(true);
    };
    onCleanup(() => {
      cancelled = true;
      img.onload = null;
      img.onerror = null;
      img.src = '';
    });
  });

  // Text layout (runs) — fetched only while this page is mounted.
  const [textLayout] = createResource(
    () => (srcIndex() === null || !doc.loaded ? null : ([doc.docId, srcIndex()] as const)),
    async ([docId, src]) => {
      try {
        return await engine.getTextLayout(docId, src as number);
      } catch {
        return null;
      }
    }
  );

  const pageMatches = createMemo(() => {
    const src = srcIndex();
    if (src === null || searchStore.state.flat.length === 0) return [];
    return searchStore.rectsForPage(src);
  });

  // unrotated page-space container transform
  const spaceTransform = () => {
    const w = props.geom.widthPt * viewport.zoom;
    const h = props.geom.heightPt * viewport.zoom;
    switch (props.geom.rotation) {
      case 90:
        return `translate(${h}px, 0) rotate(90deg)`;
      case 180:
        return `translate(${w}px, ${h}px) rotate(180deg)`;
      case 270:
        return `translate(0, ${w}px) rotate(270deg)`;
      default:
        return '';
    }
  };

  return (
    <section
      class="page"
      style={{
        top: `${top()}px`,
        left: `${left()}px`,
        width: `${cssW()}px`,
        height: `${cssH()}px`,
      }}
      aria-label={`Page ${props.index + 1}`}
      data-page-index={props.index}
    >
      <Show
        when={srcIndex() !== null}
        fallback={<div class="page-blank" aria-label="Blank page" />}
      >
        <Show when={!displayed()}>
          <div class="page-loading" aria-hidden="true">
            <Show when={previewUrl()}>
              <img class="page-img" src={previewUrl()!} alt="" draggable={false} />
            </Show>
          </div>
        </Show>
        <Show when={displayed()}>
          <img class="page-img" src={displayed()!} alt="" draggable={false} />
        </Show>
        <Show when={tiled()}>
          <For each={visibleTiles()}>
            {(t) => {
              const factor = () => cssW() / devW();
              return (
                <img
                  class="page-tile"
                  src={urlFor('tile', props.scaleMilli, t)!}
                  alt=""
                  draggable={false}
                  onError={() => setRenderError(true)}
                  style={{
                    left: `${t.x * factor()}px`,
                    top: `${t.y * (cssH() / devH())}px`,
                    width: `${t.w * factor()}px`,
                    height: `${t.h * (cssH() / devH())}px`,
                  }}
                />
              );
            }}
          </For>
        </Show>
        <Show when={renderError()}>
          <div class="page-render-warning" role="status">
            Part of this page could not be rendered. Try another zoom level.
          </div>
        </Show>
      </Show>

      {/* unrotated page space: text, search highlights, annotations */}
      <div
        class="page-space"
        style={{
          width: `${props.geom.widthPt * viewport.zoom}px`,
          height: `${props.geom.heightPt * viewport.zoom}px`,
          transform: spaceTransform(),
        }}
      >
        <Show when={textLayout()}>
          <TextLayer
            runs={textLayout()!.runs}
            pageHeightPt={props.geom.heightPt}
            zoom={viewport.zoom}
          />
        </Show>
        <div class="search-highlights" aria-hidden="true">
          <For each={pageMatches()}>
            {(entry) => (
              <For each={entry.rects}>
                {(r) => (
                  <div
                    class="search-hit"
                    classList={{ 'is-current': entry.index === searchStore.state.current }}
                    style={{
                      left: `${r[0] * viewport.zoom}px`,
                      top: `${(props.geom.heightPt - r[1] - r[3]) * viewport.zoom}px`,
                      width: `${r[2] * viewport.zoom}px`,
                      height: `${r[3] * viewport.zoom}px`,
                    }}
                  />
                )}
              </For>
            )}
          </For>
        </div>
        <Show when={page()}>
          <AnnotationLayer
            page={page()!}
            geom={props.geom}
            zoom={viewport.zoom}
            runs={textLayout()?.runs ?? []}
          />
        </Show>
      </div>
    </section>
  );
}

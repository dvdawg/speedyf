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
import { RENDER_SCALE_BUCKETS, tileGrid } from '../../lib/coordinates/coords';
import { renderUrl } from '../../lib/rendering/renderSource';
import { documentStore } from '../document/documentStore';
import { viewport } from '../../stores/viewportStore';
import { engine } from '../../lib/transport/engine';
import { searchStore } from '../search/searchStore';
import TextLayer from './TextLayer';
import AnnotationLayer from '../annotations/AnnotationLayer';
import type { Rotation } from '../../types/model';

const TILE_THRESHOLD_PX = 16_700_000; // ~4096×4096 device pixels
const TILE_SIZE = 1024;
const BACKDROP_BUDGET_PX = 2_000_000;

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

  /** low-res preview: cheapest bucket, upscaled while the real render loads */
  const previewUrl = createMemo(() => urlFor('page', 250));

  /** whole-page raster URL (non-tiled mode) */
  const fullUrl = createMemo(() => {
    if (tiled() || srcIndex() === null) return null;
    return urlFor('page', props.scaleMilli);
  });

  /** backdrop scale for tiled mode: biggest bucket within the pixel budget */
  const backdropUrl = createMemo(() => {
    if (!tiled()) return null;
    let bucket = RENDER_SCALE_BUCKETS[0]!;
    for (const b of RENDER_SCALE_BUCKETS) {
      if (rotW() * b * rotH() * b <= BACKDROP_BUDGET_PX) bucket = b;
    }
    return urlFor('page', Math.round(bucket * 1000));
  });

  /** visible tiles only (plus margin), never the whole grid */
  const visibleTiles = createMemo(() => {
    if (!tiled()) return [];
    const scaleCssToDev = devH() / Math.max(1, cssH());
    const margin = 768;
    const y0 = Math.max(0, (viewport.scrollTop - top()) * scaleCssToDev - margin);
    const y1 = Math.min(
      devH(),
      (viewport.scrollTop + viewport.containerH - top()) * scaleCssToDev + margin
    );
    if (y1 <= 0 || y0 >= devH()) return [];
    return tileGrid(devW(), devH(), TILE_SIZE).filter((t) => t.y + t.h >= y0 && t.y <= y1);
  });

  // Progressive swap: keep the last loaded raster on screen until the newer
  // one has actually decoded (the browser CSS-scales the old one meanwhile).
  const [displayed, setDisplayed] = createSignal<string | null>(null);
  createEffect(() => {
    const url = fullUrl() ?? backdropUrl();
    if (!url || displayed() === url) return;
    let cancelled = false;
    const img = new Image();
    img.decoding = 'async';
    img.src = url;
    img.onload = () => {
      if (!cancelled) setDisplayed(url);
    };
    onCleanup(() => {
      cancelled = true;
      img.onload = null;
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
                  style={{
                    left: `${t.x * factor()}px`,
                    top: `${t.y * factor()}px`,
                    width: `${t.w * factor()}px`,
                    height: `${t.h * factor()}px`,
                  }}
                />
              );
            }}
          </For>
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

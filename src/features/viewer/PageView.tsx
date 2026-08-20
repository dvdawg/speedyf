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
  useContext,
} from 'solid-js';
import type { Layout } from '../../lib/coordinates/layout';
import type { PageGeom, TileRect } from '../../lib/coordinates/coords';
import { tileGrid } from '../../lib/coordinates/coords';
import { RENDER_RETRIES, renderUrl, retryUrl } from '../../lib/rendering/renderSource';
import { engine } from '../../lib/transport/engine';
import TextLayer from './TextLayer';
import AnnotationLayer from '../annotations/AnnotationLayer';
import CitationLayer from '../citations/CitationLayer';
import type { Rotation } from '../../types/model';
import { TabContext } from '../../app/TabContext';

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
  const tab = useContext(TabContext)!;
  const { viewport: vp, searchStore } = tab;
  const doc = tab.documentStore.state;
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
    const x0 = Math.max(0, (vp.state.scrollLeft - left()) * scaleX - margin);
    const x1 = Math.min(
      devW(),
      (vp.state.scrollLeft + vp.state.containerW - left()) * scaleX + margin
    );
    const y0 = Math.max(0, (vp.state.scrollTop - top()) * scaleY - margin);
    const y1 = Math.min(
      devH(),
      (vp.state.scrollTop + vp.state.containerH - top()) * scaleY + margin
    );
    if (x1 <= 0 || x0 >= devW() || y1 <= 0 || y0 >= devH()) return [];
    return allTiles().filter(
      (tile) => tile.x + tile.w >= x0 && tile.x <= x1 && tile.y + tile.h >= y0 && tile.y <= y1
    );
  });

  // Per-tile state, keyed by grid position. Rows drop their own keys on
  // dispose, so both sets only ever describe currently mounted tiles.
  const tileKey = (t: TileRect) => `${t.x},${t.y}`;
  const [loadedTiles, setLoadedTiles] = createSignal<ReadonlySet<string>>(new Set());
  const [failedTiles, setFailedTiles] = createSignal<ReadonlySet<string>>(new Set());
  const withTile = (set: ReadonlySet<string>, key: string, present: boolean) => {
    if (set.has(key) === present) return set;
    const next = new Set(set);
    if (present) next.add(key);
    else next.delete(key);
    return next;
  };

  /** Every tile the viewport currently wants has settled — decoded, or given
   * up on. A tiled page has no whole-page raster to swap in, so without this
   * its loading placeholder would shimmer under the tiles forever. */
  const tilesSettled = createMemo(() => {
    const tiles = visibleTiles();
    if (!tiled() || tiles.length === 0) return false;
    const loaded = loadedTiles();
    const failed = failedTiles();
    return tiles.every((t) => loaded.has(tileKey(t)) || failed.has(tileKey(t)));
  });

  // Progressive swap: keep the last loaded raster on screen until the newer
  // one has actually decoded (the browser CSS-scales the old one meanwhile).
  const [displayed, setDisplayed] = createSignal<string | null>(null);
  const [pageRenderError, setPageRenderError] = createSignal(false);
  /** Tiles clear their own failure on a later success, so the warning tracks
   * what is actually missing rather than latching on the first hole. */
  const renderError = createMemo(() => pageRenderError() || failedTiles().size > 0);
  /** The fullUrl() that displayed() satisfies. displayed() may carry a retry
   * suffix, so it is not comparable with fullUrl() directly. */
  let loadedFor: string | null = null;
  createEffect(() => {
    const url = fullUrl();
    if (!url || loadedFor === url) return;
    setPageRenderError(false);
    let cancelled = false;
    let attempt = 0;
    const img = new Image();
    img.decoding = 'async';
    img.onload = () => {
      if (cancelled) return;
      loadedFor = url;
      // Mount the URL that actually decoded: the webview holds those bytes,
      // where the plain URL would replay the empty response that failed.
      setDisplayed(img.src);
    };
    img.onerror = () => {
      if (cancelled) return;
      // An engine-side cancel answers 204, which reads here as a load error.
      // The URL is unchanged, so nothing else would ever ask again.
      if (attempt < RENDER_RETRIES) {
        attempt += 1;
        img.src = retryUrl(url, attempt);
        return;
      }
      setPageRenderError(true);
    };
    img.src = url;
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
    const w = props.geom.widthPt * vp.state.zoom;
    const h = props.geom.heightPt * vp.state.zoom;
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
        <Show when={!displayed() && !tilesSettled()}>
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
              const key = tileKey(t);
              const base = createMemo(() => urlFor('tile', props.scaleMilli, t)!);
              const [attempt, setAttempt] = createSignal(0);
              const record = (loaded: boolean, failed: boolean) => {
                setLoadedTiles((prev) => withTile(prev, key, loaded));
                setFailedTiles((prev) => withTile(prev, key, failed));
              };
              // A new URL for this tile (generation bump) is a fresh attempt.
              createEffect(() => {
                base();
                setAttempt(0);
                record(false, false);
              });
              onCleanup(() => record(false, false));
              return (
                <img
                  class="page-tile"
                  src={retryUrl(base(), attempt())}
                  alt=""
                  draggable={false}
                  onLoad={() => record(true, false)}
                  onError={() => {
                    if (attempt() < RENDER_RETRIES) {
                      setAttempt(attempt() + 1);
                      return;
                    }
                    record(false, true);
                  }}
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
          width: `${props.geom.widthPt * vp.state.zoom}px`,
          height: `${props.geom.heightPt * vp.state.zoom}px`,
          transform: spaceTransform(),
        }}
      >
        <Show when={textLayout()}>
          <TextLayer
            runs={textLayout()!.runs}
            pageHeightPt={props.geom.heightPt}
            zoom={vp.state.zoom}
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
                      left: `${r[0] * vp.state.zoom}px`,
                      top: `${(props.geom.heightPt - r[1] - r[3]) * vp.state.zoom}px`,
                      width: `${r[2] * vp.state.zoom}px`,
                      height: `${r[3] * vp.state.zoom}px`,
                    }}
                  />
                )}
              </For>
            )}
          </For>
        </div>
        <Show when={srcIndex() !== null}>
          <CitationLayer
            docId={doc.docId}
            src={srcIndex()!}
            pageHeightPt={props.geom.heightPt}
            zoom={vp.state.zoom}
          />
        </Show>
        <Show when={page()}>
          <AnnotationLayer
            page={page()!}
            geom={props.geom}
            zoom={vp.state.zoom}
            runs={textLayout()?.runs ?? []}
          />
        </Show>
      </div>
    </section>
  );
}

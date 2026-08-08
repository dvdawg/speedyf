import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { renderUrl } from '../../lib/rendering/renderSource';
import type { PreviewSpec } from '../../types/engine';
import { openInNewTabOrFocus } from '../document/tabsController';
import { activeTab } from '../../stores/tabsStore';
import type { TabRecord } from '../../stores/tabsStore';
import { citationLabel } from './citationLabel';
import type { HoverPreview } from './linkStore';
import { libraryStore } from './libraryStore';
import {
  positionPreviewPopover,
  previewContentSize,
  previewPopoverSize,
  previewRasterStyle,
  type PreviewPopoverVariant,
} from './previewLayout';

function urlFor(tab: TabRecord, preview: PreviewSpec): string {
  const doc = tab.documentStore.state;
  return renderUrl({
    docId: preview.docId,
    srcIndex: preview.src,
    rotation: 0,
    scaleMilli: preview.scaleMilli,
    generation: preview.docId === doc.docId ? doc.generation : 0,
    kind: 'preview',
    tile: preview.tile,
  });
}

function RasterCard(props: {
  tab: TabRecord;
  preview: PreviewSpec;
  alt: string;
  caption?: string;
  detail?: string;
  pageCount?: number;
  fullPage?: boolean;
  pageNumber?: number;
  activate: () => void;
}) {
  const url = createMemo(() => urlFor(props.tab, props.preview));
  const rasterStyle = createMemo(() => previewRasterStyle(props.preview));
  const [loaded, setLoaded] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  let rasterTimeout: number | undefined;
  const clearRasterTimeout = () => {
    if (rasterTimeout !== undefined) window.clearTimeout(rasterTimeout);
    rasterTimeout = undefined;
  };
  createEffect(() => {
    void url();
    setLoaded(false);
    setFailed(false);
    clearRasterTimeout();
    rasterTimeout = window.setTimeout(() => setFailed(true), 5_000);
    onCleanup(clearRasterTimeout);
  });
  const fallback = () => props.preview.text.trim() || props.detail || 'Preview unavailable.';

  return (
    <button
      type="button"
      class="citation-preview-action"
      classList={{ 'is-internal': props.pageNumber !== undefined }}
      onClick={() => props.activate()}
    >
      <div
        class="citation-preview-viewport"
        classList={{ 'is-scrollable': props.pageNumber !== undefined }}
      >
        <Show when={!failed()} fallback={<div class="citation-preview-text">{fallback()}</div>}>
          <Show when={!loaded()}>
            <div
              class="citation-preview-skeleton is-raster-placeholder"
              aria-label="Loading preview"
            />
          </Show>
          <img
            class="citation-preview-image"
            classList={{ 'is-full-page': props.fullPage === true, 'is-loaded': loaded() }}
            style={props.pageNumber !== undefined ? rasterStyle() : undefined}
            width={props.preview.tile.w}
            height={props.preview.tile.h}
            src={url()}
            alt={props.alt}
            draggable={false}
            onLoad={() => {
              clearRasterTimeout();
              setLoaded(true);
            }}
            onError={() => {
              clearRasterTimeout();
              setFailed(true);
              setLoaded(false);
            }}
          />
        </Show>
      </div>
      <Show when={props.pageNumber !== undefined}>
        <div class="citation-preview-page-number">Page {props.pageNumber}</div>
      </Show>
      <Show when={props.caption !== undefined}>
        <div class="citation-preview-caption">
          <strong>{props.caption}</strong>
          <Show when={props.detail}>
            <span>{props.detail}</span>
          </Show>
          <Show when={props.pageCount !== undefined}>
            <span>
              {props.pageCount} {props.pageCount === 1 ? 'page' : 'pages'}
            </span>
          </Show>
        </div>
      </Show>
    </button>
  );
}

function PreviewContent(props: { tab: TabRecord; result: HoverPreview }) {
  return (
    <>
      <Show when={props.result.kind === 'internal' ? props.result : null}>
        {(result) => (
          <RasterCard
            tab={props.tab}
            preview={result().preview}
            alt={`Preview of page ${result().page + 1}`}
            pageNumber={result().page + 1}
            activate={() => {
              props.tab.citationStore.navigateInternalTarget({
                kind: 'internal',
                page: result().page,
                x: result().x,
                y: result().y,
              });
              props.tab.citationStore.close();
            }}
          />
        )}
      </Show>
      <Show when={props.result.kind === 'external' ? props.result : null}>
        {(result) => (
          <RasterCard
            tab={props.tab}
            preview={result().resolved.preview}
            alt={`First page of ${result().resolved.title ?? result().resolved.fileName}`}
            caption={result().resolved.title ?? citationLabel(result().id)}
            detail={result().resolved.fileName}
            pageCount={result().resolved.pageCount}
            fullPage
            activate={() => {
              const path = result().resolved.path;
              props.tab.citationStore.close();
              void openInNewTabOrFocus(path);
            }}
          />
        )}
      </Show>
      <Show when={props.result.kind === 'external-unresolved' ? props.result : null}>
        {(result) => (
          <div class="citation-metadata-card">
            <strong>
              {result().id
                ? result().libraryRoot
                  ? result().libraryScanning
                    ? `Library scan in progress — ${citationLabel(result().id!)}`
                    : `Not in your library — ${citationLabel(result().id!)}`
                  : citationLabel(result().id!)
                : 'External link'}
            </strong>
            <Show when={!result().id}>
              <span class="citation-uri">{result().uri}</span>
            </Show>
            <Show when={result().id && !result().libraryRoot}>
              <span>No citation library folder is configured.</span>
              <button
                type="button"
                class="citation-library-action"
                onClick={() => void libraryStore.chooseLibraryFolder()}
              >
                Choose library folder…
              </button>
            </Show>
          </div>
        )}
      </Show>
      <Show when={props.result.kind === 'error' ? props.result : null}>
        {(result) => (
          <div class="citation-metadata-card citation-preview-error" role="status">
            <strong>{result().label}</strong>
            <span>{result().message || 'Preview unavailable.'}</span>
          </div>
        )}
      </Show>
    </>
  );
}

export default function PreviewPopover() {
  const hover = () => activeTab()?.citationStore.state.hover;
  const visible = () => {
    const phase = hover()?.phase;
    return phase !== undefined && ['loading', 'shown', 'leaving'].includes(phase);
  };
  const [viewportSize, setViewportSize] = createSignal({
    width: typeof window === 'undefined' ? 1_024 : window.innerWidth,
    height: typeof window === 'undefined' ? 768 : window.innerHeight,
  });
  onMount(() => {
    const resize = () => setViewportSize({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener('resize', resize);
    onCleanup(() => window.removeEventListener('resize', resize));
  });

  const variant = (): PreviewPopoverVariant =>
    hover()?.request?.target.kind === 'internal' ? 'internal' : 'default';

  const internalContentSize = () => {
    const result = hover()?.result;
    return result?.kind === 'internal' ? previewContentSize(result.preview) : undefined;
  };

  const popoverSize = createMemo(() =>
    previewPopoverSize(viewportSize(), variant(), internalContentSize())
  );

  const position = createMemo(() =>
    positionPreviewPopover(
      hover()?.request?.anchor,
      viewportSize(),
      variant(),
      internalContentSize()
    )
  );

  const style = createMemo(() => ({
    ...position(),
    ...(variant() === 'internal' ? { width: `${popoverSize().width}px` } : {}),
  }));

  return (
    <Show when={visible() ? activeTab() : null}>
      {(tab) => (
        <Portal>
          <section
            class="citation-popover"
            classList={{ 'is-internal': variant() === 'internal' }}
            style={style()}
            role="dialog"
            aria-label="Citation preview"
            onPointerEnter={() => tab().citationStore.enterPopover()}
            onPointerLeave={() => tab().citationStore.leave()}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault();
                tab().citationStore.close();
              }
            }}
          >
            <Show when={hover()?.phase === 'loading' && !hover()?.result && !hover()?.error}>
              <div class="citation-preview-skeleton is-popover" aria-label="Loading preview" />
            </Show>
            <Show when={hover()?.result}>
              {(result) => <PreviewContent tab={tab()} result={result()} />}
            </Show>
            <Show when={!hover()?.result && hover()?.error}>
              <div class="citation-metadata-card citation-preview-error" role="status">
                {hover()?.error || 'Preview unavailable.'}
              </div>
            </Show>
          </section>
        </Portal>
      )}
    </Show>
  );
}

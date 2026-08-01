import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { renderUrl } from '../../lib/rendering/renderSource';
import type { PreviewSpec } from '../../types/engine';
import { documentStore } from '../document/documentStore';
import { openPath } from '../document/controller';
import { citationLabel } from './citationLabel';
import { citationStore, navigateInternalTarget, type HoverPreview } from './linkStore';

function urlFor(preview: PreviewSpec): string {
  return renderUrl({
    docId: preview.docId,
    srcIndex: preview.src,
    rotation: 0,
    scaleMilli: preview.scaleMilli,
    generation: preview.docId === documentStore.state.docId ? documentStore.state.generation : 0,
    kind: 'preview',
    tile: preview.tile,
  });
}

function RasterCard(props: {
  preview: PreviewSpec;
  alt: string;
  caption: string;
  detail?: string;
  pageCount?: number;
  fullPage?: boolean;
  compactCaption?: boolean;
  activate: () => void;
}) {
  const url = createMemo(() => urlFor(props.preview));
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
    <button type="button" class="citation-preview-action" onClick={() => props.activate()}>
      <Show when={!failed()} fallback={<div class="citation-preview-text">{fallback()}</div>}>
        <Show when={!loaded()}>
          <div class="citation-preview-skeleton" aria-label="Loading preview" />
        </Show>
        <img
          class="citation-preview-image"
          classList={{ 'is-full-page': props.fullPage === true, 'is-loaded': loaded() }}
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
      <div
        class="citation-preview-caption"
        classList={{ 'is-compact': props.compactCaption === true }}
      >
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
    </button>
  );
}

function PreviewContent(props: { result: HoverPreview }) {
  return (
    <>
      <Show when={props.result.kind === 'internal' ? props.result : null}>
        {(result) => (
          <RasterCard
            preview={result().preview}
            alt={`Preview of page ${result().page + 1}`}
            caption={`Page ${result().page + 1}`}
            compactCaption
            activate={() => {
              navigateInternalTarget({
                kind: 'internal',
                page: result().page,
                x: result().x,
                y: result().y,
              });
              citationStore.close();
            }}
          />
        )}
      </Show>
      <Show when={props.result.kind === 'external' ? props.result : null}>
        {(result) => (
          <RasterCard
            preview={result().resolved.preview}
            alt={`First page of ${result().resolved.title ?? result().resolved.fileName}`}
            caption={result().resolved.title ?? citationLabel(result().id)}
            detail={result().resolved.fileName}
            pageCount={result().resolved.pageCount}
            fullPage
            activate={() => {
              const path = result().resolved.path;
              citationStore.close();
              void openPath(path);
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
                onClick={() => void citationStore.chooseLibraryFolder()}
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
  const hover = () => citationStore.state.hover;
  const visible = () => ['loading', 'shown', 'leaving'].includes(hover().phase);
  const [viewportSize, setViewportSize] = createSignal({
    width: typeof window === 'undefined' ? 1_024 : window.innerWidth,
    height: typeof window === 'undefined' ? 768 : window.innerHeight,
  });
  onMount(() => {
    const resize = () => setViewportSize({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener('resize', resize);
    onCleanup(() => window.removeEventListener('resize', resize));
  });

  const position = createMemo(() => {
    const anchor = hover().request?.anchor;
    if (!anchor) return { left: '8px', top: '8px' };
    const { width, height } = viewportSize();
    const popoverW = Math.min(420, Math.max(280, width - 16));
    const popoverH = Math.min(320, Math.max(180, height - 16));
    let left = anchor.right + 10;
    if (left + popoverW > width - 8) left = anchor.left - popoverW - 10;
    left = Math.max(8, Math.min(left, width - popoverW - 8));
    let top = anchor.top;
    if (top + popoverH > height - 8) top = anchor.bottom - popoverH;
    top = Math.max(8, Math.min(top, height - popoverH - 8));
    return { left: `${left}px`, top: `${top}px` };
  });

  return (
    <Show when={visible()}>
      <Portal>
        <section
          class="citation-popover"
          style={position()}
          role="dialog"
          aria-label="Citation preview"
          onPointerEnter={() => citationStore.enterPopover()}
          onPointerLeave={() => citationStore.leave()}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault();
              citationStore.close();
            }
          }}
        >
          <Show when={hover().phase === 'loading' && !hover().result && !hover().error}>
            <div class="citation-preview-skeleton is-popover" aria-label="Loading preview" />
          </Show>
          <Show when={hover().result}>{(result) => <PreviewContent result={result()} />}</Show>
          <Show when={!hover().result && hover().error}>
            <div class="citation-metadata-card citation-preview-error" role="status">
              {hover().error || 'Preview unavailable.'}
            </div>
          </Show>
        </section>
      </Portal>
    </Show>
  );
}

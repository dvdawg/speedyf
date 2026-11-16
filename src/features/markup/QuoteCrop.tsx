/** A comment's words as they are printed: a crop of its page, rendered by
 * the engine at the screen's resolution (dark paper when the pages are
 * shown inverted), tight around the anchor, its top shown and the rest
 * faded when it is long. The plain text quote is its alternative text.
 *
 * Falls back to `fallback` when the page cannot be rendered: no document
 * on screen (the drawer shows the review of the one that is), or the render
 * fails. */
import { createMemo, createSignal, onMount, Show, type JSX } from 'solid-js';
import { activeTab } from '../../stores/tabsStore';
import { pagesInverted } from '../../stores/settings';
import { renderUrl, retryUrl, RENDER_RETRIES } from '../../lib/rendering/renderSource';
import { cropSpec } from './cropGeometry';
import type { Anchor } from './types';

/** A card's quote column when it cannot be measured yet. */
const DEFAULT_WIDTH_PX = 280;
/** How much of a crop shows before it fades out, in CSS pixels. */
export const CROP_MAX_HEIGHT_PX = 132;

export default function QuoteCrop(props: { anchor: Anchor; alt: string; fallback: JSX.Element }) {
  let box: HTMLDivElement | undefined;
  /** The column the crop is fitted to, once laid out. */
  const [width, setWidth] = createSignal<number | null>(null);
  const [attempt, setAttempt] = createSignal(0);
  const [failed, setFailed] = createSignal(false);
  // The wrap's content box: its width less the rule and gap on its left.
  onMount(() => {
    const style = box ? getComputedStyle(box) : null;
    const inset = style
      ? (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.borderLeftWidth) || 0)
      : 0;
    setWidth((box?.clientWidth ?? 0) - inset > 0 ? box!.clientWidth - inset : DEFAULT_WIDTH_PX);
  });

  const crop = createMemo(() => {
    const w = width();
    const doc = activeTab()?.documentStore.state;
    if (w === null || !doc?.loaded) return null;
    const page = doc.pages.find((p) => p.srcIndex === props.anchor.page);
    if (!page) return null;
    // the displayed page, as anchors measure it
    const turned = page.baseRotation % 180 !== 0;
    const size = {
      widthPt: turned ? page.heightPt : page.widthPt,
      heightPt: turned ? page.widthPt : page.heightPt,
    };
    const dpr = typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1;
    const spec = cropSpec(props.anchor, size, w, dpr);
    if (!spec) return null;
    const url = renderUrl({
      docId: doc.docId,
      srcIndex: props.anchor.page,
      rotation: 0,
      scaleMilli: spec.scaleMilli,
      generation: doc.generation,
      kind: 'preview',
      tile: spec.tile,
      invert: pagesInverted(),
    });
    return { url, width: spec.width, height: spec.height };
  });

  return (
    <div ref={box} class="mk-crop-wrap">
      <Show when={width() !== null}>
        <Show when={!failed() && crop()} fallback={props.fallback}>
          {(c) => (
            <div
              class="mk-crop"
              classList={{ 'is-clipped': c().height > CROP_MAX_HEIGHT_PX }}
              style={{ 'max-height': `${CROP_MAX_HEIGHT_PX}px` }}
            >
              <img
                src={retryUrl(c().url, attempt())}
                alt={props.alt}
                loading="lazy"
                decoding="async"
                draggable={false}
                style={{
                  width: `${c().width}px`,
                  'aspect-ratio': `${c().width} / ${c().height}`,
                }}
                onError={() => {
                  if (attempt() < RENDER_RETRIES) setAttempt(attempt() + 1);
                  else setFailed(true);
                }}
              />
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
}

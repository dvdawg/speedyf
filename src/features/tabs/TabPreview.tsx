/** What a tab actually contains, on hover.
 *
 * A pill can only show a file name, and for a paper that name is usually
 * "2605.25567v3.pdf" — which identifies the file and tells you nothing about
 * it. The first page does: its title, its authors, its figures. So the preview
 * is a thumbnail of page one, rendered through the ordinary pdfr:// protocol,
 * which means it is cached and memory-bounded like every other raster rather
 * than being a second image path of its own. */
import { Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { renderUrl } from '../../lib/rendering/renderSource';
import type { TabRecord } from '../../stores/tabsStore';
import { tabPreviewPosition, type Anchor } from './tabPreviewLayout';

/** Scale for the thumbnail. Small enough to be cheap, large enough that a
 * paper's title is readable — which is the entire point of it. */
const THUMB_SCALE_MILLI = 700;
const CARD_WIDTH = 232;

export default function TabPreview(props: { tab: TabRecord; anchor: Anchor }) {
  const doc = () => props.tab.documentStore.state;

  const url = () => {
    const state = doc();
    if (!state.loaded) return null;
    // The first page a reader would actually see: a blank page inserted at
    // the front has no source to render, so look past it.
    const first = state.pages.find((page) => page.srcIndex !== null);
    if (!first || first.srcIndex === null) return null;
    return renderUrl({
      docId: state.docId,
      srcIndex: first.srcIndex,
      rotation: 0,
      scaleMilli: THUMB_SCALE_MILLI,
      generation: state.generation,
      kind: 'thumb',
    });
  };

  const position = () =>
    tabPreviewPosition(
      props.anchor,
      // Height is an estimate: the card is mostly a page, and a page is taller
      // than it is wide. Being a little off only shifts the clamp.
      { width: CARD_WIDTH, height: CARD_WIDTH * 1.45 },
      { width: window.innerWidth, height: window.innerHeight }
    );

  return (
    <Portal>
      <div
        class="tab-preview"
        role="tooltip"
        style={{ left: `${position().left}px`, top: `${position().top}px` }}
      >
        <Show when={url()} fallback={<div class="tab-preview-empty">Opening…</div>}>
          <img class="tab-preview-page" src={url()!} alt="" draggable={false} />
        </Show>
        <div class="tab-preview-meta">
          <span class="tab-preview-name">{doc().name || 'Untitled'}</span>
          <Show when={doc().pages.length > 0}>
            <span class="tab-preview-pages">
              {doc().pages.length} {doc().pages.length === 1 ? 'page' : 'pages'}
            </span>
          </Show>
        </div>
      </div>
    </Portal>
  );
}

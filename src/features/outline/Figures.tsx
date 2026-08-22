/** Figure panel: every figure, table and algorithm in the document, each shown
 * as a crop of its own artwork, click to jump.
 *
 * The reason this exists: a paper discusses Figure 4 several pages from where
 * Figure 4 sits, so following the prose means losing your place to go look.
 * The captions come from the same hyperref anchors the formal-environment
 * panel reads; the crop above each one is computed in engine/figures.rs.
 *
 * Thumbnails go through the ordinary render protocol, so they are cached,
 * priority-scheduled and memory-bounded exactly like page rasters — the panel
 * adds no image path of its own. */
import { createResource, For, Show, useContext } from 'solid-js';
import { engine } from '../../lib/transport/engine';
import { renderUrl } from '../../lib/rendering/renderSource';
import type { Figure } from '../../types/engine';
import { TabContext } from '../../app/TabContext';
import { figureTopY, jumpToAnchor, layoutIndexOf } from './jumpToAnchor';
import ScriptText from '../../components/ScriptText';

export default function Figures() {
  const tab = useContext(TabContext)!;
  const doc = tab.documentStore.state;
  const [figures] = createResource(
    () => (doc.loaded ? doc.docId : null),
    (docId) => engine.getFigures(docId).catch(() => [] as Figure[])
  );

  /** Where clicking the row should land: the top of the artwork, not the
   * caption anchor the figure was found by. Falls back to the caption when the
   * page is gone or the crop is unusable. */
  const targetY = (figure: Figure): number | null => {
    const index = layoutIndexOf(doc, figure.page);
    const heightPt = index === null ? undefined : doc.pages[index]?.heightPt;
    if (heightPt === undefined) return figure.y;
    return figureTopY(figure.tile.y, figure.scaleMilli, heightPt) ?? figure.y;
  };

  // The crop is in display space, so it is unaffected by the viewer's own
  // rotation — the thumbnail always shows the figure the way it is printed.
  const thumbUrl = (figure: Figure) =>
    renderUrl({
      docId: doc.docId,
      srcIndex: figure.page,
      rotation: 0,
      scaleMilli: figure.scaleMilli,
      generation: doc.generation,
      kind: 'preview',
      tile: figure.tile,
    });

  return (
    <div class="sidebar-scroll figure-panel" aria-label="Figures and tables">
      <Show when={!figures.loading} fallback={<div class="panel-note">Finding figures…</div>}>
        <Show
          when={(figures() ?? []).length > 0}
          fallback={<div class="panel-note">No figures or tables extractable.</div>}
        >
          <For each={figures()}>
            {(figure) => (
              <button
                type="button"
                class="figure-row"
                onClick={() => jumpToAnchor(tab.viewport, doc, figure.page, targetY(figure))}
              >
                <img
                  class="figure-thumb"
                  src={thumbUrl(figure)}
                  alt=""
                  loading="lazy"
                  draggable={false}
                />
                <span class="figure-label">
                  <ScriptText value={figure.label} />
                  <span class="figure-page">page {figure.page + 1}</span>
                </span>
                <Show when={figure.title}>
                  <span class="figure-caption">
                    <ScriptText value={figure.title} />
                  </span>
                </Show>
              </button>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}

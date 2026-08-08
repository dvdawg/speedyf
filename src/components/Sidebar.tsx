/** Thumbnail sidebar: virtualized (only near-viewport thumbs mount an <img>),
 * current-page tracking, click navigation, and edit-mode page operations
 * (drag reorder, rotate, duplicate, delete, add blank page). */
import { createMemo, createSignal, For, onCleanup, onMount, Show, useContext } from 'solid-js';
import { renderUrl } from '../lib/rendering/renderSource';
import type { Layout } from '../lib/coordinates/layout';
import { visibleRange } from '../lib/coordinates/layout';
import IconButton from './IconButton';
import {
  IconChevronDown,
  IconChevronUp,
  IconDuplicate,
  IconPlus,
  IconRotate,
  IconTrash,
} from './icons';
import { addBlankPageAfter } from '../features/editor/editorActions';
import type { Rotation } from '../types/model';
import { TabContext } from '../app/TabContext';

const THUMB_W = 148;
const SLOT_PAD = 30; // label + spacing
const GAP = 10;

export default function Sidebar() {
  const tab = useContext(TabContext)!;
  const { documentStore, viewport: vp } = tab;
  const requestScrollToPage = vp.requestScrollToPage;
  const doc = documentStore.state;
  let scroller!: HTMLDivElement;
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewH, setViewH] = createSignal(600);
  const [dragIndex, setDragIndex] = createSignal<number | null>(null);
  const [dropIndex, setDropIndex] = createSignal<number | null>(null);

  onMount(() => {
    setViewH(scroller.clientHeight || 600);
    const ro = new ResizeObserver((es) => {
      const h = es[0]?.contentRect.height;
      if (h) setViewH(h);
    });
    ro.observe(scroller);
    onCleanup(() => ro.disconnect());
  });

  const geom = createMemo(() =>
    doc.pages.map((p) => {
      const rot = (p.baseRotation + p.userRotation + vp.state.viewRotation) % 360;
      const [w, h] = rot % 180 === 0 ? [p.widthPt, p.heightPt] : [p.heightPt, p.widthPt];
      return { thumbH: (h / w) * THUMB_W, rot: rot as Rotation, srcIndex: p.srcIndex, id: p.id };
    })
  );

  const layout = createMemo<Layout>(() => {
    const tops: number[] = [];
    const heights: number[] = [];
    let y = 12;
    for (const g of geom()) {
      tops.push(y);
      const slotH = g.thumbH + SLOT_PAD;
      heights.push(slotH);
      y += slotH + GAP;
    }
    return {
      tops,
      heights,
      lefts: [],
      widths: [],
      totalH: y + 12,
      totalW: THUMB_W,
    };
  });

  const range = createMemo(() => visibleRange(layout(), scrollTop(), viewH(), viewH()));
  const indices = createMemo(() => {
    const [a, b] = range();
    return Array.from({ length: Math.max(0, b - a + 1) }, (_, k) => a + k);
  });

  const thumbUrl = (i: number) => {
    const g = geom()[i];
    if (!g || g.srcIndex === null) return null;
    const p = doc.pages[i]!;
    const rotW = g.rot % 180 === 0 ? p.widthPt : p.heightPt;
    const scale = (THUMB_W * vp.state.dpr) / rotW;
    const scaleMilli = Math.max(20, Math.round((scale * 1000) / 10) * 10);
    return renderUrl({
      docId: doc.docId,
      srcIndex: g.srcIndex,
      rotation: g.rot,
      scaleMilli,
      generation: doc.generation,
      kind: 'thumb',
    });
  };

  const reorderTo = (from: number, to: number) => {
    if (from === to || from + 1 === to) return;
    const page = doc.pages[from];
    if (!page) return;
    const target = to > from ? to - 1 : to;
    documentStore.apply({ type: 'reorder', pageId: page.id, toIndex: target });
  };

  return (
    <aside class="sidebar" aria-label="Page thumbnails">
      <div class="sidebar-scroll" ref={scroller} onScroll={() => setScrollTop(scroller.scrollTop)}>
        <div style={{ height: `${layout().totalH}px`, position: 'relative' }}>
          <For each={indices()}>
            {(i) => {
              const g = () => geom()[i];
              return (
                <Show when={g()}>
                  <div
                    class="thumb-slot"
                    classList={{
                      'is-current': vp.state.currentPage === i,
                      'is-drop-target': dropIndex() === i,
                    }}
                    style={{ top: `${layout().tops[i]}px`, height: `${layout().heights[i]}px` }}
                    draggable={vp.state.editMode}
                    onDragStart={(e) => {
                      setDragIndex(i);
                      e.dataTransfer!.effectAllowed = 'move';
                      e.dataTransfer!.setData('text/plain', String(i));
                    }}
                    onDragOver={(e) => {
                      if (dragIndex() === null) return;
                      e.preventDefault();
                      setDropIndex(i);
                    }}
                    onDragLeave={() => setDropIndex((d) => (d === i ? null : d))}
                    onDrop={(e) => {
                      e.preventDefault();
                      const from = dragIndex();
                      setDragIndex(null);
                      setDropIndex(null);
                      if (from !== null) reorderTo(from, i);
                    }}
                    onDragEnd={() => {
                      setDragIndex(null);
                      setDropIndex(null);
                    }}
                  >
                    <button
                      type="button"
                      class="thumb-btn"
                      style={{ height: `${g()!.thumbH}px` }}
                      onClick={() => requestScrollToPage(i)}
                      aria-label={`Go to page ${i + 1}`}
                      aria-current={vp.state.currentPage === i ? 'page' : undefined}
                    >
                      <Show
                        when={thumbUrl(i)}
                        fallback={<div class="thumb-blank" aria-hidden="true" />}
                      >
                        <img src={thumbUrl(i)!} alt="" loading="lazy" draggable={false} />
                      </Show>
                    </button>
                    <div class="thumb-meta">
                      <span class="thumb-num">{i + 1}</span>
                      <Show when={vp.state.editMode}>
                        <span class="thumb-ops">
                          <IconButton
                            label={`Move page ${i + 1} up`}
                            disabled={i === 0}
                            onClick={() =>
                              documentStore.apply({
                                type: 'reorder',
                                pageId: doc.pages[i]!.id,
                                toIndex: i - 1,
                              })
                            }
                          >
                            <IconChevronUp />
                          </IconButton>
                          <IconButton
                            label={`Move page ${i + 1} down`}
                            disabled={i >= doc.pages.length - 1}
                            onClick={() =>
                              documentStore.apply({
                                type: 'reorder',
                                pageId: doc.pages[i]!.id,
                                toIndex: i + 1,
                              })
                            }
                          >
                            <IconChevronDown />
                          </IconButton>
                          <IconButton
                            label={`Rotate page ${i + 1} 90°`}
                            onClick={() =>
                              documentStore.apply({
                                type: 'rotate',
                                pageId: doc.pages[i]!.id,
                                delta: 90,
                              })
                            }
                          >
                            <IconRotate />
                          </IconButton>
                          <IconButton
                            label={`Duplicate page ${i + 1}`}
                            onClick={() =>
                              documentStore.apply({ type: 'duplicate', pageId: doc.pages[i]!.id })
                            }
                          >
                            <IconDuplicate />
                          </IconButton>
                          <IconButton
                            label={`Delete page ${i + 1}`}
                            disabled={doc.pages.length <= 1}
                            onClick={() =>
                              documentStore.apply({ type: 'delete', pageId: doc.pages[i]!.id })
                            }
                          >
                            <IconTrash />
                          </IconButton>
                        </span>
                      </Show>
                    </div>
                  </div>
                </Show>
              );
            }}
          </For>
        </div>
      </div>
      <Show when={vp.state.editMode}>
        <div class="sidebar-footer">
          <button
            type="button"
            class="secondary-btn"
            onClick={() => addBlankPageAfter(documentStore, vp.state.currentPage)}
          >
            <IconPlus /> Blank page
          </button>
        </div>
      </Show>
    </aside>
  );
}

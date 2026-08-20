/** Always-rendered tab strip: a Home button plus one pill per open tab.
 *
 * Pills shrink to fit, then the strip scrolls. Shrinking alone was the
 * original design, but a pill narrow enough to fit twenty of them shows about
 * four characters — and for papers those four characters are the start of an
 * arXiv id, which distinguishes nothing. So pills stop shrinking while they
 * are still readable and the strip scrolls past that, with the active tab kept
 * in view. Hovering one shows its first page (TabPreview), which is the only
 * thing that reliably says what a paper is.
 *
 * Drag-to-reorder mirrors Sidebar.tsx's page-thumbnail DnD pattern. */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose, IconHome } from '../../components/icons';
import { tabsStore } from '../../stores/tabsStore';
import { activateTab, closeTab, goHome, reorderTab } from '../document/tabsController';
import TabPreview from './TabPreview';
import type { Anchor } from './tabPreviewLayout';

/** Long enough that sweeping across the strip does not flash cards, short
 * enough that a deliberate hover feels answered. */
const HOVER_DELAY_MS = 350;

export default function TabStrip() {
  const [dragIndex, setDragIndex] = createSignal<number | null>(null);
  const [dropIndex, setDropIndex] = createSignal<number | null>(null);
  const [hovered, setHovered] = createSignal<{ id: string; anchor: Anchor } | null>(null);
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  let strip!: HTMLDivElement;

  const clearHover = () => {
    clearTimeout(hoverTimer);
    setHovered(null);
  };
  onCleanup(() => clearTimeout(hoverTimer));

  const hoverTab = (id: string, element: HTMLElement) => {
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      const box = element.getBoundingClientRect();
      setHovered({ id, anchor: { left: box.left, right: box.right, bottom: box.bottom } });
    }, HOVER_DELAY_MS);
  };

  // A tab activated from anywhere — a shortcut, a citation, a reopened session
  // — may be scrolled out of sight.
  createEffect(() => {
    const id = tabsStore.state.activeId;
    if (!id || !strip) return;
    const pill = strip.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"]`);
    pill?.scrollIntoView({ inline: 'nearest', block: 'nearest' });
  });

  // A mouse without a horizontal wheel still has to reach the far tabs.
  const onWheel = (e: WheelEvent) => {
    if (e.deltaX !== 0 || e.shiftKey) return;
    const target = e.currentTarget as HTMLElement;
    if (target.scrollWidth <= target.clientWidth) return;
    e.preventDefault();
    target.scrollLeft += e.deltaY;
  };

  const reorderTo = (from: number, to: number) => {
    if (from === to || from + 1 === to) return;
    const target = to > from ? to - 1 : to;
    reorderTab(from, target);
  };

  return (
    <div class="tab-strip" role="tablist" aria-label="Open documents">
      <IconButton label="Home" onClick={goHome}>
        <IconHome />
      </IconButton>
      <div class="tab-strip-tabs" ref={strip} onWheel={onWheel} onScroll={clearHover}>
        <For each={tabsStore.state.tabs}>
          {(tab, i) => {
            const doc = () => tab.documentStore.state;
            const isActive = () => tabsStore.state.activeId === tab.id;
            return (
              <div
                class="tab-pill"
                classList={{ 'is-active': isActive(), 'is-drop-target': dropIndex() === i() }}
                role="tab"
                aria-selected={isActive()}
                title={doc().path ?? doc().name}
                data-tab-id={tab.id}
                draggable={true}
                onClick={() => activateTab(tab.id)}
                onPointerEnter={(e) => hoverTab(tab.id, e.currentTarget)}
                onPointerLeave={clearHover}
                onDragStart={(e) => {
                  clearHover();
                  setDragIndex(i());
                  e.dataTransfer!.effectAllowed = 'move';
                  e.dataTransfer!.setData('text/plain', String(i()));
                }}
                onDragOver={(e) => {
                  if (dragIndex() === null) return;
                  e.preventDefault();
                  setDropIndex(i());
                }}
                onDragLeave={() => setDropIndex((d) => (d === i() ? null : d))}
                onDrop={(e) => {
                  e.preventDefault();
                  const from = dragIndex();
                  setDragIndex(null);
                  setDropIndex(null);
                  if (from !== null) reorderTo(from, i());
                }}
                onDragEnd={() => {
                  setDragIndex(null);
                  setDropIndex(null);
                }}
              >
                <span class="tab-pill-name">{doc().name || 'Untitled'}</span>
                {doc().dirty && (
                  <span class="tab-pill-dirty" title="Unsaved changes">
                    ●
                  </span>
                )}
                <button
                  type="button"
                  class="icon-btn tab-pill-close"
                  title={`Close ${doc().name || 'tab'}`}
                  aria-label={`Close ${doc().name || 'tab'}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    clearHover();
                    void closeTab(tab.id);
                  }}
                >
                  <IconClose />
                </button>
              </div>
            );
          }}
        </For>
      </div>
      <Show when={hovered()}>
        {(state) => (
          <Show when={tabsStore.state.tabs.find((t) => t.id === state().id)}>
            {(tab) => <TabPreview tab={tab()} anchor={state().anchor} />}
          </Show>
        )}
      </Show>
    </div>
  );
}

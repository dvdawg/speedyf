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
 * Captured pointer gestures reorder in the strip or detach outside it. */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import IconButton from '../../components/IconButton';
import { IconClose, IconHome, IconNewWindow, IconDetachTab } from '../../components/icons';
import { newWindow } from '../windows/windowController';
import { isOutsideTabStrip, isTabDrag } from './tabDrag';
import { tabsStore } from '../../stores/tabsStore';
import { activateTab, closeTab, goHome, reorderTab, detachTab } from '../document/tabsController';
import TabPreview from './TabPreview';
import type { Anchor } from './tabPreviewLayout';

/** Long enough that sweeping across the strip does not flash cards, short
 * enough that a deliberate hover feels answered. */
const HOVER_DELAY_MS = 350;

export default function TabStrip() {
  const [dragId, setDragId] = createSignal<string | null>(null);
  const [dragPosition, setDragPosition] = createSignal({ left: 0, top: 0, width: 0 });
  const [detachHint, setDetachHint] = createSignal(false);
  let gesture: {
    id: string;
    x: number;
    y: number;
    offsetX: number;
    offsetY: number;
    width: number;
    pointer: number;
    element: HTMLElement;
  } | null = null;
  let suppressClick = false;
  let stripRow!: HTMLDivElement;
  const [dropIndex, setDropIndex] = createSignal<number | null>(null);
  const [hovered, setHovered] = createSignal<{ id: string; anchor: Anchor } | null>(null);
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  let strip!: HTMLDivElement;

  const clearHover = () => {
    clearTimeout(hoverTimer);
    setHovered(null);
  };
  const cancelDrag = () => {
    const previous = gesture;
    gesture = null;
    document.documentElement.classList.remove('is-dragging-tab');
    document.removeEventListener('selectstart', preventSelection, true);
    document.removeEventListener('dragstart', preventSelection, true);
    if (previous?.element.hasPointerCapture(previous.pointer))
      previous.element.releasePointerCapture(previous.pointer);
    setDragId(null);
    setDropIndex(null);
    setDetachHint(false);
  };
  // Capturing a pointer routes its events; it does not cancel the browser's
  // selection gesture. The PDF text layer explicitly enables selection, so
  // lock it for the entire press, including before the drag threshold.
  const preventSelection = (event: Event) => event.preventDefault();
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && gesture) {
      event.preventDefault();
      event.stopImmediatePropagation();
      suppressClick = true;
      cancelDrag();
    }
  };
  document.addEventListener('keydown', onKeyDown, true);
  window.addEventListener('blur', cancelDrag);
  onCleanup(() => {
    clearTimeout(hoverTimer);
    cancelDrag();
    document.removeEventListener('keydown', onKeyDown, true);
    window.removeEventListener('blur', cancelDrag);
  });

  const hoverTab = (id: string, element: HTMLElement) => {
    if (gesture) return;
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
    <div class="tab-strip" ref={stripRow} role="tablist" aria-label="Open documents">
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
                classList={{
                  'is-active': isActive(),
                  'is-drop-target': dropIndex() === i(),
                  'is-dragging': dragId() === tab.id,
                }}
                role="tab"
                aria-selected={isActive()}
                title={doc().path ?? doc().name}
                data-tab-id={tab.id}
                tabIndex={isActive() ? 0 : -1}
                onClick={() => {
                  if (suppressClick) {
                    suppressClick = false;
                    return;
                  }
                  activateTab(tab.id);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    activateTab(tab.id);
                  }
                }}
                onPointerEnter={(e) => hoverTab(tab.id, e.currentTarget)}
                onPointerLeave={clearHover}
                onPointerDown={(e) => {
                  if (
                    e.button !== 0 ||
                    (e.target as HTMLElement).closest('button') ||
                    tab.opening ||
                    doc().saving
                  )
                    return;
                  e.preventDefault();
                  cancelDrag();
                  clearHover();
                  suppressClick = false;
                  const box = e.currentTarget.getBoundingClientRect();
                  gesture = {
                    id: tab.id,
                    x: e.clientX,
                    y: e.clientY,
                    offsetX: e.clientX - box.left,
                    offsetY: e.clientY - box.top,
                    width: box.width,
                    pointer: e.pointerId,
                    element: e.currentTarget,
                  };
                  document.documentElement.classList.add('is-dragging-tab');
                  document.addEventListener('selectstart', preventSelection, true);
                  document.addEventListener('dragstart', preventSelection, true);
                  e.currentTarget.setPointerCapture(e.pointerId);
                }}
                onPointerMove={(e) => {
                  if (!gesture || gesture.pointer !== e.pointerId) return;
                  e.preventDefault();
                  if (!dragId() && !isTabDrag(gesture, e.clientX, e.clientY)) return;
                  if (!dragId()) window.getSelection()?.removeAllRanges();
                  setDragPosition({
                    left: e.clientX - gesture.offsetX,
                    top: e.clientY - gesture.offsetY,
                    width: gesture.width,
                  });
                  setDragId(gesture.id);
                  suppressClick = true;
                  const outside = isOutsideTabStrip(
                    stripRow.getBoundingClientRect(),
                    e.clientX,
                    e.clientY
                  );
                  setDetachHint(outside);
                  if (outside) {
                    setDropIndex(null);
                    return;
                  }
                  // Scroll the strip near its edges, then pick an insertion gap.
                  const bounds = strip.getBoundingClientRect();
                  if (e.clientX < bounds.left + 24) strip.scrollLeft -= 16;
                  else if (e.clientX > bounds.right - 24) strip.scrollLeft += 16;
                  const pills = [...strip.querySelectorAll<HTMLElement>('[data-tab-id]')];
                  const index = pills.findIndex((pill) => {
                    const box = pill.getBoundingClientRect();
                    return e.clientX < box.left + box.width / 2;
                  });
                  setDropIndex(index < 0 ? pills.length : index);
                }}
                onPointerUp={(e) => {
                  if (!gesture || gesture.pointer !== e.pointerId) return;
                  const id = gesture.id;
                  const dragged = dragId() !== null;
                  const detach =
                    dragged &&
                    isOutsideTabStrip(stripRow.getBoundingClientRect(), e.clientX, e.clientY);
                  const target = dropIndex();
                  cancelDrag();
                  if (detach) void detachTab(id, [e.screenX - 120, e.screenY - 20]);
                  else if (dragged && target !== null) {
                    const from = tabsStore.state.tabs.findIndex((t) => t.id === id);
                    if (from >= 0) reorderTo(from, target);
                  }
                }}
                onPointerCancel={cancelDrag}
                onLostPointerCapture={cancelDrag}
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
      <IconButton
        label="Move active tab to new window"
        disabled={!tabsStore.state.activeId}
        onClick={() => {
          const id = tabsStore.state.activeId;
          if (id) void detachTab(id);
        }}
      >
        <IconDetachTab />
      </IconButton>
      <IconButton label="New window" onClick={() => void newWindow()}>
        <IconNewWindow />
      </IconButton>
      <Show when={tabsStore.state.tabs.find((tab) => tab.id === dragId())}>
        {(tab) => (
          <Portal>
            <div
              class="tab-drag-preview"
              aria-hidden="true"
              style={{ left: `${dragPosition().left}px`, top: `${dragPosition().top}px` }}
            >
              <div
                class="tab-pill"
                classList={{ 'is-active': tabsStore.state.activeId === tab().id }}
                style={{ width: `${dragPosition().width}px` }}
              >
                <span class="tab-pill-name">{tab().documentStore.state.name || 'Untitled'}</span>
                <Show when={tab().documentStore.state.dirty}>
                  <span class="tab-pill-dirty">●</span>
                </Show>
                <span class="icon-btn tab-pill-close">
                  <IconClose />
                </span>
              </div>
              <Show when={detachHint()}>
                <div class="tab-detach-hint">Release to move tab to a new window</div>
              </Show>
            </div>
          </Portal>
        )}
      </Show>
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

/** Always-rendered tab strip: a Home button plus one pill per open tab.
 * Pills shrink to fit (pure flexbox, no JS width math) as more tabs open,
 * per the "Chrome-style shrink rather than scroll/overflow" spec. Drag-to-
 * reorder mirrors Sidebar.tsx's page-thumbnail DnD pattern. */
import { createSignal, For } from 'solid-js';
import IconButton from '../../components/IconButton';
import { IconClose, IconHome } from '../../components/icons';
import { tabsStore } from '../../stores/tabsStore';
import { activateTab, closeTab, goHome, reorderTab } from '../document/tabsController';

export default function TabStrip() {
  const [dragIndex, setDragIndex] = createSignal<number | null>(null);
  const [dropIndex, setDropIndex] = createSignal<number | null>(null);

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
      <div class="tab-strip-tabs">
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
                draggable={true}
                onClick={() => activateTab(tab.id)}
                onDragStart={(e) => {
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
    </div>
  );
}

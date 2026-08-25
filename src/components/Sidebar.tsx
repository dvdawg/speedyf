/** Left panel shell: a small mode toggle (page thumbnails, table of contents,
 * formal environments, figures, or your own notes) plus whichever content is
 * currently selected. */
import { For, Show } from 'solid-js';
import IconButton from './IconButton';
import { IconFigure, IconList, IconNote, IconPages, IconQed } from './icons';
import { ui, setUi, extensionPanelId, resetSidebarWidth, setSidebarWidth } from '../stores/uiStore';
import { extensionPanels } from '../features/extensions/extensionStore';
import ExtensionPanelView from '../features/extensions/ExtensionPanel';
import PageThumbnails from '../features/outline/PageThumbnails';
import Outline from '../features/outline/Outline';
import FormalEnvs from '../features/outline/FormalEnvs';
import Figures from '../features/outline/Figures';
import NotesPanel from '../features/annotations/NotesPanel';

export default function Sidebar() {
  let aside!: HTMLElement;

  /** Drag the trailing edge to resize.
   *
   * Width is taken from the pointer's position relative to the sidebar's own
   * left edge rather than accumulated from deltas, so the handle cannot drift
   * away from the cursor over a long drag — and so a clamp at either end does
   * not have to be "un-accumulated" when the pointer comes back.
   */
  const startResize = (e: PointerEvent) => {
    e.preventDefault();
    const left = aside.getBoundingClientRect().left;
    const move = (ev: PointerEvent) => setSidebarWidth(ev.clientX - left);
    const finish = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      document.body.classList.remove('is-resizing-sidebar');
    };
    // The document underneath is full of selectable text; without this a drag
    // that strays over it selects a paragraph on the way past.
    document.body.classList.add('is-resizing-sidebar');
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
  };

  /** Keyboard resizing, because a drag handle nobody can reach is not a
   * control. Arrows nudge, Home resets. */
  const onHandleKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 40 : 10;
    if (e.key === 'ArrowLeft') setSidebarWidth(ui.sidebarWidth - step);
    else if (e.key === 'ArrowRight') setSidebarWidth(ui.sidebarWidth + step);
    else if (e.key === 'Home') resetSidebarWidth();
    else return;
    e.preventDefault();
  };

  return (
    <aside
      class="sidebar"
      aria-label="Document navigation"
      ref={aside}
      style={{ width: `${ui.sidebarWidth}px` }}
    >
      <div class="sidebar-mode-toggle" role="tablist" aria-label="Sidebar view">
        <IconButton
          label="Pages"
          active={ui.sidebarMode === 'pages'}
          onClick={() => setUi('sidebarMode', 'pages')}
        >
          <IconPages />
        </IconButton>
        <IconButton
          label="Table of contents"
          active={ui.sidebarMode === 'outline'}
          onClick={() => setUi('sidebarMode', 'outline')}
        >
          <IconList />
        </IconButton>
        <IconButton
          label="Formal environments"
          active={ui.sidebarMode === 'formal'}
          onClick={() => setUi('sidebarMode', 'formal')}
        >
          <IconQed />
        </IconButton>
        <IconButton
          label="Figures and tables"
          active={ui.sidebarMode === 'figures'}
          onClick={() => setUi('sidebarMode', 'figures')}
        >
          <IconFigure />
        </IconButton>
        <IconButton
          label="Notes and annotations"
          active={ui.sidebarMode === 'notes'}
          onClick={() => setUi('sidebarMode', 'notes')}
        >
          <IconNote />
        </IconButton>
        {/* Contributed panels sit after every built-in, so installing an
            extension never moves a control the user already knows. */}
        <For each={extensionPanels()}>
          {(panel) => (
            <IconButton
              label={panel.title}
              active={extensionPanelId(ui.sidebarMode) === panel.id}
              onClick={() => setUi('sidebarMode', `ext:${panel.id}`)}
            >
              {/* Initials rather than a shared extension glyph: three
                  contributed panels drew three identical icons, and the only
                  way to tell them apart was to hover each one. */}
              <span class="ext-panel-initial" aria-hidden="true">
                {panel.title.slice(0, 2)}
              </span>
            </IconButton>
          )}
        </For>
      </div>
      <Show when={ui.sidebarMode === 'pages'}>
        <PageThumbnails />
      </Show>
      <Show when={ui.sidebarMode === 'outline'}>
        <Outline />
      </Show>
      <Show when={ui.sidebarMode === 'formal'}>
        <FormalEnvs />
      </Show>
      <Show when={ui.sidebarMode === 'figures'}>
        <Figures />
      </Show>
      <Show when={ui.sidebarMode === 'notes'}>
        <NotesPanel />
      </Show>
      <For each={extensionPanels()}>
        {(panel) => (
          <Show when={extensionPanelId(ui.sidebarMode) === panel.id}>
            <ExtensionPanelView panel={panel} />
          </Show>
        )}
      </For>
      <div
        class="sidebar-resize"
        role="separator"
        aria-label="Resize sidebar"
        aria-orientation="vertical"
        aria-valuenow={ui.sidebarWidth}
        tabindex="0"
        onPointerDown={startResize}
        onKeyDown={onHandleKey}
        onDblClick={() => resetSidebarWidth()}
      />
    </aside>
  );
}

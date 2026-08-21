/** Left panel shell: a small mode toggle (page thumbnails, table of contents,
 * formal environments, figures, or your own notes) plus whichever content is
 * currently selected. */
import { Show } from 'solid-js';
import IconButton from './IconButton';
import { IconFigure, IconList, IconNote, IconPages, IconQed } from './icons';
import { ui, setUi } from '../stores/uiStore';
import PageThumbnails from '../features/outline/PageThumbnails';
import Outline from '../features/outline/Outline';
import FormalEnvs from '../features/outline/FormalEnvs';
import Figures from '../features/outline/Figures';
import NotesPanel from '../features/annotations/NotesPanel';

export default function Sidebar() {
  return (
    <aside class="sidebar" aria-label="Document navigation">
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
    </aside>
  );
}

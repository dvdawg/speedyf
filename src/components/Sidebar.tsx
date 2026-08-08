/** Left panel shell: a small mode toggle (page thumbnails vs. table of
 * contents) plus whichever content is currently selected. */
import { Show } from 'solid-js';
import IconButton from './IconButton';
import { IconList, IconPages } from './icons';
import { ui, setUi } from '../stores/uiStore';
import PageThumbnails from '../features/outline/PageThumbnails';
import Outline from '../features/outline/Outline';

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
      </div>
      <Show when={ui.sidebarMode === 'pages'} fallback={<Outline />}>
        <PageThumbnails />
      </Show>
    </aside>
  );
}

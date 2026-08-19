/** Recently-opened files. Clicking a row opens it in a new tab (or focuses the
 * tab already showing it); an entry that no longer opens is dropped. */
import { For, Show } from 'solid-js';
import { openInNewTabOrFocus } from '../document/tabsController';
import { recentStore } from '../../stores/recentStore';
import { askConfirm } from '../../stores/modalStore';
import { formatDate, formatSize } from './format';

export default function RecentsPanel() {
  const openRecent = async (path: string) => {
    const ok = await openInNewTabOrFocus(path);
    if (!ok) recentStore.remove(path);
  };

  const clearRecent = async () => {
    if (await askConfirm('Clear all recent files?', 'Clear')) recentStore.clear();
  };

  return (
    <>
      <div class="home-panel-head">
        <h2>Recents</h2>
        <Show when={recentStore.state.entries.length > 0}>
          <button type="button" class="secondary-btn" onClick={() => void clearRecent()}>
            Clear recent
          </button>
        </Show>
      </div>
      <div class="home-panel-body">
        <Show
          when={recentStore.state.entries.length > 0}
          fallback={
            <div class="home-placeholder">
              <p class="home-placeholder-title">No recent files</p>
              <p>Open a PDF to get started, or drop a file anywhere in this window.</p>
            </div>
          }
        >
          <div class="home-recent-list" role="list">
            <For each={recentStore.state.entries}>
              {(entry) => (
                <button
                  type="button"
                  class="home-recent-row"
                  role="listitem"
                  onClick={() => void openRecent(entry.path)}
                >
                  <span class="home-recent-top">
                    <span class="home-recent-name">{entry.name}</span>
                    <span class="home-recent-date">{formatDate(entry.lastOpened)}</span>
                    <span class="home-recent-size">{formatSize(entry.sizeBytes)}</span>
                  </span>
                  <span class="home-recent-path" title={entry.path}>
                    {entry.path}
                  </span>
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>
    </>
  );
}

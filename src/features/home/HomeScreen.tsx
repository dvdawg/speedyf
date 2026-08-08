/** Shown when no tab is active (App boots with no tabs, or Home is clicked).
 * Independent of open tabs — going Home never closes anything. */
import { For, Show } from 'solid-js';
import { openFromDialog, openInNewTabOrFocus } from '../document/tabsController';
import { IconOpen } from '../../components/icons';
import { recentStore } from '../../stores/recentStore';
import { askConfirm } from '../../stores/modalStore';

function formatSize(bytes: number | null): string {
  if (bytes === null) return '–';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

function formatDate(ms: number): string {
  const d = new Date(ms);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (sameDay) {
    return d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  }
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

export default function HomeScreen() {
  const openRecent = async (path: string) => {
    const ok = await openInNewTabOrFocus(path);
    if (!ok) recentStore.remove(path);
  };

  const clearRecent = async () => {
    if (await askConfirm('Clear all recent files?', 'Clear')) recentStore.clear();
  };

  return (
    <div class="home-screen">
      <div class="home-header">
        <h1>SpeedyF</h1>
        <button type="button" class="primary-btn" onClick={() => void openFromDialog()}>
          <IconOpen /> Open PDF…
        </button>
      </div>
      <Show
        when={recentStore.state.entries.length > 0}
        fallback={
          <div class="home-empty">
            <p>
              No recent files. Open a PDF to get started, or drop a file anywhere in this window.
            </p>
          </div>
        }
      >
        <div class="home-recent">
          <div class="home-recent-head">
            <span>Recent</span>
            <button type="button" class="secondary-btn" onClick={() => void clearRecent()}>
              Clear recent
            </button>
          </div>
          <div class="home-recent-list" role="list">
            <For each={recentStore.state.entries}>
              {(entry) => (
                <button
                  type="button"
                  class="home-recent-row"
                  role="listitem"
                  onClick={() => void openRecent(entry.path)}
                >
                  <span class="home-recent-name">{entry.name}</span>
                  <span class="home-recent-date">{formatDate(entry.lastOpened)}</span>
                  <span class="home-recent-size">{formatSize(entry.sizeBytes)}</span>
                  <span class="home-recent-path" title={entry.path}>
                    {entry.path}
                  </span>
                </button>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
}

import { Show } from 'solid-js';
import { documentStore } from '../features/document/documentStore';
import { searchStore } from '../features/search/searchStore';
import { viewport } from '../stores/viewportStore';
import { effectiveTheme, settings, updateSettings } from '../stores/settings';
import { engine } from '../lib/transport/engine';

export default function StatusBar() {
  const doc = documentStore.state;
  const s = searchStore.state;

  return (
    <footer class="status-bar">
      <div class="sb-left">
        <Show when={doc.loaded} fallback={<span class="sb-dim">No document</span>}>
          <span class="sb-name" title={doc.path ?? doc.name}>
            {doc.name}
          </span>
          <Show when={doc.dirty}>
            <span class="sb-dirty" title="Unsaved changes">
              ● edited
            </span>
          </Show>
          <Show when={doc.saving}>
            <span class="sb-dim">saving…</span>
          </Show>
        </Show>
      </div>
      <div class="sb-center" aria-live="polite">
        <Show when={doc.loaded && !s.indexingDone && s.total > 0}>
          <span class="sb-dim">
            Indexing text {s.indexed}/{s.total}
          </span>
        </Show>
        <Show when={s.truncated}>
          <span class="sb-warn" title="Text index reached its memory budget">
            index truncated
          </span>
        </Show>
      </div>
      <div class="sb-right">
        <Show when={doc.loaded}>
          <span class="sb-dim">
            p. {viewport.currentPage + 1}/{doc.pages.length} · {Math.round(viewport.zoom * 100)}%
          </span>
        </Show>
        <label class="sb-control" title="Reduce cache memory budgets">
          <input
            type="checkbox"
            checked={settings.lowMemory}
            onInput={(e) => {
              updateSettings({ lowMemory: e.currentTarget.checked });
              void engine.setLowMemory(e.currentTarget.checked);
            }}
          />
          low&nbsp;memory
        </label>
        <label class="sb-control" title="Color theme">
          <span class="sr-only">Theme</span>
          <select
            value={settings.theme}
            onInput={(e) => updateSettings({ theme: e.currentTarget.value as 'system' | 'light' | 'dark' })}
            aria-label={`Theme (currently ${effectiveTheme()})`}
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </label>
      </div>
    </footer>
  );
}

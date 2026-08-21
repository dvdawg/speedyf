import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { activeTab } from '../stores/tabsStore';
import { applyLowMemory, effectiveTheme, settings, updateSettings } from '../stores/settings';
import { engine } from '../lib/transport/engine';
import type { EngineMetrics } from '../lib/transport/engine';
import { libraryStore } from '../features/citations/libraryStore';

export default function StatusBar() {
  const doc = () => activeTab()?.documentStore.state;
  const s = () => activeTab()?.searchStore.state;
  const [metrics, setMetrics] = createSignal<EngineMetrics | null>(null);

  onMount(() => {
    let disposed = false;
    // Both polls cross the IPC boundary and take engine locks, so they are
    // skipped whenever their answer cannot be seen or cannot have changed:
    // metrics while the window is hidden, library status while no scan is
    // running. Idling with tabs open should cost nothing.
    const hidden = () => document.visibilityState === 'hidden';
    const refresh = async () => {
      if (!doc()?.loaded) {
        setMetrics(null);
        return;
      }
      if (hidden()) return;
      try {
        const next = await engine.metrics();
        if (!disposed) setMetrics(next);
      } catch {
        // Diagnostics must never interfere with document interaction.
      }
    };
    // Only a running scan changes this status, and every scan is started by a
    // command that refreshes explicitly — so an idle library needs no polling.
    const refreshLibrary = async () => {
      if (hidden() || !libraryStore.state.library.scanning) return;
      await libraryStore.refreshLibraryStatus();
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1_500);
    const libraryTimer = window.setInterval(() => void refreshLibrary(), 1_200);
    // A hidden window skips its polls entirely; catch up as soon as it returns.
    const onVisibility = () => {
      if (!hidden()) {
        void refresh();
        void refreshLibrary();
      }
    };
    document.addEventListener('visibilitychange', onVisibility);
    onCleanup(() => {
      disposed = true;
      window.clearInterval(timer);
      window.clearInterval(libraryTimer);
      document.removeEventListener('visibilitychange', onVisibility);
    });
  });

  return (
    <footer class="status-bar">
      <div class="sb-left">
        <Show when={doc()?.loaded} fallback={<span class="sb-dim">No document</span>}>
          <span class="sb-name" title={doc()!.path ?? doc()!.name}>
            {doc()!.name}
          </span>
          <Show when={doc()!.dirty}>
            <span class="sb-dirty" title="Unsaved changes">
              ● edited
            </span>
          </Show>
          <Show when={doc()!.saving}>
            <span class="sb-dim">saving…</span>
          </Show>
        </Show>
      </div>
      <div class="sb-center" aria-live="polite">
        <Show when={doc()?.loaded && !s()!.indexingDone && s()!.total > 0}>
          <span class="sb-dim">
            Indexing text {s()!.indexed}/{s()!.total}
          </span>
        </Show>
        <Show when={libraryStore.state.library.scanning}>
          <span class="sb-dim">
            Citation library {libraryStore.state.library.indexed}/{libraryStore.state.library.total}
          </span>
        </Show>
        <Show when={libraryStore.state.libraryError}>
          <span class="sb-warn" title={libraryStore.state.libraryError ?? undefined}>
            citation library error
          </span>
        </Show>
        <Show when={s()?.truncated}>
          <span class="sb-warn" title="Text index reached its memory budget">
            index truncated
          </span>
        </Show>
        <Show when={doc()?.loaded && (doc()?.pages.length ?? 0) > 65_535}>
          <span
            class="sb-warn"
            title="Viewing and search support all pages, but PDFium output documents are limited to 65,535 pages. Delete pages below that limit before saving."
          >
            save disabled above 65,535 pages
          </span>
        </Show>
      </div>
      <div class="sb-right">
        <Show when={doc()?.loaded && metrics()}>
          {(value) => (
            <span
              class="sb-dim"
              title={`Engine diagnostics: ${value().rendered} renders, queue ${value().queueDepth}`}
              aria-label={`Engine cache ${value().cacheHits} hits from ${value().cacheLookups} lookups; ${value().skippedStale} stale tasks skipped`}
            >
              cache {value().cacheHits}/{value().cacheLookups} · stale {value().skippedStale}
            </span>
          )}
        </Show>
        <Show when={doc()?.loaded}>
          <span class="sb-dim">
            p. {(activeTab()?.viewport.state.currentPage ?? 0) + 1}/{doc()!.pages.length} ·{' '}
            {Math.round((activeTab()?.viewport.state.zoom ?? 1) * 100)}%
          </span>
        </Show>
        <label class="sb-control" title="Reduce cache memory budgets">
          <input
            type="checkbox"
            checked={settings.lowMemory}
            onInput={(e) => applyLowMemory(e.currentTarget.checked)}
          />
          low&nbsp;memory
        </label>
        <button
          type="button"
          class="sb-control sb-button"
          title={
            libraryStore.state.library.roots.join('\n') || 'Choose folders of local PDF papers'
          }
          onClick={() => void libraryStore.addLibraryFolder()}
        >
          Library…
        </button>
        <label class="sb-control" title="Color theme">
          <span class="sr-only">Theme</span>
          <select
            value={settings.theme}
            onInput={(e) =>
              updateSettings({ theme: e.currentTarget.value as 'system' | 'light' | 'dark' })
            }
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

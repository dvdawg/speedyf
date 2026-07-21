import { createEffect, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import Toolbar from '../components/Toolbar';
import Sidebar from '../components/Sidebar';
import StatusBar from '../components/StatusBar';
import Modals from '../components/Modals';
import Viewer from '../features/viewer/Viewer';
import SearchPanel from '../features/search/SearchPanel';
import FormPanel from '../features/editor/FormPanel';
import { documentStore } from '../features/document/documentStore';
import { guardUnsaved, openPath } from '../features/document/controller';
import { searchStore } from '../features/search/searchStore';
import { engine } from '../lib/transport/engine';
import { viewport } from '../stores/viewportStore';
import { effectiveTheme, settings } from '../stores/settings';
import { installShortcuts } from './shortcuts';
import { clearImagePreviews } from '../features/editor/editorActions';

export default function App() {
  const [dropActive, setDropActive] = createSignal(false);

  // theme: follow system, or forced light/dark
  createEffect(() => {
    document.documentElement.dataset.theme = effectiveTheme();
    void settings.theme; // dependency
  });
  onMount(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => {
      if (settings.theme === 'system') {
        document.documentElement.dataset.theme = effectiveTheme();
      }
    };
    mq.addEventListener('change', onChange);
    onCleanup(() => mq.removeEventListener('change', onChange));
  });

  // window title reflects document + dirty state
  createEffect(() => {
    const doc = documentStore.state;
    const title = doc.loaded ? `${doc.dirty ? '• ' : ''}${doc.name} — SpeedyF` : 'SpeedyF';
    void getCurrentWindow().setTitle(title).catch(() => undefined);
  });

  onMount(() => {
    const cleanupShortcuts = installShortcuts();

    // engine events → search store
    let unlistenProgress: (() => void) | undefined;
    void engine
      .onSearchProgress((p) => {
        if (p.docId === documentStore.state.docId) {
          searchStore.onIndexProgress(p.indexed, p.total, p.truncated);
        }
      })
      .then((un) => (unlistenProgress = un));

    // native file drag-drop
    let unlistenDrop: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const t = event.payload.type;
        if (t === 'over') setDropActive(true);
        else if (t === 'leave') setDropActive(false);
        else if (t === 'drop') {
          setDropActive(false);
          const pdf = event.payload.paths.find((p) => p.toLowerCase().endsWith('.pdf'));
          if (pdf) void openPath(pdf);
        }
      })
      .then((un) => (unlistenDrop = un));

    // unsaved-changes close guard
    let closing = false;
    let unlistenClose: (() => void) | undefined;
    const win = getCurrentWindow();
    void win
      .onCloseRequested(async (e) => {
        if (closing) return;
        if (documentStore.state.dirty) {
          e.preventDefault();
          const ok = await guardUnsaved();
          if (ok) {
            closing = true;
            await win.destroy();
          }
        }
      })
      .then((un) => (unlistenClose = un));

    // apply persisted low-memory budget at startup
    if (settings.lowMemory) void engine.setLowMemory(true);

    onCleanup(() => {
      cleanupShortcuts();
      unlistenProgress?.();
      unlistenDrop?.();
      unlistenClose?.();
      clearImagePreviews();
    });
  });

  return (
    <div class="app-shell" classList={{ 'drop-active': dropActive() }}>
      <Toolbar />
      <div class="app-body">
        <Show when={viewport.sidebarOpen && documentStore.state.loaded}>
          <Sidebar />
        </Show>
        <main class="app-main">
          <Viewer />
        </main>
        <Show when={viewport.searchOpen && documentStore.state.loaded}>
          <SearchPanel />
        </Show>
        <Show when={viewport.formPanelOpen && documentStore.state.loaded}>
          <FormPanel />
        </Show>
      </div>
      <StatusBar />
      <Show when={dropActive()}>
        <div class="drop-overlay" aria-hidden="true">
          <div class="drop-card">Drop PDF to open</div>
        </div>
      </Show>
      <Modals />
    </div>
  );
}

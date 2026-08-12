import { createEffect, onCleanup, onMount, Show } from 'solid-js';
import { createSignal, For } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import Toolbar from '../components/Toolbar';
import TabStrip from '../features/tabs/TabStrip';
import Sidebar from '../components/Sidebar';
import StatusBar from '../components/StatusBar';
import Modals from '../components/Modals';
import Viewer from '../features/viewer/Viewer';
import SearchPanel from '../features/search/SearchPanel';
import FormPanel from '../features/editor/FormPanel';
import { engine } from '../lib/transport/engine';
import { ui } from '../stores/uiStore';
import { effectiveTheme, settings } from '../stores/settings';
import { installShortcuts } from './shortcuts';
import { clearImagePreviews } from '../features/editor/editorActions';
import PreviewPopover from '../features/citations/PreviewPopover';
import HomeScreen from '../features/home/HomeScreen';
import { libraryStore } from '../features/citations/libraryStore';
import { activeTab, tabsStore } from '../stores/tabsStore';
import {
  closeAllWithGuard,
  openInNewTabOrFocus,
  restoreSession,
} from '../features/document/tabsController';
import { TabContext } from './TabContext';

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

  // window title reflects the active tab's document + dirty state
  createEffect(() => {
    const doc = activeTab()?.documentStore.state;
    const title = doc?.loaded ? `${doc.dirty ? '• ' : ''}${doc.name} — SpeedyF` : 'SpeedyF';
    void getCurrentWindow()
      .setTitle(title)
      .catch(() => undefined);
  });

  onMount(() => {
    const cleanupShortcuts = installShortcuts();
    let disposed = false;

    // engine events → the owning tab's own search store, even if it's a
    // background tab (not the currently visible one)
    let unlistenProgress: (() => void) | undefined;
    void engine
      .onSearchProgress((p) => {
        const tab = tabsStore.state.tabs.find((t) => t.documentStore.state.docId === p.docId);
        tab?.searchStore.onIndexProgress(p.indexed, p.total, p.truncated);
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenProgress = unlisten;
      });

    // Files the OS asked us to open (double-click, "Open With", Dock drop):
    // drain anything that arrived before we were ready to listen (cold
    // launch-to-open), then keep listening for the rest of this session.
    void invoke<string[]>('take_pending_opens').then((paths) => {
      for (const p of paths) void openInNewTabOrFocus(p);
    });
    let unlistenOpen: (() => void) | undefined;
    void listen<string[]>('file-open-requested', (event) => {
      for (const p of event.payload) void openInNewTabOrFocus(p);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else unlistenOpen = unlisten;
    });

    // native file drag-drop always opens a new tab (or focuses an existing one)
    let unlistenDrop: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const t = event.payload.type;
        if (t === 'over') setDropActive(true);
        else if (t === 'leave') setDropActive(false);
        else if (t === 'drop') {
          setDropActive(false);
          const pdf = event.payload.paths.find((p) => p.toLowerCase().endsWith('.pdf'));
          if (pdf) void openInNewTabOrFocus(pdf);
        }
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenDrop = unlisten;
      });

    // unsaved-changes close guard, across every open tab
    let closing = false;
    let unlistenClose: (() => void) | undefined;
    const win = getCurrentWindow();
    void win
      .onCloseRequested(async (e) => {
        if (closing) return;
        const anyDirty = tabsStore.state.tabs.some((t) => t.documentStore.state.dirty);
        if (anyDirty) {
          e.preventDefault();
          const ok = await closeAllWithGuard();
          if (ok) {
            closing = true;
            await win.destroy();
          }
        }
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenClose = unlisten;
      });

    // apply persisted low-memory budget at startup
    if (settings.lowMemory) void engine.setLowMemory(true);
    void libraryStore.initializeLibrary();
    void restoreSession();

    onCleanup(() => {
      disposed = true;
      cleanupShortcuts();
      unlistenProgress?.();
      unlistenOpen?.();
      unlistenDrop?.();
      unlistenClose?.();
      clearImagePreviews();
    });
  });

  return (
    <div class="app-shell" classList={{ 'drop-active': dropActive() }}>
      <TabStrip />
      <Toolbar />
      <div class="content-area">
        <Show when={tabsStore.state.tabs.length > 0}>
          <div class="tab-workspaces">
            <For each={tabsStore.state.tabs}>
              {(tab) => {
                // Sidebar state (open/mode) is window-level, so without this
                // every open document would mount its own thumbnail list and
                // race the others for the one engine thread — for panels
                // nobody can see. The Viewer stays mounted either way and
                // sheds its own page rasters internally.
                const active = () => tab.id === tabsStore.state.activeId;
                const visible = () => active() && tab.documentStore.state.loaded;
                return (
                  <TabContext.Provider value={tab}>
                    <div class="tab-workspace" classList={{ 'is-active': active() }}>
                      <div class="app-body">
                        <Show when={ui.sidebarOpen && visible()}>
                          <Sidebar />
                        </Show>
                        <main class="app-main">
                          <Viewer />
                        </main>
                        <Show when={tab.viewport.state.searchOpen && visible()}>
                          <SearchPanel />
                        </Show>
                        <Show when={tab.viewport.state.formPanelOpen && visible()}>
                          <FormPanel />
                        </Show>
                      </div>
                    </div>
                  </TabContext.Provider>
                );
              }}
            </For>
          </div>
        </Show>
        <Show when={tabsStore.state.activeId === null}>
          <HomeScreen />
        </Show>
      </div>
      <StatusBar />
      <Show when={dropActive()}>
        <div class="drop-overlay" aria-hidden="true">
          <div class="drop-card">Drop PDF to open</div>
        </div>
      </Show>
      <Modals />
      <PreviewPopover />
    </div>
  );
}

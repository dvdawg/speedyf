import { createEffect, onCleanup, onMount, Show } from 'solid-js';
import { createSignal, For } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { invoke } from '@tauri-apps/api/core';
import Toolbar from '../components/Toolbar';
import TabStrip from '../features/tabs/TabStrip';
import Sidebar from '../components/Sidebar';
import StatusBar from '../components/StatusBar';
import Modals from '../components/Modals';
import CommandPalette from '../features/palette/CommandPalette';
import Viewer from '../features/viewer/Viewer';
import ContextHeader from '../features/viewer/ContextHeader';
import SearchPanel from '../features/search/SearchPanel';
import FormPanel from '../features/editor/FormPanel';
import { engine } from '../lib/transport/engine';
import { ui } from '../stores/uiStore';
import { effectiveTheme, pagesInverted, settings } from '../stores/settings';
import { installShortcuts } from './shortcuts';
import { loadExtensions } from '../features/extensions/extensionStore';
import { clearImagePreviews } from '../features/editor/editorActions';
import { clearFormFields } from '../features/forms/formStore';
import PreviewPopover from '../features/citations/PreviewPopover';
import HomeScreen from '../features/home/HomeScreen';
import ProjectsScreen from '../features/projects/ProjectsScreen';
import { libraryStore } from '../features/citations/libraryStore';
import { dropOverlayText, projectsDropTarget } from '../features/projects/dnd';
import { addDroppedPdfs, isPdf } from '../features/projects/fileDrop';
import { activeTab, tabsStore } from '../stores/tabsStore';
import {
  acceptOfferedTab,
  closeAllWithGuard,
  openInNewTabOrFocus,
  initializeWindow,
  prepareWindowClose,
} from '../features/document/tabsController';
import { windowTransferPending } from '../features/windows/windowController';
import { showError } from '../stores/modalStore';
import { TabContext } from './TabContext';

export default function App() {
  /** What the overlay says while a file is held over the window; null when
   * none is. The text depends on what is under the pointer. */
  const [dropOverlay, setDropOverlay] = createSignal<string | null>(null);

  onMount(() => void loadExtensions());

  // theme: follow system, or forced light/dark. `effectiveTheme` tracks the
  // system preference itself, so this one effect covers both.
  createEffect(() => {
    document.documentElement.dataset.theme = effectiveTheme();
    // Pages come back inverted from the engine; the shell has to repaint the
    // paper it draws behind and around them to match.
    document.documentElement.dataset.invertPages = pagesInverted() ? 'on' : 'off';
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
    let unlistenOpen: (() => void) | undefined;
    // Register first, then drain; native routing delivers to exactly one window.
    // Scoped to this window, not the plain `listen` from the event module:
    // that one subscribes with target Any, which receives events emitted to
    // *every* window — a document handed to one window would be opened by all.
    void getCurrentWindow()
      .listen<string[]>('file-open-requested', (event) => {
        for (const p of event.payload) void openInNewTabOrFocus(p);
      })
      .then(async (unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        unlistenOpen = unlisten;
        await initializeWindow();
        const paths = await invoke<string[]>('take_pending_opens');
        for (const p of paths) await openInNewTabOrFocus(p);
      })
      .catch((error) => showError(`Could not initialize this window: ${String(error)}`));

    // Another window handing this one a tab from its tab menu. Window-scoped
    // like the listener above, so only the addressee hears it.
    let unlistenOffer: (() => void) | undefined;
    void getCurrentWindow()
      .listen('tab-transfer-offered', () => void acceptOfferedTab())
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenOffer = unlisten;
      });

    let unlistenFocus: (() => void) | undefined;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void engine.setActiveDocument(activeTab()?.documentStore.state.docId ?? null);
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenFocus = unlisten;
      });

    // Native file drag-drop. A PDF let go over the Projects screen is copied
    // into the library — and filed, when let go on a project row; anywhere
    // else it opens in a tab (or focuses the one it is in). The position is
    // physical pixels relative to the webview; the DOM hit-test wants CSS
    // pixels.
    let unlistenDrop: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === 'leave') {
          setDropOverlay(null);
          return;
        }
        const scale = window.devicePixelRatio || 1;
        const target = projectsDropTarget(payload.position.x / scale, payload.position.y / scale);
        if (payload.type !== 'drop') {
          setDropOverlay(dropOverlayText(target));
          return;
        }
        setDropOverlay(null);
        if (target) {
          void addDroppedPdfs(payload.paths, target);
        } else {
          const pdf = payload.paths.find(isPdf);
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
        e.preventDefault();
        if (closing || windowTransferPending()) return;
        closing = true;
        try {
          if (await closeAllWithGuard()) {
            await prepareWindowClose();
            await win.destroy();
          }
        } finally {
          closing = false;
        }
      })
      .then((unlisten) => {
        if (disposed) unlisten();
        else unlistenClose = unlisten;
      });

    // apply persisted low-memory budget at startup
    if (settings.lowMemory) void engine.setLowMemory(true);
    // The engine decides whether a lookup may go out; the setting lives here.
    void engine.librarySetLookup(settings.onlineLookup, settings.lookupContact, settings.s2Key);
    void libraryStore.initializeLibrary();

    onCleanup(() => {
      disposed = true;
      cleanupShortcuts();
      unlistenProgress?.();
      unlistenOpen?.();
      unlistenDrop?.();
      unlistenClose?.();
      unlistenFocus?.();
      unlistenOffer?.();
      clearImagePreviews();
      clearFormFields();
    });
  });

  return (
    <>
      <div
        class="app-shell"
        inert={windowTransferPending()}
        classList={{ 'drop-active': dropOverlay() !== null }}
      >
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
                            <ContextHeader />
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
            <Show when={settings.launchScreen === 'projects'} fallback={<HomeScreen />}>
              <ProjectsScreen />
            </Show>
          </Show>
        </div>
        <StatusBar />
        <Show when={dropOverlay()}>
          {(text) => (
            <div class="drop-overlay" aria-hidden="true">
              <div class="drop-card">{text()}</div>
            </div>
          )}
        </Show>
        <Modals />
        <CommandPalette />
        <PreviewPopover />
      </div>
      <Show when={windowTransferPending()}>
        <div class="window-transfer-overlay" role="status">
          Moving tab to another window…
        </div>
      </Show>
    </>
  );
}

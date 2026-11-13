/** App preferences. Writes the same `settings` store the StatusBar controls
 * write, so the two surfaces stay in sync without extra wiring.
 *
 * The library rows at the bottom are the settings projects.md §3–§4 call for:
 * which launch screen opens, where a paper from outside the library is copied
 * to, whether metadata may be looked up online, and where `library.json`
 * lives. Lookup is the one row with an effect beyond the store — the engine
 * decides whether a request may go out — so both of its fields reach it the
 * moment they change, the way low memory does. */
import { createSignal, onMount, Show } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { engine, isEngineError } from '../../lib/transport/engine';
import { applyLowMemory, effectiveTheme, settings, updateSettings } from '../../stores/settings';
import type { LaunchScreen, ThemePref } from '../../stores/settings';
import type { LibraryDataInfo } from '../../types/engine';

const message = (error: unknown) =>
  isEngineError(error) ? error.message : error instanceof Error ? error.message : String(error);

/** The three lookup settings reach the engine together: it keeps one copy
 * of them and decides, per request, whether anything may go out and under
 * what name. */
function pushLookup() {
  void engine.librarySetLookup(settings.onlineLookup, settings.lookupContact, settings.s2Key);
}

function setOnlineLookup(enabled: boolean) {
  updateSettings({ onlineLookup: enabled });
  pushLookup();
}

function setLookupContact(contact: string) {
  updateSettings({ lookupContact: contact });
  pushLookup();
}

function setS2Key(key: string) {
  updateSettings({ s2Key: key.trim() });
  pushLookup();
}

export default function SettingsPanel() {
  const [dataDir, setDataDir] = createSignal<LibraryDataInfo | null>(null);
  const [dataError, setDataError] = createSignal<string | null>(null);

  onMount(() => {
    engine.libraryDataDir().then(setDataDir, (reason: unknown) => setDataError(message(reason)));
  });

  const chooseInbox = async () => {
    // Start in the current inbox, else the first library folder: the inbox
    // has to be inside the library, and the dialog should open there.
    const start = settings.libraryInbox ?? settings.libraryRoots[0];
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: 'Choose the library folder new items are copied into',
      ...(start ? { defaultPath: start } : {}),
    });
    if (typeof picked === 'string') updateSettings({ libraryInbox: picked });
  };

  const moveData = async () => {
    const current = dataDir();
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: 'Choose the folder that holds library.json',
      ...(current ? { defaultPath: current.dir } : {}),
    });
    if (typeof picked !== 'string') return;
    setDataError(null);
    try {
      setDataDir(await engine.libraryMoveData(picked));
    } catch (reason) {
      setDataError(message(reason));
    }
  };

  return (
    <>
      <div class="home-panel-head">
        <h2>Settings</h2>
      </div>
      <div class="home-panel-body">
        <div class="home-setting">
          <label class="home-setting-text" for="setting-theme">
            <span class="home-setting-title">Theme</span>
            <span class="home-setting-desc">
              Follow the system appearance, or pin SpeedyF to light or dark.
            </span>
          </label>
          <select
            id="setting-theme"
            value={settings.theme}
            onInput={(e) => updateSettings({ theme: e.currentTarget.value as ThemePref })}
            aria-label={`Theme (currently ${effectiveTheme()})`}
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-invert-pages">
            <span class="home-setting-title">Invert page colors in dark theme</span>
            <span class="home-setting-desc">
              Render pages as dark paper with light ink. Hues are preserved rather than flipped, and
              photographs are left alone. Printing and export are unaffected.
            </span>
          </label>
          <input
            id="setting-invert-pages"
            type="checkbox"
            checked={settings.invertPages}
            onInput={(e) => updateSettings({ invertPages: e.currentTarget.checked })}
          />
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-low-memory">
            <span class="home-setting-title">Low memory</span>
            <span class="home-setting-desc">
              Reduce cache memory budgets. Uses less RAM on large documents, at the cost of
              re-rendering pages more often.
            </span>
          </label>
          <input
            id="setting-low-memory"
            type="checkbox"
            checked={settings.lowMemory}
            onInput={(e) => applyLowMemory(e.currentTarget.checked)}
          />
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-launch-screen">
            <span class="home-setting-title">Launch screen</span>
            <span class="home-setting-desc">
              What shows when no document is open: Home, with recents and the library search, or
              Projects, with your items.
            </span>
          </label>
          <select
            id="setting-launch-screen"
            value={settings.launchScreen}
            onInput={(e) => updateSettings({ launchScreen: e.currentTarget.value as LaunchScreen })}
          >
            <option value="home">Home</option>
            <option value="projects">Projects</option>
          </select>
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-inbox-choose">
            <span class="home-setting-title">Inbox folder</span>
            <span class="home-setting-desc">
              The library folder a PDF from outside the library is copied into when you add it to a
              project. The original is never moved.
            </span>
          </label>
          <div class="library-setting-value">
            <Show
              when={settings.libraryInbox}
              fallback={<span class="library-setting-path">not set</span>}
            >
              {(inbox) => (
                <span class="library-setting-path" title={inbox()}>
                  {inbox()}
                </span>
              )}
            </Show>
            <div class="library-setting-actions">
              <button
                id="setting-inbox-choose"
                type="button"
                class="secondary-btn"
                onClick={() => void chooseInbox()}
              >
                Choose…
              </button>
              <Show when={settings.libraryInbox}>
                <button
                  type="button"
                  class="secondary-btn"
                  onClick={() => updateSettings({ libraryInbox: null })}
                >
                  Clear
                </button>
              </Show>
            </div>
          </div>
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-online-lookup">
            <span class="home-setting-title">Look up metadata online</span>
            <span class="home-setting-desc">
              Sends DOIs and arXiv ids, and for items without either, their first-page title, to
              Crossref, DataCite, OpenAlex, arXiv, Semantic Scholar, INSPIRE, DBLP and Open Library.
              Off by default.
            </span>
          </label>
          <input
            id="setting-online-lookup"
            type="checkbox"
            checked={settings.onlineLookup}
            onInput={(e) => setOnlineLookup(e.currentTarget.checked)}
          />
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-lookup-contact">
            <span class="home-setting-title">Contact for lookups</span>
            <span class="home-setting-desc">
              Sent with each lookup so Crossref can reach you about your requests; it gives
              identified clients better service. Optional.
            </span>
          </label>
          <input
            id="setting-lookup-contact"
            class="library-setting-contact"
            type="text"
            placeholder="optional, e.g. you@example.edu"
            value={settings.lookupContact}
            onChange={(e) => setLookupContact(e.currentTarget.value)}
          />
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-s2-key">
            <span class="home-setting-title">Semantic Scholar API key (optional)</span>
            <span class="home-setting-desc">
              Sent only to Semantic Scholar; a better rate limit, nothing else.
            </span>
          </label>
          <input
            id="setting-s2-key"
            class="library-setting-contact"
            type="text"
            autocomplete="off"
            spellcheck={false}
            placeholder="optional"
            value={settings.s2Key}
            onChange={(e) => setS2Key(e.currentTarget.value)}
          />
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-data-move">
            <span class="home-setting-title">Library data folder</span>
            <span class="home-setting-desc">
              Where library.json — your projects, memberships and edits — is kept. Choose a synced
              folder to share it between machines. A folder that already holds a library.json is
              adopted as it is, and a sync conflict is not merged: the copy that arrives last wins.
            </span>
          </label>
          <div class="library-setting-value">
            <Show
              when={dataDir()}
              fallback={
                <span class="library-setting-path">{dataError() ? 'unknown' : 'reading…'}</span>
              }
            >
              {(info) => (
                <span class="library-setting-path" title={info().dir}>
                  {info().dir}
                  {info().isDefault ? ' (default)' : ''}
                </span>
              )}
            </Show>
            <div class="library-setting-actions">
              <button
                id="setting-data-move"
                type="button"
                class="secondary-btn"
                onClick={() => void moveData()}
              >
                Move…
              </button>
            </div>
            <Show when={dataError()}>
              <span class="library-setting-error" role="alert">
                {dataError()}
              </span>
            </Show>
          </div>
        </div>
      </div>
    </>
  );
}

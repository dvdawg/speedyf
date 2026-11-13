/** App preferences. Writes the same `settings` store the StatusBar controls
 * write, so the two surfaces stay in sync without extra wiring.
 *
 * The library rows at the bottom are the settings projects.md §3–§4 call for:
 * which launch screen opens, where a paper from outside the library is copied
 * to, whether metadata may be looked up online, and where `library.json`
 * lives. Lookup is the one row with an effect beyond the store — the engine
 * decides whether a request may go out — so both of its fields reach it the
 * moment they change, the way low memory does.
 *
 * Backups and the BibTeX options live with the library data, not in the
 * settings store: the engine owns both, and a synced library carries them
 * to every machine. The BibTeX options sit in a closed "Advanced"
 * disclosure at the bottom; their defaults are what most people want. */
import { createSignal, For, onMount, Show } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { engine, isEngineError } from '../../lib/transport/engine';
import { applyLowMemory, effectiveTheme, settings, updateSettings } from '../../stores/settings';
import type { LaunchScreen, ThemePref } from '../../stores/settings';
import type { BibtexOptions, LibraryBackup, LibraryDataInfo } from '../../types/engine';
import { formatDate } from './format';

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

/** "Nov 12, 2026, 9:40 AM" for a backup's RFC 3339 stamp. */
function backupWhen(at: string): string {
  const ms = Date.parse(at);
  if (Number.isNaN(ms)) return at;
  return new Date(ms).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** The BibTeX options, in the order the Advanced group shows them. */
const BIBTEX_ROWS: { key: keyof BibtexOptions; title: string; desc: string }[] = [
  {
    key: 'includeAbstract',
    title: 'Include abstract',
    desc: 'Write the abstract into each entry.',
  },
  {
    key: 'includeUrl',
    title: 'Include URL',
    desc: 'The doi.org link, or the arXiv abstract page for a preprint.',
  },
  {
    key: 'includeFile',
    title: 'Include file path',
    desc: 'The path of the main PDF, for reference managers that link files.',
  },
  {
    key: 'protectTitle',
    title: 'Protect title case',
    desc: 'Double braces around titles, so styles keep their capitals.',
  },
  {
    key: 'abbreviateJournals',
    title: 'Abbreviate journal names',
    desc: 'Phys. Rev. Lett., J. Mach. Learn. Res. and other common physics, math and CS journals. Other names are left as they are.',
  },
];

export default function SettingsPanel() {
  const [dataDir, setDataDir] = createSignal<LibraryDataInfo | null>(null);
  const [dataError, setDataError] = createSignal<string | null>(null);
  const [backups, setBackups] = createSignal<LibraryBackup[] | null>(null);
  const [backupError, setBackupError] = createSignal<string | null>(null);
  const [restoreOpen, setRestoreOpen] = createSignal(false);
  const [confirming, setConfirming] = createSignal<string | null>(null);
  const [restoring, setRestoring] = createSignal(false);
  const [restored, setRestored] = createSignal<string | null>(null);
  const [bibtex, setBibtex] = createSignal<BibtexOptions | null>(null);
  const [bibtexError, setBibtexError] = createSignal<string | null>(null);

  const loadBackups = () =>
    engine.libraryBackups().then(setBackups, (reason: unknown) => setBackupError(message(reason)));

  onMount(() => {
    engine.libraryDataDir().then(setDataDir, (reason: unknown) => setDataError(message(reason)));
    void loadBackups();
    engine
      .libraryBibtexOptions()
      .then(setBibtex, (reason: unknown) => setBibtexError(message(reason)));
  });

  const toggleRestore = () => {
    const open = !restoreOpen();
    setRestoreOpen(open);
    setConfirming(null);
    setRestored(null);
    if (open) void loadBackups();
  };

  const restore = async (backup: LibraryBackup) => {
    setRestoring(true);
    setBackupError(null);
    try {
      await engine.libraryRestoreBackup(backup.file);
      setConfirming(null);
      setRestored(`Restored the backup from ${backupWhen(backup.at)}.`);
      await loadBackups();
    } catch (reason) {
      setBackupError(message(reason));
    } finally {
      setRestoring(false);
    }
  };

  const setBibtexOption = async (key: keyof BibtexOptions, value: boolean) => {
    const current = bibtex();
    if (!current) return;
    const next = { ...current, [key]: value };
    setBibtex(next);
    setBibtexError(null);
    try {
      setBibtex(await engine.librarySetBibtexOptions(next));
    } catch (reason) {
      setBibtex(current);
      setBibtexError(message(reason));
    }
  };

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
                Choose...
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
              Where library.json (your projects, memberships and edits) is kept. Choose a synced
              folder to share it between machines. A folder that already holds a library.json is
              adopted as it is, and a sync conflict is not merged: the copy that arrives last wins.
            </span>
          </label>
          <div class="library-setting-value">
            <Show
              when={dataDir()}
              fallback={
                <span class="library-setting-path">{dataError() ? 'unknown' : 'reading...'}</span>
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
                Move...
              </button>
            </div>
            <Show when={dataError()}>
              <span class="library-setting-error" role="alert">
                {dataError()}
              </span>
            </Show>
          </div>
        </div>

        <div class="home-setting">
          <label class="home-setting-text" for="setting-backups-restore">
            <span class="home-setting-title">Backups</span>
            <span class="home-setting-desc">
              library.json is backed up, compressed, into a backups folder beside it, at most once
              an hour while it changes. Every backup of the last day is kept, then one a day for two
              weeks and one a week for eight weeks.
            </span>
          </label>
          <div class="library-setting-value">
            <span class="library-setting-path">
              <Show
                when={backups()}
                fallback={backupError() && !restoreOpen() ? 'unknown' : 'reading...'}
              >
                {(list) => (
                  <Show when={list()[0]} fallback="No backup yet">
                    {(last) => `Last backup ${formatDate(Date.parse(last().at))}`}
                  </Show>
                )}
              </Show>
            </span>
            <div class="library-setting-actions">
              <button
                id="setting-backups-restore"
                type="button"
                class="secondary-btn"
                aria-expanded={restoreOpen()}
                onClick={toggleRestore}
              >
                Restore...
              </button>
            </div>
            <Show when={backupError()}>
              <span class="library-setting-error" role="alert">
                {backupError()}
              </span>
            </Show>
          </div>
        </div>
        <Show when={restoreOpen()}>
          <div class="settings-backups" role="region" aria-label="Backups to restore">
            <Show when={restored()}>
              <p class="settings-backups-note" role="status">
                {restored()} The state before it was backed up too.
              </p>
            </Show>
            <Show
              when={(backups() ?? []).length > 0}
              fallback={<p class="settings-backups-note">No backups yet.</p>}
            >
              <ul class="settings-backup-list">
                <For each={backups() ?? []}>
                  {(backup) => (
                    <li class="settings-backup">
                      <span class="settings-backup-when">{backupWhen(backup.at)}</span>
                      <span class="settings-backup-counts">
                        {plural(backup.papers, 'item', 'items')},{' '}
                        {plural(backup.projects, 'project', 'projects')}
                      </span>
                      <Show
                        when={confirming() === backup.file}
                        fallback={
                          <button
                            type="button"
                            class="secondary-btn"
                            disabled={restoring()}
                            aria-label={`Restore the backup from ${backupWhen(backup.at)}`}
                            onClick={() => setConfirming(backup.file)}
                          >
                            Restore
                          </button>
                        }
                      >
                        <span class="settings-backup-confirm">
                          <span>Replace the library with this backup?</span>
                          <button
                            type="button"
                            class="primary-btn"
                            disabled={restoring()}
                            onClick={() => void restore(backup)}
                          >
                            Replace
                          </button>
                          <button
                            type="button"
                            class="secondary-btn"
                            disabled={restoring()}
                            onClick={() => setConfirming(null)}
                          >
                            Cancel
                          </button>
                        </span>
                      </Show>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        </Show>

        <details class="settings-advanced">
          <summary class="settings-advanced-summary">Advanced</summary>
          <div class="settings-advanced-body">
            <h3 class="settings-group-title">BibTeX</h3>
            <p class="settings-group-desc">
              How entries are written when you copy BibTeX, export a .bib, and in synced .bib files,
              which are rewritten when an option changes. Kept with the library data.
            </p>
            <Show when={bibtexError()}>
              <span class="library-setting-error" role="alert">
                {bibtexError()}
              </span>
            </Show>
            <Show when={bibtex()}>
              {(options) => (
                <For each={BIBTEX_ROWS}>
                  {(row) => (
                    <div class="home-setting">
                      <label class="home-setting-text" for={`setting-bibtex-${row.key}`}>
                        <span class="home-setting-title">{row.title}</span>
                        <span class="home-setting-desc">{row.desc}</span>
                      </label>
                      <input
                        id={`setting-bibtex-${row.key}`}
                        type="checkbox"
                        checked={options()[row.key]}
                        onInput={(e) => void setBibtexOption(row.key, e.currentTarget.checked)}
                      />
                    </div>
                  )}
                </For>
              )}
            </Show>
          </div>
        </details>
      </div>
    </>
  );
}

/** Installed extensions: what is here, what it may do, and what went wrong.
 *
 * The error surface is the point of this panel as much as the list is. An
 * extension that fails to load, throws, or hangs has no console anyone will
 * look at, so whatever the host learned about it is reported here.
 */
import { For, Show, createSignal } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { IconExtension } from '../../components/icons';
import {
  bindingErrors,
  extensionState,
  installExtensionFromGit,
  setSettingValue,
  settingValue,
  installExtension,
  reloadExtension,
  setExtensionEnabled,
  uninstallExtension,
  type ExtensionRecord,
} from '../extensions/extensionStore';

export default function ExtensionsPanel() {
  const [busy, setBusy] = createSignal(false);
  const [installError, setInstallError] = createSignal<string | null>(null);
  const [url, setUrl] = createSignal('');

  const install = async (run: () => Promise<void>) => {
    setBusy(true);
    setInstallError(null);
    try {
      await run();
    } catch (err) {
      // Manifest and clone failures both arrive here already phrased for a
      // human — the Rust side names the field or the git error.
      setInstallError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const addFromUrl = async () => {
    const value = url().trim();
    if (!value) return;
    await install(() => installExtensionFromGit(value));
    if (!installError()) setUrl('');
  };

  const addFromFolder = async () => {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: 'Choose an extension folder',
    });
    if (typeof picked !== 'string') return;
    await install(() => installExtension(picked));
  };

  return (
    <>
      <div class="home-panel-head">
        <h2>Extensions</h2>
        <button
          type="button"
          class="secondary-btn"
          disabled={busy()}
          onClick={() => void addFromFolder()}
        >
          Load folder…
        </button>
      </div>
      <div class="home-panel-body">
        <div class="ext-install-url">
          <input
            type="text"
            placeholder="https://github.com/someone/extension#v1.0.0"
            aria-label="Extension repository URL"
            value={url()}
            disabled={busy()}
            onInput={(e) => setUrl(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void addFromUrl();
            }}
          />
          <button
            type="button"
            class="secondary-btn"
            disabled={busy() || url().trim().length === 0}
            onClick={() => void addFromUrl()}
          >
            {busy() ? 'Installing…' : 'Install'}
          </button>
        </div>
        <p class="ext-install-hint">
          Clones with git. Add <code>#branch</code>, <code>#tag</code> or <code>#commit</code> to
          pin a version.
        </p>

        <Show when={installError()}>
          <div class="ext-install-error" role="alert">
            {installError()}
          </div>
        </Show>

        <Show
          when={extensionState.extensions.length > 0}
          fallback={
            <Show when={extensionState.loaded}>
              <div class="home-placeholder">
                <span class="home-placeholder-icon" aria-hidden="true">
                  <IconExtension />
                </span>
                <p class="home-placeholder-title">No extensions installed</p>
                <p>
                  Extensions add commands to the command palette. Load one from a folder containing
                  a <code>speedyf.json</code>.
                </p>
              </div>
            </Show>
          }
        >
          <div class="ext-list">
            <For each={extensionState.extensions}>{(ext) => <ExtensionRow ext={ext} />}</For>
          </div>
        </Show>
      </div>
    </>
  );
}

function ExtensionRow(props: { ext: ExtensionRecord }) {
  const ext = () => props.ext;
  const [confirming, setConfirming] = createSignal(false);

  return (
    <div class="ext-row" classList={{ 'is-disabled': !ext().enabled, 'is-broken': !!ext().error }}>
      <div class="ext-row-head">
        <label class="ext-toggle" title={ext().enabled ? 'Disable' : 'Enable'}>
          <input
            type="checkbox"
            checked={ext().enabled}
            onInput={(e) => void setExtensionEnabled(ext().id, e.currentTarget.checked)}
          />
          <span class="sr-only">Enabled</span>
        </label>
        <div class="ext-identity">
          <span class="ext-name">{ext().name}</span>
          <span class="ext-version">{ext().version}</span>
        </div>
        <div class="ext-actions">
          <button type="button" class="ext-action" onClick={() => void reloadExtension(ext().id)}>
            Reload
          </button>
          <Show
            when={confirming()}
            fallback={
              <button type="button" class="ext-action" onClick={() => setConfirming(true)}>
                Remove
              </button>
            }
          >
            <button
              type="button"
              class="ext-action is-danger"
              onClick={() => void uninstallExtension(ext().id)}
            >
              Confirm
            </button>
            <button type="button" class="ext-action" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </Show>
        </div>
      </div>

      <Show when={ext().description}>
        <p class="ext-description">{ext().description}</p>
      </Show>

      <div class="ext-facts">
        <span class="ext-id" title={ext().source}>
          {ext().id}
        </span>
        <Show
          when={ext().permissions.length > 0}
          fallback={<span class="ext-perms is-none">no permissions</span>}
        >
          <span class="ext-perms">{ext().permissions.join(', ')}</span>
        </Show>
      </div>

      <Show when={ext().commands.length > 0}>
        <ul class="ext-commands">
          <For each={ext().commands}>
            {(command) => (
              <li>
                <span class="ext-command-title">{command.title}</span>
                <code class="ext-command-id">{command.id}</code>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <Show when={ext().keybindings.length > 0}>
        <ul class="ext-commands">
          <For each={ext().keybindings}>
            {(binding) => (
              <li>
                <span class="ext-command-title">{binding.command}</span>
                <kbd class="ext-key">{binding.key}</kbd>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <Show when={bindingErrors(ext().id).length > 0}>
        <div class="ext-error" role="status">
          <For each={bindingErrors(ext().id)}>{(message) => <div>{message}</div>}</For>
        </div>
      </Show>

      <Show when={ext().settings.length > 0}>
        <div class="ext-settings">
          <For each={ext().settings}>
            {(setting) => {
              const id = `ext-setting-${ext().id}-${setting.key}`;
              const value = () => settingValue(ext(), setting.key);
              const write = (next: string) => setSettingValue(ext().id, setting.key, next);
              return (
                <div class="ext-setting">
                  <label class="ext-setting-label" for={id}>
                    <span>{setting.title}</span>
                    <Show when={setting.description}>
                      <span class="ext-setting-desc">{setting.description}</span>
                    </Show>
                  </label>
                  <Show when={setting.type === 'checkbox'}>
                    <input
                      id={id}
                      type="checkbox"
                      checked={value() === 'true'}
                      onInput={(e) => write(e.currentTarget.checked ? 'true' : 'false')}
                    />
                  </Show>
                  <Show when={setting.type === 'select'}>
                    <select id={id} value={value()} onInput={(e) => write(e.currentTarget.value)}>
                      <For each={setting.options ?? []}>
                        {(option) => <option value={option}>{option}</option>}
                      </For>
                    </select>
                  </Show>
                  <Show when={setting.type === 'text'}>
                    <input
                      id={id}
                      type="text"
                      class="ext-setting-text"
                      value={value()}
                      onChange={(e) => write(e.currentTarget.value)}
                    />
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>

      <Show when={ext().error}>
        <div class="ext-error" role="status">
          {ext().error}
        </div>
      </Show>

      <Show when={ext().log.length > 0}>
        <details class="ext-log">
          <summary>{ext().log.length} log entries</summary>
          <For each={ext().log}>
            {(line) => (
              <div class="ext-log-line" classList={{ 'is-error': line.level === 'error' }}>
                {line.message}
              </div>
            )}
          </For>
        </details>
      </Show>
    </div>
  );
}

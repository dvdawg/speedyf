/** Installed extensions: what is here, what it may do, and what went wrong.
 *
 * Ordered by what someone came here for. The list is the page; installing is
 * occasional, so it sits underneath rather than greeting you with a form. Each
 * card gives identity, purpose and controls at a glance and folds the reference
 * material away — every contribution and log line expanded at once turned a
 * shelf of four extensions into a wall of text.
 *
 * The error surface is as much the point as the list. An extension that fails
 * to load, throws, or hangs has no console anyone will look at, so whatever the
 * host learned about it is reported here, and never behind a disclosure.
 */
import { For, Show, createSignal } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { IconExtension } from '../../components/icons';
import ExtensionIcon from '../extensions/ExtensionIcon';
import {
  bindingErrors,
  extensionState,
  installExtension,
  installExtensionFromGit,
  reloadExtension,
  setExtensionEnabled,
  setSettingValue,
  settingValue,
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
      return true;
    } catch (err) {
      // Manifest and clone failures arrive already phrased for a human — the
      // Rust side names the offending field, or the git error.
      setInstallError(err instanceof Error ? err.message : String(err));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const addFromUrl = async () => {
    const value = url().trim();
    if (!value) return;
    if (await install(() => installExtensionFromGit(value))) setUrl('');
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

  const count = () => extensionState.extensions.length;

  return (
    <>
      <div class="home-panel-head">
        <h2>Extensions</h2>
        <Show when={count() > 0}>
          <span class="ext-count">{count() === 1 ? '1 installed' : `${count()} installed`}</span>
        </Show>
      </div>
      <div class="home-panel-body">
        <Show
          when={count() > 0}
          fallback={
            <Show when={extensionState.loaded}>
              <div class="home-placeholder ext-empty">
                <span class="home-placeholder-icon" aria-hidden="true">
                  <IconExtension />
                </span>
                <p class="home-placeholder-title">No extensions installed</p>
                <p>
                  Extensions add commands, panels and actions to SpeedyF. Add one below, or read{' '}
                  <code>EXTENSIONS.md</code> to write your own.
                </p>
              </div>
            </Show>
          }
        >
          <div class="ext-list">
            <For each={extensionState.extensions}>{(ext) => <ExtensionRow ext={ext} />}</For>
          </div>
        </Show>

        <div class="ext-add">
          <h3 class="ext-add-title">Add an extension</h3>
          <div class="ext-add-row">
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
          <div class="ext-add-alt">
            <span>
              Clones with git — add <code>#tag</code> to pin a version.
            </span>
            <button
              type="button"
              class="ext-add-link"
              disabled={busy()}
              onClick={() => void addFromFolder()}
            >
              Load from a folder instead…
            </button>
          </div>
          <Show when={installError()}>
            <div class="ext-error" role="alert">
              {installError()}
            </div>
          </Show>
        </div>
      </div>
    </>
  );
}

function ExtensionRow(props: { ext: ExtensionRecord }) {
  const ext = () => props.ext;
  const [confirming, setConfirming] = createSignal(false);

  /** One line naming what this extension adds, so a card says what it does
   * without anything being unfolded. */
  const contributes = () => {
    const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? '' : 's'}`;
    const parts: string[] = [];
    if (ext().commands.length) parts.push(plural(ext().commands.length, 'command'));
    if (ext().panels.length) parts.push(plural(ext().panels.length, 'panel'));
    if (ext().keybindings.length) parts.push(plural(ext().keybindings.length, 'shortcut'));
    const actions = ext().selectionActions.length + ext().citationActions.length;
    if (actions) parts.push(plural(actions, 'action'));
    if (ext().documentTasks.length) parts.push(plural(ext().documentTasks.length, 'task'));
    return parts.join(' · ');
  };

  const hasDetails = () => ext().commands.length > 0 || ext().log.length > 0;

  return (
    <div class="ext-row" classList={{ 'is-disabled': !ext().enabled, 'is-broken': !!ext().error }}>
      <div class="ext-row-head">
        <ExtensionIcon name={ext().name} iconPath={ext().iconUrl} />
        <div class="ext-identity">
          <span class="ext-name">{ext().name}</span>
          <span class="ext-version">{ext().version}</span>
        </div>
        <div class="ext-actions">
          <button
            type="button"
            class="ext-action ext-toggle-btn"
            classList={{ 'is-on': ext().enabled }}
            aria-pressed={ext().enabled}
            title={ext().enabled ? 'Disable this extension' : 'Enable this extension'}
            onClick={() => void setExtensionEnabled(ext().id, !ext().enabled)}
          >
            {ext().enabled ? 'Enabled' : 'Disabled'}
          </button>
          <button
            type="button"
            class="ext-action"
            title="Restart with the current files"
            onClick={() => void reloadExtension(ext().id)}
          >
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
        <Show when={contributes()}>
          <span class="ext-contributes">{contributes()}</span>
        </Show>
        <Show
          when={ext().permissions.length > 0}
          fallback={<span class="ext-perms is-none">no permissions</span>}
        >
          {/* Collapsed to a count past one: the full list is what the title
              attribute is for, and eight permission strings in a row buried
              everything else on the card. */}
          <span class="ext-perms" title={ext().permissions.join('\n')}>
            {ext().permissions.length === 1
              ? ext().permissions[0]
              : `${ext().permissions.length} permissions`}
          </span>
        </Show>
      </div>

      {/* Never folded away: an error is the reason someone opened this page. */}
      <Show when={ext().error}>
        <div class="ext-error" role="status">
          {ext().error}
        </div>
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

      <Show when={hasDetails()}>
        <details class="ext-details">
          <summary>
            What it adds{ext().log.length > 0 ? `, and ${ext().log.length} log lines` : ''}
          </summary>
          <Show when={ext().commands.length > 0}>
            <ul class="ext-commands">
              <For each={ext().commands}>
                {(command) => {
                  const chord = () =>
                    ext().keybindings.find((k) => k.command === command.localId)?.key;
                  return (
                    <li>
                      <span class="ext-command-title">{command.title}</span>
                      <Show when={chord()}>
                        <kbd class="ext-key">{chord()}</kbd>
                      </Show>
                    </li>
                  );
                }}
              </For>
            </ul>
          </Show>
          <Show when={ext().log.length > 0}>
            <div class="ext-log">
              <For each={ext().log.slice(-20)}>
                {(line) => (
                  <div class="ext-log-line" classList={{ 'is-error': line.level === 'error' }}>
                    {line.message}
                  </div>
                )}
              </For>
            </div>
          </Show>
        </details>
      </Show>
    </div>
  );
}

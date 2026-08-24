/** What is installed, what is running, and what each one is allowed to do.
 *
 * One store for the window: extensions are not per-tab or per-document, and
 * v1 has nothing document-scoped to offer them anyway.
 *
 * Workers are started lazily. An installed extension costs nothing until one of
 * its commands is actually run, which keeps a shelf of installed-but-unused
 * extensions off the startup path.
 */
import { createStore, produce } from 'solid-js/store';
import { invoke } from '@tauri-apps/api/core';
import { ExtensionHost, blobWorkerFactory } from './host';
import type { CallMessage } from './protocol';

export interface ExtensionCommand {
  /** `<extension id>/<command id>`, namespaced by the Rust side. */
  id: string;
  localId: string;
  title: string;
  keywords: string[];
}

export interface ExtensionManifest {
  id: string;
  name: string;
  version: string;
  permissions: string[];
}

export interface ExtensionRecord {
  id: string;
  name: string;
  version: string;
  description?: string;
  source: string;
  permissions: string[];
  enabled: boolean;
  manifest: ExtensionManifest | null;
  commands: ExtensionCommand[];
  /** Why it cannot run: a bad manifest, or a worker that died. */
  error: string | null;
  /** Most recent diagnostics, newest last. Bounded — an extension logging in a
   * loop must not grow the store without limit. */
  log: { level: string; message: string }[];
}

const MAX_LOG_LINES = 50;

interface State {
  loaded: boolean;
  extensions: ExtensionRecord[];
}

const [state, setState] = createStore<State>({ loaded: false, extensions: [] });

export { state as extensionState };

/** Live workers, keyed by extension id. Not in the store: a Worker is not
 * serializable state and nothing should re-render because one started. */
const hosts = new Map<string, ExtensionHost>();

interface ExtensionDto {
  id: string;
  name: string;
  version: string;
  description?: string;
  source: string;
  permissions: string[];
  enabled: boolean;
  manifest: ExtensionManifest | null;
  commands: ExtensionCommand[];
  error: string | null;
}

export async function loadExtensions(): Promise<void> {
  try {
    const list = await invoke<ExtensionDto[]>('list_extensions');
    setState(
      produce((s) => {
        // Keep the log of anything still installed; it is the only record of
        // why an extension misbehaved, and a refresh should not erase it.
        const previous = new Map(s.extensions.map((e) => [e.id, e.log]));
        s.extensions = list.map((dto) => ({
          ...dto,
          error: dto.error ?? null,
          log: previous.get(dto.id) ?? [],
        }));
        s.loaded = true;
      })
    );
  } catch (err) {
    setState('loaded', true);
    console.error('could not list extensions', err);
  }
}

/** Every command contributed by an enabled, error-free extension. */
export function extensionCommands(): ExtensionCommand[] {
  return state.extensions.filter((e) => e.enabled && !e.error).flatMap((e) => e.commands);
}

export function findExtensionForCommand(qualifiedId: string): ExtensionRecord | undefined {
  return state.extensions.find((e) => e.commands.some((c) => c.id === qualifiedId));
}

function record(id: string): ExtensionRecord | undefined {
  return state.extensions.find((e) => e.id === id);
}

function appendLog(id: string, level: string, message: string): void {
  setState(
    'extensions',
    (e) => e.id === id,
    produce((e: ExtensionRecord) => {
      e.log.push({ level, message });
      if (e.log.length > MAX_LOG_LINES) e.log.splice(0, e.log.length - MAX_LOG_LINES);
    })
  );
}

function setError(id: string, error: string | null): void {
  setState('extensions', (e) => e.id === id, 'error', error);
}

/** The permission gate.
 *
 * v1 exposes no APIs to extensions at all, so every call is refused. The gate
 * exists anyway, and refuses by default, because the alternative — adding
 * permissions once extensions already run unrestricted — is a breaking change
 * for every extension in the wild. Refusing from the first release means
 * nothing is ever built against access it was not granted.
 */
async function handleCall(extension: ExtensionRecord, call: CallMessage): Promise<unknown> {
  throw new Error(
    `"${call.api}" is not available to extensions in this version of SpeedyF` +
      (extension.permissions.length === 0 ? '' : ` (granted: ${extension.permissions.join(', ')})`)
  );
}

function hostFor(extension: ExtensionRecord): ExtensionHost {
  const existing = hosts.get(extension.id);
  if (existing) return existing;

  const host = new ExtensionHost({
    id: extension.id,
    apiVersion: 1,
    permissions: extension.permissions,
    spawn: async () => {
      const source = await invoke<string>('extension_source', { id: extension.id });
      return blobWorkerFactory(source)();
    },
    events: {
      onLog: (level, message) => appendLog(extension.id, level, message),
      onCrash: (reason) => {
        appendLog(extension.id, 'error', reason);
        setError(extension.id, reason);
        hosts.delete(extension.id);
      },
      onCall: (call) => handleCall(extension, call),
    },
  });
  hosts.set(extension.id, host);
  return host;
}

/** Run a contributed command by its namespaced id.
 *
 * Resolves when the extension reports done, and never hangs — the host times
 * out. Failures are recorded against the extension rather than thrown at the
 * caller, because the caller is a palette row that has already closed.
 */
export async function runExtensionCommand(qualifiedId: string): Promise<void> {
  const extension = findExtensionForCommand(qualifiedId);
  if (!extension || !extension.enabled) return;
  const command = extension.commands.find((c) => c.id === qualifiedId);
  if (!command) return;

  try {
    await hostFor(extension).invoke(command.localId);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    appendLog(extension.id, 'error', `${command.title}: ${message}`);
  }
}

/** Stop an extension's worker without touching what is installed. */
export function stopExtension(id: string): void {
  hosts.get(id)?.stop();
  hosts.delete(id);
}

export async function setExtensionEnabled(id: string, enabled: boolean): Promise<void> {
  // A disabled extension must not keep running, so the worker goes first —
  // if the write fails, the worse outcome is a stopped extension, not a
  // running one the user believes is off.
  if (!enabled) stopExtension(id);
  await invoke('set_extension_enabled', { id, enabled });
  await loadExtensions();
}

export async function installExtension(folder: string): Promise<void> {
  await invoke('install_extension', { folder });
  await loadExtensions();
}

export async function uninstallExtension(id: string): Promise<void> {
  stopExtension(id);
  await invoke('uninstall_extension', { id });
  await loadExtensions();
}

/** Restart a running extension — the developer-loop action, and the way to
 * clear a crash without reinstalling. */
export async function reloadExtension(id: string): Promise<void> {
  stopExtension(id);
  setError(id, null);
  await loadExtensions();
  const extension = record(id);
  if (extension?.enabled) {
    try {
      await hostFor(extension).start();
    } catch {
      /* onCrash has already recorded why */
    }
  }
}

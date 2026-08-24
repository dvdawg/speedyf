/** Running one extension.
 *
 * Extension code runs in a Web Worker: no DOM, no access to app state, and one
 * channel out. That is measured rather than assumed — the spike that settled
 * this confirmed `document` and `window` are both absent inside the worker.
 *
 * The worker is created from a blob URL built out of source the Rust side
 * reads, not from a URL on a custom scheme. `new Worker(url)` requires the URL
 * to be same-origin with the creating document, which no CSP setting relaxes
 * and which a custom scheme can never satisfy; blob URLs inherit the creating
 * document's origin, so they do. A useful side effect: the worker inherits the
 * app's CSP, whose `connect-src` is `ipc:` alone, so an extension cannot reach
 * the network at all — denied by the platform rather than by our policy.
 *
 * Everything here is defensive by default. A hung extension resolves by
 * timeout, a throwing extension is caught and reported, and a crashed worker
 * takes nothing else down with it.
 */
import {
  ACTIVATE_TIMEOUT_MS,
  INVOKE_TIMEOUT_MS,
  parseExtensionMessage,
  type CallMessage,
  type HostMessage,
} from './protocol';

export type HostStatus = 'stopped' | 'starting' | 'running' | 'failed';

export interface HostEvents {
  /** A diagnostic worth showing in the Extensions panel. */
  onLog(level: 'info' | 'warn' | 'error', message: string): void;
  /** The worker died or failed to start. The host is already stopped. */
  onCrash(reason: string): void;
  /** An extension asked the host to do something. Resolve or reject; the host
   * turns either into a reply. Permission checks live in the caller, which is
   * what knows what the user granted. */
  onCall(call: CallMessage): Promise<unknown>;
}

/** Anything Worker-shaped, so tests can drive a fake one. */
export interface WorkerLike {
  postMessage(message: unknown): void;
  terminate(): void;
  onmessage: ((event: { data: unknown }) => void) | null;
  onerror: ((event: { message?: string }) => void) | null;
}

export interface ExtensionHostOptions {
  id: string;
  apiVersion: number;
  permissions: readonly string[];
  /** Injected so tests need no real Worker, and so the blob plumbing has one
   * place to live. */
  spawn(): Promise<WorkerLike>;
  events: HostEvents;
  /** Overridable for tests; real callers take the protocol defaults. */
  invokeTimeoutMs?: number;
  activateTimeoutMs?: number;
}

interface Pending {
  resolve(): void;
  reject(reason: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

export class ExtensionHost {
  private worker: WorkerLike | null = null;
  private nextRequestId = 1;
  private pending = new Map<number, Pending>();
  private state: HostStatus = 'stopped';
  private starting: Promise<void> | null = null;

  constructor(private readonly options: ExtensionHostOptions) {}

  get status(): HostStatus {
    return this.state;
  }

  /** Start the worker and wait for it to acknowledge activation.
   *
   * Concurrent starts share one attempt: two commands invoked in the same tick
   * on a stopped extension must not race two workers into existence.
   */
  async start(): Promise<void> {
    if (this.state === 'running') return;
    if (this.starting) return this.starting;
    this.starting = this.doStart().finally(() => {
      this.starting = null;
    });
    return this.starting;
  }

  private async doStart(): Promise<void> {
    this.state = 'starting';
    let worker: WorkerLike;
    try {
      worker = await this.options.spawn();
    } catch (err) {
      this.state = 'failed';
      const reason = `failed to load: ${describe(err)}`;
      this.options.events.onCrash(reason);
      throw new Error(reason);
    }

    this.worker = worker;
    worker.onmessage = (event) => this.receive(event.data);
    worker.onerror = (event) => {
      // A worker error is terminal for that worker: whatever threw did so
      // outside any call we can attribute it to.
      this.fail(`worker error: ${event.message || 'unknown'}`);
    };

    const activate: HostMessage = {
      type: 'activate',
      id: this.options.id,
      apiVersion: this.options.apiVersion,
      // Copied into a plain array on purpose. Callers pass state straight out
      // of a Solid store, whose arrays are Proxies, and postMessage structured-
      // clones its argument — a Proxy throws "object can not be cloned". Every
      // message crossing to a worker must be plain data, and this is the
      // boundary that owes that guarantee rather than each caller.
      permissions: [...this.options.permissions],
    };
    try {
      worker.postMessage(activate);
    } catch (err) {
      this.fail(`could not activate: ${describe(err)}`);
      throw new Error('could not activate');
    }
    // Activation is not acknowledged in v1 — an extension with no commands has
    // nothing to say — so reaching this point without an error is success.
    this.state = 'running';
  }

  /** Run one of the extension's commands, by its bare (un-namespaced) id.
   *
   * Resolves when the extension reports done, rejects on error or timeout.
   * Never hangs: the caller can await this from a click handler safely.
   */
  async invoke(command: string): Promise<void> {
    await this.start();
    const worker = this.worker;
    if (!worker || this.state !== 'running') {
      throw new Error('extension is not running');
    }
    const requestId = this.nextRequestId++;
    return new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        // Deliberately not a crash: a slow command is not a broken extension,
        // and killing the worker would lose any state it had built up.
        reject(new Error(`"${command}" did not finish within ${this.timeout()}ms`));
      }, this.timeout());
      this.pending.set(requestId, { resolve, reject, timer });
      try {
        worker.postMessage({ type: 'invoke', requestId, command } satisfies HostMessage);
      } catch (err) {
        this.settle(requestId, new Error(describe(err)));
      }
    });
  }

  /** Stop the worker, failing anything still outstanding. */
  stop(): void {
    const worker = this.worker;
    this.worker = null;
    this.state = 'stopped';
    for (const id of [...this.pending.keys()]) {
      this.settle(id, new Error('extension stopped'));
    }
    if (worker) {
      worker.onmessage = null;
      worker.onerror = null;
      try {
        worker.terminate();
      } catch {
        /* already gone */
      }
    }
  }

  async restart(): Promise<void> {
    this.stop();
    await this.start();
  }

  private timeout(): number {
    return this.options.invokeTimeoutMs ?? INVOKE_TIMEOUT_MS;
  }

  private receive(data: unknown): void {
    const message = parseExtensionMessage(data);
    if (!message) {
      // Not a crash. A malformed message is a bug in the extension, and
      // reporting it is more useful than tearing the worker down.
      this.options.events.onLog('warn', 'ignored a malformed message from the extension');
      return;
    }
    switch (message.type) {
      case 'log':
        this.options.events.onLog(message.level, message.message);
        return;
      case 'done':
        this.settle(
          message.requestId,
          message.ok ? null : new Error(message.error || 'command failed')
        );
        return;
      case 'call':
        void this.handleCall(message);
        return;
    }
  }

  private async handleCall(call: CallMessage): Promise<void> {
    let reply: HostMessage;
    try {
      const value = await this.options.events.onCall(call);
      reply = { type: 'reply', requestId: call.requestId, ok: true, value };
    } catch (err) {
      reply = { type: 'reply', requestId: call.requestId, ok: false, error: describe(err) };
    }
    // The worker may have gone away while the host was working.
    try {
      this.worker?.postMessage(reply);
    } catch {
      /* worker gone; nothing to answer */
    }
  }

  /** Resolve or reject one outstanding invoke, clearing its timer exactly once. */
  private settle(requestId: number, error: Error | null): void {
    const pending = this.pending.get(requestId);
    if (!pending) return;
    this.pending.delete(requestId);
    clearTimeout(pending.timer);
    if (error) pending.reject(error);
    else pending.resolve();
  }

  private fail(reason: string): void {
    this.stop();
    this.state = 'failed';
    this.options.events.onCrash(reason);
  }
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Build a worker from extension source.
 *
 * Separated from the host so tests never touch a real Worker, and so the one
 * place that knows about blobs is this function.
 */
export function blobWorkerFactory(source: string): () => Promise<WorkerLike> {
  return async () => {
    const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
    try {
      const worker = new Worker(url) as unknown as WorkerLike;
      return worker;
    } finally {
      // Safe immediately: the worker has already been handed the blob.
      URL.revokeObjectURL(url);
    }
  };
}

export { ACTIVATE_TIMEOUT_MS };

import { describe, expect, it, vi } from 'vitest';
import { ExtensionHost, type WorkerLike } from './host';
import { parseExtensionMessage } from './protocol';

/** A worker under the test's control: it only does what a case tells it to. */
class FakeWorker implements WorkerLike {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((event: { message?: string }) => void) | null = null;
  sent: unknown[] = [];
  terminated = false;
  /** Set to reply automatically to every invoke. */
  autoDone: 'ok' | 'fail' | null = null;
  throwOnPost = false;

  postMessage(message: unknown): void {
    if (this.throwOnPost) throw new Error('channel closed');
    // A real Worker structured-clones its argument, and anything that cannot
    // be cloned — a Proxy, a function, a class instance — throws here rather
    // than at the far end. The fake enforces the same rule so a message the
    // real worker would reject cannot pass a test.
    structuredClone(message);
    this.sent.push(message);
    const m = message as { type?: string; requestId?: number };
    if (this.autoDone && m.type === 'invoke') {
      queueMicrotask(() =>
        this.emit(
          this.autoDone === 'ok'
            ? { type: 'done', requestId: m.requestId, ok: true }
            : { type: 'done', requestId: m.requestId, ok: false, error: 'boom' }
        )
      );
    }
  }

  terminate(): void {
    this.terminated = true;
  }

  emit(data: unknown): void {
    this.onmessage?.({ data });
  }

  invokeAt(index: number): { requestId: number; command: string } {
    const found = this.invokes()[index];
    if (!found) throw new Error(`no invoke at index ${index}`);
    return found;
  }

  /** The invokes seen so far. Indexing is checked by `invokeAt`, so cases can
   * read one without repeating a non-null assertion. */
  invokes(): { requestId: number; command: string }[] {
    return this.sent.filter(
      (m): m is { type: 'invoke'; requestId: number; command: string } =>
        (m as { type?: string }).type === 'invoke'
    );
  }
}

/** Let queued microtasks run. `invoke()` awaits `start()` before it posts, so
 * a message is never on the wire in the same tick the call was made. */
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function makeHost(worker: FakeWorker, overrides: Record<string, unknown> = {}) {
  const logs: [string, string][] = [];
  const crashes: string[] = [];
  const host = new ExtensionHost({
    id: 'com.example.thing',
    apiVersion: 1,
    permissions: ['citations:read'],
    spawn: async () => worker,
    invokeTimeoutMs: 50,
    events: {
      onLog: (level, message) => logs.push([level, message]),
      onCrash: (reason) => crashes.push(reason),
      onCall: async () => undefined,
    },
    ...overrides,
  });
  return { host, logs, crashes };
}

describe('ExtensionHost lifecycle', () => {
  /** Regression: permissions arrive from a Solid store, whose arrays are
   * Proxies. postMessage structured-clones, and a Proxy is not cloneable, so
   * passing the caller's array straight through broke activation for every
   * extension. */
  it('sends plain data even when handed reactive state', async () => {
    const worker = new FakeWorker();
    const proxied = new Proxy(['citations:read'], {});
    const { host } = makeHost(worker, { permissions: proxied });

    await expect(host.start()).resolves.toBeUndefined();
    expect(worker.sent[0]).toEqual({
      type: 'activate',
      id: 'com.example.thing',
      apiVersion: 1,
      permissions: ['citations:read'],
    });
  });

  it('activates the worker with its identity and granted permissions', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker);
    await host.start();

    expect(host.status).toBe('running');
    expect(worker.sent[0]).toEqual({
      type: 'activate',
      id: 'com.example.thing',
      apiVersion: 1,
      permissions: ['citations:read'],
    });
  });

  it('shares one worker between concurrent starts', async () => {
    const worker = new FakeWorker();
    const spawn = vi.fn(async () => worker);
    const { host } = makeHost(worker, { spawn });

    await Promise.all([host.start(), host.start(), host.start()]);
    expect(spawn).toHaveBeenCalledTimes(1);
  });

  it('reports a worker that cannot be loaded, and stays failed', async () => {
    const worker = new FakeWorker();
    const { host, crashes } = makeHost(worker, {
      spawn: async () => {
        throw new Error('no such file');
      },
    });

    await expect(host.start()).rejects.toThrow(/no such file/);
    expect(host.status).toBe('failed');
    expect(crashes[0]).toMatch(/no such file/);
  });

  it('terminates the worker on stop', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker);
    await host.start();
    host.stop();

    expect(worker.terminated).toBe(true);
    expect(host.status).toBe('stopped');
  });
});

describe('ExtensionHost invoke', () => {
  it('resolves when the extension reports done', async () => {
    const worker = new FakeWorker();
    worker.autoDone = 'ok';
    const { host } = makeHost(worker);

    await expect(host.invoke('export')).resolves.toBeUndefined();
    expect(worker.invokeAt(0).command).toBe('export');
  });

  it('rejects with the extensions own error', async () => {
    const worker = new FakeWorker();
    worker.autoDone = 'fail';
    const { host } = makeHost(worker);

    await expect(host.invoke('export')).rejects.toThrow('boom');
  });

  it('starts the worker on first invoke', async () => {
    const worker = new FakeWorker();
    worker.autoDone = 'ok';
    const { host } = makeHost(worker);

    await host.invoke('export');
    expect(host.status).toBe('running');
  });

  /** The point of the whole design: third-party code cannot hang the UI. */
  it('gives up on an extension that never answers', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker);

    await expect(host.invoke('hangs')).rejects.toThrow(/did not finish/);
    // Not a crash — a slow command is not proof of a broken extension.
    expect(host.status).toBe('running');
  });

  it('keeps concurrent invokes apart by correlation id', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker);
    await host.start();

    const first = host.invoke('a');
    const second = host.invoke('b');
    await tick();
    const [a, b] = [worker.invokeAt(0), worker.invokeAt(1)];
    expect(a.requestId).not.toBe(b.requestId);

    // Answer them out of order; each must settle its own caller.
    worker.emit({ type: 'done', requestId: b.requestId, ok: false, error: 'second failed' });
    worker.emit({ type: 'done', requestId: a.requestId, ok: true });

    await expect(first).resolves.toBeUndefined();
    await expect(second).rejects.toThrow('second failed');
  });

  it('ignores a reply to a request that already settled', async () => {
    const worker = new FakeWorker();
    worker.autoDone = 'ok';
    const { host } = makeHost(worker);
    await host.invoke('export');
    const { requestId } = worker.invokeAt(0);

    // A late duplicate must not throw or resolve anything a second time.
    expect(() => worker.emit({ type: 'done', requestId, ok: false, error: 'late' })).not.toThrow();
  });

  it('fails outstanding invokes when the extension is stopped', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker);
    await host.start();
    const pending = host.invoke('slow');
    await tick();
    host.stop();

    await expect(pending).rejects.toThrow(/stopped/);
  });

  it('surfaces a dead channel rather than hanging', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker);
    await host.start();
    worker.throwOnPost = true;

    await expect(host.invoke('export')).rejects.toThrow(/channel closed/);
  });
});

describe('ExtensionHost message handling', () => {
  it('forwards logs to the panel instead of the console', async () => {
    const worker = new FakeWorker();
    const { host, logs } = makeHost(worker);
    await host.start();

    worker.emit({ type: 'log', level: 'warn', message: 'careful' });
    expect(logs).toContainEqual(['warn', 'careful']);
  });

  it('drops a malformed message without killing the worker', async () => {
    const worker = new FakeWorker();
    const { host, logs, crashes } = makeHost(worker);
    await host.start();

    worker.emit({ type: 'done' }); // no requestId
    worker.emit('not an object');
    worker.emit(null);

    expect(host.status).toBe('running');
    expect(crashes).toEqual([]);
    expect(logs.filter(([level]) => level === 'warn')).toHaveLength(3);
  });

  it('answers a call with a reply carrying the same id', async () => {
    const worker = new FakeWorker();
    const { host } = makeHost(worker, {
      events: {
        onLog: () => {},
        onCrash: () => {},
        onCall: async (call: { api: string }) => `handled ${call.api}`,
      },
    });
    await host.start();

    worker.emit({ type: 'call', requestId: 9, api: 'citations.list' });
    await vi.waitFor(() => {
      expect(worker.sent).toContainEqual({
        type: 'reply',
        requestId: 9,
        ok: true,
        value: 'handled citations.list',
      });
    });
  });

  it('turns a refused call into an error reply rather than a crash', async () => {
    const worker = new FakeWorker();
    const { host, crashes } = makeHost(worker, {
      events: {
        onLog: () => {},
        onCrash: () => {},
        onCall: async () => {
          throw new Error('permission "network" not granted');
        },
      },
    });
    await host.start();

    worker.emit({ type: 'call', requestId: 3, api: 'net.fetch' });
    await vi.waitFor(() => {
      expect(worker.sent).toContainEqual({
        type: 'reply',
        requestId: 3,
        ok: false,
        error: 'permission "network" not granted',
      });
    });
    expect(crashes).toEqual([]);
    expect(host.status).toBe('running');
  });

  it('treats a worker error as terminal and reports it', async () => {
    const worker = new FakeWorker();
    const { host, crashes } = makeHost(worker);
    await host.start();

    worker.onerror?.({ message: 'ReferenceError: x is not defined' });
    expect(host.status).toBe('failed');
    expect(crashes[0]).toMatch(/ReferenceError/);
    expect(worker.terminated).toBe(true);
  });
});

describe('parseExtensionMessage', () => {
  it('accepts the three shapes an extension may send', () => {
    expect(parseExtensionMessage({ type: 'call', requestId: 1, api: 'a.b' })).toMatchObject({
      type: 'call',
    });
    expect(parseExtensionMessage({ type: 'done', requestId: 1, ok: true })).toMatchObject({
      type: 'done',
    });
    expect(parseExtensionMessage({ type: 'log', level: 'info', message: 'x' })).toMatchObject({
      type: 'log',
    });
  });

  it('rejects anything else', () => {
    for (const bad of [
      null,
      'string',
      42,
      {},
      { type: 'call' },
      { type: 'call', requestId: '1', api: 'a' },
      { type: 'done', requestId: 1 },
      { type: 'log', level: 'shout', message: 'x' },
      { type: 'evaluate', code: 'process.exit()' },
    ]) {
      expect(parseExtensionMessage(bad)).toBeNull();
    }
  });
});

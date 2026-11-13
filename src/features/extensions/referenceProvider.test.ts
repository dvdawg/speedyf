import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { canCallApi, readReferenceEntries } from './api';
import { ExtensionHost, type WorkerLike } from './host';
import {
  RECHECK_MS,
  ReferenceQueue,
  chooseProvider,
  providerTag,
  type PendingDocument,
  type ProviderChoice,
  type ReferenceQueueDeps,
} from './referenceProvider';
import { REFERENCE_PROVIDER_TIMEOUT_MS, type ReferenceRequest } from './protocol';

const REFERENCES: ProviderChoice = {
  extensionId: 'dev.speedyf.references',
  version: '4.0.0',
  command: 'provide',
};

/** An engine and an extension under the test's control. `answer` decides
 * what the provider does with each document it is handed. */
function harness(
  documents: PendingDocument[],
  answer: (
    value: ReferenceRequest,
    contribute: (args: unknown) => unknown
  ) => Promise<void> = async () => {}
) {
  const queue: PendingDocument[] = [...documents];
  const stored: { token: string; entries: unknown }[] = [];
  const tags: (string | null)[] = [];
  const invoked: { value: ReferenceRequest; timeoutMs: number }[] = [];
  const logs: string[] = [];
  let busy = false;
  let pendingCalls = 0;
  const deps: ReferenceQueueDeps = {
    setProvider: async (tag) => {
      tags.push(tag);
    },
    pending: async (limit) => {
      pendingCalls++;
      return queue.splice(0, limit);
    },
    store: async (token, entries) => {
      stored.push({ token, entries });
    },
    invoke: async (choice, value, timeoutMs) => {
      invoked.push({ value, timeoutMs });
      await answer(value, (args) => q.contribute(choice.extensionId, args));
    },
    busy: () => busy,
    // Nothing pending: park, as a real recheck a minute later would.
    wait: (ms) =>
      ms === RECHECK_MS ? new Promise(() => {}) : new Promise((r) => setTimeout(r, 0)),
    log: (_id, level, message) => logs.push(`${level}: ${message}`),
  };
  const q = new ReferenceQueue(deps);
  return {
    q,
    stored,
    tags,
    invoked,
    logs,
    add: (document: PendingDocument) => queue.push(document),
    setBusy: (value: boolean) => {
      busy = value;
    },
    pendingCalls: () => pendingCalls,
  };
}

describe('choosing the reference provider', () => {
  const base = {
    version: '1.0.0',
    enabled: true,
    error: null,
    permissions: ['library:contributeReferences'],
    referenceProvider: 'provide',
  };

  it('is the first enabled, working extension that declares one with the permission', () => {
    expect(
      chooseProvider([
        { ...base, id: 'a.off', enabled: false },
        { ...base, id: 'b.broken', error: 'bad manifest' },
        { ...base, id: 'c.ungranted', permissions: [] },
        { ...base, id: 'd.none', referenceProvider: null },
        { ...base, id: 'e.first' },
        { ...base, id: 'f.second' },
      ])
    ).toEqual({ extensionId: 'e.first', version: '1.0.0', command: 'provide' });
    expect(chooseProvider([{ ...base, id: 'a.off', enabled: false }])).toBeNull();
  });

  it('is known to the engine by id and version', () => {
    expect(providerTag(REFERENCES)).toBe('dev.speedyf.references@4.0.0');
    expect(providerTag(null)).toBeNull();
  });

  it('is the only thing that may contribute references', () => {
    expect(canCallApi(['library:contributeReferences'], 'references.contribute')).toBe(true);
    expect(canCallApi(['library:read', 'document:read'], 'references.contribute')).toBe(false);
  });
});

describe('the reference queue', () => {
  it('hands each pending document to the provider and stores what it contributed', async () => {
    const h = harness(
      [
        { token: 't1', text: 'one' },
        { token: 't2', text: 'two' },
      ],
      async (value, contribute) => {
        contribute({ document: value.document, entries: [{ label: '1', text: value.text }] });
      }
    );
    await h.q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(h.stored).toHaveLength(2));
    expect(h.tags).toEqual(['dev.speedyf.references@4.0.0']);
    expect(h.invoked).toEqual([
      { value: { document: 't1', text: 'one' }, timeoutMs: REFERENCE_PROVIDER_TIMEOUT_MS },
      { value: { document: 't2', text: 'two' }, timeoutMs: REFERENCE_PROVIDER_TIMEOUT_MS },
    ]);
    expect(h.stored).toEqual([
      { token: 't1', entries: [{ label: '1', text: 'one' }] },
      { token: 't2', entries: [{ label: '1', text: 'two' }] },
    ]);
    expect(REFERENCE_PROVIDER_TIMEOUT_MS).toBeGreaterThan(5_000);
  });

  it('stores an empty list for a provider that found nothing', async () => {
    const h = harness([{ token: 't1', text: 'prose' }]);
    await h.q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(h.stored).toEqual([{ token: 't1', entries: [] }]));
  });

  it('gives a document up when the provider fails or times out, and goes on', async () => {
    const h = harness(
      [
        { token: 'slow', text: 'a thesis' },
        { token: 't2', text: 'two' },
      ],
      async (value, contribute) => {
        if (value.document === 'slow') {
          // It contributed, then timed out: what it sent is not used.
          contribute({ document: 'slow', entries: [{ text: 'partial' }] });
          throw new Error(`"provide" did not finish within ${REFERENCE_PROVIDER_TIMEOUT_MS}ms`);
        }
        contribute({ document: value.document, entries: [{ text: 'ok' }] });
      }
    );
    await h.q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(h.stored).toHaveLength(2));
    expect(h.stored).toEqual([
      { token: 'slow', entries: null },
      { token: 't2', entries: [{ text: 'ok' }] },
    ]);
    expect(h.logs[0]).toContain('did not finish within 20000ms');
  });

  it('refuses a contribution for any document but the one being read', async () => {
    const h = harness([{ token: 't1', text: 'one' }], async (_value, contribute) => {
      expect(() => contribute({ document: 'someone-else', entries: [] })).toThrow(
        /handed this extension/
      );
    });
    expect(() => h.q.contribute(REFERENCES.extensionId, { document: 't1', entries: [] })).toThrow();
    await h.q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(h.stored).toHaveLength(1));
    expect(() => h.q.contribute('com.other.thing', { document: 't1', entries: [] })).toThrow();
  });

  it('does nothing with no provider, and tells the engine so', async () => {
    const h = harness([{ token: 't1', text: 'one' }]);
    await h.q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(h.stored).toHaveLength(1));
    h.add({ token: 't2', text: 'two' });
    await h.q.setProvider(null);
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(h.tags).toEqual(['dev.speedyf.references@4.0.0', null]);
    expect(h.stored).toHaveLength(1);
    expect(h.invoked).toHaveLength(1);
  });

  it('reads again under a new version, and when re-enabled', async () => {
    const h = harness([{ token: 't1', text: 'one' }]);
    await h.q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(h.stored).toHaveLength(1));
    // Same provider again: nothing to tell the engine.
    await h.q.setProvider({ ...REFERENCES });
    expect(h.tags).toHaveLength(1);
    // A new version: the engine re-queues every document, the queue asks.
    h.add({ token: 't1-again', text: 'one' });
    await h.q.setProvider({ ...REFERENCES, version: '4.0.1' });
    await vi.waitFor(() => expect(h.stored).toHaveLength(2));
    expect(h.tags.at(-1)).toBe('dev.speedyf.references@4.0.1');
    await h.q.setProvider(null);
    h.add({ token: 't3', text: 'three' });
    await h.q.setProvider({ ...REFERENCES, version: '4.0.1' });
    await vi.waitFor(() => expect(h.stored).toHaveLength(3));
  });

  it('waits while the reader is busy', async () => {
    const h = harness([{ token: 't1', text: 'one' }]);
    h.setBusy(true);
    await h.q.setProvider(REFERENCES);
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(h.pendingCalls()).toBe(0);
    h.setBusy(false);
    await vi.waitFor(() => expect(h.stored).toHaveLength(1));
  });
});

describe('references.contribute arguments', () => {
  it('drops malformed entries one by one and caps the rest', () => {
    const { document, entries } = readReferenceEntries({
      document: 'tok',
      entries: [
        null,
        'a string',
        { text: 7 },
        { text: '   ' },
        { label: 3, text: 'kept, label dropped', year: '2001' },
        { label: '1', text: ` ${'x'.repeat(2000)} `, year: 2001 },
        { label: 'x'.repeat(40), text: 'kept', year: 2001.5 },
      ],
    });
    expect(document).toBe('tok');
    expect(entries).toEqual([
      { text: 'kept, label dropped' },
      { label: '1', text: 'x'.repeat(1500), year: 2001 },
      { text: 'kept' },
    ]);
    const many = readReferenceEntries({
      document: 'tok',
      entries: Array.from({ length: 1200 }, (_, n) => ({ text: `e${n}` })),
    });
    expect(many.entries).toHaveLength(1000);
  });

  it('refuses arguments without a document or an entries array', () => {
    expect(() => readReferenceEntries({ entries: [] })).toThrow(/document/);
    expect(() => readReferenceEntries({ document: 'tok', entries: {} })).toThrow(/entries/);
  });
});

describe('the built-in References as the provider, through a real host', () => {
  /** The shipped worker, behind the host the app uses, with the queue's
   * `contribute` as the only API it can reach. */
  it('reads a library document handed to it into the entries the engine stores', async () => {
    const source = readFileSync(
      join(__dirname, '../../../extensions/dev.speedyf.references/worker.js'),
      'utf8'
    );
    const stored: { token: string; entries: unknown }[] = [];
    const documents: PendingDocument[] = [
      {
        token: 'r1abc',
        text: 'Body cites [1].\nAcknowledgments. We thank X. References [1] A. Zhong and M. R. DeWeese, Beyond linear response:\nEquivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024). [2] B. Two, arXiv:2401.00003.',
      },
    ];
    const q: ReferenceQueue = new ReferenceQueue({
      setProvider: async () => {},
      pending: async (limit) => documents.splice(0, limit),
      store: async (token, entries) => {
        stored.push({ token, entries });
      },
      invoke: (choice, value, timeoutMs) => host.invoke(choice.command, { value, timeoutMs }),
      busy: () => false,
      wait: (ms) =>
        ms === RECHECK_MS ? new Promise(() => {}) : new Promise((r) => setTimeout(r, 0)),
      log: () => {},
    });
    const host = new ExtensionHost({
      id: REFERENCES.extensionId,
      apiVersion: 1,
      permissions: ['library:contributeReferences'],
      spawn: async () => {
        const worker: WorkerLike & { postMessage(message: unknown): void } = {
          onmessage: null,
          onerror: null,
          postMessage(message: unknown) {
            queueMicrotask(() => inside.onmessage?.({ data: structuredClone(message) }));
          },
          terminate() {},
        };
        const inside = {
          onmessage: undefined as ((e: { data: unknown }) => void) | undefined,
          postMessage(message: unknown) {
            queueMicrotask(() => worker.onmessage?.({ data: structuredClone(message) }));
          },
        };
        new Function('self', source)(inside);
        return worker;
      },
      events: {
        onLog: () => {},
        onCrash: () => {},
        onCall: async (call) => {
          if (call.api !== 'references.contribute') throw new Error(`unexpected ${call.api}`);
          return q.contribute(REFERENCES.extensionId, call.args);
        },
      },
    });
    await q.setProvider(REFERENCES);
    await vi.waitFor(() => expect(stored).toHaveLength(1));
    expect(stored[0]).toEqual({
      token: 'r1abc',
      entries: [
        {
          label: '1',
          text: 'A. Zhong and M. R. DeWeese, Beyond linear response: Equivalence between thermodynamic geometry and optimal transport, Phys. Rev. Lett. 133, 057102 (2024).',
          year: 2024,
        },
        { label: '2', text: 'B. Two, arXiv:2401.00003.' },
      ],
    });
    host.stop();
  });
});

// @vitest-environment happy-dom
/** The permission gate and plumbing for the math extension's surface, driven
 * through the store with a fake worker host: `net.fetch` goes to the core and
 * lights the status line while in flight; layout APIs and the network are
 * refused without their permissions; `ui.showMath` fills the popover; an
 * equation action is invoked with the block and a network-sized timeout. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CallMessage } from './protocol';

interface FakeHostOptions {
  id: string;
  events: { onCall(call: CallMessage): Promise<unknown> };
}
const hosts = new Map<string, { options: FakeHostOptions; invoke: ReturnType<typeof vi.fn> }>();
let invokeBehaviour: (command: string, extra: unknown) => Promise<void> = async () => {};

vi.mock('./host', () => ({
  blobWorkerFactory: () => () => ({}),
  ExtensionHost: class {
    options: FakeHostOptions;
    invoke = vi.fn((command: string, extra: unknown) => invokeBehaviour(command, extra));
    constructor(options: FakeHostOptions) {
      this.options = options;
      hosts.set(options.id, { options, invoke: this.invoke });
    }
    stop() {}
    start() {
      return Promise.resolve();
    }
  },
}));

const pendingFetch: { resolve?: (value: unknown) => void } = {};
const tauriInvoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
  switch (command) {
    case 'list_extensions':
      return dtos;
    case 'extension_net_fetch':
      return new Promise((resolve) => {
        pendingFetch.resolve = resolve;
      });
    case 'get_page_glyphs':
      return {
        groups: [],
        lines: [],
        glyphs: [
          {
            text: 'x',
            code: 120,
            font: 'CMMI10',
            size: 10,
            x: 1,
            y: 2,
            w: 3,
            h: 4,
            baseline: 5,
            bold: false,
            italic: true,
          },
        ],
      };
    default:
      void args;
      return null;
  }
});
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => tauriInvoke(command, args),
}));

const tab = {
  documentStore: {
    state: {
      loaded: true,
      docId: 7,
      path: '/papers/a.pdf',
      name: 'a.pdf',
      pages: [{ srcIndex: 0 }, { srcIndex: 5 }],
    },
  },
  viewport: { state: { currentPage: 0 } },
  zoom: { pagesGeom: () => [] },
};
vi.mock('../../stores/tabsStore', () => ({ activeTab: () => tab }));
vi.mock('../../app/commands', () => ({ COMMANDS: [], runCommand: () => false }));
vi.mock('../../app/shortcuts', () => ({ reservedChords: () => new Set() }));

function dto(id: string, name: string, permissions: string[], extra: object = {}) {
  return {
    id,
    name,
    version: '1',
    source: 'built in',
    permissions,
    enabled: true,
    builtin: true,
    manifest: null,
    commands: [{ id: `${id}/copy`, localId: 'copy', title: 'Copy', keywords: [] }],
    panels: [],
    selectionActions: [],
    citationActions: [],
    documentTasks: [],
    equationActions: [
      { id: `${id}/copyEquation`, localId: 'copyEquation', title: 'Copy LaTeX', extensionId: id },
    ],
    network: permissions.includes('network') ? ['export.arxiv.org'] : [],
    settings: [],
    iconUrl: null,
    error: null,
    ...extra,
  };
}

const dtos = [
  dto('dev.speedyf.mathlatex', 'Math to LaTeX', ['document:layout', 'network']),
  dto('com.example.plain', 'Plain', []),
];

const store = await import('./extensionStore');
const popover = await import('./mathPopoverStore');
const selection = await import('../../stores/selectionStore');

/** The host for an extension, created by running one of its commands. */
async function hostOf(id: string) {
  await store.runExtensionCommand(`${id}/copy`);
  return hosts.get(id)!;
}

function call(api: string, args?: unknown): CallMessage {
  return { type: 'call', requestId: 1, api, args };
}

beforeEach(async () => {
  invokeBehaviour = async () => {};
  await store.loadExtensions();
});

afterEach(() => {
  popover.closeMath();
  selection.clearSelection();
  tauriInvoke.mockClear();
});

describe('net.fetch', () => {
  it('is made by the core and shown as in flight until it settles', async () => {
    const host = await hostOf('dev.speedyf.mathlatex');
    expect(store.extensionsOnline()).toEqual([]);
    const result = host.options.events.onCall(
      call('net.fetch', { url: 'https://export.arxiv.org/abs/1', as: 'text' })
    );
    await Promise.resolve();
    expect(tauriInvoke).toHaveBeenCalledWith('extension_net_fetch', {
      extensionId: 'dev.speedyf.mathlatex',
      url: 'https://export.arxiv.org/abs/1',
      kind: 'text',
      maxBytes: null,
    });
    expect(store.extensionsOnline()).toEqual(['Math to LaTeX']);
    pendingFetch.resolve!({ status: 200, contentType: 'text/plain', body: 'ok' });
    expect(await result).toEqual({ status: 200, contentType: 'text/plain', body: 'ok' });
    expect(store.extensionsOnline()).toEqual([]);
  });

  it('is refused without the network permission, before the core is asked', async () => {
    const host = await hostOf('com.example.plain');
    await expect(
      host.options.events.onCall(call('net.fetch', { url: 'https://export.arxiv.org/' }))
    ).rejects.toThrow('"net.fetch" needs the "network" permission');
    expect(tauriInvoke).not.toHaveBeenCalledWith('extension_net_fetch', expect.anything());
  });
});

describe('layout APIs', () => {
  it('refuses glyphs without document:layout', async () => {
    const host = await hostOf('com.example.plain');
    await expect(host.options.events.onCall(call('selection.glyphs'))).rejects.toThrow(
      '"selection.glyphs" needs the "document:layout" permission'
    );
    await expect(
      host.options.events.onCall(call('region.glyphs', { page: 0, bbox: [0, 0, 10, 10] }))
    ).rejects.toThrow('document:layout');
  });

  it('answers null for selection.glyphs with nothing selected', async () => {
    const host = await hostOf('dev.speedyf.mathlatex');
    expect(await host.options.events.onCall(call('selection.glyphs'))).toBeNull();
  });

  it('reads a region on the page in the file the reader page maps to', async () => {
    const host = await hostOf('dev.speedyf.mathlatex');
    const out = (await host.options.events.onCall(
      call('region.glyphs', { page: 1, bbox: [10, 20, 30, 40] })
    )) as { page: number; glyphs: { font: string }[] };
    expect(tauriInvoke).toHaveBeenCalledWith('get_page_glyphs', {
      docId: 7,
      src: 5,
      rects: [[10, 20, 30, 40]],
      grow: true,
    });
    expect(out.page).toBe(1);
    expect(out.glyphs[0]!.font).toBe('CMMI10');
  });
});

describe('ui.showMath and equation actions', () => {
  it('needs no permission and fills the popover', async () => {
    const host = await hostOf('com.example.plain');
    expect(
      await host.options.events.onCall(
        call('ui.showMath', { latex: 'x', display: false, source: 'fonts', confidence: 'low' })
      )
    ).toEqual({ shown: true });
    expect(popover.mathPopover()).toMatchObject({ phase: 'shown', latex: 'x' });
  });

  it('invokes an equation action with the block and a network timeout', async () => {
    const [action] = store.equationActions();
    expect(store.equationActions()).toHaveLength(2);
    const anchor = { left: 1, top: 2, right: 3, bottom: 4 };
    const target = { page: 1, bbox: [1, 2, 3, 4] as [number, number, number, number], number: '2' };
    let seen: unknown;
    invokeBehaviour = async (command, extra) => {
      seen = { command, extra };
      expect(popover.mathPopover()).toMatchObject({ phase: 'loading', anchor });
    };
    await store.runEquationAction(action!, target, anchor);
    expect(seen).toEqual({
      command: 'copyEquation',
      extra: { value: target, timeoutMs: 60_000 },
    });
    // Finished without showing anything: the loading popover goes away.
    expect(popover.mathPopover()).toBeNull();
  });

  it('shows an equation action failure in the popover', async () => {
    const [action] = store.equationActions();
    invokeBehaviour = async () => {
      throw new Error('This looks like a scan.');
    };
    await store.runEquationAction(
      action!,
      { page: 0, bbox: [0, 0, 1, 1], number: null },
      { left: 0, top: 0, right: 1, bottom: 1 }
    );
    expect(popover.mathPopover()).toMatchObject({
      phase: 'error',
      message: 'This looks like a scan.',
    });
  });

  it('offers no equation actions once the extensions are switched off', async () => {
    tauriInvoke.mockImplementationOnce(async () => dtos.map((d) => ({ ...d, enabled: false })));
    await store.loadExtensions();
    expect(store.equationActions()).toEqual([]);
  });
});

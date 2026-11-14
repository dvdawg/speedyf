// @vitest-environment happy-dom
/** The surfaces and gate behind the built-in Notepad, driven through the
 * store with a fake worker host: which project the reader works in (the
 * one it was opened from, its only one, or the one picked once), what a
 * disabled extension leaves behind (nothing on screen, every file kept),
 * the new APIs' permissions, and the equation converter slot. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CallMessage } from './protocol';

interface FakeHostOptions {
  id: string;
  events: { onCall(call: CallMessage): Promise<unknown> };
}
const hosts = new Map<string, { options: FakeHostOptions; invoke: ReturnType<typeof vi.fn> }>();
let invokeBehaviour: (
  id: string,
  command: string,
  extra: unknown
) => Promise<void> = async () => {};
let paused = 0;

vi.mock('./host', () => ({
  blobWorkerFactory: () => () => ({}),
  ExtensionHost: class {
    options: FakeHostOptions;
    invoke: ReturnType<typeof vi.fn>;
    constructor(options: FakeHostOptions) {
      this.options = options;
      this.invoke = vi.fn((command: string, extra: unknown) =>
        invokeBehaviour(options.id, command, extra)
      );
      hosts.set(options.id, { options, invoke: this.invoke });
    }
    stop() {}
    start() {
      return Promise.resolve();
    }
    pauseDeadlines() {
      paused++;
      return () => {
        paused--;
      };
    }
  },
}));

const tauriCalls: { command: string; args: Record<string, unknown> | undefined }[] = [];
let projects = [
  { id: 'p1', name: 'Thesis', paperCount: 2 },
  { id: 'p2', name: 'Noneq', paperCount: 1 },
];
const tauriInvoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
  tauriCalls.push({ command, args });
  switch (command) {
    case 'list_extensions':
      return dtos;
    case 'library_projects':
      return projects;
    case 'library_papers':
      return {
        total: 1,
        rows: [
          {
            id: 'i1',
            title: 'Quantum things',
            authors: 'Attias',
            year: 2022,
            venue: null,
            citeKey: 'attias2022quantum',
            alphaLabel: 'Att22',
            projectIds: ['p1', 'p2'],
            missing: false,
            hasIdentifier: true,
            incompleteCount: 0,
            isPaper: true,
            candidateCount: 0,
            fileCount: 1,
            firstAuthor: 'Attias',
            keyHistory: ['attias2022'],
          },
        ],
      };
    case 'project_file_read':
      return { text: 'note', base64: null };
    case 'set_extension_enabled':
      dtos[0]!.enabled = args?.enabled as boolean;
      return null;
    default:
      return null;
  }
});
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => tauriInvoke(command, args),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

const tab = {
  documentStore: {
    state: {
      loaded: true,
      docId: 7,
      path: '/papers/a.pdf',
      name: 'a.pdf',
      pages: [{ srcIndex: 0 }, { srcIndex: 1 }],
    },
  },
  viewport: { state: { currentPage: 1 } },
  zoom: { pagesGeom: () => [] },
};
vi.mock('../../stores/tabsStore', () => ({
  activeTab: () => tab,
  findTabByPath: () => null,
  tabsStore: { state: { tabs: [], activeId: null } },
}));
vi.mock('../../app/commands', () => ({ COMMANDS: [], runCommand: () => false }));
vi.mock('../../app/shortcuts', () => ({ reservedChords: () => new Set() }));
vi.mock('../document/tabsController', () => ({
  goHome: vi.fn(),
  openInNewTabOrFocus: vi.fn(async () => true),
}));

const NOTEPAD = 'dev.speedyf.notepad';
const editor = { file: 'notepad.md', command: 'editor' };

function notepadDto() {
  return {
    id: NOTEPAD,
    name: 'Notepad',
    version: '1',
    source: 'built in',
    permissions: ['projects:read', 'projects:files', 'render:region'],
    enabled: true,
    builtin: true,
    manifest: {
      id: NOTEPAD,
      name: 'Notepad',
      version: '1',
      permissions: [],
      contributes: {
        commands: [
          { id: 'open', title: 'Open Notepad' },
          { id: 'jump', title: 'Jump', hidden: true },
        ],
        readerPanes: [
          {
            id: 'notepad',
            title: 'Notepad',
            when: 'documentInProject',
            key: 'mod+shift+n',
            editor,
          },
        ],
        projectViews: [{ id: 'notepad', title: 'Notepad', editor }],
        itemDetailLines: { command: 'mentions' },
      },
    },
    commands: [
      { id: `${NOTEPAD}/open`, localId: 'open', title: 'Open Notepad', keywords: [] },
      {
        id: `${NOTEPAD}/region`,
        localId: 'region',
        title: 'Send Region to Notepad',
        keywords: [],
        when: 'documentInProject',
      },
    ],
    panels: [],
    selectionActions: [
      {
        id: `${NOTEPAD}/quote`,
        localId: 'quote',
        title: 'Quote to Notepad',
        extensionId: NOTEPAD,
        when: 'documentInProject',
      },
    ],
    citationActions: [],
    documentTasks: [],
    equationActions: [
      {
        id: `${NOTEPAD}/equation`,
        localId: 'equation',
        title: 'Send to Notepad',
        extensionId: NOTEPAD,
        when: 'documentInProject',
      },
    ],
    network: [],
    settings: [],
    iconUrl: null,
    error: null,
  };
}

function mathDto() {
  return {
    ...notepadDto(),
    id: 'dev.speedyf.mathlatex',
    name: 'Math to LaTeX',
    permissions: ['document:layout', 'network'],
    manifest: {
      id: 'dev.speedyf.mathlatex',
      name: 'Math to LaTeX',
      version: '1',
      permissions: [],
      contributes: { equationConverter: { command: 'copyEquation' } },
    },
    commands: [],
    selectionActions: [],
    equationActions: [],
  };
}

let dtos = [notepadDto(), mathDto()];

const store = await import('./extensionStore');
const reader = await import('./readerContext');
const surfaces = await import('./surfaces');
const hostApi = await import('./editorHost');
const selection = await import('../../stores/selectionStore');

const call = (api: string, args?: unknown): CallMessage => ({
  type: 'call',
  requestId: 1,
  api,
  args,
});

async function hostOf(id: string) {
  // Run something so the store makes the host.
  await store.runPanelAction(id, 'noop');
  const host = hosts.get(id);
  if (!host) throw new Error(`no host for ${id}`);
  return host;
}

beforeEach(async () => {
  dtos = [notepadDto(), mathDto()];
  projects = [
    { id: 'p1', name: 'Thesis', paperCount: 2 },
    { id: 'p2', name: 'Noneq', paperCount: 1 },
  ];
  invokeBehaviour = async () => {};
  tauriCalls.length = 0;
  await store.loadExtensions();
});

afterEach(() => {
  surfaces.closeReaderPane();
});

describe('the reader project', () => {
  it('is the project the paper was opened from, else its only one, else the one picked', async () => {
    await createRoot(async (dispose) => {
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'attias2022quantum',
        title: 'Quantum things',
        projectIds: ['p1', 'p2'],
      });
      // Opened from no project, in two: nobody has said which yet.
      reader.noteOrigin('/papers/a.pdf', null);
      expect(reader.documentInProject()).toBe(true);
      expect(reader.readerProjectId()).toBeNull();

      // The picker asks once ...
      const asked = reader.askReaderProject();
      await vi.waitFor(() => expect(reader.projectQuestion()).not.toBeNull());
      expect(reader.projectQuestion()!.options.map((o) => o.name)).toEqual(['Thesis', 'Noneq']);
      reader.projectQuestion()!.answer('p2');
      expect(await asked).toBe('p2');
      // ... and the answer is kept for the session.
      expect(reader.readerProjectId()).toBe('p2');
      expect(await reader.askReaderProject()).toBe('p2');

      // Opened from a project, that one wins.
      reader.noteOrigin('/papers/a.pdf', 'p1');
      expect(reader.readerProjectId()).toBe('p1');

      // In one project only: that one.
      reader.noteOrigin('/papers/a.pdf', null);
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i9',
        citeKey: 'k',
        title: '',
        projectIds: ['p1'],
      });
      expect(reader.readerProjectId()).toBe('p1');
      dispose();
    });
  });

  it('offers nothing project-shaped for an unfiled PDF', () => {
    createRoot((dispose) => {
      reader.setReaderItemForTest('/papers/a.pdf', null);
      expect(reader.documentInProject()).toBe(false);
      expect(store.readerPanes()).toEqual([]);
      expect(store.selectionActions()).toEqual([]);
      expect(store.equationActions()).toEqual([]);
      expect(store.extensionCommands().map((c) => c.title)).toEqual(['Open Notepad']);
      expect(surfaces.toggleReaderPane()).toBe(false);
      expect(surfaces.shownPane()).toBeNull();
      dispose();
    });
  });

  it('offers the drawer, the actions and the region command for an item in a project', () => {
    createRoot((dispose) => {
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'k',
        title: '',
        projectIds: ['p1'],
      });
      expect(store.readerPanes().map((p) => p.id)).toEqual([`${NOTEPAD}/notepad`]);
      expect(store.selectionActions().map((a) => a.title)).toEqual(['Quote to Notepad']);
      expect(store.equationActions().map((a) => a.title)).toEqual(['Send to Notepad']);
      expect(surfaces.toggleReaderPane()).toBe(true);
      expect(surfaces.shownPane()?.title).toBe('Notepad');
      // Moving to an unfiled PDF hides it; coming back shows it again.
      reader.setReaderItemForTest('/papers/a.pdf', null);
      expect(surfaces.shownPane()).toBeNull();
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'k',
        title: '',
        projectIds: ['p1'],
      });
      expect(surfaces.shownPane()?.title).toBe('Notepad');
      dispose();
    });
  });
});

describe('a disabled Notepad', () => {
  it('takes the drawer, the view, the actions and the item lines away, and leaves the files', async () => {
    await createRoot(async (dispose) => {
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'k',
        title: '',
        projectIds: ['p1'],
      });
      surfaces.toggleReaderPane();
      hostApi.setItemLine(
        'i1',
        { extensionId: NOTEPAD, text: 'Mentioned in notepad (1)', command: 'jump', value: null },
        NOTEPAD
      );
      expect(hostApi.itemLines('i1', store.extensionUsable)).toHaveLength(1);

      await store.setExtensionEnabled(NOTEPAD, false);
      expect(store.readerPanes()).toEqual([]);
      expect(store.projectViews()).toEqual([]);
      expect(store.selectionActions()).toEqual([]);
      expect(store.equationActions()).toEqual([]);
      expect(surfaces.shownPane()).toBeNull();
      expect(hostApi.itemLines('i1', store.extensionUsable)).toEqual([]);
      // Nothing was removed: the files stay for when it comes back.
      expect(tauriCalls.map((c) => c.command)).not.toContain('project_file_remove');
      dispose();
    });
  });
});

describe('the gate', () => {
  it('refuses project APIs without their permissions', async () => {
    const plain = await hostOf('dev.speedyf.mathlatex');
    for (const api of [
      'projects.items',
      'reader.context',
      'navigate.openItem',
      'projectFiles.read',
      'region.capture',
      'equations.snapshot',
      'editor.insert',
      'editor.replace',
      'editor.resolve',
      'editor.mark',
      'editor.replaceMarked',
    ]) {
      await expect(plain.options.events.onCall(call(api, {}))).rejects.toThrow(/permission/);
    }
  });

  it('answers project reads with no paths, and the reader context', async () => {
    const host = await hostOf(NOTEPAD);
    reader.setReaderItemForTest('/papers/a.pdf', {
      id: 'i1',
      citeKey: 'attias2022quantum',
      title: 'Quantum things',
      projectIds: ['p1'],
    });
    const items = (await host.options.events.onCall(call('projects.items'))) as object[];
    expect(items).toEqual([
      {
        id: 'i1',
        citeKey: 'attias2022quantum',
        keyHistory: ['attias2022'],
        title: 'Quantum things',
        authors: 'Attias',
        firstAuthor: 'Attias',
        alphaLabel: 'Att22',
        year: 2022,
        projectIds: ['p1', 'p2'],
      },
    ]);
    expect(await host.options.events.onCall(call('reader.context'))).toMatchObject({
      item: { id: 'i1', citeKey: 'attias2022quantum' },
      projectIds: ['p1'],
      projectId: 'p1',
      page: 1,
    });
  });

  it('checks project file paths before the core sees them', async () => {
    const host = await hostOf(NOTEPAD);
    for (const path of ['../x.md', '/etc/passwd', 'a//b', '.hidden', 'a/b/c/d/e']) {
      await expect(
        host.options.events.onCall(call('projectFiles.read', { projectId: 'p1', path }))
      ).rejects.toThrow(/path/);
    }
    expect(
      await host.options.events.onCall(
        call('projectFiles.read', { projectId: 'p1', path: 'notepad.md' })
      )
    ).toBe('note');
    expect(tauriCalls.find((c) => c.command === 'project_file_read')?.args).toEqual({
      extensionId: NOTEPAD,
      projectId: 'p1',
      path: 'notepad.md',
      kind: 'text',
    });
  });

  it("lets an item line act only through the extension's own commands", async () => {
    const host = await hostOf(NOTEPAD);
    await expect(
      host.options.events.onCall(
        call('ui.itemLine', { itemId: 'i1', text: 'x', action: { command: 'nope' } })
      )
    ).rejects.toThrow(/this extension's commands/);
    expect(
      await host.options.events.onCall(
        call('ui.itemLine', {
          itemId: 'i1',
          text: 'Mentioned in notepad (2)',
          action: { command: 'jump', value: 1 },
        })
      )
    ).toEqual({ shown: true });
  });
});

describe('a send from the reader', () => {
  const KEY = { extensionId: NOTEPAD, projectId: 'p1', file: 'notepad.md' };

  /** A core holding the note, for the host's sessions. */
  function noteCore(text: string) {
    const state = { text, rev: 1 };
    const sessions = hostApi.useNoteSessionsForTest(
      {
        open: async () => ({ text: state.text, rev: state.rev }),
        edit: async (_key, _base, op, origin) => {
          state.text = state.text.slice(0, op.from) + op.insert + state.text.slice(op.to);
          const rev = ++state.rev;
          queueMicrotask(() =>
            sessions.dispatch({ kind: 'edit', key: KEY, rev, op, origin, len: state.text.length })
          );
          return rev;
        },
        close: async () => {},
      },
      'test-window',
      () => {}
    );
    return state;
  }

  afterEach(() => hostApi.clearDecorationsOf(NOTEPAD));

  it("opens the reader's drawer on the note when no view of it is showing", async () => {
    await createRoot(async (dispose) => {
      noteCore('');
      const host = await hostOf(NOTEPAD);
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'attias2022quantum',
        title: 'Quantum things',
        projectIds: ['p1', 'p2'],
      });
      reader.noteOrigin('/papers/a.pdf', null);
      expect(surfaces.shownPane()).toBeNull();
      const placed = (await host.options.events.onCall(
        call('editor.insert', { projectId: 'p1', pending: { kind: 'equation' } })
      )) as { handle: string; offset: number };
      expect(placed.handle).toMatch(/^test-window:/);
      // The drawer is open, on the note the send went to.
      expect(surfaces.shownPane()?.id).toBe(`${NOTEPAD}/notepad`);
      expect(reader.readerProjectId()).toBe('p1');
      // Filled in, the text is in the note.
      expect(
        await host.options.events.onCall(
          call('editor.resolve', { handle: placed.handle, text: '$$ a $$' })
        )
      ).toEqual({ resolved: true, offset: 0 });
      dispose();
    });
  });

  it('leaves the drawer shut when the Projects notepad shows the note, and reveals there', async () => {
    await createRoot(async (dispose) => {
      noteCore('intro\n');
      const host = await hostOf(NOTEPAD);
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'attias2022quantum',
        title: 'Quantum things',
        projectIds: ['p1'],
      });
      const revealed: [number, string | undefined][] = [];
      const unregister = hostApi.registerEditor(KEY, {
        insertAtCaret: () => null,
        caretBlockEnd: () => null,
        reveal: (offset, decor) => revealed.push([offset, decor]),
      });
      const placed = (await host.options.events.onCall(
        call('editor.insert', { projectId: 'p1', pending: { kind: 'text' } })
      )) as { handle: string };
      expect(surfaces.shownPane()).toBeNull();
      await host.options.events.onCall(call('editor.reveal', { handle: placed.handle }));
      expect(revealed).toEqual([[6, placed.handle]]);
      unregister();
      dispose();
    });
  });

  it("names the converter's source in the status line it was given", async () => {
    noteCore('');
    const notepad = await hostOf(NOTEPAD);
    const math = await hostOf('dev.speedyf.mathlatex');
    const sent = (await notepad.options.events.onCall(
      call('editor.insert', { projectId: 'p1', text: '$$ a $$' })
    )) as { handle: string };
    await notepad.options.events.onCall(
      call('editor.mark', { handle: sent.handle, label: 'checking source' })
    );
    const token = hostApi.equationToken({ docId: 7, page: 1, bbox: [1, 2, 3, 4], number: null });
    const session = hostApi.noteSessions().get(KEY)!;
    let label: unknown = null;
    invokeBehaviour = async (id, _command, extra) => {
      if (id !== 'dev.speedyf.mathlatex') return;
      const value = (extra as { value: Record<string, unknown> }).value;
      await math.options.events.onCall(
        call('equations.checking', { ticket: value.ticket, source: 'arxiv' })
      );
      const d = session.decoration(sent.handle);
      label = d?.type === 'mark' ? d.label : null;
    };
    await notepad.options.events.onCall(
      call('equations.convert', { equation: token, mode: 'exact', status: sent.handle })
    );
    expect(label).toBe('checking arXiv source');
    // A ticket nobody waits on names nothing.
    expect(
      await math.options.events.onCall(
        call('equations.checking', { ticket: 't999', source: 'journal' })
      )
    ).toEqual({ shown: false });
  });
});

describe('the equation converter slot', () => {
  it('asks the enabled converter and takes its formula as the answer', async () => {
    const notepad = await hostOf(NOTEPAD);
    const math = await hostOf('dev.speedyf.mathlatex');
    const token = hostApi.equationToken({ docId: 7, page: 1, bbox: [1, 2, 3, 4], number: '3' });
    invokeBehaviour = async (id, command, extra) => {
      if (id !== 'dev.speedyf.mathlatex') return;
      expect(command).toBe('copyEquation');
      const value = (extra as { value: Record<string, unknown> }).value;
      expect(value).toEqual({
        page: 1,
        bbox: [1, 2, 3, 4],
        number: '3',
        equation: token,
        ticket: expect.stringMatching(/^t\d+$/),
      });
      // It answers as it would an equation action.
      await math.options.events.onCall(
        call('ui.showMath', { latex: 'a^2', display: true, source: 'fonts', confidence: 'high' })
      );
    };
    expect(
      await notepad.options.events.onCall(call('equations.convert', { equation: token }))
    ).toEqual({ latex: 'a^2', display: true, source: 'fonts', confidence: 'high' });
    // The consumer was not timed out while the converter worked.
    expect(paused).toBe(0);
  });

  it('passes the mode through, and keeps two conversions at once apart by ticket', async () => {
    const notepad = await hostOf(NOTEPAD);
    const math = await hostOf('dev.speedyf.mathlatex');
    const token = hostApi.equationToken({ docId: 7, page: 1, bbox: [1, 2, 3, 4], number: null });
    const seen: Record<string, unknown>[] = [];
    let releaseExact: () => void = () => {};
    const exactGate = new Promise<void>((resolve) => (releaseExact = resolve));
    invokeBehaviour = async (id, _command, extra) => {
      if (id !== 'dev.speedyf.mathlatex') return;
      const value = (extra as { value: Record<string, unknown> }).value;
      seen.push(value);
      const answer = (latex: string, source: string) =>
        math.options.events.onCall(
          call('ui.showMath', {
            latex,
            display: true,
            source,
            confidence: 'high',
            ticket: value.ticket,
          })
        );
      if (value.mode === 'exact') {
        await exactGate;
        await answer('exact', 'arxiv');
      } else {
        await answer('local', 'fonts');
      }
    };
    const exact = notepad.options.events.onCall(
      call('equations.convert', { equation: token, mode: 'exact' })
    );
    const local = await notepad.options.events.onCall(
      call('equations.convert', { equation: token, mode: 'local' })
    );
    expect(local).toMatchObject({ latex: 'local', source: 'fonts' });
    releaseExact();
    expect(await exact).toMatchObject({ latex: 'exact', source: 'arxiv' });
    expect(seen.map((v) => v.mode)).toEqual(['exact', 'local']);
    expect(seen[0]!.ticket).not.toBe(seen[1]!.ticket);
    await expect(
      notepad.options.events.onCall(call('equations.convert', { equation: token, mode: 'fast' }))
    ).rejects.toThrow(/mode/);
  });

  it('asks the converter to prefetch once per document, and nobody else', async () => {
    const math = await hostOf('dev.speedyf.mathlatex');
    const notepad = await hostOf(NOTEPAD);
    math.invoke.mockClear();
    notepad.invoke.mockClear();
    store.prefetchEquationSource(41);
    store.prefetchEquationSource(41);
    store.prefetchEquationSource(42);
    expect(math.invoke.mock.calls).toEqual([
      ['copyEquation', expect.objectContaining({ value: { mode: 'prefetch' } })],
      ['copyEquation', expect.objectContaining({ value: { mode: 'prefetch' } })],
    ]);
    expect(notepad.invoke).not.toHaveBeenCalled();
    // A disabled converter (its hosts not consented to) is never asked.
    dtos = [notepadDto(), { ...mathDto(), enabled: false }];
    await store.loadExtensions();
    math.invoke.mockClear();
    store.prefetchEquationSource(43);
    expect(math.invoke).not.toHaveBeenCalled();
  });

  it('answers null with no converter, and refuses a token nobody issued', async () => {
    const notepad = await hostOf(NOTEPAD);
    dtos = [notepadDto()];
    await store.loadExtensions();
    const token = hostApi.equationToken({ docId: 7, page: 0, bbox: [1, 2, 3, 4], number: null });
    expect(
      await notepad.options.events.onCall(call('equations.convert', { equation: token }))
    ).toBeNull();
    await expect(
      notepad.options.events.onCall(call('equations.convert', { equation: 'eq-forged' }))
    ).rejects.toThrow(/token/);
  });
});

describe('the Quote to Notepad key', () => {
  /** A page's text, selected as a drag would leave it, and its capture. */
  function selectOnPage(text: string) {
    const span = document.createElement('span');
    span.textContent = text;
    document.body.append(span);
    document.getSelection()!.setBaseAndExtent(span.firstChild!, 0, span.firstChild!, text.length);
    // The viewer hands the store its capture; the key takes it before the
    // settle time is up.
    selection.noteSelectionChanged(() => text);
    return span;
  }

  it('runs the quote with the current selection, and does nothing without one', async () => {
    const manifest = JSON.parse(
      readFileSync(join(__dirname, '../../../extensions/dev.speedyf.notepad/speedyf.json'), 'utf8')
    ) as { contributes: { keybindings: { command: string; key: string }[] } };
    expect(manifest.contributes.keybindings).toEqual([{ command: 'quote', key: 'mod+shift+e' }]);
    dtos = [
      {
        ...notepadDto(),
        manifest: {
          ...notepadDto().manifest,
          contributes: { ...notepadDto().manifest.contributes, ...manifest.contributes },
        },
      },
    ];
    await store.loadExtensions();
    await createRoot(async (dispose) => {
      reader.setReaderItemForTest('/papers/a.pdf', {
        id: 'i1',
        citeKey: 'k',
        title: '',
        projectIds: ['p1'],
      });
      // Bound (that no built-in takes it is checked with the real keymap
      // in navCommands.test.ts).
      expect(store.extensionBindings()).toEqual([
        {
          extensionId: NOTEPAD,
          command: 'quote',
          chord: { key: 'e', mod: true, shift: true, alt: false },
        },
      ]);
      const notepad = await hostOf(NOTEPAD);
      notepad.invoke.mockClear();

      // Nothing selected: nothing happens.
      selection.clearSelection();
      document.getSelection()!.removeAllRanges();
      await store.runBoundCommand(NOTEPAD, 'quote');
      expect(notepad.invoke).not.toHaveBeenCalled();

      const span = selectOnPage('a quoted passage');
      await store.runBoundCommand(NOTEPAD, 'quote');
      expect(notepad.invoke).toHaveBeenCalledTimes(1);
      expect(notepad.invoke.mock.calls[0]![0]).toBe('quote');
      expect(notepad.invoke.mock.calls[0]![1]).toMatchObject({ value: 'a quoted passage' });

      // Clicked away: the old capture is not quoted again.
      notepad.invoke.mockClear();
      document.getSelection()!.removeAllRanges();
      await store.runBoundCommand(NOTEPAD, 'quote');
      expect(notepad.invoke).not.toHaveBeenCalled();

      // A paper in no project: the key does nothing.
      selectOnPage('another passage');
      reader.setReaderItemForTest('/papers/a.pdf', null);
      await store.runBoundCommand(NOTEPAD, 'quote');
      expect(notepad.invoke).not.toHaveBeenCalled();
      span.remove();
      dispose();
    });
  });
});

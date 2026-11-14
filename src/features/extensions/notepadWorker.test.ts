/** The built-in Notepad: what it writes into a note and how it reads one
 * back, driven the way the host drives it.
 *
 * The pure functions are reached the way the Notation tests reach theirs,
 * by returning them from the worker's own source; the commands are driven
 * through a fake `self` answering its `call`s with canned results. */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { inlineSpans, splitBlocks } from './editor/markdown';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.notepad/worker.js'),
  'utf8'
);

interface Item {
  id: string;
  citeKey: string;
  keyHistory: string[];
  title: string;
  authors: string;
  firstAuthor: string | null;
  year: number | null;
  projectIds: string[];
  alphaLabel: string;
}

interface Notepad {
  escapeQuote(text: string): string;
  quoteLine(text: string, key: string, page: number, box: number[] | null): string;
  pageLink(key: string, page: number | null, box: number[] | null): string;
  baseLabel(item: Item): string;
  alphaLabels(items: Item[]): Map<string, string>;
  resolveKey(items: Item[], key: string): Item | null;
  citationsIn(text: string): { key: string; offset: number }[];
  mentions(text: string, item: Item): { count: number; first: number | null };
  displayMath(latex: string): string;
}

const notepad = new Function(
  'self',
  `${SOURCE}\nreturn { escapeQuote, quoteLine, pageLink, baseLabel, alphaLabels, resolveKey, citationsIn, mentions, displayMath };`
)({}) as Notepad;

const ATTIAS: Item = {
  id: 'i1',
  citeKey: 'attias2022quantum',
  keyHistory: ['attias2022'],
  title: 'Quantum things',
  authors: 'Attias, Smith',
  firstAuthor: 'Attias',
  alphaLabel: 'AS22',
  year: 2022,
  projectIds: ['p1', 'p2'],
};

describe('notepad: quotes', () => {
  it('escapes Markdown and citations in quoted text and keeps it one quote block', () => {
    const selected =
      '# A *bold* claim_\nwith [refs] and $x$\n\n> nested, `code`, <tag> @someone {x} 50|50 ~';
    const line = notepad.quoteLine(selected, 'attias2022quantum', 11, [72.4, 310.6, 240, 28]);
    expect(line).toBe(
      '> "\\# A \\*bold\\* claim\\_ with \\[refs\\] and \\$x\\$ \\> nested, \\`code\\`, \\<tag\\> \\@someone \\{x\\} 50\\|50 \\~" [@attias2022quantum, p. 12]{box=72,311,240,28}'
    );
    // One line, so one quote block, whatever the selection held.
    expect(line.includes('\n')).toBe(false);
    expect(splitBlocks(line)).toHaveLength(1);
    // Read back, the quote is the text as selected (whitespace collapsed),
    // and the only citation is the pinpoint.
    const spans = inlineSpans(line.slice(2));
    const cites = spans.filter((s) => s.kind === 'cite');
    expect(cites).toHaveLength(1);
    const text = spans
      .filter((s) => s.kind === 'text')
      .map((s) => (s.kind === 'text' ? s.text : ''))
      .join('');
    expect(text).toContain(
      '"# A *bold* claim_ with [refs] and $x$ > nested, `code`, <tag> @someone {x} 50|50 ~"'
    );
  });

  it('keeps non-ASCII user text as it is', () => {
    expect(notepad.escapeQuote('Schrödinger — α → β')).toBe('Schrödinger — α → β');
  });

  it('writes pinpoint links that the editor reads back', () => {
    const link = notepad.pageLink('k', 0, [1, 2, 3, 4]);
    expect(link).toBe('[@k, p. 1]{box=1,2,3,4}');
    const [span] = inlineSpans(link);
    expect(span).toMatchObject({ kind: 'cite', cite: { key: 'k', page: 0, box: [1, 2, 3, 4] } });
    expect(notepad.pageLink('k', null, null)).toBe('[@k]');
    expect(notepad.displayMath('a\n  = b')).toBe('$$ a = b $$');
  });
});

describe('notepad: cite keys', () => {
  it('labels an item as the graph does, alpha.bst style', () => {
    expect(notepad.baseLabel(ATTIAS)).toBe('AS22');
    // No alpha label: first author and two-digit year, then the key.
    expect(notepad.baseLabel({ ...ATTIAS, alphaLabel: '' })).toBe('Attias22');
    expect(
      notepad.baseLabel({ ...ATTIAS, alphaLabel: '', firstAuthor: null, authors: '', year: null })
    ).toBe('attias2022quantum');
  });

  it('suffixes repeated labels by year, then title, as alpha.bst does', () => {
    const labels = notepad.alphaLabels([
      { ...ATTIAS, id: 'b', title: 'Second' },
      { ...ATTIAS, id: 'a', title: 'First' },
      { ...ATTIAS, id: 'old', year: 2021, alphaLabel: 'AS22' },
      { ...ATTIAS, id: 'solo', alphaLabel: 'Jar97', year: 1997 },
    ]);
    expect(labels.get('old')).toBe('[AS22a]');
    expect(labels.get('a')).toBe('[AS22b]');
    expect(labels.get('b')).toBe('[AS22c]');
    expect(labels.get('solo')).toBe('[Jar97]');
  });

  it('resolves a key the item no longer has through its history', () => {
    const other: Item = { ...ATTIAS, id: 'i2', citeKey: 'attias2022', keyHistory: [] };
    // The current holder of a key wins over a history entry.
    expect(notepad.resolveKey([ATTIAS, other], 'attias2022')?.id).toBe('i2');
    expect(notepad.resolveKey([ATTIAS], 'attias2022')?.id).toBe('i1');
    expect(notepad.resolveKey([ATTIAS], 'nobody1999')).toBeNull();
  });

  it('counts mentions by any key the item had, and finds the first', () => {
    const text = [
      '# Notes',
      'See @attias2022 and [@attias2022quantum, p. 3]{box=1,2,3,4}.',
      'Not \\@attias2022quantum, nor email@attias2022quantum.',
      '> "quote" [@attias2022quantum]',
    ].join('\n');
    const found = notepad.mentions(text, ATTIAS);
    expect(found.count).toBe(3);
    expect(found.first).toBe(text.indexOf('@attias2022'));
  });
});

/* ---- driving the commands ---- */

type Api = (args: Record<string, unknown>) => unknown;

function boot(apis: Record<string, Api>) {
  const calls: { api: string; args: Record<string, unknown> }[] = [];
  const logs: string[] = [];
  const fake: { onmessage?: (e: { data: unknown }) => void; postMessage(m: unknown): void } = {
    postMessage(message: unknown) {
      const m = message as Record<string, unknown>;
      if (m.type === 'call') {
        const api = String(m.api);
        const args = (m.args ?? {}) as Record<string, unknown>;
        calls.push({ api, args });
        const handler = apis[api];
        Promise.resolve()
          .then(() => {
            if (!handler) throw new Error(`no ${api}`);
            return handler(args);
          })
          .then(
            (value) =>
              fake.onmessage!({ data: { type: 'reply', requestId: m.requestId, ok: true, value } }),
            (err: Error) =>
              fake.onmessage!({
                data: { type: 'reply', requestId: m.requestId, ok: false, error: err.message },
              })
          );
      } else if (m.type === 'done') {
        done.get(m.requestId as number)?.(m as { ok: boolean; error?: string });
      } else if (m.type === 'log') {
        logs.push(String(m.message));
      }
    },
  };
  const done = new Map<number, (m: { ok: boolean; error?: string }) => void>();
  new Function('self', SOURCE)(fake);
  let next = 1;
  const invoke = (command: string, value?: unknown) =>
    new Promise<{ ok: boolean; error?: string }>((resolve) => {
      const requestId = next++;
      done.set(requestId, resolve);
      fake.onmessage!({ data: { type: 'invoke', requestId, command, value } });
    });
  return { calls, logs, invoke };
}

const readerIn = (projectId: string | null) => () => ({
  item: { id: 'i1', citeKey: 'attias2022quantum', title: 'Quantum things' },
  projectIds: ['p1', 'p2'],
  projectId,
  page: 3,
  selection: { page: 4, box: [10, 20, 30, 40], text: 'x' },
});

describe('notepad: reader actions', () => {
  it('quotes into the project the paper was opened from', async () => {
    const { calls, invoke } = boot({
      'reader.context': readerIn('p2'),
      'editor.insert': () => ({ offset: 10 }),
      'editor.reveal': () => ({ revealed: true }),
    });
    expect(await invoke('quote', 'Some text')).toEqual({ type: 'done', requestId: 1, ok: true });
    const insert = calls.find((c) => c.api === 'editor.insert')!;
    expect(insert.args).toEqual({
      projectId: 'p2',
      text: '> "Some text" [@attias2022quantum, p. 5]{box=10,20,30,40}',
      at: 'end',
    });
    expect(calls.map((c) => c.api)).not.toContain('reader.chooseProject');
  });

  it('asks which project for a paper in two opened from neither, and stops when dismissed', async () => {
    const answers = ['p1', null];
    const { calls, invoke } = boot({
      'reader.context': readerIn(null),
      'reader.chooseProject': () => answers.shift(),
      'editor.insert': () => ({ offset: 0 }),
      'editor.reveal': () => ({}),
    });
    await invoke('quote', 'first');
    expect(calls.filter((c) => c.api === 'reader.chooseProject')).toHaveLength(1);
    expect(calls.find((c) => c.api === 'editor.insert')!.args.projectId).toBe('p1');
    await invoke('quote', 'second');
    expect(calls.filter((c) => c.api === 'editor.insert')).toHaveLength(1);
  });

  it('sends an equation as LaTeX when a converter reads it, else as a picture', async () => {
    let converted: unknown = { latex: 'E = mc^2', display: true };
    const { calls, invoke } = boot({
      'reader.context': readerIn('p1'),
      'equations.convert': () => converted,
      'equations.snapshot': () => ({ page: 2, box: [1, 2, 3, 4], png: [137, 80, 78, 71] }),
      'projectFiles.write': () => ({ written: 4 }),
      'editor.insert': () => ({ offset: 0 }),
      'editor.reveal': () => ({}),
    });
    const value = { page: 2, bbox: [100, 200, 300, 40], number: '3', equation: 'eq-1' };
    await invoke('equation', value);
    const first = calls.filter((c) => c.api === 'editor.insert')[0]!;
    expect(first.args.text).toBe(
      '$$ E = mc^2 $$\n\nEquation (3), [@attias2022quantum, p. 3]{box=100,200,300,40}'
    );
    expect(calls.map((c) => c.api)).not.toContain('equations.snapshot');

    converted = null;
    await invoke('equation', value);
    const write = calls.find((c) => c.api === 'projectFiles.write')!;
    expect(write.args.path).toMatch(/^assets\/equation-\d{8}-\d{6}-[a-z0-9]+\.png$/);
    expect(write.args.bytes).toEqual([137, 80, 78, 71]);
    const second = calls.filter((c) => c.api === 'editor.insert')[1]!;
    expect(second.args.text).toBe(
      `![Equation (3)](${String(write.args.path)})\n\nEquation (3), [@attias2022quantum, p. 3]{box=100,200,300,40}`
    );
  });

  it('saves a captured region and does nothing when it is cancelled', async () => {
    let region: unknown = null;
    const { calls, invoke } = boot({
      'reader.context': readerIn('p1'),
      'region.capture': () => region,
      'projectFiles.write': () => ({ written: 3 }),
      'editor.insert': () => ({ offset: 0 }),
      'editor.reveal': () => ({}),
    });
    await invoke('region');
    expect(calls.map((c) => c.api)).not.toContain('editor.insert');
    region = { page: 0, box: [1, 2, 3, 4], png: [1, 2, 3] };
    await invoke('region');
    const insert = calls.find((c) => c.api === 'editor.insert')!;
    expect(String(insert.args.text)).toMatch(
      /^!\[Region, p\. 1\]\(assets\/region-.*\.png\)\n\n\[@attias2022quantum, p\. 1\]\{box=1,2,3,4\}$/
    );
  });
});

describe('notepad: editor and backlinks', () => {
  const items = [
    ATTIAS,
    { ...ATTIAS, id: 'i3', citeKey: 'other2020', keyHistory: [], projectIds: ['p9'] },
  ];

  it('tells the editor every key, offering the project items in the picker', async () => {
    const { calls, invoke } = boot({
      'projects.items': () => items,
      'editor.citations': () => ({ stored: 2 }),
    });
    await invoke('editor', { event: 'citations', projectId: 'p1', file: 'notepad.md' });
    const entries = calls.find((c) => c.api === 'editor.citations')!.args.entries as {
      key: string;
      pick: boolean;
      aliases: string[];
      label: string;
    }[];
    expect(entries.map((e) => [e.key, e.pick, e.label])).toEqual([
      ['attias2022quantum', true, '[AS22]'],
      ['other2020', false, '[AS22]'],
    ]);
    expect(entries[0]!.aliases).toEqual(['attias2022']);
  });

  it('opens a renamed key at its page and place, and leaves an unknown key alone', async () => {
    const { calls, invoke, logs } = boot({
      'projects.items': () => items,
      'navigate.openItem': () => ({ opened: true }),
    });
    await invoke('editor', {
      event: 'activate',
      projectId: 'p1',
      file: 'notepad.md',
      cite: { key: 'attias2022', locator: 'p. 4', page: 3, box: [1, 2, 3, 4] },
    });
    expect(calls.find((c) => c.api === 'navigate.openItem')!.args).toEqual({
      itemId: 'i1',
      projectId: 'p1',
      page: 3,
      box: [1, 2, 3, 4],
    });
    await invoke('editor', {
      event: 'activate',
      projectId: 'p1',
      file: 'notepad.md',
      cite: { key: 'gone1999', locator: null, page: null, box: null },
    });
    expect(calls.filter((c) => c.api === 'navigate.openItem')).toHaveLength(1);
    expect(logs.at(-1)).toBe('no item has the key gone1999');
  });

  it('puts "Mentioned in notepad (n)" on an item, jumping to the first mention', async () => {
    const notes: Record<string, string | null> = {
      p1: 'Nothing here.',
      p2: 'Intro.\nSee @attias2022quantum and [@attias2022, p. 2].',
    };
    const { calls, invoke } = boot({
      'projectFiles.read': (args) => notes[String(args.projectId)] ?? null,
      'ui.itemLine': () => ({ shown: true }),
    });
    await invoke('mentions', { item: ATTIAS, projectId: null });
    expect(calls.find((c) => c.api === 'ui.itemLine')!.args).toEqual({
      itemId: 'i1',
      text: 'Mentioned in notepad (2)',
      action: { command: 'jump', value: { projectId: 'p2', offset: notes.p2!.indexOf('@') } },
    });
    // Under one project, only that project's notepad counts.
    await invoke('mentions', { item: ATTIAS, projectId: 'p1' });
    expect(calls.filter((c) => c.api === 'ui.itemLine')[1]!.args).toEqual({
      itemId: 'i1',
      text: null,
      action: null,
    });
  });
});

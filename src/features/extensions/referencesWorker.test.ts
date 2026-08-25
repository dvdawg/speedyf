/** Drives the shipped References worker the way the host does.
 *
 * The worker is plain JavaScript with a `postMessage` protocol, so it can be
 * loaded into a fake `self` and answered with canned API results. That makes
 * the document-switch path testable end to end without a webview: this is the
 * one piece the component tests cannot reach, and the one where switching
 * documents actually goes wrong.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it } from 'vitest';

const SOURCE = readFileSync(
  join(__dirname, '../../../extensions/dev.speedyf.references/worker.js'),
  'utf8'
);

interface Doc {
  name: string;
  pageCount: number;
  citations: { kind: string; value: string }[];
}

/** A running worker with the host on the other end of the pipe. */
function startWorker(docs: Record<string, Doc>) {
  const storage = new Map<string, string>();
  const drawn: unknown[] = [];
  let open: string | null = null;

  const fake = {
    onmessage: undefined as ((e: { data: unknown }) => void) | undefined,
    postMessage(message: Record<string, unknown>) {
      if (message.type !== 'call') return;
      void answer(message);
    },
  };

  async function answer(message: Record<string, unknown>) {
    const args = (message.args ?? {}) as Record<string, unknown>;
    const doc = open ? docs[open] : undefined;
    let value: unknown = null;
    let ok = true;
    switch (message.api) {
      case 'document.info':
        if (!doc) ok = false;
        else value = { name: doc.name, pageCount: doc.pageCount, currentPage: 0 };
        break;
      case 'document.links':
        value = [];
        break;
      case 'citations.list':
        value = args.page === 0 && doc ? doc.citations : [];
        break;
      case 'citations.resolve':
        // The shape the host always answers with, found or not.
        value = { inLibrary: false, fileName: null, title: null, page: null };
        break;
      case 'storage.get':
        value = storage.get(String(args.key)) ?? null;
        break;
      case 'storage.set':
        storage.set(String(args.key), String(args.value));
        value = null;
        break;
      case 'ui.render':
        drawn.push(args.ui);
        value = null;
        break;
      default:
        value = null;
    }
    fake.onmessage?.({
      data: { type: 'reply', requestId: message.requestId, ok, value, error: 'no document' },
    });
  }

  new Function('self', SOURCE)(fake);

  return {
    drawn,
    openDocument(key: string | null) {
      open = key;
    },
    /** One `invoke`, resolved when the worker reports the command finished. */
    async invoke(command: string) {
      return new Promise<void>((resolve, reject) => {
        const requestId = 10_000 + drawn.length;
        const post = fake.postMessage.bind(fake);
        fake.postMessage = (message: Record<string, unknown>) => {
          if (message.type === 'done' && message.requestId === requestId) {
            fake.postMessage = post;
            if (message.ok) resolve();
            else reject(new Error(String(message.error)));
            return;
          }
          post(message);
        };
        fake.onmessage?.({ data: { type: 'invoke', command, requestId } });
      });
    },
  };
}

const DOCS: Record<string, Doc> = {
  a: { name: 'paper-a.pdf', pageCount: 1, citations: [{ kind: 'arxiv', value: '2101.00001' }] },
  b: { name: 'paper-b.pdf', pageCount: 1, citations: [{ kind: 'arxiv', value: '2202.00002' }] },
};

function titlesIn(ui: unknown): string[] {
  const description = ui as { sections?: { rows?: { title: string }[] }[] };
  return (description.sections ?? []).flatMap((s) => (s.rows ?? []).map((r) => r.title));
}

describe('References worker across documents', () => {
  let worker: ReturnType<typeof startWorker>;

  beforeEach(() => {
    worker = startWorker(DOCS);
  });

  it('lists the open document’s references', async () => {
    worker.openDocument('a');
    await worker.invoke('refs');
    expect(titlesIn(worker.drawn.at(-1)).join(' ')).toContain('2101.00001');
  });

  it('lists the new document’s references after a switch', async () => {
    worker.openDocument('a');
    await worker.invoke('refs');
    worker.openDocument('b');
    await worker.invoke('refs');

    const last = titlesIn(worker.drawn.at(-1)).join(' ');
    expect(last).toContain('2202.00002');
    expect(last).not.toContain('2101.00001');
  });

  it('says so when no document is open', async () => {
    worker.openDocument(null);
    await worker.invoke('refs');
    expect(JSON.stringify(worker.drawn.at(-1))).toContain('Open a paper');
  });

  /** The panel refreshes twice in quick succession when a tab is opened: once
   * while the tab is empty, once when its document arrives. The second must
   * not be dropped. */
  it('does not drop a second render issued while the first is still running', async () => {
    worker.openDocument(null);
    const first = worker.invoke('refs');
    worker.openDocument('b');
    const second = worker.invoke('refs');
    await Promise.all([first, second]);

    expect(titlesIn(worker.drawn.at(-1)).join(' ')).toContain('2202.00002');
  });
});

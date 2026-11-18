/** Saving a signed document whose edits cannot be appended to it: the
 * engine refuses, the user is offered a copy, and the copy is written to a
 * new name with the signatures knowingly broken - never over the original. */
import { afterEach, expect, it, vi } from 'vitest';

const askConfirm = vi.fn<(message: string, label?: string) => Promise<boolean>>();
const showError = vi.fn<(message: string) => void>();
vi.mock('../../stores/modalStore', () => ({
  askConfirm: (message: string, label?: string) => askConfirm(message, label),
  askUnsaved: vi.fn(),
  showError: (message: string) => showError(message),
}));
const saveDialog = vi.fn<() => Promise<string | null>>();
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: () => saveDialog() }));

import { engine } from '../../lib/transport/engine';
import type { TabRecord } from '../../stores/tabsStore';
import { createDocumentStore } from './documentStore';
import { saveDocument } from './controller';
import { recentStore } from '../../stores/recentStore';

/** What reopening the saved copy needs from the engine. */
function stubReopen() {
  vi.spyOn(engine, 'open').mockResolvedValue({
    docId: 9,
    path: '/tmp/copy.pdf',
    name: 'copy.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  vi.spyOn(engine, 'getAnnotations').mockResolvedValue([]);
  vi.spyOn(engine, 'close').mockResolvedValue();
  vi.spyOn(engine, 'bumpGeneration').mockResolvedValue(1);
  vi.spyOn(recentStore, 'recordOpen').mockImplementation(() => {});
}

function signedTab(): TabRecord {
  const documentStore = createDocumentStore();
  documentStore.initFromMeta({
    docId: 3,
    path: '/tmp/signed.pdf',
    name: 'signed.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  return {
    documentStore,
    viewport: { state: { currentPage: 0 }, requestScrollToPage: vi.fn() },
    searchStore: { resetForDocument: vi.fn() },
    citationStore: { syncDocument: vi.fn() },
    structureStore: { syncDocument: vi.fn() },
  } as unknown as TabRecord;
}

afterEach(() => {
  vi.restoreAllMocks();
  askConfirm.mockReset();
  showError.mockReset();
  saveDialog.mockReset();
});

it('offers a copy when the engine refuses to rewrite a signed document', async () => {
  const tab = signedTab();
  const save = vi
    .spyOn(engine, 'saveDocument')
    .mockRejectedValueOnce({ code: 'signed', message: 'signed' })
    .mockResolvedValueOnce({ path: '/tmp/copy.pdf', bytes: 1, durationMs: 1 });
  stubReopen();
  askConfirm.mockResolvedValue(true);
  // First the user's own Save As target, refused; then the copy.
  saveDialog.mockResolvedValueOnce('/tmp/first.pdf').mockResolvedValueOnce('/tmp/copy');

  expect(await saveDocument(tab, true)).toBe(true);
  expect(askConfirm).toHaveBeenCalledTimes(1);
  expect(askConfirm.mock.calls[0]![0]).toContain('digitally signed');
  expect(save).toHaveBeenCalledTimes(2);
  expect(save.mock.calls[0]![3]).toBe(false);
  expect(save.mock.calls[1]![2]).toBe('/tmp/copy.pdf');
  expect(save.mock.calls[1]![3]).toBe(true);
  expect(showError).not.toHaveBeenCalled();
});

it('never writes the copy over the signed original', async () => {
  const tab = signedTab();
  const save = vi
    .spyOn(engine, 'saveDocument')
    .mockRejectedValueOnce({ code: 'signed', message: 'signed' });
  askConfirm.mockResolvedValue(true);
  saveDialog.mockResolvedValueOnce('/tmp/elsewhere.pdf').mockResolvedValueOnce('/tmp/signed.pdf');

  expect(await saveDocument(tab, true)).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
  expect(showError.mock.calls[0]![0]).toContain('new name');
});

it('keeps the original untouched when the copy is declined', async () => {
  const tab = signedTab();
  const save = vi
    .spyOn(engine, 'saveDocument')
    .mockRejectedValueOnce({ code: 'signed', message: 'signed' });
  askConfirm.mockResolvedValue(false);
  saveDialog.mockResolvedValue('/tmp/elsewhere.pdf');

  expect(await saveDocument(tab, true)).toBe(false);
  expect(save).toHaveBeenCalledTimes(1);
  expect(showError).not.toHaveBeenCalled();
});

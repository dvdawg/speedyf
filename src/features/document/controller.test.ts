import { expect, it, vi } from 'vitest';
import { engine } from '../../lib/transport/engine';
import { recentStore } from '../../stores/recentStore';
import type { TabRecord } from '../../stores/tabsStore';
import { createDocumentStore } from './documentStore';
import { reloadTabFromDisk } from './controller';

it('restores editable text and its formatting immediately after saving', async () => {
  vi.spyOn(engine, 'open').mockResolvedValue({
    docId: 2,
    path: '/tmp/styled.pdf',
    name: 'styled.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  vi.spyOn(engine, 'getAnnotations').mockResolvedValue([
    {
      src: 0,
      annots: [
        {
          index: 0,
          kind: 'textbox',
          rect: { x: 30, y: 40, w: 150, h: 40 },
          color: '#2255aa',
          opacity: 1,
          text: 'Saved text',
          fontSizePt: 24,
          fontFamily: 'Times-Roman',
          bold: true,
          italic: true,
          underline: true,
          strokeWidth: null,
          quads: null,
          strokes: null,
        },
      ],
    },
  ]);
  vi.spyOn(engine, 'close').mockResolvedValue();
  vi.spyOn(engine, 'bumpGeneration').mockResolvedValue(1);
  vi.spyOn(recentStore, 'recordOpen').mockImplementation(() => {});
  const documentStore = createDocumentStore();
  const tab = {
    documentStore,
    searchStore: { resetForDocument: vi.fn() },
    citationStore: { syncDocument: vi.fn() },
    structureStore: { syncDocument: vi.fn() },
  } as unknown as TabRecord;
  try {
    await reloadTabFromDisk(tab, '/tmp/styled.pdf');
    const page = documentStore.state.pages[0]!;
    expect(documentStore.state.annotations[page.id]?.[0]).toMatchObject({
      pageId: page.id,
      text: 'Saved text',
      fontFamily: 'Times-Roman',
      fontSizePt: 24,
      bold: true,
      italic: true,
      underline: true,
    });
    expect(documentStore.state.dirty).toBe(false);
    expect(documentStore.buildEditPlan().pages[0]!.texts).toEqual([]);
  } finally {
    vi.restoreAllMocks();
  }
});

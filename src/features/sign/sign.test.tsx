// @vitest-environment happy-dom
/**
 * Visual signatures from the outside in: the saved-signature store and what
 * it persists, the create dialog in each of its three modes, placing a
 * signature on a page with a click (and undoing it), and fitting one into an
 * empty signature field. The engine side - writing the stamp and reading it
 * back - is covered in src-tauri/src/engine/signature.rs.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { testField } from '../forms/testField';
import type { FormFieldInfo } from '../../types/engine';
import type { Annotation, PageEntry } from '../../types/model';
import type * as formStoreModule from '../forms/formStore';

const api = vi.hoisted(() => ({
  stored: null as string | null,
  saves: [] as string[],
  outline: vi.fn(),
  prepareImage: vi.fn(),
}));

vi.mock('../../lib/transport/signatures', () => ({
  signaturesApi: {
    load: () => Promise.resolve(api.stored),
    save: (json: string) => {
      api.saves.push(json);
      api.stored = json;
      return Promise.resolve();
    },
    outline: api.outline,
    prepareImage: api.prepareImage,
  },
}));

const dialog = vi.hoisted(() => ({ picked: null as string | null }));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: () => Promise.resolve(dialog.picked),
}));

let fields: FormFieldInfo[] = [];
vi.mock('../forms/formStore', async (importOriginal) => {
  const actual = await importOriginal<typeof formStoreModule>();
  return { ...actual, formFields: () => Promise.resolve(fields) };
});

import {
  addSignature,
  armPlacement,
  defaultSignature,
  loadSignatures,
  openSignDialog,
  removeSignature,
  resetSignStore,
  signState,
  type SavedSignature,
} from './signStore';
import SignDialog from './SignDialog';
import SignatureFieldLayer from './SignatureFieldLayer';
import AnnotationLayer from '../annotations/AnnotationLayer';
import { TabContext } from '../../app/TabContext';
import { createDocumentStore } from '../document/documentStore';
import type { TabRecord } from '../../stores/tabsStore';

const typedArt = {
  type: 'typed' as const,
  w: 200,
  h: 50,
  text: 'Ada Lovelace',
  style: 'flowing' as const,
  d: 'M4 4L196 46Z',
};

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

let documentStore: ReturnType<typeof createDocumentStore>;
let page: PageEntry;

function newDocument() {
  documentStore = createDocumentStore();
  documentStore.initFromMeta({
    docId: 7,
    path: '/tmp/contract.pdf',
    name: 'contract.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  page = documentStore.state.pages[0] as PageEntry;
  return { id: 'tab-1', documentStore } as unknown as TabRecord;
}

const signaturesOnPage = (): Annotation[] =>
  (documentStore.state.annotations[page.id] ?? []).filter((a) => a.kind === 'signature');

beforeEach(() => {
  api.stored = null;
  api.saves = [];
  api.outline.mockReset();
  api.prepareImage.mockReset();
  dialog.picked = null;
  fields = [];
  resetSignStore();
});

afterEach(() => cleanup());

describe('the saved-signature store', () => {
  it('starts empty, saves what is added and reads it back', async () => {
    await loadSignatures();
    expect(signState.items).toEqual([]);
    const saved = await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' });
    expect(defaultSignature('signature')?.id).toBe(saved.id);
    expect(defaultSignature('initials')).toBeNull();

    const file = JSON.parse(api.stored!) as { v: number; items: SavedSignature[] };
    expect(file.v).toBe(1);
    expect(file.items[0]?.art).toEqual(typedArt);

    resetSignStore();
    await loadSignatures();
    expect(signState.items.map((s) => s.id)).toEqual([saved.id]);
  });

  it('keeps several, newest first, and removes one', async () => {
    const a = await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' });
    const b = await addSignature({ role: 'initials', art: typedArt, color: '#1d3c8f' });
    const c = await addSignature({ role: 'signature', art: typedArt, color: '#1d3c8f' });
    expect(defaultSignature('signature')?.id).toBe(c.id);
    expect(defaultSignature('initials')?.id).toBe(b.id);
    await removeSignature(c.id);
    expect(defaultSignature('signature')?.id).toBe(a.id);
    expect((JSON.parse(api.stored!) as { items: unknown[] }).items).toHaveLength(2);
  });

  it('ignores a damaged file rather than failing', async () => {
    api.stored = '{"v":1,"items":[{"id":3},null]}';
    await loadSignatures();
    expect(signState.items).toEqual([]);
  });
});

describe('placing a signature on a page', () => {
  const mountLayer = () => {
    const tab = newDocument();
    const geom = { widthPt: page.widthPt, heightPt: page.heightPt, rotation: 0 as const };
    const view = render(() => (
      <TabContext.Provider value={tab}>
        <AnnotationLayer page={page} geom={geom} zoom={1} runs={[]} />
      </TabContext.Provider>
    ));
    const layer = view.container.querySelector('.annot-layer') as HTMLDivElement;
    // happy-dom does no layout: give the layer the page's box at zoom 1, so
    // client coordinates map onto PDF points.
    layer.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 612, height: 792, right: 612, bottom: 792 }) as DOMRect;
    return layer;
  };

  it('places the armed signature where the page is clicked, then undoes it', async () => {
    const layer = mountLayer();
    const saved = await addSignature({ role: 'signature', art: typedArt, color: '#1d3c8f' });
    armPlacement(saved);
    expect(layer.classList.contains('is-placing-signature')).toBe(true);

    // A click at the page's top-left corner: centred there, then kept on the
    // page.
    fireEvent.pointerDown(layer, { button: 0, clientX: 0, clientY: 0, pointerId: 1 });
    await tick();
    const [placed] = signaturesOnPage();
    expect(placed).toBeDefined();
    expect(placed!.signature).toEqual(typedArt);
    expect(placed!.color).toBe('#1d3c8f');
    expect(placed!.rect.w / placed!.rect.h).toBeCloseTo(4);
    expect(placed!.rect.x).toBe(0);
    expect(placed!.rect.y + placed!.rect.h).toBeCloseTo(792);
    expect(signState.pending).toBeNull();
    expect(documentStore.state.selected?.annotId).toBe(placed!.id);
    expect(layer.querySelector('.annot-signature path')?.getAttribute('d')).toBe(typedArt.d);
    expect(documentStore.state.dirty).toBe(true);

    documentStore.undo();
    expect(signaturesOnPage()).toEqual([]);
    documentStore.redo();
    expect(signaturesOnPage()).toHaveLength(1);
  });

  it('fits an armed signature into the empty field it is clicked in', async () => {
    fields = [
      testField({
        name: 'Signature1',
        kind: 'signature',
        page: 0,
        index: 0,
        rect: { x: 300, y: 92, w: 200, h: 40 },
        signed: false,
      }),
    ];
    const layer = mountLayer();
    armPlacement(await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' }));
    // (310, 690) on screen is (310, 102) on the page: inside the field.
    fireEvent.pointerDown(layer, { button: 0, clientX: 310, clientY: 690, pointerId: 1 });
    await tick();
    const r = signaturesOnPage()[0]!.rect;
    expect(r.x).toBeGreaterThanOrEqual(300);
    expect(r.x + r.w).toBeLessThanOrEqual(500);
    expect(r.y).toBeGreaterThanOrEqual(92);
    expect(r.y + r.h).toBeLessThanOrEqual(132);
    expect(r.x + r.w / 2).toBeCloseTo(400);
  });

  it('lets Escape put an armed signature down', async () => {
    mountLayer();
    armPlacement(await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' }));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(signState.pending).toBeNull();
  });

  it('deletes a placed signature from the keyboard, undoably', async () => {
    const layer = mountLayer();
    armPlacement(await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' }));
    fireEvent.pointerDown(layer, { button: 0, clientX: 0, clientY: 0, pointerId: 1 });
    await tick();
    const target = layer.querySelector('.annot-keyboard-target') as HTMLButtonElement;
    fireEvent.keyDown(target, { key: 'Delete' });
    expect(signaturesOnPage()).toEqual([]);
    documentStore.undo();
    expect(signaturesOnPage()).toHaveLength(1);
  });

  it('resizes a placed signature without stretching it', async () => {
    const layer = mountLayer();
    armPlacement(await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' }));
    fireEvent.pointerDown(layer, { button: 0, clientX: 0, clientY: 0, pointerId: 1 });
    await tick();
    const before = signaturesOnPage()[0]!.rect;
    const handle = layer.querySelector('.handle-se') as HTMLDivElement;
    expect(handle).not.toBeNull();
    // Drag the south-east corner well right and a little down.
    fireEvent.pointerDown(handle, { button: 0, clientX: 150, clientY: 37.5, pointerId: 2 });
    fireEvent.pointerUp(layer, { clientX: 400, clientY: 60, pointerId: 2 });
    const after = signaturesOnPage()[0]!.rect;
    expect(after.w).toBeGreaterThan(before.w);
    expect(after.w / after.h).toBeCloseTo(before.w / before.h);
    documentStore.undo();
    expect(signaturesOnPage()[0]!.rect).toEqual(before);
  });
});

describe('the create dialog', () => {
  const mountDialog = (role: 'signature' | 'initials' = 'signature') => {
    openSignDialog(role);
    return render(() => <SignDialog />);
  };

  it('types a name in a chosen style and arms it for placing', async () => {
    api.outline.mockResolvedValue({ d: 'M1 1L9 9Z', w: 120, h: 30, missing: '' });
    const view = mountDialog();
    const name = view.getByLabelText('Name') as HTMLInputElement;
    fireEvent.input(name, { target: { value: 'Ada Lovelace' } });
    // Every style previews the name being typed.
    expect(view.getAllByText('Ada Lovelace')).toHaveLength(3);
    fireEvent.click(view.getByLabelText('Hand style'));
    fireEvent.click(view.getByLabelText('Blue ink'));
    fireEvent.click(view.getByText('Save and place'));
    await tick();
    await tick();

    expect(api.outline).toHaveBeenCalledWith('Ada Lovelace', 'hand');
    expect(signState.dialog).toBeNull();
    expect(signState.pending?.art).toEqual({
      type: 'typed',
      w: 120,
      h: 30,
      text: 'Ada Lovelace',
      style: 'hand',
      d: 'M1 1L9 9Z',
    });
    expect(signState.pending?.color).toBe('#1d3c8f');
    expect(api.saves).toHaveLength(1);
  });

  it('refuses a style that cannot write the name, and says why', async () => {
    api.outline.mockResolvedValue({ d: 'M1 1Z', w: 10, h: 10, missing: 'X' });
    const view = mountDialog();
    fireEvent.input(view.getByLabelText('Name'), { target: { value: 'Ada X' } });
    fireEvent.click(view.getByText('Save and place'));
    await tick();
    expect(view.getByRole('alert').textContent).toContain('cannot write X');
    expect(signState.dialog).not.toBeNull();
    expect(api.saves).toHaveLength(0);
  });

  it('draws strokes, undoes one, and saves the rest as vector strokes', async () => {
    const view = mountDialog('initials');
    fireEvent.click(view.getByText('Draw'));
    const pad = view.getByRole('img') as unknown as SVGSVGElement;
    const save = view.getByText('Save and place') as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    const stroke = (y: number) => {
      fireEvent.pointerDown(pad, { button: 0, clientX: 10, clientY: y, pointerId: 1 });
      for (let x = 20; x <= 120; x += 10) {
        fireEvent.pointerMove(pad, { clientX: x, clientY: y + x / 10, pointerId: 1 });
      }
      fireEvent.pointerUp(pad, { clientX: 130, clientY: y + 13, pointerId: 1 });
    };
    stroke(20);
    stroke(60);
    expect(pad.querySelectorAll('path')).toHaveLength(2);
    fireEvent.click(view.getByText('Undo stroke'));
    expect(pad.querySelectorAll('path')).toHaveLength(1);

    fireEvent.click(save);
    await tick();
    await tick();
    const art = signState.pending?.art;
    expect(signState.pending?.role).toBe('initials');
    expect(art?.type).toBe('drawn');
    if (art?.type !== 'drawn') return;
    expect(art.strokes).toHaveLength(1);
    expect(art.strokes[0]!.length).toBeGreaterThan(3);
    // Cropped to the ink, with the pen's width of margin.
    const xs = art.strokes[0]!.map((p) => p.x);
    expect(Math.min(...xs)).toBeCloseTo(art.width);
  });

  it('imports a picture through the engine', async () => {
    dialog.picked = '/tmp/scan.png';
    api.prepareImage.mockResolvedValue({ png: 'iVBORw0KGgo=', w: 300, h: 90 });
    const view = mountDialog();
    fireEvent.click(view.getByText('Image'));
    fireEvent.click(view.getByText('Choose image...'));
    await tick();
    await tick();
    expect(api.prepareImage).toHaveBeenCalledWith('/tmp/scan.png');
    fireEvent.click(view.getByText('Save and place'));
    await tick();
    await tick();
    expect(signState.pending?.art).toEqual({ type: 'image', w: 300, h: 90, png: 'iVBORw0KGgo=' });
  });
});

describe('empty signature fields', () => {
  const field = (over: Partial<FormFieldInfo>): FormFieldInfo =>
    testField({
      name: 'Signature1',
      kind: 'signature',
      index: 3,
      rect: { x: 100, y: 100, w: 200, h: 40 },
      exportValue: '',
      signed: false,
      ...over,
    });

  const mountFields = async () => {
    const tab = newDocument();
    const view = render(() => (
      <TabContext.Provider value={tab}>
        <SignatureFieldLayer docId={7} src={0} page={page} zoom={1} />
      </TabContext.Provider>
    ));
    await tick();
    return view;
  };

  it('offers only unsigned, editable signature fields, labelled as visual', async () => {
    fields = [
      field({}),
      field({ name: 'Signed', signed: true, index: 4 }),
      field({ name: 'Locked', readOnly: true, index: 5 }),
      field({ name: 'Text', kind: 'text', index: 6 }),
    ];
    const view = await mountFields();
    const targets = view.container.querySelectorAll('.sign-field');
    expect(targets).toHaveLength(1);
    const label = targets[0]!.getAttribute('title') ?? '';
    expect(label).toMatch(/visual signature/);
    expect(label).toMatch(/not a certificate signature/);
  });

  it('fits the saved signature into the clicked field, and then stops offering it', async () => {
    fields = [field({})];
    await addSignature({ role: 'signature', art: typedArt, color: '#1a1a1a' });
    const view = await mountFields();
    fireEvent.click(view.container.querySelector('.sign-field')!);
    await tick();
    const [placed] = signaturesOnPage();
    expect(placed).toBeDefined();
    const r = placed!.rect;
    expect(r.x).toBeGreaterThanOrEqual(100);
    expect(r.x + r.w).toBeLessThanOrEqual(300);
    expect(r.y).toBeGreaterThanOrEqual(100);
    expect(r.y + r.h).toBeLessThanOrEqual(140);
    expect(r.w / r.h).toBeCloseTo(4);
    expect(view.container.querySelector('.sign-field')).toBeNull();
    documentStore.undo();
    expect(view.container.querySelector('.sign-field')).not.toBeNull();
  });

  it('asks for a signature first when none is saved, then fills the field', async () => {
    fields = [field({})];
    api.outline.mockResolvedValue({ d: 'M1 1Z', w: 100, h: 25, missing: '' });
    const view = await mountFields();
    fireEvent.click(view.container.querySelector('.sign-field')!);
    await tick();
    expect(signState.dialog?.role).toBe('signature');

    const dialogView = render(() => <SignDialog />);
    fireEvent.input(dialogView.getByLabelText('Name'), { target: { value: 'Ada' } });
    fireEvent.click(dialogView.getByText('Save and place'));
    await tick();
    await tick();
    expect(signState.pending).toBeNull();
    const [placed] = signaturesOnPage();
    expect(placed?.rect.y).toBeGreaterThanOrEqual(100);
    expect(placed!.rect.y + placed!.rect.h).toBeLessThanOrEqual(140);
  });
});

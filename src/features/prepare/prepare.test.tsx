// @vitest-environment happy-dom
/**
 * Prepare form mode on screen: entering and leaving it, drawing fields with
 * the tool row (and snapping to blanks), moving, nudging and deleting them,
 * the properties panel, auto-detect's proposals, the Tab order, and the
 * plan a save sends. The engine side of the same flow is tested in
 * src-tauri/src/engine/forms/design_tests.rs.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Blank, FormInfo, ProposedField } from '../../types/engine';
import type * as engineModule from '../../lib/transport/engine';
import type * as modalModule from '../../stores/modalStore';

const fake = vi.hoisted(() => ({
  blanks: new Map<number, unknown[]>(),
  form: { fields: [], xfa: false } as unknown,
  proposals: [] as unknown[],
  signatures: [] as unknown[],
  confirm: true,
  asked: [] as string[],
}));

vi.mock('../../lib/transport/engine', async (importOriginal) => {
  const actual = await importOriginal<typeof engineModule>();
  return {
    ...actual,
    engine: {
      ...actual.engine,
      getPageBlanks: (_docId: number, src: number) => Promise.resolve(fake.blanks.get(src) ?? []),
      getFormFields: () => Promise.resolve(fake.form),
      getFormProposals: () => Promise.resolve(fake.proposals),
      checkSignatures: () => Promise.resolve(fake.signatures),
    },
  };
});

vi.mock('../../stores/modalStore', async (importOriginal) => {
  const actual = await importOriginal<typeof modalModule>();
  return {
    ...actual,
    askConfirm: (message: string) => {
      fake.asked.push(message);
      return Promise.resolve(fake.confirm);
    },
  };
});

import { TabContext } from '../../app/TabContext';
import Toolbar from '../../components/Toolbar';
import { addTab, createTab, removeTab, setActiveId, tabsStore } from '../../stores/tabsStore';
import { clearFormFields } from '../forms/formStore';
import { clearSignatures } from '../signatures/signatureStore';
import { testField } from '../forms/testField';
import { resetFill } from '../sign/fillStore';
import PrepareLayer from './PrepareLayer';
import PreparePanel from './PreparePanel';
import { prepareKey } from './PrepareTools';
import {
  designOf,
  detectFields,
  prepareState,
  preparing,
  resetPrepare,
  select,
  setFlatten,
  setTool,
  startPreparing,
  stopPreparing,
} from './prepareStore';
import { COMMANDS } from '../../app/commands';

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const settle = async () => {
  for (let i = 0; i < 4; i += 1) await tick();
};

const CHECK: Blank = {
  rect: { x: 72, y: 560, w: 10, h: 10 },
  kind: 'check',
  fontSizePt: 9,
  cells: 0,
};
const LINE: Blank = {
  rect: { x: 120, y: 700, w: 200, h: 14 },
  kind: 'line',
  fontSizePt: 9,
  cells: 0,
};

beforeEach(() => {
  fake.blanks.clear();
  fake.blanks.set(0, [LINE, CHECK]);
  fake.form = {
    xfa: false,
    fields: [
      testField({
        name: 'existing',
        kind: 'text',
        index: 0,
        rect: { x: 72, y: 400, w: 150, h: 16 },
      }),
    ],
  } satisfies FormInfo;
  fake.proposals = [];
  fake.signatures = [];
  fake.confirm = true;
  fake.asked = [];
  clearSignatures();
  resetFill();
  resetPrepare();
  clearFormFields();
});

afterEach(() => {
  cleanup();
  for (const tab of [...tabsStore.state.tabs]) removeTab(tab.id);
  setActiveId(null);
  vi.restoreAllMocks();
});

async function mount() {
  const tab = createTab();
  tab.documentStore.initFromMeta({
    docId: 7,
    path: '/tmp/flat.pdf',
    name: 'flat.pdf',
    pageCount: 1,
    sizes: [[612, 792, 0, 0, 0]],
    estimatedSize: [612, 792],
  });
  addTab(tab);
  setActiveId(tab.id);
  const page = tab.documentStore.state.pages[0]!;
  const view = render(() => (
    <>
      <Toolbar />
      <TabContext.Provider value={tab}>
        <div class="page">
          <PrepareLayer
            pageId={page.id}
            src={0}
            geom={{ widthPt: 612, heightPt: 792, rotation: 0 }}
            zoom={1}
          />
        </div>
        <PreparePanel />
      </TabContext.Provider>
    </>
  ));
  await startPreparing(tab);
  await settle();
  const layer = () => view.container.querySelector('.prepare-layer') as HTMLElement;
  vi.spyOn(layer(), 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 612, 792));
  /** Pointer events at page coordinates (y up). */
  const at = (x: number, y: number) => ({ button: 0, clientX: x, clientY: 792 - y, pointerId: 1 });
  const drag = (from: [number, number], to: [number, number], target?: Element) => {
    fireEvent.pointerDown(target ?? layer(), at(...from));
    fireEvent.pointerMove(layer(), at(...to));
    fireEvent.pointerUp(layer(), at(...to));
  };
  const design = () => designOf(tab);
  const made = () => design().fields.filter((f) => !f.origin);
  return { ...view, tab, page, layer, at, drag, design, made };
}

describe('the mode', () => {
  it('enters with the form read in, shows its tool row, and leaves on Done', async () => {
    const view = await mount();
    expect(preparing(view.tab)).toBe(true);
    expect(view.design().fields.map((f) => f.name)).toEqual(['existing']);
    const row = view.getByRole('toolbar', { name: 'Prepare form' });
    for (const label of [
      'Text field',
      'Multiline text field',
      'Checkbox',
      'Radio button',
      'Dropdown',
      'List box',
      'Signature field',
      'Date field',
    ]) {
      expect(row.querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    }
    expect(view.tab.viewport.state.formPanelOpen).toBe(true);
    // Entering is not an edit.
    expect(view.tab.documentStore.state.dirty).toBe(false);
    fireEvent.click(view.getAllByRole('button', { name: 'Done' })[0]!);
    expect(preparing(view.tab)).toBe(false);
    expect(view.queryByRole('toolbar', { name: 'Prepare form' })).toBeNull();
  });

  it('is in the palette, not on the toolbar', () => {
    const ids = COMMANDS.map((c) => c.id);
    expect(ids).toContain('forms.prepare');
    expect(ids).toContain('forms.autodetect');
    expect(ids).toContain('forms.exportData');
    expect(ids).toContain('forms.importData');
  });
});

describe('drawing', () => {
  it('drags out a field, then returns to selecting', async () => {
    const view = await mount();
    setTool('text');
    view.drag([300, 300], [450, 320]);
    expect(view.made()).toHaveLength(1);
    const f = view.made()[0]!;
    expect(f.kind).toBe('text');
    expect(f.rect).toMatchObject({ x: 300, y: 300, w: 150, h: 20 });
    expect(prepareState.tool).toBe('select');
    expect(prepareState.selected).toEqual([f.id]);
    expect(view.tab.documentStore.state.dirty).toBe(true);
    view.tab.documentStore.undo();
    expect(view.made()).toHaveLength(0);
  });

  it('snaps to the blank under a click or a drag', async () => {
    const view = await mount();
    setTool('checkbox');
    view.drag([76, 564], [76, 564]);
    expect(view.made()[0]!.rect).toEqual(CHECK.rect);
    expect(view.made()[0]!.border).toBe(false);
    setTool('text');
    view.drag([118, 699], [322, 713]);
    const line = view.made()[1]!;
    expect(line.rect).toEqual(LINE.rect);
    expect(line.fontSize).toBe(9);
    // Away from any blank, a click makes the usual size.
    setTool('date');
    view.drag([300, 200], [300, 200]);
    const date = view.made()[2]!;
    expect(date.rect).toMatchObject({ w: 90, h: 18 });
    expect(date.format).toMatchObject({ kind: 'date' });
  });
});

describe('editing on the page', () => {
  it('moves by dragging, nudges with the arrows, duplicates and deletes', async () => {
    const view = await mount();
    const existing = view.design().fields[0]!;
    const box = view.container.querySelector(`[data-field="${existing.id}"]`)!;
    view.drag([80, 405], [100, 395], box);
    expect(view.design().fields[0]!.rect).toMatchObject({ x: 92, y: 390 });
    const key = (k: string, extra: Partial<KeyboardEventInit> = {}) =>
      prepareKey(new KeyboardEvent('keydown', { key: k, ...extra }));
    expect(key('ArrowRight')).toBe(true);
    expect(key('ArrowUp', { shiftKey: true })).toBe(true);
    expect(view.design().fields[0]!.rect).toMatchObject({ x: 93, y: 400 });
    // Two nudges in a row are one undo step.
    view.tab.documentStore.undo();
    expect(view.design().fields[0]!.rect).toMatchObject({ x: 92, y: 390 });
    expect(key('d', { metaKey: true })).toBe(true);
    expect(view.design().fields).toHaveLength(2);
    expect(key('Delete')).toBe(true);
    expect(view.design().fields).toHaveLength(1);
    select([existing.id]);
    key('Delete');
    expect(view.design().fields).toHaveLength(0);
    expect(view.design().deleted).toHaveLength(1);
    // Where it was is covered: the raster still shows it.
    await settle();
    expect(view.container.querySelectorAll('.prepare-cover')).toHaveLength(1);
    expect(key('Escape')).toBe(true);
    expect(preparing(view.tab)).toBe(false);
  });
});

describe('the properties panel', () => {
  it('edits the selected field, one undo step per box', async () => {
    const view = await mount();
    const existing = view.design().fields[0]!;
    select([existing.id]);
    await settle();
    const name = view.getByRole('textbox', { name: 'Name' }) as HTMLInputElement;
    fireEvent.input(name, { target: { value: 'Full' } });
    fireEvent.input(name, { target: { value: 'Full name' } });
    expect(view.design().fields[0]!.name).toBe('Full name');
    fireEvent.click(view.getByRole('checkbox', { name: 'Required' }));
    expect(view.design().fields[0]!.required).toBe(true);
    const format = view.getByRole('combobox', { name: 'Format' }) as HTMLSelectElement;
    fireEvent.change(format, { target: { value: 'currency' } });
    expect(view.design().fields[0]!.format).toMatchObject({ kind: 'number', currency: '$' });
    view.tab.documentStore.undo();
    view.tab.documentStore.undo();
    expect(view.design().fields[0]!.name).toBe('Full name');
    view.tab.documentStore.undo();
    expect(view.design().fields[0]!.name).toBe('existing');
  });

  it('refuses a name another field has', async () => {
    const view = await mount();
    setTool('text');
    view.drag([300, 300], [450, 320]);
    await settle();
    const name = view.getByRole('textbox', { name: 'Name' }) as HTMLInputElement;
    fireEvent.input(name, { target: { value: 'existing' } });
    expect(view.getByText('Another field has this name')).toBeTruthy();
    expect(view.made()[0]!.name).toBe('Text 1');
  });

  it("edits a dropdown's options", async () => {
    const view = await mount();
    setTool('combo');
    view.drag([300, 300], [450, 320]);
    await settle();
    fireEvent.click(view.getByRole('button', { name: 'Add option' }));
    const label = view.getByRole('textbox', { name: 'Option 3 label' });
    fireEvent.input(label, { target: { value: 'Texas' } });
    fireEvent.click(view.getAllByRole('button', { name: 'Move up' })[2]!);
    const options = view.made()[0]!.options;
    expect(options.map((o) => o.label)).toEqual(['Option 1', 'Texas', 'Option 2']);
    expect(options[1]!.export).toBe('Texas');
  });
});

describe('auto-detect', () => {
  it('proposes fields, skipping ones already there; accept and reject', async () => {
    const proposal = (name: string, x: number, y: number, kind: 'text' | 'checkbox' = 'text') =>
      ({
        page: 0,
        rect: { x, y, w: kind === 'text' ? 150 : 10, h: kind === 'text' ? 16 : 10 },
        kind,
        name,
        label: name,
        fontSize: 9,
        maxLen: 0,
        comb: false,
        multiline: false,
        blank: kind === 'text' ? 'line' : 'check',
      }) satisfies ProposedField;
    fake.proposals = [
      proposal('Name', 120, 700),
      proposal('Over existing', 72, 400),
      proposal('Married', 72, 560, 'checkbox'),
      proposal('City', 120, 650),
    ];
    const view = await mount();
    await detectFields(view.tab);
    await settle();
    expect(prepareState.proposals.map((p) => p.name)).toEqual(['Name', 'Married', 'City']);
    expect(view.container.querySelectorAll('.prepare-proposal')).toHaveLength(3);
    fireEvent.click(view.getByRole('button', { name: 'Accept Name' }));
    fireEvent.click(view.getByRole('button', { name: 'Reject City' }));
    expect(view.made().map((f) => f.name)).toEqual(['Name']);
    fireEvent.click(view.getByRole('button', { name: 'Accept all' }));
    expect(view.made().map((f) => [f.name, f.kind])).toEqual([
      ['Name', 'text'],
      ['Married', 'checkbox'],
    ]);
    expect(prepareState.proposals).toHaveLength(0);
  });
});

describe('Tab order and the plan', () => {
  it('orders by rows, shows numbers, and saves it all', async () => {
    const view = await mount();
    setTool('text');
    view.drag([300, 600], [450, 620]);
    setTool('text');
    view.drag([50, 700], [200, 716]);
    select([]);
    await settle();
    fireEvent.click(view.getByRole('button', { name: 'Order by rows' }));
    fireEvent.click(view.getByRole('checkbox', { name: 'Show numbers on the page' }));
    await settle();
    const badges = [...view.container.querySelectorAll('.prepare-tab-badge')].map(
      (b) => b.textContent
    );
    expect(badges.sort()).toEqual(['1', '2', '3']);
    const rows = [...view.container.querySelectorAll('.prepare-tab-name')].map(
      (r) => r.textContent
    );
    expect(rows).toEqual(['Text 2', 'Text 1', 'existing']);
    fireEvent.click(view.getByRole('checkbox', { name: 'Flatten form on save' }));
    await settle();
    const plan = view.tab.documentStore.buildEditPlan().formDesign!;
    expect(plan.fields.map((f) => f.name)).toEqual(['Text 1', 'Text 2']);
    expect(plan.tabOrders[0]!.mode).toBe('R');
    expect(plan.tabOrders[0]!.order).toHaveLength(3);
    expect(plan.tabOrders[0]!.order[2]!.widget?.name).toBe('existing');
    expect(plan.flatten).toBe(true);
    stopPreparing();
    // Leaving the mode keeps the edits for the save.
    expect(view.tab.documentStore.buildEditPlan().formDesign).toBeDefined();
  });
});

describe('flatten', () => {
  it('turns on without asking on an unsigned document', async () => {
    const { tab, design } = await mount();
    expect(await setFlatten(tab, true)).toBe(true);
    expect(design().flatten).toBe(true);
    expect(fake.asked).toEqual([]);
  });

  it('asks first on a signed document, and stays off if declined', async () => {
    fake.signatures = [{ fieldName: 'Signature1' }];
    fake.confirm = false;
    const { tab, design } = await mount();
    expect(await setFlatten(tab, true)).toBe(false);
    expect(design().flatten).toBe(false);
    expect(fake.asked[0]).toContain('digitally signed');

    fake.confirm = true;
    expect(await setFlatten(tab, true)).toBe(true);
    expect(design().flatten).toBe(true);
    // turning it back off never needs asking
    expect(await setFlatten(tab, false)).toBe(true);
    expect(fake.asked).toHaveLength(2);
  });
});
